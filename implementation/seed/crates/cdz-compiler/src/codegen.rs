//! `cdz-rustc` — the reference Cadenza → WebAssembly-component compiler.
//!
//! This is the foreign-language *seed compiler* (constitution XIV; bootstrap.md §"The Seed
//! Reference Compiler Is Native And Compiles Cadenza To A Component"). It is written from
//! first principles as a clean AST → wasm lowering; the Cadenza-authored compiler is a
//! forward-port of this design, and the two must agree (the compiler-vs-compiler
//! differential that replaces the old interpreter-vs-compiler one).
//!
//! **Purity.** This module is a pure function `ast_bytes -> component_bytes`: no host, no
//! filesystem, no wasmtime. That is deliberate — the same core compiles to `wasm32` and is
//! wrapped as a component exporting `compile : list<u8> -> list<u8>`, the SAME ABI the
//! Cadenza compiler exports, so cdz-rustc runs everywhere wasm runs and the self-hosting
//! fixpoint is a byte-identity check between two components
//! (spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md).
//!
//! A construct the compiler does not yet lower is *declined* (an honest backlog entry),
//! never miscompiled (self-hosting-and-bootstrap.md §"An Unsupported Construct Is Declined,
//! Not Miscompiled").

use crate::ast::{self, Node};

// The value-heap runtime interface — the component-model envelope byte-chunks (RT_HEAD/RT_TAIL/
// RT_IMPORT_CONTENT/HOST_MEM_MODULE/RT_MEM/RT_GLOBAL), the import indices (`himport`), the core
// signatures (`rt_import_types`), `RT_N_IMPORTS`/`RT_TAIL_PREFIX_LEN`, and the required-runtime pin
// (`REQUIRED_RUNTIME_HASH`) — is GENERATED from the runtime's WIT by `xtask build` (see
// wit_envelope.rs). It is the compiler's single-source-of-truth view of the runtime contract; do not
// hand-edit `heap_envelope.rs`. See spec/learnings/2026-07-06-the-envelope-blobs-are-generated-from-the-runtime-contract.md.
#[path = "heap_envelope.rs"]
mod heap_envelope;
#[allow(unused_imports)]
use heap_envelope::{
    himport, rt_import_types, COMPILE_ARTIFACTS_HEAD, COMPILE_ARTIFACTS_TAIL, COMPILE_HEAD,
    COMPILE_RESULT_HEAD, COMPILE_RESULT_TAIL, COMPILE_TAIL,
    HOST_MEM_MODULE, REQUIRED_RUNTIME_HASH, RT_GLOBAL, RT_HEAD, RT_IMPORT_CONTENT, RT_MEM,
    RT_N_IMPORTS, RT_TAIL, RT_TAIL_PREFIX_LEN, RUNNABLE_ENVELOPE_TAIL,
};

// ─── Compile errors: decline (not-yet-compiled) vs reject (a type error) ─────────────

/// A reason the compiler did not produce a component. Either a **decline** — a construct it
/// does not yet lower, read by the gate as `todo` (never a disagreement) — or a **reject** —
/// a program the type system refuses, carrying the machine-readable diagnostic code
/// (constitution VII: a well-typed program is required before a component is emitted;
/// Amendment 0.4.0 makes the seed compiler enforce this rather than defer it).
#[derive(Debug, Clone)]
pub struct Decline(pub String, pub Option<String>);

impl Decline {
    /// The diagnostic code, if this is a type rejection rather than a plain decline.
    pub fn code(&self) -> Option<&str> {
        self.1.as_deref()
    }
    /// The human-readable message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Decline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.1 {
            Some(code) => write!(f, "rejected {code}: {}", self.0),
            None => write!(f, "declined: {}", self.0),
        }
    }
}

fn decline<T>(msg: impl Into<String>) -> Result<T, Decline> {
    let msg = msg.into();
    // DEV-desk tracing (ask-50): every decline funnels through here, so one instrumentation point
    // logs "why did it decline" for all 240 call sites. Emits nothing unless built `--features trace`
    // (the wasm-component build never enables it, so its bytes are unchanged). `target: "cdz::decline"`
    // lets an operator filter to just the decline stream (`CADENZA_TRACE=cdz::decline=debug`).
    #[cfg(feature = "trace")]
    tracing::debug!(target: "cdz::decline", %msg, "declined");
    Err(Decline(msg, None))
}

/// Reject a program as ill-typed with the diagnostic `code` (constitution XI: machine-
/// readable). Distinct from `decline` — a rejection is a definite compile-time type error the
/// gate checks against the case's `(compiler …)` clause, not a not-yet-implemented gap.
fn reject<T>(code: &str, msg: impl Into<String>) -> Result<T, Decline> {
    let msg = msg.into();
    // DEV-desk tracing (ask-50): the reject twin of the decline instrumentation above.
    #[cfg(feature = "trace")]
    tracing::debug!(target: "cdz::reject", %code, %msg, "rejected");
    Err(Decline(msg, Some(code.to_string())))
}

// ─── Kinds and wasm value types ──────────────────────────────────────────────────

/// The scalar kind of a compiled expression's wasm result. The seed realizes the scalar
/// core the corpus exercises without host imports; compound values (String, List, Bytes,
/// Record, Sum, Tuple) are a later concern and are declined for now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int64,
    Bool,
    Float64,
    Unit,
    /// A divergent expression — one that always traps (`unreachable`). It has no value and
    /// unifies with any expected kind, because wasm's `unreachable` is stack-polymorphic and
    /// validates against any result type. A function whose whole body is `Never` is emitted
    /// with an arbitrary (Int64) result type; its body just traps at runtime.
    Never,
    /// A runtime compound value living on the linear-memory heap, represented as an i32 pointer
    /// to its heap object (M2). It never crosses the component boundary directly — a compound
    /// result is presented as the `cadenza:run/run` resource whose `display` walks the heap — so
    /// it has a core valtype (i32) but no `comp_valtype`.
    Heap,
    /// A `string` at the HOST BOUNDARY only — the type of a host import's string parameter/result
    /// (host-interface-binding.md §A Host Import Is A WIT-Typed Function). At the core level it is
    /// lowered as a `(ptr, len)` pair, so it is not a single core valtype; it appears only in a
    /// `HostImport` signature, never as an ordinary expression kind, and the emitter handles its
    /// (ptr,len) lowering at the call site. Distinct from a runtime heap String value.
    HostString,
}

impl Kind {
    /// The core wasm valtype byte (used in function signatures and local declarations).
    /// Bool is an i32 in core wasm; Unit has no representation (empty result).
    fn core_valtype(self) -> u8 {
        match self {
            Kind::Int64 | Kind::Never => 0x7E, // i64 (Never: arbitrary; body traps)
            Kind::Bool => 0x7F,                // i32
            Kind::Float64 => 0x7C,             // f64
            Kind::Unit => 0x40,                // empty block type (no value)
            Kind::Heap => 0x7F,                // i32 heap pointer
            // A host-boundary string is a (ptr,len) pair, not a single core valtype. It never
            // appears as a function's core signature valtype — the call-site emitter expands it
            // to two i32s. Reaching here would be a bug; give it the pointer's i32.
            Kind::HostString => 0x7F,
        }
    }

    /// The component-model primitive valtype byte the run export presents at the boundary.
    fn comp_valtype(self) -> u8 {
        match self {
            Kind::Int64 | Kind::Never => 0x78, // s64
            Kind::Bool => 0x7F,                // bool
            Kind::Float64 => 0x75,             // f64
            Kind::Unit => 0x40,                // (unused; unit uses a distinct envelope)
            Kind::Heap => 0x40,                // (unused; a compound uses the resource envelope)
            Kind::HostString => 0x73,          // string (host boundary only)
        }
    }

    /// The concrete kind to present externally: a purely-divergent result is reported as
    /// Int64 (the body traps, so the value is never produced).
    fn externalized(self) -> Kind {
        match self {
            Kind::Never => Kind::Int64,
            k => k,
        }
    }

    /// Unify two kinds where one may be `Never` (divergent). Two concrete kinds unify only
    /// if equal.
    fn unify(a: Kind, b: Kind) -> Option<Kind> {
        match (a, b) {
            (Kind::Never, k) | (k, Kind::Never) => Some(k),
            (x, y) if x == y => Some(x),
            _ => None,
        }
    }
}

// ─── wasm opcodes (the subset this compiler emits) ─────────────────────────────────

// `mod op` is GENERATED into `op.rs` by `xtask build` from xtask/src/opcodes.rs — each opcode byte
// is derived by encoding a `wasm_encoder::Instruction` (the authoritative source of the spec's
// opcode numbers), and the SAME table is emitted as `compiler/op.cdz` so both compiler
// implementations share one opcode table. Edit the curated list in xtask, not `op.rs`.
#[path = "op.rs"]
mod op;

// ─── LEB128 and float encoding ─────────────────────────────────────────────────────

/// Unsigned LEB128.
fn uleb128(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            break;
        } else {
            out.push(byte | 0x80);
        }
    }
}

/// Signed LEB128 for an i64 (used by `i64.const`).
fn sleb128(mut n: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7; // arithmetic shift preserves sign
        let sign_bit_set = byte & 0x40 != 0;
        if (n == 0 && !sign_bit_set) || (n == -1 && sign_bit_set) {
            out.push(byte);
            break;
        } else {
            out.push(byte | 0x80);
        }
    }
}

fn uleb_bytes(n: u64) -> Vec<u8> {
    let mut v = Vec::new();
    uleb128(n, &mut v);
    v
}

// ─── Section / vector builders ─────────────────────────────────────────────────────

/// A wasm section: `<id> <byte-length-uleb> <contents>`.
fn section(id: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(contents.len() + 5);
    out.push(id);
    uleb128(contents.len() as u64, &mut out);
    out.extend_from_slice(contents);
    out
}

/// A wasm vector: `<count-uleb> <items concatenated>`.
fn wasm_vec(count: usize, items: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len() + 5);
    uleb128(count as u64, &mut out);
    out.extend_from_slice(items);
    out
}

// ─── Function model ──────────────────────────────────────────────────────────────

/// A user function collected from the module: its name, parameter names, body, and (once
/// synthesized) its parameter and return kinds. `index` is its wasm function index.
struct Func {
    name: String,
    params: Vec<String>,
    body: Node,
    index: u32,
    param_kinds: Vec<Kind>,
    ret_kind: Kind,
}

/// A compiled function body: the extra locals it declares (beyond params, in index order)
/// and its instruction bytes (without the trailing `end`).
struct Body {
    extra_locals: Vec<Kind>,
    code: Vec<u8>,
}

/// Which overflow-checked arithmetic helpers a module needs. Each is emitted as its own
/// wasm function `(i64, i64) -> i64` that traps (`unreachable`) on signed overflow, so the
/// call sites stay small and never collide on scratch locals.
#[derive(Default, Clone, Copy)]
struct Helpers {
    add: bool,
    sub: bool,
    mul: bool,
}

// ─── Public entry ─────────────────────────────────────────────────────────────────

/// Compile a program's canonical binary AST to a complete WebAssembly **component**. This
/// is the `compile : list<u8> -> list<u8>` seam — the entry the wasm-component build of
/// cdz-rustc exports, and the same ABI the Cadenza-authored compiler exports. The native
/// gate harness calls `compile_program` directly (it holds the tree), so this is unused in
/// a native build but is the load-bearing entry for the component build.
#[allow(dead_code)]
pub fn compile(ast_bytes: &[u8]) -> Result<Vec<u8>, Decline> {
    let node = ast::decode(ast_bytes).map_err(|e| Decline(format!("binary AST decode: {e}"), None))?;
    compile_program(&node)
}

/// Compile an already-parsed program `Node`. Used by the gates, which hold the tree in
/// memory. Expects `(module <name> <form>…)`.
pub fn compile_program(node: &Node) -> Result<Vec<u8>, Decline> {
    let items = match node {
        Node::List(items) => items,
        _ => return decline("program is not a module form"),
    };
    if name_of(items.first()) != Some("module") {
        return decline("program is not a (module …) form");
    }
    if items.len() < 2 {
        return decline("module has no name");
    }
    let forms = &items[2..];

    let mut compiler = Compiler::new(forms)?;
    compiler.compile_module()
}

// ─── The compiler ─────────────────────────────────────────────────────────────────

/// The prelude's sum-type declarations, in Cadenza `(type …)` source. Option and Result are
/// ORDINARY library sum types, declared here rather than special-cased in the compiler
/// (constitution IX: the compiler encodes no behavior of its own). The compiler reads these
/// the same way it reads a program's own `(type …)` forms; when a real prelude source file
/// exists this constant is replaced by reading it.
///
/// `Ast` is likewise an ORDINARY sum type — "a variant per syntactic form" (type-system.md #The
/// Abstract Syntax Tree Type Is An Ordinary Sum Type), one whose constructors carry TYPED payloads
/// (`Ast.Int` an Int64, `Ast.Name`/`Ast.Str` a String, `Ast.List` a list of children). Declaring it
/// here — rather than special-casing the `Ast.*` constructors — gives its variants the same
/// payload-type machinery (`sum_payload_types`) every other sum has, so `(Ast.Int "x")` is rejected
/// as a wrong-type payload EXACTLY as a user `(T.Mk "x")` is, with no `Ast`-specific code. The
/// variants and payloads mirror `quote_to_ast`'s form-to-constructor map: `Int Int64`, `Float
/// Float64`, `Str String`, `Bool Bool`, `Name String`, `List (List Ast)`.
const PRELUDE_TYPES: &str = "\
    (type Option (Some a | None))\n\
    (type Result (Ok a | Err e))\n\
    (type Sign (Neg | Zero | Pos))\n\
    (type Ast (Int Int64 | Float Float64 | Str String | Bool Bool | Name String | List (List Ast)))\n";

struct Compiler {
    funcs: Vec<Func>,
    helpers: Helpers,
    /// Function index of each helper, once assigned (after the user functions).
    helper_add_idx: u32,
    helper_sub_idx: u32,
    helper_mul_idx: u32,
    /// Variant tag → its declared sum type's name, built from the prelude's and the program's
    /// `(type …)` declarations. Lets the compiler decide whether two sum values belong to the
    /// same type (comparable) or different types (a comparison type error) by DECLARATION, not
    /// by hardcoded variant names.
    sum_types: std::collections::BTreeMap<String, String>,
    /// The tags of the NULLARY variants (declared as a bare name `None`/`Zero`, not `(V pay)`).
    /// A nullary variant's argument type is Unit, so applying it to a non-unit payload is a type
    /// error (CDZ0201) — this set is how `check_type_rejections` recognizes such a variant.
    nullary_variants: std::collections::BTreeSet<String>,
    /// Sum type name → its variants in DECLARATION order. A variant's DISCRIMINANT is its index in
    /// this list — the small per-sum integer the runtime stores (`sum-new`/`sum-disc`) and the
    /// compiler-emitted renderer switches on to recover the variant name. Built alongside
    /// `sum_types` from every `(type …)` declaration.
    sum_variants: std::collections::BTreeMap<String, Vec<String>>,
    /// Variant tag → the SCALAR kinds of its payload's tuple slots, for a runtime match to know
    /// which payload slots to unbox (Int64/Bool/Float64 → `get-int`/`get-bool`/`get-float`) and
    /// which to keep as an opaque heap handle (a nested sum/tuple/list → `Kind::Heap`). Parsed from
    /// the declared payload type: `(Cons (Tuple Int64 IntList))` → `[Int64, Heap]`; a single
    /// non-tuple payload `(Ok a)` → `[<kind of a>]`; a nullary variant → `[]`. A type name the
    /// parser does not recognize as a scalar (a user sum type, a type parameter) → `Kind::Heap`
    /// (an opaque handle). Only consulted on the runtime-heap consumption path; the const path
    /// resolves binders structurally and never reads this.
    sum_payload_kinds: std::collections::BTreeMap<String, Vec<Kind>>,
    /// Variant tag → its payload's per-slot TYPE nodes (the same slots `sum_payload_kinds` records
    /// as flat kinds). This preserves the full structure a flat `Kind` erases — a nested `(Tuple …)`
    /// slot keeps its element types — so a runtime match binding a NESTED tuple binder
    /// (`(Ctor (tuple op (tuple a b)))`) can unbox each inner scalar by its declared type. Parsed
    /// from the same declaration as `sum_payload_kinds`; a slot whose type is itself `(Tuple …)`
    /// keeps that node, so `bind_sum_payload` recurses into it. Only the runtime-heap match path
    /// reads this; the const path resolves binders structurally.
    sum_payload_types: std::collections::BTreeMap<String, Vec<Node>>,
    /// The offset added to every emitted `call` target index. 0 for the ordinary self-contained
    /// component (defined functions start at wasm index 0). `RT_FUNC_BASE` for the runtime-compound
    /// component, whose defined functions follow the 11 heap imports + 4 fixed helper funcs — so a
    /// user function's wasm index is `call_base + its 0-based index`. Set in `compile_module` once
    /// `main`'s result is known to be a runtime heap value. It shifts only emitted call bytes; a
    /// function's return KIND is invariant to it (so it is safe to observe a body's kind before
    /// fixing the base).
    call_base: u32,
    /// The effects the program declares via `(effect Name (op …) …)` — a routing-agnostic contract
    /// per effect (capabilities-and-effects.md §An Effect Declaration Names The Effect And Types Its
    /// Operations). Consulted by `gen_perform` (is `E.op` a declared operation?), by `gen_handle`
    /// (does an arm name an operation `E` declares — else CDZ0403?), and by `gen_host` (a delegated
    /// operation's WIT signature is its declared `(-> T… R)`). Says nothing about routing.
    effects: std::collections::BTreeMap<String, EffectDecl>,
    /// The host functions the emitted component imports — the program's manifest. COMPUTED (not
    /// declared) from the entrypoint's `(host …)` delegation: each delegated effect's operations
    /// become boundary imports (capabilities-and-effects.md §The Program Manifest Is The Union Of
    /// Its Entrypoints' Delegations). Populated by `compile_module` (which holds `&mut self`) BEFORE
    /// the emit pass, so host funcs occupy the low core-func indices and `call_base` shifts the user
    /// functions past them; the emit pass only READS it. Each import's name is the flat
    /// `effect.op` string (the boundary call the host records). NOT read from a declaration or a
    /// `(use …)` — those surfaces are retired.
    host_imports: Vec<HostImport>,
    /// Effect-context specializations of RECURSIVE effectful functions (Stage 3 — effect-context
    /// monomorphization, options/effects-model/lowering-to-wasm.md §Effect-context monomorphization).
    /// A recursive function that performs an effect cannot be discharged by inlining (its body would
    /// inline without bound), so it is emitted ONCE PER HANDLER CONTEXT as a real wasm function whose
    /// enclosing handler states are threaded as hidden trailing parameters and returned as extra
    /// results (evidence passing — each handler context gets its OWN state on the call stack, so
    /// nested/wrapped effects compose without a global's single-slot clobber). Populated lazily
    /// during emission (a `RefCell` because `emit` holds `&self`); the specialized bodies are
    /// appended to the module after the user functions. See `Specialization`.
    specializations: std::cell::RefCell<Vec<Specialization>>,
    /// The static `Shape` of the `compile` entry's single parameter, when it takes the kinded-artifact
    /// ABI's `list<artifact>` input (ask-41). Set by `compile_component_module` once it detects the
    /// artifact ABI; read by `compile_func` to give the compile entry's `inputs` parameter that shape,
    /// so `shape_of`/`gen_runtime_member` can see through the opaque `Heap` handle and project fields
    /// out of a projected input artifact (`(. (List.at inputs 0) bytes)`). `None` on every other path.
    compile_input_shape: Option<Shape>,
    /// Top-level VALUE definitions `(def name value)` — a name bound to an ordinary expression at
    /// module scope, usable by every sibling function (core-semantics.md #A Module Evaluates To A
    /// Record Of Its Exports: each `def` registers its name; a value-def registers a value field).
    /// These are the shared data tables a self-hosted compiler carries (the `@generated` opcode
    /// record `(def op (record …))`, ask-71). Bound as compile-time ALIASES prepended to each
    /// function's emit env — so a use folds/resolves exactly as a `let`-bound value does (a literal /
    /// record-of-literals folds; a `(. name field)` projects). Collected in `Compiler::new`; a
    /// value-def is NOT a function (no entry in `funcs`).
    module_values: Vec<(String, Node)>,
}

/// One effect-context specialization of a recursive effectful function: the function emitted as a
/// real wasm function under a fixed handler context, with the enclosing handlers' states threaded
/// as hidden trailing params/returns. Its wasm function index is `spec_base + its position` in the
/// registry.
struct Specialization {
    /// The user function being specialized.
    fn_name: String,
    /// A stable key identifying the handler context this copy is specialized under — the list of
    /// enclosing `(effect, arm-body-fingerprint, state_kind)` the function's performs resolve to,
    /// innermost last. Two call sites under the same context share one specialization.
    key: String,
    /// The state kinds threaded through this specialization, one per enclosing handler context that
    /// discharges an effect the function reaches (innermost last) — the hidden trailing params and
    /// extra returns. A `Unit` state contributes nothing (zero-width).
    state_kinds: Vec<Kind>,
    /// The compiled body (filled once emission of this specialization completes). `None` while the
    /// specialization is being emitted (the placeholder that also breaks self-recursion in the
    /// registry — a self-call finds the reserved slot and emits a `call` to it).
    body: std::cell::RefCell<Option<Body>>,
    /// This specialization's result kind (the original function's return kind).
    ret_kind: Kind,
    /// The original function's parameter kinds (before the trailing state params).
    param_kinds: Vec<Kind>,
}

impl Compiler {
    fn new(forms: &[Node]) -> Result<Compiler, Decline> {
        // Effect declarations (`(effect Name (op …) …)`) — routing-agnostic contracts. The legacy
        // `(import (host …))` / `(use (capability …))` surfaces are RETIRED (the manifest is now
        // computed from an entrypoint's `(host …)` delegation as it is lowered, not read from a
        // declaration).
        let effects = collect_effects(forms);
        // An effect's operations are a CLOSED, statically-known SET, each name bound to one operation
        // type (capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
        // Operations). Declaring an op name twice — `(effect E (op f …) (op f …))` — makes the set
        // ill-defined, the SAME ill-formedness a duplicate record field (`(record (a 1) (a 2))`) or a
        // duplicate module definition is rejected for (CDZ0201, 14-effects-and-handlers.sexp §"an
        // effect that declares an operation name twice is rejected"); a fixed set cannot name the same
        // member twice, so it is NOT resolved by keeping one. Checked here (over the collected ops per
        // effect), the effect-declaration sibling of the module-def and record-field duplicate checks.
        for (ename, decl) in &effects {
            for i in 0..decl.ops.len() {
                for j in (i + 1)..decl.ops.len() {
                    if decl.ops[i].name == decl.ops[j].name {
                        return reject(
                            "CDZ0201",
                            format!("effect `{ename}` declares operation `{}` more than once", decl.ops[i].name),
                        );
                    }
                }
            }
        }

        // Collect defs. `main` becomes function index 0 (the sole `run` export); the rest
        // follow in source order so named / recursive / mutually-recursive calls resolve. A top-level
        // VALUE def `(def name value)` is collected separately into `module_values` — it binds a
        // module-scope name (not a function), usable by every sibling function (ask-71).
        let mut raw: Vec<(String, Vec<String>, Node)> = Vec::new();
        let mut module_values: Vec<(String, Node)> = Vec::new();
        for form in forms {
            match parse_def(form)? {
                Some(Def::Func(name, params, body)) => raw.push((name, params, body)),
                Some(Def::Value(name, value)) => module_values.push((name, value)),
                None => {}
            }
        }
        // A module evaluates to a RECORD of its exports — each `(def name …)` registers `name` as a
        // field — and a record has a FIXED SET of field names (core-semantics.md #A Record Has A Fixed
        // Set Of Named Fields; #A Module Evaluates To A Record Of Its Exports). So two definitions of
        // the same name register one field twice — the same ill-formedness `(record (a 1) (a 2))` is
        // rejected for (CDZ0201), NOT resolved by a first-wins precedence the fixed field set forbids
        // (11-modules.sexp §"a module with two definitions of the same name is rejected"). Detected
        // here, over the whole module's defs, before the entrypoint is chosen or any body compiled.
        // The FULL module namespace is functions + value-defs (both register a field); a name defined
        // twice across either — two functions, two values, or a function and a value — is the same
        // duplicate-member error. Collect all defined names in source order and scan for a duplicate.
        let all_def_names: Vec<&String> =
            raw.iter().map(|(n, _, _)| n).chain(module_values.iter().map(|(n, _)| n)).collect();
        for i in 0..all_def_names.len() {
            for j in (i + 1)..all_def_names.len() {
                if all_def_names[i] == all_def_names[j] {
                    return reject(
                        "CDZ0201",
                        format!("module defines `{}` more than once", all_def_names[i]),
                    );
                }
            }
        }
        // A sum type's variant names are a SET (type-system.md #The Structural Types Are Record,
        // Tuple, And Sum: a sum's shape is its variant names with their payload types), so a `(type T
        // (A Int64 | A Bool))` declaring `A` twice makes the variant set ill-defined and is rejected
        // (CDZ0201) — the fourth closed name-set duplicate check beside record fields, module defs,
        // and effect ops (05-compound-types.sexp §"a sum declaring a variant name twice is a type
        // error"). Checked over the program's own `(type …)` forms (the prelude is well-formed by
        // construction); a duplicate WITHIN one declaration only — two different types reusing a
        // variant name is the allowed last-writer-wins reuse-override the `Expr.Neg`/`Sign.Neg` case
        // exercises. Runs before `collect_sum_types` registers the variants (which would silently bind
        // the duplicate tag with two payload types — the ambiguity the closed set forbids).
        if let Some((ty, variant)) = first_duplicate_variant_in_a_sum(forms) {
            return reject(
                "CDZ0201",
                format!("sum type `{ty}` declares variant `{variant}` more than once"),
            );
        }
        // The entrypoint is `main` (the nullary `run` export) OR `compile` (the `bytes → bytes`
        // `cadenza:compiler/compile` export — bootstrap.md §"The Compiler Is Authored In Cadenza": a
        // Cadenza-authored compiler exports `compile : list<u8> -> list<u8>`, driven by the host's
        // `component-check` harness over the whole corpus). Whichever is present becomes function
        // index 0. `main` wins if both are declared (a program with both is exercised via `main`).
        let entry_pos = raw
            .iter()
            .position(|(n, _, _)| n == "main")
            .or_else(|| raw.iter().position(|(n, _, _)| n == "compile"));
        let entry_pos = match entry_pos {
            Some(p) => p,
            None => return decline("module has no (def (main) …) or (def (compile b) …) entrypoint"),
        };
        // entry first, then the others in order.
        let mut ordered: Vec<(String, Vec<String>, Node)> = Vec::new();
        ordered.push(raw[entry_pos].clone_tuple());
        for (i, d) in raw.iter().enumerate() {
            if i != entry_pos {
                ordered.push(d.clone_tuple());
            }
        }

        // Determine which overflow helpers the whole module needs.
        let mut helpers = Helpers::default();
        for (_, _, body) in &ordered {
            scan_helpers(body, &mut helpers);
        }

        // Helpers follow the user functions in index space, in add/sub/mul order.
        let n_user = ordered.len() as u32;
        let mut next = n_user;
        let mut alloc = |want: bool| {
            if want { let i = next; next += 1; i } else { 0 }
        };
        let helper_add_idx = alloc(helpers.add);
        let helper_sub_idx = alloc(helpers.sub);
        let helper_mul_idx = alloc(helpers.mul);

        // Seed every user function's return kind as Int64 (all realized corpus functions
        // are Int64 → Int64); refine each precisely from its own body below. Cross-function
        // and recursive calls read the seeded kind, which is correct for the corpus.
        let mut funcs: Vec<Func> = ordered
            .into_iter()
            .enumerate()
            .map(|(i, (name, params, body))| {
                let param_kinds = vec![Kind::Int64; params.len()];
                Func { name, params, body, index: i as u32, param_kinds, ret_kind: Kind::Int64 }
            })
            .collect();

        // Build the variant → sum-type map from the prelude's and the program's `(type …)`
        // declarations, so sum-type identity is data-driven (not hardcoded variant names).
        let mut sum_types = std::collections::BTreeMap::new();
        let mut nullary_variants = std::collections::BTreeSet::new();
        let mut sum_variants = std::collections::BTreeMap::new();
        let mut sum_payload_kinds = std::collections::BTreeMap::new();
        let mut sum_payload_types = std::collections::BTreeMap::new();
        if let Ok(prelude) = ast::read_all(PRELUDE_TYPES) {
            collect_sum_types(
                &prelude,
                &mut sum_types,
                &mut nullary_variants,
                &mut sum_variants,
                &mut sum_payload_kinds,
                &mut sum_payload_types,
            );
        }
        collect_sum_types(
            forms,
            &mut sum_types,
            &mut nullary_variants,
            &mut sum_variants,
            &mut sum_payload_kinds,
            &mut sum_payload_types,
        );

        let mut compiler = Compiler {
            funcs: Vec::new(),
            helpers,
            helper_add_idx,
            helper_sub_idx,
            helper_mul_idx,
            sum_types,
            nullary_variants,
            sum_variants,
            sum_payload_kinds,
            sum_payload_types,
            call_base: 0,
            effects,
            host_imports: Vec::new(),
            specializations: std::cell::RefCell::new(Vec::new()),
            compile_input_shape: None,
            module_values,
        };
        // Move funcs in, then refine each function's return kind to a FIXPOINT. Kinds have
        // ONE source of truth — `emit` — so a function's return kind is what emitting its body
        // yields. A caller reads its callee's kind, so a single pass in definition order can
        // read a not-yet-refined callee (e.g. `main` calling a Bool-returning predicate
        // defined after it, seeing the seeded Int64 and mistyping its own signature). Iterate
        // to a fixpoint so every caller sees its callees' final kinds; bounded by the function
        // count since each pass can only refine kinds monotonically toward stability.
        compiler.funcs = std::mem::take(&mut funcs);
        // Infer parameter and return kinds by UNIFICATION over the program (Hindley-Milner /
        // Algorithm W — Cadenza is an ML+LISP+Rust hybrid; inference is principled, not ad-hoc
        // call-site guessing). Kinds flow from how each name is used: a parameter used in a
        // Bool position unifies to Bool. See `infer::infer_kinds`.
        compiler.infer_kinds();
        Ok(compiler)
    }

    // ─── Type inference (Hindley-Milner over the ground-kind lattice) ────────────────
    //
    // Each function's parameter and return kinds are inferred by UNIFICATION: a parameter
    // starts as a type variable and is unified against the kinds its uses require (a `Bool`
    // condition forces its scrutinee to Bool, an arithmetic operand forces Int64, both `if`
    // branches unify to one kind, …). Cross-function calls read the callee's current
    // parameter/return kinds, so the whole program is inferred to a fixpoint. This replaces
    // the earlier "seed every parameter Int64" stopgap (which declined Bool/Float parameters);
    // it is the ML-style inference the spec pins (type-system.md §Inference,
    // spec/learnings/2026-07-04-inference-is-hindley-milner.md). `Kind` is the current
    // monomorphic ground lattice; full HM adds first-class type variables over the structural
    // type universe.

    /// Infer every function's parameter and return kinds by unification, iterating to a
    /// fixpoint so callers observe their callees' solved kinds.
    fn infer_kinds(&mut self) {
        let n = self.funcs.len();
        for _ in 0..=(n + 1) {
            let mut changed = false;
            for i in 0..n {
                let (params, ret) = self.infer_one(i);
                if self.funcs[i].param_kinds != params || self.funcs[i].ret_kind != ret {
                    self.funcs[i].param_kinds = params;
                    self.funcs[i].ret_kind = ret;
                    changed = true;
                }
            }
            // ARGUMENT → CALLEE-PARAM propagation (the reverse of `infer_one`'s callee-param →
            // argument direction). `infer_one` constrains each caller's ARGUMENT to the callee's
            // current param kind; it does NOT tell the callee that a call site passes a Heap value.
            // So a callee whose parameter is only RETURNED / THREADED (never used in a
            // kind-forcing op) — e.g. `iterate`'s `ktab`, returned in the base branch and re-passed
            // a freshly-built `(list)` in the recursive branch — stays at its Int64 default. At
            // emit the Heap argument then mismatches the Int64 param and `gen_call` INLINES the
            // (recursive) callee, re-expanding without bound → the compile-cost blowup (GAP 3m).
            // Walk each body, infer each user-fn call's argument kinds in the caller's context, and
            // UPGRADE the callee's param kind toward Heap (the same "Heap beats scalar" tie-break
            // `constrain` uses). Runs inside the fixpoint so it converges: once `ktab : Heap`, the
            // recursive call's Heap argument MATCHES and emits a real `call`, not an inline.
            for i in 0..n {
                let ups = self.collect_arg_param_upgrades(i);
                for (callee, pos, kind) in ups {
                    if let Some(pk) = self.funcs[callee as usize].param_kinds.get_mut(pos) {
                        if *pk != Kind::Heap && kind == Kind::Heap {
                            *pk = Kind::Heap;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Collect `(callee-index, param-position, argument-kind)` upgrades from function `i`'s body:
    /// for every call to a user function, the kind each argument infers to in `i`'s context. Used
    /// by `infer_kinds` to push an argument's Heap kind onto the callee's parameter (the arg →
    /// callee-param direction of unification, which `infer_one` omits). Only Heap upgrades are
    /// applied by the caller, matching the coarse lattice's "Heap is more defined than a scalar
    /// default" rule — so a genuinely-scalar parameter is untouched and a threaded compound
    /// accumulator converges to Heap.
    fn collect_arg_param_upgrades(&self, i: usize) -> Vec<(u32, usize, Kind)> {
        // Seed the caller's params at their CURRENT solved kinds so an argument that is a bare
        // parameter reference (`(iterate funcs ktab …)` — passing `ktab` along) reports that kind.
        let vars: Vec<(String, Option<Kind>)> = self.funcs[i]
            .params
            .iter()
            .zip(self.funcs[i].param_kinds.iter())
            .map(|(p, pk)| (p.clone(), Some(*pk)))
            .collect();
        let mut ictx = InferCtx { compiler: self, vars };
        let mut out = Vec::new();
        let body = self.funcs[i].body.clone();
        self.walk_call_args(&body, &mut ictx, &mut out);
        out
    }

    /// Recursively walk `node`, appending `(callee-index, param-pos, arg-kind)` for every call to a
    /// user function. `ictx` supplies the enclosing function's variable kinds so an argument's kind
    /// is inferred in context. Binds `let` names as it descends so a let-bound argument infers too.
    fn walk_call_args(&self, node: &Node, ictx: &mut InferCtx, out: &mut Vec<(u32, usize, Kind)>) {
        let items = match node {
            Node::List(items) => items,
            _ => return,
        };
        if let Some(Node::Name(head)) = items.first() {
            // Bind `let` names before descending into the body so a name bound to a compound
            // infers Heap in the body's calls.
            if head == "let" && items.len() >= 2 {
                if let Some(Node::List(binds)) = items.get(1) {
                    for pair in binds {
                        if let Node::List(kv) = pair {
                            if let (Some(Node::Name(name)), Some(vexpr)) = (kv.first(), kv.get(1)) {
                                let k = ictx.infer(vexpr);
                                ictx.vars.push((name.clone(), k));
                            }
                        }
                    }
                }
            }
            // A call to a user function: record each argument's inferred kind against the callee's
            // parameter position.
            if let Some(f) = self.lookup_fn(head) {
                let callee = f.index;
                for (pos, arg) in items[1..].iter().enumerate() {
                    if let Some(k) = ictx.infer(arg) {
                        out.push((callee, pos, k));
                    }
                }
            }
        }
        // Descend into every child (arguments, branches, arms, …) to catch nested calls.
        for child in items {
            self.walk_call_args(child, ictx, out);
        }
    }

    /// Infer function `i`'s (parameter kinds, return kind). Parameters begin as type variables
    /// (`None`); the body walk unifies them against required kinds; any left unconstrained
    /// default to Int64 (the ground default for the realized corpus).
    fn infer_one(&self, i: usize) -> (Vec<Kind>, Kind) {
        let arity = self.funcs[i].params.len();
        // Params begin as type variables (`None`) so the body re-derives each kind fresh every
        // fixpoint pass — EXCEPT a param already solved to `Heap` (the "more defined" kind), which
        // is pre-seeded from `param_kinds`. That Heap may have been established by ARGUMENT →
        // callee-param propagation (a call site passing this param a runtime compound), which the
        // body itself does not witness — e.g. `iterate`'s `ktab`, only returned and threaded. Pre-
        // seeding it Heap lets the return-kind inference SEE it (so the base branch `ktab` reads
        // Heap and the `if`-branch re-read converges the return to Heap). A SCALAR param stays
        // `None` so a Bool/Float refinement from the body still applies (seeding Int64 would lock it
        // by first-write-wins); Heap never needs that — it only ever upgrades, never downgrades.
        let seed = |k: Kind| if k == Kind::Heap { Some(Kind::Heap) } else { None };
        let vars: Vec<(String, Option<Kind>)> = self.funcs[i]
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (p.clone(), self.funcs[i].param_kinds.get(j).copied().and_then(seed)))
            .collect();
        let mut ictx = InferCtx { compiler: self, vars };
        let body = self.funcs[i].body.clone();
        let ret = ictx.infer(&body).unwrap_or(Kind::Int64);
        // Only the first `arity` variables are the parameters; the walk may have appended
        // let-bound names to the same environment (used for lookup), which are not parameters.
        let params = ictx.vars[..arity].iter().map(|(_, k)| k.unwrap_or(Kind::Int64)).collect();
        (params, ret)
    }

    fn lookup_fn(&self, name: &str) -> Option<&Func> {
        self.funcs.iter().find(|f| f.name == name)
    }

    /// Infer the static `Shape` of an expression that produces a runtime heap value — the
    /// type-directed information the tag-free renderer needs (field/variant names, nesting). This
    /// MIRRORS the emitter's resolution: a `(tuple …)`/`(list …)`/`(record …)` form yields its
    /// structural shape; a call inlines the callee (substituting argument shapes for its
    /// parameters, matching `gen_call`'s per-call monomorphization); `if`/`match`/`let`/`do` follow
    /// their result form; a scalar leaf comes from its `Kind`. Returns `None` when the shape is not
    /// locally determinable or contains a not-yet-renderable leaf (runtime float/string) — the
    /// caller then declines (decline-don't-miscompile), it does not emit a wrong renderer.
    fn shape_of(&self, node: &Node, env: &[Local]) -> Option<Shape> {
        self.shape_of_guarded(node, env, &mut Vec::new())
    }

    /// `shape_of` with an inline-recursion guard: `stack` holds the names of the functions
    /// currently being inlined to determine a shape. Inlining a function already on the stack means
    /// the value's type is RECURSIVE (a linked list / tree / AST built by a self-recursive
    /// function), whose static `Shape` is infinite — the tree-shaped renderer cannot represent it,
    /// so this returns `None` (the caller declines: decline-don't-miscompile). Without the guard,
    /// inlining a recursive builder either overflows the compiler stack or under-approximates the
    /// shape (assigning an unbuilt variant a placeholder leaf), which mis-renders the value.
    fn shape_of_guarded(
        &self,
        node: &Node,
        env: &[Local],
        stack: &mut Vec<String>,
    ) -> Option<Shape> {
        match node {
            Node::Int(_) => Some(Shape::Int),
            Node::Bool(_) => Some(Shape::Bool),
            Node::Float(_) => Some(Shape::Float),
            // A string literal is a runtime String value (a Bytes-backed UTF-8 leaf); its shape is
            // `Str`, which the renderer quotes/escapes as `"…"` (distinct from `Bytes`' `b"…"`).
            Node::Str(_) => Some(Shape::Str),
            Node::Name(n) if n == "unit" => Some(Shape::Unit),
            // An empty application `()` is the unit value (like `eval_const` / `gen_list`).
            Node::List(elems) if elems.is_empty() => Some(Shape::Unit),
            Node::Name(n) => {
                // A local: an alias re-resolves; a materialized HEAP local carries its recorded
                // `Shape` (a `let`-bound runtime compound); a runtime scalar param yields its kind's
                // shape.
                let l = env.iter().rev().find(|l| l.name == *n)?;
                if let Some((anode, aenv)) = &l.alias {
                    let anode = anode.clone();
                    let aenv = aenv.clone();
                    self.shape_of_guarded(&anode, &aenv, stack)
                } else if let Some(s) = &l.shape {
                    Some(s.clone())
                } else {
                    Shape::from_kind(l.kind)
                }
            }
            Node::List(elems) => self.shape_of_list(elems, env, stack),
        }
    }

    fn shape_of_list(&self, elems: &[Node], env: &[Local], stack: &mut Vec<String>) -> Option<Shape> {
        let head = match elems.first() {
            Some(Node::Name(h)) => h.as_str(),
            // A QUALIFIED constructor head `(. Type Variant)` — `(IntList.Cons …)` — is a runtime
            // sum whose shape covers the whole (user) sum type; `sum_shape` qualifies the render
            // names. The full `Type.Variant` string drives both the discriminant (via its tag) and
            // the qualified render.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(2)).map_or(false, is_constructor_name) =>
            {
                let ty = name_of(hd.get(1))?;
                let v = name_of(hd.get(2))?;
                let qualified = format!("{ty}.{v}");
                return self.sum_shape(&qualified, elems.get(1), env, stack);
            }
            // A dotted intrinsic whose RESULT is a runtime Bytes value: `(Bytes.of …)`,
            // `(Bytes.concat …)`, `(Bytes.slice …)` (well, slice is fallible — Option), and
            // `(String.to-bytes …)`. Only the ones that produce a Bytes value directly get
            // `Shape::Bytes`; the renderer walks it via `bytes-len`/`bytes-get`. This is the
            // compiler's own output type flowing at run time.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("Bytes")
                    && matches!(name_of(hd.get(2)), Some("of") | Some("concat") | Some("compact")) =>
            {
                return Some(Shape::Bytes);
            }
            // A String op whose result is a runtime String — `(String.concat …)` — renders `"…"`
            // (`Shape::Str`). `(String.to-bytes …)` reinterprets the same leaf as `Bytes` (`b"…"`).
            // A String is a Bytes-backed leaf, so both walk the value via `bytes-len`/`bytes-get`.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("String")
                    && matches!(name_of(hd.get(2)), Some("concat")) =>
            {
                return Some(Shape::Str);
            }
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("String")
                    && matches!(name_of(hd.get(2)), Some("to-bytes")) =>
            {
                return Some(Shape::Bytes);
            }
            // A fallible String access — `(String.at s i)` / `(String.slice s a b)` — yields an
            // `Option<String>` (a one-scalar / sub-string). The renderer walks the Sum and its `Str`
            // payload; discriminant order matches `gen_runtime_string_at`'s Some/None.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("String")
                    && matches!(name_of(hd.get(2)), Some("at") | Some("slice")) =>
            {
                return self.option_shape(Shape::Str);
            }
            // A fallible Bytes access whose RESULT is a runtime `Option`: `(Bytes.at b i)` yields
            // `Option<Int>` (a byte), `(Bytes.slice b s n)` yields `Option<Bytes>`. The renderer
            // walks the Sum and its payload; the discriminant order matches what
            // `gen_runtime_bytes_at`/`gen_runtime_bytes_slice` emit (both from `sum_variants["Option"]`).
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("Bytes")
                    && matches!(name_of(hd.get(2)), Some("at") | Some("slice")) =>
            {
                let payload = if name_of(hd.get(2)) == Some("slice") {
                    Shape::Bytes
                } else {
                    Shape::Int // a byte
                };
                return self.option_shape(payload);
            }
            // A grown list value: `(List.push v e)` renders as `(list … e)`, so its element shape
            // comes from the pushed element `e` (homogeneous — every element shares it);
            // `(List.update v i e)` takes the element shape from `e`, else from the base list `v`;
            // `(List.concat a b)` is a list of the SAME element as either operand, so its element
            // shape comes from `a` (fall back to `b`). A grown list is the SAME `Shape::List` as a
            // `(list …)` literal — growth changes only the representation (a persistent tree vs a flat
            // array), not the type or the render.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("List")
                    && matches!(name_of(hd.get(2)), Some("push") | Some("update") | Some("concat")) =>
            {
                let elem = match name_of(hd.get(2)) {
                    // push(v, e): element shape is e's; v must also be a list of the same element.
                    Some("push") => elems.get(2).and_then(|e| self.shape_of_guarded(e, env, stack)),
                    // update(v, i, e): element shape from e, else from v.
                    Some("update") => elems
                        .get(3)
                        .and_then(|e| self.shape_of_guarded(e, env, stack))
                        .or_else(|| match elems.get(1).and_then(|v| self.shape_of_guarded(v, env, stack)) {
                            Some(Shape::List(inner)) => Some(*inner),
                            _ => None,
                        }),
                    // concat(a, b): element shape from a's list, else b's.
                    Some("concat") => {
                        let from = |i: usize, s: &mut Vec<String>| {
                            match elems.get(i).and_then(|v| self.shape_of_guarded(v, env, s)) {
                                Some(Shape::List(inner)) => Some(*inner),
                                _ => None,
                            }
                        };
                        from(1, stack).or_else(|| from(2, stack))
                    }
                    _ => None,
                };
                return Some(Shape::List(Box::new(elem?)));
            }
            // `(List.at v i)` yields an `Option<element>` (a runtime sum): the renderer walks the
            // Option and its payload, whose shape is the list operand's element shape. Mirrors the
            // `Bytes.at` → `Option<Int>` shape rule; the discriminant order matches `gen_runtime_list_at`.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("List")
                    && name_of(hd.get(2)) == Some("at") =>
            {
                let elem = match elems.get(1).and_then(|v| self.shape_of_guarded(v, env, stack)) {
                    Some(Shape::List(inner)) => *inner,
                    _ => return None,
                };
                return self.option_shape(elem);
            }
            // `(Int64.checked-add a b)` → `Option<Int>` (a runtime sum: `(Some sum)` / `(None unit)` on
            // overflow); `(Int64.wrapping-add a b)` → `Int`. Both operands are Int64 scalars.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("Int64")
                    && matches!(name_of(hd.get(2)),
                        Some("checked-add") | Some("checked-sub") | Some("checked-mul")) =>
            {
                return self.option_shape(Shape::Int);
            }
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1)) == Some("Int64")
                    && matches!(name_of(hd.get(2)),
                        Some("wrapping-add") | Some("wrapping-sub") | Some("wrapping-mul")) =>
            {
                return Some(Shape::Int);
            }
            // `(Option.expect o msg)` / `(Result.expect r msg)` yields the PRESENT variant's payload
            // (the trap path renders nothing), so its render shape is the scrutinee Option/Result's
            // `Some`/`Ok` payload shape. (Only consulted when `expect` returns a Heap handle — a
            // compound payload; a concretely-Int payload unboxes to a scalar and takes the
            // runtime-scalar path, which has no in-program renderer.)
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && matches!(name_of(hd.get(1)), Some("Option") | Some("Result"))
                    && name_of(hd.get(2)) == Some("expect") =>
            {
                let scrut = elems.get(1)?;
                let present = if name_of(hd.get(1)) == Some("Result") { "Ok" } else { "Some" };
                return match self.shape_of_guarded(scrut, env, stack)? {
                    Shape::Sum(variants) => variants
                        .into_iter()
                        .find(|(v, _)| variant_tag(v) == present)
                        .map(|(_, payload)| payload),
                    _ => None,
                };
            }
            // A PERFORM `(E.op …)` of a declared effect: its runtime value shape follows the op's
            // declared RESULT type. A `Diag.collect : Unit -> (List Int64)` read-out yields a list;
            // a `Fresh.next : Unit -> Int64` yields an int. The handler discharges it (its value is
            // the resumed value / threaded state), but the shape the renderer walks is fixed by the
            // declared result type, so it is determinable here without resolving the handler.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1))
                        .zip(name_of(hd.get(2)))
                        .map_or(false, |(e, o)| self.effects.get(e).map_or(false, |d| d.op(o).is_some())) =>
            {
                let e = name_of(hd.get(1))?;
                let o = name_of(hd.get(2))?;
                let op = self.effects.get(e)?.op(o)?;
                return shape_of_type_node(&op.result_type);
            }
            _ => return None,
        };
        match head {
            // A `(handle <init> (arms…) body)`: its value is its BODY's value, so its shape is the
            // body's shape (the accumulated state is observable only through the effect's own
            // operations, which resolve to their declared result shapes above).
            "handle" if elems.len() == 4 => self.shape_of_guarded(&elems[3], env, stack),
            // A `(do …)` yields its last form's value.
            "do" if elems.len() >= 2 => self.shape_of_guarded(elems.last()?, env, stack),
            // A `(let (…) body)` yields its body's value; bind each name as an alias so the body's
            // shape resolves (mirrors the emit/`eval_const` let handling).
            "let" if elems.len() >= 3 => {
                let binds = match elems.get(1)? {
                    Node::List(b) => b,
                    _ => return None,
                };
                let mut inner = env.to_vec();
                for pair in binds {
                    if let Node::List(p) = pair {
                        if let Some(Node::Name(name)) = p.first() {
                            inner.push(Local::aliased(name.clone(), p.get(1)?.clone(), inner.clone()));
                            continue;
                        }
                    }
                    return None;
                }
                self.shape_of_guarded(elems.last()?, &inner, stack)
            }
            "tuple" => {
                let shapes: Vec<Shape> = elems[1..]
                    .iter()
                    .map(|e| self.shape_of_guarded(e, env, stack))
                    .collect::<Option<_>>()?;
                if shapes.is_empty() {
                    Some(Shape::Unit)
                } else {
                    Some(Shape::Tuple(shapes))
                }
            }
            "list" => {
                // A list is homogeneous; every element must share one shape (the corpus rejects a
                // heterogeneous list before this). Take the first element's shape as the element
                // shape; an empty list has no runtime element form to render yet → decline.
                let first = elems.get(1)?;
                let elem = self.shape_of_guarded(first, env, stack)?;
                Some(Shape::List(Box::new(elem)))
            }
            "record" => {
                let mut fields: Vec<(String, Shape)> = Vec::new();
                for f in &elems[1..] {
                    if let Node::List(kv) = f {
                        if let (Some(Node::Name(k)), Some(v)) = (kv.first(), kv.get(1)) {
                            fields.push((k.clone(), self.shape_of_guarded(v, env, stack)?));
                            continue;
                        }
                    }
                    return None;
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Some(Shape::Record(fields))
            }
            ":" => self.shape_of_guarded(elems.get(1)?, env, stack), // annotation is transparent
            "if" if elems.len() == 4 => {
                // Both branches MUST agree on a shape: the renderer walks ONE static shape for the
                // `if`'s value regardless of which branch ran, so a disagreement means the value's
                // shape is not statically determinable — decline (do not pick one branch's shape,
                // which would mis-render the other). For a value whose variant is fixed (both
                // branches build the SAME sum type), `sum_shape` gives both branches the identical
                // whole-type shape, so they agree; a RECURSIVE builder (one branch Nil, the other
                // Cons carrying a recursive call) hits the recursion guard below and returns `None`,
                // so the branches disagree (one `Some`, one `None`) → decline.
                let t = self.shape_of_guarded(&elems[2], env, stack);
                let e = self.shape_of_guarded(&elems[3], env, stack);
                match (t, e) {
                    // Unify (not just require ==): two branches that build the SAME sum type but
                    // one placeholds a variant's payload (a `None` branch vs a `(Some (Some n))`
                    // branch — `Option (Option Int64)`) merge to the concrete nested shape.
                    (Some(a), Some(b)) => Self::merge_branch_shapes(&a, &b),
                    _ => None,
                }
            }
            // A `(match scrutinee arm…)` yields the value of the selected arm's body; the renderer
            // walks ONE static shape regardless of which arm ran, so — exactly like `if` — every
            // arm body must AGREE on a shape (else the shape is not statically determinable →
            // decline). This is the compiler's emit/lower DISPATCH shape: a `match` on a variant
            // whose every arm builds the same output type (Bytes, a Core node, …). Each arm's
            // pattern binders are bound as aliases so a body referencing one resolves (a payload
            // binder aliases to the scrutinee's sub-node; a recursive builder referencing it hits
            // the recursion guard → None → the arms disagree → decline, never a wrong shape). A
            // catch-all/`_`/name arm's body is shaped directly (no binder to add beyond the name).
            "match" if elems.len() >= 3 => {
                let scrutinee = &elems[1];
                let mut result: Option<Shape> = None;
                for arm in &elems[2..] {
                    let a = match arm {
                        Node::List(a) if a.len() == 2 => a,
                        _ => return None, // malformed arm — not shape-determinable
                    };
                    let (pattern, body) = (&a[0], &a[1]);
                    // Bind the arm's pattern binders (best-effort, matching `try_match`'s aliasing):
                    // a bare name / `(Ctor binder…)` binds names to the scrutinee's sub-nodes so the
                    // body's shape resolves. If binding is beyond compile-time resolution the body
                    // simply references an unbound name → its shape is None → arms disagree → decline.
                    let arm_env = match self.try_match(pattern, scrutinee, env) {
                        Ok(Some(binds)) => {
                            let mut e = env.to_vec();
                            e.extend(binds);
                            e
                        }
                        _ => env.to_vec(),
                    };
                    let sh = self.shape_of_guarded(body, &arm_env, stack);
                    result = match (result, sh) {
                        (None, s) => s,
                        // Unify arm shapes the same way `if` does — a placeheld variant payload in
                        // one arm yields to the concrete shape in another (nested Option, etc.).
                        (Some(a), Some(b)) => Some(Self::merge_branch_shapes(&a, &b)?),
                        _ => return None, // arms disagree → not statically determinable
                    };
                }
                result
            }
            // Scalar-producing operators: their result is a scalar leaf whose shape follows from
            // the operator, not from a callee (a runtime tuple element like `(= n 0)` or `(+ n 1)`
            // arrives here).
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" => Some(Shape::Int),
            "=" | "<" | ">" | "<=" | ">=" => Some(Shape::Bool),
            // A constructor application `(Some n)` / `(None unit)`: a runtime sum value. Its shape
            // is the WHOLE sum type (every variant in declaration order), so two different variants
            // of one type (`(Some n)` and `(None unit)` in an `if`) produce the SAME shape and the
            // renderer's discriminant switch covers all arms. The applied variant's payload shape
            // is inferred from its argument; other unary variants' payloads are left as `Int` (a
            // placeholder the renderer only reaches for a discriminant that is actually built — an
            // unbuilt variant's arm is dead), and nullary variants are `Unit`.
            _ if is_constructor_name(head) => self.sum_shape(head, elems.get(1), env, stack),
            // `(tuple.N t)` — the RENDER shape of a positional access. Take element N's shape from a
            // STRUCTURALLY-resolvable tuple operand (an inline `(tuple …)` / an alias to one). A
            // RUNTIME tuple operand (a value returned from a function, built in a match arm) does NOT
            // resolve structurally → return `None`, so the whole-program render DECLINES cleanly
            // rather than walking the value against a guessed shape and trapping. (The `tuple.N`
            // consumption path — feeding a match / a scalar op — is unaffected: it emits `arr-get`
            // in `gen_tuple_access` and does not consult this render shape.)
            _ if head.starts_with("tuple.") && elems.len() == 2 => {
                let idx: usize = head[6..].parse().ok()?;
                let (t, tenv) = self.resolve(&elems[1], env)?;
                let items = match &t {
                    Node::List(items) if name_of(items.first()) == Some("tuple") => items,
                    _ => return None,
                };
                self.shape_of_guarded(items.get(idx + 1)?, &tenv, stack)
            }
            // A call to a user function: inline it (bind parameters to argument nodes as aliases,
            // exactly as `gen_call`/`gen_apply` do) and take the body's shape. A call to a function
            // ALREADY being inlined is a recursive builder whose result type is infinite — decline
            // (the renderer is tree-shaped; see `shape_of_guarded`).
            _ => {
                if stack.iter().any(|n| n == head) {
                    return None;
                }
                let f = self.lookup_fn(head)?;
                let args = &elems[1..];
                if args.len() != f.params.len() {
                    return None;
                }
                let mut inner = env.to_vec();
                for (p, a) in f.params.iter().zip(args.iter()) {
                    inner.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
                }
                let body = f.body.clone();
                stack.push(head.to_string());
                let shape = self.shape_of_guarded(&body, &inner, stack);
                stack.pop();
                shape
            }
        }
    }

    /// The `Shape::Sum` for an applied constructor `variant` (payload node `payload`, if any). The
    /// shape covers EVERY variant of the constructor's declared sum type in declaration order (so
    /// the discriminant indexes it and the renderer's switch is total); each variant's payload
    /// shape is `Unit` for a nullary variant, the applied variant's inferred payload shape for the
    /// applied one, and `Int` as a harmless placeholder for other unary variants (their render arm
    /// is only reached for a discriminant actually built, which carries the matching payload). The
    /// discriminant assignment here (index in `sum_variants`) MUST match what `gen_runtime_ctor`
    /// emits.
    fn sum_shape(
        &self,
        variant: &str,
        payload: Option<&Node>,
        env: &[Local],
        stack: &mut Vec<String>,
    ) -> Option<Shape> {
        // The construction site writes the variant either BARE (`Some`) or QUALIFIED
        // (`Sign.Pos`, `IntList.Cons`); the canonical text renders it EXACTLY as written (the const
        // path stores the as-written name). `sum_variants` stores bare tags, so when the applied
        // variant is qualified, every sibling variant in the rendered Shape must be qualified with
        // the SAME type prefix so the renderer's discriminant switch produces `(IntList.Cons …)` /
        // `(IntList.Nil unit)`, not the bare `(Cons …)`.
        let tag = variant_tag(variant);
        let qualifier: Option<&str> = variant.rsplit_once('.').map(|(ty, _)| ty);
        let type_name = self.sum_types.get(tag)?.clone();
        let order = self.sum_variants.get(&type_name)?.clone();

        // Is this sum type RECURSIVE — does any variant's declared payload type mention the type
        // itself (directly or nested)? A recursive type's fully-expanded `Shape` is infinite, so we
        // build EVERY variant's payload shape FROM ITS DECLARATION (`sum_payload_types`), mapping a
        // self-reference to `Shape::Rec(type_name)` (a finite cut the renderer resolves to a
        // recursive call). This also fills in the OTHER variants' real payload shapes (not the `Int`
        // placeholder the non-recursive path uses), because a recursive value walks arms whose
        // discriminant is only known at run time — every arm must render correctly, not just the
        // applied one. A NON-recursive type keeps the original behavior (applied variant's inferred
        // shape; `Int` placeholder for a sibling unary variant's dead arm) so nothing else changes.
        let recursive = self.sum_type_is_recursive(&type_name);

        let variants: Vec<(String, Shape)> = order
            .iter()
            .map(|v| {
                let sh = if recursive {
                    // Every variant's shape comes from its DECLARED payload type, self-refs → Rec.
                    self.variant_payload_shape_from_decl(v, &type_name)
                } else if v == tag {
                    match payload {
                        Some(p) => self.shape_of_guarded(p, env, stack)?,
                        None => Shape::Unit,
                    }
                } else if self.nullary_variants.contains(v) {
                    Shape::Unit
                } else {
                    Shape::Int // placeholder for an unbuilt unary variant's arm (dead here)
                };
                // Render each variant name AS WRITTEN at the construction site: bare when the
                // applied variant was bare (`Some`/`None`), qualified with the shared type prefix
                // when it was qualified (`IntList.Cons` → `IntList.Nil`, `Sign.Pos` → `Sign.Zero`).
                let name = match qualifier {
                    Some(q) => format!("{q}.{v}"),
                    None => v.clone(),
                };
                Some((name, sh))
            })
            .collect::<Option<_>>()?;
        Some(Shape::Sum(variants))
    }

    /// Does sum type `type_name` mention itself in any variant's declared payload type — is it
    /// RECURSIVE? Scans every variant's `sum_payload_types` slot for a type node naming `type_name`
    /// (directly `IntList` or nested `(Tuple Int64 IntList)` / `(List IntList)`). A recursive type's
    /// fully-expanded render shape is infinite, so it must render via a recursive fn (`Shape::Rec`).
    fn sum_type_is_recursive(&self, type_name: &str) -> bool {
        let order = match self.sum_variants.get(type_name) {
            Some(o) => o,
            None => return false,
        };
        order.iter().any(|v| {
            self.sum_payload_types
                .get(v)
                .map_or(false, |slots| slots.iter().any(|t| type_node_mentions(t, type_name)))
        })
    }

    /// The render `Shape` of variant `v`'s payload, built FROM ITS DECLARATION (`sum_payload_types`),
    /// with any reference to the enclosing recursive type `self_ty` cut to `Shape::Rec(self_ty)`. A
    /// nullary variant (no slots) is `Unit`; a single slot is that slot's shape; multiple slots form
    /// a `Tuple` (the payload is a tuple of the slots, matching the runtime layout). A slot type the
    /// shape system cannot express yet (a foreign sum, a function) makes this `Unit` — a conservative
    /// placeholder for a dead/unreachable arm rather than a hard failure (the applied arm's slots are
    /// concrete for the corpus's recursive types: Int64 + the self-reference).
    fn variant_payload_shape_from_decl(&self, v: &str, self_ty: &str) -> Shape {
        let slots = match self.sum_payload_types.get(v) {
            Some(s) if !s.is_empty() => s,
            _ => return Shape::Unit, // nullary variant → unit payload
        };
        let shape_of_slot = |t: &Node| -> Shape {
            if type_node_mentions(t, self_ty) {
                // A slot that IS or CONTAINS the recursive type. A bare self-name is the recursive
                // cut; a compound carrying it (`(Tuple Int64 IntList)`) expands with the self-ref
                // inside → Rec at that position.
                self.type_node_shape_with_rec(t, self_ty)
            } else {
                shape_of_type_node(t).unwrap_or(Shape::Unit)
            }
        };
        if slots.len() == 1 {
            shape_of_slot(&slots[0])
        } else {
            Shape::Tuple(slots.iter().map(shape_of_slot).collect())
        }
    }

    /// Collect the `type_shapes` map the renderer needs: every recursive sum type reachable from
    /// `top`, keyed by type name → its full `Sum` shape (the EXACT shape built at the construction
    /// site, so qualification matches). Walks the shape tree; for each `Shape::Sum`, derives the type
    /// name from its first variant's tag (via `sum_types`) and records the shape if that type is
    /// recursive. A `Shape::Rec(T)` in the tree does not recurse the walk (its expansion is the
    /// already-recorded `Sum` for T) — the recorded `Sum` is what the renderer resolves `Rec(T)` to.
    fn collect_type_shapes(&self, top: &Shape) -> std::collections::BTreeMap<String, Shape> {
        let mut out = std::collections::BTreeMap::new();
        self.collect_type_shapes_into(top, &mut out);
        out
    }

    fn collect_type_shapes_into(&self, s: &Shape, out: &mut std::collections::BTreeMap<String, Shape>) {
        match s {
            Shape::Sum(variants) => {
                if let Some((first_name, _)) = variants.first() {
                    if let Some(type_name) = self.sum_types.get(variant_tag(first_name)) {
                        if self.sum_type_is_recursive(type_name) && !out.contains_key(type_name) {
                            out.insert(type_name.clone(), s.clone());
                        }
                    }
                }
                for (_, payload) in variants {
                    self.collect_type_shapes_into(payload, out);
                }
            }
            Shape::Tuple(elems) => elems.iter().for_each(|e| self.collect_type_shapes_into(e, out)),
            Shape::Record(fields) => fields.iter().for_each(|(_, v)| self.collect_type_shapes_into(v, out)),
            Shape::List(elem) => self.collect_type_shapes_into(elem, out),
            // `Rec(T)` terminates the walk (its expansion is the recorded `Sum` for T); leaves have
            // no nested shapes.
            _ => {}
        }
    }

    /// A type node → `Shape`, mapping any occurrence of `self_ty` (the enclosing recursive sum) to
    /// `Shape::Rec(self_ty)`. Handles the nesting the recursive corpus types use: a bare self-name,
    /// `(Tuple … self_ty …)`, `(List self_ty)`. A non-self compound falls back to `shape_of_type_node`.
    fn type_node_shape_with_rec(&self, n: &Node, self_ty: &str) -> Shape {
        match n {
            Node::Name(name) if name == self_ty => Shape::Rec(self_ty.to_string()),
            Node::List(items) => match name_of(items.first()) {
                Some("Tuple") => Shape::Tuple(
                    items[1..].iter().map(|t| self.type_node_shape_with_rec(t, self_ty)).collect(),
                ),
                Some("List") => match items.get(1) {
                    Some(elem) => Shape::List(Box::new(self.type_node_shape_with_rec(elem, self_ty))),
                    None => Shape::Unit,
                },
                _ => shape_of_type_node(n).unwrap_or(Shape::Unit),
            },
            _ => shape_of_type_node(n).unwrap_or(Shape::Unit),
        }
    }

    /// The `Shape::Sum` for a runtime `Option<payload>` — the shape of a fallible access's result
    /// (`Bytes.at` → `Option<Int>`, `Bytes.slice` → `Option<Bytes>`). Built directly from a payload
    /// SHAPE (not a payload node), covering the whole Option type in declaration order so the
    /// discriminant indexes it and the renderer's switch is total. The `Some` arm carries `payload`;
    /// every other variant (`None`) is nullary → `Unit`. The variant NAMES are bare (`Some`/`None`),
    /// matching the corpus `(Some …)` / `(None unit)` render. The discriminant order MUST match what
    /// `gen_runtime_bytes_at`/`gen_runtime_bytes_slice` emit — both read `variant_disc` from the same
    /// `sum_variants["Option"]` order this iterates, so they agree by construction.
    fn option_shape(&self, payload: Shape) -> Option<Shape> {
        let type_name = self.sum_types.get("Some")?;
        let order = self.sum_variants.get(type_name)?;
        let variants: Vec<(String, Shape)> = order
            .iter()
            .map(|v| {
                let sh = if v == "Some" { payload.clone() } else { Shape::Unit };
                (v.clone(), sh)
            })
            .collect();
        Some(Shape::Sum(variants))
    }

    /// Unify two branch shapes for the value of an `if`/`match` (the renderer walks ONE static shape
    /// regardless of which branch ran). Identical shapes unify to themselves. Two `Sum` shapes over
    /// the SAME ordered variant-name set unify variant-wise: a branch that BUILDS a variant carries
    /// its real payload shape, while a branch that does NOT build it carries a `sum_shape` PLACEHOLDER
    /// (`Int` for an un-built unary variant's dead arm, `Unit` for a nullary) — so the two branches
    /// disagree on that variant's payload only because one placeheld it. Prefer the richer (built)
    /// payload; recurse so nested sums (`Option (Option Int64)` — a `None` branch vs a `(Some (Some
    /// n))` branch) unify to the concrete nested shape. This is exactly the case where `(if c (None
    /// unit) (Some (Some n)))` was declining "cannot infer runtime compound result shape": both
    /// branches ARE the same Option type, differing only in the placeheld `Some` payload. Returns
    /// `None` when the shapes are genuinely incompatible (different variant sets, scalar-vs-compound),
    /// preserving the decline-don't-miscompile guard. Nested `Tuple`/`List`/`Record` unify structurally.
    fn merge_branch_shapes(a: &Shape, b: &Shape) -> Option<Shape> {
        if a == b {
            return Some(a.clone());
        }
        match (a, b) {
            // The `Int` placeholder `sum_shape` uses for an un-built variant's dead arm yields to the
            // other branch's concrete payload (whatever it is — a scalar, or a nested compound).
            (Shape::Int, other) | (other, Shape::Int) => Some(other.clone()),
            (Shape::Sum(va), Shape::Sum(vb)) if va.len() == vb.len() => {
                let merged: Vec<(String, Shape)> = va
                    .iter()
                    .zip(vb.iter())
                    .map(|((na, sa), (nb, sb))| {
                        if na != nb {
                            return None; // different variant name/order → incompatible
                        }
                        Some((na.clone(), Self::merge_branch_shapes(sa, sb)?))
                    })
                    .collect::<Option<_>>()?;
                Some(Shape::Sum(merged))
            }
            (Shape::Tuple(ta), Shape::Tuple(tb)) if ta.len() == tb.len() => {
                let merged: Vec<Shape> = ta
                    .iter()
                    .zip(tb.iter())
                    .map(|(x, y)| Self::merge_branch_shapes(x, y))
                    .collect::<Option<_>>()?;
                Some(Shape::Tuple(merged))
            }
            (Shape::List(x), Shape::List(y)) => {
                Some(Shape::List(Box::new(Self::merge_branch_shapes(x, y)?)))
            }
            (Shape::Record(fa), Shape::Record(fb)) if fa.len() == fb.len() => {
                let merged: Vec<(String, Shape)> = fa
                    .iter()
                    .zip(fb.iter())
                    .map(|((ka, sa), (kb, sb))| {
                        if ka != kb {
                            return None;
                        }
                        Some((ka.clone(), Self::merge_branch_shapes(sa, sb)?))
                    })
                    .collect::<Option<_>>()?;
                Some(Shape::Record(merged))
            }
            _ => None,
        }
    }

    // ─── Module assembly ───────────────────────────────────────────────────────────

    /// The program's manifest: the host imports computed from the entrypoint's `(host …)`
    /// delegations. Each delegated effect contributes ALL its declared operations as boundary
    /// imports named `effect.op`, in a stable order (delegation order, then declaration order). The
    /// value-heap runtime is the one exempt import and is not a host function here. `main` (func 0)
    /// is the sole entrypoint. A delegated effect the emit pass never reaches is CDZ0404 (checked in
    /// `gen_host`), so an effect here that no perform matches never affects a running program's
    /// behavior; but its op imports must still be present so the emitted `call` indices are stable.
    fn compute_manifest(&self) -> Vec<HostImport> {
        let mut out: Vec<HostImport> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let body = self.funcs[0].body.clone();
        let mut delegated: Vec<String> = Vec::new();
        collect_delegated_effects(&body, &mut delegated);
        for eff in delegated {
            if let Some(decl) = self.effects.get(&eff) {
                for op in &decl.ops {
                    let name = format!("{eff}.{}", op.name);
                    if seen.insert(name.clone()) {
                        // A `Unit` parameter carries no data and has no boundary/component
                        // representation — strip it, so `ask : Unit -> Int64` imports as a
                        // no-parameter boundary func `ask: func() -> s64`. The perform elides the
                        // unit argument to match.
                        let params: Vec<Kind> =
                            op.params.iter().copied().filter(|k| *k != Kind::Unit).collect();
                        out.push(HostImport { name, params, result: op.result });
                    }
                }
            }
        }
        out
    }

    fn compile_module(&mut self) -> Result<Vec<u8>, Decline> {
        // A `compile` entrypoint (`(def (compile b) …)`) is exported as `cadenza:compiler/compile :
        // func(list<u8>) -> list<u8>` — the `bytes → bytes` seam a Cadenza-authored compiler exports
        // (bootstrap.md §"The Compiler Is Authored In Cadenza"), driven by the host's
        // `component-check`/`run_compiler_component` harness over the whole corpus. It takes exactly
        // ONE `Bytes`/`list<u8>` parameter and returns `Bytes`/`list<u8>`; both cross the boundary via
        // the canonical list ABI. Emitted through a dedicated envelope (COMPILE_HEAD/TAIL). This is
        // distinct from the nullary `run` entry below — checked first because it is the only entry
        // that legitimately takes a parameter.
        if self.funcs[0].name == "compile" {
            return self.compile_component_module();
        }

        // The entrypoint is exported as `run: () -> output` (component-abi.md §The Program Exports
        // A Nullary Run) — a NULLARY function whose result crosses the boundary. A `main` declared
        // WITH parameters (`(def (main n) …)`) has no channel to receive an argument through that
        // signature, so the emitted core func's arity would disagree with the lifted export and the
        // component is invalid. Decline cleanly rather than emit it (decline-don't-miscompile): the
        // entrypoint must be nullary. (To exercise a function over a runtime input, call it from a
        // nullary `main` with a literal argument — the value is then a genuine runtime operand.)
        if !self.funcs[0].params.is_empty() {
            return decline("the entrypoint `main` must take no parameters (it is exported as the nullary `run`)");
        }

        // Compute the program's manifest — the host imports — from the entrypoint's `(host …)`
        // delegations, BEFORE the emit pass (so host funcs occupy the low core-func indices and
        // `call_base` shifts the user functions past them, and `gen_delegated_call` resolves each
        // `effect.op` to a stable index). The manifest is the union of the delegated effects'
        // operations (capabilities-and-effects.md §The Program Manifest Is The Union Of Its
        // Entrypoints' Delegations); `main` is the sole entrypoint the seed recognizes.
        self.host_imports = self.compute_manifest();

        // A `main` that const-folds to a COMPOUND value (string/sum/tuple/…) has no scalar
        // wasm representation — it crosses the boundary as its proper type, a resource owning
        // `display()` (runnable_component). But FIRST type-check the body (an ill-typed program
        // like `(= map record)` folds to a scalar Bool and must reject, not emit) — only a
        // genuinely COMPOUND folded value takes this path; a scalar/Bool falls through to the
        // normal scalar compile, which runs the type rejections.
        if self.funcs[0].params.is_empty() {
            let env: Vec<Local> = Vec::new();
            if let Ok(Some(v)) = self.eval_const(&self.funcs[0].body.clone(), &env) {
                if is_compound_cval(&v) {
                    // A compound-folded body skips the scalar `emit` path, so its type
                    // rejections were never run. Walk the tree and reject an ill-typed form
                    // (e.g. a non-homogeneous `(list 1 true)`) BEFORE emitting a resource for
                    // it — the compiler must not emit a component for an ill-typed program.
                    self.check_tree(&self.funcs[0].body.clone(), &env)?;
                    if let Some(component) = compound_component(&v) {
                        return Ok(component);
                    }
                }
            }
        }

        // Compile each user function's body (this also allocates its locals). The emitted result
        // kind is ground truth — write it back over the inferred `ret_kind` so the function's
        // wasm signature matches its body (inference is a best-effort pre-pass). Compile in
        // INDEX order and write each kind back immediately, so a caller reads exactly the callee
        // kinds it did before this pass existed (main is index 0, compiled first).
        //
        // A function that DECLINES is not an immediate failure: it may be DEAD — reached only by
        // compile-time folding, never by an emitted runtime `call` (e.g. a `parse`/`classify`
        // returning an Option/Result whose every caller matches its result at compile time). Its
        // decline is deferred; whether it is fatal depends on reachability from `main`.
        let n = self.funcs.len();
        // If INFERENCE already says `main` returns a runtime heap value, we are on the
        // runtime-compound path: set `call_base` to the import-shifted base BEFORE the (single)
        // compile pass, so bodies emit their `call` targets at the right indices AND a const
        // sub-value inside a runtime compound (e.g. the `(None unit)` branch of a runtime-sum
        // `if`) builds a runtime value rather than declining. The offset shifts only emitted call
        // bytes; kinds and reachability are invariant to it. (A scalar `main` keeps `call_base=0`.)
        if self.funcs[0].ret_kind.externalized() == Kind::Heap {
            self.call_base = RT_FUNC_BASE;
        } else if !self.host_imports.is_empty() {
            // Host-import path: the imported host funcs occupy the LOW core-func indices
            // (`0..n_host_imports`), so every USER function is shifted past them. Set the base
            // before the compile pass so emitted `call` targets to user funcs land correctly and
            // host calls (emitted at the raw import index) resolve.
            self.call_base = self.host_imports.len() as u32;
        }

        // Compile every body. A reachable body that declines FOR A HEAP REASON — it needs a
        // runtime value-heap constructor the scalar path cannot provide (`len`/`sum` folding a
        // runtime linked list to a scalar; a `main`-scalar program computing over runtime heap
        // values) — is not necessarily fatal: RETRY the whole pass in runtime mode
        // (`call_base = RT_FUNC_BASE`), where those constructors lower against the value-heap
        // runtime import. The retry fires only when the scalar pass left a reachable HEAP decline
        // AND we were not already in runtime mode, so a genuinely-scalar program compiles once and
        // a const-foldable-sum program (the classifier cases) is unaffected — it declines nothing
        // reachable on the scalar path because its sums fold.
        // DEV-desk tracing (ask-50): mark which PASS is running. A single `compile_program` can walk
        // every body TWICE — a scalar pass, then (on a reachable heap-decline) a runtime retry — so a
        // `declined: …` is ambiguous without knowing its pass. `mode=scalar` here, `mode=runtime` at
        // the retry below. (Emits nothing unless built `--features trace`.)
        #[cfg(feature = "trace")]
        tracing::debug!(target: "cdz::pass", mode = "scalar", "compile pass");
        let (mut bodies, mut reachable) = self.compile_all_bodies(n)?;
        if self.call_base == 0
            && reachable.1.iter().any(|d| d.as_ref().map_or(false, |d| is_heap_decline(d)))
        {
            #[cfg(feature = "trace")]
            tracing::debug!(target: "cdz::pass", mode = "runtime", "runtime retry (scalar pass left a heap decline)");
            self.call_base = RT_FUNC_BASE;
            let retried = self.compile_all_bodies(n)?;
            bodies = retried.0;
            reachable = retried.1;
        }
        // A body that STILL declined after any retry is fatal when it is reachable from `main`, OR
        // when it is a kept UNBOUND-NAME reject (CDZ0101 — `compile_all_bodies` retains those for a
        // dead definition too, since the unbound-name rule is unconditional; every other dead decline
        // was already cleared to a trap stub).
        for i in 0..n {
            let is_unbound = reachable.1[i].as_ref().map_or(false, |d| d.code() == Some("CDZ0101"));
            if reachable.0.contains(&(i as u32)) || is_unbound {
                if let Some(d) = reachable.1[i].take() {
                    return Err(d);
                }
            }
        }

        let main_ret = self.funcs[0].ret_kind.externalized();

        // Effect-context specializations (Stage 3) are appended in the plain self-contained scalar
        // component below, the runtime-SCALAR path (`runtime_scalar_component`, ask-44), the
        // compile-ENTRY path (`compile_component`, ask-46), AND the runtime-COMPOUND path
        // (`runtime_compound_component`, ask-49 — a recursive effectful `handle` whose RESULT is a
        // heap value rendered in-program, e.g. `main` collecting a `list<diagnostic>` then returning
        // a compound). Specs sit at `[fixed][user][helpers][SPECS][render][run]`, so the render fns
        // shift past them (`emit_renderer` takes `n_specs`), resolving the interleave. Only the
        // HOST-import assembly path still lacks spec-appending — a spec's `call` target would be
        // missing there — so decline cleanly on THAT (a separate later extension).
        if !self.specializations.borrow().is_empty() && !self.host_imports.is_empty() {
            return decline(
                "recursive effectful function under host delegation not yet emitted (scalar, runtime-scalar, runtime-compound, and compile-entry paths covered)",
            );
        }

        // ── Runtime-compound path (M2 Phase B/C): `main` returns a value-heap handle. ──
        // The program cannot be a self-contained scalar component; it imports the value-heap
        // runtime and renders its result to a string in-program (component-abi.md §The Value-Heap
        // Runtime). `call_base` was already set to `RT_FUNC_BASE` before the compile pass (when
        // inference said `main` returns Heap), so `bodies` already have their `call` targets at the
        // import-shifted indices — reuse them directly. Then emit the TYPE-DIRECTED renderer and
        // assemble the runtime-compound component from the fixed byte-shape.
        if main_ret == Kind::Heap {
            // The renderer is TYPE-DIRECTED and tag-free: infer `main`'s result Shape (with the
            // field/variant names the tag-free runtime does not hold) and emit a walk that reads
            // the value through the runtime's accessors. A shape we cannot infer, or one carrying
            // a not-yet-renderable leaf (runtime float/string), declines here — the compiler emits
            // no wrong renderer (decline-don't-miscompile).
            let main_body = self.funcs[0].body.clone();
            let top_shape = match self.shape_of(&main_body, &[]) {
                Some(s) => s,
                None => return decline("cannot infer runtime compound result shape"),
            };
            let n_helpers = self.helper_count();
            let (spec_types, n_specs, spec_bodies) = self.spec_artifacts()?;
            // The recursive-type shape table: each recursive sum type reachable from the top shape
            // → its full `Sum` shape, so the renderer resolves a `Shape::Rec(T)` payload to a
            // recursive CALL into T's render fn (walking a runtime spine to its actual depth).
            let type_shapes = self.collect_type_shapes(&top_shape);
            let (render_bodies, run_body) = emit_renderer(&top_shape, n, n_helpers, n_specs, type_shapes)?;
            let helper_bodies = self.helper_bodies();
            return Ok(runtime_compound_component(
                &self.funcs,
                &bodies,
                &helper_bodies,
                &spec_types,
                n_specs,
                &spec_bodies,
                &render_bodies,
                &run_body,
            ));
        }

        // ── Runtime-scalar path: `main` returns a SCALAR but the program computes over runtime
        // value-heap values (the retry above set `call_base = RT_FUNC_BASE`). It imports the
        // value-heap runtime like the runtime-compound path, but its `run` export returns the
        // scalar directly (no in-program renderer) — a recursive `len`/`sum` folding a runtime
        // linked list to an Int64, the core compiler idiom. ──
        if self.call_base == RT_FUNC_BASE && main_ret != Kind::Heap {
            let helper_bodies = self.helper_bodies();
            let (spec_types, n_specs, spec_bodies) = self.spec_artifacts()?;
            return runtime_scalar_component(
                &self.funcs, &bodies, &helper_bodies, main_ret, &spec_types, n_specs, &spec_bodies,
            );
        }

        // ── Host-import path: `main` returns a scalar/unit but the program imports host
        // functions. Emit a component that imports each host function at the component boundary,
        // lowers it into a core func the program module calls, and lifts `run`. ──
        if !self.host_imports.is_empty() {
            return host_import_component(&self.host_imports, &self.funcs, &bodies, &self.helper_bodies(), main_ret);
        }

        // Effect-context specializations (Stage 3) emitted lazily during the compile pass, if any.
        let specs = self.specializations.borrow();
        let n_specs = specs.len();

        // ── Type section: one functype per user function, one per helper, one per specialization. ──
        let mut type_items = Vec::new();
        let mut n_types = 0usize;
        for f in &self.funcs {
            type_items.extend_from_slice(&functype(&f.param_kinds, f.ret_kind.externalized()));
            n_types += 1;
        }
        // Helpers all share the shape (i64,i64)->i64, but we emit one type each for a
        // 1:1 func→type mapping (simpler; wasm permits duplicate types).
        let helper_ty = functype(&[Kind::Int64, Kind::Int64], Kind::Int64);
        for _ in 0..self.helper_count() {
            type_items.extend_from_slice(&helper_ty);
            n_types += 1;
        }
        // A specialization's signature threads state: `(orig-params…, states…) -> (ret, states…)`.
        for s in specs.iter() {
            type_items.extend_from_slice(&functype_spec(&s.param_kinds, &s.state_kinds, s.ret_kind.externalized()));
            n_types += 1;
        }
        let type_sec = section(1, &wasm_vec(n_types, &type_items));

        // ── Function section: func i uses type i (1:1). ──
        let n_funcs = self.funcs.len() + self.helper_count() + n_specs;
        let mut func_items = Vec::new();
        for i in 0..n_funcs {
            uleb128(i as u64, &mut func_items);
        }
        let func_sec = section(3, &wasm_vec(n_funcs, &func_items));

        // ── Export section: export core func 0 (main) as "run". ──
        let mut export_item = Vec::new();
        uleb128(3, &mut export_item); // name length
        export_item.extend_from_slice(b"run");
        export_item.push(0x00); // export kind: func
        uleb128(0, &mut export_item); // func index 0
        let export_sec = section(7, &wasm_vec(1, &export_item));

        // ── Code section: user bodies, then helper bodies, then specialization bodies. ──
        let mut code_items = Vec::new();
        for b in &bodies {
            code_items.extend_from_slice(&encode_body(b));
        }
        for hb in self.helper_bodies() {
            code_items.extend_from_slice(&encode_body(&hb));
        }
        for s in specs.iter() {
            let sb = s.body.borrow();
            let body = sb.as_ref().ok_or_else(|| Decline("specialization body never emitted".into(), None))?;
            code_items.extend_from_slice(&encode_body(body));
        }
        let code_sec = section(10, &wasm_vec(n_funcs, &code_items));

        // ── Core module. ──
        let mut core = Vec::new();
        core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]); // \0asm v1
        core.extend_from_slice(&type_sec);
        core.extend_from_slice(&func_sec);
        core.extend_from_slice(&export_sec);
        core.extend_from_slice(&code_sec);

        // ── Component envelope. ──
        Ok(wrap_component(&core, main_ret))
    }

    /// Does this `compile` body evaluate to a `Result` (`Ok`/`Err`) value — the diagnostics-ABI
    /// signal? Walks the body's TAIL positions only (the value the function returns), NOT arbitrary
    /// sub-expressions: an `Ok`/`Err` application here; `if`/`match`/`let`/`do` recurse into their
    /// value-producing sub-forms (both `if` branches, every `match` arm body, the `let`/`do` last
    /// form); a call to a user function recurses into ITS body (one level per callee, guarded by
    /// `seen` against recursion). Deliberately independent of `shape_of` branch-agreement: a
    /// branch-on-rejection body's arms build DIFFERENT Result variants and never agree as one shape,
    /// but the body is still Result-typed. Returns false for a bare-`Bytes` body (the plain ABI).
    fn compile_body_is_result(&self, node: &Node, seen: &mut Vec<String>) -> bool {
        let items = match node {
            Node::List(items) if !items.is_empty() => items,
            _ => return false,
        };
        // A direct `(Ok …)` / `(Err …)` constructor application.
        if let Some(tag) = constructor_of(items.first()) {
            let t = variant_tag(&tag);
            return t == "Ok" || t == "Err";
        }
        match name_of(items.first()) {
            // `if`: EITHER branch producing a Result makes the body Result-typed (branch-on-rejection
            // builds `Ok` in one branch, `Err` in the other).
            Some("if") if items.len() == 4 => {
                self.compile_body_is_result(&items[2], seen)
                    || self.compile_body_is_result(&items[3], seen)
            }
            // `match`: any arm body producing a Result.
            Some("match") if items.len() >= 3 => items[2..].iter().any(|arm| match arm {
                Node::List(a) if a.len() == 2 => self.compile_body_is_result(&a[1], seen),
                _ => false,
            }),
            // `let`/`do`: the value is the last form.
            Some("let") if items.len() >= 3 => self.compile_body_is_result(items.last().unwrap(), seen),
            Some("do") if items.len() >= 2 => self.compile_body_is_result(items.last().unwrap(), seen),
            // `(handle <state> <arms> <body>)` evaluates to its BODY (index 3) — an effect handler
            // wrapping the `Ok`/`Err` result is Result-typed (ask-51, twin of the artifact-ABI walk).
            Some("handle") if items.len() == 4 => self.compile_body_is_result(&items[3], seen),
            // `(: e T)` annotation is transparent.
            Some(":") if items.len() >= 2 => self.compile_body_is_result(&items[1], seen),
            // A call to a user function: recurse into its body (one level per callee, cycle-guarded).
            Some(head) if !is_special_form_head(items.first()) => {
                if seen.iter().any(|n| n == head) {
                    return false;
                }
                match self.lookup_fn(head) {
                    Some(f) => {
                        seen.push(head.to_string());
                        let r = self.compile_body_is_result(&f.body.clone(), seen);
                        seen.pop();
                        r
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Does this `compile` body evaluate to a `compile-output` record (ask-41)? The artifact ABI
    /// signal: a tail-position `(record (artifacts …) (diagnostics …))` (a record whose fields are
    /// exactly `artifacts` and `diagnostics`). Same tail-walk as `compile_body_is_result` — through
    /// `if`/`match`/`let`/`do` and one level of helper-call — so branch-on-rejection (both branches a
    /// `compile-output` record, one with an empty artifacts list, one with an empty diagnostics list)
    /// is detected regardless of shape agreement.
    fn compile_body_is_artifacts(&self, node: &Node, seen: &mut Vec<String>) -> bool {
        let items = match node {
            Node::List(items) if !items.is_empty() => items,
            _ => return false,
        };
        // A direct `(record (artifacts …) (diagnostics …))`: a record with exactly those two fields.
        if name_of(items.first()) == Some("record") {
            let mut fields: Vec<&str> = items[1..]
                .iter()
                .filter_map(|f| match f {
                    Node::List(kv) => name_of(kv.first()),
                    _ => None,
                })
                .collect();
            fields.sort_unstable();
            return fields == ["artifacts", "diagnostics"];
        }
        match name_of(items.first()) {
            Some("if") if items.len() == 4 => {
                self.compile_body_is_artifacts(&items[2], seen)
                    || self.compile_body_is_artifacts(&items[3], seen)
            }
            Some("match") if items.len() >= 3 => items[2..].iter().any(|arm| match arm {
                Node::List(a) if a.len() == 2 => self.compile_body_is_artifacts(&a[1], seen),
                _ => false,
            }),
            Some("let") if items.len() >= 3 => self.compile_body_is_artifacts(items.last().unwrap(), seen),
            Some("do") if items.len() >= 2 => self.compile_body_is_artifacts(items.last().unwrap(), seen),
            // A `(handle <state> <arms> <body>)` evaluates to its BODY (index 3) — so an effect handler
            // wrapping the `compile-output` record (the natural shape for effect-based diagnostics:
            // `(handle (list) ((Diag.emit …)(Diag.collect …)) (record (artifacts …)(diagnostics …)))`)
            // is the artifact ABI. Recurse into the handle's tail, exactly as through `let`/`do` (ask-51).
            Some("handle") if items.len() == 4 => self.compile_body_is_artifacts(&items[3], seen),
            Some(":") if items.len() >= 2 => self.compile_body_is_artifacts(&items[1], seen),
            Some(head) if !is_special_form_head(items.first()) => {
                if seen.iter().any(|n| n == head) {
                    return false;
                }
                match self.lookup_fn(head) {
                    Some(f) => {
                        seen.push(head.to_string());
                        let r = self.compile_body_is_artifacts(&f.body.clone(), seen);
                        seen.pop();
                        r
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Emit a `compile : list<u8> -> list<u8>` component from a `(def (compile b) …)` entry — the
    /// `cadenza:compiler/compile` seam a Cadenza-authored compiler exports. Func 0 (`compile`) takes
    /// one `Bytes` handle and returns a `Bytes` handle, both on the value-heap runtime path. The
    /// emitted core module's `compile: (i32 ptr, i32 len) -> i32 retptr` reads the incoming bytes from
    /// linear memory into a runtime `bytes-alloc`+`bytes-set` handle, calls the user body, then writes
    /// the result handle's bytes back to linear memory via `cabi_realloc` and returns a `(ptr,len)`
    /// retptr — the canonical-ABI list marshalling the generated COMPILE_HEAD/TAIL lift expects.
    /// (A body that evaluates to a `Result` takes the diagnostics-ABI wrapper instead —
    /// `compile_body_is_result` + `result_abi`.)
    fn compile_component_module(&mut self) -> Result<Vec<u8>, Decline> {
        if self.funcs[0].params.len() != 1 {
            return decline("the `compile` entrypoint takes exactly one `Bytes` (list<u8>) parameter");
        }
        // DIAGNOSTICS ABI (ask-40): a `compile` body that returns a `Result<Bytes, list<diagnostic>>`
        // is exported as `compile: list<u8> -> result<list<u8>, list<diagnostic>>` (build-tool-interface.md
        // §The Tool Produces A Component, A Manifest, And Diagnostics — the failure arm carries the
        // diagnostics, distinguished by TYPE, not by an empty-bytes sentinel or a trap). A body that
        // returns a bare `Bytes` keeps the plain `list<u8> -> list<u8>` seam. `result_abi` carries the
        // `Ok` discriminant so the wrapper knows which arm is the component-bytes success arm.
        //
        // Detection walks the body's TAIL positions for an `Ok`/`Err` constructor — NOT `shape_of`,
        // which on an `if`/`match` demands both branches AGREE on one shape. The whole point of the
        // diagnostics ABI is branch-on-rejection — `(if reject? (Err diags) (Ok bytes))` — whose arms
        // build DIFFERENT Result variants (they never agree as one shape), so a shape-agreement check
        // wrongly falls through to the bytes path and reads the Result heap handle as Bytes (the
        // `Ok (0 bytes)` miscompile). A body is Result-typed if ANY tail position (through
        // `if`/`match`/`let`/`do`, and one level of helper-call) is an `Ok`/`Err` application; the `Ok`
        // discriminant is a fixed lookup regardless of which branch produced it.
        // ARTIFACT ABI (ask-41 / Amendment 0.8.0) takes precedence: a body evaluating to a
        // `compile-output` record (`(record (artifacts …) (diagnostics …))`) is exported as
        // `compile: list<artifact> → compile-output`. Else a `Result`-typed body → the diagnostics
        // `result<…>` ABI (ask-40). Else the plain `list<u8> → list<u8>` seam.
        let compile_body = self.funcs[0].body.clone();
        let abi = if self.compile_body_is_artifacts(&compile_body, &mut Vec::new()) {
            CompileAbi::Artifacts
        } else if self.compile_body_is_result(&compile_body, &mut Vec::new()) {
            match self.variant_disc("Ok") {
                Ok(ok) => CompileAbi::Result(ok),
                Err(_) => CompileAbi::Bytes,
            }
        } else {
            CompileAbi::Bytes
        };

        // The compile entry runs entirely on the value-heap runtime path: its parameter is a runtime
        // `Bytes` handle and its result is a runtime `Bytes`/`Result` handle. Force both kinds to Heap
        // up front (the identity `(compile b) b` gives inference nothing to constrain `b` with) and set
        // the import-shifted call base BEFORE the compile pass, exactly like the runtime-compound path.
        self.funcs[0].param_kinds = vec![Kind::Heap];
        self.funcs[0].ret_kind = Kind::Heap;
        self.call_base = RT_FUNC_BASE;

        // Under the kinded-artifact ABI the `compile` parameter is a `list<artifact>` (ask-41), a
        // FIXED shape by the build-tool contract: `artifact = record { bytes: list<u8>, kind: string }`
        // (fields sorted by key → bytes, kind). Give the `inputs` parameter that shape so `shape_of`
        // can see through the opaque `Heap` handle and `gen_runtime_member` can project a field off a
        // projected input artifact — `(. (List.at inputs 0) bytes)`, how the compiler reads its input.
        if matches!(abi, CompileAbi::Artifacts) {
            self.compile_input_shape = Some(Shape::List(Box::new(Shape::Record(vec![
                ("bytes".to_string(), Shape::Bytes),
                ("kind".to_string(), Shape::Str),
            ]))));
        }

        let n = self.funcs.len();
        let (bodies, reachable) = self.compile_all_bodies(n)?;
        for i in 0..n {
            if reachable.0.contains(&(i as u32)) {
                if let Some(d) = reachable.1[i].clone() {
                    return Err(d);
                }
            }
        }
        // Effect-context specializations (ask-45/ask-46): a recursive effectful `handle` emits a
        // per-context monomorphization whose `call`-target is `spec_wasm_index = call_base + n_funcs +
        // n_helpers + pos`. `compile_component` appends them at `[fixed][user][helpers][SPECS][wrapper]`
        // (matching that index), so the compile entry composes with recursive effects exactly as the
        // run entry does after ask-45 — the diagnostics handler can now be installed at `compile`.
        let helper_bodies = self.helper_bodies();
        let (spec_types, n_specs, spec_bodies) = self.spec_artifacts()?;
        Ok(compile_component(
            &self.funcs,
            &bodies,
            &helper_bodies,
            &spec_types,
            n_specs,
            &spec_bodies,
            abi,
        ))
    }

    /// Compile every function body under the CURRENT `self.call_base`, run reachability from
    /// `main`, and resolve each function to a `Body`: a compiled body as-is, a trap stub for a
    /// DEAD declined function. Returns the bodies (dead-declines already stubbed) and
    /// `(reachable-set, per-function deferred decline)` — a reachable function that declined has
    /// its `Decline` preserved so the caller can either retry in runtime mode or fail. Re-runnable
    /// (idempotent per `call_base`), so the runtime-mode retry just calls it again.
    fn compile_all_bodies(
        &mut self,
        n: usize,
    ) -> Result<(Vec<Body>, (std::collections::BTreeSet<u32>, Vec<Option<Decline>>)), Decline> {
        // Reset effect-context specializations: this pass re-derives them, and `spec_wasm_index`
        // depends on the current `call_base` (which differs between the scalar pass and the
        // runtime-mode retry), so a stale spec from a prior pass would carry wrong call targets.
        self.specializations.borrow_mut().clear();
        // A recursive-call reachability edge to a specialization is recorded via its user function
        // (the spec shares the function's reachability); the reserved-slot bodies are appended after
        // the user funcs regardless, so no per-spec reachability edge is needed.
        let mut compiled: Vec<Option<Body>> = (0..n).map(|_| None).collect();
        let mut called_sets: Vec<std::collections::BTreeSet<u32>> =
            (0..n).map(|_| Default::default()).collect();
        let mut deferred: Vec<Option<Decline>> = (0..n).map(|_| None).collect();
        for i in 0..n {
            match self.compile_func(i) {
                Ok((body, kind, called)) => {
                    self.funcs[i].ret_kind = kind;
                    compiled[i] = Some(body);
                    called_sets[i] = called;
                }
                Err(d) => deferred[i] = Some(d),
            }
        }

        // Reachability closure: `main` (index 0) plus everything transitively reached by an
        // emitted runtime `call`.
        let mut reachable: std::collections::BTreeSet<u32> = Default::default();
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            if !reachable.insert(i) {
                continue;
            }
            for &c in &called_sets[i as usize] {
                stack.push(c);
            }
        }

        // Resolve: compiled body as-is; a DEAD declined function → trap stub. A reachable declined
        // function keeps its decline in `deferred` for the caller to handle (retry or fail).
        let mut bodies: Vec<Body> = Vec::with_capacity(n);
        for i in 0..n {
            match compiled[i].take() {
                Some(body) => bodies.push(body),
                None => {
                    bodies.push(self.trap_stub());
                    // A DEAD (unreachable) function's decline is normally cleared — a not-yet-supported
                    // construct in code `main` never calls is dead-code-eliminated to a trap stub, and
                    // a decline that DEPENDS on the call context (a HOF like `(def (ap g v) (g v))`
                    // whose `g` only resolves when inlined at a call site → CDZ0401 "undeclared
                    // capability: g" as a standalone fn; an effectful helper discharged only under a
                    // caller's handler) is legitimately dead and MUST NOT abort the compile.
                    // EXCEPTION: a genuine UNBOUND-NAME reject (CDZ0101) is UNCONDITIONAL — a module's
                    // every definition is an export whose body must resolve, whether or not it is
                    // reachable from `main` (core-semantics.md #Binding Is Lexical — the unbound-name
                    // rule is not gated on reachability; 02-binding-and-control.sexp §"an unbound name
                    // in an uncalled sibling definition is still rejected"). An unbound name does not
                    // become bound by any call context, so keeping it for a dead function is sound and
                    // does not touch the inlining-dependent CDZ0401/effect declines. Keep ONLY CDZ0101.
                    let is_unbound = deferred[i].as_ref().map_or(false, |d| d.code() == Some("CDZ0101"));
                    if !reachable.contains(&(i as u32)) && !is_unbound {
                        deferred[i] = None;
                    }
                }
            }
        }
        Ok((bodies, (reachable, deferred)))
    }

    fn helper_count(&self) -> usize {
        self.helpers.add as usize + self.helpers.sub as usize + self.helpers.mul as usize
    }

    fn helper_bodies(&self) -> Vec<Body> {
        let mut v = Vec::new();
        if self.helpers.add {
            v.push(checked_add_body());
        }
        if self.helpers.sub {
            v.push(checked_sub_body());
        }
        if self.helpers.mul {
            v.push(checked_mul_body());
        }
        v
    }

    /// The effect-context specializations (Stage 3), extracted as plain assembly inputs so a
    /// component-assembly function can append them without touching the RefCell registry: the
    /// concatenated per-spec functype bytes (`(orig-params…, states…) -> (ret, states…)`, matching
    /// `functype_spec`) and the encoded bodies, IN ORDER. The bodies were emitted by
    /// `gen_specialized_call` with `call`-targets at `spec_wasm_index(pos) = call_base + n_funcs +
    /// n_helpers + pos`, so an assembler MUST place them at exactly `[fixed][user][helpers][SPECS]…`,
    /// after helpers and before any trailing `run`/render func. Returns `(type_items, n_specs,
    /// bodies)`; errors if a reserved spec slot was never filled (an internal invariant break).
    fn spec_artifacts(&self) -> Result<(Vec<u8>, usize, Vec<Body>), Decline> {
        let specs = self.specializations.borrow();
        let mut type_items = Vec::new();
        let mut bodies = Vec::new();
        for s in specs.iter() {
            type_items.extend_from_slice(&functype_spec(
                &s.param_kinds,
                &s.state_kinds,
                s.ret_kind.externalized(),
            ));
            let sb = s.body.borrow();
            let body = sb
                .as_ref()
                .ok_or_else(|| Decline("specialization body never emitted".into(), None))?;
            bodies.push(Body { extra_locals: body.extra_locals.clone(), code: body.code.clone() });
        }
        Ok((type_items, specs.len(), bodies))
    }

    /// Compile function `i`'s body into a `Body` (extra locals + code).
    /// The base emit env of module-scope VALUE definitions (ask-71), as compile-time aliases in
    /// source order. Each value-def's expression captures the env of the value-defs BEFORE it, so a
    /// later value-def may reference an earlier one (`(def a 1) (def b (+ a 1))`), exactly as a
    /// sequence of `let` bindings scopes. A function's own params are pushed AFTER these, so a param
    /// of the same name shadows a module value (lexical scope). These are aliases — no runtime local,
    /// no code — so a use folds/resolves like any `let`-bound structural/scalar value.
    fn module_value_env(&self) -> Vec<Local> {
        let mut env: Vec<Local> = Vec::new();
        for (name, value) in &self.module_values {
            let captured = env.clone();
            env.push(Local::aliased(name.clone(), value.clone(), captured));
        }
        env
    }

    /// Compile function `i`'s body, returning the emitted body AND its actual result kind.
    /// The emitted kind is ground truth (inference is a best-effort pre-pass); the caller
    /// writes it back so the wasm signature matches what the body actually leaves on the stack
    /// — e.g. a body that projects a Bool record field yields Bool even if inference guessed
    /// Int64.
    fn compile_func(&self, i: usize) -> Result<(Body, Kind, std::collections::BTreeSet<u32>), Decline> {
        let arity = self.funcs[i].params.len() as u32;
        let mut ctx = FnCtx {
            next_local: arity,
            extra_locals: Vec::new(),
            called: Default::default(),
            routers: Vec::new(),
            inlining: Vec::new(),
        };
        // Module-scope VALUE definitions come first (as compile-time aliases), so every function
        // sees them (ask-71). A parameter of the SAME name shadows a module value (pushed after),
        // matching lexical scope. Built once per function; a value-def's expression captures the
        // prior value-defs' env so a later value-def may reference an earlier one.
        let mut env: Vec<Local> = self.module_value_env();
        // Parameters occupy locals 0..arity with their kinds. The `compile` entry (func 0) under the
        // kinded-artifact ABI gets its single `inputs` parameter shaped as the fixed `list<artifact>`
        // (ask-41), so field projection off an input artifact resolves (see `compile_input_shape`).
        env.extend(
            self.funcs[i]
                .params
                .iter()
                .cloned()
                .zip(self.funcs[i].param_kinds.iter().cloned())
                .enumerate()
                .map(|(idx, (name, kind))| {
                    let shape = if i == 0 && idx == 0 {
                        self.compile_input_shape.clone()
                    } else {
                        None
                    };
                    Local::scalar_shaped(name, idx as u32, kind, shape)
                }),
        );
        let body = self.funcs[i].body.clone();
        let (code, kind) = self.emit(&body, &env, &mut ctx)?;
        // A function whose body is a DEFINITE trap (`Kind::Never` — e.g. a `resolve`'s PUnknown arm
        // `(KConst (Bytes.len (Bytes.of (list 256))))`, whose out-of-range byte const-folds to a
        // trap) always diverges. Its emitted body can be a malformed byte sequence in a
        // runtime-heap context (a `Bytes.len` over an already-`unreachable` argument, etc.) that
        // fails wasm validation — so replace it with a single clean `unreachable`. Its signature
        // keeps the externalized (Never→i64) result; `unreachable` is stack-polymorphic and validates
        // against any signature, and the function never returns anyway. This mirrors `trap_stub` (for
        // a DEAD function) — here the function is LIVE but divergent. Fixes the final self-host
        // blocker (Tier 2f): the compiler's `resolve` has such a trapping arm, and without this its
        // whole body emitted invalid bytes on the runtime-compound path, poisoning every call.
        if kind == Kind::Never {
            // Keep the function's PRE-INFERRED external kind for its signature (a `resolve` whose
            // body always traps was still inferred to return `Heap`/the Core sum), so callers
            // compiled before this body — the fixpoint compiles in index order — read a STABLE
            // signature and their `call` result type matches. The body is a single `unreachable`
            // (`trap_stub`): the function always diverges, so no value is produced, and
            // `unreachable` is stack-polymorphic (validates against any declared result). Reporting
            // `Never` here would flip the signature to i64 mid-fixpoint and mismatch an
            // already-emitted caller (the Tier 2f INVALID "expected i32 found i64"). A genuinely
            // Never-typed function (never inferred otherwise) keeps `Never`→i64, unchanged.
            let sig_kind = self.funcs[i].ret_kind;
            let reported = if sig_kind == Kind::Never { Kind::Never } else { sig_kind };
            return Ok((self.trap_stub(), reported, ctx.called));
        }
        Ok((Body { extra_locals: ctx.extra_locals, code }, kind, ctx.called))
    }

    /// A trap stub for a function that is never reached by a runtime call — its every call site
    /// was const-folded away (e.g. a `parse`/`classify` whose result the caller matches at
    /// compile time). Such a function cannot run, so its body is a single `unreachable`; its
    /// wasm signature keeps the inferred kinds (a valid type is all a never-called function
    /// needs — `unreachable` is stack-polymorphic and validates against any result). This is
    /// dead-function elimination: a sum-returning helper that has no scalar lowering is *dead*,
    /// not a compile failure, once every use of it folds to a constant.
    fn trap_stub(&self) -> Body {
        Body { extra_locals: Vec::new(), code: vec![op::UNREACHABLE] }
    }

    // ─── Code generation ───────────────────────────────────────────────────────────

    /// Emit code for `node`, returning its bytes and result kind.
    fn emit(&self, node: &Node, env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        match node {
            Node::Int(n) => {
                let mut c = vec![op::I64_CONST];
                sleb128(*n, &mut c);
                Ok((c, Kind::Int64))
            }
            Node::Bool(b) => Ok((vec![op::I32_CONST, if *b { 1 } else { 0 }], Kind::Bool)),
            Node::Float(f) => {
                let mut c = vec![op::F64_CONST];
                c.extend_from_slice(&f.to_le_bytes());
                Ok((c, Kind::Float64))
            }
            Node::Str(s) => self.gen_runtime_string_literal(s, ctx),
            Node::Name(n) => self.gen_name(n, env, ctx),
            Node::List(elems) => self.gen_list(elems, env, ctx),
        }
    }

    /// Is this node a *structural* (non-scalar) value — a record, tuple, or constructor
    /// application — that lives only at compile time? Such a value is bound in `let` as an
    /// alias and consumed by `match` / member access / tuple access, never materialized as a
    /// runtime scalar.
    fn is_structural(&self, node: &Node, env: &[Local]) -> bool {
        match node {
            Node::Str(_) => true,
            // A bare constructor name (`None`, `Some`) is a Constructor VALUE — structural, so
            // `(let ((ctor None)) …)` binds it as an alias to be applied later.
            Node::Name(n) if is_constructor_name(n) => true,
            Node::Name(n) => {
                // A name aliased to a structural value is itself structural.
                env.iter().rev().find(|l| l.name == *n).map_or(false, |l| {
                    l.alias.as_ref().map_or(false, |(node, e)| self.is_structural(node, e))
                })
            }
            Node::List(elems) => {
                // A constructor application is a structural sum value — whether its head is a bare
                // `Some` (`(Some 5)`) or a QUALIFIED `Type.Variant` (`(IntList.Cons …)`, which the
                // reader expands to `((. IntList Cons) …)` — a `.`-list head). `constructor_of`
                // recognizes both, so a program-declared sum's qualified constructor bound in a
                // `let` (`(let ((xs (IntList.Cons …))) …)`) is aliased as a compile-time structure
                // rather than emitted as a runtime dotted-application (which has no lowering).
                if constructor_of(elems.first()).is_some() {
                    return true;
                }
                match elems.first() {
                    Some(Node::Name(h)) => {
                        matches!(
                            h.as_str(),
                            "record" | "tuple" | "list" | "map" | "quote" | "quasiquote" | "fn"
                        )
                        // A partial application producing a lambda is a structural (compile-
                        // time) value: bind it as an alias so its call site inlines it.
                        || self.resolve_lambda(node, env).is_some()
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Does this binding alias a lambda (`(fn …)`)? Lambdas are compile-time values inlined
    /// at their call sites (compile-time beta reduction), never materialized as runtime
    /// closures — sufficient for every statically-known lambda the realized corpus uses.
    fn is_lambda_alias(&self, l: &Local) -> bool {
        match &l.alias {
            Some((node, e)) => self.resolve_lambda(node, e).is_some(),
            None => false,
        }
    }

    /// Resolve a node to a lambda `(remaining-params, body, captured-env)` if it is one
    /// (following name aliases). Handles a bare `(fn …)`, and a *partial application* of a
    /// multi-parameter lambda — `(add 3)` where `add = (fn (x y) …)` resolves to the residual
    /// lambda over `y` with `x` bound to `3` — so currying works by compile-time reduction.
    fn resolve_lambda(&self, node: &Node, env: &[Local]) -> Option<(Vec<String>, Node, Vec<Local>)> {
        match node {
            Node::List(items) if name_of(items.first()) == Some("fn") => {
                let params = match items.get(1)? {
                    Node::Name(p) => vec![p.clone()],
                    Node::List(ps) => ps.iter().filter_map(|p| match p {
                        Node::Name(n) => Some(n.clone()),
                        _ => None,
                    }).collect(),
                    _ => return None,
                };
                let body = items.last()?.clone();
                Some((params, body, env.to_vec()))
            }
            // See through a `let` (binding its bindings into the captured env — this is how a
            // closure captures its creation scope) and a `:` annotation, to a lambda. These
            // are checked before the generic partial-application arm since `let`/`:` are
            // special forms, not callees.
            Node::List(items) if name_of(items.first()) == Some("let") => {
                let binds = match items.get(1) {
                    Some(Node::List(b)) => b,
                    _ => return None,
                };
                let mut inner = env.to_vec();
                for pair in binds {
                    if let Node::List(p) = pair {
                        if let Some(Node::Name(name)) = p.first() {
                            inner.push(Local::aliased(name.clone(), p[1].clone(), inner.clone()));
                            continue;
                        }
                    }
                    return None;
                }
                self.resolve_lambda(items.last()?, &inner)
            }
            Node::List(items) if name_of(items.first()) == Some(":") => {
                self.resolve_lambda(items.get(1)?, env)
            }
            // A member projection `(. record field)` that yields a lambda (a module export
            // function reached by member access): resolve the record, project the field.
            Node::List(items) if name_of(items.first()) == Some(".") => {
                let field = name_of(items.get(2))?;
                let obj_node = items.get(1)?;
                let (obj, oenv) = self.resolve(obj_node, env)?;
                if let Node::List(rec) = &obj {
                    if name_of(rec.first()) == Some("record") {
                        // Make every export mutually visible in each export's body: bind each
                        // export NAME to the projection `(. obj name)`, captured under the site
                        // env (where the module is in scope). A body that calls a sibling — or
                        // recurses on itself — resolves that name to a re-projection from the
                        // same module record, so intra-module references work exactly as
                        // top-level defs are mutually visible (11-modules.sexp §"a module
                        // function calls a sibling export", §"a module function is recursive").
                        // Re-projecting each time is self-sustaining, so no cyclic environment
                        // is needed; a bounded recursion terminates by const-folding at its base
                        // case (see `gen_if`, which drops the dead branch of a constant `if`).
                        let mut sib_env = oenv.clone();
                        for entry in &rec[1..] {
                            if let Node::List(kv) = entry {
                                if let Some(Node::Name(fname)) = kv.first() {
                                    let proj = Node::List(vec![
                                        Node::Name(".".into()),
                                        obj_node.clone(),
                                        Node::Name(fname.clone()),
                                    ]);
                                    sib_env.push(Local::aliased(fname.clone(), proj, env.to_vec()));
                                }
                            }
                        }
                        for entry in &rec[1..] {
                            if let Node::List(kv) = entry {
                                if name_of(kv.first()) == Some(field) {
                                    return self.resolve_lambda(&kv[1], &sib_env);
                                }
                            }
                        }
                    }
                }
                None
            }
            // A partial application `(f a b …)` where `f` is not a special form: resolve `f`,
            // bind the given args, return the residual lambda over the remaining parameters.
            Node::List(items) if items.len() >= 2 && !is_special_form_head(items.first()) => {
                let (params, body, captured) = self.resolve_lambda(&items[0], env)?;
                let args = &items[1..];
                if args.len() >= params.len() {
                    return None; // full/over-application is not a lambda value
                }
                let mut body_env = captured;
                for (p, a) in params.iter().zip(args.iter()) {
                    body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
                }
                let remaining = params[args.len()..].to_vec();
                Some((remaining, body, body_env))
            }
            Node::Name(n) => {
                // A local alias to a lambda?
                if let Some(local) = env.iter().rev().find(|l| l.name == *n) {
                    if let Some((anode, aenv)) = local.alias.as_ref() {
                        return self.resolve_lambda(anode, aenv);
                    }
                }
                // A NAMED DEF is a lambda too: `(def (f x y) body)` ≡ `(fn (x y) body)`, so a
                // named def can be partially applied / passed as a value exactly as a lambda
                // can (spec: `(f a b)` and `((f a) b)` are the same program). Resolving it here
                // inlines it (compile-time beta reduction) — sound for the pure, non-recursive
                // uses the corpus exercises as values.
                self.lookup_fn(n).map(|f| (f.params.clone(), f.body.clone(), Vec::new()))
            }
            _ => None,
        }
    }

    /// Apply a callee to arguments by compile-time inlining. The callee must resolve to a
    /// lambda; currying is handled by peeling parameters — extra args re-apply the result,
    /// too-few args would need a runtime closure (declined). Arguments are bound as aliases
    /// so they inline into the body (matching the corpus's pure, statically-known lambdas).
    fn gen_apply(
        &self,
        callee: &Node,
        args: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // A curried callee `((adder 10) 5)`: the callee is itself an application. Resolve it
        // to a lambda by inlining the inner application's body as this lambda.
        let (params, body, captured) = match self.resolve_lambda(callee, env) {
            Some(l) => l,
            None => {
                // The callee may be a nested application `(adder 10)` producing a lambda.
                if let Node::List(inner) = callee {
                    // Emit the inner application to a lambda value: resolve it structurally.
                    if let Some(l) = self.resolve_apply_to_lambda(inner, env)? {
                        l
                    } else {
                        return decline("callee is not a compile-time-resolvable lambda");
                    }
                } else {
                    return decline("callee is not a lambda");
                }
            }
        };
        if args.len() < params.len() {
            return decline("partial application (needs a runtime closure)");
        }
        // Bind each parameter to its argument node as an alias, so uses inline.
        let mut body_env = captured;
        for (p, a) in params.iter().zip(args.iter()) {
            body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
        }
        // Emit the body; any surplus args apply to its result (curried return).
        if args.len() == params.len() {
            self.emit(&body, &body_env, ctx)
        } else {
            // (fn (x) (fn (y) …)) applied to 2 args, etc.: emit body as a callee re-applied.
            let rest = &args[params.len()..];
            // The body must itself resolve to a lambda taking the remaining args.
            let mut applied = vec![body.clone()];
            applied.extend_from_slice(rest);
            self.gen_apply(&body, rest, &body_env, ctx).or_else(|_| {
                let _ = &applied;
                decline("curried over-application not resolvable")
            })
        }
    }

    /// Resolve an application node `(f a …)` to the lambda it yields, if `f` is a lambda
    /// whose body (after binding params to args) is itself a lambda. Handles `(adder 10)`
    /// returning `(fn (x) (+ x n))`.
    fn resolve_apply_to_lambda(
        &self,
        app: &[Node],
        env: &[Local],
    ) -> Result<Option<(Vec<String>, Node, Vec<Local>)>, Decline> {
        let callee = &app[0];
        let args = &app[1..];
        let (params, body, captured) = match self.resolve_lambda(callee, env) {
            Some(l) => l,
            None => {
                if let Node::List(inner) = callee {
                    match self.resolve_apply_to_lambda(inner, env)? {
                        Some(l) => l,
                        None => return Ok(None),
                    }
                } else {
                    return Ok(None);
                }
            }
        };
        if args.len() != params.len() {
            return Ok(None);
        }
        let mut body_env = captured;
        for (p, a) in params.iter().zip(args.iter()) {
            body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
        }
        Ok(self.resolve_lambda(&body, &body_env))
    }

    // ─── Compile-time constant evaluation ───────────────────────────────────────────
    //
    // Structural values — records, tuples, sums, lists, bytes, strings — have no runtime
    // representation in this scalar compiler. Where an operation *consumes* such a value and
    // produces a SCALAR or a TRAP (equality → Bool; length/index → Int; out-of-range → trap),
    // the only correct lowering is to evaluate it at compile time and emit the scalar literal
    // (or `unreachable` for a trap). This is the same compile-time-resolution discipline the
    // `match`/member/tuple-access paths already use — not transcript modeling: it is forced
    // by the absence of a runtime representation for the operand. Scalar arithmetic remains
    // ordinary runtime codegen.

    /// Evaluate `node` to a compile-time value, if it lies in the pure constant fragment.
    /// Returns `Ok(Some(v))` for a value, `Ok(None)` if outside the fragment (caller falls
    /// back to codegen), or `Err(Trap)` if it definitely traps.
    fn eval_const(&self, node: &Node, env: &[Local]) -> Result<Option<CVal>, ConstTrap> {
        match node {
            Node::Int(n) => Ok(Some(CVal::Int(*n))),
            Node::Bool(b) => Ok(Some(CVal::Bool(*b))),
            Node::Float(f) => Ok(Some(CVal::Float(*f))),
            Node::Str(s) => Ok(Some(CVal::Str(s.clone()))),
            Node::Name(n) if n == "unit" => Ok(Some(CVal::unit())),
            Node::Name(n) if n == "nan" || n == "NaN" => Ok(Some(CVal::Float(f64::NAN))),
            Node::Name(n) => match env.iter().rev().find(|l| l.name == *n) {
                Some(l) => match &l.alias {
                    Some((anode, aenv)) => self.eval_const(anode, aenv),
                    None => Ok(None), // a runtime scalar local — not a compile-time constant
                },
                // A bare NULLARY constructor used as a VALUE (`None`, `NNil`, `Zero`) is the nullary
                // sum value — equivalent to `(Ctor unit)` (core-semantics.md #A Sum Type Constructor
                // Is A Single-Arity Function: a nullary variant's argument type is Unit). The corpus
                // usually writes `(None unit)`, but the bare form is natural (a reader building `NNil`
                // for an empty node), so fold it to the same Sum value. Not shadowed by a local.
                None if self.nullary_variants.contains(variant_tag(n)) => Ok(Some(CVal::Sum {
                    variant: n.clone(),
                    payload: Box::new(CVal::unit()),
                })),
                None => Ok(None),
            },
            // `()` — a form with zero elements — is the empty TUPLE, which is unit
            // (core-semantics.md §unit is the empty tuple). This is distinct from an empty
            // *list* `(list)`, which has a `list` head and folds to `CVal::List([])`, and
            // from an empty tuple written `(tuple)`, which has a `tuple` head — all three
            // land on the correct value, only `()` is unit.
            Node::List(items) if items.is_empty() => Ok(Some(CVal::unit())),
            Node::List(items) => self.eval_const_list(items, env),
        }
    }

    fn eval_const_list(&self, items: &[Node], env: &[Local]) -> Result<Option<CVal>, ConstTrap> {
        // A malformed form is not a constant — bail to `Ok(None)` (not foldable) so codegen's
        // `check_arity` rejects it, rather than indexing past a short form and panicking.
        if check_arity(items).is_err() {
            return Ok(None);
        }
        let head = match items.first() {
            Some(Node::Name(h)) => h.as_str(),
            // ((. obj field) arg) — a dotted intrinsic (Int.to-byte etc.) over a constant.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".") => {
                return self.eval_const_dotted(items, env);
            }
            _ => return Ok(None),
        };
        // Application-head resolution is LEXICAL (core-semantics.md #Binding Is Lexical): a name
        // in head position resolves to the nearest enclosing binding BEFORE the built-in
        // CONSTRUCTOR form of the same spelling. So `(let ((list (fn (a b) (+ a b)))) (list 3 4))`
        // applies the bound lambda (→ 7), NOT the built-in list constructor (→ `(list 3 4)`). The
        // bare-NAME arm of `eval_const` already consults `env` first; this is the symmetric guard
        // for a compound-constructor FORM head — a `tuple`/`list`/`record`/`map` head shadowed by
        // a local is not the built-in form here, so bail to `Ok(None)` and let the emit path
        // resolve the binding (its lambda-alias check inlines it). Without this the folder builds a
        // `CVal::List` for `(list 3 4)`, and main's result-kind inference then emits a
        // runtime-compound (list) component for a program that must return the scalar 7 — a
        // miscompile (02-binding-and-control.sexp §"a let binding shadows a built-in constructor
        // name in application-head position"). Scoped to the four constructor keywords so a name
        // bound to a CONSTRUCTOR value (`(let ((ctor None)) (ctor unit))`, head `ctor`) still folds
        // through the constructor-application arm below — that is not a built-in-form shadow.
        if matches!(head, "tuple" | "list" | "record" | "map")
            && env.iter().any(|l| l.name == head)
        {
            return Ok(None);
        }
        match head {
            "tuple" => self.eval_const_seq(&items[1..], env).map(|o| o.map(CVal::Tuple)),
            "list" => self.eval_const_seq(&items[1..], env).map(|o| o.map(CVal::List)),
            // A RECORD's keys are field-name LABELS (a fixed set fixed by the form): keyed by
            // `String`, sorted by name. A record and a map are DISTINCT types (never compare equal,
            // a map is not member-projectable).
            "record" => {
                let mut fields = Vec::new();
                for entry in &items[1..] {
                    if let Node::List(kv) = entry {
                        // Bounds-check the value node: a malformed entry `(a)` names a field but
                        // carries no value. Treat it as not-constant-foldable (`Ok(None)`) so the
                        // body falls through to the scalar path where `check_type_rejections`
                        // rejects it CDZ0201 — never index `kv[1]` on a 1-element entry (the
                        // never-crash guard; 07-type-system.sexp §"a record/map … with no value").
                        if let (Some(Node::Name(k)), Some(vnode)) = (kv.first(), kv.get(1)) {
                            match self.eval_const(vnode, env)? {
                                Some(v) => fields.push((k.clone(), v)),
                                None => return Ok(None),
                            }
                            continue;
                        }
                    }
                    return Ok(None);
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Some(CVal::Record(fields)))
            }
            // A MAP's keys are VALUES (collections-and-text.md §Keys Are Compared By Value), so each
            // key is a const-foldable value node — an int (`(map (1 10))`), a string, etc., NOT just a
            // name. Fold both key and value; sort by the KEY's canonical text (the deterministic
            // order-independent form) so equality and rendering are insertion-order-independent, and
            // de-dup (a repeated key is caught upstream by CDZ0201, but keep the last-writer here for
            // safety). A key that does not fold to a value → not a constant (Ok(None)).
            "map" => {
                let mut entries: Vec<(CVal, CVal)> = Vec::new();
                for entry in &items[1..] {
                    if let Node::List(kv) = entry {
                        if let (Some(knode), Some(vnode)) = (kv.first(), kv.get(1)) {
                            // Fold the KEY as a value (an int `(map (1 10))`, a string, a bound
                            // name). A bare name in key position that does NOT resolve to a bound
                            // value is a SYMBOLIC key — its spelling as a String value (a String
                            // is a value of one key type, satisfying §Keys Are Compared By Value).
                            // This is the `(map (a 1) (b 2))` idiom: `a`/`b` are symbol-like keys,
                            // not variable references. A bound name still folds to its value, so
                            // `(let ((k 5)) (map (k 10)))` keys by 5, not "k".
                            let k = match self.eval_const(knode, env)? {
                                Some(k) => k,
                                None => match knode {
                                    Node::Name(sym) => CVal::Str(sym.clone()),
                                    _ => return Ok(None),
                                },
                            };
                            match self.eval_const(vnode, env)? {
                                Some(v) => {
                                    // Last-writer-wins on a duplicate key (compared by value).
                                    if let Some(slot) = entries.iter_mut().find(|(ek, _)| cval_eq(ek, &k)) {
                                        slot.1 = v;
                                    } else {
                                        entries.push((k, v));
                                    }
                                }
                                None => return Ok(None),
                            }
                            continue;
                        }
                    }
                    return Ok(None);
                }
                // A map associates keys of ONE type with values of ONE type (collections-and-text.md
                // #A Map Associates Keys With Values). If the folded entries are HETEROGENEOUS in key
                // or value type, this is an ill-typed map — do NOT fold it to a `CVal::Map` (which
                // would silently build a heterogeneous-key/value map even when the literal is merely
                // constructed/returned, bypassing the emit-path homogeneity check). Return `Ok(None)`
                // so the form falls through to the emit path, where `check_type_rejections`'s `map`
                // arm issues the CDZ0201 (the KEY/VALUE homogeneity checks there). This is what makes
                // `(let ((j 5)) (let ((k true)) (map (j 1) (k 2))))` — keys Int64/Bool, fully foldable —
                // reject on CONSTRUCTION, matching the value-homogeneity case whose unbound keys keep
                // it un-folded (05-compound-types.sexp §"a map literal with keys of two different types
                // is a type error" + its value companion).
                if let Some((first_k, first_v)) = entries.first() {
                    let kt = StaticType::of_cval(first_k);
                    let vt = StaticType::of_cval(first_v);
                    let mixed = entries.iter().any(|(k, v)| {
                        StaticType::of_cval(k) != kt || StaticType::of_cval(v) != vt
                    });
                    if mixed {
                        return Ok(None);
                    }
                }
                entries.sort_by(|a, b| cval_canonical_key(&a.0).cmp(&cval_canonical_key(&b.0)));
                Ok(Some(CVal::Map(entries)))
            }
            "=" => {
                let a = self.eval_const(&items[1], env)?;
                let b = self.eval_const(&items[2], env)?;
                match (a, b) {
                    (Some(x), Some(y)) => Ok(Some(CVal::Bool(cval_eq(&x, &y)))),
                    _ => Ok(None),
                }
            }
            // Integer arithmetic / comparison over constants folds to a scalar — so an
            // unquoted arithmetic expression `,(+ 1 1)` embeds its VALUE (2), and constant
            // folding matches the emitted runtime result. Overflow/div-by-zero is a trap.
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" if items.len() == 3 => {
                match (self.eval_const(&items[1], env)?, self.eval_const(&items[2], env)?) {
                    (Some(CVal::Int(a)), Some(CVal::Int(b))) => {
                        Ok(Some(CVal::Int(fold_int_op(head, a, b)?)))
                    }
                    _ => Ok(None),
                }
            }
            "<" | ">" | "<=" | ">=" if items.len() == 3 => {
                match (self.eval_const(&items[1], env)?, self.eval_const(&items[2], env)?) {
                    (Some(CVal::Int(a)), Some(CVal::Int(b))) => Ok(Some(CVal::Bool(match head {
                        "<" => a < b,
                        ">" => a > b,
                        "<=" => a <= b,
                        _ => a >= b,
                    }))),
                    // Bool carries a total order in which false < true (core-semantics.md
                    // #Ordering Where Offered Is Total). Rust's `bool` Ord is exactly false < true,
                    // so the operators fold directly (03-equality-and-observation.sexp §"false is
                    // less than true", §"a boolean is less-than-or-equal to itself").
                    (Some(CVal::Bool(a)), Some(CVal::Bool(b))) => Ok(Some(CVal::Bool(match head {
                        "<" => a < b,
                        ">" => a > b,
                        "<=" => a <= b,
                        _ => a >= b,
                    }))),
                    _ => Ok(None),
                }
            }
            "if" => {
                match self.eval_const(&items[1], env)? {
                    Some(CVal::Bool(true)) => self.eval_const(&items[2], env),
                    Some(CVal::Bool(false)) => self.eval_const(&items[3], env),
                    _ => Ok(None),
                }
            }
            // Boolean connectives fold through their `if` desugaring — SHORT-CIRCUIT, so the right
            // operand is evaluated only on the branch the left operand selects. This preserves the
            // shielding property in the constant folder too: `(and false <trap>)` folds to `false`
            // without evaluating `<trap>` (matching `(if false <trap> false)`), never a ConstTrap.
            "and" | "or" | "not" if check_arity(items).is_ok() => {
                self.eval_const(&desugar_connective(head, items), env)
            }
            // `let` / `do` / `match` / member / annotation whose RESULT is a compound value:
            // evaluate them as constants so a compound-returning body (e.g. a match that picks
            // a string arm) folds. These mirror the emit paths but produce a `CVal`.
            "let" => {
                let binds = match items.get(1) {
                    Some(Node::List(b)) => b,
                    _ => return Ok(None),
                };
                let mut inner = env.to_vec();
                for pair in binds {
                    if let Node::List(p) = pair {
                        if let Some(Node::Name(name)) = p.first() {
                            // EAGERLY fold a COMPUTED binding value (a `(…)` application) to a constant
                            // ONCE, and memoize it as a literal node alias. A `let` binding is
                            // referenced many times in a body (`(let ((root (mod-root b))) …)` — `root`
                            // used in every child read); the default lazy alias re-evaluates the value
                            // NODE on EACH reference, so a body with nested lets over a threaded value
                            // re-folds the same sub-expression combinatorially — the compile-time
                            // blowup a large reader hit (a self-hosting `compile-bytes <literal>` fold
                            // went from seconds to >minutes as the reader grew). Folding once and
                            // binding the resulting constant (round-tripped via `cval_to_node`)
                            // collapses that to linear.
                            //
                            // Only a `Node::List` value (a computed application) is eager-folded: a
                            // bare-NAME value (`(let ((ctor None)) …)`) must stay a lazy alias to its
                            // raw node — `None` is an UNAPPLIED constructor VALUE, and folding it to the
                            // Sum `(None unit)` then re-applying `(ctor unit)` would double-apply it
                            // (core-semantics.md #The Prelude Binds Constructor Values Only). A bare
                            // name also costs nothing to re-resolve (an O(1) lookup, not a re-fold), so
                            // there is no blowup to fix there. Fall back to the lazy alias when a
                            // computed value is not a compile-time constant (a runtime local → `None`),
                            // traps, or is not node-representable — so the runtime path is unchanged.
                            let memoized = if matches!(&p[1], Node::List(_)) {
                                match self.eval_const(&p[1], &inner) {
                                    Ok(Some(v)) => cval_to_node(&v),
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            match memoized {
                                Some(node) => {
                                    inner.push(Local::aliased(name.clone(), node, Vec::new()));
                                }
                                None => {
                                    inner.push(Local::aliased(
                                        name.clone(),
                                        p[1].clone(),
                                        inner.clone(),
                                    ));
                                }
                            }
                            continue;
                        }
                    }
                    return Ok(None);
                }
                self.eval_const(items.last().unwrap(), &inner)
            }
            "do" => self.eval_const(items.last().unwrap(), env),
            ":" => self.eval_const(&items[1], env),
            "match" => self.eval_const_match(items, env),
            // Member access `(. record field)` over a constant record — `field` is an export
            // name or a `(meta KEY)` metadata access (mapped to the reserved manifest key).
            "." => {
                // Scalar prelude CONSTANTS (`Int64.max`/`.min`) fold to their value BEFORE resolving
                // the object — `Int64` now resolves to its module RECORD (of function builtin-refs),
                // which does NOT list `max`/`min`, so without this short-circuit the record-projection
                // below would take `max` as a missing field and wrongly trap. Mirrors `gen_member`.
                if let (Some("Int64"), Some("max")) = (name_of(items.get(1)), name_of(items.get(2))) {
                    return Ok(Some(CVal::Int(i64::MAX)));
                }
                if let (Some("Int64"), Some("min")) = (name_of(items.get(1)), name_of(items.get(2))) {
                    return Ok(Some(CVal::Int(i64::MIN)));
                }
                // `Map.empty` is the empty-map VALUE (not a function): it folds to an empty map
                // (collections-and-text.md §A Map Is Built By Functional Construction). The `Map.*`
                // FUNCTIONS (`insert`/`lookup`/`remove`/`size`) are applied and fold in
                // `eval_const_dotted`; only the bare `empty` value is handled here.
                if let (Some("Map"), Some("empty")) = (name_of(items.get(1)), name_of(items.get(2))) {
                    return Ok(Some(CVal::Map(Vec::new())));
                }
                let meta_key = meta_field_key(&items[2]);
                let field = match meta_key.as_deref().or_else(|| name_of(items.get(2))) {
                    Some(f) => f,
                    None => return Ok(None),
                };
                // Project at the NODE level: resolve the object to its record form and fold ONLY
                // the requested field. Folding the whole record would require every field to be
                // constant, but a module record's export fields are lambdas (`(fn …)`, not
                // constants) — reading a data field (e.g. `(meta capabilities)`) must not depend
                // on the sibling lambda fields folding.
                if let Some((obj_node, obj_env)) = self.resolve(&items[1], env) {
                    if let Node::List(rec) = &obj_node {
                        if name_of(rec.first()) == Some("record") {
                            for entry in &rec[1..] {
                                if let Node::List(kv) = entry {
                                    if name_of(kv.first()) == Some(field) {
                                        return self.eval_const(&kv[1], &obj_env);
                                    }
                                }
                            }
                            return Err(ConstTrap); // missing field traps
                        }
                    }
                }
                // Fall back to folding the object as a whole (a non-module constant record).
                match self.eval_const(&items[1], env)? {
                    Some(CVal::Record(fields)) => {
                        match fields.into_iter().find(|(k, _)| k == field) {
                            Some((_, v)) => Ok(Some(v)),
                            None => Err(ConstTrap), // missing field traps
                        }
                    }
                    Some(_) => Err(ConstTrap), // member access on a non-record traps
                    None => Ok(None),
                }
            }
            // Positional tuple access `(tuple.N t)` over a constant tuple.
            _ if head.starts_with("tuple.") => {
                let idx: usize = match head[6..].parse() {
                    Ok(i) => i,
                    Err(_) => return Ok(None),
                };
                match self.eval_const(&items[1], env)? {
                    Some(CVal::Tuple(elems)) => match elems.into_iter().nth(idx) {
                        Some(v) => Ok(Some(v)),
                        None => Err(ConstTrap),
                    },
                    Some(_) => Err(ConstTrap),
                    None => Ok(None),
                }
            }
            // `(quote X)` / `(quasiquote X)` build an AST value. `CVal::Ast` carries the
            // canonical quoted NODE (so `Ast.encode`/`decode` round-trip on the real AST); the
            // conversion to `Ast.*`-constructor form happens only when matching (`quote_to_ast`).
            "quote" => Ok(self.quote_node(&items[1], env, 0).map(CVal::Ast)),
            "quasiquote" => Ok(self.quote_node(&items[1], env, 1).map(CVal::Ast)),
            // A nominal record constructor `(Point (x 0) (y 0))` — a capitalized head whose
            // args are ALL labeled `(field value)` pairs — is a structural record carrying a
            // compile-time name tag. The tag lives only in the type system, so at the value
            // level this IS the structural record `{x:0, y:0}`; the nominal name is dropped,
            // and two same-shape nominal records compare equal in the dynamic seed
            // (type-system.md §A Nominal Record Is A Structural Record Carrying A Name Tag).
            _ if is_constructor_name(head)
                && items.len() > 1
                && items[1..].iter().all(is_labeled_field) =>
            {
                let mut fields = Vec::new();
                for entry in &items[1..] {
                    if let Node::List(kv) = entry {
                        if let Some(Node::Name(k)) = kv.first() {
                            match self.eval_const(&kv[1], env)? {
                                Some(v) => fields.push((k.clone(), v)),
                                None => return Ok(None),
                            }
                        }
                    }
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Some(CVal::Record(fields)))
            }
            // A constructor application `(Some 42)` / `(Sign.Zero unit)` builds a sum value — UNLESS
            // the capitalized name is bound to a user FUNCTION. A `(def (Foo x) …)` binds `Foo` in
            // module scope, and a name resolves to its nearest lexical binding (core-semantics.md
            // #Binding Is Lexical), so `(Foo 10)` INVOKES that function, it is not an ad-hoc
            // constructor synthesizing `(Foo 10)`. Capitalization is not a binding-precedence rule
            // (09-functions.sexp §"a function whose name is capitalized is called, not treated as a
            // constructor"). A user def is not const-folded here (a call has no constant value in this
            // folder) — fall through so the emit path calls it.
            _ if is_constructor_name(head) && self.lookup_fn(head).is_none() => {
                let payload = match items.get(1) {
                    Some(p) => match self.eval_const(p, env)? {
                        Some(v) => v,
                        None => return Ok(None),
                    },
                    None => CVal::unit(),
                };
                Ok(Some(CVal::Sum { variant: head.to_string(), payload: Box::new(payload) }))
            }
            // `(ctor arg)` where `ctor` is a name bound to a bare constructor (`(let ((ctor
            // None)) (ctor unit))`) — the prelude binds a Constructor VALUE, applied here to
            // build the Sum. Resolve the head-name's alias to the constructor tag.
            _ if items.len() == 2 => {
                if let Some(local) = env.iter().rev().find(|l| l.name == head) {
                    if let Some((Node::Name(ctor), _)) = &local.alias {
                        if is_constructor_name(ctor) {
                            let payload = match self.eval_const(&items[1], env)? {
                                Some(v) => v,
                                None => return Ok(None),
                            };
                            return Ok(Some(CVal::Sum {
                                variant: ctor.clone(),
                                payload: Box::new(payload),
                            }));
                        }
                    }
                }
                Ok(None)
            }
            // A call to a NON-RECURSIVE user function whose arguments all const-fold: beta-reduce it
            // — bind each parameter to its argument's CVal (as a literal alias) and fold the body. So
            // a pure helper applied to constants folds to its value: `(eq2 "foo" "foo")` on `(def (eq2
            // a b) (= a b))` folds through the body's `(= a b)` to `true` (03-equality-and-observation
            // .sexp §"two runtime strings compare equal by their contents" — the compiler-authored
            // name-dispatch idiom `(eq2 x y)` over two runtime Strings). This is the value-level twin
            // of the resolve-based structural reduction (ask-65): a statically-applied pure function's
            // result IS a constant, foldable exactly like a `let`/`if`-selected one, so it does not
            // require the not-yet-emitted runtime heap-walk comparator.
            //
            // NARROWLY GUARDED (a broad beta-reduce here regressed the CBOR/reader helpers to an
            // INVALID component — folding a deep Bytes/`match`/OOB-access helper produces a CVal the
            // emit path then mis-lowers): fold ONLY when the result is a SCALAR (`Int`/`Bool`/`Float`/
            // `Unit`) AND every argument is itself a scalar or a STRING literal. That covers the
            // equality-of-runtime-values idiom (String/scalar args → a Bool result) while excluding
            // the Bytes/heap-threading helpers whose partial fold destabilizes emission. Also guarded
            // on `!fn_is_recursive` (a recursive callee never terminates under eager folding).
            _ if self.lookup_fn(head).is_some() && !self.fn_is_recursive(head) => {
                let f = self.lookup_fn(head).unwrap();
                if items.len() - 1 != f.params.len() {
                    return Ok(None); // arity handled (rejected/declined) at emit; not a constant here
                }
                let mut call_env: Vec<Local> = Vec::new();
                for (p, arg) in f.params.iter().zip(&items[1..]) {
                    let v = match self.eval_const(arg, env)? {
                        Some(v) => v,
                        None => return Ok(None), // a runtime argument → emit-path call, not a fold
                    };
                    // Only SCALAR or STRING arguments (the equality idiom); a Bytes/tuple/list/sum/map
                    // argument routes to the emit path (folding a Bytes/heap helper mis-lowers).
                    if !matches!(v, CVal::Int(_) | CVal::Bool(_) | CVal::Float(_) | CVal::Str(_)) {
                        return Ok(None);
                    }
                    let node = match cval_to_node(&v) {
                        Some(n) => n,
                        None => return Ok(None),
                    };
                    call_env.push(Local::aliased(p.clone(), node, Vec::new()));
                }
                match self.eval_const(&f.body.clone(), &call_env)? {
                    // Only a SCALAR result is safe to substitute for the call (a compound result would
                    // need the emit path's heap construction; a Bytes result mis-lowers as above).
                    Some(v @ (CVal::Int(_) | CVal::Bool(_) | CVal::Float(_))) => Ok(Some(v)),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    /// Evaluate a `(match scrutinee arm…)` to the CVal of the selected arm's body, using the
    /// same compile-time pattern resolution as codegen. A non-match on all arms is a trap.
    fn eval_const_match(&self, items: &[Node], env: &[Local]) -> Result<Option<CVal>, ConstTrap> {
        if items.len() < 2 {
            return Ok(None);
        }
        let scrutinee = &items[1];
        // Only fold a match whose SCRUTINEE is compile-time known. A runtime scrutinee (a
        // parameter or computed value) is not a constant — return None so codegen emits a
        // real runtime comparison cascade. (Without this guard, a runtime scrutinee's literal
        // arms read as non-matches and the fold wrongly falls through to `else` — the silent
        // miscompile that skipped recursion base cases.)
        if !self.match_scrutinee_is_static(scrutinee, env) {
            return Ok(None);
        }
        for arm in &items[2..] {
            let a = match arm {
                Node::List(a) if a.len() == 2 => a,
                _ => return Ok(None),
            };
            let (pattern, body) = (&a[0], &a[1]);
            if name_of(Some(pattern)) == Some("else") || name_of(Some(pattern)) == Some("_") {
                return self.eval_const(body, env);
            }
            match self.try_match(pattern, scrutinee, env) {
                Ok(Some(binds)) => {
                    let mut body_env = env.to_vec();
                    body_env.extend(binds);
                    return self.eval_const(body, &body_env);
                }
                Ok(None) => continue,      // definite non-match; try next arm
                Err(_) => return Ok(None), // beyond compile-time resolution → not a constant
            }
        }
        Err(ConstTrap) // no arm matched a known scrutinee → non-exhaustive (a trap/reject)
    }

    /// Evaluate a dotted intrinsic over constants: `Bytes.of`, `Bytes.len`, `Bytes.at`,
    /// `Bytes.concat`, `String.len`, `String.to-bytes`, `String.concat`, `String.at`,
    /// `String.slice`, `List.len`, `List.at`, `List.rest`. Produces a `CVal` or a trap.
    fn eval_const_dotted(&self, items: &[Node], env: &[Local]) -> Result<Option<CVal>, ConstTrap> {
        let dparts = match items.first() {
            Some(Node::List(d)) => d,
            _ => return Ok(None),
        };
        let obj = name_of(dparts.get(1));
        let field = name_of(dparts.get(2));
        // A qualified constructor application `(Ast.Int 1)` / `(Sign.Zero unit)` builds a sum
        // value — the variant is the field. (Handled before arg-evaluation since the payload
        // may itself be structural.)
        if let Some(variant) = field {
            if is_constructor_name(variant) {
                let payload = match items.get(1) {
                    Some(p) => match self.eval_const(p, env)? {
                        Some(v) => v,
                        None => return Ok(None),
                    },
                    None => CVal::unit(),
                };
                // Store the QUALIFIED variant name (`Sign.Pos`, `Ast.Int`) so the canonical
                // text renders it as written; sum-type/equality logic strips to the last
                // segment via `variant_tag`.
                let qualified = match obj {
                    Some(o) => format!("{o}.{variant}"),
                    None => variant.to_string(),
                };
                return Ok(Some(CVal::Sum { variant: qualified, payload: Box::new(payload) }));
            }
        }
        // Evaluate the arguments as constants.
        let mut args = Vec::new();
        for a in &items[1..] {
            match self.eval_const(a, env)? {
                Some(v) => args.push(v),
                None => return Ok(None),
            }
        }
        match (obj, field) {
            (Some("Bytes"), Some("of")) => {
                // (Bytes.of (list i…)) — each element must be 0..=255 or it traps.
                let elems = match args.first() {
                    Some(CVal::List(v)) => v,
                    _ => return Ok(None),
                };
                let mut bytes = Vec::new();
                for e in elems {
                    match e {
                        CVal::Int(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                        CVal::Int(_) => return Err(ConstTrap), // out of range → trap
                        _ => return Ok(None),
                    }
                }
                Ok(Some(CVal::Bytes(bytes)))
            }
            // Int64 checked arithmetic: `(Int64.checked-add a b)` → `Option<Int64>` — `(Some sum)`
            // when the exact result is in range, `(None unit)` on overflow (numeric-model.md #Overflow
            // Is Defined: a defined VALUE outcome, the fallible companion of the trapping `+`). Both
            // operands must be Int64. Folds two constants; the runtime path is `gen_int64_checked`.
            (Some("Int64"), Some("checked-add")) | (Some("Int64"), Some("checked-sub"))
                | (Some("Int64"), Some("checked-mul")) => {
                match (args.first(), args.get(1)) {
                    (Some(CVal::Int(a)), Some(CVal::Int(b))) => {
                        let r = match field {
                            Some("checked-add") => a.checked_add(*b),
                            Some("checked-sub") => a.checked_sub(*b),
                            _ => a.checked_mul(*b),
                        };
                        Ok(Some(match r {
                            Some(v) => CVal::Sum { variant: "Some".into(), payload: Box::new(CVal::Int(v)) },
                            None => CVal::Sum { variant: "None".into(), payload: Box::new(CVal::unit()) },
                        }))
                    }
                    _ => Ok(None),
                }
            }
            // Int64 wrapping arithmetic: `(Int64.wrapping-add a b)` → `Int64` that WRAPS modulo 2^64 on
            // overflow (numeric-model.md #Overflow Is Defined: a defined value outcome — two's-complement
            // wraparound, never a trap). The runtime path is the raw `i64.add/sub/mul` (wasm wraps).
            (Some("Int64"), Some("wrapping-add")) | (Some("Int64"), Some("wrapping-sub"))
                | (Some("Int64"), Some("wrapping-mul")) => {
                match (args.first(), args.get(1)) {
                    (Some(CVal::Int(a)), Some(CVal::Int(b))) => {
                        let v = match field {
                            Some("wrapping-add") => a.wrapping_add(*b),
                            Some("wrapping-sub") => a.wrapping_sub(*b),
                            _ => a.wrapping_mul(*b),
                        };
                        Ok(Some(CVal::Int(v)))
                    }
                    _ => Ok(None),
                }
            }
            (Some("Bytes"), Some("len")) => match args.first() {
                Some(CVal::Bytes(b)) => Ok(Some(CVal::Int(b.len() as i64))),
                _ => Ok(None),
            },
            (Some("Bytes"), Some("at")) => match (args.first(), args.get(1)) {
                (Some(CVal::Bytes(b)), Some(CVal::Int(i))) => {
                    // Fallible, not trapping (collections-and-text.md #Indexing And Lookup Are
                    // Fallible, Not Trapping): in-bounds → `(Some byte)` (byte as Int64 0..=255),
                    // out-of-bounds / negative → `(None unit)`. Mirrors List.at.
                    match usize::try_from(*i).ok().and_then(|i| b.get(i)) {
                        Some(byte) => Ok(Some(CVal::Sum {
                            variant: "Some".into(),
                            payload: Box::new(CVal::Int(*byte as i64)),
                        })),
                        None => Ok(Some(CVal::Sum {
                            variant: "None".into(),
                            payload: Box::new(CVal::unit()),
                        })),
                    }
                }
                _ => Ok(None),
            },
            (Some("Bytes"), Some("concat")) => match (args.first(), args.get(1)) {
                (Some(CVal::Bytes(a)), Some(CVal::Bytes(b))) => {
                    let mut v = a.clone();
                    v.extend_from_slice(b);
                    Ok(Some(CVal::Bytes(v)))
                }
                _ => Ok(None),
            },
            (Some("Bytes"), Some("slice")) => {
                // (Bytes.slice b start length) — the `length` bytes of `b` beginning at `start`,
                // FALLIBLE (collections-and-text.md #Indexing And Lookup Are Fallible, Not
                // Trapping): a valid range yields `(Some slice)`, a negative start/length or a
                // start+length running past the end yields `(None unit)`. COPY semantics — the
                // result owns its bytes; a later runtime MAY share the parent's storage as an
                // unobservable optimization (memory-and-resource-model.md #Sharing Is Not
                // Observable), which must keep these same cases green.
                let none = || {
                    Ok(Some(CVal::Sum { variant: "None".into(), payload: Box::new(CVal::unit()) }))
                };
                match (args.first(), args.get(1), args.get(2)) {
                    (Some(CVal::Bytes(b)), Some(CVal::Int(start)), Some(CVal::Int(len))) => {
                        let (start, len) = (*start, *len);
                        if start < 0 || len < 0 {
                            return none(); // negative start or length → None
                        }
                        let (start, len) = (start as usize, len as usize);
                        match start.checked_add(len) {
                            Some(end) if end <= b.len() => Ok(Some(CVal::Sum {
                                variant: "Some".into(),
                                payload: Box::new(CVal::Bytes(b[start..end].to_vec())),
                            })),
                            _ => none(), // runs past the end → None
                        }
                    }
                    _ => Ok(None),
                }
            }
            (Some("Bytes"), Some("compact")) => match args.first() {
                // (Bytes.compact b) materializes `b` into independent storage — value-preserving,
                // observable only through the resource measure, never through a value operation
                // (memory-and-resource-model.md #Retained Storage Is Accounted For What It Holds
                // Live). On the const path the value already owns its bytes, so compact is the
                // identity on value; it exists so a later view representation can drop a large
                // parent while keeping a small slice.
                Some(CVal::Bytes(b)) => Ok(Some(CVal::Bytes(b.clone()))),
                _ => Ok(None),
            },
            (Some("String"), Some("scalar-len")) => match args.first() {
                // Scalar length: the count of Unicode scalar values (collections-and-text.md #A
                // String Offers Both A Scalar Length And A Byte Length). The string is already NFC
                // (the reader normalizes at read time), so this counts the normalized scalars.
                Some(CVal::Str(s)) => Ok(Some(CVal::Int(s.chars().count() as i64))),
                _ => Ok(None),
            },
            (Some("String"), Some("byte-len")) => match args.first() {
                // Byte length: the count of bytes in the UTF-8 encoding of the NORMALIZED string
                // (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).
                // Obtained directly from the stored (already-NFC) bytes — it need not materialize a
                // separate Bytes value, but agrees with `Bytes.len (String.to-bytes s)`.
                Some(CVal::Str(s)) => Ok(Some(CVal::Int(s.len() as i64))),
                _ => Ok(None),
            },
            (Some("String"), Some("to-bytes")) => match args.first() {
                Some(CVal::Str(s)) => Ok(Some(CVal::Bytes(s.as_bytes().to_vec()))),
                _ => Ok(None),
            },
            (Some("String"), Some("from-bytes")) => match args.first() {
                // Total decode (collections-and-text.md #Decoding Bytes To A String Is Total, Not
                // Trapping): well-formed UTF-8 → (Some s), ill-formed → None. NEVER traps — the
                // ill-formed case is an ordinary value the program handles. The decoded string is
                // renormalized to NFC to match the reader's invariant that every String value is NFC.
                Some(CVal::Bytes(b)) => match std::str::from_utf8(b) {
                    Ok(s) => {
                        let nfc: String =
                            unicode_normalization::UnicodeNormalization::nfc(s.chars()).collect();
                        Ok(Some(CVal::Sum {
                            variant: "Some".to_string(),
                            payload: Box::new(CVal::Str(nfc)),
                        }))
                    }
                    Err(_) => Ok(Some(CVal::Sum {
                        variant: "None".to_string(),
                        payload: Box::new(CVal::unit()),
                    })),
                },
                _ => Ok(None),
            },
            (Some("String"), Some("concat")) => match (args.first(), args.get(1)) {
                (Some(CVal::Str(a)), Some(CVal::Str(b))) => Ok(Some(CVal::Str(format!("{a}{b}")))),
                _ => Ok(None),
            },
            (Some("String"), Some("at")) => match (args.first(), args.get(1)) {
                (Some(CVal::Str(s)), Some(CVal::Int(i))) => {
                    // Fallible, not trapping: in-bounds SCALAR index → `(Some "<char>")`,
                    // out-of-bounds / negative → `(None unit)`. Indexes by Unicode scalar value
                    // (`chars().nth`), not byte offset. Mirrors List.at / Bytes.at.
                    match usize::try_from(*i).ok().and_then(|i| s.chars().nth(i)) {
                        Some(ch) => Ok(Some(CVal::Sum {
                            variant: "Some".into(),
                            payload: Box::new(CVal::Str(ch.to_string())),
                        })),
                        None => Ok(Some(CVal::Sum {
                            variant: "None".into(),
                            payload: Box::new(CVal::unit()),
                        })),
                    }
                }
                _ => Ok(None),
            },
            (Some("String"), Some("slice")) => {
                match (args.first(), args.get(1), args.get(2)) {
                    (Some(CVal::Str(s)), Some(CVal::Int(a)), Some(CVal::Int(b))) => {
                        // Fallible sub-sequence: a valid range [a,b) (0 ≤ a ≤ b ≤ len) → `(Some
                        // "<slice>")` (an empty range a==b is Some of the empty string, present not
                        // absent); an out-of-range or inverted range → `(None unit)`.
                        let chars: Vec<char> = s.chars().collect();
                        let (a, b) = (*a, *b);
                        if a < 0 || b < a || (b as usize) > chars.len() {
                            return Ok(Some(CVal::Sum {
                                variant: "None".into(),
                                payload: Box::new(CVal::unit()),
                            }));
                        }
                        let sub: String = chars[a as usize..b as usize].iter().collect();
                        Ok(Some(CVal::Sum {
                            variant: "Some".into(),
                            payload: Box::new(CVal::Str(sub)),
                        }))
                    }
                    _ => Ok(None),
                }
            }
            // Ast.encode : Ast → Bytes; Ast.decode : Bytes → Ast. Implemented with the real
            // canonical codec so they round-trip exactly (contracts/ast-encoding.md bijection).
            (Some("Ast"), Some("encode")) => match args.first() {
                // Encode a quote-built AST (`CVal::Ast`) OR an `Ast.*`-constructor-built AST
                // (`CVal::Sum`, bridged to its node by `cval_to_ast_node`): both denote the ONE
                // AST value the encoding is a bijection over (12-metaprogramming.sexp §"decode of
                // encode of an Ast.Int constructor round-trips"), so both encode to the same
                // canonical bytes.
                Some(v) => match cval_to_ast_node(v) {
                    Some(node) => Ok(Some(CVal::Bytes(ast::encode(&node)))),
                    None => Ok(None),
                },
                _ => Ok(None),
            },
            // `Ast.decode : Bytes → Result<Ast, e>` — TOTAL over untrusted external bytes: it never
            // traps, it returns `(Ok ast)` for a canonical encoding and `(Err <reason>)` for
            // malformed input OR trailing bytes (deterministic-value-form.md #Decoding Refuses is the
            // ERROR CASE of a fallible decode, not a hard failure — bytes may come from an external
            // source, so a program `match`es the result). The error payload is the decoder's reason
            // String (rendered `(Err "…")`); `ast::decode` already rejects trailing bytes.
            (Some("Ast"), Some("decode")) => match args.first() {
                Some(CVal::Bytes(b)) => match ast::decode(b) {
                    Ok(node) => Ok(Some(CVal::Sum {
                        variant: "Ok".into(),
                        payload: Box::new(CVal::Ast(node)),
                    })),
                    Err(e) => Ok(Some(CVal::Sum {
                        variant: "Err".into(),
                        payload: Box::new(CVal::Str(e.0)),
                    })),
                },
                _ => Ok(None),
            },
            // `Option.expect` / `Result.expect` unwrap the contained value or TRAP on absence with
            // the given message (core-semantics.md §Requiring The Value Of An Optional Traps On
            // Absence). This is a method on the OPTION/RESULT type, not a generic operation:
            // `Option.expect (Some x) msg` → x, `(None _)` → trap; `Result.expect (Ok x) msg` → x,
            // `(Err e)` → trap. The variant tag decides present-vs-absent; the message is the trap
            // reason (a defined trap, so it declines to `ConstTrap` here — the folder cannot carry
            // a custom message, but the absent case is a genuine trap either way).
            (Some("Option"), Some("expect")) => match args.first() {
                Some(CVal::Sum { variant, payload }) if variant_tag(variant) == "Some" => {
                    Ok(Some((**payload).clone()))
                }
                Some(CVal::Sum { variant, .. }) if variant_tag(variant) == "None" => Err(ConstTrap),
                _ => Ok(None),
            },
            (Some("Result"), Some("expect")) => match args.first() {
                Some(CVal::Sum { variant, payload }) if variant_tag(variant) == "Ok" => {
                    Ok(Some((**payload).clone()))
                }
                Some(CVal::Sum { variant, .. }) if variant_tag(variant) == "Err" => Err(ConstTrap),
                _ => Ok(None),
            },
            (Some("List"), Some("len")) => match args.first() {
                Some(CVal::List(v)) => Ok(Some(CVal::Int(v.len() as i64))),
                _ => Ok(None),
            },
            (Some("List"), Some("at")) => match (args.first(), args.get(1)) {
                (Some(CVal::List(v)), Some(CVal::Int(i))) => {
                    // Fallible, NOT trapping (collections-and-text.md #Indexing And Lookup Are
                    // Fallible, Not Trapping): an in-bounds index yields `(Some elem)`, an
                    // out-of-bounds / negative / empty-list index yields `(None unit)` — one total
                    // Option return type. A negative index is out of bounds (never wrapped to a
                    // large unsigned offset); `usize::try_from` rejects it, giving None.
                    match usize::try_from(*i).ok().and_then(|i| v.get(i)) {
                        Some(x) => Ok(Some(CVal::Sum {
                            variant: "Some".into(),
                            payload: Box::new(x.clone()),
                        })),
                        None => Ok(Some(CVal::Sum {
                            variant: "None".into(),
                            payload: Box::new(CVal::unit()),
                        })),
                    }
                }
                _ => Ok(None),
            },
            // ── Map operations over a compile-time-known map (collections-and-text.md §Maps). Keys are
            // VALUES compared by `cval_eq`; the map stays sorted by canonical key form so equality and
            // rendering are order-independent. `insert`/`remove` return the new map; `swap`/`take`
            // return `(tuple <prior/removed value Option> <new map>)`; `lookup`→Option; `size`→Int. ──
            (Some("Map"), Some("size")) => match args.first() {
                Some(CVal::Map(m)) => Ok(Some(CVal::Int(m.len() as i64))),
                _ => Ok(None),
            },
            (Some("Map"), Some("lookup")) => match (args.first(), args.get(1)) {
                (Some(CVal::Map(m)), Some(k)) => Ok(Some(map_lookup_cval(m, k))),
                _ => Ok(None),
            },
            (Some("Map"), Some("insert")) => match (args.first(), args.get(1), args.get(2)) {
                (Some(CVal::Map(m)), Some(k), Some(v)) => {
                    Ok(Some(CVal::Map(map_insert_cval(m, k.clone(), v.clone()))))
                }
                _ => Ok(None),
            },
            (Some("Map"), Some("remove")) => match (args.first(), args.get(1)) {
                (Some(CVal::Map(m)), Some(k)) => Ok(Some(CVal::Map(map_remove_cval(m, k).0))),
                _ => Ok(None),
            },
            // `swap`/`take` yield `(tuple <prior/removed value as Option> <new map>)` — the
            // value-yielding forms (collections-and-text.md §A Map Is Built By Functional Construction).
            (Some("Map"), Some("swap")) => match (args.first(), args.get(1), args.get(2)) {
                (Some(CVal::Map(m)), Some(k), Some(v)) => {
                    let prior = map_lookup_cval(m, k);
                    let next = map_insert_cval(m, k.clone(), v.clone());
                    Ok(Some(CVal::Tuple(vec![prior, CVal::Map(next)])))
                }
                _ => Ok(None),
            },
            (Some("Map"), Some("take")) => match (args.first(), args.get(1)) {
                (Some(CVal::Map(m)), Some(k)) => {
                    let (next, removed) = map_remove_cval(m, k);
                    Ok(Some(CVal::Tuple(vec![removed, CVal::Map(next)])))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn eval_const_seq(&self, nodes: &[Node], env: &[Local]) -> Result<Option<Vec<CVal>>, ConstTrap> {
        let mut out = Vec::new();
        for n in nodes {
            match self.eval_const(n, env)? {
                Some(v) => out.push(v),
                None => return Ok(None),
            }
        }
        Ok(Some(out))
    }

    /// Emit a compile-time constant as wasm, if it is a scalar (Int/Bool/Float) — the only
    /// forms with a scalar wasm representation. Compound constants have no scalar form.
    fn emit_const(&self, v: &CVal) -> Option<(Vec<u8>, Kind)> {
        match v {
            CVal::Int(n) => {
                let mut c = vec![op::I64_CONST];
                sleb128(*n, &mut c);
                Some((c, Kind::Int64))
            }
            CVal::Bool(b) => Some((vec![op::I32_CONST, if *b { 1 } else { 0 }], Kind::Bool)),
            CVal::Float(f) => {
                let mut c = vec![op::F64_CONST];
                c.extend_from_slice(&f.to_le_bytes());
                Some((c, Kind::Float64))
            }
            _ => None,
        }
    }

    fn gen_name(&self, n: &str, env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if n == "unit" {
            return Ok((vec![], Kind::Unit));
        }
        if n == "nan" || n == "NaN" {
            let mut c = vec![op::F64_CONST];
            c.extend_from_slice(&f64::NAN.to_le_bytes());
            return Ok((c, Kind::Float64));
        }
        if let Some(local) = env.iter().rev().find(|l| l.name == n) {
            // A compile-time alias (a structural value or a pattern binder): re-emit the
            // aliased node under the environment it was captured in.
            if let Some((node, captured)) = &local.alias {
                return self.emit(node, captured, ctx);
            }
            let mut c = vec![op::LOCAL_GET];
            uleb128(local.idx as u64, &mut c);
            // REFERENCE-COUNTING (ask-63): a `Kind::Heap` local holds an RC'd runtime handle, and the
            // runtime's growth/build ops (`vec-push`, `map-insert`, `bytes-concat`, `sum-new`, …)
            // CONSUME their heap argument (the FBIP/persistent-vector contract). Reading a heap local
            // yields a COPY of the handle bits, not a new reference — so if the SAME local flows to
            // two consuming ops (`(both e) = (+ (use e 1) (use e 2))`), the first consumer frees the
            // backing and the second double-frees (trap in `op_drop`→`talc::deallocate`). Handing each
            // reader its OWN reference — `dup` before every heap-local read — fixes that: the reader
            // consumes the fresh reference, the local's stored reference is never the one consumed, so
            // the count can never underflow (crash-impossible). This OVER-retains (the owning
            // reference is not yet reclaimed — a leak the precise-drop Perceus pass, M2 Phase D, will
            // close), but it never changes an output value, so every value-checking gate stays green.
            // Scalars (Int64/Bool/Float/Unit) carry no reference and are untouched — byte-identical.
            // An ALIAS re-emits its node (a fresh construction per read), so it never shares a handle
            // and is handled above, before this point.
            if local.kind == Kind::Heap {
                c.push(op::LOCAL_GET);
                uleb128(local.idx as u64, &mut c);
                c.push(op::CALL);
                uleb128(himport::DUP as u64, &mut c);
            }
            return Ok((c, local.kind));
        }
        // A digit-led, all-digit(-and-separator) token is plainly a NUMERIC LITERAL, not a
        // name — it only became a `Node::Name` because the reader could not parse it as an i64
        // (it is outside the Int64 range). That is a malformed literal (CDZ0201, a well-
        // formedness rejection), NOT an unbound name (CDZ0101). Classify it as such before the
        // unbound-name path so an out-of-range integer surfaces the honest diagnostic
        // (01-literals.sexp §"an out-of-range integer literal is a malformed literal").
        if looks_like_numeric_literal(n) {
            return reject("CDZ0201", format!("integer literal out of the Int64 range: {n}"));
        }
        // A bare NULLARY constructor as a runtime VALUE (`None`, `NNil`, `Zero`) is the nullary sum
        // value — lower it exactly as the applied `(Ctor unit)` does (`gen_runtime_sum` with a unit
        // payload). This is the runtime companion of the `eval_const` bare-nullary fold: a reader
        // that builds `NNil` (not `(Node.NNil unit)`) for an empty node, or an `if` branch returning
        // a bare nullary variant, now compiles. `gen_runtime_sum` declines on the scalar path with a
        // HEAP reason (→ runtime-mode retry), so a bare nullary in a scalar-only program still routes
        // correctly. Only fires for a genuine nullary variant; a bare UNARY constructor (`Some`) is a
        // constructor FUNCTION handled by the application paths, not a value.
        if self.nullary_variants.contains(variant_tag(n)) {
            let unit_payload = Node::Name("unit".into());
            let elems = [Node::Name(n.to_string()), unit_payload];
            return self.gen_runtime_sum(n, &elems, env, ctx);
        }
        // A name bound nowhere is a scope error rejected before running (constitution: binding
        // is lexical; every generation makes this front-end rejection). A form-keyword or a
        // constructor reaching here is instead a not-yet-compiled *form* (a compiler gap), so
        // it declines as a todo rather than masquerading as an unbound user name.
        if is_form_keyword(n) || is_constructor_name(n) {
            decline(format!("unsupported bare form/constructor: {n}"))
        } else {
            reject("CDZ0101", format!("unbound name: {n}"))
        }
    }

    /// Static type rejections (constitution VII, Amendment 0.4.0): refuse an ill-typed
    /// application with its machine-readable diagnostic code, so the compiler never emits a
    /// component for a program that is not well-typed. Realized incrementally over the type
    /// rules the corpus exercises; a rule not yet checked simply is not rejected here.
    fn check_type_rejections(&self, elems: &[Node], env: &[Local]) -> Result<(), Decline> {
        // A NULLARY variant applied to a non-unit payload — whether written bare `(None 5)` or
        // qualified `((. Sign Pos) 5)` (the canonical form of `(Sign.Pos 5)`) — is a type error.
        // Its argument type is Unit (core-semantics.md #A Sum Type Constructor Is A Single-Arity
        // Function, 2nd sentence), so an Int64/Bool/… payload mismatches. Checked here up front
        // because a qualified constructor's head is a `.`-list, not a name, so it never reaches
        // the name-keyed match below (05-compound-types.sexp §"a nullary variant applied to a
        // non-unit payload is a type error").
        if elems.len() == 2 {
            if let Some(ctor) = constructor_of(elems.first()) {
                let tag = variant_tag(&ctor);
                if self.nullary_variants.contains(tag)
                    && self.static_type(&elems[1], env).map_or(false, |t| t != StaticType::Unit)
                {
                    return reject("CDZ0201", "a nullary variant carries a non-unit payload");
                }
                // The typed-payload companion of the nullary check: a UNARY variant applied to a
                // payload of the WRONG type. A sum constructor is a single-arity function whose argument
                // is type-checked against its declared payload type (core-semantics.md #A Sum Type
                // Constructor Is A Single-Arity Function, #Applying A Function Binds Its Parameter To Its
                // Argument) — `(type T (Mk Int64))` then `(T.Mk "x")` applies `Mk` to a String where the
                // payload type is Int64, a type error (CDZ0201). Without this the variant is constructed
                // with the mistyped payload — an observably ill-typed value (`(T.Mk "x")` renders as
                // such, and matching it binds the String where an Int64 is declared). The check is
                // UNIFORM across every payload type SHAPE — scalar, String, List, Record, AND Tuple: a
                // `(Pair (Tuple Int64 Int64))` variant applied to `(tuple 1 2 3)` is a wrong-ARITY tuple
                // (a tuple's length is part of its type), rejected exactly as the scalar case is. The
                // declared payload ARGUMENT type node is reconstructed from `sum_payload_types[tag]`
                // (which flattens a `(Tuple …)` payload into its element slots for the runtime-match
                // binder): a single slot IS the argument type; multiple slots came from a tuple payload,
                // so the argument type is `(Tuple slot…)`. `arg_contradicts_declared_type` then checks
                // the one argument against it (deferring a `(Tuple …)`/`(List …)`/… head to
                // `annotation_contradicts`, which checks arity + element types). A POLYMORPHIC payload
                // (a type parameter `a`, as in Option's `Some a`) is a bare non-scalar name that imposes
                // nothing — so `(Some "x")` is untouched; only a CONCRETE declared payload is checked.
                if !self.nullary_variants.contains(tag) {
                    if let Some(slots) = self.sum_payload_types.get(tag) {
                        let arg_ty = match slots.len() {
                            0 => None,
                            1 => Some(slots[0].clone()),
                            // Multiple slots ⇒ a `(Tuple …)` payload the flattener unwrapped; the
                            // constructor's single argument type is that tuple. Reconstruct it so the
                            // argument (a `(tuple …)`) is checked for arity + element types.
                            _ => {
                                let mut t = vec![Node::Name("Tuple".into())];
                                t.extend(slots.iter().cloned());
                                Some(Node::List(t))
                            }
                        };
                        if let Some(arg_ty) = arg_ty {
                            if self.arg_contradicts_declared_type(&elems[1], &arg_ty, env) {
                                return reject(
                                    "CDZ0201",
                                    "a unary variant applied to a payload of the wrong type",
                                );
                            }
                        }
                    }
                }
            }
        }
        // The LOW-arity mirror of over-application: a UNARY constructor applied to ZERO arguments —
        // a bare `(Some)` / `(Ok)` / `(Type.Cons)` with no payload. A sum constructor produces its
        // value only when applied to exactly one argument (core-semantics.md #A Sum Type
        // Constructor Is A Single-Arity Function), so `(Some)` MUST be rejected (CDZ0201), NOT
        // fabricate a `(Some unit)` — a value of `Option Unit` the program never wrote, which would
        // slip past a `(Some x)` payload-annotation check. Scoped to a KNOWN variant that is NOT
        // nullary (a nullary `(None)` legitimately takes a unit payload; an unknown capitalized head
        // — a module name `(List)`, an undeclared `(Foo)` — imposes nothing), symmetric with the
        // `elems.len() > 2` over-application arm below (09-functions.sexp §"under-applying a unary
        // constructor is a type error, not a fabricated unit payload").
        if elems.len() == 1 {
            if let Some(ctor) = constructor_of(elems.first()) {
                let tag = variant_tag(&ctor);
                if self.sum_types.contains_key(tag) && !self.nullary_variants.contains(tag) {
                    return reject("CDZ0201", "under-applying a single-arity constructor");
                }
            }
        }
        // `(List.push lst elem)` / `(List.update lst i elem)` produce a NEW list, which must satisfy
        // the same element-share-one-type rule a `(list …)` literal does (collections-and-text.md #A
        // List Is An Ordered Homogeneous Sequence / #A List Is Grown By Functional Construction). So
        // pushing an element whose type differs from the list's element type is a type error (CDZ0201)
        // — `(List.push (list 1 2) true)` appends a Bool to an Int64 list. A `List.push` that skips
        // this renders the result at the pushed element's type (the stored ints come back as bools) —
        // a WRONG VALUE, not merely a missing rejection. Compare the pushed element's static type to
        // the list's element type (recovered from a const-foldable list operand's first element);
        // reject only a PROVABLE mismatch (both statically known), else leave conservative. The head
        // is the `.`-list `(. List push)`, so this runs before the bare-name dispatch below (which
        // returns `Ok(())` for a dotted head). (05-compound-types.sexp §"pushing an element of a
        // different type onto a list is a type error".)
        if let Some(Node::List(hd)) = elems.first() {
            if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("List")
                && matches!(name_of(hd.get(2)), Some("push") | Some("update"))
            {
                // The element argument is the LAST operand: `(List.push lst elem)` → elems[2];
                // `(List.update lst i elem)` → elems[3]. The list operand is elems[1].
                if let (Some(list_node), Some(elem_node)) = (elems.get(1), elems.last()) {
                    if let (Ok(Some(CVal::List(items))), Some(et)) =
                        (self.eval_const(list_node, env), self.static_type(elem_node, env))
                    {
                        if let Some(first) = items.first() {
                            let lt = StaticType::of_cval(first);
                            if lt != et {
                                return reject(
                                    "CDZ0201",
                                    "pushing an element of a different type onto a list",
                                );
                            }
                        }
                    }
                }
            }
            // `(List.concat a b)` joins two lists into ONE list, so both must share the SAME element
            // type (collections-and-text.md #A List Is An Ordered Homogeneous Sequence) — concatenating
            // an Int64 list with a Bool list is a type error (CDZ0201). A skip would render the result
            // at one operand's element type, mistyping the other's elements (a WRONG VALUE). Compare
            // the two const-foldable lists' first elements; PROVABLE mismatch only (both known and
            // non-empty), else conservative.
            if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("List")
                && name_of(hd.get(2)) == Some("concat")
            {
                if let (Some(a_node), Some(b_node)) = (elems.get(1), elems.get(2)) {
                    if let (Ok(Some(CVal::List(a))), Ok(Some(CVal::List(b)))) =
                        (self.eval_const(a_node, env), self.eval_const(b_node, env))
                    {
                        if let (Some(fa), Some(fb)) = (a.first(), b.first()) {
                            if StaticType::of_cval(fa) != StaticType::of_cval(fb) {
                                return reject(
                                    "CDZ0201",
                                    "concatenating lists of different element types",
                                );
                            }
                        }
                    }
                }
            }
        }
        // `(Map.insert m k v)` / `(Map.swap m k v)` produce a NEW map, which must satisfy the same
        // one-key-type + one-value-type rule a `(map …)` literal does (collections-and-text.md #A Map
        // Associates Keys With Values). Inserting a KEY or VALUE whose type differs from the map's is
        // a type error (CDZ0201) — `(Map.insert (Map.insert Map.empty 1 10) 2 true)` mixes an Int64
        // value with a Bool; `… true 20` mixes an Int64 key with a Bool. A skip miscompiles the same
        // way List.push does (renders at the inserted type). Compare the inserted key/value static
        // type to the const-folded map's first entry; PROVABLE mismatch only. Same `.`-head-before-
        // bare-name-dispatch placement as the List.push check (05-compound-types.sexp §"inserting a
        // value/key of a different type into a map is a type error").
        if let Some(Node::List(hd)) = elems.first() {
            if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("Map")
                && matches!(name_of(hd.get(2)), Some("insert") | Some("swap"))
            {
                // `(Map.insert m k v)` → map=elems[1], key=elems[2], value=elems[3].
                if let (Some(map_node), Some(key_node), Some(val_node)) =
                    (elems.get(1), elems.get(2), elems.get(3))
                {
                    if let Ok(Some(CVal::Map(entries))) = self.eval_const(map_node, env) {
                        if let Some((k0, v0)) = entries.first() {
                            if let Some(kt) = self.static_type(key_node, env) {
                                if StaticType::of_cval(k0) != kt {
                                    return reject(
                                        "CDZ0201",
                                        "inserting a key of a different type into a map",
                                    );
                                }
                            }
                            if let Some(vt) = self.static_type(val_node, env) {
                                if StaticType::of_cval(v0) != vt {
                                    return reject(
                                        "CDZ0201",
                                        "inserting a value of a different type into a map",
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        let head = match elems.first() {
            Some(Node::Name(h)) => h.as_str(),
            // A non-name head applied to arguments — `(5 3)`, `(true 1)`, `(3.5 1)` — is
            // applying a non-function: a type error (CDZ0201). A literal scalar head with args
            // can only be an application, and no scalar is callable.
            Some(Node::Int(_)) | Some(Node::Float(_)) | Some(Node::Bool(_)) | Some(Node::Str(_))
                if elems.len() > 1 =>
            {
                return reject("CDZ0201", "applying a non-function");
            }
            _ => return Ok(()),
        };
        match head {
            // Arithmetic over operands of different types does not silently promote.
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" if elems.len() == 3 => {
                let a = self.static_type(&elems[1], env);
                let b = self.static_type(&elems[2], env);
                if let (Some(ta), Some(tb)) = (a, b) {
                    // Both numeric but different width/kind (Int vs Float): no silent promote.
                    if ta != tb && ta.is_numeric() && tb.is_numeric() {
                        return reject("CDZ0301", "numeric types do not silently promote");
                    }
                    // A non-numeric operand to a numeric op: a plain type mismatch.
                    if !ta.is_numeric() || !tb.is_numeric() {
                        return reject("CDZ0201", "operation on mismatched types");
                    }
                }
            }
            // Equality is valid only between values of the SAME type. Comparing across a
            // nominal boundary, or between two different structural types (e.g. a map and a
            // record), is a type error — not a `false` answer.
            "=" if elems.len() == 3 => {
                let na = self.nominal_name(&elems[1], env);
                let nb = self.nominal_name(&elems[2], env);
                if let (Some(x), Some(y)) = (&na, &nb) {
                    if x != y {
                        return reject("CDZ0202", "comparison across a nominal boundary");
                    }
                }
                // Exactly one operand is nominal, the other a PLAIN structural value of the SAME
                // coarse shape — `(= (Point …) (record …))` or the flip. A nominal value is
                // declared distinct from the untagged shape it wraps, so comparing it to that plain
                // shape is a cross-boundary comparison (CDZ0202, type-system.md #Nominal Types Are
                // Not Comparable Across Their Boundary, 2nd sentence), checked on either side
                // (05-compound-types.sexp §"a nominal/plain record compared to a plain/nominal
                // record … is a type error"). Guarded on a matching coarse type so a nominal-vs-
                // scalar mismatch stays the different-types rejection (CDZ0201) below, not this one.
                if na.is_some() != nb.is_some() {
                    if let (Some(ta), Some(tb)) =
                        (self.static_type(&elems[1], env), self.static_type(&elems[2], env))
                    {
                        if ta == tb {
                            return reject("CDZ0202", "comparison between a nominal value and a plain structural value");
                        }
                    }
                }
                // Different structural types (map vs record, list vs tuple, …) are not
                // comparable — a mismatched-type operation (CDZ0201). Two numeric widths keep
                // the no-silent-promote code (CDZ0301).
                if let (Some(ta), Some(tb)) =
                    (self.static_type(&elems[1], env), self.static_type(&elems[2], env))
                {
                    if ta != tb {
                        let code = if ta.is_numeric() && tb.is_numeric() { "CDZ0301" } else { "CDZ0201" };
                        return reject(code, "comparison between values of different types");
                    }
                }
                // Same coarse type but incompatible SHAPE — records with different field-name
                // sets, tuples of different length, sums with different variants — are values
                // of different types and are not comparable (CDZ0201).
                if let (Ok(Some(x)), Ok(Some(y))) =
                    (self.eval_const(&elems[1], env), self.eval_const(&elems[2], env))
                {
                    if self.shapes_incompatible(&x, &y) {
                        return reject("CDZ0201", "comparison between values of different shapes");
                    }
                }
            }
            // The ordering operators are held to the SAME operand-typing rule as `=` and the
            // arithmetic operators (type-system.md §"The comparison operators type-check their
            // operands exactly as = and + do"). An ordering offers a total order over ONE type's
            // values (core-semantics.md #Ordering Where Offered Is Total), so operands of two
            // DIFFERENT types are rejected: two different NUMERIC types are the silent-promotion
            // the arithmetic operators forbid (CDZ0301, `(< 5 2.0)` like `(+ 5 2.0)`); any other
            // cross-kind pair (Int vs Bool, Int vs String) has no shared order at all, a general
            // type error (CDZ0201, `(< 1 true)` like `(= 1 true)`). An ordering is offered on
            // Int64/Float64 and Bool (false < true) — a same-type pair of any of these is fine.
            "<" | ">" | "<=" | ">=" if elems.len() == 3 => {
                if let (Some(ta), Some(tb)) =
                    (self.static_type(&elems[1], env), self.static_type(&elems[2], env))
                {
                    if ta != tb {
                        let code = if ta.is_numeric() && tb.is_numeric() { "CDZ0301" } else { "CDZ0201" };
                        return reject(code, "ordering between values of different types");
                    }
                }
            }
            // An annotation `(: value Type)` that contradicts the value's type is rejected.
            ":" if elems.len() == 3 => {
                if let (Some(vt), Some(ann)) =
                    (self.static_type(&elems[1], env), type_name(&elems[2]))
                {
                    if !vt.matches_annotation(ann) {
                        return reject("CDZ0203", "annotation contradicts the value's type");
                    }
                }
                // The head-level `matches_annotation` above compares only the head type NAME, so a
                // COMPOUND annotation `(Option Int64)` on a `(Some true)` passes it (the head `Option`
                // matches). A payload/element/nested contradiction must be caught by DESCENDING into
                // the type parameters (type-system.md #Annotations Constrain, Never Contradict).
                // `annotation_contradicts` recurses through `(Option/Result …)` payloads, `(List E)`
                // elements, and `(Tuple …)` positions to any depth — so `(: (Some (Some 5)) (Option
                // (Option Bool)))` and `(: (list 1 2) (List Bool))` are rejected at the scalar leaf.
                // Only a PROVABLE leaf mismatch rejects; an unprovable/runtime value or a not-yet-
                // covered form is left conservative (decline-don't-miscompile).
                if self.annotation_contradicts(&elems[1], &elems[2], env) {
                    return reject(
                        "CDZ0203",
                        "annotation's parameter type contradicts the value",
                    );
                }
            }
            // A conditional's two branches must have the same type — the whole `if` is an
            // expression of one type. Two known, differing branch types are a type error
            // (CDZ0201), caught regardless of which branch the condition selects.
            "if" if elems.len() == 4 => {
                // The condition selects a branch and MUST be a Bool — there is no truthiness and
                // no silent coercion (numeric-model.md #Numeric Types Do Not Silently Promote).
                // A statically-known non-Bool condition — an Int64 `(if 1 …)`, a tuple `(if
                // (tuple 1 2) …)` — is ill-typed (CDZ0201). Rejecting here, before `gen_name`
                // runs, also keeps a compound condition's diagnostic honest: `(tuple 1 2)` is a
                // not-a-Bool type error, not an `unbound name: tuple` (the constructor is intact).
                if let Some(tc) = self.static_type(&elems[1], env) {
                    if tc != StaticType::Bool {
                        return reject("CDZ0201", "conditional condition is not a Bool");
                    }
                }
                if let (Some(ta), Some(tb)) =
                    (self.static_type(&elems[2], env), self.static_type(&elems[3], env))
                {
                    if ta != tb {
                        return reject("CDZ0201", "conditional branches have different types");
                    }
                }
                // The coarse `StaticType` above catches a KIND mismatch (tuple vs Int64) but not two
                // branches of the SAME kind but different SHAPE — `(tuple 1 2)` vs `(tuple 3 4 5)`
                // (different arity), or `(tuple 1 2)` vs `(tuple 1 true)` (different element type at a
                // position). A tuple's arity and element types ARE its type (type-system.md
                // #Structural Values Are Comparable Only When Their Shapes Match), so the branches are
                // different types and the `if` is ill-typed (CDZ0201) — the same structural depth the
                // list/map element-homogeneity check applies. When both branches const-fold, compare
                // their shapes (02-binding-and-control.sexp §"a conditional with two tuple branches of
                // different arity / element type is a type error").
                if let (Ok(Some(va)), Ok(Some(vb))) =
                    (self.eval_const(&elems[2], env), self.eval_const(&elems[3], env))
                {
                    if self.shapes_incompatible(&va, &vb) {
                        return reject("CDZ0201", "conditional branches have different shapes");
                    }
                }
            }
            // The boolean connectives type-check EACH operand as a Bool whether or not it is
            // evaluated (core-semantics.md #Boolean Connectives Short-Circuit — the same discipline
            // as a conditional's branches). A statically-known non-Bool operand is CDZ0201. Checked
            // here, before the desugar to `if`, so `(and true 1)` reports "operand is not a Bool"
            // rather than the desugared `if`'s "branches have different types" — and so the
            // non-evaluated operand is still checked (short-circuit shields TRAPS, not TYPE errors).
            // The boolean connectives type-check EACH operand as a Bool whether or not it is evaluated
            // (short-circuit shields TRAPS, not TYPE errors). The SCOPE check on a short-circuited
            // operand is NOT done here: `check_type_rejections` runs via `check_tree`, which walks the
            // tree with the ENCLOSING env and does NOT extend it with `let`/`match`/`fn` binders — so a
            // `let`-bound name in a connective operand (`(let ((x k)) (and (> x 0) (< x 9)))`) would
            // read as unbound (ask-66: this false reject broke self-compilation). Instead the connective
            // is EXCLUDED from the const-fold (see `gen_list`) so it desugars to an `if` at EMIT, where
            // `gen_if` scope-checks the dropped branch with the CORRECT lexical env — the same
            // dropped-branch scope check the unselected-`if`-branch case uses. So the short-circuited
            // unbound-name reject (`(and false undefined-name)`→CDZ0101) fires there, with a scoped env.
            "and" | "or" if elems.len() == 3 => {
                for operand in &elems[1..3] {
                    if let Some(t) = self.static_type(operand, env) {
                        if t != StaticType::Bool {
                            return reject("CDZ0201", "boolean connective operand is not a Bool");
                        }
                    }
                }
            }
            "not" if elems.len() == 2 => {
                if let Some(t) = self.static_type(&elems[1], env) {
                    if t != StaticType::Bool {
                        return reject("CDZ0201", "boolean negation operand is not a Bool");
                    }
                }
            }
            // `unquote` / `unquote-splicing` are only meaningful INSIDE a quasiquote (where
            // `quote_node`/`quote_to_ast` consume them). Reaching one as an ordinary form means
            // it appeared outside a quasiquote — a syntax error (CDZ0401).
            "unquote" | "unquote-splicing" => {
                return reject("CDZ0401", "unquote outside quasiquote");
            }
            // `,@ <expr>` (unquote-splicing) evaluates its operand to a LIST and splices the
            // list's ELEMENTS into the parent (metaprogramming.md #Quasiquote Constructs AST With
            // Selective Evaluation). Splicing a NON-list — `(f ,@5)`, `(f ,@x)` with x an Int64 —
            // has no elements to splice and is ill-typed (CDZ0201). Scan the quasiquote's
            // immediate body for a splice whose operand's static type is a known non-list.
            // (Reported here with the honest code rather than the misleading "unbound name:
            // quasiquote" a failed `quote_node` surfaces — 12-metaprogramming.sexp §"splicing a
            // non-list value into a quasiquote is a type error".)
            "quasiquote" if elems.len() == 2 => {
                // `unquote`/`unquote-splicing` each take EXACTLY ONE operand (metaprogramming.md
                // #Quasiquote Constructs AST With Selective Evaluation). A malformed `(unquote 1
                // 2)` — the body itself or nested anywhere in it — is rejected (CDZ0201) rather
                // than silently taking the first operand and dropping the rest (12-metaprogramming
                // .sexp §"unquote with more than one operand inside a quasiquote is malformed").
                if let Some(code) = malformed_unquote_arity(&elems[1]) {
                    return reject(code, "unquote/unquote-splicing takes exactly one operand");
                }
                if let Node::List(body) = &elems[1] {
                    for child in body {
                        if let Node::List(ci) = child {
                            if name_of(ci.first()) == Some("unquote-splicing") && ci.len() == 2 {
                                if let Some(t) = self.static_type(&ci[1], env) {
                                    if t != StaticType::List {
                                        return reject(
                                            "CDZ0201",
                                            "unquote-splicing of a non-list value",
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Member access `(. obj field)` projects a RECORD field; applied to any KNOWN
            // non-record value (a scalar, string, tuple, map, sum, …) it is a type error
            // (CDZ0201). `Int64.max`-style prelude constants and module-record projections
            // resolve to a record or to an unknown, so are not rejected here.
            "." if elems.len() == 3 => {
                if let Some(t) = self.static_type(&elems[1], env) {
                    if t != StaticType::Record {
                        return reject("CDZ0201", "member access on a non-record");
                    }
                }
                // Member-access well-formedness is TWO conditions: the operand is a record (above)
                // AND the record HAS the named field (type-system.md #A Record Is Restricted To A
                // Named Set Of Its Fields; core-semantics.md #Member Access Projects A Record Field —
                // naming a field the record does not contain is a compile-time type error). Projecting
                // a field the record's type does not carry — `(. (record (x 1)) z)` — has no defined
                // result, so it is a STATIC type error (CDZ0201), rejected here BEFORE lowering rather
                // than deferred to a runtime trap (the emit path `gen_member` would otherwise lower a
                // missing field to `unreachable`, a component that traps rather than a rejected
                // program). Only when the operand resolves to a COMPILE-TIME record (its field set is
                // known) and the field is a plain name absent from it; a runtime-record operand
                // (a parameter) resolves to no `(record …)` and imposes nothing (the field-presence
                // twin of the tuple-arity range check below). This is the record-operand-MISSING-field
                // sibling the "operand-is-record" check above did not cover (the MASTER PATTERN:
                // projection well-formedness = operand-is-record AND field-exists).
                //
                // EXCLUDE a built-in MODULE operand (`List`/`Bytes`/`String`/`Ast`/`Int64`): its dotted
                // access `(List.concat …)` is the built-in-operation dispatch surface (routed by
                // `gen_runtime_member` / the applied-dotted path), NOT ordinary record projection — and
                // its module record deliberately lists only the ops wired as bare builtin-refs, so an op
                // reached only through application (or one not yet added to the record) must NOT be
                // rejected here as a "missing field". Only a genuine user/data record is field-checked.
                let operand_is_builtin_module = matches!(&elems[1], Node::Name(n) if builtin_module_record(n).is_some());
                if !operand_is_builtin_module {
                    if let Some(field) = meta_field_key(&elems[2]).as_deref().or_else(|| name_of(elems.get(2))) {
                        if let Some(fields) = self.resolved_record_fields(&elems[1], env) {
                            if !fields.iter().any(|f| f == field) {
                                return reject(
                                    "CDZ0201",
                                    format!("record has no field `{field}`"),
                                );
                            }
                        }
                    }
                }
            }
            // Positional tuple access `(tuple.N t)` projects element N of a TUPLE; applied to any
            // KNOWN non-tuple value (a scalar `(tuple.0 5)`, a record `(tuple.0 (record …))`, …) it
            // is a type error (CDZ0201), the positional-accessor mirror of member access on a
            // non-record above (05-compound-types.sexp §"tuple access on a non-tuple/record is a
            // type error").
            _ if head.starts_with("tuple.") && elems.len() == 2 => {
                if let Some(t) = self.static_type(&elems[1], env) {
                    if t != StaticType::Tuple {
                        return reject("CDZ0201", "tuple access on a non-tuple");
                    }
                }
                // The tuple's ARITY is part of its type (type-system.md #A Tuple Is Split At A
                // Position Into A Prefix And A Suffix), so `(tuple.3 (tuple 10 20 30))` — position 3
                // of a 3-element tuple (valid 0..2) — names an element the tuple does not have and is
                // a STATIC type error (CDZ0201), NOT a runtime trap deferred to `arr-get`. Range-check
                // whenever the operand's tuple arity is statically known — a `(tuple …)` literal, a
                // let-bound tuple, OR a tuple RETURNED by a function (`(tuple.2 (mk))` where `(mk)`
                // returns `(tuple 1 2)`): `resolve` beta-reduces the call to its `(tuple …)` body, so
                // the arity is known at the projection site exactly as for a literal. An operand of
                // genuinely unknown arity (a PARAMETER tuple, whose shape is not known in the callee)
                // resolves to no `(tuple …)` and imposes nothing — it declines later, never a false
                // reject (05-compound-types.sexp §"a positional tuple access out of the tuple's static
                // arity is a type error" + §"… on a function-returned tuple is a type error, not a
                // trap"). `resolve` subsumes the const-fold path (a `(tuple …)` literal resolves to
                // itself) and also reaches the fn-return / alias cases const-fold does not.
                if let Ok(idx) = head[6..].parse::<usize>() {
                    if let Some(arity) = self.resolved_tuple_arity(&elems[1], env) {
                        if idx >= arity {
                            return reject("CDZ0201", format!(
                                "tuple position {idx} is out of the tuple's arity {arity}"
                            ));
                        }
                    }
                }
            }
            // A list is an ordered HOMOGENEOUS sequence (collections-and-text.md #A List Is An
            // Ordered Homogeneous Sequence): every element shares one type. Two elements whose
            // statically-known types disagree — `(list 1 true)`, `(list 1 2.5)` — make the list
            // non-homogeneous and ill-typed (CDZ0201). Numeric types are distinct and do not
            // silently unify, so Int64 and Float64 mix is a mismatch too. Elements whose type is
            // not statically known impose nothing (a not-yet-known rule is not a rejection).
            "list" if elems.len() > 2 => {
                let mut seen: Option<StaticType> = None;
                for e in &elems[1..] {
                    if let Some(t) = self.static_type(e, env) {
                        match seen {
                            Some(prev) if prev != t => {
                                return reject("CDZ0201", "list elements do not share one type");
                            }
                            None => seen = Some(t),
                            _ => {}
                        }
                    }
                }
                // The coarse-KIND check above catches an Int-vs-Bool mix but not two elements of the
                // same KIND but different SHAPE — `(list (record (a 1)) (record (b 2)))` (both
                // records, different field sets) or `(list (tuple 1 2) (tuple 1 2 3))` (both tuples,
                // different arities). Those are different types too, so the list is not homogeneous
                // (CDZ0201) — the same shape distinction the equality path applies, per element
                // against the first const-foldable element (05-compound-types.sexp §"a list of
                // records/tuples with different field sets/arities is a type error").
                if let Some(first) = self.first_const_element(&elems[1..], env) {
                    for e in &elems[2..] {
                        if let Ok(Some(v)) = self.eval_const(e, env) {
                            if self.shapes_incompatible(&first, &v) {
                                return reject("CDZ0201", "list elements do not share one shape");
                            }
                        }
                    }
                }
            }
            // A record has a fixed SET of named fields (core-semantics.md #A Record Has A Fixed
            // Set Of Named Fields): a field name appearing twice — `(record (a 1) (a 2))`, or
            // non-adjacent `(record (a 1) (b 2) (a 3))` — is not a set and is ill-typed (CDZ0201),
            // since it makes `(. r a)` ambiguous. Checked over the whole field list, not only
            // consecutive names.
            "record" if elems.len() > 1 => {
                // Each entry MUST be a `(name value)` pair (core-semantics.md #A Record Has A
                // Fixed Set Of Named Fields). A malformed entry — `(a)` with no value, a bare
                // non-list, or a `(a v w)` with an extra operand — is ill-typed (CDZ0201). This
                // rejection runs BEFORE `eval_const` reaches for the value node, so a
                // value-less entry is a compile-time rejection rather than a codegen panic
                // (never-crash: constitution never emits a component for an ill-formed program).
                if malformed_kv_entry(&elems[1..]) {
                    return reject("CDZ0201", "a record entry is not a (name value) pair");
                }
                if let Some(dup) = duplicate_field_name(&elems[1..]) {
                    return reject("CDZ0201", format!("record names the field `{dup}` more than once"));
                }
            }
            // A map associates keys of one type with VALUES of one type (collections-and-text.md
            // #A Map Associates Keys With Values), and MUST contain each key at most once. A
            // repeated key `(map (a 1) (a 2))` is ill-typed (CDZ0201) — the association is
            // ambiguous. Two entries whose statically-known value types disagree — `(map (a 1) (b
            // true))`, `(map (a 1) (b 2.5))` — make the map non-homogeneous, also CDZ0201.
            "map" if elems.len() > 1 => {
                // A map entry is likewise a `(key value)` pair; a malformed entry — `(a)` with no
                // value, or an over-long `(a v w)` — is ill-typed (CDZ0201), rejected before
                // `eval_const` indexes the absent value node (the never-crash companion to the
                // record case above).
                if malformed_kv_entry(&elems[1..]) {
                    return reject("CDZ0201", "a map entry is not a (key value) pair");
                }
                if let Some(dup) = duplicate_field_name(&elems[1..]) {
                    return reject("CDZ0201", format!("map contains the key `{dup}` more than once"));
                }
                let mut seen: Option<StaticType> = None;
                let mut first_val: Option<CVal> = None;
                // A map associates KEYS of one type too (collections-and-text.md #A Map Associates
                // Keys With Values), so a literal whose keys' statically-known types disagree — `(let
                // ((j 5)) (let ((k true)) (map (j 1) (k 2))))`, keys 5 (Int64) and true (Bool) — is
                // non-homogeneous (CDZ0201), the KEY sibling of the value check below (already enforced
                // on the `Map.insert` path). A key is an ordinary expression, so consult its
                // `static_type`; a key whose type is not statically known (an unbound bare name — the
                // separate coercion case — or an opaque runtime value) imposes nothing. A bound-name
                // key resolves to its value's type, so `j`/`k` above are Int64/Bool and mismatch.
                let mut seen_key: Option<StaticType> = None;
                for entry in &elems[1..] {
                    if let Node::List(kv) = entry {
                        if kv.len() == 2 {
                            if let Some(kt) = self.static_type(&kv[0], env) {
                                match seen_key {
                                    Some(prev) if prev != kt => {
                                        return reject("CDZ0201", "map keys do not share one type");
                                    }
                                    None => seen_key = Some(kt),
                                    _ => {}
                                }
                            }
                            if let Some(t) = self.static_type(&kv[1], env) {
                                match seen {
                                    Some(prev) if prev != t => {
                                        return reject("CDZ0201", "map values do not share one type");
                                    }
                                    None => seen = Some(t),
                                    _ => {}
                                }
                            }
                            // Same-KIND but different-SHAPE map values — record values with
                            // different field sets, tuple values of different arities — are values
                            // of different types too, so the map is not value-homogeneous (CDZ0201),
                            // mirroring the list-element shape check (05-compound-types.sexp §"a map
                            // with record/tuple values of different field sets/arities is a type
                            // error").
                            if let Ok(Some(v)) = self.eval_const(&kv[1], env) {
                                match &first_val {
                                    Some(fv) if self.shapes_incompatible(fv, &v) => {
                                        return reject("CDZ0201", "map values do not share one shape");
                                    }
                                    None => first_val = Some(v),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            // A sum-type CONSTRUCTOR is a single-arity function (core-semantics.md #A Sum Type
            // Constructor Is A Single-Arity Function; #Functions Are Single-Arity). So `(Some 1
            // 2)` desugars to `((Some 1) 2)` — applying the complete Sum value `(Some 1)`, which
            // is not a function, to `2`: an apply-a-non-function type error (CDZ0201), NOT a
            // silent drop of the extra argument. A NOMINAL RECORD constructor `(Point (x 0) (y
            // 0))` also has a capitalized head with >1 operand, but its operands are all labeled
            // `(field value)` fields — that is a record literal, not an over-application — so the
            // rejection is limited to constructors whose extra operands are positional.
            _ if is_constructor_name(head)
                && elems.len() > 2
                && !elems[1..].iter().all(is_labeled_field) =>
            {
                return reject("CDZ0201", "over-applying a single-arity constructor");
            }
            _ => {}
        }
        Ok(())
    }

    /// Recursively type-check every form in a subtree, running `check_type_rejections` at each
    /// list node. The scalar `emit` path checks each form as it lowers it, but a body that
    /// const-folds to a COMPOUND value takes the resource path in `compile_module` WITHOUT
    /// emitting its sub-forms — so its type rejections would never run. This walk restores them
    /// for the compound path: an ill-typed form anywhere in the tree (a non-homogeneous list, a
    /// cross-type comparison, …) is rejected before a resource is emitted. It only ever REJECTS
    /// on statically-known mismatches (an unknown type imposes nothing), so it cannot introduce a
    /// false rejection even though it does not thread binding-form scopes.
    fn check_tree(&self, node: &Node, env: &[Local]) -> Result<(), Decline> {
        if let Node::List(elems) = node {
            // Do not descend into forms whose sub-nodes are NOT evaluated as ordinary
            // expressions, or their child forms would be misread: a `quote`/`quasiquote` body is
            // quoted data (an `unquote` inside it is legal, not a stray form); a `match`'s arms
            // are `(pattern body)` pairs whose pattern `(1 "one")` is not an application. These
            // forms own their own checking via `eval_const`/`gen_match`. Everything else is an
            // ordinary expression subtree, checked at each node.
            match name_of(elems.first()) {
                // A `quote` body is inert quoted DATA — not descended into as ordinary expressions.
                // BUT an `unquote`/`unquote-splicing` appearing inside a PLAIN quote is outside any
                // quasiquote (a plain quote is not a selective-evaluation template — metaprogramming.md
                // §Quote Produces An AST Value), so it is the same `,`-outside-quasiquote syntax error
                // a bare `,x` is (CDZ0401). Scan the quote body for such an unquote (one not enclosed
                // by a nested quasiquote, which would consume it) and reject; otherwise the body is
                // inert and needs no further check. (12-metaprogramming.sexp §"an unquote nested inside
                // a plain quote is a syntax error".)
                Some("quote") => {
                    if let Some(body) = elems.get(1) {
                        if unquote_outside_quasiquote(body, 0) {
                            return reject("CDZ0401", "unquote outside quasiquote");
                        }
                    }
                    return Ok(());
                }
                // `match` owns its own checking of its ARMS (a pattern `(1 "one")` / `(I x)` is not an
                // ordinary application). But the SCRUTINEE is an ordinary expression and MUST be checked
                // as one — else a wrong-payload constructor written DIRECTLY in scrutinee position
                // (`(match (I true) …)` where `I`'s payload is Int64) bypasses the constructor-
                // application payload check that every other position runs, and the ill-typed value
                // flows through the arm binder and out (a wrong VALUE — c82). A let-bound scrutinee was
                // already caught because the `let` value goes through ordinary checking first; this
                // makes the inline scrutinee checked identically. Recurse into the scrutinee only
                // (elems[1]); the arm patterns/bodies stay owned by `gen_match`/`eval_const`.
                Some("match") if elems.len() >= 2 => return self.check_tree(&elems[1], env),
                Some("match") => return Ok(()),
                // A `(type Name (V1 payload | V2 | …))` declaration's body is variant-declaration
                // SYNTAX, not an ordinary expression: `(Red | Green | Blue)` reads as the head
                // `Red` applied to `| Green | Blue`, which a node-local check would misread as an
                // over-applied constructor (CDZ0201). It carries no runtime sub-expression to
                // type-check, so prune it entirely — `collect_sum_types` is the only pass that
                // reads a `(type …)` body.
                Some("type") => return Ok(()),
                // A `quasiquote` body is quoted data too, so do not RECURSE into it — but still
                // run `check_type_rejections` on the quasiquote node itself, which checks that an
                // `unquote-splicing` operand is a list (a splice type error hidden in the quoted
                // body would otherwise be pruned away here).
                Some("quasiquote") => return self.check_type_rejections(elems, env),
                // An annotation `(: value Type)`: the SECOND operand is a TYPE node, not an ordinary
                // expression — `(Tuple Int64 Int64)`, `(List Bool)`, `(Record (a Int64))` are type
                // constructors, not applications. Descending into it as an expression misreads a
                // capitalized multi-operand type head as an over-applied constructor (`(: (tuple 1 2)
                // (Tuple Int64 Int64))` wrongly declined "over-applying a single-arity constructor").
                // Run `check_type_rejections` on the `:` node itself (which does the annotation-vs-value
                // contradiction check), then recurse ONLY into the VALUE operand — never the type.
                Some(":") if elems.len() == 3 => {
                    self.check_type_rejections(elems, env)?;
                    return self.check_tree(&elems[1], env);
                }
                _ => {}
            }
            self.check_type_rejections(elems, env)?;
            for child in elems {
                self.check_tree(child, env)?;
            }
        }
        Ok(())
    }

    /// Are two compile-time values of INCOMPATIBLE shape — comparable coarse type but a shape
    /// mismatch that makes them different types? Records with different field-name sets,
    /// tuples of different length, and sums of DIFFERENT declared types are incompatible (a
    /// comparison type error). Two variants of the SAME sum (`Some` vs `None`) are compatible
    /// — an ordinary unequal, not a type error — decided by the declared sum-type map, not by
    /// hardcoded variant names.
    fn shapes_incompatible(&self, a: &CVal, b: &CVal) -> bool {
        match (a, b) {
            (CVal::Record(x), CVal::Record(y)) => {
                let keys = |v: &Vec<(String, CVal)>| v.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
                // A record's FIELD SET is part of its type: different field sets are incompatible;
                // identical field sets are compatible only if every corresponding value is
                // shape-compatible too — shape matching is RECURSIVE (a nested field-set mismatch
                // is a mismatch).
                if keys(x) != keys(y) {
                    return true;
                }
                x.iter().zip(y).any(|((_, vx), (_, vy))| self.shapes_incompatible(vx, vy))
            }
            // Two maps are ALWAYS shape-compatible: a map's KEY SET is runtime data, not part of
            // its type (unlike a record's field set), so maps with different keys, sizes, or an
            // empty vs non-empty map are simply UNEQUAL — not a type error (05-compound-types.sexp
            // §"two maps with different keys are unequal, not a type error"). Sharing the record
            // arm here was the different-keyset-map miscompile (and its list-of-maps homogeneity
            // manifestation): a map's field-set is not a shape.
            (CVal::Map(_), CVal::Map(_)) => false,
            // Two tuples: incompatible if their arities differ, OR if any element pair is
            // incompatible (recursive — a nested tuple-arity or element-kind mismatch counts).
            (CVal::Tuple(x), CVal::Tuple(y)) => {
                x.len() != y.len()
                    || x.iter().zip(y).any(|(ex, ey)| self.shapes_incompatible(ex, ey))
            }
            // Two lists: a list's LENGTH is runtime data, NOT part of its type (a list is `List
            // ElemType`, an ordered homogeneous sequence of unbounded length) — UNLIKE a tuple's
            // arity. So two lists of DIFFERENT length are the SAME type: comparing them is well-typed
            // and simply UNEQUAL, and an `if` with two different-length list branches is well-typed
            // (05-compound-types.sexp §"two lists of different length are unequal, not a type error" /
            // §"a conditional with two list branches of different length is well-typed"). Sharing the
            // tuple arm's `len() != len()` here was a false rejection (the list analogue of the
            // different-keyset-MAP miscompile — a length/key-set is not a shape). Incompatible ONLY if
            // a corresponding ELEMENT pair is incompatible (different element TYPE); a length
            // difference alone is compatible. Both lists are element-homogeneous (enforced at
            // construction), so comparing the overlapping prefix suffices to catch an element-type
            // mismatch.
            (CVal::List(x), CVal::List(y)) => {
                x.iter().zip(y).any(|(ex, ey)| self.shapes_incompatible(ex, ey))
            }
            (CVal::Sum { variant: vx, payload: px }, CVal::Sum { variant: vy, payload: py }) => {
                // Look up each variant's declared sum type by its TAG (last segment of a
                // possibly-qualified `Type.Variant`). Two variants of DIFFERENT sum types are
                // incompatible. Two variants of the SAME sum type are COMPATIBLE — even different
                // variants (`Some 1` vs `None unit`), which simply compare unequal, not a type
                // error; their payload shapes legitimately differ per variant. Only when it is the
                // SAME variant do the payloads occupy the same position, so only then is a payload
                // shape mismatch a nested incompatibility.
                match (self.sum_types.get(variant_tag(vx)), self.sum_types.get(variant_tag(vy))) {
                    (Some(tx), Some(ty)) if tx != ty => true,
                    _ if variant_tag(vx) == variant_tag(vy) => self.shapes_incompatible(px, py),
                    _ => false,
                }
            }
            // A COARSE-kind mismatch at a nested position — a tuple against a scalar, a record
            // against a list — is a shape mismatch too (the elements at that position are different
            // types). Two scalars of the same kind, or two values whose coarse kinds match and are
            // handled above, are compatible. Compare coarse kinds to catch a kind mismatch that the
            // structural arms above do not (e.g. `(tuple …)` vs an Int).
            (x, y) => StaticType::of_cval(x) != StaticType::of_cval(y),
        }
    }

    /// Does the annotation `ty` PROVABLY contradict `value`'s type? Recursive, so a mismatch at ANY
    /// depth is caught (type-system.md #Annotations Constrain, Never Contradict). Descends the three
    /// shape-carrying type forms into their VALUE sub-nodes:
    ///   • `(Option T)` / `(Result T E)` over a value-carrying constructor `(Some p)`/`(Ok p)`/`(Err
    ///     p)` → check `p` against `T`/`E` recursively (so `(: (Some (Some 5)) (Option (Option Bool)))`
    ///     descends twice: inner `(Some 5)` vs `(Option Bool)` → payload `5` vs `Bool` → contradiction);
    ///   • `(List E)` over a `(list e0 e1 …)` → check EACH element against `E` recursively (so `(:
    ///     (list 1 2) (List Bool))` rejects: `1` vs `Bool`);
    ///   • `(Tuple T0 T1 …)` over a `(tuple e0 e1 …)` of matching arity → check element-wise.
    /// Returns `true` for a provable structural contradiction: a scalar-leaf type mismatch
    /// (`matches_annotation` false on a statically-known type), a compound value under a wrong-KIND
    /// compound annotation, a record field-set mismatch, OR a tuple ARITY mismatch (a tuple's length
    /// is part of its type). An unprovable leaf (a param/runtime value of unknown type) or an unknown
    /// type form is left conservative (decline-don't-miscompile: a not-yet-known typing is never a
    /// false rejection). The head-level scalar contradiction (`(: (tuple …) Int64)`) is still caught by
    /// `matches_annotation` at the `:` arm; this adds the nested-parameter depth.
    fn annotation_contradicts(&self, value: &Node, ty: &Node, env: &[Local]) -> bool {
        let ann_items = match ty {
            Node::List(items) if !items.is_empty() => items,
            _ => return false, // a bare type NAME is handled by the head-level `matches_annotation`
        };
        let head = name_of(ann_items.first());
        match (head, value) {
            // `(Option T)` / `(Result T E)` over a constructor application: descend the payload.
            (Some("Option") | Some("Result"), Node::List(v)) => {
                if let Some(ctor) = constructor_of(v.first()) {
                    let param_idx = match (head, variant_tag(&ctor)) {
                        (Some("Option"), "Some") => 1,
                        (Some("Result"), "Ok") => 1,
                        (Some("Result"), "Err") => 2,
                        _ => return false, // a nullary variant (None/…) carries no payload to check
                    };
                    if let (Some(payload), Some(param_ty)) = (v.get(1), ann_items.get(param_idx)) {
                        return self.annotation_leaf_or_nested_contradicts(payload, param_ty, env);
                    }
                }
                false
            }
            // `(List E)` over a `(list …)`: every element must be compatible with `E`.
            (Some("List"), Node::List(v)) if name_of(v.first()) == Some("list") => {
                let elem_ty = match ann_items.get(1) {
                    Some(t) => t,
                    None => return false,
                };
                v[1..].iter().any(|e| self.annotation_leaf_or_nested_contradicts(e, elem_ty, env))
            }
            // `(Tuple T0 T1 …)` over a `(tuple e0 e1 …)`: a tuple's ARITY is part of its type
            // (type-system.md #A Tuple Is Reshaped Positionally …), so a length mismatch is itself a
            // contradiction (CDZ0203) — `(: (tuple 1 2) (Tuple Int64 Int64 Int64))` annotates a
            // two-tuple as a three-tuple, which cannot unify (07-type-system.sexp §"a tuple annotated
            // with the wrong arity is rejected"). Then, at MATCHING arity, each position's element type
            // is checked. A checker that walked only the shared positions and ignored the length
            // silently accepted the ill-typed program.
            (Some("Tuple"), Node::List(v)) if name_of(v.first()) == Some("tuple") => {
                let elems = &v[1..];
                let params = &ann_items[1..];
                if elems.len() != params.len() {
                    return true; // arity mismatch — a tuple's length is part of its type
                }
                elems.iter().zip(params).any(|(e, t)| self.annotation_leaf_or_nested_contradicts(e, t, env))
            }
            // `(Record (a Ta) (b Tb) …)` over a `(record (a va) (b vb) …)`: each declared field's
            // TYPE must be compatible with the corresponding value field — the record companion of
            // the tuple-position / list-element / sum-payload checks (type-system.md #Annotations
            // Constrain, Never Contradict). A checker that stops at the head `Record` and the field
            // NAMES silently accepts `(: (record (a 1)) (Record (a Bool)))` — the field `a`'s value
            // is Int64 but its declared type is Bool, a contradiction (CDZ0203). Match value fields to
            // annotation fields BY NAME (a record is order-independent); a value field the annotation
            // does not mention, or vice versa, is a field-SET mismatch (a different shape check, not
            // this type check) and is left alone here.
            (Some("Record"), Node::List(v)) if name_of(v.first()) == Some("record") => {
                // The field-NAME SET must agree: a record type is its exact set of labeled fields
                // (type-system.md #A Record Is A Product Of Named Fields), so an annotation naming a
                // field the value lacks (`(: (record (a 1)) (Record (b Int64)))`) or omitting one the
                // value has, or adding an extra (`(Record (a Int64) (b Bool))`), annotates a DIFFERENT
                // record type — a contradiction (CDZ0203), not a silent accept. Compare the two label
                // sets; any asymmetry is a mismatch. (A field the annotation does not mention is NOT a
                // subtype here — this language's record type is the exact field set.)
                let ann_fields: Vec<&str> = ann_items[1..]
                    .iter()
                    .filter_map(|f| match f {
                        Node::List(kv) if kv.len() == 2 => name_of(kv.first()),
                        _ => None,
                    })
                    .collect();
                let val_fields: Vec<&str> = v[1..]
                    .iter()
                    .filter_map(|f| match f {
                        Node::List(kv) if kv.len() == 2 => name_of(kv.first()),
                        _ => None,
                    })
                    .collect();
                // A field named in one set but not the other → field-set mismatch.
                if ann_fields.iter().any(|a| !val_fields.contains(a))
                    || val_fields.iter().any(|b| !ann_fields.contains(b))
                {
                    return true;
                }
                ann_items[1..].iter().any(|field_ty_node| {
                    // Each annotation field is `(name Type)`; find the value field with that name.
                    let ft = match field_ty_node {
                        Node::List(kv) if kv.len() == 2 => kv,
                        _ => return false,
                    };
                    let fname = match name_of(ft.first()) {
                        Some(n) => n,
                        None => return false,
                    };
                    let field_ty = &ft[1];
                    v[1..].iter().any(|value_field| {
                        if let Node::List(vf) = value_field {
                            if vf.len() == 2 && name_of(vf.first()) == Some(fname) {
                                return self.annotation_leaf_or_nested_contradicts(&vf[1], field_ty, env);
                            }
                        }
                        false
                    })
                })
            }
            // A structural annotation head whose KIND disagrees with the value's kind — a `(Record …)`
            // value annotated `(Tuple …)`, a `(tuple …)` value annotated `(List …)`, a sum value
            // annotated `(Tuple …)`, etc. The head-level `matches_annotation` only rejects a compound
            // annotated with a SCALAR name (a compound-vs-compound head mismatch has a non-scalar ann
            // head, so it passed). Match the value's provable `StaticType` against the annotation head's
            // structural kind; a disagreement is a contradiction (CDZ0203). Only fires when BOTH the
            // annotation head is a known structural type constructor AND the value's static type is
            // known — an unknown/opaque value or an unrecognized head imposes nothing (conservative).
            (Some(h), _) if is_structural_type_head(h) => {
                match self.static_type(value, env) {
                    Some(vt) => !static_type_matches_structural_head(vt, h),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// One recursion step for `annotation_contradicts`: a sub-value against a sub-type that may be a
    /// bare scalar NAME (checked via `static_type` + `matches_annotation`) OR a nested shape form
    /// (recurse). Keeps the leaf tolerance (fixed-width/numeric-family) identical to the top level.
    fn annotation_leaf_or_nested_contradicts(&self, value: &Node, ty: &Node, env: &[Local]) -> bool {
        match type_name(ty) {
            // A nested shape form (`(Option …)`, `(List …)`, `(Tuple …)`, `(Record …)`) — recurse.
            Some("Option") | Some("Result") | Some("List") | Some("Tuple") | Some("Record") => {
                self.annotation_contradicts(value, ty, env)
            }
            // A bare scalar type name — compare the value's provable static type.
            Some(name) => self
                .static_type(value, env)
                .map_or(false, |vt| !vt.matches_annotation(name)),
            None => false,
        }
    }

    /// Does an argument `arg` PROVABLY contradict a declared parameter/result TYPE node `ty`? Used to
    /// type-check a perform's arguments against an effect op's declared parameter types and a handler
    /// arm's resume value against the declared result type (capabilities-and-effects.md #Performing An
    /// Operation Is Typed And Contributes To The Row: a perform's arguments are checked against the
    /// declared parameter types, and it yields the declared result type — an effect op is typed exactly
    /// as an ordinary function). Uniform across ALL type shapes, so it fires for a String, a compound
    /// (`(List Int64)`), a tuple/record parameter or result — not only scalars (the coarse `Kind` maps
    /// every non-scalar to `Heap`, which would skip the check). CONSERVATIVE — only a provable mismatch
    /// counts, so a not-yet-inferable arg (a runtime `Heap` value, an opaque local) never false-rejects:
    /// - a structural COMPOUND head (`(List …)`/`(Tuple …)`/`(Record …)`/`(Map …)`/`(Option …)`/
    ///   `(Result …)`): defer to `annotation_contradicts`, which checks the head KIND and, for a literal
    ///   compound arg, its element/field/payload types (an Int arg vs a `(List …)` param → kind
    ///   mismatch; a `(list 1 true)` arg vs `(List Int64)` → element mismatch);
    /// - a bare SCALAR type name (`Int64`/`Float64`/`Bool`/`String`/`Unit`/`Bytes` + the fixed-width
    ///   family): compare the arg's provable `static_type` via `matches_annotation`;
    /// - anything else (a bare user-type name, an unrecognized applied head) imposes nothing — the coarse
    ///   `Heap`-parameter tolerance is preserved for types the seed does not yet check structurally.
    fn arg_contradicts_declared_type(&self, arg: &Node, ty: &Node, env: &[Local]) -> bool {
        match ty {
            // A compound/shape annotation form — reuse the annotation checker's recursion, but only for
            // a recognized structural head (a non-structural applied type imposes nothing).
            Node::List(items) if !items.is_empty() => match name_of(items.first()) {
                Some(h) if is_structural_type_head(h) => self.annotation_contradicts(arg, ty, env),
                _ => false,
            },
            // A bare type NAME — only a known SCALAR name imposes a checkable constraint (a bare
            // user-type / opaque name is left unchecked, as the coarse-Kind path did).
            Node::Name(n) if is_scalar_type_name(n) => self
                .static_type(arg, env)
                .map_or(false, |vt| !vt.matches_annotation(n)),
            _ => false,
        }
    }

    /// A name referenced in `node` that is bound NOWHERE and is a genuine value reference (not a
    /// keyword/constructor/built-in/numeric-literal), or None. CONSERVATIVE: bails (None) on any form
    /// that introduces a binder (`let`/`match`/`fn`, and `do` — which can carry `def`s), since such a
    /// form could bind the name locally and this walk does not track those binders. So it reports only
    /// a name that is unambiguously free (an operand like `(+ b 1)` with `b` unbound), never a false
    /// positive. Head positions of applications are skipped (a call head is resolved separately — a
    /// user fn / operator / dotted intrinsic — not a value name).
    fn provably_unbound_name(&self, node: &Node, env: &[Local]) -> Option<String> {
        match node {
            Node::Name(n) => {
                if n == "unit"
                    || n == "nan"
                    || n == "NaN"
                    || env.iter().any(|l| l.name == *n)
                    || is_form_keyword(n)
                    || is_constructor_name(n)
                    || self.nullary_variants.contains(variant_tag(n))
                    || self.lookup_fn(n).is_some()
                    || builtin_module_record(n).is_some()
                    || looks_like_numeric_literal(n)
                {
                    return None;
                }
                Some(n.clone())
            }
            Node::List(items) => {
                match name_of(items.first()) {
                    // Bail on any binder-introducing form — a local binding could resolve the name.
                    Some("let") | Some("match") | Some("fn") | Some("do") | Some("module")
                    | Some("quote") | Some("quasiquote") => None,
                    // Member access `(. obj field)`: only `obj` is a value reference — `field` is a
                    // record-field / module-member NAME, not a bindable variable (`Int64.max` reads as
                    // `(. Int64 max)`, where `max` is a module member, NOT an unbound name). Check only
                    // the object position. A qualified constructor / builtin-module head like `Int64`
                    // resolves as an object, so it is not reported (it is not a free VALUE name here).
                    Some(".") => items.get(1).and_then(|obj| self.provably_unbound_name(obj, env)),
                    // A list whose head is itself a `.`-list — a qualified constructor application
                    // `((. Type Ctor) payload)` or a dotted-intrinsic application `((. Mod op) args)`.
                    // The head names a constructor/operation (not a value); scan only the arguments.
                    _ if matches!(items.first(), Some(Node::List(_))) => {
                        items[1..].iter().find_map(|c| self.provably_unbound_name(c, env))
                    }
                    // An ordinary application `(f a b …)` with a bare-name head: the head is resolved
                    // as a callee (a user fn / operator / form), not a value name — skip it, scan args.
                    Some(_) => items[1..].iter().find_map(|c| self.provably_unbound_name(c, env)),
                    // A list with no head (empty) — nothing to check.
                    None => None,
                }
            }
            _ => None,
        }
    }

    /// Walk a quasiquote body at nesting `level` (1 = the body of the outermost `quasiquote`) and
    /// return a provably-unbound name referenced by an ACTIVE `unquote`/`unquote-splicing` operand
    /// (one that brings the level to 0), or None. An active unquote's operand is evaluated as an
    /// expression, so an unbound name in it is CDZ0101 (the same scope error the bare expression is);
    /// a nested `quasiquote` RAISES the level (its unquotes are shielded, not active here), and a
    /// deeper `unquote` (level>1) is inert quoted data, not evaluated. Uses `provably_unbound_name`,
    /// which is conservative (bails on binder forms, treats member/callee positions as labels) and is
    /// given the emit-time `env`, so a `let`/param-bound name in an unquote is correctly seen as bound.
    fn quasiquote_active_unquote_unbound(
        &self,
        node: &Node,
        env: &[Local],
        level: u32,
    ) -> Option<String> {
        let items = match node {
            Node::List(items) => items,
            _ => return None,
        };
        match name_of(items.first()) {
            Some("unquote") | Some("unquote-splicing") => {
                if level == 1 {
                    // Active: its operand is evaluated at this level — scope-check the operand.
                    items.get(1).and_then(|inner| self.provably_unbound_name(inner, env))
                } else {
                    // Inert (level>1): the operand is quoted data at one lower level.
                    items.get(1).and_then(|inner| {
                        self.quasiquote_active_unquote_unbound(inner, env, level - 1)
                    })
                }
            }
            // A nested quasiquote shields its body — its unquotes are one level deeper.
            Some("quasiquote") => items
                .get(1)
                .and_then(|inner| self.quasiquote_active_unquote_unbound(inner, env, level + 1)),
            // Any other list: an active unquote may hide in any child at the SAME level.
            _ => items
                .iter()
                .find_map(|c| self.quasiquote_active_unquote_unbound(c, env, level)),
        }
    }

    /// The first element of a list that const-folds to a compile-time value, used as the shape
    /// reference against which later elements are checked for shape compatibility.
    fn first_const_element(&self, elems: &[Node], env: &[Local]) -> Option<CVal> {
        elems.iter().find_map(|e| match self.eval_const(e, env) {
            Ok(Some(v)) => Some(v),
            _ => None,
        })
    }

    /// The static type of a node, when locally determinable (a literal, a typed local, or a
    /// scalar-producing form). `None` when not statically known here.
    fn static_type(&self, node: &Node, env: &[Local]) -> Option<StaticType> {
        match node {
            Node::Int(_) => Some(StaticType::Int),
            Node::Float(_) => Some(StaticType::Float),
            Node::Bool(_) => Some(StaticType::Bool),
            Node::Str(_) => Some(StaticType::Str),
            Node::Name(n) if n == "unit" => Some(StaticType::Unit),
            Node::Name(n) if n == "nan" || n == "NaN" => Some(StaticType::Float),
            // A boolean connective is a Bool regardless of whether its operands are constant, so a
            // RUNTIME `(and a b)` in a condition position (`(if (and a b) …)`) is statically known
            // to be a Bool (core-semantics.md #Boolean Connectives Short-Circuit).
            Node::List(items)
                if matches!(name_of(items.first()), Some("and") | Some("or") | Some("not")) =>
            {
                Some(StaticType::Bool)
            }
            // A name bound to a RUNTIME scalar local (not an alias) is not a compile-time
            // constant, so `eval_const` yields nothing — but its scalar KIND is still statically
            // known, which is enough to know it is not a list/compound. Consult the local's kind.
            Node::Name(n) => match env.iter().rev().find(|l| l.name == *n) {
                Some(l) if l.alias.is_none() => match l.kind {
                    Kind::Int64 => Some(StaticType::Int),
                    Kind::Float64 => Some(StaticType::Float),
                    Kind::Bool => Some(StaticType::Bool),
                    Kind::Unit => Some(StaticType::Unit),
                    Kind::Never => None,
                    Kind::Heap => None, // a runtime compound has no scalar static type
                    Kind::HostString => None, // host-boundary only; never an expression kind
                },
                _ => match self.eval_const(node, env) {
                    Ok(Some(v)) => Some(StaticType::of_cval(&v)),
                    _ => None,
                },
            },
            _ => match self.eval_const(node, env) {
                Ok(Some(v)) => Some(StaticType::of_cval(&v)),
                _ => None,
            },
        }
    }

    /// The nominal type name a node denotes, if it is a nominal record constructor
    /// `(Name (field val)…)` OR a user-declared SUM constructor `(A.Mk 1)` / `(Mk 1)`. A nominal
    /// type's identity is its fully-qualified NAME (type-system.md #Nominal Is An Orthogonal Modifier
    /// Over Any Structural Type), so this returns the TYPE name — the record constructor's own head,
    /// or a sum constructor's DECLARED type — used to detect a comparison across the nominal boundary
    /// (`(= (A.Mk 1) (B.Mk 1))` — distinct nominal sums that share a variant name and payload shape —
    /// is CDZ0202, not a structural `false`). A plain structural record / non-constructor yields None.
    fn nominal_name(&self, node: &Node, _env: &[Local]) -> Option<String> {
        if let Node::List(items) = node {
            // A nominal RECORD constructor `(Point (x 0) (y 0))`: capitalized head, all labeled fields.
            if let Some(Node::Name(h)) = items.first() {
                if is_constructor_name(h)
                    && items.len() > 1
                    && items[1..].iter().all(is_labeled_field)
                {
                    return Some(h.clone());
                }
            }
            // A user-declared SUM constructor — bare `(Mk 1)` or qualified `(A.Mk 1)` (head `(. A Mk)`).
            // Its nominal identity is its DECLARED sum-type name: a qualified head names the type
            // directly (`A`), a bare head resolves via `sum_types[tag]`. Built-in sums (`Option`/
            // `Result`, whose tags are NOT in `sum_types`) are structural/polymorphic, not nominal —
            // they return None here so `(= (Some 1) (Ok 1))` keeps its existing handling. Only a
            // constructor of a user-`(type …)`-declared sum is a nominal boundary.
            if let Some(ctor) = constructor_of(items.first()) {
                let tag = variant_tag(&ctor);
                // A qualified head `(. Type Variant)` names its type explicitly; a bare head looks the
                // declared type up. Either way the type must be a user-declared sum (`sum_variants`).
                let qualified_type = match items.first() {
                    Some(Node::List(h)) if name_of(h.first()) == Some(".") => name_of(h.get(1)),
                    _ => None,
                };
                let ty = match qualified_type {
                    // A QUALIFIED head `(. A Mk)` names its type `A` explicitly — trust it if `A` is a
                    // declared sum type (`sum_variants` is keyed by TYPE name). This is load-bearing
                    // when two types share a variant name (`A`/`B` both declare `Mk`): `sum_types` is
                    // keyed by the bare tag, so `sum_types["Mk"]` holds only ONE of them — consulting
                    // it would misclassify the other constructor as non-nominal. The qualifier is the
                    // authoritative type name.
                    Some(t) if self.sum_variants.contains_key(t) => Some(t.to_string()),
                    Some(_) => None,
                    None => self.sum_types.get(tag).cloned(),
                };
                // The BUILT-IN sums `Option`/`Result`/`Ast` (prelude-declared) are STRUCTURAL, NOT
                // nominal — comparing across them is the ordinary different-variant-set shape error
                // (CDZ0201, handled by `shapes_incompatible`), not a nominal-boundary error (CDZ0202).
                // Only a USER `(type …)`-declared sum is a nominal boundary. Excluding them here keeps
                // `(= (Some 1) (Ok 1))` → CDZ0201 (05-compound-types.sexp §"comparing sums with disjoint
                // variant sets is a type error") rather than mis-coding it CDZ0202. `Ast` is
                // load-bearing here: `(quote 42)` folds to a plain structural `CVal::Ast`, so if
                // `(Ast.Int 42)` were treated as nominal, `(= (quote 42) (Ast.Int 42))` would be a
                // nominal-vs-structural mismatch instead of the true structural equality the AST cases
                // require (they are ONE sum value — 12-metaprogramming.sexp).
                if let Some(ty) = ty {
                    if !is_builtin_structural_sum(&ty) {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    fn gen_list(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // An empty application `()` is the unit value (core-semantics.md §unit is the empty tuple),
        // the same as `eval_const` treats it — so a nullary constructor's `()` payload
        // (`(IntList.Nil ())`) emits as unit rather than falling through to "application with
        // non-name head". Emits no code; its kind is Unit.
        if elems.is_empty() {
            return Ok((Vec::new(), Kind::Unit));
        }

        // Well-formedness FIRST: a special form with the wrong number of operands is malformed
        // and is rejected here, so no later pass (const-fold, type-check, codegen) can index
        // past the end of a short form and panic. A compiler MUST NOT crash on any input.
        check_arity(elems)?;

        // Static type rejections (constitution VII): a program that is not well-typed is
        // refused with its diagnostic code BEFORE any folding or codegen, so the compiler
        // never emits a component for an ill-typed program. Checked ahead of const-folding
        // because folding erases the type information a rejection needs (e.g. a nominal tag).
        // Use the RECURSIVE `check_tree`, not a single-node check: const-folding an operation
        // (e.g. `(= (map (a 1) (b true)) …)`) collapses the whole subtree to one scalar, so an
        // ill-typed NESTED operand (a non-homogeneous map/list) would never be visited by a
        // node-local check. check_tree walks every sub-form (pruning quote/quasiquote/match,
        // which own their checking) and rejects a statically-known mismatch anywhere within.
        self.check_tree(&Node::List(elems.to_vec()), env)?;

        // A `quasiquote` whose ACTIVE unquote references a provably-unbound name is CDZ0101 — an
        // active unquote MUST evaluate its operand (metaprogramming.md #Quasiquote Constructs AST With
        // Selective Evaluation), so an unbound name in it is the ordinary scope error, exactly as the
        // bare expression is (`` `(a ,(+ b 1)) `` with `b` unbound; 12-metaprogramming.sexp §"an
        // unquote of an expression with an unbound name is rejected, not quoted"). Checked HERE at emit
        // with the real lexical `env` (NOT in `check_type_rejections`/`check_tree`, which lacks `let`/
        // `match`/`fn` binders — a check there would false-reject `(let ((b 5)) `(a ,(+ b 1)))`, the
        // ask-66 trap). `quote_node`'s fold returns only None on an unbound operand (loses the coded
        // reject → a bare "declined", scored a todo not a pass), so the CDZ0101 is raised here instead.
        if name_of(elems.first()) == Some("quasiquote") {
            if let Some(body) = elems.get(1) {
                if let Some(bad) = self.quasiquote_active_unquote_unbound(body, env, 1) {
                    return reject("CDZ0101", format!("unbound name: {bad}"));
                }
            }
        }

        // Compile-time constant folding, attempted FIRST after type-checking: an operation
        // over structural constants (equality of records/tuples/sums/bytes/lists → Bool;
        // length/index → Int; out-of-range → trap) has no runtime operand representation, so
        // its only correct lowering is the resulting scalar literal, or `unreachable` for a
        // definite trap. A compound or non-constant result falls through to ordinary codegen.
        // `match`/`let`/`do`/`if` are EXCLUDED here — they own their lowering so that a rejection
        // hidden in a sub-form (e.g. member access on a let-bound map) surfaces via the
        // recursive `emit` of their bodies rather than being masked by a folded `unreachable`
        // (a ConstTrap can't distinguish a runtime trap from a type error). `match` also owns
        // its non-exhaustive → CDZ0210 rejection. `if` is excluded so a CONSTANT-condition `if`
        // reaches `gen_if`, which scope-checks the DROPPED branch (an unbound name in it is CDZ0101
        // even though the fold would drop it — `eval_const` returns only a value, never a scope
        // error); `gen_if` still folds the condition and emits only the taken branch, so this does
        // not reintroduce the 2^depth nested-conditional cost the fold path avoids.
        // `and`/`or`/`not` are excluded too: they desugar to a short-circuit `if` at emit, and only
        // `gen_if` (reached via that desugar) scope-checks the dropped operand with the CORRECT
        // lexical env (a `let`/param binder is in scope there). Folding the connective HERE would emit
        // `false`/`true` without ever scope-checking the short-circuited operand — and the pre-fold
        // `check_type_rejections` cannot scope-check it (it lacks the `let`/`match` binders; ask-66).
        // A connective used as an `if` CONDITION or nested sub-expression still folds via `eval_const`
        // (its arm there is unchanged), so this only reroutes a TOP-LEVEL connective through `gen_if`.
        let head0 = name_of(elems.first());
        if !matches!(head0, Some("match") | Some("let") | Some("do") | Some("if")
            | Some("and") | Some("or") | Some("not"))
        {
            match self.eval_const(&Node::List(elems.to_vec()), env) {
                Ok(Some(v)) => {
                    if let Some(scalar) = self.emit_const(&v) {
                        return Ok(scalar);
                    }
                    // compound constant: fall through (may be consumed structurally elsewhere)
                }
                Ok(None) => {}
                Err(ConstTrap) => return Ok((vec![op::UNREACHABLE], Kind::Never)),
            }
        }
        let head = match elems.first() {
            Some(Node::Name(h)) => h.as_str(),
            // A list head is either `((. obj field) arg)` (a dotted intrinsic like Int.to-byte)
            // or a computed callee `((adder 10) 5)` (curried lambda application). Distinguish
            // by the head's own head.
            Some(Node::List(hd)) => {
                if name_of(hd.first()) == Some(".") {
                    // A PERFORM `(E.op args…)` — head `(. E op)` where `E` is a declared effect and
                    // `op` one of its operations. Routed BEFORE the constructor/lambda/dotted-apply
                    // checks (a performed op is none of those). `gen_perform` resolves the
                    // discharging router statically and lowers it (Tier-1 inline / boundary call /
                    // CDZ0401 if it has no home).
                    if let (Some(e), Some(o)) = (name_of(hd.get(1)), name_of(hd.get(2))) {
                        if self.effects.get(e).map_or(false, |d| d.op(o).is_some()) {
                            return self.gen_perform(e, o, elems, env, ctx);
                        }
                    }
                    // A QUALIFIED CONSTRUCTOR head `(. Type Variant)` applied — `(IntList.Cons …)` —
                    // building a runtime sum. Route to `gen_runtime_sum` with the qualified variant
                    // name (`Type.Variant`), so a recursive user sum type (a linked list, the AST)
                    // constructs at run time, the same as a bare constructor `(Some n)`. Checked
                    // before the intrinsic/lambda paths since a constructor is neither.
                    if let (Some(ty), Some(v)) = (name_of(hd.get(1)), name_of(hd.get(2))) {
                        if is_constructor_name(v) {
                            let qualified = format!("{ty}.{v}");
                            return self.gen_runtime_sum(&qualified, elems, env, ctx);
                        }
                    }
                    // `((. obj field) arg)` — either a prelude intrinsic (Int.to-byte) or a
                    // module-export lambda reached by member access. Prefer lambda inlining
                    // when the projection resolves to a lambda; else the dotted intrinsic.
                    if self.resolve_lambda(&elems[0], env).is_some() {
                        return self.gen_apply(&elems[0], &elems[1..], env, ctx);
                    }
                    return self.gen_dotted_apply(elems, env, ctx);
                }
                return self.gen_apply(&elems[0], &elems[1..], env, ctx);
            }
            // Applying a non-function literal — `(5 3)` — traps ("applied a non-function").
            Some(Node::Int(_)) | Some(Node::Float(_)) | Some(Node::Bool(_))
                if elems.len() > 1 =>
            {
                return Ok((vec![op::UNREACHABLE], Kind::Never));
            }
            _ => return decline("application with non-name head"),
        };
        // A `fn` in value position is a lambda — a compile-time value inlined at its call
        // site, never a scalar. Reaching here means it flowed somewhere it can't be inlined.
        if head == "fn" {
            return decline("bare lambda in scalar position");
        }
        // A name bound to a lambda, applied: inline (compile-time beta reduction).
        if env.iter().rev().any(|l| l.name == head && self.is_lambda_alias(l)) {
            return self.gen_apply(&elems[0], &elems[1..], env, ctx);
        }
        match head {
            // Overflow-checked arithmetic (helper call).
            "+" => self.gen_checked(elems, env, ctx, self.helper_add_idx),
            "-" => self.gen_checked(elems, env, ctx, self.helper_sub_idx),
            "*" => self.gen_checked(elems, env, ctx, self.helper_mul_idx),
            // Direct i64 ops (wasm traps on div/rem by zero and MIN/-1 — matches "overflow traps").
            "/" => self.gen_binop(elems, env, ctx, op::I64_DIV_S, Kind::Int64),
            "%" => self.gen_binop(elems, env, ctx, op::I64_REM_S, Kind::Int64),
            "&" => self.gen_binop(elems, env, ctx, op::I64_AND, Kind::Int64),
            "|" => self.gen_binop(elems, env, ctx, op::I64_OR, Kind::Int64),
            "^" => self.gen_binop(elems, env, ctx, op::I64_XOR, Kind::Int64),
            // Shifts guard the count and (for `<<`) overflow at RUNTIME too, not only in the
            // constant folder — wasm's raw i64.shl/i64.shr_s MASK the count mod 64 and WRAP on
            // overflow, which #Overflow Is Defined forbids. `gen_shift` emits the guard so the
            // runtime path traps identically to the const path (06-numeric-model.sexp §"a runtime
            // left shift by the bit width or more traps", §"a runtime overflowing left shift traps").
            "<<" => self.gen_shift(elems, env, ctx, true),
            ">>" => self.gen_shift(elems, env, ctx, false),
            // Ordering comparisons (result Bool). Emitted for Int64 (i64 signed) and Bool (i32
            // unsigned — false=0 < true=1), the two orderings core-semantics.md #Ordering Where
            // Offered Is Total offers; a cross-type pair was already rejected in
            // check_type_rejections. Most Bool orderings are compile-time constants folded before
            // here; this covers a runtime Bool ordering too (decline-don't-miscompile otherwise).
            "<" | ">" | "<=" | ">=" => self.gen_ordering(head, elems, env, ctx),
            "=" => self.gen_eq(elems, env, ctx),
            "if" => self.gen_if(elems, env, ctx),
            // Boolean connectives DESUGAR to short-circuit conditionals (core-semantics.md #Boolean
            // Connectives Short-Circuit): `(and a b)` = `(if a b false)`, `(or a b)` = `(if a true
            // b)`, `(not a)` = `(if a false true)`. Emitting through `gen_if` reuses the proven
            // short-circuit lowering (only the selected branch runs, so a connective shields a
            // trapping/effectful right operand exactly as an unselected branch does) and Bool result
            // typing; no new machinery. Operand Bool-typing was already enforced in
            // `check_type_rejections` (run via `check_tree` before this), so the desugared `if`'s
            // own branch-agreement check never fires spuriously.
            "and" | "or" | "not" => self.emit(&desugar_connective(head, elems), env, ctx),
            "do" => self.gen_do(elems, env, ctx),
            "let" => self.gen_let(elems, env, ctx),
            "match" => self.gen_match(elems, env, ctx),
            // Intra-program effect handling: `(handle <init> (arms…) body)` discharges an effect in
            // program; `(host (Effect…) body)` delegates a set of effects to the boundary. Both push
            // a router frame, emit their body, pop (options/effects-model/lowering-to-wasm.md).
            "handle" => self.gen_handle(elems, env, ctx),
            "host" => self.gen_host(elems, env, ctx),
            // A bare `resume` reaching codegen is outside a handler arm's Tier-1 rewrite (which
            // consumes `(resume …)`) — a malformed use. Decline (never miscompile).
            "resume" => decline("resume outside a handler arm"),
            // A COMPILER-INTERNAL state accessor `(@state-local N)` — the node a non-unit handler
            // state binder aliases to. Emits `local.get N`, reading the current threaded state. Its
            // kind is the handler's state kind; recover it from the local's declared valtype is not
            // possible here, so the caller (state-threading) ensures its uses type-check. Not a
            // user-writable form (`@` cannot begin a source identifier that reaches here).
            "@state-local" => {
                let slot = match elems.get(1) {
                    Some(Node::Int(n)) => *n as u64,
                    _ => return decline("malformed @state-local"),
                };
                let kind = match elems.get(2) {
                    Some(Node::Int(tag)) => state_kind_untag(*tag),
                    _ => Kind::Int64,
                };
                let mut c = vec![op::LOCAL_GET];
                uleb128(slot, &mut c);
                Ok((c, kind))
            }
            // An `(effect …)` declaration is a compile-time-only form (parsed by `collect_effects`),
            // like `(type …)`; it produces no runtime value. Reaching it in value position is a
            // stray declaration — decline.
            "effect" => decline("effect declaration in value position"),
            // A bare BUILT-IN OPERATION VALUE `(builtin <id>)` in value position — projected from a
            // built-in module record (`(. Bytes len)`) but not applied. Per core-semantics.md §A
            // Built-In Module Is A Record Of Its Operations (ask-58), its only fixed outcome is when
            // APPLIED (`((builtin id) args)`, handled in the call path); as a bare VALUE (a program
            // result, stored, compared) it has no representation yet, so DECLINE — never miscompile.
            // The APPLIED form is intercepted before this arm (a `(builtin id)`-headed application is
            // routed to the built-in lowering), so reaching here is genuinely the unapplied use.
            "builtin" => decline("bare built-in operation value not representable (apply it)"),
            ":" => self.emit(&elems[1], env, ctx), // annotation is transparent at runtime
            "." => self.gen_member(elems, env, ctx),
            _ if head.starts_with("tuple.") => self.gen_tuple_access(head, elems, env, ctx),
            // A tuple/record/list carrying a runtime element reaches here (an all-constant one
            // folded to the baked-text path above). It has no scalar representation — construct it
            // on the value heap via the runtime as a positional array and return a `Kind::Heap`
            // handle (M2 Phase B/C). Only reached when the runtime-compound component shape is
            // active (main returns a heap value); the fold-gate above still handles the
            // all-constant case, keeping those PASS cases byte-identical.
            "tuple" | "record" | "list" => self.gen_runtime_ctor(head, elems, env, ctx),
            // A constructor application `(Some n)` carrying a runtime payload builds a runtime SUM
            // on the value heap (`sum-new(disc, payload)`), returning a `Kind::Heap` handle. An
            // all-constant one folds to the baked-text path above; a qualified variant or a payload
            // the runtime cannot box declines inside `gen_runtime_sum` (decline-don't-miscompile).
            // A capitalized head bound to a user FUNCTION is a CALL, not a constructor — a `(def (Foo
            // x) …)` binds `Foo` in scope and a name resolves to its nearest lexical binding
            // (core-semantics.md #Binding Is Lexical). `gen_call` is checked FIRST for such a name so
            // `(Foo 10)` invokes the function (→ 11), rather than `gen_runtime_sum` synthesizing the
            // value `(Foo 10)` and ignoring the def — a wrong value (09-functions.sexp §"a function
            // whose name is capitalized is called, not treated as a constructor"). Only a capitalized
            // name with NO user def is treated as a sum constructor.
            _ if is_constructor_name(head) && self.lookup_fn(head).is_none() => {
                self.gen_runtime_sum(head, elems, env, ctx)
            }
            _ => self.gen_call(head, elems, env, ctx),
        }
    }

    /// A binary op whose operands are both `expected_operand_kind` and result is `result`.
    fn gen_binop(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
        opcode: u8,
        result: Kind,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("binary op arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        let (b, kb) = self.emit(&elems[2], env, ctx)?;
        if ka != Kind::Int64 || kb != Kind::Int64 {
            return decline("non-integer operand to integer op");
        }
        let mut c = a;
        c.extend_from_slice(&b);
        c.push(opcode);
        Ok((c, result))
    }

    /// An ordering comparison `(< a b)` / `>` / `<=` / `>=`, result Bool. Emitted for the two
    /// types core-semantics.md #Ordering Where Offered Is Total offers an order over: Int64 (i64
    /// signed compare) and Bool (i32 UNSIGNED compare — false=0 < true=1). Both operands share one
    /// kind here — a cross-type pair was rejected earlier in `check_type_rejections` (CDZ0301/
    /// CDZ0201). A non-orderable kind (Float runtime, a heap compound, unit) declines rather than
    /// miscompiling (Float ordering needs a NaN-aware runtime compare, like float equality).
    fn gen_ordering(
        &self,
        head: &str,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("ordering arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        let (b, kb) = self.emit(&elems[2], env, ctx)?;
        let k = match Kind::unify(ka, kb) {
            Some(k) => k,
            None => return decline("ordering of differing kinds"),
        };
        let opcode = match k {
            Kind::Int64 => match head {
                "<" => op::I64_LT_S,
                ">" => op::I64_GT_S,
                "<=" => op::I64_LE_S,
                _ => op::I64_GE_S,
            },
            Kind::Bool => match head {
                "<" => op::I32_LT_U,
                ">" => op::I32_GT_U,
                "<=" => op::I32_LE_U,
                _ => op::I32_GE_U,
            },
            _ => return decline("non-integer/bool operand to ordering op"),
        };
        let mut c = a;
        c.extend_from_slice(&b);
        c.push(opcode);
        Ok((c, Kind::Bool))
    }

    /// A runtime shift `(<< a b)` / `(>> a b)` with the count and overflow guarded inline, so
    /// the emitted code enforces #Overflow Is Defined rather than falling through to wasm's
    /// masking-and-wrapping i64.shl/i64.shr_s. A shift count outside `0..64` has no defined
    /// value (wasm masks it mod 64, turning `<< 64` into `<< 0` and `<< -1` into `<< 63`), so an
    /// out-of-range count traps. A left shift is exact multiplication by 2^count, so it overflows
    /// exactly when the low bits shifted out do not sign-extend back — checked by re-deriving `a`
    /// with an arithmetic right shift. These match `fold_int_op`'s const path exactly, so the two
    /// paths agree (06-numeric-model.sexp runtime-shift cases).
    fn gen_shift(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
        is_left: bool,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("shift arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        let (b, kb) = self.emit(&elems[2], env, ctx)?;
        if ka != Kind::Int64 || kb != Kind::Int64 {
            return decline("non-integer operand to shift");
        }
        let la = ctx.alloc_local(Kind::Int64); // the value being shifted
        let lb = ctx.alloc_local(Kind::Int64); // the shift count
        let mut c = a;
        c.push(op::LOCAL_SET);
        uleb128(la as u64, &mut c);
        c.extend_from_slice(&b);
        c.push(op::LOCAL_SET);
        uleb128(lb as u64, &mut c);
        // Count guard: (u64)count >= 64  → trap. An unsigned compare catches both count >= 64 and
        // a negative count (which is a huge unsigned value).
        c.push(op::LOCAL_GET);
        uleb128(lb as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(64, &mut c);
        c.push(op::I64_GE_U);
        c.extend_from_slice(&[op::IF, 0x40, op::UNREACHABLE, op::END]);
        // Compute the shift: a <op> count.
        c.push(op::LOCAL_GET);
        uleb128(la as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(lb as u64, &mut c);
        c.push(if is_left { op::I64_SHL } else { op::I64_SHR_S });
        if is_left {
            // Overflow guard: a left shift overflows iff arithmetic-shifting the result back by the
            // same count does not recover `a` (a bit was shifted past the sign). Save result, test
            // `(result >> count) != a`, trap on mismatch, then return result.
            let lr = ctx.alloc_local(Kind::Int64);
            c.push(op::LOCAL_SET);
            uleb128(lr as u64, &mut c);
            c.push(op::LOCAL_GET);
            uleb128(lr as u64, &mut c);
            c.push(op::LOCAL_GET);
            uleb128(lb as u64, &mut c);
            c.push(op::I64_SHR_S);
            c.push(op::LOCAL_GET);
            uleb128(la as u64, &mut c);
            c.push(op::I64_NE);
            c.extend_from_slice(&[op::IF, 0x40, op::UNREACHABLE, op::END]);
            c.push(op::LOCAL_GET);
            uleb128(lr as u64, &mut c);
        }
        Ok((c, Kind::Int64))
    }

    /// Overflow-checked add/sub/mul: emit operands then `call` the helper.
    fn gen_checked(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
        helper_idx: u32,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("arith arity");
        }
        // A definitely non-numeric operand (e.g. a string) makes arithmetic a type mismatch,
        // which traps in the dynamic semantics — `(+ 1 "two")`.
        if self.operand_nonnumeric(&elems[1], env) || self.operand_nonnumeric(&elems[2], env) {
            return Ok((vec![op::UNREACHABLE], Kind::Never));
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        let (b, kb) = self.emit(&elems[2], env, ctx)?;
        // Mixing numeric types (e.g. Int64 + Float64) has no defined result in the dynamic
        // semantics — it traps ("numeric type mismatch"). A trap is reproduced by emitting
        // `unreachable`; the trap's reason is not part of the observable projection.
        if arith_type_mismatch(ka, kb) {
            return Ok((vec![op::UNREACHABLE], Kind::Never));
        }
        if ka != Kind::Int64 || kb != Kind::Int64 {
            return decline("non-integer operand to arithmetic");
        }
        let mut c = a;
        c.extend_from_slice(&b);
        c.push(op::CALL);
        uleb128((helper_idx + self.call_base) as u64, &mut c);
        Ok((c, Kind::Int64))
    }

    /// Is `node` a definitely non-numeric operand (a string, bytes, or other compound value)?
    /// Such an operand to an arithmetic op is a type mismatch that traps in the dynamic
    /// semantics. Only reports true when it const-evals to a concrete non-numeric value.
    fn operand_nonnumeric(&self, node: &Node, env: &[Local]) -> bool {
        matches!(
            self.eval_const(node, env),
            Ok(Some(CVal::Str(_)))
                | Ok(Some(CVal::Bytes(_)))
                | Ok(Some(CVal::Tuple(_)))
                | Ok(Some(CVal::List(_)))
                | Ok(Some(CVal::Record(_)))
                | Ok(Some(CVal::Sum { .. }))
                | Ok(Some(CVal::Ast(_)))
        )
    }

    /// Equality: structural over scalars. Int64 uses i64.eq; Bool uses i64.eq on i32s
    /// (fine — both are 0/1); Float uses the canonical-byte-form rule where every NaN is
    /// equal to every NaN, so a plain f64.eq is wrong for nan==nan — handled specially.
    fn gen_eq(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("= arity");
        }
        // Float equality follows the canonical byte form (every NaN equal to every NaN;
        // -0.0 ≠ 0.0), which wasm's f64.eq does not implement. When both operands are
        // compile-time float constants — every realized corpus float-equality case — fold to
        // the boolean per the canonical-byte-form rule.
        if let (Some(x), Some(y)) =
            (self.resolve_float_const(&elems[1], env), self.resolve_float_const(&elems[2], env))
        {
            let eq = float_canonical_eq(x, y);
            return Ok((vec![op::I32_CONST, if eq { 1 } else { 0 }], Kind::Bool));
        }
        // A runtime equality where at least one operand is PROVABLY a byte-backed leaf (a String or
        // Bytes value) is a structural byte comparison. Because `=` is well-typed (its operands share
        // a type), proving ONE side byte-backed means BOTH are — so a length-then-bytewise compare is
        // sound. This is the name-dispatch primitive a Cadenza-authored compiler needs
        // (`(= head "def")`). Detected BEFORE emitting the operands so a bare `Kind::Heap` (a tuple/
        // sum/list, whose structural equality still needs a heap walk) is unaffected. Two bare String
        // params with no provable side decline (decline-don't-miscompile), as does any non-byte heap.
        if self.provably_bytes_like(&elems[1], env) || self.provably_bytes_like(&elems[2], env) {
            return self.gen_runtime_bytes_eq(&elems[1], &elems[2], env, ctx);
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        let (b, kb) = self.emit(&elems[2], env, ctx)?;
        let k = match Kind::unify(ka, kb) {
            Some(k) => k,
            None => return decline("equality of differing kinds"),
        };
        match k {
            Kind::Int64 => {
                let mut c = a;
                c.extend_from_slice(&b);
                c.push(op::I64_EQ);
                Ok((c, Kind::Bool))
            }
            Kind::Bool => {
                // both are i32 (0/1) — i32.eq is 0x46
                let mut c = a;
                c.extend_from_slice(&b);
                c.push(0x46);
                Ok((c, Kind::Bool))
            }
            Kind::Float64 => {
                // Non-constant float equality by the CANONICAL BYTE FORM (core-semantics.md
                // §Floating-Point Equality Follows The Canonical Byte Form): every NaN equals every
                // NaN, and -0.0 ≠ 0.0 (distinct bits). wasm `f64.eq` implements NEITHER (it says
                // nan≠nan AND -0.0==0.0), so we canonicalize each operand's BITS — map any NaN to one
                // canonical NaN bit pattern (the same `f64::NAN` bits the `nan` literal emits), leave
                // every other value's bits alone — then compare the i64 bit reps with `i64.eq`. This
                // is the exact runtime twin of the const-fold `float_canonical_eq` (NaN-both → equal;
                // else `to_bits()` compare). `a`/`b` are the two f64 operand byte sequences.
                Ok((self.emit_float_canonical_eq(&a, &b, ctx), Kind::Bool))
            }
            Kind::Unit => Ok((vec![op::I32_CONST, 1], Kind::Bool)), // unit == unit
            Kind::Never => Ok((a, Kind::Never)),                    // both diverge
            // Structural equality of two runtime heap compounds needs a heap-walking comparator;
            // not yet emitted (decline-don't-miscompile). Const compounds still compare via eval_const.
            Kind::Heap => decline("runtime compound equality (heap walk) not yet emitted"),
            Kind::HostString => decline("host-boundary string equality not emitted"),
        }
    }

    /// Emit canonical-byte-form equality of two f64 operands (`a_code`/`b_code` leave one f64 each
    /// on the stack): `true` iff `canon(a)` and `canon(b)` have identical bits, where `canon(x)` maps
    /// any NaN (of any bit pattern) to the ONE canonical NaN and leaves every other value's bits
    /// intact. This makes every NaN equal every NaN (both canonicalize to the same bits) while
    /// keeping -0.0 ≠ 0.0 (distinct bits, neither is NaN) — the exact rule `float_canonical_eq`
    /// folds. Returns i32 (0/1) on the stack.
    fn emit_float_canonical_eq(&self, a_code: &[u8], b_code: &[u8], ctx: &mut FnCtx) -> Vec<u8> {
        // f64.ne = 0x62 (NaN test: `x != x` is true iff x is NaN); i64.reinterpret_f64 = 0xBD;
        // select = 0x1B (pops [v1, v2, cond] → v1 if cond≠0 else v2). Canonical NaN = the `nan`
        // literal's bits (`f64::NAN.to_le_bytes()`), so a runtime NaN canonicalizes to the same bits
        // a constant `nan` produces — keeping the runtime and const-fold paths in agreement.
        const F64_NE: u8 = 0x62;
        const I64_REINTERPRET_F64: u8 = 0xBD;
        const SELECT: u8 = 0x1B;
        let fa = ctx.alloc_local(Kind::Float64);
        let fb = ctx.alloc_local(Kind::Float64);
        let mut c = Vec::new();
        // fa = a ; fb = b (store both so each can be read twice — for the NaN test and the bit read).
        c.extend_from_slice(a_code);
        c.push(op::LOCAL_SET);
        uleb128(fa as u64, &mut c);
        c.extend_from_slice(b_code);
        c.push(op::LOCAL_SET);
        uleb128(fb as u64, &mut c);
        // canon(local) onto the stack as an i64: select(canonical_nan_bits, reinterpret(local),
        // local != local).
        let mut canon = |slot: u32, c: &mut Vec<u8>| {
            // v1 = canonical NaN bits (i64.const)
            c.push(op::I64_CONST);
            sleb128(f64::NAN.to_bits() as i64, c);
            // v2 = reinterpret_i64(local)
            c.push(op::LOCAL_GET);
            uleb128(slot as u64, c);
            c.push(I64_REINTERPRET_F64);
            // cond = local != local  (true iff NaN)
            c.push(op::LOCAL_GET);
            uleb128(slot as u64, c);
            c.push(op::LOCAL_GET);
            uleb128(slot as u64, c);
            c.push(F64_NE);
            // select → v1 if cond else v2
            c.push(SELECT);
        };
        canon(fa, &mut c);
        canon(fb, &mut c);
        c.push(op::I64_EQ);
        c
    }

    /// Is `node`'s static shape a byte-backed leaf (a String or Bytes value)? A String literal is
    /// one directly; otherwise consult `shape_of` (which sees through fn calls / `if` / `match`). A
    /// bare parameter whose shape is not locally determinable answers `false` — so `(= s "def")`
    /// (one literal side) is byte equality while `(= a b)` of two opaque params is not, and only the
    /// former reaches `gen_runtime_bytes_eq`. Used to route runtime `=` to structural byte compare.
    fn provably_bytes_like(&self, node: &Node, env: &[Local]) -> bool {
        matches!(node, Node::Str(_))
            || matches!(self.shape_of(node, env), Some(Shape::Str) | Some(Shape::Bytes))
    }

    /// Emit a runtime structural byte equality of two String/Bytes values: `true` iff they have the
    /// same length and every byte agrees. Lowers against the frozen `bytes-len`/`bytes-get` heap
    /// imports (a String is a Bytes-backed leaf, so this serves both `String` `=` — the compiler's
    /// name-dispatch primitive — and `Bytes` `=`). Both operands are consumed to their handles;
    /// leaves an i32 (0/1) Bool. Short-circuits on unequal length (no byte loop) and on the first
    /// differing byte. Requires the value-heap runtime (declines on the scalar path).
    fn gen_runtime_bytes_eq(
        &self,
        lhs: &Node,
        rhs: &Node,
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        let (ac, ka) = self.emit(lhs, env, ctx)?;
        if ka != Kind::Heap {
            return decline("byte equality of a non-heap left operand");
        }
        let (bc, kb) = self.emit(rhs, env, ctx)?;
        if kb != Kind::Heap {
            return decline("byte equality of a non-heap right operand");
        }
        let a = ctx.alloc_local(Kind::Bool); // i32 handle a
        let b = ctx.alloc_local(Kind::Bool); // i32 handle b
        let n = ctx.alloc_local(Kind::Bool); // i32 loop index
        let res = ctx.alloc_local(Kind::Bool); // i32 result (0/1)
        const I32_EQ: u8 = 0x46;
        const I32_NE: u8 = 0x47;
        let bytes_len = |h: u32, c: &mut Vec<u8>| {
            c.extend_from_slice(&[op::LOCAL_GET, h as u8, op::CALL]);
            uleb128(himport::BYTES_LEN as u64, c);
        };
        let mut c = ac;
        c.push(op::LOCAL_SET);
        uleb128(a as u64, &mut c);
        c.extend_from_slice(&bc);
        c.push(op::LOCAL_SET);
        uleb128(b as u64, &mut c);
        // res = (bytes-len(a) == bytes-len(b))  — start assuming equal-length; if not, the whole
        // comparison is false and the loop is skipped.
        bytes_len(a, &mut c);
        bytes_len(b, &mut c);
        c.extend_from_slice(&[I32_EQ, op::LOCAL_SET, res as u8]);
        // if res { n = 0; block { loop { if n >= len(a) break; if get(a,n) != get(b,n) { res=0; break } ; n+=1 } } }
        c.extend_from_slice(&[op::LOCAL_GET, res as u8, op::IF, 0x40]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
        c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
        // n >= bytes-len(a) → br 1
        c.extend_from_slice(&[op::LOCAL_GET, n as u8]);
        bytes_len(a, &mut c);
        c.extend_from_slice(&[0x4E /*i32.ge_s*/, op::BR_IF, 1]);
        // if bytes-get(a,n) != bytes-get(b,n) { res = 0 ; br 1 }
        c.extend_from_slice(&[op::LOCAL_GET, a as u8, op::LOCAL_GET, n as u8, op::CALL]);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::LOCAL_GET, n as u8, op::CALL]);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.extend_from_slice(&[I32_NE, op::IF, 0x40]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, res as u8, op::BR, 2]); // br out of loop+block
        c.push(op::END); // if
        // n += 1 ; continue
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
        c.extend_from_slice(&[op::END, op::END]); // loop, block
        c.push(op::END); // outer if res
        c.extend_from_slice(&[op::LOCAL_GET, res as u8]);
        Ok((c, Kind::Bool))
    }

    /// Resolve a node to a compile-time f64 constant (a float literal, `nan`/`NaN`, or a name
    /// aliased to one), for folding canonical-byte-form float equality.
    fn resolve_float_const(&self, node: &Node, env: &[Local]) -> Option<f64> {
        match node {
            Node::Float(f) => Some(*f),
            Node::Name(n) if n == "nan" || n == "NaN" => Some(f64::NAN),
            Node::Name(n) => {
                let local = env.iter().rev().find(|l| l.name == *n)?;
                let (anode, aenv) = local.alias.as_ref()?;
                self.resolve_float_const(anode, aenv)
            }
            _ => None,
        }
    }

    fn gen_if(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 4 {
            return decline("if arity");
        }
        // A compile-time-known condition selects one branch; the other is DEAD and MUST NOT be
        // emitted — matching `eval_const`'s `if` arm, which evaluates only the taken branch.
        // Emitting the untaken branch would (a) fail to terminate for a bounded recursion whose
        // base case is a constant condition (the dead recursive branch would inline forever) and
        // (b) cost 2^depth in nested conditionals. A literal-branch type mismatch is already
        // rejected up front by `check_type_rejections` (independent of which branch is taken), so
        // dropping the dead branch here does not weaken type-checking.
        match self.eval_const(&elems[1], env) {
            Ok(Some(CVal::Bool(b))) => {
                let (taken, dropped) = if b { (&elems[2], &elems[3]) } else { (&elems[3], &elems[2]) };
                // Every branch is scope-checked whether or not evaluated (core-semantics.md #Binding
                // Is Lexical — UNCONDITIONAL — with #Conditionals Evaluate One Branch: an unselected
                // branch cannot carry a deferred scope error). A constant condition drops the untaken
                // branch here (it is NOT emitted, so `gen_name` never sees its names), so a
                // PROVABLY-unbound name in it — `(if true 1 undefined-name)` — would otherwise run to
                // the taken branch's value instead of the unbound-name reject the equivalent
                // dynamic-condition form gives. Reject it up front (CDZ0101), the scope companion of
                // the type check `check_tree` already applies to a dropped branch (`(if true 1 (+ 1
                // true))`). Conservative: `provably_unbound_name` reports ONLY a name bound nowhere
                // in `env` that is not a keyword/constructor/builtin/numeric and bails on any binder-
                // introducing sub-form, so a branch whose names are all resolvable is never falsely
                // rejected (02-binding-and-control.sexp §"an unbound name in an unselected conditional
                // branch is still rejected").
                if let Some(bad) = self.provably_unbound_name(dropped, env) {
                    return reject("CDZ0101", format!("unbound name: {bad}"));
                }
                return self.emit(taken, env, ctx);
            }
            Ok(Some(_)) => return decline("if condition is not Bool"),
            Err(ConstTrap) => return Ok((vec![op::UNREACHABLE], Kind::Never)),
            Ok(None) => {}
        }
        let (cond, kc) = self.emit(&elems[1], env, ctx)?;
        if kc != Kind::Bool {
            return decline("if condition is not Bool");
        }
        let (then_c, kt) = self.emit(&elems[2], env, ctx)?;
        let (else_c, ke) = self.emit(&elems[3], env, ctx)?;
        // A diverging branch (a trap) unifies with the other's kind — so a conditional whose
        // untaken branch traps still has the taken branch's kind.
        let result = match Kind::unify(kt, ke) {
            Some(k) => k,
            None => return decline("if branches differ in kind"),
        };
        let mut c = cond;
        c.push(op::IF);
        c.push(result.core_valtype()); // block result type (Never → i64; the branch traps)
        c.extend_from_slice(&then_c);
        c.push(op::ELSE);
        c.extend_from_slice(&else_c);
        c.push(op::END);
        Ok((c, result))
    }

    fn gen_do(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        let forms = &elems[1..];
        if forms.is_empty() {
            return decline("empty do");
        }
        let mut c = Vec::new();
        // A nested `(module name …)` form binds `name` in the enclosing scope (a record of
        // its exports); it produces no runtime value. Thread an extended env through the do.
        let mut do_env = env.to_vec();
        for (i, form) in forms.iter().enumerate() {
            let last = i + 1 == forms.len();
            // `(module name …)` binds its name as a structural record alias and yields unit.
            if form.head_name() == Some("module") {
                if let Node::List(mitems) = form {
                    if let Some(Node::Name(mname)) = mitems.get(1) {
                        let rec = module_to_record(mitems);
                        do_env.push(Local::aliased(mname.clone(), rec, do_env.clone()));
                        if last {
                            // A module as the final form: its value is the record (structural,
                            // no scalar) — not observable as a scalar; decline.
                            return decline("module as a do-block's final value");
                        }
                        continue;
                    }
                }
            }
            // A `def` DECLARATION in a sequencing block binds its name for the forms that FOLLOW
            // it — no enclosing `let` needed (core-semantics.md §A Declaration In A Sequencing
            // Block Is Scoped To The Forms That Follow It, the same rule `module` uses above). A
            // value def `(def x 5)` binds the value; a function def `(def (f n) body)` binds a
            // lambda `(fn (n) body)` inlined at its call sites. The binding is an alias over the
            // current do env, so a following def sees the ones before it.
            if form.head_name() == Some("def") {
                if let Node::List(ditems) = form {
                    let value = match (ditems.get(1), ditems.get(2)) {
                        // Value declaration: signature is a bare name; bind name → value node.
                        (Some(Node::Name(vname)), Some(vexpr)) => {
                            Some((vname.clone(), vexpr.clone()))
                        }
                        // Function declaration: signature is `(f params…)`; bind f → lambda.
                        (Some(Node::List(sig)), Some(body)) => match sig.first() {
                            Some(Node::Name(fname)) => {
                                let params: Vec<Node> = if sig.len() > 1 {
                                    sig[1..].to_vec()
                                } else {
                                    vec![Node::Name("_".into())] // nullary → takes unit
                                };
                                let lambda = Node::List(vec![
                                    Node::Name("fn".into()),
                                    Node::List(params),
                                    body.clone(),
                                ]);
                                Some((fname.clone(), lambda))
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some((name, node)) = value {
                        do_env.push(Local::aliased(name, node, do_env.clone()));
                        if last {
                            // A declaration as the final form introduces a name but is not itself
                            // a value — a do block yields its last form's value, and a declaration
                            // has none. Decline (same as a trailing module).
                            return decline("declaration as a do-block's final value");
                        }
                        continue;
                    }
                }
            }
            // A `(type Name (V | …))` declaration is a COMPILE-TIME-ONLY form: it registers its
            // variants as constructors (done once, up front, by `collect_sum_types` over the whole
            // program) and produces no runtime value — like `module`/`def` above. It is inert in
            // the sequencing block: skip it (never `emit` it, or its `(V | …)` body would be
            // misread as an over-applied constructor). A trailing `(type …)` has no value, so a
            // do block cannot end on one — decline, matching the `module`/`def` trailing rule.
            if form.head_name() == Some("type") {
                if last {
                    return decline("type declaration as a do-block's final value");
                }
                continue;
            }
            // A non-final form that is a pure COMPOUND constant (a record/tuple/list/… with no
            // scalar wasm value and no effect) is simply discarded — a sequencing block
            // evaluates each form for its effects and keeps only the last value, and a pure
            // compound has no effect and no droppable runtime value. If it instead traps, that
            // IS observable — emit the trap.
            if !last {
                match self.eval_const(form, &do_env) {
                    Ok(Some(v)) if self.emit_const(&v).is_none() => continue, // pure compound: skip
                    Err(ConstTrap) => return Ok((vec![op::UNREACHABLE], Kind::Never)),
                    _ => {}
                }
            }
            let (fc, fk) = self.emit(form, &do_env, ctx)?;
            c.extend_from_slice(&fc);
            if !last {
                // Not the last form: drop its value (unless it produced none, e.g. unit).
                if fk != Kind::Unit && fk != Kind::Never {
                    c.push(op::DROP);
                }
            } else {
                return Ok((c, fk));
            }
        }
        unreachable!()
    }

    fn gen_let(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        let binds = match elems.get(1) {
            Some(Node::List(b)) => b,
            _ => return decline("malformed let bindings"),
        };
        let mut c = Vec::new();
        let mut inner_env = env.to_vec();
        for pair in binds {
            let p = match pair {
                Node::List(p) => p,
                _ => return decline("malformed let binding"),
            };
            let name = match p.first() {
                Some(Node::Name(n)) => n.clone(),
                _ => return decline("let binding without a name"),
            };
            // A structural value (record/tuple/sum/string) is bound as a compile-time alias
            // — no runtime local, no code emitted — capturing the current scope so its free
            // names resolve where it was written.
            if self.is_structural(&p[1], &inner_env) {
                let captured = inner_env.clone();
                inner_env.push(Local::aliased(name, p[1].clone(), captured));
                continue;
            }
            // A binding to a SCALAR LITERAL (`(let ((x 1)) …)`) is likewise bound as a
            // compile-time alias, so a use that must fold to the constant — an `(unquote x)` in a
            // quasiquote, which embeds x's VALUE as an AST node — sees `1`, not the runtime local
            // `Name("x")`. Without this, `` `(f ,x) `` (x a runtime local) builds `(Ast.Name "x")`
            // instead of `(Ast.Int 1)` and wrongly compares unequal to `(quote (f 1))`
            // (12-metaprogramming.sexp §"an AST from quasiquoting a runtime value equals …"). A
            // literal alias still emits identically wherever x is used as a scalar (emit folds it).
            if matches!(p.get(1), Some(Node::Int(_) | Node::Bool(_) | Node::Float(_) | Node::Str(_))) {
                let captured = inner_env.clone();
                inner_env.push(Local::aliased(name, p[1].clone(), captured));
                continue;
            }
            // A binding whose value CONST-FOLDS to a COMPOUND constant — a `(tuple …)`/`(list …)`/
            // `(record …)`/sum returned by a called function, e.g. `(unbox (Box.B (tuple (list)
            // (Term.Var 7))))` reducing through `unbox`'s `match` to `(tuple (list) (Term.Var 7))` —
            // is bound as a compile-time ALIAS to its RECONSTRUCTED structural node, exactly as the
            // scalar-literal and syntactically-structural cases above are. So a later `(tuple.N p)` /
            // `(. p f)` / `match` over the bound name projects the folded structure (the INLINE form
            // `(tuple.1 (unbox …))` already does, via `resolve`), rather than emitting a runtime local
            // whose kind is the callee's inferred return kind. Without this, `unbox`'s return kind
            // mis-infers as the Int64 DEFAULT (its only arm returns an unconstrained payload binder),
            // so `p` binds as an Int64 scalar and `(tuple.1 p)` REJECTS "tuple access on a non-tuple"
            // (CDZ0201) — a valid program wrongly called ill-typed (05-compound-types.sexp §"a tuple
            // payload extracted through a helper return must not be rejected as a type error"). The
            // We RESOLVE the value (compile-time beta reduction of the callee + match/if selection —
            // the SAME reduction the INLINE form `(tuple.1 (unbox …))` already performs via
            // `gen_tuple_access`'s `resolve`), and if it lands on a `(tuple …)`/`(list …)`/`(record …)`/
            // constructor STRUCTURE, alias `p` to it. A later projection over `p` then resolves the
            // structure exactly as the inline form does; the helper (`unbox`) becomes dead and clears
            // to a trap stub. `resolve` returns None (or a non-structure) for a genuine runtime value
            // (a recursive builder, an opaque parameter), so that stays a runtime local on the path
            // below — only a statically-reducible compound is aliased here.
            if let Some((rnode, renv)) = self.resolve(&p[1], &inner_env) {
                let is_structure = matches!(&rnode, Node::List(items)
                    if constructor_of(items.first()).is_some()
                        || matches!(name_of(items.first()), Some("tuple") | Some("list") | Some("record")));
                if is_structure {
                    inner_env.push(Local::aliased(name, rnode, renv));
                    continue;
                }
            }
            let (ec, ek) = self.emit(&p[1], &inner_env, ctx)?;
            if ek == Kind::Unit {
                // Binding a unit value: no storage needed; bind as a unit alias.
                inner_env.push(Local::aliased(name, Node::Name("unit".into()), Vec::new()));
                c.extend_from_slice(&ec); // (no bytes for unit, but keep effects if any)
                continue;
            }
            let idx = ctx.alloc_local(ek);
            c.extend_from_slice(&ec);
            c.push(op::LOCAL_SET);
            uleb128(idx as u64, &mut c);
            // For a materialized HEAP binding (a runtime compound: a tuple/record/sum returned from
            // a function, a bytes value), record the value's static `Shape` so a later `tuple.N`/
            // projection/render on the bound name can see through the opaque handle to the element's
            // kind (the value heap is tag-free — the compiler carries the shape, not the runtime).
            let shape = if ek == Kind::Heap { self.shape_of(&p[1], &inner_env) } else { None };
            inner_env.push(Local::scalar_shaped(name, idx, ek, shape));
        }
        let (bc, bk) = self.emit(elems.last().unwrap(), &inner_env, ctx)?;
        c.extend_from_slice(&bc);
        Ok((c, bk))
    }

    /// Check a match ARM's pattern against the scrutinee's statically-known shape, recursively —
    /// the arity/kind/type rules all COMPOSE (core-semantics.md #Patterns Compose: a pattern's
    /// sub-pattern faces the corresponding sub-value, to any depth). Three rejections, each applied
    /// at every nesting level:
    ///   • a `(tuple …)` pattern must face a tuple of the SAME arity (never a sum — a tuple pattern
    ///     can't match a sum value), then EACH element pattern is checked against the corresponding
    ///     scrutinee element (a nested `(tuple b c d)` facing a 2-element element is a type error);
    ///   • a CONSTRUCTOR pattern `(Some p)` / `(Sign.Pos p)` facing a matching constructor value
    ///     descends into the payload (so `(Some (tuple a b c))` against `(Some (tuple 1 2))` catches
    ///     the wrong-arity inner tuple);
    ///   • a LITERAL pattern (`true`, `5`) must share the scrutinee element's type (a `true` pattern
    ///     against an Int64 element can never match — a type error, not a silent non-match).
    /// A wildcard / name pattern binds without constraining shape (Ok). Only rejects on a STATICALLY-
    /// KNOWN scrutinee shape (a resolved literal/tuple/constructor node); an opaque scrutinee element
    /// (a runtime value with no known form) imposes nothing.
    fn check_pattern_shape(
        &self,
        pattern: &Node,
        scrut_node: &Node,
        scrut_env: &[Local],
    ) -> Result<(), Decline> {
        // A LITERAL pattern must match the scrutinee element's TYPE (Equality Is Structural is
        // defined only within one type). Check against the element's statically-known type; an
        // element of unknown type imposes nothing.
        if let Some(pt) = literal_pattern_type(Some(pattern)) {
            if let Some(st) = self.static_type(scrut_node, scrut_env) {
                if pt != st {
                    return reject(
                        "CDZ0201",
                        "literal pattern type does not match the scrutinee type",
                    );
                }
            }
            return Ok(());
        }
        // A COMPOSITE pattern — a `(tuple …)` or a constructor `(Some p)` — is the only other shape-
        // constraining form; a bare name / `_` binds anything.
        let pitems = match pattern {
            Node::List(p) if !p.is_empty() => p,
            _ => return Ok(()),
        };
        let pat_is_tuple = name_of(pitems.first()) == Some("tuple");
        let pat_ctor = constructor_of(pitems.first());
        if !pat_is_tuple && pat_ctor.is_none() {
            return Ok(()); // not a shape-constraining pattern (e.g. a record/literal-list pattern)
        }
        // Resolve the scrutinee node (following name aliases) to a concrete form, if any.
        let (snode, senv) = match self.resolve(scrut_node, scrut_env) {
            Some(pair) => pair,
            None => return Ok(()), // scrutinee shape not statically known — impose nothing
        };
        let sitems = match &snode {
            Node::List(s) if !s.is_empty() => s,
            _ => return Ok(()),
        };
        let scrut_ctor = constructor_of(sitems.first());
        if pat_is_tuple {
            // A tuple pattern against a SUM value (a constructor application) is the wrong KIND — a
            // tuple pattern can never match a sum (02-binding-and-control.sexp §"a tuple pattern
            // against a sum scrutinee is a type error").
            if scrut_ctor.is_some() {
                return reject("CDZ0201", "tuple pattern against a non-tuple scrutinee");
            }
            // Only a `(tuple …)` scrutinee constrains a tuple pattern's arity; another known form
            // (a record, a list) is a different-kind mismatch left to the ordinary path.
            if name_of(sitems.first()) != Some("tuple") {
                return Ok(());
            }
            if pitems.len() != sitems.len() {
                return reject(
                    "CDZ0201",
                    "tuple pattern arity does not match the scrutinee tuple",
                );
            }
            // Arity matches — recurse element-wise so a nested wrong-arity / wrong-type / wrong-kind
            // pattern is caught (Patterns Compose).
            for (psub, ssub) in pitems[1..].iter().zip(&sitems[1..]) {
                self.check_pattern_shape(psub, ssub, &senv)?;
            }
            return Ok(());
        }
        // A CONSTRUCTOR pattern `(Some p)`: only descends when it matches the scrutinee's OWN
        // constructor — a DIFFERENT constructor (or a non-sum scrutinee) is an ordinary non-match /
        // handled elsewhere, not a shape error to raise here (a `Some`-arm against a `None` value is
        // a legitimate non-match, and cross-variant exhaustiveness is checked separately). When the
        // constructors agree, descend into the single payload sub-pattern against the payload value.
        if let (Some(pc), Some(sc)) = (&pat_ctor, &scrut_ctor) {
            if variant_tag(pc) == variant_tag(sc) && pitems.len() == 2 && sitems.len() == 2 {
                self.check_pattern_shape(&pitems[1], &sitems[1], &senv)?;
            }
        }
        Ok(())
    }

    /// `(match <scrutinee> (<pattern> <body>)…)` — compile-time pattern resolution. Every
    /// match scrutinee the realized corpus uses is inline-constructed (a literal or a
    /// constructor/tuple application), so the matching arm is decided at compile time and
    /// only that arm's body is emitted, with pattern binders bound as aliases to the
    /// scrutinee's sub-expressions. A scrutinee matching no arm emits `unreachable` — a trap,
    /// which reproduces the corpus's "no matching pattern" trap.
    fn gen_match(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() < 2 {
            return decline("match arity");
        }
        let scrutinee = &elems[1];

        // A tuple pattern deconstructs a TUPLE of the SAME arity (core-semantics.md #A Tuple Is
        // Deconstructible By Pattern Matching). When the scrutinee's shape is statically known, a
        // `(tuple …)` arm that cannot match it is a type error (CDZ0201), not a silent non-match
        // falling through to a wildcard (02-binding-and-control.sexp §"a tuple pattern of the
        // wrong arity / against a sum scrutinee is a type error"). Two shape mismatches:
        //   • scrutinee is a tuple but the arm's element count differs — wrong arity;
        //   • scrutinee is a SUM value (a constructor application) — a tuple pattern is the wrong
        //     KIND entirely (a tuple pattern can never match a sum).
        if let Some((scrut_node, scrut_env)) = self.resolve(scrutinee, env) {
            for arm in &elems[2..] {
                if let Node::List(a) = arm {
                    if let Some(pat) = a.first() {
                        // Recursively check the arm's pattern against the scrutinee's static shape —
                        // the arity/kind/type rules COMPOSE (core-semantics.md #Patterns Compose: a
                        // pattern's sub-pattern faces the corresponding sub-value, to any depth), so
                        // a NESTED wrong-arity tuple, a nested wrong-type literal, or a wrong-arity
                        // tuple under a constructor is a type error too — not a runtime non-match
                        // that falls through to a wildcard (02-binding-and-control.sexp §"a nested
                        // tuple pattern of the wrong arity / a nested literal pattern of the wrong
                        // type / a wrong-arity tuple pattern nested under a constructor pattern").
                        self.check_pattern_shape(pat, &scrut_node, &scrut_env)?;
                    }
                }
            }
        }

        // A LITERAL pattern matches the scrutinee by equality, which is defined only WITHIN one
        // type (core-semantics.md #Equality Is Structural — a cross-type comparison is a type
        // error). So a literal pattern whose type differs from the scrutinee's — a `true` (Bool)
        // pattern against an Int64 scrutinee, a `5` (Int64) pattern against a Bool — can never
        // meaningfully match; the arm is a static type mismatch, CDZ0201, not a silent non-match
        // (02-binding-and-control.sexp §"a literal pattern's type must match the scrutinee's").
        if let Some(st) = self.static_type(scrutinee, env) {
            for arm in &elems[2..] {
                if let Node::List(a) = arm {
                    if let Some(pt) = literal_pattern_type(a.first()) {
                        if pt != st {
                            return reject(
                                "CDZ0201",
                                "literal pattern type does not match the scrutinee type",
                            );
                        }
                    }
                }
            }
        }

        // A match is an expression of ONE type, so its arm BODIES must agree — two arms whose
        // statically-known result types differ (`(match 5 (5 1) (_ true))`: Int64 arm `1`, Bool arm
        // `true`) are ill-typed (CDZ0201), the match analogue of the `if`-branch agreement check.
        // This MUST run even when a CONSTANT scrutinee selects one arm: the const-fold path below
        // emits only the selected arm, so without this check the unselected arm's type error is
        // silently dropped and the program runs (core-semantics.md #Conditionals Evaluate One Branch —
        // every arm is type-checked whether or not evaluated; 02-binding-and-control.sexp §"a match
        // whose arm bodies have different types is a type error even when a constant scrutinee selects
        // one"). Compare each arm body's coarse `static_type`; only PROVABLE disagreement rejects (an
        // arm whose type is not statically known imposes nothing — a runtime-scrutinee match's
        // arms-differ-in-kind is handled separately at emit). Pattern binders are not bound here, so a
        // body referencing one yields no static type and is skipped — conservative, never a false
        // reject. `else`/`_`/name-pattern arms and literal-pattern arms alike contribute their body.
        {
            let mut seen: Option<StaticType> = None;
            for arm in &elems[2..] {
                if let Node::List(a) = arm {
                    if a.len() == 2 {
                        if let Some(t) = self.static_type(&a[1], env) {
                            match seen {
                                Some(prev) if prev != t => {
                                    return reject("CDZ0201", "match arm bodies have different types");
                                }
                                None => seen = Some(t),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Every arm's BODY is type-checked whether or not the constant scrutinee selects it — an
        // INTERNALLY ill-typed unselected arm (`(match 5 (5 1) (_ (+ 1 true)))`: the `_` arm mixes
        // Int64 and Bool) is a type error (CDZ0201), exactly as the `if` form rejects a trapping-only-
        // by-type unselected branch `(if true 1 (+ 1 true))` (core-semantics.md #Conditionals Evaluate
        // One Branch — every branch/arm is checked; 02-binding-and-control.sexp §"an internally
        // ill-typed unselected match arm body is a type error"). The const-fold path below emits only
        // the selected arm, so a per-arm-body `check_tree` here is what surfaces the unselected arm's
        // internal error — `check_tree` prunes at `match`, delegating each arm body's checking to this
        // recursion. Distinct from the arm-RESULT-type agreement check above (which compares arms to
        // each other); this checks each body INTERNALLY. Pattern binders are not bound here, so a body
        // referencing one imposes nothing on that name (`check_type_rejections` only rejects a PROVABLE
        // mismatch — both operands' static types known and incompatible), never a false reject.
        for arm in &elems[2..] {
            if let Node::List(a) = arm {
                if a.len() == 2 {
                    self.check_tree(&a[1], env)?;
                }
            }
        }

        // (0) A single-variant match over a constructor scrutinee reduces to a match over its
        // PAYLOAD: `(match (Ctor p) ((Ctor sub0) A) ((Ctor sub1) B))` → `(match p (sub0 A)
        // (sub1 B))`. This lets a nested literal like `(Some 0)` be tested against a RUNTIME
        // payload `n` via the ordinary payload match, rather than only compile-time-resolving
        // a known payload (the fix for nested-literal-vs-runtime-payload).
        if let Some(reduced) = self.reduce_constructor_match(scrutinee, &elems[2..], env) {
            return self.emit(&reduced, env, ctx);
        }
        // Likewise a tuple-pattern match over a tuple whose elements are runtime values
        // desugars to element-wise matching (each element bound, then matched positionally).
        if let Some(reduced) = self.reduce_tuple_match(scrutinee, &elems[2..], env, ctx) {
            return self.emit(&reduced, env, ctx);
        }

        // (1) A COMPILE-TIME-KNOWN scrutinee (an inline literal, constructor, tuple, quoted
        // AST, or a const-foldable expression) is resolved at compile time: only the selected
        // arm's body is emitted, with pattern binders aliased to the scrutinee's sub-nodes.
        if self.match_scrutinee_is_static(scrutinee, env) {
            // Exhaustiveness is a compile-time property of the ARM SET against the scrutinee's
            // type, NOT of which value the scrutinee happens to hold. A match on a SUM value
            // whose arms leave a declared variant uncovered (and have no catch-all) is non-
            // exhaustive → CDZ0210, EVEN when the constant scrutinee is a covered variant.
            // (Without this the check was scrutinee-value-driven: it only fired when the constant
            // scrutinee WAS the missing variant. 02-binding-and-control.sexp §"a sum match missing
            // a variant is non-exhaustive even when the scrutinee is the covered one".)
            if !self.sum_match_exhaustive(scrutinee, &elems[2..], env) {
                return reject("CDZ0210", "match does not cover every variant of the sum");
            }
            // Bool exhaustiveness is likewise a property of the ARM SET vs the TYPE, not of which
            // value the constant scrutinee holds: `(match true (true 1))` covers only `true`, so it
            // is non-exhaustive (CDZ0210) EVEN THOUGH the constant scrutinee `true` matches the sole
            // arm. Without this, the static path returned the matched arm's value and only the
            // MISSING-value case (`(match true (false 0))` — scrutinee matches no arm, caught below)
            // rejected — an asymmetry that mis-accepted a bool match missing the arm the scrutinee
            // happens to take (02-binding-and-control.sexp §"a bool match missing the false arm").
            // A bool has exactly {true,false}; `bool_match_exhaustive` = both literals present or a
            // catch-all. Gated on the scrutinee being a known Bool so a non-bool static scrutinee
            // (an Int literal match) is unaffected.
            if self.match_scrutinee_is_bool(scrutinee, env)
                && !bool_match_exhaustive(&elems[2..])
            {
                return reject("CDZ0210", "match does not cover the scrutinee");
            }
            // An unbounded SCALAR scrutinee — Int64 (2^64 values), Float64, String, Bytes — cannot be
            // covered by any FINITE set of literal arms, so a match over one is exhaustive ONLY with a
            // catch-all (a bare-name binder / `else` / `_`). This is the value-set-vs-arm-set check
            // applied to int literals, the Int64 companion of the bool/sum cases above: `(match 5 (5
            // 1))` must reject CDZ0210 even though the CONSTANT scrutinee `5` hits the sole arm `5` —
            // without it the static path's `try_match` returns the matched arm and the exhaustiveness
            // check is skipped, the same value-driven shortcut the bool path had (02-binding-and-control
            // .sexp §"an int match on a constant scrutinee is non-exhaustive even when the constant hits
            // the sole arm"). Bool is EXCLUDED (its finite {true,false} is handled just above); a Sum is
            // excluded (handled by `sum_match_exhaustive`); Unit has one value so a single arm covers it.
            if matches!(
                self.static_type(scrutinee, env),
                Some(StaticType::Int | StaticType::Float | StaticType::Str | StaticType::Bytes)
            ) && !arms_have_catch_all(&elems[2..])
            {
                return reject("CDZ0210", "match on an unbounded scalar type has no catch-all arm");
            }
            for arm in &elems[2..] {
                let a = match arm {
                    Node::List(a) if a.len() == 2 => a,
                    _ => return decline("malformed match arm"),
                };
                let (pattern, body) = (&a[0], &a[1]);
                if name_of(Some(pattern)) == Some("else") || name_of(Some(pattern)) == Some("_") {
                    return self.emit(body, env, ctx);
                }
                if let Some(mut bound) = self.try_match(pattern, scrutinee, env)? {
                    let mut body_env = env.to_vec();
                    body_env.append(&mut bound);
                    return self.emit(body, &body_env, ctx);
                }
            }
            // No arm matched a statically-known scrutinee — a non-exhaustive match, which a
            // static type system rejects (constitution VII); the variant/literal set is known.
            return reject("CDZ0210", "match does not cover the scrutinee");
        }

        // (2) A RUNTIME scrutinee (a parameter or computed value). Emit a real comparison
        // cascade: evaluate the scrutinee into a local, then for each literal arm test
        // `scrutinee == literal` and branch; a name-binder / `else` / `_` arm is the tail
        // catch-all. This is the fix for the silent miscompile where literal arms against a
        // runtime scrutinee were skipped (recursion base cases never fired).
        self.gen_match_runtime(scrutinee, &elems[2..], env, ctx)
    }

    /// Is a match over a SUM scrutinee exhaustive — does its arm set cover every declared
    /// variant of the scrutinee's sum type (or have a catch-all)? Returns `true` (not our
    /// concern) when the scrutinee is not a sum value, when its variant is not a declared sum
    /// type, or when an arm is a catch-all (`else`/`_`/bare-name binder). Otherwise it compares
    /// the covered variant tags against the sum type's full variant set. Exhaustiveness is a
    /// property of the arm set vs. the TYPE, independent of which variant the scrutinee holds.
    fn sum_match_exhaustive(&self, scrutinee: &Node, arms: &[Node], env: &[Local]) -> bool {
        // Resolve the scrutinee to a constructor application to learn its sum TYPE and payload.
        let (sctor, spayload, senv) = match self.resolve(scrutinee, env) {
            Some((Node::List(items), e)) => match constructor_of(items.first()) {
                Some(c) => (c, items.get(1).cloned(), e),
                None => return true, // not a sum value → not a sum-exhaustiveness question
            },
            _ => return true,
        };
        let type_name = match self.sum_types.get(variant_tag(&sctor)) {
            Some(t) => t.clone(),
            None => return true, // undeclared variant — no known variant set to check against
        };
        // The full declared variant set for this sum type. Restrict to CONSTRUCTOR-named entries
        // (capitalized): a `(type Result (Ok a | Err e))` declaration's lowercase `a`/`e` are
        // TYPE PARAMETERS, not variants — `collect_sum_types` records them from the flat
        // declaration body, but they are not values to cover.
        let all: std::collections::BTreeSet<&str> = self
            .sum_types
            .iter()
            .filter(|(v, t)| **t == type_name && is_constructor_name(v))
            .map(|(v, _)| v.as_str())
            .collect();
        // The variants the arms cover; a catch-all arm makes the match exhaustive outright.
        let mut covered: std::collections::BTreeSet<String> = Default::default();
        for arm in arms {
            let pattern = match arm {
                Node::List(a) if a.len() == 2 => &a[0],
                _ => continue,
            };
            match pattern {
                // A bare-name binder / `else` / `_` matches any value → catch-all → exhaustive.
                Node::Name(_) => return true,
                _ => {
                    // A constructor pattern `(Ctor sub)` or a qualified `((. Ty Ctor) sub)`.
                    let head = match pattern {
                        Node::List(p) => p.first(),
                        other => Some(other),
                    };
                    if let Some(c) = constructor_of(head) {
                        covered.insert(variant_tag(&c).to_string());
                    }
                }
            }
        }
        if !all.iter().all(|v| covered.contains(variant_tag(v))) {
            return false;
        }
        // Exhaustiveness COMPOSES into nested patterns (core-semantics.md #Patterns Compose with
        // #Matching Is Exhaustive Or Rejected): a value of `Option (Option Int64)` ranges over `(Some
        // (Some _))`, `(Some (None _))`, `(None _)`, so arming `(Some (Some x))` + `(None _)` leaves
        // `(Some (None _))` uncovered — non-exhaustive even though the OUTER set `{Some,None}` is
        // covered. Descend into the nested constructor position: for the variant the scrutinee HOLDS,
        // its payload's own variant set must be covered by the sub-patterns of the arms matching that
        // variant. The sub-scrutinee is the scrutinee's payload (a compile-time-known value on this
        // static path), so its inner type is recoverable; recurse with the sub-patterns as arms. Only
        // the held variant's payload is available to type here (a value-driven inference), which
        // suffices for the constant-scrutinee path where the uncovered nested value lives under the
        // held variant. A sub-pattern that is a bare binder (`(Some x)`) is a nested catch-all — the
        // recursion returns true. A non-constructor sub-pattern (a tuple `(tuple h t)`, a literal)
        // makes the payload resolve to a non-sum → the recursion returns true (not a sum question).
        if let Some(payload) = spayload {
            let held = variant_tag(&sctor);
            let mut sub_arms: Vec<Node> = Vec::new();
            for arm in arms {
                let (pattern, body) = match arm {
                    Node::List(a) if a.len() == 2 => (&a[0], &a[1]),
                    _ => continue,
                };
                // Only arms matching the HELD variant contribute a sub-pattern over its payload.
                if let Node::List(p) = pattern {
                    if let Some(c) = constructor_of(p.first()) {
                        if variant_tag(&c) == held {
                            if let Some(sub) = p.get(1) {
                                // Reuse the arm body verbatim — only the sub-PATTERN drives the
                                // recursive coverage check; the body is never inspected.
                                sub_arms.push(Node::List(vec![sub.clone(), body.clone()]));
                            }
                        }
                    }
                }
            }
            if !sub_arms.is_empty() && !self.sum_match_exhaustive(&payload, &sub_arms, &senv) {
                return false;
            }
        }
        true
    }

    /// Reduce a match over a constructor scrutinee whose payload is a RUNTIME value to a match
    /// over that payload, when every arm matches the same constructor. Returns the rewritten
    /// `(match payload arm…)` node, or None if the reduction does not apply (payload is
    /// compile-time-known — the static path handles it — or the arms are not uniform).
    fn reduce_constructor_match(
        &self,
        scrutinee: &Node,
        arms: &[Node],
        env: &[Local],
    ) -> Option<Node> {
        // The scrutinee must resolve to a constructor application `(Ctor payload)`.
        let (sval, _senv) = self.resolve(scrutinee, env)?;
        let sitems = match &sval {
            Node::List(items) => items,
            _ => return None,
        };
        let sctor = constructor_of(sitems.first())?;
        let payload = sitems.get(1)?;
        // Only reduce when the payload is a RUNTIME value; a compile-time-known payload is
        // handled correctly by the static resolution path.
        if self.match_scrutinee_is_static(payload, env) {
            return None;
        }
        // Every arm must be `(Ctor sub)` with the same constructor; rewrite to `(sub body)`.
        let mut new_arms = Vec::new();
        for arm in arms {
            let a = match arm {
                Node::List(a) if a.len() == 2 => a,
                _ => return None,
            };
            let pitems = match &a[0] {
                Node::List(p) if p.len() == 2 => p,
                _ => return None, // an `else`/bare pattern — don't reduce (keep whole-value match)
            };
            if constructor_of(pitems.first())? != sctor {
                return None; // mixed constructors — not a single-variant match
            }
            new_arms.push(Node::List(vec![pitems[1].clone(), a[1].clone()]));
        }
        let mut out = vec![Node::Name("match".into()), payload.clone()];
        out.extend(new_arms);
        Some(Node::List(out))
    }

    /// Reduce a match over a tuple scrutinee with RUNTIME elements to nested single-scrutinee
    /// matches — the desugar that lets the archetypal constant-fold pass compile: `(match (tuple
    /// (fold a) (fold b)) ((tuple (E.Lit x) (E.Lit y)) …) ((tuple fa fb) …))` (a recursive fold
    /// matching the TUPLE of its two recursive results with CONSTRUCTOR patterns in the tuple
    /// positions, 20-structural-editing.sexp §"a bottom-up fold matches a tuple of its recursive
    /// results with constructor patterns"). The tuple's elements cannot be resolved at the pattern
    /// site (a recursive self-call does not bottom out at compile time), so the static `try_match`
    /// path declines; instead bind each element ONCE to a fresh compiler-internal name, then compile
    /// the arm MATRIX into nested single-scrutinee matches with fall-through — the SAME form the spec
    /// records as already compiling (the hand-written `is-lit`/bind-then-re-match workaround). The
    /// elements are bound once (not re-evaluated per arm), so a later arm's whole-element binder
    /// (`fa`/`fb`) sees the same value the earlier arm matched.
    ///
    /// Every arm must be a `(tuple p0 … pn)` of the scrutinee's arity whose column patterns are
    /// binder / `_` / constructor `(Ctor …)` / scalar-literal — the shapes an optimizer's rewrite
    /// arms use. A nested `(tuple …)`/`(record …)` column pattern declines (returns None → the static
    /// path or an honest decline handles it); a non-tuple arm pattern declines. Returns None if the
    /// reduction does not apply (all-static elements → the static const-fold path; a shape beyond it).
    fn reduce_tuple_match(
        &self,
        scrutinee: &Node,
        arms: &[Node],
        env: &[Local],
        ctx: &FnCtx,
    ) -> Option<Node> {
        let (sval, _) = self.resolve(scrutinee, env)?;
        let sitems = match &sval {
            Node::List(items) if name_of(items.first()) == Some("tuple") => items,
            _ => return None,
        };
        let elems: Vec<Node> = sitems[1..].to_vec();
        // Only reduce when at least one element is a runtime value (else the static path folds).
        if elems.iter().all(|e| self.match_scrutinee_is_static(e, env)) {
            return None;
        }
        let arity = elems.len();
        // Fresh compiler-internal element names (`@mN_i`), seeded from the current local counter so
        // nested reductions do not collide; even a repeat is safe (a `let` shadows lexically, and a
        // user identifier can never contain `@`). Bound ONCE to the elements, wrapping the compiled
        // matrix so every column reference re-reads the same value.
        let base = ctx.next_local;
        let names: Vec<Node> = (0..arity)
            .map(|i| Node::Name(format!("@m{base}_{i}")))
            .collect();
        // Parse every arm into its column patterns + body. A `(tuple …)` arm of the right arity
        // contributes its columns; a bare CATCH-ALL arm (`_`/`else`/a name binding the whole tuple)
        // becomes an all-`_` row (a name catch-all rebinds the whole tuple `(tuple @m…)` in its body,
        // reconstructed from the element names). A column pattern that is a nested tuple/record is
        // beyond this first landing → None (fall through). A catch-all TERMINATES the matrix (later
        // rows are unreachable), the exhaustive form an optimizer's rewrite arms take.
        let mut rows: Vec<(Vec<Node>, Node)> = Vec::new();
        for arm in arms {
            let a = match arm {
                Node::List(a) if a.len() == 2 => a,
                _ => return None,
            };
            match &a[0] {
                // A `(tuple p0 … pn)` arm of the scrutinee arity.
                Node::List(p) if name_of(p.first()) == Some("tuple") && p.len() == arity + 1 => {
                    let cols: Vec<Node> = p[1..].to_vec();
                    for c in &cols {
                        match c {
                            Node::Name(_) | Node::Int(_) | Node::Bool(_) | Node::Str(_) => {}
                            _ if is_constructor_pattern(c) => {}
                            _ => return None, // nested tuple/record column — not this reduce
                        }
                    }
                    rows.push((cols, a[1].clone()));
                }
                // A bare catch-all: `_`/`else` discards; a name binds the whole tuple.
                Node::Name(n) => {
                    let all_wild: Vec<Node> = (0..arity).map(|_| Node::Name("_".into())).collect();
                    let body = if n == "_" || n == "else" {
                        a[1].clone()
                    } else {
                        // Rebind the catch-all name to the reconstructed tuple `(tuple @m…)`, so the
                        // body's use of it sees the same value.
                        let mut tup = vec![Node::Name("tuple".into())];
                        tup.extend(names.iter().cloned());
                        Node::List(vec![
                            Node::Name("let".into()),
                            Node::List(vec![Node::List(vec![Node::Name(n.clone()), Node::List(tup)])]),
                            a[1].clone(),
                        ])
                    };
                    rows.push((all_wild, body));
                    break; // a catch-all makes the match exhaustive; later arms are unreachable
                }
                _ => return None,
            }
        }
        let matched = self.compile_tuple_matrix(&names, &rows)?;
        let binds: Vec<Node> = names
            .iter()
            .zip(elems.iter())
            .map(|(n, e)| Node::List(vec![n.clone(), e.clone()]))
            .collect();
        Some(Node::List(vec![
            Node::Name("let".into()),
            Node::List(binds),
            matched,
        ]))
    }

    /// Compile a tuple-match arm MATRIX (rows of column patterns over the fresh element names
    /// `cols`) into nested single-scrutinee matches with fall-through. Standard backtracking
    /// translation: take the first row; if all its columns are irrefutable (binder/`_`), bind each
    /// binder to its column and emit the body (later rows are unreachable). Otherwise pick the
    /// leftmost REFUTABLE column `j` and emit `(match cols[j] (pat_j <success>) (else <fail>))`,
    /// where `<fail>` compiles the remaining rows and `<success>` compiles the SAME first row with
    /// column `j` consumed (replaced by `_`) — so its other columns are still checked and, on
    /// failure, fall through to the same remaining rows. Each recursive call strictly reduces the
    /// count of refutable cells, so it terminates. The fall-through is duplicated across a row's
    /// refutable columns (no join points), which is exactly the "separate nested single-scrutinee
    /// matches" form the spec records as compiling. Returns None if a row is malformed.
    fn compile_tuple_matrix(&self, cols: &[Node], rows: &[(Vec<Node>, Node)]) -> Option<Node> {
        let (pats, body) = rows.first()?;
        // Leftmost refutable column: a constructor pattern or a scalar literal. A binder / `_` is
        // irrefutable (always matches).
        let refutable = |p: &Node| {
            matches!(p, Node::Int(_) | Node::Bool(_) | Node::Str(_)) || is_constructor_pattern(p)
        };
        match pats.iter().position(refutable) {
            // No refutable column → this row always matches: bind its binders and emit the body.
            None => {
                let mut out = body.clone();
                for (k, p) in pats.iter().enumerate().rev() {
                    if let Node::Name(n) = p {
                        if n != "_" {
                            out = Node::List(vec![
                                Node::Name("let".into()),
                                Node::List(vec![Node::List(vec![
                                    Node::Name(n.clone()),
                                    cols[k].clone(),
                                ])]),
                                out,
                            ]);
                        }
                    }
                }
                Some(out)
            }
            Some(j) => {
                // Fall-through: the remaining rows (empty → a non-exhaustive runtime match, which the
                // single-scrutinee emitter turns into `unreachable`; a well-typed exhaustive source
                // always ends in a catch-all row so this stays reachable only when it should).
                let fail = self.compile_tuple_matrix(cols, &rows[1..])?;
                // Success: the SAME first row with column j consumed (its binders bound by the match
                // arm below), so the row's OTHER columns are still checked; on their failure, fall to
                // the same remaining rows.
                let mut resid = pats.clone();
                resid[j] = Node::Name("_".into());
                let mut success_rows: Vec<(Vec<Node>, Node)> = vec![(resid, body.clone())];
                success_rows.extend_from_slice(&rows[1..]);
                let success = self.compile_tuple_matrix(cols, &success_rows)?;
                // `(match cols[j] (pat_j success) (else fail))` — pat_j's own binders scope over
                // success; a constructor pat_j's nested payload binders are handled by the runtime
                // sum-match emitter (which threads the `else` through nested payload dispatch too).
                Some(Node::List(vec![
                    Node::Name("match".into()),
                    cols[j].clone(),
                    Node::List(vec![pats[j].clone(), success]),
                    Node::List(vec![Node::Name("else".into()), fail]),
                ]))
            }
        }
    }

    /// Is the match scrutinee compile-time resolvable (so the arm can be chosen statically)?
    /// True only when it is a compile-time constant (`eval_const` yields a value) or resolves
    /// to a STRUCTURAL constructor/tuple/record/quote form — NOT for an arbitrary computed
    /// expression like `(% n 2)`, which resolves to itself as a list but is a runtime value.
    fn match_scrutinee_is_static(&self, scrutinee: &Node, env: &[Local]) -> bool {
        if matches!(self.eval_const(scrutinee, env), Ok(Some(_)) | Err(_)) {
            return true;
        }
        match self.resolve(scrutinee, env) {
            Some((Node::Str(_), _)) => true,
            Some((Node::List(items), _)) => {
                // A structural head (constructor / tuple / record / Ast form), not an operator.
                let h = items.first();
                constructor_of(h).is_some()
                    || matches!(name_of(h), Some("tuple") | Some("record") | Some("list"))
            }
            _ => false,
        }
    }

    /// Is the match scrutinee a Bool? A Bool match has exactly two values, so its arm set must
    /// cover both `true` and `false` (or have a catch-all) to be exhaustive — a check the static
    /// path applies alongside `sum_match_exhaustive`. Recognized from a const-foldable Bool value
    /// or a scrutinee that resolves to a boolean literal.
    fn match_scrutinee_is_bool(&self, scrutinee: &Node, env: &[Local]) -> bool {
        if matches!(self.eval_const(scrutinee, env), Ok(Some(CVal::Bool(_)))) {
            return true;
        }
        matches!(self.resolve(scrutinee, env), Some((Node::Bool(_), _)))
    }

    /// Emit a runtime comparison cascade for a match on a scalar runtime scrutinee. Only
    /// literal (Int/Bool) patterns and a final binder/`else`/`_` catch-all are supported;
    /// anything else declines (never miscompiles).
    fn gen_match_runtime(
        &self,
        scrutinee: &Node,
        arms: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let (scrut_code, scrut_kind) = self.emit(scrutinee, env, ctx)?;
        // A HEAP scrutinee whose value is a runtime SUM: dispatch on the runtime discriminant
        // (`sum-disc`), binding each arm's payload from the heap (`sum-payload` + `arr-get`).
        // This is the CONSUMPTION half of the recursive-sum idiom — a function matching a heap
        // value it did not construct at compile time, e.g. a recursively-built linked list / AST
        // node whose variant is only known at run time. It mirrors the renderer's `Shape::Sum`
        // heap-walk (which reads the same accessors to render), the inverse direction.
        if scrut_kind == Kind::Heap {
            // A HEAP scrutinee whose arms are `(tuple …)` patterns is a runtime TUPLE match, NOT a
            // sum match — a tuple is a heap array, its elements read by `arr-get`, no discriminant.
            // This arises when the scrutinee is a runtime tuple whose shape is not statically
            // resolvable — most importantly a TAIL-RECURSIVE tuple-returning function's result (`(go
            // 3)` where `go` returns `(tuple …)` in every branch): its `shape_of` hits the recursion
            // guard (→ None) so `reduce_tuple_match` above could not fire, and without this it fell to
            // `gen_match_runtime_sum` which declines "without a constructor arm". This is the
            // consumption side a recursive-descent PARSER needs — matching a `(node, cursor)` tuple
            // threaded through mutual recursion (ask-73). A tuple match is irrefutable, so the first
            // `(tuple …)` arm handles the value (a later arm is dead); a leading `_`/`else` catch-all
            // is deferred to the sum path (which handles bare-name/catch-all arms).
            if arms.iter().any(arm_is_tuple_pattern) && !arms.iter().any(arm_is_catch_all) {
                return self.gen_match_runtime_tuple(scrutinee, &scrut_code, arms, env, ctx);
            }
            return self.gen_match_runtime_sum(scrutinee, &scrut_code, arms, env, ctx);
        }
        if !matches!(scrut_kind, Kind::Int64 | Kind::Bool) {
            return decline("runtime match on a non-scalar scrutinee");
        }
        // A Bool has exactly two values, so a match over a runtime Bool is exhaustive ONLY if
        // it has a catch-all (a bare-name/`else`/`_` arm) or names BOTH `true` and `false`.
        // Reject a non-exhaustive one up front (CDZ0210) — otherwise the "last bool arm is the
        // unconditional else" shortcut below would treat a lone `(true …)` arm as total and
        // wrongly yield its value for the uncovered input. (An Int match's exhaustiveness is
        // checked structurally: it runs out of arms with no catch-all → CDZ0210 in gen_match_arms.)
        if scrut_kind == Kind::Bool && !bool_match_exhaustive(arms) {
            return reject("CDZ0210", "match does not cover the scrutinee");
        }
        // Store the scrutinee in a local so each arm can re-read it.
        let slot = ctx.alloc_local(scrut_kind);
        let mut prelude = scrut_code;
        prelude.push(op::LOCAL_SET);
        uleb128(slot as u64, &mut prelude);

        // Build the arms into (test?, body-node, binder-name?) and emit a nested if/else.
        let body = self.gen_match_arms(slot, scrut_kind, arms, env, ctx)?;
        let mut c = prelude;
        c.extend_from_slice(&body.0);
        Ok((c, body.1))
    }

    /// Emit a runtime TUPLE match: the scrutinee is a heap tuple (a flat `arr` of its elements), and
    /// the arm is a `(tuple b0 … bn)` pattern. A tuple match is IRREFUTABLE (a tuple has one shape),
    /// so the FIRST `(tuple …)` arm binds the value and its body is emitted; a later arm is dead. The
    /// element slots are read by `arr-get(handle, i)` and bound via `bind_tuple_elems`, exactly as a
    /// sum's tuple PAYLOAD is — the same machinery, minus the discriminant dispatch. Slot kinds come
    /// from the scrutinee's inferred tuple `Shape` when available; when the shape is not statically
    /// resolvable (the ask-73 case — a tail-recursive tuple-returning function whose `shape_of` hits
    /// the recursion guard), a scalar slot bound and USED as a scalar recovers its kind through the
    /// arm-unification override, else stays an opaque `Heap` handle (a nested compound, kept as-is).
    fn gen_match_runtime_tuple(
        &self,
        scrutinee: &Node,
        scrut_code: &[u8],
        arms: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime tuple match needs the value-heap runtime");
        }
        // The first `(tuple …)` arm handles the value (irrefutable).
        let (pattern, body) = arms
            .iter()
            .find_map(|arm| match arm {
                Node::List(a) if a.len() == 2 && arm_is_tuple_pattern(arm) => Some((&a[0], &a[1])),
                _ => None,
            })
            .ok_or_else(|| Decline("runtime tuple match without a tuple arm".into(), None))?;
        let binders = match pattern {
            Node::List(p) => &p[1..],
            _ => return decline("runtime tuple match pattern is not a tuple"),
        };
        // Materialize the tuple handle into a local (the flat element array).
        let handle = ctx.alloc_local(Kind::Heap);
        let mut c = scrut_code.to_vec();
        c.push(op::LOCAL_SET);
        uleb128(handle as u64, &mut c);
        // Per-slot TYPE nodes: `bind_tuple_elems` uses these to unbox a scalar slot and to recurse a
        // nested `(Tuple …)` slot. The scrutinee's shape is often not statically resolvable here (the
        // tail-recursive-tuple-return case), so leave the declared types EMPTY (all-Heap default) and
        // let the arm-usage override below recover each scalar slot's kind. (A future refinement could
        // derive slot types from `shape_of` when available; the override covers the corpus cases.)
        let slot_types: Vec<Node> = Vec::new();
        // A per-slot KIND override recovered by unifying the arm-body uses of each binder — the same
        // recovery the sum path uses for a polymorphic payload, here for a tuple whose element kinds
        // are not statically known (the tail-recursive-tuple-return case). A slot used as a scalar
        // resolves to that scalar; else it stays Heap.
        let mut override_kinds = self.infer_tuple_binder_kinds(binders, body, env);
        // Where arm-body usage did not pin a slot (a binder merely RETURNED bare — `((tuple a b) a)`
        // — pins nothing), recover it from the SCRUTINEE's tuple element kinds: `(go 3 0)` where
        // `go`'s base branch is `(tuple acc 0)` gives both slots Int64. This is what makes a
        // tail-recursive tuple-returning function whose result is destructured to a SCALAR
        // (`(match (go 3 0) ((tuple a b) a))`) infer `a : Int64` — so `main`'s return is a scalar,
        // not an opaque Heap handle (ask-73 accumulator half). The two sources compose: arm-usage
        // wins where present (it saw the concrete use), the scrutinee fills the rest.
        if let Some(scrut_kinds) = self.scrutinee_tuple_slot_kinds(scrutinee, env, binders.len()) {
            let base = override_kinds
                .take()
                .unwrap_or_else(|| vec![Kind::Heap; binders.len()]);
            let merged: Vec<Kind> = base
                .into_iter()
                .enumerate()
                .map(|(i, k)| {
                    if matches!(k, Kind::Int64 | Kind::Bool | Kind::Float64) {
                        k // arm-usage already pinned this slot
                    } else {
                        scrut_kinds.get(i).copied().flatten().unwrap_or(Kind::Heap)
                    }
                })
                .collect();
            override_kinds = Some(merged);
        }
        let mut body_env = env.to_vec();
        self.bind_tuple_elems(handle, binders, &slot_types, override_kinds.as_ref(), &mut c, &mut body_env, ctx)?;
        let (bc, bk) = self.emit(body, &body_env, ctx)?;
        c.extend_from_slice(&bc);
        Ok((c, bk))
    }

    /// Recover a runtime tuple match's per-slot scalar KINDS by inferring the arm body under the
    /// binders as inference variables (the tuple analogue of `infer_sum_payload_override`): a binder
    /// USED as an Int64/Bool/Float64 in the body resolves to that kind, so `bind_tuple_elems` unboxes
    /// it. A binder whose kind stays unknown (a nested compound, or unused) is left for the declared
    /// slot type / `Heap` default. `None` if nothing concrete is recovered.
    fn infer_tuple_binder_kinds(
        &self,
        binders: &[Node],
        body: &Node,
        env: &[Local],
    ) -> Option<Vec<Kind>> {
        // Seed the enclosing scalar locals (so a binder compared to a known-kind local infers), then
        // each tuple slot's bare-name binder as a fresh variable.
        let mut vars: Vec<(String, Option<Kind>)> = env
            .iter()
            .filter(|l| l.alias.is_none())
            .map(|l| (l.name.clone(), Some(l.kind)))
            .collect();
        let slots: Vec<(usize, String)> = binders
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b {
                Node::Name(n) if n != "_" => {
                    vars.push((n.clone(), None));
                    Some((i, n.clone()))
                }
                _ => None,
            })
            .collect();
        if slots.is_empty() {
            return None;
        }
        let mut ictx = InferCtx { compiler: self, vars };
        let _ = ictx.infer(body);
        let mut kinds = vec![Kind::Heap; binders.len()];
        let mut any = false;
        for (i, name) in &slots {
            if let Some((_, Some(k))) = ictx.vars.iter().find(|(n, _)| n == name) {
                if matches!(k, Kind::Int64 | Kind::Bool | Kind::Float64) {
                    kinds[*i] = *k;
                    any = true;
                }
            }
        }
        if any { Some(kinds) } else { None }
    }

    /// Recover a runtime tuple match's per-slot scalar KINDS from the SCRUTINEE's tuple element
    /// kinds — the piece `infer_tuple_binder_kinds` (arm-body usage) cannot supply when a binder is
    /// merely returned bare (`((tuple a b) a)` → `a`'s only use pins nothing). Navigate the
    /// scrutinee to a representative `(tuple …)` form: follow `if`/`match`/`let`/`do`/`:` to the
    /// result, INLINE a user call (binding its params to the argument nodes as aliases), and SKIP a
    /// recursive self-call branch — its result kind equals a base branch's by induction, exactly the
    /// tuple twin of the tail-recursive SCALAR accumulator return-kind inference (GAP: the recursive
    /// branch `(go (- n 1) …)` supplies no shape; the base branch `(tuple acc 0)` supplies both). At
    /// each concrete `(tuple e0 …)` reached, element kinds come from `shape_of_guarded` (a compound
    /// element → not a scalar kind → left `None` → the Heap default). Per-slot kinds merge across
    /// branches (a genuine disagreement → `None` for that slot). Returns per-slot kinds (`None` slot
    /// = unknown), or `None` if no `(tuple …)` of the right arity was reached at all.
    fn scrutinee_tuple_slot_kinds(
        &self,
        scrutinee: &Node,
        env: &[Local],
        arity: usize,
    ) -> Option<Vec<Option<Kind>>> {
        let mut acc: Option<Vec<Option<Kind>>> = None;
        self.walk_scrutinee_tuples(scrutinee, env, arity, &mut Vec::new(), &mut acc);
        acc
    }

    fn walk_scrutinee_tuples(
        &self,
        node: &Node,
        env: &[Local],
        arity: usize,
        stack: &mut Vec<String>,
        acc: &mut Option<Vec<Option<Kind>>>,
    ) {
        match node {
            // An alias (a `let`/param binding) to a tuple-producing expression: resolve and walk it.
            Node::Name(n) => {
                if let Some(l) = env.iter().rev().find(|l| l.name == *n) {
                    if let Some((anode, aenv)) = &l.alias {
                        let anode = anode.clone();
                        let aenv = aenv.clone();
                        self.walk_scrutinee_tuples(&anode, &aenv, arity, stack, acc);
                    }
                }
            }
            Node::List(elems) => match name_of(elems.first()) {
                Some("tuple") if elems.len() == arity + 1 => {
                    let this: Vec<Option<Kind>> = elems[1..]
                        .iter()
                        .map(|e| self.tuple_slot_scalar_kind(e, env, stack))
                        .collect();
                    merge_slot_kinds(acc, this);
                }
                Some("if") if elems.len() == 4 => {
                    self.walk_scrutinee_tuples(&elems[2], env, arity, stack, acc);
                    self.walk_scrutinee_tuples(&elems[3], env, arity, stack, acc);
                }
                Some("do") if elems.len() >= 2 => {
                    self.walk_scrutinee_tuples(elems.last().unwrap(), env, arity, stack, acc);
                }
                Some(":") if elems.len() >= 2 => {
                    self.walk_scrutinee_tuples(&elems[1], env, arity, stack, acc);
                }
                Some("let") if elems.len() >= 3 => {
                    if let Some(Node::List(binds)) = elems.get(1) {
                        let mut inner = env.to_vec();
                        let mut ok = true;
                        for pair in binds {
                            match pair {
                                Node::List(p) => match (p.first(), p.get(1)) {
                                    (Some(Node::Name(name)), Some(v)) => inner.push(
                                        Local::aliased(name.clone(), v.clone(), inner.clone()),
                                    ),
                                    _ => {
                                        ok = false;
                                        break;
                                    }
                                },
                                _ => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            self.walk_scrutinee_tuples(
                                elems.last().unwrap(),
                                &inner,
                                arity,
                                stack,
                                acc,
                            );
                        }
                    }
                }
                Some("match") if elems.len() >= 3 => {
                    let scrut = &elems[1];
                    for arm in &elems[2..] {
                        if let Node::List(a) = arm {
                            if a.len() == 2 {
                                let arm_env = match self.try_match(&a[0], scrut, env) {
                                    Ok(Some(binds)) => {
                                        let mut e = env.to_vec();
                                        e.extend(binds);
                                        e
                                    }
                                    _ => env.to_vec(),
                                };
                                self.walk_scrutinee_tuples(&a[1], &arm_env, arity, stack, acc);
                            }
                        }
                    }
                }
                // A user-function call: inline it (bind params to args as aliases). A call ALREADY
                // being inlined is the recursive self-call — SKIP it (its tuple result kind equals a
                // base branch's), never inline forever.
                Some(h) if !is_special_form_head(elems.first()) => {
                    if stack.iter().any(|n| n == h) {
                        return;
                    }
                    if let Some(f) = self.lookup_fn(h) {
                        let args = &elems[1..];
                        if args.len() == f.params.len() {
                            let mut inner = env.to_vec();
                            for (p, a) in f.params.iter().zip(args) {
                                inner.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
                            }
                            let body = f.body.clone();
                            stack.push(h.to_string());
                            self.walk_scrutinee_tuples(&body, &inner, arity, stack, acc);
                            stack.pop();
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// The scalar `Kind` a tuple-slot expression `e` unboxes to (Int64/Bool/Float64), or `None` if it
    /// is a compound (an opaque Heap handle stays Heap). Tries `shape_of` FIRST (which resolves a
    /// structural leaf — an inline `(Ast.Int …)` slot is a Sum → None, a `5` is Int); then FALLS BACK
    /// to inferring the expression's `Kind`. The fallback is what handles a RECURSIVE scalar producer
    /// in a tuple slot — `decode-node`'s cursor slot `(skip-item b i)` is a mutually-recursive Int64
    /// function whose `shape_of` hits the recursion guard (→ None) but whose return KIND is solidly
    /// Int64 (the fixpoint solved it): a fresh `InferCtx` over the enclosing scalar locals reads back
    /// the callee's `ret_kind`. This is the ask-77 piece — a slot kind knowable by Kind even when its
    /// Shape is infinite. Only a concrete scalar counts; Heap/Never/Unit → None (kept opaque).
    fn tuple_slot_scalar_kind(&self, e: &Node, env: &[Local], stack: &[String]) -> Option<Kind> {
        if let Some(k) = self.shape_of_guarded(e, env, &mut stack.to_vec()).and_then(shape_scalar_kind) {
            return Some(k);
        }
        // Kind fallback: infer `e`'s kind with the enclosing scalar locals seeded (a slot expression
        // like `(skip-item b i)` references params bound in `env`). A recursive call resolves to the
        // callee's solved `ret_kind`, so a recursive Int64 producer is recovered though its Shape is not.
        let vars: Vec<(String, Option<Kind>)> = env
            .iter()
            .filter(|l| l.alias.is_none())
            .map(|l| (l.name.clone(), Some(l.kind)))
            .collect();
        let mut ictx = InferCtx { compiler: self, vars };
        match ictx.infer(e) {
            Some(k @ (Kind::Int64 | Kind::Bool | Kind::Float64)) => Some(k),
            _ => None,
        }
    }

    /// Emit a runtime SUM match: dispatch on the heap value's discriminant and bind each arm's
    /// payload from the heap. `scrut_code` leaves the sum handle on the stack. The arms are
    /// constructor patterns `(Ctor binder)` / `((. Ty Ctor) binder)` over ONE sum type; the
    /// discriminant is the variant's index in its declared order (matching `gen_runtime_sum`).
    ///
    /// For each variant i, emit `if sum-disc(handle) == i { bind payload ; body } else { … }`. A
    /// binder that is a `(tuple b0 … bn)` pattern reads each slot from `arr-get(payload, k)` and
    /// unboxes per the variant's recorded payload kinds (`sum_payload_kinds`); a bare-name binder
    /// binds the whole payload handle (Kind::Heap); `_` binds nothing. A well-typed exhaustive
    /// match always selects one arm, so the innermost `else` is `unreachable`.
    fn gen_match_runtime_sum(
        &self,
        scrutinee: &Node,
        scrut_code: &[u8],
        arms: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // Consuming a runtime sum reads it through the value-heap accessors (`sum-disc`,
        // `sum-payload`, `arr-get`). On the SCALAR path (`call_base == 0`) the module imports
        // none of those, so decline with a HEAP reason — `compile_module` then dead-stubs an
        // unreachable consumer or retries in runtime mode where the imports exist. (A Heap-kinded
        // scrutinee can only arise once we are already on a heap-bearing path, but a helper
        // matching a runtime sum whose result folds to a scalar reaches here on the scalar pass;
        // without this guard it emitted `sum-disc` into an import-free module — the "fn returns a
        // heap sub-node from a match arm" miscompile.)
        if self.call_base == 0 {
            return decline("runtime sum match needs the value-heap runtime");
        }
        // Store the handle in a local so each arm re-reads it (for disc and payload).
        let handle = ctx.alloc_local(Kind::Heap);
        let mut prelude = scrut_code.to_vec();
        prelude.push(op::LOCAL_SET);
        uleb128(handle as u64, &mut prelude);

        // Learn the sum TYPE and its declared variant order from the FIRST constructor arm, so the
        // discriminant a runtime `sum-disc` returns indexes the right variant. Every arm must be a
        // constructor of the SAME type (a well-typed match); exhaustiveness was checked upstream
        // for a static scrutinee, but a runtime scrutinee's arms are validated here structurally.
        let first_ctor = arms
            .iter()
            .find_map(|arm| match arm {
                Node::List(a) if a.len() == 2 => constructor_of(a.first().and_then(|p| match p {
                    Node::List(items) => items.first(),
                    other => Some(other),
                })),
                _ => None,
            });
        let ctor = match first_ctor {
            Some(c) => c,
            None => {
                // A `(list …)` element pattern over a RUNTIME list scrutinee (a recursive fold over
                // a parameter list). The static/const-fold path handles an inline-list scrutinee
                // (core-semantics.md §A List Is Deconstructed By Element Patterns With An Optional
                // Rest), but binding a runtime `rest` needs a materialized list tail — a runtime
                // list-tail primitive not yet emitted (ask-13's rest-binder lowering; deferred to
                // the runtime work). Decline honestly rather than mis-reporting "no constructor arm".
                if arms.iter().any(|arm| matches!(arm, Node::List(a)
                    if a.len() == 2 && name_of(match a.first() { Some(Node::List(p)) => p.first(), _ => None }) == Some("list")))
                {
                    return decline("runtime list element-pattern (rest binder) needs a list-tail primitive");
                }
                return decline("runtime sum match without a constructor arm");
            }
        };
        let type_name = match self.sum_types.get(variant_tag(&ctor)) {
            Some(t) => t.clone(),
            None => return decline("runtime sum match on an undeclared variant"),
        };
        let order = match self.sum_variants.get(&type_name) {
            Some(o) => o.clone(),
            None => return decline("sum type has no recorded variant order"),
        };

        // A per-variant payload-KIND override derived from the SCRUTINEE's static shape. The stored
        // `sum_payload_kinds` are keyed by the DECLARED type: a polymorphic variant (Option's
        // `Some a`) records its payload as an opaque `Heap` (the type parameter `a`), so a bare
        // binder `x` on `(Some x)` would bind as `Heap`. But when the scrutinee is a concretely-typed
        // producer — `(Bytes.at b i)` yields `Option<Int64>` (a boxed BYTE), `shape_of` knows the
        // `Some` payload is `Int`, not an opaque handle — the binder must unbox to that Int64 so it
        // unifies with a scalar `None` arm (Tier 2c: `(match (Bytes.at b i) ((Some x) x) (None -1))`
        // failed because `x:Heap` did not unify with `None`'s Int64). Compute the concrete slot kinds
        // per variant from the scrutinee shape; `bind_sum_payload` prefers them over the declared
        // (opaque) kinds. Empty map (shape not inferable) → the declared kinds, unchanged behavior.
        let mut payload_override = self
            .shape_of(scrutinee, env)
            .map(|s| shape_variant_payload_kinds(&s))
            .unwrap_or_default();

        // When the scrutinee's shape yields no concrete payload kind — the scrutinee is an opaque
        // `Heap` PARAMETER, so its payload kind was erased at the function boundary (e.g. `(def
        // (unwrap o d) (match o ((Some x) x) ((None _) d)))` — `o : Option` arrives as a bare handle)
        // — recover each single-payload variant's binder kind by UNIFYING the arm result kinds.
        // A binder returned directly (`((Some x) x)`) makes the `Some` arm's result kind the payload
        // kind; unified against a sibling scalar arm (`((None _) d)`, `d : Int64`) it resolves to
        // that scalar, so `bind_sum_payload` unboxes `x` to Int64 and it unifies with `d`. Without
        // this, `x` binds as the declared opaque `Heap` (Option's polymorphic `Some a`) and the arms
        // "differ in kind". Only fills variants the scrutinee shape did not already pin.
        if let Some(inferred) = self.infer_sum_payload_override(arms, env) {
            for (tag, kinds) in inferred {
                payload_override.entry(tag).or_insert(kinds);
            }
        }

        // The per-variant payload SHAPE, parallel to the kind override: lets a HEAP payload binder
        // carry its compound shape (a `Some`-payload record) so a later `(. bound field)` resolves.
        let payload_shapes = self
            .shape_of(scrutinee, env)
            .map(|s| shape_variant_payload_shapes(&s))
            .unwrap_or_default();

        let body = self.gen_sum_arms(handle, &order, arms, &payload_override, &payload_shapes, env, ctx)?;
        let mut c = prelude;
        c.extend_from_slice(&body.0);
        Ok((c, body.1))
    }

    /// Recover a per-variant payload-KIND override by UNIFYING a runtime sum match's arm result
    /// kinds, for when the scrutinee's shape could not pin them (an opaque `Heap` parameter). For a
    /// SINGLE-payload constructor arm `((Ctor binder) body)` whose `binder` is a bare name, seed the
    /// binder as an inference variable in a SHARED `InferCtx` (also carrying the enclosing locals'
    /// kinds), infer each arm body, UNIFY the arm results (a concrete scalar beats the Int64 default,
    /// via `unify_branch_kinds`), then back-propagate that unified kind to each arm body — so a binder
    /// RETURNED directly (`((Some x) x)`) is constrained to the sibling arms' kind. Read back each
    /// such binder's solved kind; a concrete scalar (`Int64`/`Bool`/`Float64`) becomes that variant's
    /// `[kind]` override (so `bind_sum_payload` unboxes it). Returns `None` if nothing concrete was
    /// recovered (leaving the declared opaque kinds). Catch-all arms and multi-slot tuple binders are
    /// skipped (the scrutinee-shape path handles tuple payloads). This is the parameter-boundary twin
    /// of the scrutinee-shape override: the built-in polymorphic `Option`'s payload survives a match
    /// in a helper the same way a user sum's does.
    fn infer_sum_payload_override(
        &self,
        arms: &[Node],
        env: &[Local],
    ) -> Option<std::collections::BTreeMap<String, Vec<Kind>>> {
        // Seed the inference vars: the enclosing scalar locals (a param `d` used in a `None` arm
        // must infer as its known kind), then each constructor arm's single bare binder.
        let mut vars: Vec<(String, Option<Kind>)> = env
            .iter()
            .filter(|l| l.alias.is_none())
            .map(|l| (l.name.clone(), Some(l.kind)))
            .collect();
        // Collect binders whose kind we want to recover, per constructor arm:
        //   * a single bare-name binder `(Ctor b)` — recovered as a one-slot override `[kind(b)]`;
        //   * a FLAT tuple binder `(Ctor (tuple s0 s1 …))` — each `si` recovered, override
        //     `[kind(s0), kind(s1), …]` (the payload IS the tuple array, so its slots are the tuple
        //     elements). This is the assoc-list / env idiom: `((Some (tuple key val)) (= key k))`
        //     over a list whose element shape is opaque (a parameter), where `key`/`val` would
        //     otherwise bind as opaque `Heap` and the `=`/arm-unification fail.
        // A tag maps to a Vec of (slot-index, binder-name); a bare binder is a single slot 0.
        let mut binders: Vec<(String, Vec<(usize, String)>)> = Vec::new();
        for arm in arms {
            let a = match arm {
                Node::List(a) if a.len() == 2 => a,
                _ => return None,
            };
            if let Node::List(pat) = &a[0] {
                if let Some(ctor) = constructor_of(pat.first()) {
                    let tag = variant_tag(&ctor).to_string();
                    match pat.get(1) {
                        Some(Node::Name(b)) if b != "_" => {
                            binders.push((tag, vec![(0, b.clone())]));
                            vars.push((b.clone(), None));
                        }
                        Some(Node::List(t)) if name_of(t.first()) == Some("tuple") => {
                            let mut slots = Vec::new();
                            for (i, sb) in t[1..].iter().enumerate() {
                                if let Node::Name(name) = sb {
                                    if name != "_" {
                                        slots.push((i, name.clone()));
                                        vars.push((name.clone(), None));
                                    }
                                }
                            }
                            if !slots.is_empty() {
                                binders.push((tag, slots));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if binders.is_empty() {
            return None;
        }
        let mut ictx = InferCtx { compiler: self, vars };
        // Infer + unify the arm result kinds, then back-propagate to each arm body (mirroring the
        // `if`/`match` result-kind rule) so a binder returned directly is constrained.
        let mut result: Option<Kind> = None;
        for arm in arms {
            if let Node::List(a) = arm {
                if a.len() == 2 {
                    let k = ictx.infer(&a[1]);
                    result = unify_branch_kinds(result, k);
                }
            }
        }
        if let Some(k) = result {
            for arm in arms {
                if let Node::List(a) = arm {
                    if a.len() == 2 {
                        ictx.expect(&a[1], k);
                    }
                }
            }
        }
        // Read back each binder's solved kind. For a single bare binder → a one-slot override
        // `[kind]`. For a tuple binder → a per-slot vector (slot `i` → its recovered kind), so
        // `bind_sum_payload_kinds`'s tuple branch unboxes each scalar element. A slot that did not
        // resolve to a concrete scalar stays `Heap` (an opaque handle — a nested compound element).
        // Only emit an override if at least one slot recovered a concrete scalar (else the declared
        // kinds already suffice and an all-Heap override adds nothing).
        let mut out: std::collections::BTreeMap<String, Vec<Kind>> = Default::default();
        for (tag, slots) in binders {
            let arity = slots.iter().map(|(i, _)| *i + 1).max().unwrap_or(0);
            let mut kinds = vec![Kind::Heap; arity];
            let mut any_scalar = false;
            for (i, name) in &slots {
                if let Some((_, Some(k))) = ictx.vars.iter().find(|(n, _)| n == name) {
                    if matches!(k, Kind::Int64 | Kind::Bool | Kind::Float64) {
                        kinds[*i] = *k;
                        any_scalar = true;
                    }
                }
            }
            if any_scalar {
                out.insert(tag, kinds);
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Emit the nested if/else dispatch for a runtime sum match, reading the handle from local
    /// `handle`. `order` is the sum type's variants in declaration order (index = discriminant).
    /// Emits, for the FIRST remaining arm, `if sum-disc(handle) == disc { bind ; body } else
    /// { <rest> }`; the innermost `else` (no arms left) is `unreachable` (an exhaustive match
    /// always selects an arm). An `else`/`_`/bare-name catch-all arm is the unconditional tail.
    fn gen_sum_arms(
        &self,
        handle: u32,
        order: &[String],
        arms: &[Node],
        payload_override: &std::collections::BTreeMap<String, Vec<Kind>>,
        payload_shapes: &std::collections::BTreeMap<String, Shape>,
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let arm = match arms.first() {
            Some(Node::List(a)) if a.len() == 2 => a,
            Some(_) => return decline("malformed match arm"),
            // Ran out of arms: an exhaustive match always matched one, so this is unreachable.
            // Emit `unreachable` typed as the surrounding result (Never unifies with anything).
            None => return Ok((vec![op::UNREACHABLE], Kind::Never)),
        };
        let (pattern, arm_body) = (&arm[0], &arm[1]);

        // A catch-all arm (`else`, `_`, or a bare name binding the whole scrutinee value).
        match pattern {
            Node::Name(n) if n == "else" || n == "_" => return self.emit(arm_body, env, ctx),
            Node::Name(n) => {
                let mut body_env = env.to_vec();
                body_env.push(Local::scalar(n.clone(), handle, Kind::Heap));
                return self.emit(arm_body, &body_env, ctx);
            }
            _ => {}
        }

        // A constructor pattern `(Ctor binder)` / `((. Ty Ctor) binder)`. The remaining arms form
        // this arm's else branch (its fall-through) — computed FIRST so a NESTED constructor payload
        // binder can share it: when the payload is itself a sum and its inner discriminant does not
        // match, control must fall through to the SAME sibling arms as an outer-disc mismatch. (The
        // else bytes reference already-allocated locals, so emitting them in two branches is safe —
        // only one runs.) The `order` here is only used to sanity-check the top-level arm's variant;
        // `gen_ctor_arm` re-derives the sum type/order from each pattern's own constructor, so it
        // works for an inner sum of a different type too.
        let _ = order;
        let (else_c, else_k) = self.gen_sum_arms(handle, order, &arms[1..], payload_override, payload_shapes, env, ctx)?;
        self.gen_ctor_arm(handle, pattern, arm_body, &else_c, else_k, payload_override, payload_shapes, env, ctx)
    }

    /// Emit one constructor-pattern arm against the runtime sum in local `handle`:
    /// `if sum-disc(handle) == disc { bind the pattern's payload ; arm_body } else { <else_c> }`,
    /// where `else_c`/`else_k` is the pre-emitted fall-through (the sibling arms). The sum type and
    /// discriminant are derived from the pattern's OWN constructor, so this works both for a
    /// top-level arm and — via recursion — for a NESTED constructor payload binder `(Ctor (Inner b))`
    /// whose payload is itself a runtime sum (an `Option`/`Result` carrying a user-sum, `(W.Wrap
    /// (N.L v))`, …). For a nested binder the payload is materialized into a local and this recurses
    /// on the inner pattern, threading the SAME `else_c` as the inner dispatch's fall-through — so a
    /// non-matching inner variant falls through to the outer arm's siblings, exactly as a hand-written
    /// `(match payload (inner body) (_ <siblings>))` would. A `(tuple …)`/bare-name/`_` binder binds
    /// as before (`bind_sum_payload_kinds`).
    fn gen_ctor_arm(
        &self,
        handle: u32,
        pattern: &Node,
        arm_body: &Node,
        else_c: &[u8],
        else_k: Kind,
        payload_override: &std::collections::BTreeMap<String, Vec<Kind>>,
        payload_shapes: &std::collections::BTreeMap<String, Shape>,
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let pat_items = match pattern {
            Node::List(items) if items.len() == 2 => items,
            _ => return decline("runtime sum match arm is not a constructor pattern"),
        };
        let ctor = match constructor_of(pat_items.first()) {
            Some(c) => c,
            None => return decline("runtime sum match arm is not a constructor pattern"),
        };
        let tag = variant_tag(&ctor).to_string();
        // Derive the sum type + variant order from THIS pattern's constructor (not the caller's), so
        // a nested inner pattern of a different sum type resolves its own discriminant.
        let type_name = match self.sum_types.get(&tag) {
            Some(t) => t.clone(),
            None => return decline("runtime sum match on an undeclared variant"),
        };
        let order = match self.sum_variants.get(&type_name) {
            Some(o) => o.clone(),
            None => return decline("sum type has no recorded variant order"),
        };
        let disc = match order.iter().position(|v| v == &tag) {
            Some(i) => i as u32,
            None => return decline("runtime sum match arm variant not in the sum type's order"),
        };
        let binder = &pat_items[1];

        // Compute this arm's payload-binding code + body (the `then`). A NESTED constructor payload
        // binder materializes the payload (itself a runtime sum) into a local and recurses; a flat
        // binder (`(tuple …)`/name/`_`) binds via `bind_sum_payload_kinds` and emits the body.
        let (bind_code, then_c, then_k) = if is_constructor_pattern(binder) {
            let payload = ctx.alloc_local(Kind::Heap);
            let mut bc = vec![op::LOCAL_GET];
            uleb128(handle as u64, &mut bc);
            bc.push(op::CALL);
            uleb128(himport::SUM_PAYLOAD as u64, &mut bc);
            bc.push(op::LOCAL_SET);
            uleb128(payload as u64, &mut bc);
            let (inner_c, inner_k) =
                self.gen_ctor_arm(payload, binder, arm_body, else_c, else_k, payload_override, payload_shapes, env, ctx)?;
            (bc, inner_c, inner_k)
        } else if let Some(refutable) =
            tuple_binder_refutable_slots(binder).filter(|r| !r.is_empty())
        {
            // A `(tuple … (Inner.X v) …)` binder — a CONSTRUCTOR pattern occupying a tuple SLOT of the
            // payload (the composition of a tuple-payload binder and a nested-constructor binder, the
            // shape a HOL kernel's equation arm `(Comb (tuple (Comb …) r))` takes). The ctor slot is
            // REFUTABLE, so it needs runtime discriminant DISPATCH on that slot — not just binding — and
            // its non-match must fall through to the SAME sibling arms as an outer-disc mismatch. Bind
            // the payload tuple's IRREFUTABLE slots (names/`_`/scalars) normally, read the refutable
            // slot's handle, then RECURSE the ctor-arm dispatcher on that slot with the shared `else_c`
            // fall-through — exactly as the nested-ctor-directly-under-ctor case threads `else_c`, but
            // one level down through a tuple element. (05-compound-types.sexp §"a constructor pattern in
            // a tuple payload slot is matched in one arm".) A single refutable slot is lowered here; two
            // refutable slots in one tuple (`(tuple (A x) (B y))`) is a product-of-sums dispatch not yet
            // emitted — decline (the bind-then-re-match route around it is always available).
            if refutable.len() != 1 {
                return decline("runtime sum match: multiple constructor patterns in one tuple payload");
            }
            let slot_j = refutable[0];
            let tuple_items = match binder {
                Node::List(items) => items,
                _ => unreachable!("tuple_binder_refutable_slots returned Some for a non-list"),
            };
            let tuple_binders = &tuple_items[1..];
            // Materialize the payload tuple ARRAY once.
            let arr = ctx.alloc_local(Kind::Heap);
            let mut bc = vec![op::LOCAL_GET];
            uleb128(handle as u64, &mut bc);
            bc.push(op::CALL);
            uleb128(himport::SUM_PAYLOAD as u64, &mut bc);
            bc.push(op::LOCAL_SET);
            uleb128(arr as u64, &mut bc);
            // Bind the IRREFUTABLE slots: the refutable slot is replaced by `_` so `bind_tuple_elems`
            // skips it (its handle is read separately below). The declared per-slot types come from
            // this variant's tuple payload (`sum_payload_types[tag]` unwraps the `(Tuple …)`), and the
            // per-slot kind override (arm unification) recovers concrete scalars (`k : Int64`).
            let mut irref_binders: Vec<Node> = tuple_binders.to_vec();
            irref_binders[slot_j] = Node::Name("_".into());
            let slot_types = self.sum_payload_types.get(&tag).cloned().unwrap_or_default();
            let mut body_env = env.to_vec();
            self.bind_tuple_elems(
                arr, &irref_binders, &slot_types, payload_override.get(&tag),
                &mut bc, &mut body_env, ctx,
            )?;
            // Read the refutable slot's handle, then dispatch the inner constructor on it, threading
            // the shared `else_c` so a non-matching inner variant falls through to the sibling arms.
            let slot_handle = ctx.alloc_local(Kind::Heap);
            bc.push(op::LOCAL_GET);
            uleb128(arr as u64, &mut bc);
            bc.push(op::I32_CONST);
            sleb128(slot_j as i64, &mut bc);
            bc.push(op::CALL);
            uleb128(himport::ARR_GET as u64, &mut bc);
            bc.push(op::LOCAL_SET);
            uleb128(slot_handle as u64, &mut bc);
            let (inner_c, inner_k) = self.gen_ctor_arm(
                slot_handle, &tuple_binders[slot_j], arm_body, else_c, else_k,
                payload_override, payload_shapes, &body_env, ctx,
            )?;
            (bc, inner_c, inner_k)
        } else {
            // A concrete per-variant payload-kind override (from the scrutinee's shape) takes
            // precedence over the declared (possibly opaque) kinds — so `(Some x)` on a `(Bytes.at …)`
            // binds `x` as the Int64 byte, not an opaque Heap handle. The parallel payload-SHAPE
            // override lets a HEAP payload binder carry its compound shape (a `Some`-payload record).
            let (bind_code, body_env) =
                self.bind_sum_payload_kinds(handle, &tag, binder, payload_override.get(&tag), payload_shapes.get(&tag), env, ctx)?;
            let (then_c, then_k) = self.emit(arm_body, &body_env, ctx)?;
            (bind_code, then_c, then_k)
        };

        let result = match Kind::unify(then_k, else_k) {
            Some(k) => k,
            None => return decline("runtime sum match arms differ in kind"),
        };

        // condition: sum-disc(handle) == disc
        let mut c = vec![op::LOCAL_GET];
        uleb128(handle as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_DISC as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(disc as i64, &mut c);
        c.push(0x46); // i32.eq
        c.push(op::IF);
        c.push(result.core_valtype());
        c.extend_from_slice(&bind_code);
        c.extend_from_slice(&then_c);
        c.push(op::ELSE);
        c.extend_from_slice(else_c);
        c.push(op::END);
        Ok((c, result))
    }

    /// Bind a constructor arm's payload binders to heap-read locals, returning the code that
    /// populates those locals (to run inside the arm's `then` block) and the extended environment
    /// the arm body is emitted under. `payload = sum-payload(handle)` is a heap object; a
    /// `(tuple b0 … bn)` binder reads each slot `arr-get(payload, k)` (unboxing scalars per the
    /// variant's recorded payload kinds); a bare-name binder binds the whole payload handle; `_`
    /// or `unit` binds nothing (a nullary variant's unit payload).
    /// Bind a runtime sum variant's payload from the heap value in local `handle`, with an optional
    /// per-slot payload-KIND override (from the scrutinee's static shape). When `override_kinds` is
    /// `Some`, it replaces the declared `sum_payload_kinds` for this variant — so a concretely-typed
    /// producer (a `(Bytes.at …)` yielding `Option<Int64>`) binds its `(Some x)` payload as the
    /// Int64 byte, not the prelude's opaque `Heap` (the polymorphic type parameter `a`). `None` → the
    /// declared kinds. A `(tuple …)` binder reads each slot via `arr-get` (recursing into a nested
    /// tuple); a bare-name binder binds the whole payload; `_`/`unit` bind nothing.
    fn bind_sum_payload_kinds(
        &self,
        handle: u32,
        tag: &str,
        binder: &Node,
        override_kinds: Option<&Vec<Kind>>,
        payload_shape: Option<&Shape>,
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Vec<Local>), Decline> {
        let mut code = Vec::new();
        let mut body_env = env.to_vec();

        // A `_` / `unit` binder for a nullary (or ignored) payload binds nothing.
        if matches!(binder, Node::Name(n) if n == "_" || n == "unit") {
            return Ok((code, body_env));
        }

        // The payload's per-slot kinds: the scrutinee-shape override when present (concrete types),
        // else the declared kinds (Int64/Bool/Float → unbox; Heap → keep the handle). Absent (e.g. a
        // prelude variant with no override) → treat every slot as Heap.
        let slot_kinds = match override_kinds {
            Some(k) => k.clone(),
            None => self.sum_payload_kinds.get(tag).cloned().unwrap_or_default(),
        };

        match binder {
            // A tuple-destructuring binder `(tuple b0 … bn)`: read each element from the payload
            // array. `payload = sum-payload(handle)` is itself a heap array of the tuple elements.
            // A slot binder that is ITSELF a `(tuple …)` recurses (a nested payload tuple, the shape
            // a resolver's `(NPrim (Tuple op (Tuple a b)))` node takes) — `bind_tuple_elems` reads
            // the sub-array handle and destructures it by the same slot logic.
            Node::List(items) if name_of(items.first()) == Some("tuple") => {
                // Materialize the payload handle once into a local.
                let payload = ctx.alloc_local(Kind::Heap);
                code.push(op::LOCAL_GET);
                uleb128(handle as u64, &mut code);
                code.push(op::CALL);
                uleb128(himport::SUM_PAYLOAD as u64, &mut code);
                code.push(op::LOCAL_SET);
                uleb128(payload as u64, &mut code);

                // Per-slot TYPE nodes, so a nested tuple slot recurses with its element types (and
                // each scalar sub-element unboxes correctly). Absent (a prelude variant) → empty,
                // which `bind_tuple_elems` treats as all-Heap slots (the pre-existing convention).
                let slot_types = self.sum_payload_types.get(tag).cloned().unwrap_or_default();
                // A per-slot KIND override (from arm unification) takes precedence over the declared
                // types: for a built-in `Some` whose payload is a tuple (`((Some (tuple key val)) …)`
                // over an opaque list), the declared payload type is the polymorphic `a` (no slot
                // types), so `key`/`val` would bind as opaque `Heap`; the arm-unification override
                // recovers each slot's concrete scalar kind (`key : Int64` from `(= key k)`) so it
                // unboxes. `override_kinds` is that per-slot vector when the payload is the tuple.
                self.bind_tuple_elems(payload, &items[1..], &slot_types, override_kinds, &mut code, &mut body_env, ctx)?;
                Ok((code, body_env))
            }
            // A bare-name binder binds the whole payload. Its kind is the single recorded slot kind
            // (a one-field payload like `(Ok a)`), defaulting to an opaque heap handle.
            Node::Name(name) => {
                let kind = match slot_kinds.as_slice() {
                    [k] => *k,
                    _ => Kind::Heap,
                };
                let local = ctx.alloc_local(kind);
                code.push(op::LOCAL_GET);
                uleb128(handle as u64, &mut code);
                code.push(op::CALL);
                uleb128(himport::SUM_PAYLOAD as u64, &mut code);
                if let Some(unbox) = unbox_fn(kind) {
                    code.push(op::CALL);
                    uleb128(unbox as u64, &mut code);
                }
                code.push(op::LOCAL_SET);
                uleb128(local as u64, &mut code);
                // For a HEAP payload (a compound: record/list/nested sum, or a String leaf), carry
                // its static `Shape` so a later projection/access/equality on the bound name sees
                // through the opaque handle — e.g. `(match (List.at inputs 0) ((Some a) (. a bytes))
                // …)` where `a`'s shape is the list element's `Record`, or `((Ast.Name nm) (= nm s))`
                // where `nm`'s shape is `Str` (so `=` lowers to a byte compare, not a heap-walk
                // decline). The shape comes from the scrutinee's inferred payload shape
                // (`shape_variant_payload_shapes`) when available; ELSE fall back to the variant's
                // DECLARED single-slot payload type (`sum_payload_types[tag]`) — so `Ast.Name`'s
                // `String` payload gives `nm : Str` even when the scrutinee `h` arrived through a deep
                // nesting whose runtime shape is opaque (the two-runtime-string equality gap). Absent
                // both → `None` (opaque, unchanged). A scalar payload keeps `None` (its kind suffices).
                let shape = if kind == Kind::Heap {
                    payload_shape.cloned().or_else(|| {
                        match self.sum_payload_types.get(tag).map(Vec::as_slice) {
                            Some([ty]) => shape_of_type_node(ty),
                            _ => None,
                        }
                    })
                } else {
                    None
                };
                body_env.push(Local::scalar_shaped(name.clone(), local, kind, shape));
                Ok((code, body_env))
            }
            _ => decline("runtime sum match: unsupported payload binder"),
        }
    }

    /// Bind a `(tuple b0 … bn)` binder's elements from a heap array whose handle is already in local
    /// `arr`. Each slot `i`: read `arr-get(arr, i)`, then bind `binders[i]` by its shape —
    /// - a scalar name (Int64/Bool/Float64 per `slot_types[i]`) → unbox and bind the local;
    /// - a bare Heap name → bind the raw handle (a sub-node/list/string, kept opaque);
    /// - `_` → bind nothing;
    /// - a NESTED `(tuple …)` binder → materialize the slot handle into a fresh local and RECURSE
    ///   with the nested element types (`slot_types[i]` is a `(Tuple …)` node, so its `[1..]` are the
    ///   inner slot types). This is the whole Tier-2b fix: a payload tuple may nest arbitrarily deep,
    ///   the shape a resolver's tagged node (`(NPrim (Tuple op (Tuple a b)))`) takes.
    /// Appends to `code` and pushes each binding onto `body_env`. `slot_types` may be shorter than
    /// `binders` (a prelude variant records none) — a missing slot type defaults to an opaque Heap
    /// handle, the pre-existing convention.
    fn bind_tuple_elems(
        &self,
        arr: u32,
        binders: &[Node],
        slot_types: &[Node],
        override_kinds: Option<&Vec<Kind>>,
        code: &mut Vec<u8>,
        body_env: &mut Vec<Local>,
        ctx: &mut FnCtx,
    ) -> Result<(), Decline> {
        for (i, b) in binders.iter().enumerate() {
            let slot_ty = slot_types.get(i);
            // A per-slot KIND override (from arm unification) takes precedence over the declared
            // type — the assoc-list `((Some (tuple key val)) …)` case where the payload tuple's slot
            // types are unknown (a polymorphic `Some a`) but `key`/`val` recover concrete scalars
            // from their uses. Only a concrete scalar override applies; a `Heap` override slot falls
            // back to the declared type (a nested compound keeps its structural type).
            let override_slot = override_kinds
                .and_then(|ks| ks.get(i))
                .filter(|k| matches!(k, Kind::Int64 | Kind::Bool | Kind::Float64));
            match b {
                // `_` binds nothing.
                Node::Name(n) if n == "_" => continue,
                // A nested tuple binder: read the slot's sub-array handle into a local, recurse.
                Node::List(items) if name_of(items.first()) == Some("tuple") => {
                    let sub = ctx.alloc_local(Kind::Heap);
                    code.push(op::LOCAL_GET);
                    uleb128(arr as u64, code);
                    code.push(op::I32_CONST);
                    sleb128(i as i64, code);
                    code.push(op::CALL);
                    uleb128(himport::ARR_GET as u64, code);
                    code.push(op::LOCAL_SET);
                    uleb128(sub as u64, code);
                    // The nested slot's element types: `slot_ty` is a `(Tuple …)` node → its tail.
                    let sub_types: Vec<Node> = match slot_ty {
                        Some(Node::List(t)) if name_of(t.first()) == Some("Tuple") => t[1..].to_vec(),
                        _ => Vec::new(),
                    };
                    self.bind_tuple_elems(sub, &items[1..], &sub_types, None, code, body_env, ctx)?;
                }
                // A name binder: unbox to its overridden/declared scalar kind, else keep the handle.
                Node::Name(name) => {
                    let kind = override_slot
                        .copied()
                        .or_else(|| slot_ty.map(type_node_to_kind))
                        .unwrap_or(Kind::Heap);
                    let local = ctx.alloc_local(kind);
                    code.push(op::LOCAL_GET);
                    uleb128(arr as u64, code);
                    code.push(op::I32_CONST);
                    sleb128(i as i64, code);
                    code.push(op::CALL);
                    uleb128(himport::ARR_GET as u64, code);
                    if let Some(unbox) = unbox_fn(kind) {
                        code.push(op::CALL);
                        uleb128(unbox as u64, code);
                    }
                    code.push(op::LOCAL_SET);
                    uleb128(local as u64, code);
                    body_env.push(Local::scalar(name.clone(), local, kind));
                }
                _ => return decline("runtime sum match: unsupported nested payload binder"),
            }
        }
        Ok(())
    }

    /// Emit the nested if/else for the remaining `arms`, reading the scrutinee from `slot`.
    fn gen_match_arms(
        &self,
        slot: u32,
        scrut_kind: Kind,
        arms: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let arm = match arms.first() {
            Some(Node::List(a)) if a.len() == 2 => a,
            Some(_) => return decline("malformed match arm"),
            // Ran out of arms with no catch-all: a non-exhaustive match over a runtime
            // scrutinee → reject (the literal set does not cover all values).
            None => return reject("CDZ0210", "match does not cover the scrutinee"),
        };
        let (pattern, body) = (&arm[0], &arm[1]);

        // A catch-all arm: `else`, `_`, or a bare name binder (binds the scrutinee value).
        match pattern {
            Node::Name(n) if n == "else" || n == "_" => {
                return self.emit(body, env, ctx);
            }
            Node::Name(n) => {
                // Bind the name to the scrutinee local for the body.
                let mut body_env = env.to_vec();
                body_env.push(Local::scalar(n.clone(), slot, scrut_kind));
                return self.emit(body, &body_env, ctx);
            }
            _ => {}
        }

        // A literal pattern: emit `if (slot == literal) then body else <rest>`.
        let lit = match pattern {
            Node::Int(_) | Node::Bool(_) => pattern,
            _ => return decline("runtime match with a non-literal pattern"),
        };
        // A Bool scrutinee is exhausted by {true, false}: the LAST arm of a two-value bool
        // match covers the only remaining value, so it is the unconditional else (no trailing
        // non-exhaustive reject). This makes `(match b (true …) (false …))` total, matching the
        // corpus's exhaustive bool match with no `else`.
        if scrut_kind == Kind::Bool && matches!(pattern, Node::Bool(_)) && arms.len() == 1 {
            return self.emit(body, env, ctx);
        }
        let (lit_code, lit_kind) = self.emit(lit, env, ctx)?;
        if lit_kind != scrut_kind {
            return decline("runtime match literal kind mismatch");
        }
        // condition: slot == literal
        let mut cond = vec![op::LOCAL_GET];
        uleb128(slot as u64, &mut cond);
        cond.extend_from_slice(&lit_code);
        cond.push(if scrut_kind == Kind::Int64 { op::I64_EQ } else { 0x46 }); // i64.eq / i32.eq

        let (then_c, then_k) = self.emit(body, env, ctx)?;
        let (else_c, else_k) = self.gen_match_arms(slot, scrut_kind, &arms[1..], env, ctx)?;
        let result = match Kind::unify(then_k, else_k) {
            Some(k) => k,
            None => return decline("runtime match arms differ in kind"),
        };
        let mut c = cond;
        c.push(op::IF);
        c.push(result.core_valtype());
        c.extend_from_slice(&then_c);
        c.push(op::ELSE);
        c.extend_from_slice(&else_c);
        c.push(op::END);
        Ok((c, result))
    }

    /// Attempt to match `pattern` against `scrutinee` (resolved structurally) at compile
    /// time. Returns `Some(bindings)` on a match (bindings alias pattern binders to the
    /// scrutinee's sub-nodes), `None` on a definite non-match, or a decline if the shape is
    /// beyond compile-time resolution.
    fn try_match(
        &self,
        pattern: &Node,
        scrutinee: &Node,
        env: &[Local],
    ) -> Result<Option<Vec<Local>>, Decline> {
        match pattern {
            // Literal patterns match by equality against a literal scrutinee.
            Node::Int(p) => Ok(match self.resolve_scalar_literal(scrutinee, env) {
                Some(ScalarLit::Int(v)) if v == *p => Some(vec![]),
                Some(_) => None,
                None => None,
            }),
            Node::Str(p) => {
                // Fold the scrutinee to a string value — a literal, or an expression like
                // `(String.concat …)` / `(String.slice …)` that yields a string — and compare.
                let sval = match self.eval_const(scrutinee, env) {
                    Ok(Some(CVal::Str(s))) => Some(s),
                    _ => match self.resolve(scrutinee, env) {
                        Some((Node::Str(s), _)) => Some(s),
                        _ => None,
                    },
                };
                Ok(match sval {
                    Some(v) if &v == p => Some(vec![]),
                    _ => None,
                })
            }
            Node::Bool(p) => Ok(match self.resolve_scalar_literal(scrutinee, env) {
                Some(ScalarLit::Bool(v)) if v == *p => Some(vec![]),
                _ => None,
            }),
            // A bare name pattern binds the whole scrutinee.
            Node::Name(n) if n == "_" => Ok(Some(vec![])),
            Node::Name(n) => {
                Ok(Some(vec![Local::aliased(n.clone(), scrutinee.clone(), env.to_vec())]))
            }
            // A list pattern: constructor `(Ctor binder)` or tuple `(tuple a b …)`.
            Node::List(pitems) => self.try_match_list(pitems, scrutinee, env),
            _ => decline("unsupported pattern"),
        }
    }

    fn try_match_list(
        &self,
        pitems: &[Node],
        scrutinee: &Node,
        env: &[Local],
    ) -> Result<Option<Vec<Local>>, Decline> {
        // Resolve the scrutinee to its structure.
        let (sval, senv) = match self.resolve(scrutinee, env) {
            Some(r) => r,
            None => return decline("match scrutinee is not compile-time-resolvable"),
        };
        let sitems = match &sval {
            Node::List(items) => items,
            _ => return Ok(None),
        };
        let shead = name_of(sitems.first());

        // Tuple pattern (tuple a b …) against (tuple x y …).
        if name_of(pitems.first()) == Some("tuple") {
            if shead != Some("tuple") || sitems.len() != pitems.len() {
                return Ok(None);
            }
            let mut binds = Vec::new();
            for (pp, ss) in pitems[1..].iter().zip(&sitems[1..]) {
                match self.try_match(pp, ss, &senv)? {
                    Some(mut b) => binds.append(&mut b),
                    None => return Ok(None),
                }
            }
            return Ok(Some(binds));
        }

        // Element pattern `(list p0 p1 …)` or `(list p0 … .. rest)` against a `(list e0 e1 …)`
        // scrutinee (core-semantics.md §A List Is Deconstructed By Element Patterns With An
        // Optional Rest). A `..` marker (read as the name `..`) splits leading element patterns
        // from an optional rest binder: `(list)` matches only the empty list; `(list a b)` matches
        // a length-2 list; `(list x .. rest)` matches any non-empty list, binding `x` to the first
        // element and `rest` to a `(list …)` of the remainder. The scrutinee is observed only
        // through its length and elements-in-order (a `(list …)` node) — no representation leak.
        if name_of(pitems.first()) == Some("list") {
            if shead != Some("list") {
                return Ok(None); // a `(list …)` pattern can only match a list value
            }
            let pelems = &pitems[1..];
            let selems = &sitems[1..];
            // Split the pattern's element sub-patterns at a `..` rest marker, if present.
            let rest_pos = pelems.iter().position(|p| name_of(Some(p)) == Some(".."));
            let (leading, rest_binder) = match rest_pos {
                Some(i) => {
                    // Exactly one binder may follow `..`; `(list x ..)` (no binder) or
                    // `(list x .. a b)` (more than one) is a malformed pattern.
                    if pelems.len() != i + 2 {
                        return decline("list rest pattern must be `.. <binder>`");
                    }
                    (&pelems[..i], Some(&pelems[i + 1]))
                }
                None => (pelems, None),
            };
            // Length check: exact for a fixed-arity pattern, at-least for a rest pattern.
            match rest_binder {
                None if selems.len() != leading.len() => return Ok(None),
                Some(_) if selems.len() < leading.len() => return Ok(None),
                _ => {}
            }
            let mut binds = Vec::new();
            for (pp, ss) in leading.iter().zip(selems.iter()) {
                match self.try_match(pp, ss, &senv)? {
                    Some(mut b) => binds.append(&mut b),
                    None => return Ok(None),
                }
            }
            if let Some(binder) = rest_binder {
                // The remaining elements as a fresh `(list …)` node — a compile-time value the
                // rest binder aliases (the static/const-fold path; a runtime list tail is a
                // separate lowering, deferred). A `_`/`else` rest discards it.
                let mut rest_list = vec![Node::Name("list".into())];
                rest_list.extend_from_slice(&selems[leading.len()..]);
                let rest_node = Node::List(rest_list);
                match self.try_match(binder, &rest_node, &senv)? {
                    Some(mut b) => binds.append(&mut b),
                    None => return Ok(None),
                }
            }
            return Ok(Some(binds));
        }

        // Constructor pattern (Ctor binder) against a constructor application. The pattern
        // head may be a bare `Some` or a qualified `(. Sign Neg)`; `constructor_of` handles both.
        if let Some(pctor) = constructor_of(pitems.first()) {
            if pitems.len() != 2 {
                return decline("constructor pattern arity");
            }
            match constructor_of(sitems.first()) {
                // The scrutinee IS a constructor application: match by variant name.
                Some(sctor) => {
                    if sctor != pctor {
                        return Ok(None); // a definite non-match on this arm
                    }
                    let payload = sitems.get(1).cloned().unwrap_or(Node::Name("unit".into()));
                    self.try_match(&pitems[1], &payload, &senv)
                }
                // The scrutinee is some other form (e.g. `quote`, an AST value we don't
                // resolve). Not a definite non-match — beyond compile-time resolution, so
                // decline (todo) rather than mis-trap.
                None => decline("constructor pattern against unresolved scrutinee (e.g. quote/AST)"),
            }
        } else {
            decline("unsupported list pattern")
        }
    }

    /// Resolve a node to a scalar literal if it is a compile-time integer/bool value —
    /// including one produced by a const-foldable EXPRESSION (a record-field/tuple-element
    /// access, a string op, arithmetic). A member/tuple access scrutinee like `(. r n)` must
    /// match a literal arm by its folded value, exactly as a bound name does.
    fn resolve_scalar_literal(&self, node: &Node, env: &[Local]) -> Option<ScalarLit> {
        match node {
            Node::Int(n) => Some(ScalarLit::Int(*n)),
            Node::Bool(b) => Some(ScalarLit::Bool(*b)),
            Node::Name(n) => {
                let local = env.iter().rev().find(|l| l.name == *n)?;
                let (anode, aenv) = local.alias.as_ref()?;
                self.resolve_scalar_literal(anode, aenv)
            }
            // A folded expression (field access, string op, …) that yields a scalar.
            Node::List(_) => match self.eval_const(node, env) {
                Ok(Some(CVal::Int(n))) => Some(ScalarLit::Int(n)),
                Ok(Some(CVal::Bool(b))) => Some(ScalarLit::Bool(b)),
                _ => None,
            },
            _ => None,
        }
    }

    /// `(. obj field)` — a prelude scalar constant (`Int64.max`), or a record field
    /// projection. Record projection resolves the record to its structural form at compile
    /// time and emits the field's expression; a non-record or missing field traps.
    fn gen_member(&self, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if elems.len() != 3 {
            return decline("member arity");
        }
        // Scalar prelude constants first.
        if let (Some("Int64"), Some("max")) = (name_of(elems.get(1)), name_of(elems.get(2))) {
            let mut c = vec![op::I64_CONST];
            sleb128(i64::MAX, &mut c);
            return Ok((c, Kind::Int64));
        }
        if let (Some("Int64"), Some("min")) = (name_of(elems.get(1)), name_of(elems.get(2))) {
            let mut c = vec![op::I64_CONST];
            sleb128(i64::MIN, &mut c);
            return Ok((c, Kind::Int64));
        }
        // The field is an ordinary export name, OR a `(meta KEY)` metadata access that maps to
        // the reserved manifest key (distinct from any export — see `module_to_record`).
        let meta_key = meta_field_key(&elems[2]);
        let field = match meta_key.as_deref().or_else(|| name_of(elems.get(2))) {
            Some(f) => f,
            None => return decline("member field is not a name"),
        };
        // A projection distributes over a control form that CHOOSES the record at run time:
        // `(. (if c a b) f)` → `(if c (. a f) (. b f))`, and likewise over `let`/`do`/`match`.
        // So a record selected by a conditional is projected in each branch, where the record
        // is compile-time-known, rather than requiring the whole `(. … f)` to fold at once.
        if let Some(distributed) = distribute_projection(&elems[1], |leaf| {
            Node::List(vec![Node::Name(".".into()), leaf, elems[2].clone()])
        }) {
            return self.emit(&distributed, env, ctx);
        }
        // Resolve the object to its structural form.
        let (obj_node, obj_env) = match self.resolve(&elems[1], env) {
            Some(r) => r,
            None => {
                // Not a compile-time-known structure. Before treating it as a trap (`(. 5 x)`),
                // try the RUNTIME-record path: the operand may be a genuine value-heap record
                // handle (a record `let`-bound from a function result, a record element projected
                // out of a runtime `list<record>` — e.g. the compiler reading `(. (List.at inputs
                // 0) bytes)` from its input `list<artifact>`). A runtime record is a flat
                // positional array on the value heap (`arr-alloc`/`arr-set`) whose slots are the
                // field VALUES sorted by key (matching `gen_runtime_ctor`), so field `f` is
                // `arr-get(handle, slot(f))`, unboxed to `f`'s static kind — the record companion
                // of `gen_tuple_access`'s runtime path.
                return self.gen_runtime_member(elems, field, env, ctx);
            }
        };
        let items = match &obj_node {
            Node::List(items) if name_of(items.first()) == Some("record") => items,
            // `resolve` returned a non-record structure. If it is the OPERAND UNCHANGED (a runtime
            // expression `eval_const` could not fold — e.g. `(Option.expect (List.at inputs 0) "x")`,
            // whose value is a genuine value-heap record handle), it is not a compile-time record but a
            // RUNTIME one: take the runtime-record path (`arr-get` by shape), exactly as when `resolve`
            // returned None. Only a resolved-to-a-DIFFERENT non-record structure (a scalar/list/sum) is
            // member access on a non-record — a trap (ask-52: the `Option.expect`-unwrap tail of runtime
            // field access, the per-binding-form twin of the `match`-arm binder).
            _ if &obj_node == &elems[1] => return self.gen_runtime_member(elems, field, env, ctx),
            _ => return Ok((vec![op::UNREACHABLE], Kind::Never)), // member on non-record traps
        };
        // Find the field: (record (x <e>) (y <e>) …).
        for entry in &items[1..] {
            if let Node::List(kv) = entry {
                if name_of(kv.first()) == Some(field) {
                    return self.emit(&kv[1], &obj_env, ctx);
                }
            }
        }
        // Missing field traps.
        Ok((vec![op::UNREACHABLE], Kind::Never))
    }

    /// `(. r f)` on a RUNTIME record — the operand is not a compile-time structure but a genuine
    /// value-heap handle (a record returned from a function, a record element projected out of a
    /// runtime list). A runtime record is a flat positional array whose slots are the field values
    /// SORTED BY KEY (matching `gen_runtime_ctor`), so field `f` is `arr-get(handle, slot(f))`
    /// unboxed to `f`'s static kind, read from the operand's `Shape::Record`. This is the record
    /// twin of `gen_tuple_access`'s runtime path; without it, projecting a field off a runtime
    /// record emitted `unreachable` (a latent trap that then poisoned an enclosing constructor via
    /// `box_scalar`'s `Never` catch-all) — the block on the compiler reading `(. (List.at inputs 0)
    /// bytes)` out of its input `list<artifact>`.
    fn gen_runtime_member(
        &self,
        elems: &[Node],
        field: &str,
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let (rc, rk) = self.emit(&elems[1], env, ctx)?;
        // A definite trap (`(. 5 x)` — non-record operand whose emit diverged) propagates as-is.
        if rk == Kind::Never {
            return Ok((rc, Kind::Never));
        }
        if rk != Kind::Heap {
            // A scalar operand is member access on a non-record — a trap (the corpus records it so).
            return Ok((vec![op::UNREACHABLE], Kind::Never));
        }
        // `arr-get` is a value-heap import; the scalar path has none. Decline with a HEAP reason so
        // `compile_module` retries in runtime mode (the gate every runtime consumer uses).
        if self.call_base == 0 {
            return decline("runtime record access needs the value-heap runtime");
        }
        // The field's slot = its index among the record's fields SORTED BY KEY, and its kind = that
        // field's shape. An operand whose shape is not a known record declines (don't miscompile).
        let (slot, elem_kind) = match self.shape_of(&elems[1], env) {
            Some(Shape::Record(fields)) => {
                match fields.iter().position(|(k, _)| k == field) {
                    Some(i) => (i, shape_leaf_kind(&fields[i].1)),
                    None => return Ok((vec![op::UNREACHABLE], Kind::Never)), // missing field traps
                }
            }
            // Shape not a known record: cannot place the field — decline rather than guess a slot.
            _ => return decline("runtime member access on a value of unknown record shape"),
        };
        let mut c = rc;
        c.push(op::I32_CONST);
        sleb128(slot as i64, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_GET as u64, &mut c);
        if let Some(unbox) = unbox_fn(elem_kind) {
            c.push(op::CALL);
            uleb128(unbox as u64, &mut c);
        }
        Ok((c, elem_kind))
    }

    /// The static ARITY of a tuple `node` denotes, when known — a `(tuple …)` literal, a let-bound
    /// tuple, or a tuple RETURNED by a function (`resolve` beta-reduces the call to its `(tuple …)`
    /// body). `None` when the arity is not statically known (a parameter tuple, a runtime value, a
    /// non-tuple). Used to range-check a positional `(tuple.N …)` access against the tuple's arity
    /// uniformly across literal / let-bound / fn-return operands (05-compound-types.sexp §"a positional
    /// tuple access out of the tuple's static arity is a type error" + its fn-return companion). Uses
    /// `resolve` (which follows aliases and beta-reduces calls) so it subsumes the const-fold path AND
    /// reaches the fn-return / alias cases a `CVal::Tuple` const-fold does not.
    fn resolved_tuple_arity(&self, node: &Node, env: &[Local]) -> Option<usize> {
        let (resolved, _) = self.resolve(node, env)?;
        match &resolved {
            Node::List(items) if name_of(items.first()) == Some("tuple") => Some(items.len() - 1),
            _ => None,
        }
    }

    /// The field-name set of `node` when it resolves to a COMPILE-TIME `(record …)` — an inline
    /// record literal, a let-bound record, or a record RETURNED by a function (`resolve` beta-reduces
    /// the call to its `(record …)` body). `None` when the operand does not resolve to a record
    /// (a runtime record parameter, whose field set is not known at the projection site, imposes
    /// nothing — it declines / takes the runtime path later, never a false reject). Used by the
    /// member-access check to reject a projection of a field the record does NOT have (CDZ0201), the
    /// field-presence half of member-access well-formedness — the record-operand twin of
    /// `resolved_tuple_arity`'s out-of-arity check.
    fn resolved_record_fields(&self, node: &Node, env: &[Local]) -> Option<Vec<String>> {
        let (resolved, _) = self.resolve(node, env)?;
        match &resolved {
            Node::List(items) if name_of(items.first()) == Some("record") => {
                let mut fields = Vec::new();
                for entry in &items[1..] {
                    // Each field is `(key value)`; a malformed entry (no name key) makes the field
                    // set unreliable, so bail (impose nothing) rather than reject on a guess.
                    match entry {
                        Node::List(kv) => match name_of(kv.first()) {
                            Some(k) => fields.push(k.to_string()),
                            None => return None,
                        },
                        _ => return None,
                    }
                }
                Some(fields)
            }
            _ => None,
        }
    }

    /// `(tuple.N t)` — positional tuple access. Resolve the tuple structurally and emit its
    /// N-th element.
    fn gen_tuple_access(&self, head: &str, elems: &[Node], env: &[Local], ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        let idx: usize = match head[6..].parse() {
            Ok(i) => i,
            Err(_) => return decline("bad tuple accessor"),
        };
        if elems.len() != 2 {
            return decline("tuple accessor arity");
        }
        // Distribute over a control form choosing the tuple at run time (same rule as member
        // access): `(tuple.N (if c a b))` → `(if c (tuple.N a) (tuple.N b))`.
        if let Some(distributed) = distribute_projection(&elems[1], |leaf| {
            Node::List(vec![Node::Name(head.to_string()), leaf])
        }) {
            return self.emit(&distributed, env, ctx);
        }
        // A compile-time-resolvable tuple (an inline `(tuple …)` literal, or an alias chain to one):
        // extract element N structurally and re-emit it. This is the const/structural path. It fires
        // ONLY when the operand RESOLVES to an actual `(tuple …)` node — a resolve result that is NOT
        // a tuple is NOT a definite trap: `resolve` echoes an un-reducible operand back verbatim (e.g.
        // `(go 3 …)` beta-reduces `go` but its body is a recursive `if` that does not fold, so resolve
        // returns the `(if …)`/`(go …)` node), which just means "not statically a tuple" → fall through
        // to the RUNTIME `arr-get` path below, which declines cleanly on an unknown shape. Emitting
        // `unreachable` for any non-tuple resolve wrongly made a valid tuple-threading program TRAP at
        // the caller's `tuple.N` (05-compound-types.sexp §"projecting the result of a function that
        // threads a tuple parameter must not trap" — a decline-don't-miscompile violation).
        if let Some((t, tenv)) = self.resolve(&elems[1], env) {
            if let Node::List(items) = &t {
                if name_of(items.first()) == Some("tuple") {
                    return match items.get(idx + 1) {
                        Some(e) => self.emit(&e.clone(), &tenv, ctx),
                        None => Ok((vec![op::UNREACHABLE], Kind::Never)), // index past a known arity
                    };
                }
            }
        }
        // A RUNTIME tuple — the operand is not a compile-time structure but a genuine value-heap
        // handle (a `let`-bound tuple returned from a function, a tuple parameter). A runtime
        // tuple/record is a flat positional array on the value heap (`arr-alloc`/`arr-set`), so
        // element N is `arr-get(handle, N)`, unboxed to the element's static kind. This is the
        // consumption companion of `gen_runtime_ctor` (which builds the array) — without it a
        // `tuple.N` on a runtime tuple emitted `unreachable` (a latent trap), which broke a
        // recursive-descent decoder threading a `(Node, index)` tuple through `let`. The element
        // kind comes from the operand's static `Shape`; an unknown shape declines (don't miscompile).
        let (tc, tk) = self.emit(&elems[1], env, ctx)?;
        if tk != Kind::Heap {
            return decline("tuple.N of a non-tuple runtime value");
        }
        // `arr-get` is a value-heap import; the scalar path has none. Decline with a HEAP reason so
        // `compile_module` retries in runtime mode (the same gate the other runtime consumers use).
        if self.call_base == 0 {
            return decline("runtime tuple access needs the value-heap runtime");
        }
        let elem_kind = match self.shape_of(&elems[1], env) {
            Some(Shape::Tuple(elems_sh)) => match elems_sh.get(idx) {
                Some(s) => shape_leaf_kind(s),
                None => return Ok((vec![op::UNREACHABLE], Kind::Never)), // index past arity
            },
            // Shape not statically inferable — the operand is a genuine runtime handle whose element
            // kinds we cannot recover (a bare tuple PARAMETER `(def (fst t) (tuple.0 t))`, whose
            // shape is not threaded from the call site). Guessing `Kind::Heap` and emitting a bare
            // `arr-get` MISCOMPILES a SCALAR element: the accessor returns an i32 handle where the
            // element is a boxed Int64, and the function's inferred return kind then mismatches its
            // callers → an INVALID component (the worst outcome — neither a decline nor a valid
            // component). DECLINE instead, exactly as the record accessor `(. r f)` does for a record
            // parameter of unknown shape (05-compound-types.sexp §"projecting a tuple passed as a
            // function parameter yields the element, never an invalid component"). A shape we CAN
            // infer (a let-bound runtime tuple carrying a `Shape`, a nested compound whose shape is
            // known) still lowers below. NOTE: threading the parameter tuple's shape from the call
            // site (so this computes 7 rather than declining) is the fuller fix — ask-65.
            _ => return decline("tuple.N on a value of unknown tuple shape (parameter tuple)"),
        };
        let mut c = tc;
        c.push(op::I32_CONST);
        sleb128(idx as i64, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_GET as u64, &mut c);
        if let Some(unbox) = unbox_fn(elem_kind) {
            c.push(op::CALL);
            uleb128(unbox as u64, &mut c);
        }
        Ok((c, elem_kind))
    }

    /// Resolve a node to its compile-time structural form and the environment its free names
    /// resolve in. Follows aliases; returns None if the node is not a compile-time-known
    /// structure (a runtime scalar, an unbound name, etc.).
    fn resolve(&self, node: &Node, env: &[Local]) -> Option<(Node, Vec<Local>)> {
        match node {
            Node::Name(n) => {
                if let Some(local) = env.iter().rev().find(|l| l.name == *n) {
                    let (anode, aenv) = local.alias.as_ref()?;
                    // Follow the alias chain to a concrete structure.
                    return self.resolve(anode, aenv);
                }
                // A bare NULLARY constructor (`None`, `NNil`, `Zero`) resolves to the nullary sum
                // application `(Ctor unit)` — so a `match` on a bare nullary scrutinee sees the same
                // structural value as `(Ctor unit)` (its `((Ctor _) …)` arm matches). Not bound as a
                // local (checked above), so this is the constructor itself, not a shadowing binder.
                if self.nullary_variants.contains(variant_tag(n)) {
                    let app = Node::List(vec![Node::Name(n.clone()), Node::Name("unit".into())]);
                    return Some((app, env.to_vec()));
                }
                // A BUILT-IN MODULE name (`Bytes`, …) resolves to its module RECORD of built-in-
                // operation refs, so `(. Bytes len)` is the ordinary member-access projection of a
                // record (core-semantics.md §A Built-In Module Is A Record Of Its Operations, ask-58).
                // The projected value is a `(builtin <id>)` node — a first-class built-in operation
                // value. Not shadowed by a local (checked above), so a user binding of the same name
                // wins, exactly as for any binding. The APPLIED form `(Bytes.len args)` still lowers
                // through the existing dotted-application path (which matches the syntactic `(. Bytes
                // len)` head); this only makes the BARE projection resolve to a value instead of
                // declining "unsupported bare form".
                if let Some(rec) = builtin_module_record(n) {
                    return Some((rec, env.to_vec()));
                }
                None
            }
            // `(quote X)` / `(quasiquote X)` are AST-construction forms: expand to the
            // `Ast.*` constructor node they denote, so `match`/member/equality see an ordinary
            // structural value (metaprogramming.md §Quote Produces An AST Value).
            Node::List(items) if name_of(items.first()) == Some("quote") => {
                let ast = self.quote_to_ast(items.get(1)?, env, false)?;
                self.resolve(&ast, env)
            }
            Node::List(items) if name_of(items.first()) == Some("quasiquote") => {
                let ast = self.quote_to_ast(items.get(1)?, env, true)?;
                self.resolve(&ast, env)
            }
            // A function application whose callee resolves to a lambda — beta-reduce to the body
            // with the parameters bound to the arguments, then resolve THAT. So a structure
            // returned by a function is projectable: `((fn (x) (record (v x))) 7)` reduces to
            // `(record (v 7))`, exactly as `let`/`if`-selected records do. This covers a UNARY
            // call `(mk 7)` and — crucially — a NULLARY call `(mk)` (items.len()==1, params==[]):
            // a nullary function returning a compound must be projectable like a unary one
            // (09-functions.sexp §"projected from a … returned by a nullary function"). The head
            // `(mk)` must not be a special form (let/if/quote/etc.) — those own their resolution
            // above; `resolve_lambda` only resolves a lambda/named-def/member-projection callee.
            Node::List(items)
                if !items.is_empty()
                    && !is_special_form_head(items.first())
                    && self.resolve_lambda(&items[0], env).is_some() =>
            {
                if let Some((params, body, captured)) = self.resolve_lambda(&items[0], env) {
                    let args = &items[1..];
                    if args.len() == params.len() {
                        let mut body_env = captured;
                        for (p, a) in params.iter().zip(args) {
                            body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
                        }
                        return self.resolve(&body, &body_env);
                    }
                }
                Some((node.clone(), env.to_vec()))
            }
            Node::List(_) | Node::Str(_) => {
                match self.eval_const(node, env) {
                    // A form that constant-folds to an AST value (e.g. `(Ast.decode …)`)
                    // resolves to the `Ast.*` constructor node it denotes, so a `match` over it
                    // works.
                    Ok(Some(CVal::Ast(ast))) => {
                        if let Some(astn) = self.quote_to_ast(&ast, env, false) {
                            return Some((astn, env.to_vec()));
                        }
                    }
                    // A form that folds to a SUM value — a function/`if`-returned Option/Result
                    // whose variant is decided at compile time, e.g. `(parse 5)` reducing to
                    // `(Ok 42)` — resolves to its constructor node, so `match`/`try_match` see an
                    // ordinary `(Ctor payload)` form and select the arm. This is what lets a
                    // match dispatch on a sum returned by a called function (the last import-free
                    // gap): the callee is a pure, statically-applied function, so its result is a
                    // constant the compiler reduces through, exactly as it does for a `let`- or
                    // `if`-selected sum. (A recursive callee does NOT fold — `eval_const` never
                    // folds a named call — so a fuel-bounded recursion stays a runtime call.)
                    Ok(Some(v @ CVal::Sum { .. })) => {
                        if let Some(n) = cval_to_node(&v) {
                            return Some((n, env.to_vec()));
                        }
                    }
                    // A form that folds to a COMPOUND constant — a `(tuple …)`/`(list …)`/
                    // `(record …)` returned by a called function, e.g. `(get (Some (tuple 7 8)))`
                    // reducing through its `match` to `(tuple 7 8)` — resolves to that structural
                    // node so a `tuple.N`/member access/`match` over the CALL projects the element.
                    // Without it, `gen_tuple_access`'s structural path saw the un-reconstructed
                    // call/match node (not a `(tuple …)`), treated it as a definite trap, and emitted
                    // `unreachable` → the built-in-Option payload-through-return case ran to a TRAP on
                    // a valued program (05-compound-types.sexp §"a tuple payload returned through a
                    // helper from a built-in Option must not trap"). `cval_to_node` re-emits the
                    // canonical `(tuple …)` form, which re-folds to the SAME CVal, so the round-trip
                    // is exact (a non-representable leaf → None → fall through to echoing the node).
                    Ok(Some(v @ (CVal::Tuple(_) | CVal::List(_) | CVal::Record(_)))) => {
                        if let Some(n) = cval_to_node(&v) {
                            return Some((n, env.to_vec()));
                        }
                    }
                    _ => {}
                }
                Some((node.clone(), env.to_vec()))
            }
            _ => None,
        }
    }

    /// Produce the canonical quoted NODE — the AST `(quote X)`/`(quasiquote X)` denotes as an
    /// ordinary program tree (what `Ast.encode` should encode). `level` is the quasiquote
    /// nesting depth: a `quasiquote` increments it, an `unquote` decrements it, and an unquote
    /// is EVALUATED only when it brings the level to 0 (so `,,x` in `` ``… `` evaluates the
    /// inner unquote but leaves the outer one as a literal `(unquote …)` AST node). Called with
    /// `level=0` for plain `quote`, `level=1` for `quasiquote`.
    fn quote_node(&self, node: &Node, env: &[Local], level: u32) -> Option<Node> {
        match node {
            Node::List(items) => {
                if name_of(items.first()) == Some("unquote") {
                    let inner = items.get(1)?;
                    if level <= 1 {
                        // This unquote is active (brings level to 0): evaluate and embed.
                        match self.eval_const(inner, env) {
                            Ok(Some(CVal::Ast(n))) => return Some(n),
                            Ok(Some(v)) => return cval_to_node(&v),
                            // Could not fold to a value. An active unquote MUST evaluate its operand
                            // (metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation),
                            // so if the operand is an EXPRESSION that provably references an UNBOUND name
                            // (`,(+ b 1)` with `b` bound nowhere in `env`), it is the ordinary scope
                            // error — return None so the quasiquote DECLINES rather than FALLING BACK to
                            // structurally quoting the un-evaluable expression (which would turn the
                            // selective-evaluation unquote into a second quote and swallow the scope
                            // error, running `,(+ b 1)` to the inert AST `(Ast.List (Ast.Name "+") …)`;
                            // 12-metaprogramming.sexp §"an unquote of an expression with an unbound name
                            // is rejected, not quoted"). `provably_unbound_name` uses the emit-time `env`
                            // (so a `let`-bound `b` is seen as bound and this does NOT fire) and bails on
                            // any binder-introducing operand, so a legitimately-quotable operand (a bare
                            // symbol `,x`, a literal) still falls through to structural quoting below.
                            _ => {
                                if self.provably_unbound_name(inner, env).is_some() {
                                    return None;
                                }
                                return self.quote_node(inner, env, 0);
                            }
                        }
                    }
                    // A nested unquote (level>1): stays a literal `(unquote …)` node; its inner
                    // is quoted at one lower level.
                    return Some(Node::List(vec![
                        items[0].clone(),
                        self.quote_node(inner, env, level - 1)?,
                    ]));
                }
                // A nested quasiquote raises the level for its body.
                if name_of(items.first()) == Some("quasiquote") {
                    return Some(Node::List(vec![
                        items[0].clone(),
                        self.quote_node(items.get(1)?, env, level + 1)?,
                    ]));
                }
                let mut out = Vec::new();
                for child in items {
                    if level == 1 && child.head_name() == Some("unquote-splicing") {
                        if let Node::List(ci) = child {
                            let (v, venv) = self.resolve(ci.get(1)?, env)?;
                            if let Node::List(elems) = &v {
                                if name_of(elems.first()) == Some("list") {
                                    for e in &elems[1..] {
                                        out.push(self.quote_node(e, &venv, 0)?);
                                    }
                                    continue;
                                }
                            }
                            return None;
                        }
                    }
                    out.push(self.quote_node(child, env, level)?);
                }
                Some(Node::List(out))
            }
            other => Some(other.clone()),
        }
    }

    /// Convert a quoted node to the `Ast.*` constructor node form it denotes. With
    /// `quasi=true`, `(unquote e)` evaluates `e` (embedding its AST) and `(unquote-splicing e)`
    /// splices a list's elements into the parent `Ast.List`. Returns None if a construct is
    /// beyond compile-time quoting.
    fn quote_to_ast(&self, node: &Node, env: &[Local], quasi: bool) -> Option<Node> {
        let ctor = |variant: &str, payload: Node| {
            Node::List(vec![
                Node::List(vec![Node::Name(".".into()), Node::Name("Ast".into()), Node::Name(variant.into())]),
                payload,
            ])
        };
        match node {
            Node::Int(n) => Some(ctor("Int", Node::Int(*n))),
            Node::Float(f) => Some(ctor("Float", Node::Float(*f))),
            Node::Str(s) => Some(ctor("Str", Node::Str(s.clone()))),
            Node::Bool(b) => Some(ctor("Bool", Node::Bool(*b))),
            Node::Name(n) => Some(ctor("Name", Node::Str(n.clone()))),
            Node::List(items) => {
                // Under quasiquote, handle unquote / unquote-splicing.
                if quasi {
                    if name_of(items.first()) == Some("unquote") {
                        // Evaluate the unquoted expression to its AST value. In the corpus the
                        // unquoted value is itself a quotable literal/name, so quoting it (in
                        // non-quasi mode) yields its AST form.
                        return self.quote_to_ast(items.get(1)?, env, false).or_else(|| {
                            // Or it is a name bound to a value: resolve then quote.
                            let (v, venv) = self.resolve(items.get(1)?, env)?;
                            self.quote_to_ast(&v, &venv, false)
                        });
                    }
                }
                // Build (Ast.List (list <quoted children…>)), splicing where requested.
                let mut children = vec![Node::Name("list".into())];
                for child in items {
                    if quasi && child.head_name() == Some("unquote-splicing") {
                        // Splice: the unquoted value must be a list; embed each element quoted.
                        if let Node::List(ci) = child {
                            let (v, venv) = self.resolve(ci.get(1)?, env)?;
                            if let Node::List(elems) = &v {
                                if name_of(elems.first()) == Some("list") {
                                    for e in &elems[1..] {
                                        children.push(self.quote_to_ast(e, &venv, false)?);
                                    }
                                    continue;
                                }
                            }
                            return None;
                        }
                    }
                    children.push(self.quote_to_ast(child, env, quasi)?);
                }
                Some(ctor("List", Node::List(children)))
            }
        }
    }

    /// `((. obj field) arg)` — a prelude intrinsic applied. `Int.to-byte` / `Int64.to-byte`
    /// truncate to the low 8 bits: `arg & 255` (two's complement handles negatives).
    fn gen_dotted_apply(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let dparts = match elems.first() {
            Some(Node::List(d)) => d,
            _ => return decline("dotted-apply head"),
        };
        let obj = name_of(dparts.get(1));
        let field = name_of(dparts.get(2));
        if field == Some("to-byte") {
            if elems.len() != 2 {
                return decline("to-byte arity");
            }
            let (arg, ka) = self.emit(&elems[1], env, ctx)?;
            if ka != Kind::Int64 {
                return decline("to-byte of non-integer");
            }
            let mut c = arg;
            c.push(op::I64_CONST);
            sleb128(255, &mut c);
            c.push(op::I64_AND);
            return Ok((c, Kind::Int64));
        }
        // Runtime Bytes intrinsics — the compiler's own I/O type flowing at run time. Only
        // reachable when at least one operand is a genuine RUNTIME value (an all-constant one
        // folded to the baked-text path before this). Construction (`Bytes.of`/`Bytes.concat`/
        // `Bytes.compact`) returns a Bytes heap handle; consumption (`Bytes.len`/`Bytes.at`)
        // reads one. Bytes ops are frozen heap imports 13–16, so no envelope change is needed.
        match (obj, field) {
            (Some("Bytes"), Some("of")) => return self.gen_runtime_bytes_of(elems, env, ctx),
            (Some("Bytes"), Some("concat")) => return self.gen_runtime_bytes_concat(elems, env, ctx),
            (Some("Bytes"), Some("compact")) => return self.gen_runtime_bytes_compact(elems, env, ctx),
            (Some("Bytes"), Some("slice")) => return self.gen_runtime_bytes_slice(elems, env, ctx),
            (Some("Bytes"), Some("len")) => return self.gen_runtime_bytes_len(elems, env, ctx),
            (Some("Bytes"), Some("at")) => return self.gen_runtime_bytes_at(elems, env, ctx),
            // Runtime String ops — a String is a Bytes-backed UTF-8 heap leaf (indices 13–16), so
            // these lower against the SAME frozen heap imports as Bytes (no envelope change). Only
            // reachable when an operand is a genuine runtime String (an all-constant one folded
            // via `eval_const_dotted`). `byte-len` is the stored byte count; `to-bytes` reinterprets
            // the same handle as Bytes; `concat` joins two Strings; `scalar-len` counts UTF-8 scalar
            // starts. `from-bytes` validates a runtime Bytes as UTF-8 (the reader's symbol-table
            // decode). Scalar-indexed `String.at`/`String.slice` are not lowered at runtime yet
            // (decline — the corpus cases fold as constants); they index by scalar, for the reader's
            // scalar cursor, a later pass.
            (Some("String"), Some("byte-len")) => return self.gen_runtime_string_byte_len(elems, env, ctx),
            (Some("String"), Some("scalar-len")) => return self.gen_runtime_string_scalar_len(elems, env, ctx),
            (Some("String"), Some("to-bytes")) => return self.gen_runtime_string_to_bytes(elems, env, ctx),
            (Some("String"), Some("concat")) => return self.gen_runtime_string_concat(elems, env, ctx),
            (Some("String"), Some("from-bytes")) => return self.gen_runtime_string_from_bytes(elems, env, ctx),
            (Some("String"), Some("at")) => return self.gen_runtime_string_at(elems, env, ctx),
            (Some("String"), Some("slice")) => return self.gen_runtime_string_slice(elems, env, ctx),
            // A grown list — `List.push`/`List.update` are functional constructors producing a new
            // list value that leaves the operand unchanged (collections-and-text.md #A List Is Grown
            // By Functional Construction). A RUNTIME list is backed by the value-heap runtime's
            // 32-way radix trie (the only representation that grows; the flat array backs a
            // fixed-arity tuple/record). This is an unobservable representation choice
            // (#A List's Representation Is Unspecified And Unobservable): the value still renders
            // `(list …)`. `List.len` reads a scalar count; `List.at` is fallible (Option). An
            // all-constant `List.len`/`List.at` already folded via `eval_const_dotted` before here.
            (Some("List"), Some("push")) => return self.gen_runtime_list_push(elems, env, ctx),
            (Some("List"), Some("update")) => return self.gen_runtime_list_update(elems, env, ctx),
            (Some("List"), Some("len")) => return self.gen_runtime_list_len(elems, env, ctx),
            (Some("List"), Some("at")) => return self.gen_runtime_list_at(elems, env, ctx),
            (Some("List"), Some("concat")) => return self.gen_runtime_list_concat(elems, env, ctx),
            // Int64 WRAPPING arithmetic — the raw i64 op, which wasm wraps mod 2^64 (no overflow
            // check). Result Int64. Both operands Int64.
            (Some("Int64"), Some("wrapping-add")) => return self.gen_binop(elems, env, ctx, op::I64_ADD, Kind::Int64),
            (Some("Int64"), Some("wrapping-sub")) => return self.gen_binop(elems, env, ctx, op::I64_SUB, Kind::Int64),
            (Some("Int64"), Some("wrapping-mul")) => return self.gen_binop(elems, env, ctx, op::I64_MUL, Kind::Int64),
            // Int64 CHECKED arithmetic — build a runtime `Option<Int64>` (`(Some sum)`/`(None unit)`)
            // on the value-heap path. Overflow yields None instead of trapping.
            (Some("Int64"), Some("checked-add")) => return self.gen_int64_checked(elems, env, ctx, op::I64_ADD),
            (Some("Int64"), Some("checked-sub")) => return self.gen_int64_checked(elems, env, ctx, op::I64_SUB),
            (Some("Int64"), Some("checked-mul")) => return self.gen_int64_checked(elems, env, ctx, op::I64_MUL),
            // `Option.expect`/`Result.expect` on a RUNTIME Option/Result: unwrap the present variant's
            // payload or TRAP on absence (core-semantics.md §Requiring The Value Of An Optional Traps
            // On Absence). This is the `match ((Some v) v) (None <trap>)` the language already
            // compiles, as a built-in accessor — the runtime sum is read via `sum-disc`/`sum-payload`
            // exactly like a `match`. (An all-constant Option folds via `eval_const_dotted` before
            // here.) The overflow-TRAPPING companion of `Int64.checked-add`: `(Option.expect
            // (Int64.checked-add a b) "overflow")` is a non-trapping `+` made trapping.
            (Some("Option"), Some("expect")) => return self.gen_option_expect(elems, env, ctx, "Some"),
            (Some("Result"), Some("expect")) => return self.gen_option_expect(elems, env, ctx, "Ok"),
            _ => {}
        }
        decline("unsupported dotted-application")
    }

    /// Emit a runtime `(Bytes.at b i)`: a FALLIBLE index (collections-and-text.md #Indexing And
    /// Lookup Are Fallible, Not Trapping). In-bounds (`0 <= i < bytes-len(b)`) → `(Some byte)` with
    /// the byte boxed as an Int64; out-of-bounds or negative → `(None unit)`. Builds the Option as a
    /// runtime sum (Some=disc 0, None=disc 1 in the prelude's `(type Option (Some a | None))`),
    /// leaving the sum handle; `Kind::Heap`. The payload kind is statically KNOWN here (a byte is an
    /// Int64), so boxing it as an int is sound — this is not the polymorphic-payload case.
    fn gen_runtime_bytes_at(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("Bytes.at arity");
        }
        // Resolve Some/None discriminants from the Option sum type (do not hardcode — mirror
        // gen_runtime_sum's lookup so a prelude change stays correct).
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (bc, kb) = self.emit(&elems[1], env, ctx)?;
        if kb != Kind::Heap {
            return decline("Bytes.at of a non-Bytes value");
        }
        let (ic, ki) = self.emit(&elems[2], env, ctx)?;
        if ki != Kind::Int64 {
            return decline("Bytes.at index is not an integer");
        }
        let b = ctx.alloc_local(Kind::Bool); // i32 buffer handle
        let i = ctx.alloc_local(Kind::Int64); // the index (i64, Cadenza Int64)
        let mut c = Vec::new();
        c.extend_from_slice(&bc);
        c.push(op::LOCAL_SET);
        uleb128(b as u64, &mut c);
        c.extend_from_slice(&ic);
        c.push(op::LOCAL_SET);
        uleb128(i as u64, &mut c);
        // in_bounds = (i >= 0) & (i < bytes-len(b))  — bytes-len is i32, extend to i64 to compare.
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(op::I64_GE_S);
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(b as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u
        c.push(op::I64_LT_S);
        c.push(0x71); // i32.and — both comparisons are i32 results
        c.push(op::IF);
        c.push(0x7F); // block type i32 (a heap handle is produced by each arm)
        // then: Some(box-int(bytes-get(b, wrap(i))))
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(b as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(0xA7); // i32.wrap_i64 — bytes-get takes an i32 index
        c.push(op::CALL);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u — a byte 0..=255 as the Int64 payload
        c.push(op::CALL);
        uleb128(himport::BOX_INT as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: None(unit) — unit payload = arr-alloc(0)
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Runtime `Int64.checked-add/sub/mul` — the OVERFLOW-fallible companion of the trapping `+`/`-`/`*`.
    /// Computes `r = a <op> b` (wrapping), then, if that overflowed the signed Int64 range, yields
    /// `(None unit)`; otherwise `(Some r)` (numeric-model.md #Overflow Is Defined — a defined VALUE
    /// outcome, not a trap). Builds a runtime `Option<Int64>` on the value-heap path, exactly like
    /// `Bytes.at` (so it declines on the scalar path → runtime-mode retry). Overflow detection matches
    /// the `checked_*_body` trapping helpers: add=`(a^r)&(b^r)<0`, sub=`(a^b)&(a^r)<0`,
    /// mul=`a≠0 ∧ (a=-1 ? b=MIN : r/a≠b)` (the `a=-1` guard avoids `MIN/-1` trapping the check itself).
    fn gen_int64_checked(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
        opcode: u8,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime sum value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("Int64.checked arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (ac, ka) = self.emit(&elems[1], env, ctx)?;
        let (bc, kb) = self.emit(&elems[2], env, ctx)?;
        if ka != Kind::Int64 || kb != Kind::Int64 {
            return decline("Int64.checked operand is not an integer");
        }
        let a = ctx.alloc_local(Kind::Int64);
        let b = ctx.alloc_local(Kind::Int64);
        let r = ctx.alloc_local(Kind::Int64);
        const I64_GT_S: u8 = 0x55;
        let mut c = Vec::new();
        // a, b, r = a <op> b (wrapping — the raw i64 op).
        c.extend_from_slice(&ac);
        c.extend_from_slice(&[op::LOCAL_SET, a as u8]);
        c.extend_from_slice(&bc);
        c.extend_from_slice(&[op::LOCAL_SET, b as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, a as u8, op::LOCAL_GET, b as u8, opcode, op::LOCAL_SET, r as u8]);
        // Push the OVERFLOW flag (i32, 1 = overflowed) per op.
        let get = |x: u32, c: &mut Vec<u8>| { c.push(op::LOCAL_GET); uleb128(x as u64, c); };
        match opcode {
            op::I64_ADD => {
                // ((a^r)&(b^r)) < 0
                get(a, &mut c); get(r, &mut c); c.push(op::I64_XOR);
                get(b, &mut c); get(r, &mut c); c.push(op::I64_XOR);
                c.push(op::I64_AND);
                c.extend_from_slice(&[op::I64_CONST, 0, op::I64_LT_S]);
            }
            op::I64_SUB => {
                // ((a^b)&(a^r)) < 0
                get(a, &mut c); get(b, &mut c); c.push(op::I64_XOR);
                get(a, &mut c); get(r, &mut c); c.push(op::I64_XOR);
                c.push(op::I64_AND);
                c.extend_from_slice(&[op::I64_CONST, 0, op::I64_LT_S]);
            }
            _ /* I64_MUL */ => {
                // a==0 → no overflow; a==-1 → overflow iff b==MIN; else overflow iff r/a != b.
                // (The a==-1 arm avoids `MIN / -1`, which itself traps in wasm.)
                get(a, &mut c); c.push(op::I64_EQZ); // a == 0 ?
                c.extend_from_slice(&[op::IF, 0x7F]); // → i32 flag
                c.extend_from_slice(&[op::I32_CONST, 0]); // a==0: never overflows
                c.push(op::ELSE);
                get(a, &mut c); c.push(op::I64_CONST); sleb128(-1, &mut c); c.push(op::I64_EQ);
                c.extend_from_slice(&[op::IF, 0x7F]);
                // a == -1: overflow iff b == i64::MIN
                get(b, &mut c); c.push(op::I64_CONST); sleb128(i64::MIN, &mut c); c.push(op::I64_EQ);
                c.push(op::ELSE);
                // general: r / a != b
                get(r, &mut c); get(a, &mut c); c.push(op::I64_DIV_S); get(b, &mut c); c.push(op::I64_NE);
                c.push(op::END);
                c.push(op::END);
                let _ = I64_GT_S;
            }
        }
        // if overflow { None(unit) } else { Some(box-int(r)) }  — result kind i32 heap handle.
        c.extend_from_slice(&[op::IF, 0x7F]);
        // then: None
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.extend_from_slice(&[op::I32_CONST, 0, op::CALL]);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: Some(box-int(r))
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        get(r, &mut c);
        c.push(op::CALL);
        uleb128(himport::BOX_INT as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Runtime `Option.expect` / `Result.expect` — unwrap the present variant's payload, or TRAP on
    /// absence (core-semantics.md §Requiring The Value Of An Optional Traps On Absence). This is the
    /// `match ((Some v) v) ((None _) <trap>)` the language already compiles, emitted directly as an
    /// accessor: read `sum-disc(handle)`; if it is the PRESENT variant (`Some`/`Ok`, disc from the
    /// prelude's `(type Option (Some a | None))` / `(type Result (Ok a | Err e))`), yield its
    /// `sum-payload`; else `unreachable` (a defined trap — the absent case, whose custom message the
    /// scalar heap has no channel for yet, exactly like the const fold's `ConstTrap`).
    ///
    /// The result KIND is decided by `expect_payload_kind` — a PURELY SYNTACTIC classifier shared
    /// with inference (`infer_list`) so the two never disagree on the emitted function's signature.
    /// A concretely-Int Option producer (`Int64.checked-*`, `Bytes.at`) unboxes to `Int64` (so
    /// `(Option.expect (Int64.checked-add a b) …)` is a plain trapping Int64 add usable in
    /// arithmetic); every other scrutinee keeps the raw payload HANDLE (`Heap`), which the top-level
    /// renderer walks through the scrutinee's inferred payload shape (a boxed scalar's `Int` shape
    /// does the `get-int`, a compound payload renders structurally). Declines on the scalar path so
    /// the runtime-mode retry supplies the value-heap imports, like every other heap consumer.
    fn gen_option_expect(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
        present_tag: &str,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime sum value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("Option.expect arity");
        }
        let present_disc = self.variant_disc(present_tag)?;
        let (sc, ks) = self.emit(&elems[1], env, ctx)?;
        if ks != Kind::Heap {
            return decline("Option.expect of a non-Option value");
        }
        // The payload kind: the scalar to unbox to (`Int64` for a concretely-Int Option), else the
        // raw handle (`Heap`). SYNTACTIC — identical to what inference reports (below), so a caller
        // reads the same return kind the body emits.
        let payload_kind = expect_payload_kind(&elems[1]);
        let handle = ctx.alloc_local(Kind::Heap);
        let mut c = sc;
        c.push(op::LOCAL_SET);
        uleb128(handle as u64, &mut c);
        // if sum-disc(handle) == present_disc { <payload> } else { unreachable }
        c.push(op::LOCAL_GET);
        uleb128(handle as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_DISC as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(present_disc as i64, &mut c);
        c.push(0x46); // i32.eq
        c.push(op::IF);
        c.push(payload_kind.core_valtype());
        // then: sum-payload(handle), unboxed to the scalar kind (or kept as the raw handle).
        c.push(op::LOCAL_GET);
        uleb128(handle as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_PAYLOAD as u64, &mut c);
        if let Some(unbox) = unbox_fn(payload_kind) {
            c.push(op::CALL);
            uleb128(unbox as u64, &mut c);
        }
        c.push(op::ELSE);
        // else: absent → trap (a defined trap; the message channel is the const path's ConstTrap).
        c.push(op::UNREACHABLE);
        c.push(op::END);
        Ok((c, payload_kind))
    }

    /// The discriminant of a bare variant tag (e.g. `Some`/`None`) = its index in its sum type's
    /// declaration order. Mirrors `gen_runtime_sum`'s lookup; declines if the variant is unknown.
    fn variant_disc(&self, tag: &str) -> Result<u32, Decline> {
        let type_name = match self.sum_types.get(tag) {
            Some(t) => t,
            None => return decline(format!("unknown sum variant: {tag}")),
        };
        let order = match self.sum_variants.get(type_name) {
            Some(o) => o,
            None => return decline("sum type has no recorded variant order"),
        };
        match order.iter().position(|v| v == tag) {
            Some(idx) => Ok(idx as u32),
            None => decline("variant not in its sum type's order"),
        }
    }

    /// Emit a runtime `(Bytes.of (list b0 b1 …))` carrying at least one runtime byte value:
    /// `bytes-alloc(len)`, then for each element emit its Int64 value, RANGE-CHECK it (trap if
    /// outside 0..=255 — a byte range is bounded on both sides, 10-bytes.sexp), truncate to i32,
    /// and `bytes-set(buf, i, val)` threading the buffer handle. Leaves the buffer handle (i32) on
    /// the stack; result `Kind::Heap`. The range check preserves the corpus 256/-1 trap cases: the
    /// runtime's `bytes-set` truncates `value as u8`, so the trap MUST be emitted here, not left to
    /// the runtime (which would silently wrap -1 → 255).
    fn gen_runtime_bytes_of(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // On the SCALAR path (`call_base == 0`) the module imports no heap funcs, so a `bytes-*`
        // call would reference a non-existent import. Decline with a HEAP reason so `compile_module`
        // retries the whole pass in runtime mode (`call_base = RT_FUNC_BASE`, heap imports present)
        // — the same mechanism `gen_runtime_ctor`/`gen_runtime_sum` use. An all-constant Bytes
        // folds to the baked-text path before reaching here, so this only defers genuine runtime
        // Bytes construction to the runtime pass (decline-don't-miscompile).
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        // The argument must be a `(list …)` literal — the byte values are positional.
        let byte_nodes: Vec<Node> = match elems.get(1) {
            Some(Node::List(lst)) if name_of(lst.first()) == Some("list") => lst[1..].to_vec(),
            _ => return decline("Bytes.of expects a (list …) of byte values"),
        };
        let len = byte_nodes.len() as u32;
        let buf = ctx.alloc_local(Kind::Heap);
        let mut c = Vec::new();
        // buf = bytes-alloc(len)
        c.push(op::I32_CONST);
        sleb128(len as i64, &mut c);
        c.push(op::CALL);
        uleb128(himport::BYTES_ALLOC as u64, &mut c);
        c.push(op::LOCAL_SET);
        uleb128(buf as u64, &mut c);
        for (i, item) in byte_nodes.iter().enumerate() {
            let (ec, ek) = self.emit(item, env, ctx)?;
            if ek != Kind::Int64 {
                return decline("Bytes.of byte value is not an integer");
            }
            // bytes-set(buf, i, wrap_i64(range_checked(byte)))
            c.push(op::LOCAL_GET);
            uleb128(buf as u64, &mut c);
            c.push(op::I32_CONST);
            sleb128(i as i64, &mut c);
            // Range-check the i64 byte value: trap if < 0 or > 255. Stash it in a local so the
            // guard can read it twice without recomputing the (possibly effectful) element.
            let val = ctx.alloc_local(Kind::Int64);
            c.extend_from_slice(&ec);
            c.push(op::LOCAL_SET);
            uleb128(val as u64, &mut c);
            emit_byte_range_guard(val, &mut c);
            // wrap the (now in-range) i64 to i32 for bytes-set
            c.push(op::LOCAL_GET);
            uleb128(val as u64, &mut c);
            c.push(0xA7); // i32.wrap_i64
            c.push(op::CALL);
            uleb128(himport::BYTES_SET as u64, &mut c);
            c.push(op::DROP); // bytes-set returns the buffer handle; discard, we thread via `buf`
        }
        c.push(op::LOCAL_GET);
        uleb128(buf as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime STRING literal as a Bytes-backed UTF-8 heap leaf. A runtime `String` shares
    /// the runtime's `bytes-*` representation (indices 13–16): its stored payload is the string's
    /// UTF-8 bytes (already NFC — the reader normalizes at read time, ast.rs). The value is
    /// distinguished from a `Bytes` value ONLY by its static `Shape::Str`, which drives the renderer
    /// to quote/escape it as `"…"` rather than `b"…"`; at run time the two are the same heap object,
    /// so `str-new`/`str-get` (the `string`-typed WIT ops the envelope's canon cannot marshal) are
    /// never needed and NO envelope import is added (ignition stays byte-identical). This is the
    /// keystone that unblocks a Cadenza-authored compiler's name dispatch and symbol table
    /// (SEED-GAPS Tier 0). Leaves the buffer handle (i32) on the stack; result `Kind::Heap`.
    ///
    /// On the SCALAR path (`call_base == 0`) the module imports no heap funcs, so decline with a HEAP
    /// reason — `compile_module` retries the whole pass in runtime mode where the imports exist (the
    /// same mechanism `gen_runtime_bytes_of` uses). An all-constant string used only as a
    /// const-folded operand (`String.byte-len "hi"` in a scalar `main`) folds before reaching here,
    /// so this defers only a genuine runtime string value.
    fn gen_runtime_string_literal(&self, s: &str, ctx: &mut FnCtx) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        let bytes = s.as_bytes();
        let len = bytes.len() as u32;
        let buf = ctx.alloc_local(Kind::Heap);
        let mut c = Vec::new();
        // buf = bytes-alloc(len)
        c.push(op::I32_CONST);
        sleb128(len as i64, &mut c);
        c.push(op::CALL);
        uleb128(himport::BYTES_ALLOC as u64, &mut c);
        c.push(op::LOCAL_SET);
        uleb128(buf as u64, &mut c);
        for (i, b) in bytes.iter().enumerate() {
            // bytes-set(buf, i, byte) — a UTF-8 byte is statically 0..=255, no range guard needed.
            c.push(op::LOCAL_GET);
            uleb128(buf as u64, &mut c);
            c.push(op::I32_CONST);
            sleb128(i as i64, &mut c);
            c.push(op::I32_CONST);
            sleb128(*b as i64, &mut c);
            c.push(op::CALL);
            uleb128(himport::BYTES_SET as u64, &mut c);
            c.push(op::DROP); // bytes-set returns the buffer handle; we thread via `buf`
        }
        c.push(op::LOCAL_GET);
        uleb128(buf as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(String.byte-len s)`: the byte count of the String's UTF-8 encoding. A String
    /// is a Bytes-backed leaf, so this is `bytes-len(s)` extended to i64 — identical lowering to
    /// `Bytes.len`, agreeing with `(Bytes.len (String.to-bytes s))` (13-strings.sexp). Result Int64.
    fn gen_runtime_string_byte_len(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("String.byte-len arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("String.byte-len of a non-String value");
        }
        let mut c = a;
        c.push(op::CALL);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u
        Ok((c, Kind::Int64))
    }

    /// Emit a runtime `(String.to-bytes s)`: the UTF-8 bytes of the String. A String IS a Bytes leaf
    /// at run time (the `Shape` is the only difference), so this is the identity on the handle —
    /// the result is the same heap object, reinterpreted as `Bytes`. Result `Kind::Heap`; the caller
    /// treats it as Bytes (its `Shape` comes from `shape_of`, which reports `Bytes` here). The String
    /// value is CONSUMED (its handle becomes the Bytes value), matching the RC calling convention.
    fn gen_runtime_string_to_bytes(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("String.to-bytes arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("String.to-bytes of a non-String value");
        }
        // Identity: the String's underlying bytes ARE the result Bytes value.
        Ok((a, Kind::Heap))
    }

    /// Emit a runtime `(String.concat a b)`: a single native `bytes-concat` — a String is a
    /// Bytes-backed leaf, so concatenating two Strings is concatenating their UTF-8 bytes (the result
    /// is well-formed UTF-8 since each operand is). O(1) rope node, consumes both operands, result
    /// `Kind::Heap` (a String). Renders `"…"` via `Shape::Str`.
    fn gen_runtime_string_concat(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("String.concat arity");
        }
        let (ac, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("String.concat of a non-String left operand");
        }
        let (bc, kb) = self.emit(&elems[2], env, ctx)?;
        if kb != Kind::Heap {
            return decline("String.concat of a non-String right operand");
        }
        let mut c = ac;
        c.extend_from_slice(&bc);
        c.push(op::CALL);
        uleb128(himport::BYTES_CONCAT as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(String.from-bytes b)`: a TOTAL, fallible UTF-8 decode
    /// (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). Validates the
    /// runtime Bytes `b` as well-formed UTF-8; well-formed → `(Some s)` (the String IS the same
    /// Bytes-backed leaf — no copy, the handle is reinterpreted as a String), ill-formed → `(None
    /// unit)`. NEVER traps — the ill-formed case is an ordinary value the reader handles. The
    /// validation is emitted inline (`emit_utf8_valid` — a byte-level state machine over the buffer,
    /// no new runtime op / envelope change). Result `Kind::Heap` (an `Option<String>` sum). This is
    /// the reader's symbol-table decode: input bytes → a name String.
    fn gen_runtime_string_from_bytes(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("String.from-bytes arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (bc, kb) = self.emit(&elems[1], env, ctx)?;
        if kb != Kind::Heap {
            return decline("String.from-bytes of a non-Bytes value");
        }
        // A UTF-8 validator needs several DISTINCT i32 scratch locals — reuse is what makes a
        // hand-emitted validator subtly wrong. All i32 (declared `Kind::Bool`, whose core valtype
        // is i32).
        let v = Utf8Locals {
            buf: ctx.alloc_local(Kind::Bool),
            n: ctx.alloc_local(Kind::Bool),
            len: ctx.alloc_local(Kind::Bool),
            lead: ctx.alloc_local(Kind::Bool),
            seq: ctx.alloc_local(Kind::Bool),
            k: ctx.alloc_local(Kind::Bool),
            cb: ctx.alloc_local(Kind::Bool),
            lo: ctx.alloc_local(Kind::Bool),
            hi: ctx.alloc_local(Kind::Bool),
            valid: ctx.alloc_local(Kind::Bool),
        };
        let buf = v.buf;
        let valid = v.valid;
        let mut c = bc;
        c.push(op::LOCAL_SET);
        uleb128(buf as u64, &mut c);
        emit_utf8_valid(&v, &mut c);
        // if valid { Some(buf-as-string) } else { None(unit) }
        c.push(op::LOCAL_GET);
        uleb128(valid as u64, &mut c);
        c.push(op::IF);
        c.push(0x7F); // block type i32 (a heap handle from each arm)
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(buf as u64, &mut c); // the String's payload IS the validated Bytes leaf
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(String.scalar-len s)`: the count of Unicode scalar values. A String is a
    /// Bytes-backed UTF-8 leaf, so a scalar's count is the number of bytes that are NOT UTF-8
    /// continuation bytes — a continuation byte has its top two bits `10`, i.e. `(b & 0xC0) == 0x80`.
    /// Loop over the bytes, incrementing the count for each non-continuation (leading) byte. This
    /// agrees with the const `chars().count()` because the String is well-formed UTF-8. Result Int64.
    fn gen_runtime_string_scalar_len(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("String.scalar-len arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("String.scalar-len of a non-String value");
        }
        let s = ctx.alloc_local(Kind::Bool); // i32 buffer handle
        let n = ctx.alloc_local(Kind::Bool); // i32 loop index
        let cnt = ctx.alloc_local(Kind::Int64); // i64 scalar count (Cadenza Int64)
        const I32_AND: u8 = 0x71;
        const I32_NE: u8 = 0x47;
        let mut c = a;
        c.push(op::LOCAL_SET);
        uleb128(s as u64, &mut c);
        // n = 0 ; cnt = 0
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(op::LOCAL_SET);
        uleb128(cnt as u64, &mut c);
        // block { loop { if n >= bytes-len(s) break; if (bytes-get(s,n) & 0xC0) != 0x80 cnt+=1; n+=1 } }
        c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
        // n >= bytes-len(s) → br 1
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, s as u8, op::CALL]);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.extend_from_slice(&[0x4E /*i32.ge_s*/, op::BR_IF, 1]);
        // if (bytes-get(s, n) & 0xC0) != 0x80 { cnt += 1 }
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::LOCAL_GET, n as u8, op::CALL]);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0xC0, &mut c);
        c.push(I32_AND);
        c.push(op::I32_CONST);
        sleb128(0x80, &mut c);
        c.extend_from_slice(&[I32_NE, op::IF, 0x40]);
        // cnt += 1
        c.push(op::LOCAL_GET);
        uleb128(cnt as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(1, &mut c);
        c.push(op::I64_ADD);
        c.push(op::LOCAL_SET);
        uleb128(cnt as u64, &mut c);
        c.push(op::END); // if
        // n += 1 ; continue
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
        c.extend_from_slice(&[op::END, op::END]); // loop, block
        c.push(op::LOCAL_GET);
        uleb128(cnt as u64, &mut c);
        Ok((c, Kind::Int64))
    }

    /// Emit a runtime `(String.at s i)`: a FALLIBLE SCALAR index (collections-and-text.md #A String's
    /// Scalars Are Addressable; #Indexing And Lookup Are Fallible, Not Trapping). Indexes by Unicode
    /// SCALAR (not byte): the i-th scalar in bounds → `(Some "<one-scalar substring>")`, out-of-bounds
    /// / negative → `(None unit)`. A runtime String is a UTF-8 Bytes leaf, so a scalar's byte span is
    /// [start, stop) where `start` is the byte offset of scalar `i` and `stop` that of scalar `i+1`
    /// (or the buffer end); the one-scalar String is `bytes-slice(s, start, stop-start)`. Byte offsets
    /// are found by scanning: a scalar STARTS at a byte where `(b & 0xC0) != 0x80` (a non-continuation
    /// byte), the same predicate `scalar-len` counts. Result `Kind::Heap` (an `Option<String>` sum);
    /// the `Some` payload renders `"…"` via `Shape::Str`. This is the reader's scalar cursor.
    fn gen_runtime_string_at(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("String.at arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (sc, ks) = self.emit(&elems[1], env, ctx)?;
        if ks != Kind::Heap {
            return decline("String.at of a non-String value");
        }
        let (ic, ki) = self.emit(&elems[2], env, ctx)?;
        if ki != Kind::Int64 {
            return decline("String.at index is not an integer");
        }
        // i32 scratch locals (all `Kind::Bool` = i32 core valtype).
        let s = ctx.alloc_local(Kind::Bool);      // buffer handle
        let tgt = ctx.alloc_local(Kind::Bool);     // target scalar index (i32; a negative i64 → sentinel len)
        let n = ctx.alloc_local(Kind::Bool);       // byte scan index
        let len = ctx.alloc_local(Kind::Bool);     // buffer byte length
        let sc_idx = ctx.alloc_local(Kind::Bool);  // scalars seen so far
        let start = ctx.alloc_local(Kind::Bool);   // byte offset of scalar `tgt` (len if not found)
        let stop = ctx.alloc_local(Kind::Bool);    // byte offset of scalar `tgt+1` (len if last)
        let done = ctx.alloc_local(Kind::Bool);    // 1 once `stop` is fixed (found the next scalar start)
        const AND: u8 = 0x71;
        const NE: u8 = 0x47;
        const GE_S: u8 = 0x4E;
        const EQ: u8 = 0x46;
        let mut c = sc;
        c.push(op::LOCAL_SET);
        uleb128(s as u64, &mut c);
        // tgt = (i64 index); if i < 0 or i > i32::MAX use a value >= len so it is out of bounds.
        // Wrap to i32 after clamping negatives to a large sentinel (len is the natural miss marker; we
        // set start=stop=len initially so a never-found target yields an empty [len,len) → but we must
        // return None for out-of-bounds, so track "found" via start<len).
        c.extend_from_slice(&ic);
        // if i < 0 { tgt = -1 } else { tgt = wrap_i64(i) }  — a negative or huge index never matches.
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(0x59); // i64.ge_s — the index is an i64 (Cadenza Int64), so compare as i64, not i32
        c.extend_from_slice(&[op::IF, 0x7F]);
        c.extend_from_slice(&ic);
        c.push(0xA7); // i32.wrap_i64
        c.push(op::ELSE);
        c.push(op::I32_CONST);
        sleb128(-1, &mut c); // sentinel: never equals a real scalar index (0..)
        c.extend_from_slice(&[op::END, op::LOCAL_SET, tgt as u8]);
        // len = bytes-len(s) ; start = len ; stop = len ; sc_idx = 0 ; n = 0
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::CALL]);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.extend_from_slice(&[op::LOCAL_SET, len as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::LOCAL_SET, start as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::LOCAL_SET, stop as u8]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, sc_idx as u8]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, done as u8]);
        // Scan bytes, GUARDED loop (no multi-level `br` — a running `done` flag exits cleanly): at
        // each scalar-start byte, if sc_idx==tgt set start=n; if sc_idx==tgt+1 set stop=n & done=1.
        // Loop while (n < len) & !done.
        c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
        // (n >= len) | done → br 1 (exit)
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, len as u8, GE_S]);
        c.extend_from_slice(&[op::LOCAL_GET, done as u8, 0x72 /*i32.or*/, op::BR_IF, 1]);
        // is this byte a scalar start? (bytes-get(s,n) & 0xC0) != 0x80
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::LOCAL_GET, n as u8, op::CALL]);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0xC0, &mut c);
        c.push(AND);
        c.push(op::I32_CONST);
        sleb128(0x80, &mut c);
        c.extend_from_slice(&[NE, op::IF, 0x40]);
        //   if sc_idx == tgt { start = n }
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::LOCAL_GET, tgt as u8, EQ, op::IF, 0x40]);
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_SET, start as u8, op::END]);
        //   if sc_idx == tgt+1 { stop = n ; done = 1 }  (a start already recorded; this bounds it)
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::LOCAL_GET, tgt as u8, op::I32_CONST, 1, op::I32_ADD, EQ, op::IF, 0x40]);
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_SET, stop as u8]);
        c.extend_from_slice(&[op::I32_CONST, 1, op::LOCAL_SET, done as u8, op::END]);
        //   sc_idx += 1  (only counted for a scalar-start byte)
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, sc_idx as u8]);
        c.push(op::END); // if scalar-start
        // n += 1 ; continue (only when not done — the guard re-checks)
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
        c.extend_from_slice(&[op::END, op::END]); // loop, block
        // in_bounds = start < len (the target scalar was found)
        c.extend_from_slice(&[op::LOCAL_GET, start as u8, op::LOCAL_GET, len as u8, 0x48 /*i32.lt_s*/]);
        c.extend_from_slice(&[op::IF, 0x7F]);
        // then: Some(bytes-slice(s, start, stop - start))
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::LOCAL_GET, start as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, stop as u8, op::LOCAL_GET, start as u8, op::I32_SUB]);
        c.push(op::CALL);
        uleb128(himport::BYTES_SLICE as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: None(unit)
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(String.slice s a b)`: a FALLIBLE sub-string over SCALAR offsets `[a, b)`
    /// (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping — String offsets are
    /// Unicode scalar positions, NOT byte offsets, distinct from `Bytes.slice`'s `(start, length)`).
    /// A valid range `0 <= a <= b <= scalar-count` → `(Some "<sub>")` (an empty range `a==b` is Some of
    /// the empty string, present not absent); an out-of-range or inverted range → `(None unit)`. The
    /// runtime String is a Bytes-backed UTF-8 leaf, so this scans the bytes ONCE (a guarded loop, no
    /// multi-level `br`) to find the BYTE offset of scalar `a` (`bstart`) and scalar `b` (`bstop`) — a
    /// scalar-start byte is `(byte & 0xC0) != 0x80` — then slices that byte range via the native
    /// `bytes-slice` (a rope slice node, O(1), copies no bytes) and boxes it as `Some`. The scalar
    /// count is also tallied so a boundary index EQUAL to the count maps to `bytes-len` (the end), and
    /// the validity `a <= b <= count` is checked. Mirrors `gen_runtime_string_at`'s UTF-8 walk +
    /// `gen_runtime_bytes_slice`'s Option build. Result `Kind::Heap` (an Option sum).
    fn gen_runtime_string_slice(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime string value needs the value-heap runtime");
        }
        if elems.len() != 4 {
            return decline("String.slice arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (sc, ks) = self.emit(&elems[1], env, ctx)?;
        if ks != Kind::Heap {
            return decline("String.slice of a non-String value");
        }
        let (ac, ka) = self.emit(&elems[2], env, ctx)?;
        if ka != Kind::Int64 {
            return decline("String.slice start is not an integer");
        }
        let (bc, kb) = self.emit(&elems[3], env, ctx)?;
        if kb != Kind::Int64 {
            return decline("String.slice end is not an integer");
        }
        // i32 scratch locals (all `Kind::Bool` = i32 core valtype), like `gen_runtime_string_at`.
        let s = ctx.alloc_local(Kind::Bool);       // buffer handle
        let a = ctx.alloc_local(Kind::Bool);       // scalar start index (i32; <0 sentinel = out of bounds)
        let b = ctx.alloc_local(Kind::Bool);       // scalar end index (i32; <0 sentinel = out of bounds)
        let n = ctx.alloc_local(Kind::Bool);       // byte scan index
        let len = ctx.alloc_local(Kind::Bool);     // buffer byte length
        let sc_idx = ctx.alloc_local(Kind::Bool);  // scalars seen so far (at scalar-start bytes)
        let bstart = ctx.alloc_local(Kind::Bool);  // byte offset of scalar `a` (len sentinel until set)
        let bstop = ctx.alloc_local(Kind::Bool);   // byte offset of scalar `b` (len sentinel until set)
        const AND: u8 = 0x71;
        const OR: u8 = 0x72;
        const NE: u8 = 0x47;
        const EQ: u8 = 0x46;
        const GE_S: u8 = 0x4E;
        const GT_S: u8 = 0x4A;
        const LT_S: u8 = 0x48;
        const ADD: u8 = 0x6A;
        let clamp_neg = |code: &mut Vec<u8>, ic: &[u8], slot: u32| {
            // if idx >= 0 { wrap_i64(idx) } else { -1 }  — a negative index is a definite miss.
            code.extend_from_slice(ic);
            code.push(op::I64_CONST);
            sleb128(0, code);
            code.push(0x59); // i64.ge_s
            code.extend_from_slice(&[op::IF, 0x7F]);
            code.extend_from_slice(ic);
            code.push(0xA7); // i32.wrap_i64
            code.push(op::ELSE);
            code.push(op::I32_CONST);
            sleb128(-1, code);
            code.extend_from_slice(&[op::END, op::LOCAL_SET, slot as u8]);
        };
        let mut c = sc;
        c.push(op::LOCAL_SET);
        uleb128(s as u64, &mut c);
        clamp_neg(&mut c, &ac, a);
        clamp_neg(&mut c, &bc, b);
        // len = bytes-len(s) ; bstart = len ; bstop = len ; sc_idx = 0 ; n = 0
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::CALL]);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.extend_from_slice(&[op::LOCAL_SET, len as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::LOCAL_SET, bstart as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::LOCAL_SET, bstop as u8]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, sc_idx as u8]);
        c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
        // Scan every byte to the end (we need the TOTAL scalar count to validate `b`, so no early
        // exit): at each scalar-start byte, if sc_idx==a set bstart=n; if sc_idx==b set bstop=n; then
        // sc_idx += 1. After the loop sc_idx = total scalar count; a boundary equal to the count keeps
        // its `len` sentinel (the end offset), which is exactly right.
        c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
        // (n >= len) → br 1 (exit)
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, len as u8, GE_S, op::BR_IF, 1]);
        // is this byte a scalar start? (bytes-get(s,n) & 0xC0) != 0x80
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::LOCAL_GET, n as u8, op::CALL]);
        uleb128(himport::BYTES_GET as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0xC0, &mut c);
        c.push(AND);
        c.push(op::I32_CONST);
        sleb128(0x80, &mut c);
        c.extend_from_slice(&[NE, op::IF, 0x40]);
        //   if sc_idx == a { bstart = n }
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::LOCAL_GET, a as u8, EQ, op::IF, 0x40]);
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_SET, bstart as u8, op::END]);
        //   if sc_idx == b { bstop = n }
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::LOCAL_GET, b as u8, EQ, op::IF, 0x40]);
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_SET, bstop as u8, op::END]);
        //   sc_idx += 1
        c.extend_from_slice(&[op::LOCAL_GET, sc_idx as u8, op::I32_CONST, 1, ADD, op::LOCAL_SET, sc_idx as u8]);
        c.push(op::END); // if scalar-start
        // n += 1 ; continue
        c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
        c.extend_from_slice(&[op::END, op::END]); // loop, block
        // in_bounds = (a >= 0) & (b >= 0) & (a <= b) & (b <= sc_idx)  — sc_idx is now the scalar count.
        // (a,b already clamped: a negative arg became -1, which fails a>=0.)
        c.extend_from_slice(&[op::LOCAL_GET, a as u8, op::I32_CONST, 0, GE_S]);
        c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::I32_CONST, 0, GE_S, AND]);
        c.extend_from_slice(&[op::LOCAL_GET, a as u8, op::LOCAL_GET, b as u8]);
        c.push(LT_S); // a < b
        c.extend_from_slice(&[op::LOCAL_GET, a as u8, op::LOCAL_GET, b as u8, EQ]);
        c.push(OR); // (a < b) | (a == b)  ⇒ a <= b
        c.push(AND);
        c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::LOCAL_GET, sc_idx as u8, GT_S]);
        c.push(0x45); // i32.eqz — !(b > count) ⇒ b <= count
        c.push(AND);
        c.extend_from_slice(&[op::IF, 0x7F]);
        // then: Some(bytes-slice(s, bstart, bstop - bstart))
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::LOCAL_GET, bstart as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, bstop as u8, op::LOCAL_GET, bstart as u8, op::I32_SUB]);
        c.push(op::CALL);
        uleb128(himport::BYTES_SLICE as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: None(unit). `bytes-slice` was NOT called, so `s` is still owned — drop it to honour
        // the consume contract on the whole `String.slice` (the caller handed ownership of `s`).
        c.extend_from_slice(&[op::LOCAL_GET, s as u8, op::CALL]);
        uleb128(himport::DROP as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.extend_from_slice(&[op::I32_CONST, 0, op::CALL]);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(Bytes.concat a b)`: a single native `bytes-concat` call. The runtime's rope
    /// representation makes this O(1) — it allocates ONE concat node over the two shared byte leaves
    /// and copies NO bytes (DESIGN-rope-bytes / RUNTIME-REQUESTS Request 1), killing the O(n²) copy
    /// cascade a self-hosting compiler would hit assembling a wasm module by concatenating encoded
    /// sections. `bytes-concat` CONSUMES both operands (they become the node's children without a
    /// `dup`, the RC calling convention) and returns a new owned Bytes handle. Leaves the handle on
    /// the stack; `Kind::Heap`. The bytes of `a` then `b`; the empty sequence is the identity.
    fn gen_runtime_bytes_concat(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("Bytes.concat arity");
        }
        let (ac, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("Bytes.concat of a non-Bytes left operand");
        }
        let (bc, kb) = self.emit(&elems[2], env, ctx)?;
        if kb != Kind::Heap {
            return decline("Bytes.concat of a non-Bytes right operand");
        }
        // bytes-concat(a, b) — both operands already on the stack in order.
        let mut c = ac;
        c.extend_from_slice(&bc);
        c.push(op::CALL);
        uleb128(himport::BYTES_CONCAT as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(Bytes.len b)`: `bytes-len(b)` extended to i64 (the Cadenza length is an
    /// Int64). Result `Kind::Int64` — a SCALAR, so this reaches the runtime-scalar component path.
    fn gen_runtime_bytes_len(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("Bytes.len arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("Bytes.len of a non-Bytes value");
        }
        let mut c = a;
        c.push(op::CALL);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u — length is an unsigned count
        Ok((c, Kind::Int64))
    }

    /// Emit a runtime `(Bytes.compact b)`: a single native `bytes-compact` call. `compact` is
    /// value-preserving — it returns a Bytes equal by content whose storage is INDEPENDENT of any
    /// larger buffer `b` was sliced from (memory-and-resource-model.md #Retained Storage Is
    /// Accounted For What It Holds Live), so a program can drop a large parent while keeping a small
    /// slice. On the rope runtime this flattens the node to a fresh leaf; the VALUE is unchanged
    /// (observable only through the resource measure). CONSUMES its operand, returns a new owned
    /// Bytes handle; `Kind::Heap`. (Previously a no-op identity — a compiler bug: the identity kept
    /// the parent pinned, defeating the entire point of compact. Native `bytes-compact` releases it.)
    fn gen_runtime_bytes_compact(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("Bytes.compact arity");
        }
        let (a, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("Bytes.compact of a non-Bytes value");
        }
        let mut c = a;
        c.push(op::CALL);
        uleb128(himport::BYTES_COMPACT as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime `(Bytes.slice b start length)`: a FALLIBLE range read (collections-and-text.md
    /// #Indexing And Lookup Are Fallible, Not Trapping). A valid range yields `(Some slice)`, a
    /// negative start/length or a `start + length` running past the end yields `(None unit)` — NEVER
    /// a trap and never a short result. The in-bounds case calls the native `bytes-slice` (a rope
    /// slice node over the shared parent leaf — O(1), copies no bytes), whose result is a Bytes
    /// (Heap) payload; the Option is built as a runtime sum exactly like `gen_runtime_bytes_at`
    /// (Some=disc 0, None=disc 1 from the prelude's Option). `bytes-slice` CONSUMES `b`, so the
    /// bounds must be tested BEFORE the call using a `bytes-len(b)` read; in the out-of-bounds arm
    /// `b` is not consumed by slice, so it is `drop`ped to honour the consume contract on the whole
    /// operation (the caller handed ownership of `b` to `Bytes.slice`). Result `Kind::Heap`.
    fn gen_runtime_bytes_slice(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime bytes value needs the value-heap runtime");
        }
        if elems.len() != 4 {
            return decline("Bytes.slice arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (bc, kb) = self.emit(&elems[1], env, ctx)?;
        if kb != Kind::Heap {
            return decline("Bytes.slice of a non-Bytes value");
        }
        let (sc, ks) = self.emit(&elems[2], env, ctx)?;
        if ks != Kind::Int64 {
            return decline("Bytes.slice start is not an integer");
        }
        let (lc, kl) = self.emit(&elems[3], env, ctx)?;
        if kl != Kind::Int64 {
            return decline("Bytes.slice length is not an integer");
        }
        let b = ctx.alloc_local(Kind::Bool); // i32 buffer handle
        let start = ctx.alloc_local(Kind::Int64); // i64 start (Cadenza Int64)
        let len = ctx.alloc_local(Kind::Int64); // i64 length
        let mut c = Vec::new();
        c.extend_from_slice(&bc);
        c.push(op::LOCAL_SET);
        uleb128(b as u64, &mut c);
        c.extend_from_slice(&sc);
        c.push(op::LOCAL_SET);
        uleb128(start as u64, &mut c);
        c.extend_from_slice(&lc);
        c.push(op::LOCAL_SET);
        uleb128(len as u64, &mut c);
        // in_bounds = (start >= 0) & (len >= 0) & (start + len <= bytes-len(b)).
        // All arithmetic in i64 to avoid a negative start wrapping to a large unsigned offset.
        c.push(op::LOCAL_GET);
        uleb128(start as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(op::I64_GE_S);
        c.push(op::LOCAL_GET);
        uleb128(len as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(op::I64_GE_S);
        c.push(0x71); // i32.and — (start>=0) & (len>=0)
        c.push(op::LOCAL_GET);
        uleb128(start as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(len as u64, &mut c);
        c.push(0x7C); // i64.add — start + len (both proven non-negative in the taken arm)
        c.push(op::LOCAL_GET);
        uleb128(b as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::BYTES_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u — bytes-len is i32
        c.push(op::I64_LE_S); // (start + len) <= bytes-len(b)
        c.push(0x71); // i32.and — combine with the non-negativity test
        c.push(op::IF);
        c.push(0x7F); // block type i32 (a heap handle is produced by each arm)
        // then: Some(bytes-slice(b, wrap(start), wrap(len)))
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(b as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(start as u64, &mut c);
        c.push(0xA7); // i32.wrap_i64 — bytes-slice takes i32 start
        c.push(op::LOCAL_GET);
        uleb128(len as u64, &mut c);
        c.push(0xA7); // i32.wrap_i64 — bytes-slice takes i32 len
        c.push(op::CALL);
        uleb128(himport::BYTES_SLICE as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: None(unit). `bytes-slice` was NOT called, so `b` is still live and owned here — the
        // whole `Bytes.slice` was handed ownership of `b`, so drop it to honour the consume contract.
        c.push(op::LOCAL_GET);
        uleb128(b as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::DROP as u64, &mut c);
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    // ── List: a growable, structurally-shared runtime sequence (heap imports 24–28) ─────────────
    // A runtime list is the value-heap runtime's 32-way radix trie (`vec-*`) — the representation
    // that supports functional growth as well as reading. A `(list …)` literal and a `List.push`-
    // grown list are ONE type over ONE representation; the trie is an unobservable implementation
    // detail (collections-and-text.md #A List's Representation Is Unspecified And Unobservable), so
    // both render `(list …)`. An element crosses as a boxed heap handle (`box_scalar`); a Heap
    // element is already a handle. Every op declines on the SCALAR path (`call_base == 0`) with a
    // HEAP reason so `compile_module` retries in runtime mode — the same mechanism the arr/sum/bytes
    // constructors use. (The flat `arr-*` array backs a fixed-arity tuple/record, not a list.)

    /// A runtime `(list e0 e1 …)` literal carrying a runtime element: `vec-empty` then a `vec-push`
    /// per element (each boxed per its scalar kind), leaving the list handle. `Kind::Heap`. This is
    /// the SAME representation `List.push` grows, so a literal and a grown list are read alike.
    fn gen_runtime_list_literal(
        &self,
        elem_nodes: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        let mut c = Vec::new();
        // acc = vec-empty()
        c.push(op::CALL);
        uleb128(himport::VEC_EMPTY as u64, &mut c);
        // acc = vec-push(acc, box(elem)) for each element, in order.
        for item in elem_nodes {
            let (ec, ek) = self.emit(item, env, ctx)?;
            let box_fn = self.box_scalar(ek)?;
            c.extend_from_slice(&ec);
            if box_fn != u32::MAX {
                c.push(op::CALL);
                uleb128(box_fn as u64, &mut c);
            }
            c.push(op::CALL);
            uleb128(himport::VEC_PUSH as u64, &mut c);
        }
        Ok((c, Kind::Heap))
    }

    /// `(List.push v elem)` → a NEW list = `v` with `elem` appended (`vec-push`, consumes both),
    /// leaving `v` unchanged (collections-and-text.md #A List Is Grown By Functional Construction).
    /// `Kind::Heap`. The element is boxed per its scalar kind (a Heap element is already a handle).
    fn gen_runtime_list_push(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("List.push arity");
        }
        let (vc, kv) = self.emit(&elems[1], env, ctx)?;
        if kv != Kind::Heap {
            return decline("List.push of a non-list value");
        }
        let (ec, ek) = self.emit(&elems[2], env, ctx)?;
        let box_fn = self.box_scalar(ek)?;
        let mut c = vc;
        c.extend_from_slice(&ec);
        if box_fn != u32::MAX {
            c.push(op::CALL);
            uleb128(box_fn as u64, &mut c);
        }
        c.push(op::CALL);
        uleb128(himport::VEC_PUSH as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// `(List.update v i elem)` → a NEW list = `v` with index `i` set to `elem` (`vec-update`,
    /// consumes both), `Kind::Heap`. The replace-at-index is defined only in bounds
    /// (collections-and-text.md #A List Is Grown By Functional Construction); an out-of-bounds index
    /// traps in the runtime (the compiler emits no bounds check — fail-fast like `arr-get`).
    fn gen_runtime_list_update(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        if elems.len() != 4 {
            return decline("List.update arity");
        }
        let (vc, kv) = self.emit(&elems[1], env, ctx)?;
        if kv != Kind::Heap {
            return decline("List.update of a non-list value");
        }
        let (ic, ki) = self.emit(&elems[2], env, ctx)?;
        if ki != Kind::Int64 {
            return decline("List.update index is not an integer");
        }
        let (ec, ek) = self.emit(&elems[3], env, ctx)?;
        let box_fn = self.box_scalar(ek)?;
        let mut c = vc;
        c.extend_from_slice(&ic);
        c.push(0xA7); // i32.wrap_i64 — vec-update takes an i32 index
        c.extend_from_slice(&ec);
        if box_fn != u32::MAX {
            c.push(op::CALL);
            uleb128(box_fn as u64, &mut c);
        }
        c.push(op::CALL);
        uleb128(himport::VEC_UPDATE as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// `(List.concat a b)` → a NEW list = the elements of `a` followed by those of `b` (`vec-concat`,
    /// consumes both), `Kind::Heap`. Both operands are runtime lists (Heap handles). The RRB-trie
    /// concatenation is O(log N) and its result is INDISTINGUISHABLE from a push-built list by
    /// `vec-len`/`vec-get` (collections-and-text.md #A List's Representation Is Unspecified And
    /// Unobservable), so a concatenated list renders and reads exactly like a literal. This is the op
    /// a self-hosting compiler needs to assemble output in linear time — `code-cat`'s O(n²)
    /// push-one-at-a-time becomes one O(log N) join.
    fn gen_runtime_list_concat(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("List.concat arity");
        }
        let (ac, ka) = self.emit(&elems[1], env, ctx)?;
        if ka != Kind::Heap {
            return decline("List.concat of a non-list value");
        }
        let (bc, kb) = self.emit(&elems[2], env, ctx)?;
        if kb != Kind::Heap {
            return decline("List.concat of a non-list value");
        }
        let mut c = ac;
        c.extend_from_slice(&bc);
        c.push(op::CALL);
        uleb128(himport::VEC_CONCAT as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// `(List.len v)` → element count as Int64 (`vec-len`, then i64.extend_i32_u). Reads a list
    /// however it was built (literal or grown) — the trie backs both.
    fn gen_runtime_list_len(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        if elems.len() != 2 {
            return decline("List.len arity");
        }
        let (v, kv) = self.emit(&elems[1], env, ctx)?;
        if kv != Kind::Heap {
            return decline("List.len of a non-list value");
        }
        let mut c = v;
        c.push(op::CALL);
        uleb128(himport::VEC_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u
        Ok((c, Kind::Int64))
    }

    /// Emit a runtime `(List.at v i)`: a FALLIBLE index (collections-and-text.md #Indexing And Lookup
    /// Are Fallible, Not Trapping). In-bounds (`0 <= i < vec-len(v)`) → `(Some elem)`; out-of-bounds /
    /// negative → `(None unit)`. Mirrors `gen_runtime_bytes_at` over `vec-*` instead of `bytes-*`.
    /// Unlike a byte (a raw i32 the compiler boxes), a LIST element is ALREADY a boxed heap handle —
    /// `gen_runtime_list_literal`/`vec-push` store each element via `box_scalar`, so `vec-get` returns
    /// the element's handle directly. That handle IS the `Some` payload (`Kind::Heap`); when the caller
    /// matches `((Some x) …)` and uses `x` as a scalar, the sum-match payload-kind override
    /// (scrutinee-shape / arm-unification) unboxes it — the same path `Bytes.at`'s Option uses. This
    /// is the shape a multi-arg-call lowering needs: `KCall (Tuple Int64 (list Core))` iterated by
    /// `List.at args i`. Result `Kind::Heap` (an `Option` sum). Requires the value-heap runtime.
    fn gen_runtime_list_at(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        if self.call_base == 0 {
            return decline("runtime list value needs the value-heap runtime");
        }
        if elems.len() != 3 {
            return decline("List.at arity");
        }
        let some_disc = self.variant_disc("Some")?;
        let none_disc = self.variant_disc("None")?;
        let (vc, kv) = self.emit(&elems[1], env, ctx)?;
        if kv != Kind::Heap {
            return decline("List.at of a non-list value");
        }
        let (ic, ki) = self.emit(&elems[2], env, ctx)?;
        if ki != Kind::Int64 {
            return decline("List.at index is not an integer");
        }
        let v = ctx.alloc_local(Kind::Bool); // i32 list handle
        let i = ctx.alloc_local(Kind::Int64); // the index (i64, Cadenza Int64)
        let mut c = Vec::new();
        c.extend_from_slice(&vc);
        c.push(op::LOCAL_SET);
        uleb128(v as u64, &mut c);
        c.extend_from_slice(&ic);
        c.push(op::LOCAL_SET);
        uleb128(i as u64, &mut c);
        // in_bounds = (i >= 0) & (i < vec-len(v)) — vec-len is i32, extend to i64 to compare.
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(op::I64_CONST);
        sleb128(0, &mut c);
        c.push(op::I64_GE_S);
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(v as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::VEC_LEN as u64, &mut c);
        c.push(0xAD); // i64.extend_i32_u
        c.push(op::I64_LT_S);
        c.push(0x71); // i32.and
        c.push(op::IF);
        c.push(0x7F); // block type i32 (a heap handle per arm)
        // then: Some(vec-get(v, wrap(i))) — vec-get returns the element's boxed handle directly.
        c.push(op::I32_CONST);
        sleb128(some_disc as i64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(v as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(i as u64, &mut c);
        c.push(0xA7); // i32.wrap_i64 — vec-get takes an i32 index
        c.push(op::CALL);
        uleb128(himport::VEC_GET as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::ELSE);
        // else: None(unit) — unit payload = arr-alloc(0)
        c.push(op::I32_CONST);
        sleb128(none_disc as i64, &mut c);
        c.push(op::I32_CONST);
        sleb128(0, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::END);
        Ok((c, Kind::Heap))
    }

    /// Box a just-emitted scalar into a heap object, leaving its handle (i32) on the stack. The
    /// scalar's bytes `sc` are already on the stack conceptually — this appends the box call for
    /// its `kind`. A Heap value is already a handle and needs no boxing. Declines a kind the
    /// runtime cannot yet box (Float/String/…), so a runtime compound with such a leaf declines
    /// rather than miscompiles (Phase C widens this).
    fn box_scalar(&self, kind: Kind) -> Result<u32, Decline> {
        match kind {
            Kind::Int64 => Ok(himport::BOX_INT),
            Kind::Bool => Ok(himport::BOX_BOOL),
            Kind::Heap => Ok(u32::MAX), // already a handle — sentinel: emit no box call
            // A `Never` element is a definite TRAP (an `unreachable`): the element expression
            // diverges before any value materializes, so there is nothing to box — emit no box call
            // (the `unreachable` is stack-polymorphic and satisfies the following `arr-set`/`sum-new`
            // typing). This arises when a compound payload const-folds to a trap, e.g. a `Core.KConst`
            // whose value is `(Bytes.len (Bytes.of (list 256)))` — 256 is out of byte range, so the
            // payload folds to a ConstTrap → `Kind::Never`. Without this the whole enclosing function
            // (the compiler's `resolve`, whose `PUnknown` arm builds exactly such a trapping KConst)
            // declined "cannot box", poisoning every call — the final self-host blocker (Tier 2f).
            // `Never` is NOT boxable — a diverging element has no value. It is handled at the
            // CONSTRUCTOR level (`gen_runtime_ctor`/`gen_runtime_sum` short-circuit to the element's
            // diverging bytes and return `Kind::Never`) BEFORE reaching here, so a `Never` at this
            // point is a caller that forgot the short-circuit — decline rather than emit a malformed
            // half-built compound around an `unreachable`.
            _ => decline("runtime compound element of a kind the runtime cannot box yet"),
        }
    }

    /// Emit a runtime compound constructor for `(tuple e…)`, `(record (k v)…)`, or `(list e…)`
    /// carrying at least one genuine RUNTIME element. All three are the same positional ARRAY at
    /// run time (the tag-free runtime holds no names or tuple/list distinction — those live in the
    /// static `Shape` the renderer walks): allocate an array of the right length, then for each
    /// element emit its value, box it into a heap handle, and `arr-set` it into place, threading
    /// the array handle. The array handle (i32) is left on the stack; the result kind is
    /// `Kind::Heap`. An all-constant compound never reaches here — it folds to the baked-text
    /// path, keeping existing PASS cases byte-identical (missing this guard would place a
    /// heap-constructor body in a scalar module where the heap-import indices are invalid).
    fn gen_runtime_ctor(
        &self,
        head: &str,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // A runtime `(list …)` is backed by the value-heap runtime's 32-way radix trie — the SAME
        // representation a grown list (`List.push`) uses — so that `List.push`/`List.len`/the
        // renderer read every list uniformly, whether it was written as a literal or grown. (A
        // tuple/record has fixed arity and is a flat positional array; a list is the trie.) Build it
        // by `vec-empty` then a `vec-push` per element, boxing each per its scalar kind — the
        // representation choice is unobservable (collections-and-text.md #A List's Representation Is
        // Unspecified And Unobservable). An all-constant list still takes this path only in runtime
        // mode (a const sub-value of a larger runtime heap value); the scalar-path fold-gate handles
        // the standalone constant, keeping those PASS cases byte-identical.
        if head == "list" {
            return self.gen_runtime_list_literal(&elems[1..], env, ctx);
        }
        // The element value nodes, in ARRAY (positional) order. For a record the array order is
        // the field VALUES sorted by key (matching the renderer's sorted `Shape::Record` and the
        // const `CVal::Record` order), so a record and its renderer agree on slot ↔ field.
        let value_nodes: Vec<Node> = match head {
            "tuple" => elems[1..].to_vec(),
            "record" => {
                let mut fields: Vec<(String, Node)> = Vec::new();
                for f in &elems[1..] {
                    if let Node::List(kv) = f {
                        if let (Some(Node::Name(k)), Some(v)) = (kv.first(), kv.get(1)) {
                            fields.push((k.clone(), v.clone()));
                            continue;
                        }
                    }
                    return decline("malformed record field in runtime constructor");
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                fields.into_iter().map(|(_, v)| v).collect()
            }
            _ => return decline("runtime compound constructor not yet emitted"),
        };

        // On the SCALAR path (`call_base == 0`) the module imports NO value-heap funcs, so a
        // runtime tuple/record constructor (`arr-alloc`/`arr-set`/`box-*`) has nothing to call —
        // emitting it produces an INVALID component. Decline UNCONDITIONALLY with a HEAP reason so
        // `compile_module` either dead-stubs this function (it is only reached by compile-time
        // structural projection — e.g. `main` does `(tuple.0 (dec 4))`, reducing the tuple away and
        // never calling `dec` at runtime) or RETRIES the whole pass in runtime mode where the heap
        // imports exist. This must NOT be gated on all-elements-const: a tuple with a genuine
        // runtime element (`(tuple (* n 10) 9)`, `n` a param) ALSO cannot build on the scalar path —
        // the earlier all-const-only guard let such a constructor emit `arr-alloc`/`box-int` into an
        // import-free module (the `tuple.N`-on-a-named-def-result INVALID-component bug). An
        // all-constant standalone compound still folds to the baked-text path before reaching here;
        // in RUNTIME MODE (`call_base != 0`) every compound — const or not — builds on the heap so
        // its handle can flow. Same gating as `gen_runtime_sum`.
        if self.call_base == 0 {
            return decline("constant compound (folds or is dead) — no runtime constructor");
        }

        let arity = value_nodes.len() as u32;
        let arr = ctx.alloc_local(Kind::Heap);
        let mut c = Vec::new();
        // arr = arr-alloc(arity)
        c.push(op::I32_CONST);
        sleb128(arity as i64, &mut c);
        c.push(op::CALL);
        uleb128(himport::ARR_ALLOC as u64, &mut c);
        c.push(op::LOCAL_SET);
        uleb128(arr as u64, &mut c);
        for (i, item) in value_nodes.iter().enumerate() {
            // arr-set(arr, i, box(elem)) ; drop the returned array handle
            c.push(op::LOCAL_GET);
            uleb128(arr as u64, &mut c);
            c.push(op::I32_CONST);
            sleb128(i as i64, &mut c);
            let (ec, ek) = self.emit(item, env, ctx)?;
            // A DIVERGING element (`Kind::Never` — a definite trap): the tuple/record never
            // materializes, so short-circuit to the element's diverging bytes THEN an explicit
            // `unreachable` (making the stack polymorphic so a Never-returning helper's pushed value
            // does not leak), returning `Never` — not `arr-set` around it (an invalid half-built
            // compound). The already-emitted array-alloc/index prefix `c` is discarded.
            if ek == Kind::Never {
                let mut d = ec;
                d.push(op::UNREACHABLE);
                return Ok((d, Kind::Never));
            }
            let box_fn = self.box_scalar(ek)?;
            c.extend_from_slice(&ec);
            if box_fn != u32::MAX {
                c.push(op::CALL);
                uleb128(box_fn as u64, &mut c);
            }
            c.push(op::CALL);
            uleb128(himport::ARR_SET as u64, &mut c);
            c.push(op::DROP);
        }
        // leave the array handle on the stack
        c.push(op::LOCAL_GET);
        uleb128(arr as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// Emit a runtime SUM constructor `(Variant payload)` carrying a genuine runtime payload:
    /// `sum-new(disc, box(payload))`, leaving the sum handle (i32) on the stack, result
    /// `Kind::Heap`. The discriminant is the variant's index in its sum type's declaration order
    /// (matching `sum_shape`, which the renderer switches on). Declines an all-constant payload
    /// (folds to baked text), a qualified variant (render-name reconstruction not handled — see
    /// `sum_shape`), or a payload the runtime cannot box.
    fn gen_runtime_sum(
        &self,
        variant: &str,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // On the SCALAR path (`call_base == 0`) the module imports no value-heap funcs, so a
        // `sum-new` has nothing to call. EVERY runtime sum constructor therefore declines here
        // with a HEAP reason so `compile_module` either dead-stubs an unreachable consumer (its
        // caller folded the sum away) or RETRIES the whole pass in runtime mode where the
        // value-heap import is present — the same mechanism the arr/list/bytes constructors use
        // (a non-const payload must NOT emit `sum-new` into an import-free module: that was the
        // "fn returns a heap sub-node" miscompile). An all-constant payload ALSO has a fold /
        // baked-text home, but either way the scalar path cannot build it. In RUNTIME MODE
        // (`call_base != 0`) a const sub-value — e.g. the `(None unit)` branch of a
        // runtime-sum-valued `if` — genuinely must build a runtime value so both `if` branches
        // share `Kind::Heap`, so this gate is scalar-path-only.
        let payload_node = elems.get(1);
        if self.call_base == 0 {
            return decline("runtime sum value needs the value-heap runtime");
        }
        // The discriminant is looked up by the bare TAG (`Cons` from `IntList.Cons`); a qualified
        // variant is fine — the tag drives the discriminant and the renderer reconstructs the
        // qualified name from the sum type's declaration. (A bare variant like `Some` has tag ==
        // name.)
        let tag = variant_tag(variant);
        let type_name = match self.sum_types.get(tag) {
            Some(t) => t,
            None => return decline(format!("unknown sum variant: {variant}")),
        };
        let order = match self.sum_variants.get(type_name) {
            Some(o) => o,
            None => return decline("sum type has no recorded variant order"),
        };
        let disc = match order.iter().position(|v| v == tag) {
            Some(i) => i as u32,
            None => return decline("variant not in its sum type's order"),
        };

        let sum = ctx.alloc_local(Kind::Heap);
        let mut c = Vec::new();
        // sum-new(disc, box(payload)) — emit disc, then the boxed payload, then the call.
        c.push(op::I32_CONST);
        sleb128(disc as i64, &mut c);
        // Box the payload into a heap handle. A nullary variant carries unit — whether written as
        // `(None unit)` (payload node `unit`, emits to `Kind::Unit`) or bare `None` (no node) — and
        // unit is represented as an empty array (`arr-alloc(0)`), the same as the empty tuple.
        let payload_is_unit = match payload_node {
            Some(p) => self.emit(p, env, ctx).map(|(_, k)| k == Kind::Unit).unwrap_or(false),
            None => true,
        };
        if payload_is_unit {
            c.push(op::I32_CONST);
            sleb128(0, &mut c);
            c.push(op::CALL);
            uleb128(himport::ARR_ALLOC as u64, &mut c);
        } else {
            let p = payload_node.expect("non-unit payload has a node");
            let (ec, ek) = self.emit(p, env, ctx)?;
            // A DIVERGING payload (`Kind::Never` — a definite trap, e.g. `(KConst (Bytes.len (Bytes.of
            // (list 256))))` where 256 is out of byte range): the payload traps before any value
            // exists, so the sum is never constructed. Emit the diverging bytes THEN an explicit
            // `unreachable`, and return `Never` — do NOT wrap `sum-new` around it (that half-built
            // call is the Tier 2f INVALID component). The trailing `unreachable` makes the stack
            // polymorphic so the value stack is well-typed regardless of what `ec` left (e.g. a
            // `call` to a Never-returning helper pushes its declared i64, which must not leak past
            // this point as the enclosing function's result). The already-emitted `disc` is dropped
            // with the discarded prefix `c`.
            if ek == Kind::Never {
                let mut d = ec;
                d.push(op::UNREACHABLE);
                return Ok((d, Kind::Never));
            }
            let box_fn = self.box_scalar(ek)?;
            c.extend_from_slice(&ec);
            if box_fn != u32::MAX {
                c.push(op::CALL);
                uleb128(box_fn as u64, &mut c);
            }
        }
        c.push(op::CALL);
        uleb128(himport::SUM_NEW as u64, &mut c);
        c.push(op::LOCAL_SET);
        uleb128(sum as u64, &mut c);
        c.push(op::LOCAL_GET);
        uleb128(sum as u64, &mut c);
        Ok((c, Kind::Heap))
    }

    /// A call to a user-defined module function.
    fn gen_call(
        &self,
        name: &str,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let f = match self.lookup_fn(name) {
            Some(f) => f,
            None => {
                // The name resolves to no user function. A host operation is now ALWAYS reached as
                // a qualified `effect.op` (a `.`-headed perform routed through `gen_perform`), never
                // as a bare name here, so a bare unresolved call head is not a host call.
                // An unresolved name in CALL/operator position — not a user function, not a local,
                // not a declared import — is reaching an operation the manifest does not enumerate:
                // the mandatory no-ambient-authority floor rejects it (CDZ0401,
                // capabilities-and-effects.md §Undeclared Capability Is A Compile-Time Error;
                // host-interface-binding.md §Ungranted Access Is Rejected At Compile Time). The
                // contract forbids naming concrete host functions, so a call head that resolves to
                // nothing is treated as an ungranted host operation — distinct from an unbound
                // VALUE reference (`y`), which `gen_name` reports as CDZ0101. Guarded on the head
                // NOT being a known form/construct keyword (those decline with their own honest
                // message) and having at least one argument (a bare applied name).
                if !is_form_keyword(name) && !is_constructor_name(name) {
                    return reject("CDZ0401", format!("undeclared capability: {name}"));
                }
                return decline(format!("unbound name: {name}"));
            }
        };
        let args = &elems[1..];
        // OVER-application is a TYPE ERROR, not a feature gap (ask-21). `(f 5 9)` on a unary `f` desugars
        // to `((f 5) 9)` — applying `f`'s (Int64) RESULT to `9`, i.e. applying a non-function. That is
        // CDZ0201 (apply-a-non-function), the SAME rejection the constructor over-application `(Some 1 2)`
        // already gets (09-functions.sexp: a user-function over-application is arity-checked the same way).
        // UNDER-application `(f)` / `(f 5)` on a binary `f` is a genuine partial application — a closures
        // feature the seed does not yet lower — so it stays an honest DECLINE (not a type error: a
        // partially-applied function is well-typed, just unsupported here).
        if args.len() > f.params.len() {
            return reject("CDZ0201", format!(
                "over-applying `{name}`: {} args to a function of {} parameter(s) (applies a non-function)",
                args.len(),
                f.params.len()
            ));
        }
        if args.len() < f.params.len() {
            return decline(format!(
                "call to `{name}` with {} args, expected {} (partial application needs closures)",
                args.len(),
                f.params.len()
            ));
        }
        // Cross-function effect resolution is DYNAMIC IN EXTENT (capabilities-and-effects.md
        // §Handler Resolution Is Dynamic In Extent And Statically Determined): a callee may perform
        // an operation its CALLER handles. When a router (a `handle`/`host`) is active and the
        // callee (transitively) performs an effect operation, INLINE the call — monomorphize the
        // callee into the handled region so its performs are textually enclosed by the caller's
        // routers and the existing intra-function resolution applies. This is the Stage-1
        // realization of effect-context monomorphization via inlining (options/effects-model/
        // lowering-to-wasm.md §Effect-context monomorphization); the callee, reached only by
        // inlining, is dead as a standalone runtime function. Guarded on active routers AND the
        // callee performing an effect, so an ordinary scalar call is unaffected and stays byte-
        // identical. Bind each parameter to its argument NODE (aliased), exactly like a lambda arg.
        if !ctx.routers.is_empty() && self.fn_reaches_effect(name, &mut Vec::new()) {
            // A RECURSIVE effectful function cannot be inlined — inlining its own body would not
            // terminate (§Effect-context monomorphization: the recursive-task wall). Decline
            // cleanly (Stage-3 monomorphization is where this is realized); NEVER hang. `inlining`
            // records the functions being inlined right now, so a call to one already in progress is
            // the recursive case (a self-call or a mutual-recursion cycle).
            // A RECURSIVE effectful function cannot be inlined (its body would inline without
            // bound). Discharge it by effect-context MONOMORPHIZATION: emit it once per handler
            // context as a real wasm function, threading the enclosing handlers' states as hidden
            // trailing params/returns (`gen_specialized_call`). The self-call inside the
            // specialization resolves to the same specialization → an ordinary `call`, so the
            // recursion terminates. `ctx.inlining` carrying `name` (set when we began inlining or
            // specializing it) is how a self-call is detected.
            if ctx.inlining.iter().any(|n| n == name) || self.fn_is_recursive(name) {
                return self.gen_specialized_call(name, args, env, ctx);
            }
            let mut body_env: Vec<Local> = Vec::new();
            for (p, a) in f.params.iter().zip(args.iter()) {
                body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
            }
            ctx.inlining.push(name.to_string());
            let out = self.emit(&f.body.clone(), &body_env, ctx);
            ctx.inlining.pop();
            return out;
        }
        // A named HIGHER-ORDER function receiving a lambda (or lambda-aliased name) as an argument:
        // a lambda has no scalar wasm representation to pass by value, so INLINE the call —
        // monomorphize `f` at this site by binding each parameter to its argument NODE as an alias
        // and emitting `f`'s body, exactly as `gen_apply` inlines a lambda callee. This is how a
        // let-bound lambda already flows into a HOF; extending it to a named-def HOF's argument
        // closes the "bare lambda in scalar position" decline (09-functions.sexp §"a named
        // higher-order function receives a lambda argument"). Only fires when an argument actually
        // resolves to a lambda — an ordinary scalar call still takes the fast `call` path below,
        // keeping existing PASS cases byte-identical.
        let has_lambda_arg = args.iter().any(|a| self.resolve_lambda(a, env).is_some());
        if has_lambda_arg {
            let mut body_env: Vec<Local> = Vec::new();
            for (p, a) in f.params.iter().zip(args.iter()) {
                body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
            }
            return self.emit(&f.body.clone(), &body_env, ctx);
        }
        // Emit each argument; a kind that disagrees with the callee's monomorphized parameter
        // kind does NOT immediately fail. The callee may be POLYMORPHIC — an unconstrained
        // parameter (identity's `x`, returned unchanged) generalizes to a type variable (∀a.
        // a→a), not Int64, so `(id true)` is well-typed even though `id`'s parameter was
        // monomorphized to Int64 for its Int64 call site. The seed has no runtime type variables
        // (the coarse `Kind` lattice is monomorphic), so it realizes polymorphism by per-call
        // monomorphization: INLINE the call at this site (bind the parameter to the argument
        // node, emit the body). A genuinely monomorphic body — `(+ x 1)` applied to a Bool —
        // fails its OWN type rules when inlined, so this does not weaken type-checking; it only
        // admits the calls a full HM generalization would (09-functions.sexp §"the identity
        // function applied to a boolean returns the boolean").
        let mut arg_code: Vec<Vec<u8>> = Vec::with_capacity(args.len());
        for (arg, pk) in args.iter().zip(f.param_kinds.iter()) {
            let (ac, ak) = self.emit(arg, env, ctx)?;
            // A DIVERGING argument (`Kind::Never` — a definite trap, e.g. a Never-returning helper
            // call, or a compound whose element trapped): the callee is never reached. Emit the
            // argument's diverging bytes THEN `unreachable`, and return `Never` — do NOT fall into
            // the `ak != pk` inline path (a Never-vs-Heap "mismatch" would wrongly inline) nor emit
            // the `call` (whose value stack would be malformed — a Never callee's i64-typed value
            // leaking to a caller expecting a different kind was the Tier 2f INVALID component).
            if ak == Kind::Never {
                let mut c = Vec::new();
                for prev in arg_code {
                    c.extend_from_slice(&prev);
                }
                c.extend_from_slice(&ac);
                c.push(op::UNREACHABLE);
                return Ok((c, Kind::Never));
            }
            if ak != *pk {
                return self.gen_apply(&elems[0], args, env, ctx);
            }
            arg_code.push(ac);
        }
        let mut c = Vec::new();
        for ac in arg_code {
            c.extend_from_slice(&ac);
        }
        // Record the runtime call: this callee is reachable (not fully const-folded away), so
        // it must be emitted as real wasm (compile_module keeps it live).
        ctx.called.insert(f.index);
        c.push(op::CALL);
        uleb128((f.index + self.call_base) as u64, &mut c);
        Ok((c, f.ret_kind))
    }

    /// Lower a call to an imported host function. The host funcs occupy the LOW core-func indices
    /// (`0..n_host_imports`, in declaration order), so `main`/callee user functions are shifted by
    /// `call_base` (set to the import count on the host path). Each argument is lowered to its core
    /// representation: a scalar as itself; a `String` argument as a `(ptr, len)` pair — the emitted
    /// core func imports the host string as two i32s (the canon lower marshals our memory bytes into
    /// the host's `string`). Only a compile-time-constant string argument is lowered for now (the
    /// corpus passes string literals); a runtime string argument declines.
    fn gen_host_call(
        &self,
        hi: &HostImport,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let args = &elems[1..];
        if args.len() != hi.params.len() {
            return decline(format!(
                "host call `{}` with {} args, expected {}",
                hi.name,
                args.len(),
                hi.params.len()
            ));
        }
        let idx = match self.host_imports.iter().position(|h| h.name == hi.name) {
            Some(i) => i as u32,
            None => return decline("host import not found"),
        };
        let mut c = Vec::new();
        // A rolling scratch cursor for string argument bytes, above the const-string return-pair
        // scratch (`HEAP_BASE`). Each string literal writes its bytes there and passes (ptr,len).
        let mut str_cursor = HEAP_BASE;
        for (arg, pk) in args.iter().zip(hi.params.iter()) {
            match pk {
                Kind::HostString => {
                    // Only a compile-time-constant string literal is supported for now.
                    let s = match self.eval_const(arg, env) {
                        Ok(Some(CVal::Str(s))) => s,
                        _ => return decline("runtime string argument to host call not yet lowered"),
                    };
                    let bytes = s.as_bytes();
                    let ptr = str_cursor;
                    for (i, b) in bytes.iter().enumerate() {
                        c.push(op::I32_CONST);
                        sleb128(ptr + i as i64, &mut c);
                        c.push(op::I32_CONST);
                        sleb128(*b as i64, &mut c);
                        c.extend_from_slice(&[op::I32_STORE8, 0x00, 0x00]);
                    }
                    // Push (ptr, len) as the two i32 core args for the lowered string param.
                    c.push(op::I32_CONST);
                    sleb128(ptr, &mut c);
                    c.push(op::I32_CONST);
                    sleb128(bytes.len() as i64, &mut c);
                    str_cursor += bytes.len() as i64;
                }
                _ => {
                    let (ac, ak) = self.emit(arg, env, ctx)?;
                    if ak != *pk {
                        return decline("host call argument kind mismatch");
                    }
                    c.extend_from_slice(&ac);
                }
            }
        }
        // Host imports are never trap-stubbed away; mark this one reached. They occupy the low
        // indices directly (NOT offset by call_base — call_base shifts USER funcs past the
        // imports). Record via a distinct high-bit marker so reachability keeps them (see
        // compile_module).
        c.push(op::CALL);
        uleb128(idx as u64, &mut c);
        Ok((c, hi.result))
    }

    // ─── Effects: handlers, host delegation, and perform ─────────────────────────────
    //
    // The whole intra-program effect layer lowers with NO runtime continuation machinery: the
    // discharging handler is resolved STATICALLY (nearest enclosing router on `ctx.routers`), and
    // a tail-resumptive perform against a statically-known arm is an ORDINARY INLINED FUNCTION BODY
    // — bind the op params to the arg nodes and the state binder to the current state, emit the arm
    // body with `(resume value next-state)` rewritten to `value`. This reuses the exact
    // aliased-local + `emit` machinery lambda arguments already use (options/effects-model/
    // lowering-to-wasm.md §Tier 1). A `(host …)` delegation lowers a delegated operation as a plain
    // imported-function boundary call (§8).

    /// Lower a `(handle <init> ((E.op (params…) state body)…) body)` form: parse and classify the
    /// arms (CDZ0403 if an arm names an operation its effect does not declare), push a
    /// `HandlerFrame` onto the router stack, emit the body under it, pop. The handle emits ONLY its
    /// body — arms are emitted lazily at each perform site (Tier-1 inline).
    fn gen_handle(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // Form: (handle <init> (<arm>…) <body>).
        if elems.len() != 4 {
            return decline("handle form: expected (handle <init> (arms…) body)");
        }
        let init = elems[1].clone();
        let arm_forms = match &elems[2] {
            Node::List(a) => a,
            _ => return decline("handle arms are not a list"),
        };

        // Each arm is `(<E>.<op> (params…) <state> <body>)`. The head `<E>.<op>` reads as the
        // member-access tree `(. E op)` (reader dotted-name sugar), so pull `E` and `op` from it.
        // A single handle may name arms for more than one effect (their operations disambiguate).
        let mut arms: Vec<HandlerArm> = Vec::new();
        for arm in arm_forms {
            let a = match arm {
                Node::List(a) if a.len() == 4 => a,
                _ => return decline("handle arm: expected (E.op (params…) state body)"),
            };
            let (eff, op) = match effect_op_of_head(&a[0]) {
                Some(pair) => pair,
                None => return decline("handle arm head is not E.op"),
            };
            // CDZ0403: the arm names an operation the effect does not declare (the declaration is
            // the closed set of an effect's operations).
            match self.effects.get(&eff) {
                Some(decl) if decl.op(&op).is_some() => {}
                Some(_) => {
                    return reject(
                        "CDZ0403",
                        format!("handler arm names `{eff}.{op}`, which effect `{eff}` does not declare"),
                    )
                }
                // The effect is not declared at all — a handler for an undeclared effect. Decline
                // (an honest gap; a well-formed program declares the effect it handles).
                None => return decline(format!("handle for undeclared effect `{eff}`")),
            }
            let params = match &a[1] {
                Node::List(ps) => {
                    let mut v = Vec::new();
                    for p in ps {
                        match p {
                            Node::Name(n) => v.push(n.clone()),
                            _ => return decline("handle arm parameter is not a name"),
                        }
                    }
                    v
                }
                _ => return decline("handle arm parameters are not a list"),
            };
            let state = match &a[2] {
                Node::Name(n) => n.clone(),
                _ => return decline("handle arm state binder is not a name"),
            };
            let body = a[3].clone();
            let class = classify_arm(&body);
            arms.push(HandlerArm { effect: eff, op, params, state, body, class });
        }
        if arms.is_empty() {
            return decline("handle with no arms");
        }

        // The seed state's kind decides unit-state (zero-cost) vs a real fold. `unit` is the
        // degenerate case: no local, no threading, byte-identical to a stateless inline. A non-unit
        // seed (a `Fresh` counter seeded `0`, a `Diag` list seeded `(list)`) allocates a mutable
        // wasm local, seeds it to `<init>`, and threads it across the performs
        // (capabilities-and-effects.md §A Handler Threads State Across The Operations It Discharges).
        let mut prologue = Vec::new();
        let (state_kind, state_local) = match self.emit(&init, env, ctx) {
            Ok((init_code, Kind::Unit)) => {
                // Unit state: no bytes, no local. (Emitting `unit` produced no code anyway.)
                let _ = init_code;
                (Kind::Unit, None)
            }
            Ok((init_code, k)) => {
                let slot = ctx.alloc_local(k);
                prologue.extend_from_slice(&init_code);
                prologue.push(op::LOCAL_SET);
                uleb128(slot as u64, &mut prologue);
                (k, Some(slot))
            }
            // The seed did not lower (e.g. a runtime-compound seed the emitter cannot yet produce);
            // decline honestly rather than mis-thread state.
            Err(d) => return Err(d),
        };

        let frame = HandlerFrame {
            arms,
            def_env: env.to_vec(),
            def_depth: ctx.routers.len(),
            state_kind,
            state_local,
        };
        ctx.routers.push(RouterFrame::Handler(frame));
        let out = self.emit(&elems[3], env, ctx);
        ctx.routers.pop();
        let (body_code, body_kind) = out?;
        // The handle evaluates to its BODY's value; the accumulated state is discharged at the
        // boundary (read out only if the body performed a read-out op). Prepend the state-seed
        // prologue (empty for unit-state).
        let mut code = prologue;
        code.extend_from_slice(&body_code);
        Ok((code, body_kind))
    }

    /// Lower an entrypoint `(host (Effect…) body)` delegation: push a `HostFrame` naming the
    /// delegated effects, emit the body under it, pop. On pop, each named effect no reachable
    /// perform matched is CDZ0404 (latent authority). The delegation IS the manifest grant; a
    /// delegated operation lowers to a boundary call in `gen_perform` (Stage 2). `host` is admitted
    /// only at an entrypoint — but since the seed treats `main` as the sole entrypoint and this is
    /// reached only while emitting a body, the entrypoint restriction is enforced by the caller
    /// (`emit`), which routes a `host` form to `gen_host` only inside `main`'s body.
    fn gen_host(
        &self,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // Form: (host (Effect…) body).
        if elems.len() != 3 {
            return decline("host form: expected (host (Effect…) body)");
        }
        let eff_forms = match &elems[1] {
            Node::List(e) => e,
            _ => return decline("host delegation effects are not a list"),
        };
        let mut effects = Vec::new();
        for e in eff_forms {
            match e {
                Node::Name(n) => effects.push(n.clone()),
                _ => return decline("host delegation effect is not a name"),
            }
        }
        let frame = HostFrame {
            effects: effects.clone(),
            reached: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        };
        ctx.routers.push(RouterFrame::Host(frame));
        let out = self.emit(&elems[2], env, ctx);
        let popped = ctx.routers.pop();
        let (code, kind) = out?;
        // CDZ0404: a delegation naming an effect the body never reaches carries latent authority.
        if let Some(RouterFrame::Host(f)) = popped {
            let reached = f.reached.borrow();
            for e in &effects {
                if !reached.contains(e) {
                    return reject(
                        "CDZ0404",
                        format!("entrypoint delegates `{e}` to the host but never reaches it"),
                    );
                }
            }
        }
        Ok((code, kind))
    }

    /// Lower a perform `(E.op args…)` — reached from the `.`-headed dispatcher when `E` is a
    /// declared effect and `op` one of its operations. Resolve `E.op` top-down over the router
    /// stack: a nearer `HandlerFrame` for `E` discharges it in-program (Tier 1); else a `HostFrame`
    /// delegating `E` routes it to the boundary (Stage 2); else — reached the entrypoint top with
    /// no home — CDZ0401.
    fn gen_perform(
        &self,
        eff: &str,
        op: &str,
        elems: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let args = &elems[1..];
        // A perform's ARGUMENTS are type-checked against the operation's declared parameter types,
        // exactly as an ordinary function application's are (capabilities-and-effects.md #Performing
        // An Operation Is Typed And Contributes To The Row). `(E.op true)` on an `(-> Int64 Int64)`
        // operation supplies a Bool where the parameter type is Int64 — a type error (CDZ0201), NOT a
        // value silently fed through the op's Int64 slot (which yields a wrong value, and a String
        // argument a GARBAGE integer). Without this check the perform lowered the mistyped operation:
        // a MISCOMPILE, the worst outcome (14-effects-and-handlers.sexp §"performing an operation with
        // an argument of the wrong type is a type error"). Checked here up front, before router
        // dispatch, so it fires whether the op is handled or delegated. Only a PROVABLE
        // mismatch rejects, and the check is UNIFORM across every parameter type SHAPE — scalar
        // (`Int64`/`Bool`/…), String, AND compound (`(List Int64)`, a tuple/record). The coarse
        // parameter `Kind` collapses every non-scalar type to `Heap`, so checking the arg only against
        // the scalar Kind silently skipped a String or compound parameter, feeding a mistyped value
        // through the op's slot (an Int through a String slot, a bare Int bound where a `List Int64` is
        // declared). Check the arg against the declared parameter TYPE NODE via
        // `arg_contradicts_declared_type` (handles scalar names, `String`, and the compound heads). An
        // argument of unknown static type, or a bare user-type parameter, imposes nothing (conservative
        // — never a false reject).
        if let Some(decl) = self.effects.get(eff).and_then(|d| d.op(op)) {
            for (pty, arg) in decl.param_types.iter().zip(args.iter()) {
                if self.arg_contradicts_declared_type(arg, pty, env) {
                    return reject(
                        "CDZ0201",
                        format!("perform `{eff}.{op}` argument type does not match the declared parameter type"),
                    );
                }
            }
        }
        // Walk routers nearest-first (top of stack = innermost).
        for depth in (0..ctx.routers.len()).rev() {
            match &ctx.routers[depth] {
                RouterFrame::Handler(h) => {
                    // Does this handler have an arm for this effect+op? A handler names a subset of
                    // one or more effects' ops; an op it does not name resolves PAST it to the
                    // next-outer router (a handler is not required to discharge every op).
                    if let Some(arm_idx) =
                        h.arms.iter().position(|a| a.effect == eff && a.op == op)
                    {
                        return self.emit_handler_arm(depth, arm_idx, args, env, ctx);
                    }
                }
                RouterFrame::Host(f) if f.effects.iter().any(|e| e == eff) => {
                    // Delegated to the host: mark the effect reached (clears CDZ0404) and emit the
                    // boundary call (Stage 2).
                    f.reached.borrow_mut().insert(eff.to_string());
                    return self.gen_delegated_call(eff, op, args, env, ctx);
                }
                _ => {}
            }
        }
        // No enclosing handler and no enclosing delegation: the effect would escape ungranted.
        reject(
            "CDZ0401",
            format!("`{eff}.{op}` is reached with neither an enclosing handler nor a host delegation"),
        )
    }

    /// Emit a perform against the handler arm at `routers[depth]`, arm `arm_idx`, applied to
    /// `args`. Dispatches on the arm's class; Tier-1 (Tail) inlines the arm body with `resume`
    /// unwrapped, under the arm's definition-site environment and the router stack truncated to the
    /// handle's definition depth (the under-frame).
    fn emit_handler_arm(
        &self,
        depth: usize,
        arm_idx: usize,
        args: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        // Clone the frame data we need (a handler arm's body/env), so we can mutate `ctx.routers`
        // (the under-frame truncation) while emitting.
        let (arm, def_env, def_depth, class, state_kind, state_local) = {
            let h = match &ctx.routers[depth] {
                RouterFrame::Handler(h) => h,
                _ => return decline("internal: expected handler frame"),
            };
            let arm = h.arms[arm_idx].clone();
            let class = arm.class;
            (arm, h.def_env.clone(), h.def_depth, class, h.state_kind, h.state_local)
        };
        match class {
            ArmClass::Tail => {}
            ArmClass::Abortive => {
                return decline("abortive handler (Tier 2, block/br) not yet lowered")
            }
            ArmClass::GeneralOneShot => {
                return decline("general one-shot handler (Tier 3, reified continuation) not yet lowered")
            }
        }
        // A nullary-typed operation (`next : Unit -> Int64`) is PERFORMED with no args
        // (`(Fresh.next)`) yet its arm binds a unit parameter (`(u)`); the elided argument is the
        // unit value. Normalize by padding a missing trailing `Unit`-typed argument with `unit`, so
        // the perform's supplied args line up with the arm's parameters. The op's declared param
        // kinds decide which positions may be elided.
        let declared_params: Vec<Kind> = self
            .effects
            .get(&arm.effect)
            .and_then(|d| d.op(&arm.op))
            .map(|o| o.params.clone())
            .unwrap_or_default();
        let mut supplied: Vec<Node> = args.to_vec();
        while supplied.len() < arm.params.len()
            && declared_params.get(supplied.len()) == Some(&Kind::Unit)
        {
            supplied.push(Node::Name("unit".into()));
        }
        if supplied.len() != arm.params.len() {
            return decline(format!(
                "perform `{}` with {} args, arm binds {}",
                arm.op,
                supplied.len(),
                arm.params.len()
            ));
        }
        // Bind each op parameter to its argument NODE (aliased, re-emitted under the perform-site
        // env). This is the same aliased-local machinery a lambda argument uses.
        let mut body_env = def_env;
        for (p, a) in arm.params.iter().zip(supplied.iter()) {
            body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
        }
        // The value a handler RESUMES with — `(resume <value> <state>)` — is returned to the perform
        // site as the operation's result, so it MUST have the op's declared RESULT type
        // (capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row: a
        // perform yields the operation's declared result type; an effect op is typed exactly as an
        // ordinary function). `(resume true s)` on an `(-> Int64 Int64)` op resumes with a Bool — a
        // mismatch (CDZ0201), the result-type companion of the perform-argument-type check
        // (14-effects-and-handlers.sexp §"resuming with a value of the wrong type …"). Check each TAIL
        // resume value against the op's declared RESULT TYPE NODE via `arg_contradicts_declared_type`,
        // UNIFORMLY across result type shapes — scalar (`Int64`/…) AND compound (`(List Int64)`, a
        // tuple). Checking only the coarse scalar `Kind` (`Heap` for every compound) silently accepted
        // a bare Int resumed where a `(List Int64)` is declared — a type-confusion wrong value. Only a
        // PROVABLE mismatch rejects (a value of unknown type imposes nothing — conservative). The state
        // binder is not yet in `body_env`, but a resume VALUE's type does not depend on the state.
        if let Some(result_type) =
            self.effects.get(&arm.effect).and_then(|d| d.op(&arm.op)).map(|o| o.result_type.clone())
        {
            let mut mismatch = false;
            for_each_tail_resume_value(&arm.body, &mut |val| {
                if self.arg_contradicts_declared_type(val, &result_type, &body_env) {
                    mismatch = true;
                }
            });
            if mismatch {
                return reject(
                    "CDZ0201",
                    format!("resume value type does not match `{}.{}`'s declared result type", arm.effect, arm.op),
                );
            }
        }
        // BOTH operands of a `(resume <value> <state>)` are ordinary expressions subject to lexical
        // scope (core-semantics.md #Binding Is Lexical — unconditional): an unbound name in the STATE
        // position is CDZ0101, exactly as one in the value position already is (the value is emitted, so
        // its unbound name is caught downstream; the state — for a Unit-state arm — is unwrapped away and
        // never emitted, so its unbound name slipped through silently). Scope-check each tail resume's
        // STATE operand at EMIT (where `body_env` carries the arm's param binders — a scope check needs
        // the lexical env, never `check_tree`). Conservative: `provably_unbound_name` reports only a name
        // bound nowhere and referenced as a value (bails on binder forms). 14-effects-and-handlers.sexp
        // §"an unbound name in a resume's state position is rejected".
        {
            // The state binder (`s` in `((E.op (n) s …))`) is not yet in `body_env` (it is pushed
            // below, per unit/non-unit state), but a resume state may reference it (`(resume v (+ s 1))`),
            // so add it to the scope-check env — else a legitimate state fold false-reports `s` unbound.
            let mut scope_env = body_env.clone();
            scope_env.push(Local::aliased(arm.state.clone(), Node::Name("unit".into()), Vec::new()));
            let mut unbound: Option<String> = None;
            for_each_tail_resume_state(&arm.body, &mut |st| {
                if unbound.is_none() {
                    unbound = self.provably_unbound_name(st, &scope_env);
                }
            });
            if let Some(name) = unbound {
                return reject("CDZ0101", format!("unbound name `{name}` in resume state"));
            }
        }
        // The state binder resolves to the handler's CURRENT state. Unit-state: bind `s` to `unit`
        // (zero-width, no local — the degenerate case, byte-identical to a stateless inline). A real
        // fold: bind `s` to `(local.get state_local)`, so every read of `s` in the arm reads the
        // current threaded value.
        let saved: Vec<RouterFrame> = ctx.routers.split_off(def_depth);
        let result = if state_kind == Kind::Unit || state_local.is_none() {
            // Unit-state (or no state local): bind `s` to unit and unwrap the tail resume to its
            // value. `next-state` is `s` unchanged and carries no bytes.
            body_env.push(Local::aliased(arm.state.clone(), Node::Name("unit".into()), Vec::new()));
            let rewritten = unwrap_tail_resume(&arm.body);
            self.emit(&rewritten, &body_env, ctx)
        } else {
            // Real fold: `s` reads the state local. Emit the tail structure, and at each tail
            // `(resume value next-state)` leave `value` on the stack after threading `next-state`
            // back into the state local (the mutation the immutable heap stays under). The state
            // kind is carried in the accessor node — the handler frame is off `ctx.routers` during
            // arm emission (the under-frame truncation), so the read cannot recover its kind from
            // the stack.
            let slot = state_local.unwrap();
            let state_node = Node::List(vec![
                Node::Name("@state-local".into()),
                Node::Int(slot as i64),
                Node::Int(state_kind_tag(state_kind)),
            ]);
            body_env.push(Local::aliased(arm.state.clone(), state_node, Vec::new()));
            self.emit_tail_resume_threaded(&arm.body, &body_env, ctx, slot, state_kind)
        };
        ctx.routers.extend(saved);
        result
    }

    /// Emit an arm body threading a NON-UNIT handler state through a wasm local `slot`. At each tail
    /// `(resume value next-state)`: emit `value` (leaving it on the stack), then emit `next-state`
    /// and `local.set slot` — both `value` and `next-state` read the OLD state (the local before the
    /// set), so a `Fresh.next` arm `(resume s (+ s 1))` hands back the current `s` and advances the
    /// local to `s+1`. Recurses into the tails of `if`/`do`/`let`/`match` (the same tail positions
    /// `unwrap_tail_resume` handles), so every control path threads correctly.
    fn emit_tail_resume_threaded(
        &self,
        node: &Node,
        env: &[Local],
        ctx: &mut FnCtx,
        slot: u32,
        state_kind: Kind,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        match node {
            Node::List(items) if name_of(items.first()) == Some("resume") && items.len() >= 3 => {
                // (resume value next-state): emit value, then thread next-state.
                let (vc, vk) = self.emit(&items[1], env, ctx)?;
                let (sc, sk) = self.emit(&items[2], env, ctx)?;
                if sk != state_kind {
                    return decline("resume next-state kind disagrees with the handler state kind");
                }
                let mut c = vc;
                c.extend_from_slice(&sc);
                c.push(op::LOCAL_SET);
                uleb128(slot as u64, &mut c);
                Ok((c, vk))
            }
            Node::List(items) if name_of(items.first()) == Some("do") && items.len() >= 2 => {
                // Emit non-tail forms as an ordinary `do` prefix (dropping their values), then the
                // tail form threaded. Reuse the aliased-node approach: emit each prefix form, drop.
                let mut c = Vec::new();
                for form in &items[1..items.len() - 1] {
                    let (fc, fk) = self.emit(form, env, ctx)?;
                    c.extend_from_slice(&fc);
                    if fk != Kind::Unit && fk != Kind::Never {
                        c.push(op::DROP);
                    }
                }
                let (tc, tk) =
                    self.emit_tail_resume_threaded(items.last().unwrap(), env, ctx, slot, state_kind)?;
                c.extend_from_slice(&tc);
                Ok((c, tk))
            }
            // A non-resume tail (e.g. a read-out arm whose body is just `(resume s s)` handled above,
            // or a plain value that does not resume) — emit as-is with resume unwrapped. This covers
            // an arm whose tail is not a resume at all (an abortive-shaped tail would have been
            // classified Abortive and declined earlier).
            _ => {
                let rewritten = unwrap_tail_resume(node);
                self.emit(&rewritten, env, ctx)
            }
        }
    }

    /// Does the user function `name` (transitively) perform a declared effect operation? Walks its
    /// body: a `(E.op …)` perform of a declared effect, or a call to another function that does.
    /// `stack` guards against infinite recursion on a recursive/mutually-recursive call graph
    /// (a recursive effectful function reports true and is inlined once — its self-call is then
    /// resolved within the inlined body's own router context). Used to decide cross-function
    /// inlining for effect resolution.
    fn fn_reaches_effect(&self, name: &str, stack: &mut Vec<String>) -> bool {
        if stack.iter().any(|n| n == name) {
            return false; // already visiting — do not recurse forever
        }
        let f = match self.lookup_fn(name) {
            Some(f) => f,
            None => return false,
        };
        stack.push(name.to_string());
        let body = f.body.clone();
        let out = self.node_reaches_effect(&body, stack);
        stack.pop();
        out
    }

    /// Does `node` (transitively) reach a declared-effect perform — either a direct `(E.op …)` of a
    /// declared effect, or a call to a user function that does?
    fn node_reaches_effect(&self, node: &Node, stack: &mut Vec<String>) -> bool {
        if let Node::List(items) = node {
            // A direct perform `(E.op …)` — head `(. E op)` of a declared effect.
            if let Some(head) = items.first() {
                if let Some((e, o)) = effect_op_of_head(head) {
                    if self.effects.get(&e).map_or(false, |d| d.op(&o).is_some()) {
                        return true;
                    }
                }
                // A call to a named user function that reaches an effect.
                if let Node::Name(h) = head {
                    if self.lookup_fn(h).is_some() && self.fn_reaches_effect(h, stack) {
                        return true;
                    }
                }
            }
            // Recurse into every sub-form (an effect performed anywhere in the body counts). Do not
            // descend into a nested `(handle …)` for the SAME purpose — but a nested handle's body
            // could still perform an OUTER effect, so descending is conservative-correct (it may
            // over-report, which only means an extra inline, never a miscompile).
            return items.iter().any(|c| self.node_reaches_effect(c, stack));
        }
        false
    }

    /// Is the user function `name` SELF-RECURSIVE (transitively) — does a call from its body reach a
    /// call back to `name`? A recursive effectful function cannot be discharged by inlining (its
    /// body would inline without bound); it is emitted as an effect-context SPECIALIZATION instead.
    fn fn_is_recursive(&self, name: &str) -> bool {
        let start = match self.lookup_fn(name) {
            Some(f) => f,
            None => return false,
        };
        // BFS over the call graph from `name`'s body; a call back to `name` means recursive.
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let mut work: Vec<Node> = vec![start.body.clone()];
        while let Some(node) = work.pop() {
            if let Node::List(items) = &node {
                if let Some(Node::Name(h)) = items.first() {
                    if h == name {
                        return true; // a call to the start function
                    }
                    if seen.insert(h.clone()) {
                        if let Some(callee) = self.lookup_fn(h) {
                            work.push(callee.body.clone());
                        }
                    }
                }
                for c in items {
                    work.push(c.clone());
                }
            }
        }
        false
    }

    /// A stable fingerprint of the handler-context on the router stack — the enclosing HANDLER
    /// frames (host delegations do not fold state, so they contribute only their effect set),
    /// outermost first. Two call sites under the same handlers share one specialization. Encodes
    /// each handler's effects+ops and each arm body's shape so distinct handler contexts (the
    /// same-fn-two-handlers property) get distinct specializations.
    fn context_key(routers: &[RouterFrame]) -> String {
        let mut key = String::new();
        for r in routers {
            match r {
                RouterFrame::Handler(h) => {
                    key.push('H');
                    for a in &h.arms {
                        key.push_str(&a.effect);
                        key.push('.');
                        key.push_str(&a.op);
                        key.push(':');
                        key.push_str(&format!("{:?};", a.body));
                    }
                    key.push_str(&format!("s{:?}|", h.state_kind));
                }
                RouterFrame::Host(f) => {
                    key.push('D');
                    for e in &f.effects {
                        key.push_str(e);
                        key.push(',');
                    }
                    key.push('|');
                }
            }
        }
        key
    }

    /// Lower a call to a RECURSIVE effectful function by effect-context MONOMORPHIZATION (Stage 3):
    /// emit the function once per handler context as a real wasm function whose enclosing handlers'
    /// states are threaded as hidden trailing PARAMETERS and returned as extra RESULTS (evidence
    /// passing — each handler context gets its own state on the call stack, so nested/wrapped
    /// effects compose without a global's single-slot clobber). At this call site: push the original
    /// args, push each enclosing handler's CURRENT state, `call` the specialization, then store the
    /// returned next-states back into the caller's state locals and leave the result. The self-call
    /// inside the specialization resolves here again — same context key → same specialization → an
    /// ordinary `call` (the recursion the inline path could not terminate on).
    fn gen_specialized_call(
        &self,
        name: &str,
        args: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let f = match self.lookup_fn(name) {
            Some(f) => f,
            None => return decline("specialized call to unknown function"),
        };
        if args.len() != f.params.len() {
            return decline("specialized call arity mismatch");
        }
        // A recursive effectful function reached under a HOST DELEGATION (`(host (E…) (go …))` where
        // `go` is recursive and performs a delegated `E.op`) is the ONE effect-context-monomorphization
        // combination not yet emitted — the spec body's `call` target would be missing from the
        // host-import assembly path (the clean decline at `compile_module`'s
        // `!specializations.is_empty() && !host_imports.is_empty()` guard). The spec body reconstructs
        // only HANDLER frames, NOT the enclosing host delegation, so the callee's perform of the
        // delegated op would resolve against a router stack with no Host frame and FALSE-reject CDZ0401
        // — a coded rejection of a VALID, granted program (04-capabilities.sexp §"an entrypoint
        // delegation reaches an effect performed in a recursive callee"). DECLINE cleanly here instead
        // (decline-don't-miscompile: a not-yet-emitted feature is a decline, never a false coded
        // rejection), matching the deferred-combination decline the assembly path already carries.
        if ctx.routers.iter().any(|r| matches!(r, RouterFrame::Host(_))) {
            return decline(
                "recursive effectful function under host delegation not yet emitted (a spec body cannot reconstruct the enclosing host delegation)",
            );
        }
        // The enclosing HANDLER frames whose state is threaded, outermost first — each contributes a
        // trailing state param/return. (Host delegations fold no state; a Unit-state handler threads
        // a zero-width value and contributes nothing.) Snapshot the arms + state kind so the
        // specialization can reconstruct the resolution context independent of the caller's env.
        let handlers: Vec<HandlerFrame> = ctx
            .routers
            .iter()
            .filter_map(|r| match r {
                RouterFrame::Handler(h) => Some(h.clone()),
                RouterFrame::Host(_) => None,
            })
            .collect();
        // The caller's current state local for each NON-UNIT threaded handler (to pass in), in the
        // same order the specialization expects its trailing params.
        let caller_state_locals: Vec<u32> = handlers
            .iter()
            .filter(|h| h.state_kind != Kind::Unit)
            .filter_map(|h| h.state_local)
            .collect();
        let state_kinds: Vec<Kind> =
            handlers.iter().map(|h| h.state_kind).filter(|k| *k != Kind::Unit).collect();
        if caller_state_locals.len() != state_kinds.len() {
            return decline("recursive effectful function: a threaded handler has no state local");
        }

        // EARLY runaway guard (before interning/emitting): a recursive function that installs a
        // FRESH handler wrapping its own recursive call grows the enclosing handler context by one
        // frame per recursion, so the context DEPTH climbs without bound — and each new depth
        // recursively emits another specialization body, so an over-high cap overflows the (small)
        // wasm compiler stack before a per-function count guard trips. Bound the context depth
        // directly and shallowly: no real program nests this many STATEFUL handlers around one
        // recursion (the deepest corpus case is 2), so tripping this means the context is growing
        // unboundedly. Decline cleanly — this is the general-one-shot/reified-continuation frontier.
        const MAX_HANDLER_CONTEXT_DEPTH: usize = 8;
        if handlers.len() > MAX_HANDLER_CONTEXT_DEPTH {
            return decline(format!(
                "recursive effectful function `{name}` grows its handler context without bound (a handler installed per recursive call) — beyond effect-context monomorphization"
            ));
        }

        // Get or create the specialization for (name, this handler context).
        let spec_pos = self.intern_specialization(name, &handlers)?;

        // Emit: original args, then each threaded current-state (local.get), then `call spec`.
        let mut c = Vec::new();
        for (arg, pk) in args.iter().zip(f.param_kinds.iter()) {
            let (ac, ak) = self.emit(arg, env, ctx)?;
            if ak != *pk {
                // A kind mismatch on a specialized recursive call is not the polymorphic-inline case
                // (that path is for non-recursive callees); decline honestly.
                return decline("specialized call argument kind mismatch");
            }
            c.extend_from_slice(&ac);
        }
        for &sl in &caller_state_locals {
            c.push(op::LOCAL_GET);
            uleb128(sl as u64, &mut c);
        }
        // Reserve caller locals to receive the returned next-states (multi-value results come off in
        // order result, s0, s1, …; the stack top is the LAST state, so store back in reverse).
        let recv: Vec<u32> = state_kinds.iter().map(|k| ctx.alloc_local(*k)).collect();
        let spec_wasm_idx = self.spec_wasm_index(spec_pos);
        c.push(op::CALL);
        uleb128(spec_wasm_idx as u64, &mut c);
        // Stack now: [result, s0', s1', …, sN'] with sN' on top. Pop into `recv` in reverse.
        for (i, &rl) in recv.iter().enumerate().rev() {
            let _ = i;
            c.push(op::LOCAL_SET);
            uleb128(rl as u64, &mut c);
        }
        // Copy each received next-state back into the caller's handler state local.
        for (&rl, &sl) in recv.iter().zip(caller_state_locals.iter()) {
            c.push(op::LOCAL_GET);
            uleb128(rl as u64, &mut c);
            c.push(op::LOCAL_SET);
            uleb128(sl as u64, &mut c);
        }
        // The result is left on the stack.
        Ok((c, f.ret_kind))
    }

    /// Get or create the specialization index for `name` under the handler context `handlers`. On
    /// first request the slot is RESERVED (body `None`) before the body is emitted, so a recursive
    /// self-call finds the reserved slot and emits a plain `call` (breaking the recursion). Returns
    /// the specialization's position in the registry.
    fn intern_specialization(&self, name: &str, handlers: &[HandlerFrame]) -> Result<usize, Decline> {
        let key = Self::context_key(
            &handlers.iter().cloned().map(RouterFrame::Handler).collect::<Vec<_>>(),
        );
        // Already interned (reserved or complete)?
        if let Some(pos) = self
            .specializations
            .borrow()
            .iter()
            .position(|s| s.fn_name == name && s.key == key)
        {
            return Ok(pos);
        }
        // Runaway guard: a recursive function that installs a FRESH handler wrapping its own
        // recursive call grows the handler context by one frame per recursion, so every self-call
        // has a DISTINCT context key and interning never converges — unbounded specializations,
        // which would overflow the compiler stack. That case is genuinely beyond effect-context
        // monomorphization (no finite set of specializations covers an unbounded context), so
        // DECLINE cleanly once a single function accumulates too many contexts. The bound is far
        // above any real program (the corpus needs 1–2 contexts per function); tripping it means the
        // context is growing without bound, never that a real program has that many handlers.
        const MAX_SPECS_PER_FN: usize = 64;
        if self
            .specializations
            .borrow()
            .iter()
            .filter(|s| s.fn_name == name)
            .count()
            >= MAX_SPECS_PER_FN
        {
            return decline(format!(
                "recursive effectful function `{name}` grows its handler context without bound (a handler installed per recursive call) — beyond effect-context monomorphization"
            ));
        }
        let f = self.lookup_fn(name).ok_or_else(|| Decline("spec: unknown fn".into(), None))?;
        let state_kinds: Vec<Kind> =
            handlers.iter().map(|h| h.state_kind).filter(|k| *k != Kind::Unit).collect();
        let spec_pos = {
            let mut specs = self.specializations.borrow_mut();
            let pos = specs.len();
            specs.push(Specialization {
                fn_name: name.to_string(),
                key: key.clone(),
                state_kinds: state_kinds.clone(),
                body: std::cell::RefCell::new(None),
                ret_kind: f.ret_kind,
                param_kinds: f.param_kinds.clone(),
            });
            pos
        };
        // Emit the body under the reconstructed handler context (now that the slot is reserved).
        let body = self.emit_specialization_body(name, handlers, &state_kinds)?;
        self.specializations.borrow()[spec_pos]
            .body
            .replace(Some(body));
        Ok(spec_pos)
    }

    /// Emit a specialization's body: the function under a reconstructed handler context, with each
    /// non-Unit handler's state bound to a trailing parameter local, returning `(result, states…)`.
    fn emit_specialization_body(
        &self,
        name: &str,
        handlers: &[HandlerFrame],
        state_kinds: &[Kind],
    ) -> Result<Body, Decline> {
        let f = self.lookup_fn(name).ok_or_else(|| Decline("spec body: unknown fn".into(), None))?;
        let arity = f.params.len() as u32;
        let n_state = state_kinds.len() as u32;
        let mut ctx = FnCtx {
            next_local: arity + n_state,
            extra_locals: Vec::new(),
            called: Default::default(),
            routers: Vec::new(),
            inlining: Vec::new(),
        };
        // Parameters occupy locals 0..arity; the threaded states occupy arity..arity+n_state.
        let env: Vec<Local> = f
            .params
            .iter()
            .cloned()
            .zip(f.param_kinds.iter().cloned())
            .enumerate()
            .map(|(idx, (pname, kind))| Local::scalar(pname, idx as u32, kind))
            .collect();
        // Reconstruct the router stack, remapping each non-Unit handler's state_local to its trailing
        // param. A Unit-state handler keeps `state_local = None` (threads nothing). def_env is empty
        // (arm bodies are self-contained functions of their op args + state; a corpus/self-hosting
        // handler arm does not close over caller locals). def_depth follows the reconstructed order.
        let mut state_param = arity;
        let mut routers = Vec::new();
        for (depth, h) in handlers.iter().enumerate() {
            let mut hf = h.clone();
            hf.def_env = Vec::new();
            hf.def_depth = depth;
            if hf.state_kind != Kind::Unit {
                hf.state_local = Some(state_param);
                state_param += 1;
            } else {
                hf.state_local = None;
            }
            routers.push(RouterFrame::Handler(hf));
        }
        ctx.routers = routers;
        // Mark this function as being specialized so a self-call takes the specialized path (and any
        // nested inline of a DIFFERENT effectful callee still works).
        ctx.inlining.push(name.to_string());
        let (mut code, kind) = self.emit(&f.body.clone(), &env, &mut ctx)?;
        ctx.inlining.pop();
        if kind != f.ret_kind && f.ret_kind != Kind::Never && kind != Kind::Never {
            // The body's emitted kind should match the function's return kind.
            return decline("specialization body kind disagrees with the function's return kind");
        }
        // Append the final state of each non-Unit handler as extra return values, in order — the
        // multi-value return `(result, s0, s1, …)`.
        for (i, k) in state_kinds.iter().enumerate() {
            let _ = k;
            code.push(op::LOCAL_GET);
            uleb128((arity + i as u32) as u64, &mut code);
        }
        Ok(Body { extra_locals: ctx.extra_locals, code })
    }

    /// The wasm function index of specialization `pos`: after the imports (`call_base`), the user
    /// functions, and the arithmetic helpers.
    fn spec_wasm_index(&self, pos: usize) -> u32 {
        self.call_base + self.funcs.len() as u32 + self.helper_count() as u32 + pos as u32
    }

    /// Emit a delegated operation `E.op` as a boundary host call. The delegated op's WIT signature
    /// is its declared `(-> T… R)`; the flat import name is `E.op` (host-interface-binding.md §A
    /// Host-Delegated Operation Imports Verbatim). Reuses the existing host-import lowering.
    fn gen_delegated_call(
        &self,
        eff: &str,
        op: &str,
        args: &[Node],
        env: &[Local],
        ctx: &mut FnCtx,
    ) -> Result<(Vec<u8>, Kind), Decline> {
        let name = format!("{eff}.{op}");
        let hi = match self.host_imports.iter().find(|h| h.name == name) {
            Some(h) => h.clone(),
            None => return decline(format!("delegated host op `{name}` not in the computed manifest")),
        };
        // A nullary-typed op (`ask : Unit -> Int64`) is performed with no args (`(ask.ask)`); pad a
        // missing trailing `Unit`-typed parameter with `unit`, matching the perform's supplied args
        // to the op's declared parameters (the same elision handlers allow).
        let mut supplied: Vec<Node> = args.to_vec();
        while supplied.len() < hi.params.len() && hi.params.get(supplied.len()) == Some(&Kind::Unit) {
            supplied.push(Node::Name("unit".into()));
        }
        // Reuse the host-call lowering, passing a synthetic `elems` (head + args).
        let mut elems = Vec::with_capacity(supplied.len() + 1);
        elems.push(Node::Name(name)); // head is unused by gen_host_call beyond arity/name
        elems.extend_from_slice(&supplied);
        self.gen_host_call(&hi, &elems, env, ctx)
    }
}

// ─── Inference context (unification over parameter type variables) ───────────────────

/// The state of inferring one function body: a borrow of the compiler (to read callee
/// signatures) and the parameter type variables being solved. Each variable is `(name,
/// Option<Kind>)` — `None` while unsolved, `Some(k)` once a use has forced it to a ground
/// kind. A single ground lattice (`Kind`) plus these variables is the monomorphic core of
/// Hindley-Milner the realized corpus needs; unifying a variable is recording its solution.
struct InferCtx<'a> {
    compiler: &'a Compiler,
    vars: Vec<(String, Option<Kind>)>,
}

impl<'a> InferCtx<'a> {
    /// Unify a parameter variable (by name) with a required ground kind. First constraint
    /// wins the solution; a later conflicting constraint is left as-is (the coarse lattice has
    /// no error channel here — a genuine conflict surfaces later as an emit-time decline). A
    /// non-parameter name is not a variable and imposes nothing.
    fn constrain(&mut self, name: &str, kind: Kind) {
        if let Some(slot) = self.vars.iter_mut().find(|(n, _)| n == name) {
            match slot.1 {
                None => slot.1 = Some(kind),
                // Heap UPGRADES a prior scalar guess. Order of constraint discovery is not
                // canonical: a threaded compound accumulator (`code-cat`'s `ys`) is constrained
                // to `Int64` by the recursive self-call `(code-cat t ys)` — which reads the
                // callee's still-defaulting 2nd param kind — BEFORE the match-result back-prop
                // sees it returned as a Heap value in the base arm. First-write-wins would lock
                // the weak Int64, the self-call re-imposes it every fixpoint pass, and the
                // parameter never becomes Heap — so the Heap argument at the call site forces
                // per-call INLINING of the recursive function and the compiler blows up. Letting
                // Heap win (the "more defined" kind, the same tie-break `if`/`match` result
                // inference already uses on branch disagreement) makes the solve order-independent
                // and converges the accumulator to Heap. A genuine Int64-vs-Heap conflict is a
                // type error caught at emit, so this never masks an ill-typed program.
                Some(Kind::Heap) => {}
                Some(_) if kind == Kind::Heap => slot.1 = Some(Kind::Heap),
                Some(_) => {}
            }
        }
    }

    /// Infer an expression's kind, threading unification constraints into the parameter
    /// variables. Returns the expression's ground kind (best-effort; `None` when not locally
    /// determinable — the caller defaults it).
    fn infer(&mut self, node: &Node) -> Option<Kind> {
        match node {
            Node::Int(_) => Some(Kind::Int64),
            Node::Bool(_) => Some(Kind::Bool),
            Node::Float(_) => Some(Kind::Float64),
            // A string literal is a runtime heap value (a Bytes-backed UTF-8 leaf), the analog of a
            // `(Bytes.of …)` construction — so its kind is `Heap`. An all-constant string used only
            // as a const-folded operand folds before a kind is consulted; this matters for a string
            // flowing as a genuine runtime value (a fn arg/return, a sum payload).
            Node::Str(_) => Some(Kind::Heap),
            Node::Name(n) if n == "unit" => Some(Kind::Unit),
            Node::Name(n) if n == "nan" || n == "NaN" => Some(Kind::Float64),
            // A parameter or let-bound reference: its kind is its variable's current solution
            // (unknown yet if unconstrained). Search from the END so the INNERMOST binding wins —
            // a `let` shadow is pushed after the params/outer binds, and shadowing is well-defined
            // (core-semantics.md #Shadowing Is Well-Defined). Using the FIRST match resolved `x` in
            // `(def (f x) (let ((x true)) x))` to the Int64 PARAM instead of the Bool shadow, so
            // `f`'s return kind inferred Int64 while the body emits a Bool `true` → the function's
            // signature mismatched its callers and the component FAILED wasm validation
            // (02-binding-and-control.sexp §"a let shadowing a parameter with a differently-typed
            // value is not an invalid component").
            Node::Name(n) => self.vars.iter().rev().find(|(v, _)| v == n).and_then(|(_, k)| *k),
            Node::List(elems) => self.infer_list(elems),
        }
    }

    fn infer_list(&mut self, elems: &[Node]) -> Option<Kind> {
        let head = match elems.first() {
            Some(Node::Name(h)) => h.as_str(),
            // A QUALIFIED constructor head `(. Type Variant)` applied — `(IntList.Cons …)` — builds
            // a runtime sum heap value, exactly like a bare constructor `(Some n)`. Recurse into
            // the payload for constraints, then report `Kind::Heap`.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(2)).map_or(false, is_constructor_name) =>
            {
                for e in &elems[1..] {
                    let _ = self.infer(e);
                }
                return Some(Kind::Heap);
            }
            // A `(Bytes.of …)` / `(Bytes.concat …)` / `(Bytes.compact …)` produces a runtime Bytes
            // heap value (the compiler's own I/O type). `Bytes.of (list b0 b1 …)`'s elements are
            // byte VALUES (Int64), not a runtime list — constrain each to Int64. `concat`/`compact`
            // take Bytes arguments (Heap). Report `Kind::Heap`. An all-constant one folds to the
            // baked-text path before a return kind is consulted, so this matters only for one
            // carrying a runtime element.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("Bytes")
                && matches!(name_of(hd.get(2)), Some("of") | Some("concat") | Some("compact")) =>
            {
                if name_of(hd.get(2)) == Some("of") {
                    if let Some(Node::List(lst)) = elems.get(1) {
                        if name_of(lst.first()) == Some("list") {
                            for b in &lst[1..] {
                                self.expect(b, Kind::Int64);
                            }
                        }
                    }
                } else {
                    for e in &elems[1..] {
                        let _ = self.infer(e);
                    }
                }
                return Some(Kind::Heap);
            }
            // A Bytes CONSUMER — `(Bytes.len b)`, `(Bytes.at b i)`, `(Bytes.slice b a n)` — reads a
            // Bytes value, so its first argument is a Bytes HEAP handle. Constraining it to
            // `Kind::Heap` is LOAD-BEARING: without it a recursive consumer's Bytes parameter
            // defaults to Int64, the Heap argument forces per-call INLINING of an unboundedly-
            // recursive function, and the compiler HANGS (the sum-match lesson, applied to Bytes).
            // With it the parameter is Heap so the recursive call emits a real runtime `call`.
            // `len` yields Int64; `at`/`slice` yield an Option (Heap). Index/length args are Int64.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("Bytes")
                && matches!(name_of(hd.get(2)), Some("len") | Some("at") | Some("slice")) =>
            {
                if let Some(b) = elems.get(1) {
                    self.expect(b, Kind::Heap);
                }
                for i in &elems[2..] {
                    self.expect(i, Kind::Int64);
                }
                return match name_of(hd.get(2)) {
                    Some("len") => Some(Kind::Int64),
                    _ => Some(Kind::Heap), // at/slice → Option (a runtime sum)
                };
            }
            // A String op — the runtime String rides the Bytes heap representation. A CONSUMER
            // (`byte-len`/`scalar-len`/`concat`/`to-bytes`/`at`/`slice`) reads a String, so its first
            // argument is a String HEAP handle: constraining it to `Kind::Heap` is the same
            // load-bearing rule as the Bytes/list consumers — without it a recursive String consumer's
            // parameter defaults to Int64, the Heap argument forces per-call INLINING, and the
            // compiler HANGS. `from-bytes` instead reads a BYTES handle (also Heap). Result kinds:
            // `byte-len`/`scalar-len` → Int64; `concat`/`to-bytes` → Heap; `at`/`slice`/`from-bytes`
            // → Heap (an Option, a runtime sum). Index/length args are Int64.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("String")
                && matches!(
                    name_of(hd.get(2)),
                    Some("byte-len") | Some("scalar-len") | Some("concat") | Some("to-bytes")
                        | Some("from-bytes") | Some("at") | Some("slice")
                ) =>
            {
                let op = name_of(hd.get(2));
                match op {
                    Some("concat") => {
                        self.expect(elems.get(1)?, Kind::Heap);
                        self.expect(elems.get(2)?, Kind::Heap);
                    }
                    Some("at") => {
                        self.expect(elems.get(1)?, Kind::Heap);
                        if let Some(i) = elems.get(2) {
                            self.expect(i, Kind::Int64);
                        }
                    }
                    Some("slice") => {
                        self.expect(elems.get(1)?, Kind::Heap);
                        for i in &elems[2..] {
                            self.expect(i, Kind::Int64);
                        }
                    }
                    // byte-len/scalar-len/to-bytes/from-bytes: one Heap operand.
                    _ => {
                        if let Some(a) = elems.get(1) {
                            self.expect(a, Kind::Heap);
                        }
                    }
                }
                return match op {
                    Some("byte-len") | Some("scalar-len") => Some(Kind::Int64),
                    _ => Some(Kind::Heap),
                };
            }
            // List growth/length ops. `List.push`/`List.update` yield a list (Heap); `List.len`
            // yields Int64. Every op that reads a list constrains its FIRST argument to `Kind::Heap`
            // — the same load-bearing rule as the Bytes/sum consumers, so a recursive list consumer
            // (a `build` accumulating into a list) emits a runtime `call` rather than inlining to a
            // compiler hang. Index args are Int64; the pushed/updated element is left unconstrained
            // (any boxable kind).
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("List")
                && matches!(name_of(hd.get(2)), Some("push") | Some("update") | Some("len") | Some("at") | Some("concat")) =>
            {
                let op = name_of(hd.get(2));
                if let Some(v) = elems.get(1) {
                    self.expect(v, Kind::Heap);
                }
                match op {
                    // push(v, elem): elem unconstrained; update(v, i, elem): i is Int64.
                    Some("update") => {
                        if let Some(i) = elems.get(2) {
                            self.expect(i, Kind::Int64);
                        }
                        if let Some(e) = elems.get(3) {
                            let _ = self.infer(e);
                        }
                    }
                    Some("push") => {
                        if let Some(e) = elems.get(2) {
                            let _ = self.infer(e);
                        }
                    }
                    // at(v, i): i is Int64; the result is an Option (Heap).
                    Some("at") => {
                        if let Some(i) = elems.get(2) {
                            self.expect(i, Kind::Int64);
                        }
                    }
                    // concat(a, b): the second operand is also a list (Heap) — constraining it is the
                    // load-bearing rule that keeps a recursive concat-consumer's list parameter Heap
                    // (so a self-call emits a runtime `call`, not a compile-time inline).
                    Some("concat") => {
                        if let Some(b) = elems.get(2) {
                            self.expect(b, Kind::Heap);
                        }
                    }
                    _ => {}
                }
                return match op {
                    Some("len") => Some(Kind::Int64),
                    _ => Some(Kind::Heap), // push/update/concat → list (Heap); at → Option (Heap)
                };
            }
            // `Int64.checked-*` / `wrapping-*`: both operands are Int64; `checked-*` → an Option (Heap),
            // `wrapping-*` → Int64. Constrain each operand so a recursive consumer's params infer right.
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && name_of(hd.get(1)) == Some("Int64")
                && matches!(name_of(hd.get(2)),
                    Some("checked-add") | Some("checked-sub") | Some("checked-mul")
                    | Some("wrapping-add") | Some("wrapping-sub") | Some("wrapping-mul")) =>
            {
                if let Some(a) = elems.get(1) { self.expect(a, Kind::Int64); }
                if let Some(b) = elems.get(2) { self.expect(b, Kind::Int64); }
                return match name_of(hd.get(2)) {
                    Some("wrapping-add") | Some("wrapping-sub") | Some("wrapping-mul") => Some(Kind::Int64),
                    _ => Some(Kind::Heap), // checked-* → Option (Heap)
                };
            }
            // `(Option.expect o msg)` / `(Result.expect r msg)`: the scrutinee is a runtime
            // Option/Result (Heap); the RESULT kind is the payload it unwraps to — `Int64` for a
            // concretely-Int Option (`Int64.checked-*`, `Bytes.at`), else the raw handle (Heap).
            // `expect_payload_kind` is the SAME classifier codegen uses, so the inferred return kind
            // matches the emitted body's (avoids a signature-vs-caller INVALID component).
            Some(Node::List(hd)) if name_of(hd.first()) == Some(".")
                && matches!(name_of(hd.get(1)), Some("Option") | Some("Result"))
                && name_of(hd.get(2)) == Some("expect") =>
            {
                if let Some(o) = elems.get(1) { self.expect(o, Kind::Heap); }
                return Some(expect_payload_kind(elems.get(1)?));
            }
            // A PERFORM `(E.op …)` of a declared effect: its kind is the op's declared RESULT kind
            // (a `Fresh.next` → Int64, a `Diag.collect` → Heap list). Recurse into arg operands for
            // their constraints. This lets a handle/host body's kind (which flows to `main`) reflect
            // the performed op's result — needed so a compound-returning perform makes `main` Heap.
            Some(Node::List(hd))
                if name_of(hd.first()) == Some(".")
                    && name_of(hd.get(1))
                        .zip(name_of(hd.get(2)))
                        .map_or(false, |(e, o)| {
                            self.compiler.effects.get(e).map_or(false, |d| d.op(o).is_some())
                        }) =>
            {
                for a in &elems[1..] {
                    let _ = self.infer(a);
                }
                let e = name_of(hd.get(1)).unwrap();
                let o = name_of(hd.get(2)).unwrap();
                return self.compiler.effects.get(e).and_then(|d| d.op(o)).map(|op| op.result);
            }
            _ => return None,
        };
        match head {
            // A tuple/record/list constructor produces a runtime heap value (M2). Recurse into
            // elements to gather their constraints, then report `Kind::Heap` as the result. An
            // all-constant one const-folds to the baked-text path before a return kind is
            // consulted, so this matters only for one carrying a runtime element (the
            // runtime-compound path).
            "tuple" | "list" => {
                for e in &elems[1..] {
                    let _ = self.infer(e);
                }
                Some(Kind::Heap)
            }
            "record" => {
                for f in &elems[1..] {
                    if let Node::List(kv) = f {
                        if let Some(v) = kv.get(1) {
                            let _ = self.infer(v);
                        }
                    }
                }
                Some(Kind::Heap)
            }
            // Arithmetic/bitwise: both operands and the result are Int64.
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" if elems.len() == 3 => {
                self.expect(&elems[1], Kind::Int64);
                self.expect(&elems[2], Kind::Int64);
                Some(Kind::Int64)
            }
            // Boolean connectives: every operand is a Bool and the result is a Bool
            // (core-semantics.md #Boolean Connectives Short-Circuit). Constrain each operand so a
            // parameter used only inside `(and p …)` infers as Bool, matching the desugared `if`.
            "and" | "or" if elems.len() == 3 => {
                self.expect(&elems[1], Kind::Bool);
                self.expect(&elems[2], Kind::Bool);
                Some(Kind::Bool)
            }
            "not" if elems.len() == 2 => {
                self.expect(&elems[1], Kind::Bool);
                Some(Kind::Bool)
            }
            // Ordering: operands share ONE ordered kind (Int64 or Bool — false < true), result
            // Bool. Unify the operands like `=` does rather than forcing Int64, so a Bool ordering
            // constrains its operands to Bool (core-semantics.md #Ordering Where Offered Is Total).
            "<" | ">" | "<=" | ">=" if elems.len() == 3 => {
                let a = self.infer(&elems[1]);
                let b = self.infer(&elems[2]);
                if let (Some(k), _) | (_, Some(k)) = (a, b) {
                    self.expect(&elems[1], k);
                    self.expect(&elems[2], k);
                }
                Some(Kind::Bool)
            }
            // Equality: operands share a kind (unify them), result Bool.
            "=" if elems.len() == 3 => {
                let a = self.infer(&elems[1]);
                let b = self.infer(&elems[2]);
                if let (Some(k), _) | (_, Some(k)) = (a, b) {
                    self.expect(&elems[1], k);
                    self.expect(&elems[2], k);
                }
                Some(Kind::Bool)
            }
            // `if`: condition is Bool; the two branches unify to the result kind.
            "if" if elems.len() == 4 => {
                self.expect(&elems[1], Kind::Bool);
                let t = self.infer(&elems[2]);
                let e = self.infer(&elems[3]);
                // A branch that is a bare parameter reference may have been constrained — e.g. to
                // `Heap` by a `String.concat`/`List.push`/`Bytes.*` use — DURING THE OTHER branch's
                // walk, but its kind was read here BEFORE that constraint landed. This is exactly a
                // tail-recursive accumulator whose base case returns the accumulator parameter bare
                // (`(if (< n 1) s (rep (String.concat s "x") (- n 1)))`) and whose recursive branch
                // is a bare self-call (reporting the callee's still-defaulting return kind): neither
                // branch independently reports `Heap` on the first read, so the return kind would
                // lock to `Int64` while the parameter converges to `Heap` — and the emit-time `if`
                // then sees `then:Heap`/`else:Int64` → "branches differ in kind". Re-read a
                // bare-`Name` branch's CURRENT variable kind (an O(1) lookup — no sub-tree re-walk,
                // so nested `if`s stay linear, not 2^depth) so the base case reflects the kind the
                // recursive branch pinned; the fixpoint then converges the return kind to `Heap`.
                // (The `match` twin is covered by its arm back-propagation + genuinely-`Heap`
                // recursive arms; a bare-self-call `match` accumulator would want the same re-read.)
                let t = if matches!(&elems[2], Node::Name(_)) { self.infer(&elems[2]) } else { t };
                let e = if matches!(&elems[3], Node::Name(_)) { self.infer(&elems[3]) } else { e };
                // When the branches disagree, PREFER `Kind::Heap`. This matters for a recursive
                // compound builder like `(if base v (build (List.push v …) …))`: on the first
                // fixpoint pass the recursive-call branch reports the callee's still-default Int64
                // return kind while the base branch `v` is already Heap, and a naive `t.or(e)`
                // (then-branch-biased) would lock the `if` — and thus the function's return — to
                // Int64, a fixpoint that never recovers. Heap is the "more defined" kind here (a
                // genuine compound value vs the unconstrained-parameter default), so a disagreement
                // resolves to Heap and the return kind converges to Heap on the next pass. Two equal
                // kinds (or one None) are unchanged.
                let k = unify_branch_kinds(t, e);
                if let Some(k) = k {
                    // Back-propagate the unified result kind to each branch — but ONLY when the
                    // branch is a bare parameter NAME (the pass-through accumulator that needs it, a
                    // base case `(if … s …)` returning the param). A COMPOUND branch was already
                    // fully walked by the `infer` calls above, which gathered all its constraints;
                    // re-`expect`ing it would re-`infer` the ENTIRE subtree again — and since each
                    // nested `if` does this for BOTH branches, a chain of `if`s re-walked its body
                    // 4^depth times, the compile-time blowup for deeply-nested conditionals (the
                    // other half of the let-nesting ceiling). A bare-name back-prop is O(1)
                    // (`constrain`), so nested `if`s stay linear.
                    self.expect_name_only(&elems[2], k);
                    self.expect_name_only(&elems[3], k);
                }
                k
            }
            "do" => self.infer(elems.last()?),
            // A `(handle <init> (arms…) body)` yields its BODY's kind; a `(host (Effect…) body)`
            // delegation yields its body's kind. Their result kind flows to `main`, so inference
            // must see through them — else a handle/host body that builds a runtime compound would
            // leave `main`'s ret_kind at the Int64 default and the runtime-compound path would not
            // engage (the emitted call_base would be wrong).
            "handle" if elems.len() == 4 => self.infer(&elems[3]),
            "host" if elems.len() == 3 => self.infer(&elems[2]),
            ":" if elems.len() == 3 => self.infer(&elems[1]),
            // `let` binds names to values; the body's kind is the let's kind. Bind each name
            // to its value's inferred kind so a name used in the body infers correctly (e.g. a
            // Bool call result bound by let and then returned).
            "let" if elems.len() >= 2 => {
                if let Some(Node::List(binds)) = elems.get(1) {
                    for pair in binds {
                        if let Node::List(kv) = pair {
                            if let (Some(Node::Name(name)), Some(vexpr)) = (kv.first(), kv.get(1)) {
                                if let Some(k) = self.infer(vexpr) {
                                    self.vars.push((name.clone(), Some(k)));
                                }
                            }
                        }
                    }
                }
                self.infer(elems.last()?)
            }
            // `match`: the result kind is what its arm bodies yield (unified). The scrutinee's
            // kind constrains any parameter used as the scrutinee.
            "match" if elems.len() >= 2 => {
                // A literal arm pattern constrains the scrutinee's kind: matching `b` against
                // `true`/`false` forces `b : Bool`, against an integer forces Int64. Propagate
                // that to the scrutinee (if it is a parameter) so its kind is inferred.
                //
                // A CONSTRUCTOR arm pattern — `(Some x)`, `(IntList.Cons (tuple h t))` — matches a
                // runtime SUM value, so it forces the scrutinee to `Kind::Heap` (a heap handle).
                // This is what lets a recursive consumer like `sm` (matching a linked list it did
                // not build at compile time) type its parameter as Heap, so the recursive call
                // `(sm t)` emits a real `call` rather than inlining to a compile-time stack
                // overflow. Without it the parameter defaults to Int64 and the Heap argument forces
                // per-call inlining of an unboundedly-recursive function.
                for arm in &elems[2..] {
                    if let Node::List(a) = arm {
                        if let Some(pk) = a.first().and_then(literal_pattern_kind) {
                            self.expect(&elems[1], pk);
                        } else if a.first().map_or(false, is_constructor_pattern) {
                            self.expect(&elems[1], Kind::Heap);
                        } else if let Some(binders) = irrefutable_tuple_binders(arm) {
                            // An IRREFUTABLE `(tuple b0 … bn)` pattern (every slot a name/`_`, no
                            // literal or constructor slot) binding a scrutinee that is NOT a directly-
                            // constructed `(tuple …)` — i.e. a runtime tuple RETURNED by a call, like
                            // `decode`'s `(match (decode-node …) ((tuple ast pos) ast))`. BIND each
                            // slot binder into `vars` with the kind recovered from the scrutinee's tuple
                            // elements: a scalar slot (the Int cursor `pos`) → its scalar kind; a
                            // compound slot (the heap `Ast` `ast`) → Heap. Without this the binders are
                            // unbound (→ default Int64), so an arm body returning a heap slot bare
                            // infers the match — and thus `decode`'s — result as Int64 instead of Heap,
                            // and a caller's `(match (decode b) ((Ast.Int n) …))` then takes the
                            // scalar-literal path and declines "runtime match with a non-literal
                            // pattern" (ask-77, the mutual-recursion sibling of ask-73). Slot kinds come
                            // from the same scrutinee navigation the emit path uses
                            // (`scrutinee_tuple_slot_kinds`), which inlines the producer with a recursion
                            // guard; an unprovable slot is a compound → Heap. GUARDED to a call-returned
                            // tuple (not an inline `(tuple n 9)`, which `reduce_tuple_match` handles at
                            // compile time and whose scalar `n` must NOT be forced Heap).
                            if !matches!(&elems[1], Node::List(s) if name_of(s.first()) == Some("tuple")) {
                                let scalar_slots = self
                                    .compiler
                                    .scrutinee_tuple_slot_kinds(&elems[1], &[], binders.len());
                                for (i, b) in binders.iter().enumerate() {
                                    if let Node::Name(nm) = b {
                                        if nm != "_" {
                                            let k = scalar_slots
                                                .as_ref()
                                                .and_then(|ks| ks.get(i).copied().flatten())
                                                .unwrap_or(Kind::Heap);
                                            self.vars.push((nm.clone(), Some(k)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = self.infer(&elems[1]); // gather any nested scrutinee constraints
                // The arm bodies all yield ONE result kind (the match's kind). Infer each, then —
                // as `if` does across its two branches — pick the unified kind, PREFERRING Heap
                // when the arms disagree (a genuine compound value is "more defined" than an
                // unconstrained parameter that defaulted to Int64). Then BACK-PROPAGATE that kind
                // to every arm body via `expect`, so an arm that merely RETURNS a parameter (a
                // pass-through base case like `((Code.CNil _) ys)`) constrains that parameter to
                // the result kind. Without this, a threaded compound accumulator (`code-cat`'s
                // `ys`, returned unchanged in the base arm and passed along in the recursive arm)
                // never gets a Heap constraint, defaults to Int64, and the Heap argument at the
                // call site forces per-call INLINING of the recursive function — the compile-time
                // blowup. (This is the match twin of the `if`-branch Heap-preference rule.)
                let mut result = None;
                for arm in &elems[2..] {
                    if let Node::List(a) = arm {
                        if a.len() == 2 {
                            let k = self.infer(&a[1]);
                            result = unify_branch_kinds(result, k);
                        }
                    }
                }
                // Then RE-READ each bare-`Name` arm body's CURRENT variable kind and re-unify — the
                // match twin of the `if`-form bare-branch re-read. A base arm that returns an
                // accumulator PARAMETER bare (`((FL.FNil _) out)`) is read on the first pass BEFORE a
                // later recursive arm (`((FL.FCons …) (recompute t (List.push out 7)))`) constrains
                // `out` to `Heap` via its `List.push`. So the base arm reported `out`'s stale default
                // (Int64) and the recursive arm reported the callee's still-defaulting return, locking
                // the match's result — and thus the function's return — to Int64 while `out` converges
                // to Heap; then `List.len` on the result declines "of a non-list value" (the GAP 3k
                // symptom). Re-reading is O(1) (a var lookup, no sub-tree re-walk), so nested matches
                // stay linear; the Heap-preferring `unify_branch_kinds` then converges the return to
                // Heap on the next fixpoint pass — exactly as the `if`-form fix does.
                for arm in &elems[2..] {
                    if let Node::List(a) = arm {
                        if a.len() == 2 {
                            if let Node::Name(_) = &a[1] {
                                let k = self.infer(&a[1]);
                                result = unify_branch_kinds(result, k);
                            }
                        }
                    }
                }
                if let Some(k) = result {
                    for arm in &elems[2..] {
                        if let Node::List(a) = arm {
                            if a.len() == 2 {
                                // Bare-name-only back-prop (as the `if` arm): a compound arm body was
                                // already walked by the `infer` above; re-`expect`ing it would
                                // re-`infer` the whole subtree (4^depth over nested matches). Only a
                                // pass-through bare-name arm needs the constraint.
                                self.expect_name_only(&a[1], k);
                            }
                        }
                    }
                }
                result
            }
            // A constructor application `(Some n)` produces a runtime sum heap value (M2 Phase C).
            // Recurse into the payload for its constraints, then report `Kind::Heap`. An
            // all-constant constructor folds before a return kind is consulted, so this matters
            // only for one carrying a runtime payload (the runtime-compound path).
            _ if is_constructor_name(head) => {
                for e in &elems[1..] {
                    let _ = self.infer(e);
                }
                Some(Kind::Heap)
            }
            // `(tuple.N t)` — positional access on a RUNTIME tuple: the operand is a heap array, so
            // constrain it to `Kind::Heap`, and the result is the N-th element's kind. Without this
            // the operand's kind is unconstrained and `main`'s ret_kind can misresolve; with it a
            // `(tuple.1 l)` returning a scalar element takes the scalar path (its element kind), and a
            // compound element stays Heap. The element kind comes from the operand's static shape;
            // unknown → `None` (defaults to Int64, the common scalar case). Gate `starts_with` first.
            _ if head.starts_with("tuple.") && elems.len() == 2 => {
                self.expect(&elems[1], Kind::Heap);
                None
            }
            // A call to a user function: each argument must match the callee's parameter kind
            // (propagating a caller-parameter's kind through the call), and the result is the
            // callee's return kind.
            _ => {
                if let Some(f) = self.compiler.lookup_fn(head) {
                    for (arg, pk) in elems[1..].iter().zip(f.param_kinds.iter()) {
                        self.expect(arg, *pk);
                    }
                    Some(f.ret_kind)
                } else {
                    None
                }
            }
        }
    }

    /// Record that `node` must have kind `k`: if `node` is a parameter reference, constrain its
    /// variable; otherwise just recurse to gather nested constraints.
    fn expect(&mut self, node: &Node, k: Kind) {
        if let Node::Name(n) = node {
            self.constrain(n, k);
            return;
        }
        // Recurse so nested parameter uses inside the operand are constrained too.
        let _ = self.infer(node);
    }

    /// Constrain `node` to kind `k` ONLY if it is a bare parameter name — no re-inference of a
    /// compound. Used for `if`/`match` result-kind back-propagation, where the branches were
    /// already fully `infer`red (all their constraints gathered): re-`expect`ing a compound branch
    /// would re-walk its whole subtree, and doing so for every branch of a nested conditional is
    /// exponential (4^depth). Only a bare pass-through name (an accumulator returned unchanged in a
    /// base case) needs the O(1) `constrain`.
    fn expect_name_only(&mut self, node: &Node, k: Kind) {
        if let Node::Name(n) = node {
            self.constrain(n, k);
        }
    }

}

// ─── Function-local codegen context ────────────────────────────────────────────────

/// A name in scope. Either a **scalar** bound to a wasm local (`idx`/`kind`), or a
/// **compile-time alias** to a node — a structural value (record/tuple/sum) or a pattern
/// binder's payload that has no scalar wasm representation and is instead resolved by
/// re-emitting the aliased node in the scope it was captured in. Structural values live
/// only at compile time (this is a scalar compiler); `match`, member access, and tuple
/// access consume them by inspecting the node, never materializing them at runtime.
#[derive(Clone)]
struct Local {
    name: String,
    idx: u32,
    kind: Kind,
    /// If present, this name is a compile-time alias for `(node, captured-env)` rather than
    /// a runtime local. Resolving it re-emits `node` under `env`.
    ///
    /// The captured env is `Rc`-SHARED, not deep-cloned. Each `let`/binder captures the scope it
    /// was written in; a plain `Vec<Local>` capture recursively contained every enclosing alias's
    /// OWN captured `Vec` → the env at depth d held ~2^d nested copies, so a body with ~30 nested
    /// `let`s (or an aliased-compound chain) blew compile-time memory into the GBs (the Tier-4 /
    /// "compile is 2ⁿ in let nesting" ceiling that gated the Cadenza compiler's growth). An `Rc`
    /// makes a capture a refcount bump and cloning a `Local` share the pointed-to env, so nested
    /// captures share structure instead of copying — the env is O(depth), not O(2^depth).
    alias: Option<(Node, std::rc::Rc<Vec<Local>>)>,
    /// For a MATERIALIZED runtime local of `Kind::Heap` (a `let`-bound compound whose value is a
    /// genuine runtime handle, not a compile-time alias), the static `Shape` of what it holds — so
    /// `shape_of` can see through the opaque handle to render/project it (e.g. `tuple.N` on a
    /// let-bound runtime tuple recovers the element's kind). `None` for scalars and aliases (whose
    /// shape comes from their kind / their aliased node).
    shape: Option<Shape>,
}

impl Local {
    fn scalar(name: String, idx: u32, kind: Kind) -> Local {
        Local { name, idx, kind, alias: None, shape: None }
    }
    fn scalar_shaped(name: String, idx: u32, kind: Kind, shape: Option<Shape>) -> Local {
        Local { name, idx, kind, alias: None, shape }
    }
    fn aliased(name: String, node: Node, env: Vec<Local>) -> Local {
        Local { name, idx: 0, kind: Kind::Unit, alias: Some((node, std::rc::Rc::new(env))), shape: None }
    }
}

/// The static structural type of a value, carrying the field/variant NAMES a rendering needs.
/// This is the type-directed information the TAG-FREE runtime does not hold: the compiler infers
/// it (from `main`'s body) and emits a renderer that walks the value through the runtime's
/// accessors, baking each keyword/name as a constant (component-abi.md §The Runtime Does Not Name
/// Or Render Values). Distinct from `Kind` (a scalar wasm-result lattice) and `StaticType` (a
/// non-recursive shape tag): `Shape` is the full recursive structure a renderer must walk.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Shape {
    Int,
    Bool,
    /// A tuple/record element that is itself a runtime float or string: the renderer for these is
    /// not emitted yet, so a compound carrying one DECLINES (decline-don't-miscompile), same as a
    /// runtime float scalar. `Str` is a forward reference — `shape_of` does not yet produce it (a
    /// runtime string leaf declines earlier), but the render generator already rejects it so the
    /// decline path is complete when string shapes arrive.
    Float,
    #[allow(dead_code)]
    Str,
    /// The empty product — unit renders as `unit`.
    Unit,
    /// A fixed-arity positional product; renders `(tuple e0 e1 …)`.
    Tuple(Vec<Shape>),
    /// A homogeneous, runtime-length sequence; renders `(list e0 e1 …)`. The element shape is
    /// shared by every element. At run time a list is backed by the value-heap runtime's 32-way
    /// radix trie (`vec-*`) — the representation that supports functional growth (`List.push`/
    /// `List.update`) as well as reading. That choice is unobservable (collections-and-text.md
    /// #A List's Representation Is Unspecified And Unobservable): a literal and a grown list are one
    /// type and render alike. The renderer walks the trie via `vec-len`/`vec-get`.
    List(Box<Shape>),
    /// A fixed field set, sorted by key; renders `(record (k0 v0) (k1 v1) …)`.
    Record(Vec<(String, Shape)>),
    /// A sum value: its variants in DECLARATION order (index = discriminant), each `(name,
    /// payload-shape)`. Renders `(VariantName payload)` — the renderer switches on the runtime
    /// discriminant to pick the arm, writes the name, and renders the payload with its shape. A
    /// nullary variant's payload shape is `Unit` (renders `unit`).
    Sum(Vec<(String, Shape)>),
    /// A back-reference to a RECURSIVE sum type by name (its full variant set is registered in the
    /// renderer's `type_shapes` map). A finite `Shape` tree cannot inline a recursive type (a
    /// `IntList = Cons (Tuple Int64 IntList) | Nil`, an AST) — its unrolled shape is infinite — so a
    /// recursive payload position holds `Rec(name)` instead of the type's expanded `Sum`. The
    /// renderer emits ONE render fn per recursive type (built from its declaration) and a `Rec(name)`
    /// payload lowers to a CALL back into that fn, so the walk recurses to the value's runtime depth.
    /// `sum_shape` produces it (from a self-referential `sum_payload_types` slot); `shape_of` never
    /// leaves one dangling (every `Rec(name)` a top shape reaches is registered before emission).
    Rec(String),
    /// A packed immutable byte buffer (the runtime `bytes-*` shape, index 13–16 in the heap WIT —
    /// NOT the positional array). Renders the byte-string display `b"…"` (a printable ASCII byte as
    /// itself, else a `\xNN`/named escape), byte-identical to the const `bytes_literal_text` and the
    /// `b"…"` reader's input form (options/byte-string-literal). This is the compiler's own I/O type
    /// — `compile: list<u8> -> result<list<u8>>` — so a Cadenza-authored compiler builds and consumes
    /// it at run time.
    Bytes,
}

impl Shape {
    /// From a scalar `Kind` (a leaf the runtime boxes directly).
    fn from_kind(k: Kind) -> Option<Shape> {
        match k {
            Kind::Int64 => Some(Shape::Int),
            Kind::Bool => Some(Shape::Bool),
            Kind::Float64 => Some(Shape::Float),
            Kind::Unit => Some(Shape::Unit),
            Kind::Never => None,
            Kind::Heap => None, // a heap value's shape is not a scalar kind — inferred structurally
            Kind::HostString => None, // host-boundary only; never a rendered result shape
        }
    }
}

/// Per-function state: the next free local index, the extra locals declared so far (beyond
/// parameters, in index order, for the body's local declaration), and the set of user-function
/// indices this body emits a *runtime* `call` to. The call set drives reachability-based dead-
/// function elimination: a function reached only by compile-time folding (never by an emitted
/// `call`) is dead and need not lower to wasm (see `compile_module`).
struct FnCtx {
    next_local: u32,
    extra_locals: Vec<Kind>,
    called: std::collections::BTreeSet<u32>,
    /// The router stack: the `(handle …)` handlers and `(host …)` delegations lexically enclosing
    /// the expression currently being emitted, innermost last. A perform `(E.op …)` resolves
    /// top-down (nearest first) over this stack (§Handler Resolution Is Dynamic In Extent And
    /// Statically Determined — here realized within a function; cross-function is by inlining, which
    /// pushes the callee's performs into the caller's stack). `gen_handle`/`gen_host` push a frame,
    /// emit the body, and pop. See `RouterFrame`.
    routers: Vec<RouterFrame>,
    /// The user functions currently being INLINED for cross-function effect resolution, innermost
    /// last. Cross-function effect resolution inlines an effectful callee into the handled region
    /// (§Effect-context monomorphization). A RECURSIVE effectful function would inline its own body
    /// without bound — a compile-time hang — so when a call names a function already on this stack,
    /// the inline path DECLINES instead (a recursive effectful function needs Stage-3
    /// monomorphization, not inlining; until then it is an honest todo, never a hang or a
    /// miscompile). Pushed before emitting an inlined effectful body, popped after.
    inlining: Vec<String>,
}

/// How a handler arm uses `resume`, deciding its lowering (options/effects-model/lowering-to-wasm.md
/// §Handler classification). Conservative: anything not provably `Tail` or `Abortive` is
/// `GeneralOneShot`, which declines (reject-don't-miscompile).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArmClass {
    /// `resume` occurs exactly once, in tail position; the continuation is not otherwise named.
    /// Lowered by inlining the arm body with `(resume value next-state)` → `value` (state threaded).
    Tail,
    /// `resume` never occurs — the handler discards its continuation (exception / early-exit shape).
    /// Tier 2 (`block`/`br`); not built yet, declines.
    Abortive,
    /// `resume` occurs but not in tail position, or the continuation is captured — needs a reified
    /// continuation (Tier 3). Not built yet, declines.
    GeneralOneShot,
}

/// One arm of a `(handle …)`: `(E.op (params…) state-binder arm-body)`. Each arm carries its OWN
/// effect (a single handle may discharge operations of more than one effect —
/// 14-effects-and-handlers.sexp §"two effects each declaring a same-named operation do not
/// collide"). Its body is stored with `resume` UNREWRITTEN; Tier-1 lowering rewrites
/// `(resume value next-state)` → `value` at each perform site (options/effects-model/lowering-to-wasm.md §Tier 1).
#[derive(Clone)]
struct HandlerArm {
    effect: String,
    op: String,
    params: Vec<String>,
    /// The state binder — the name bound to the current handler state, following the op params.
    state: String,
    body: Node,
    class: ArmClass,
}

/// A lexical `(handle <init> (arms…) body)` frame on the router stack. Its arms discharge
/// operations of one or more effects; a perform resolving to a matching effect+op arm dispatches on
/// the arm's class.
#[derive(Clone)]
struct HandlerFrame {
    arms: Vec<HandlerArm>,
    /// The environment captured AT THE HANDLE SITE, under which an arm body is emitted (so a name
    /// free in the arm resolves at the handle's scope, not the perform's).
    def_env: Vec<Local>,
    /// The router-stack depth AT THE HANDLE SITE — the under-frame. An arm body's nested performs
    /// resolve against `routers[..def_depth]` (the routers enclosing the handle), NOT against this
    /// frame or anything nearer the perform, so a forwarding/interposing arm re-performs to the
    /// PARENT router rather than recursing into itself
    /// (options/effects-model/lowering-to-wasm.md §the under-frame).
    def_depth: usize,
    /// The seed state's kind. `Unit` (seed `unit`, arm threads `s` unchanged) is the degenerate
    /// zero-cost case — no local, no threading, byte-identical to a stateless inline. A non-unit
    /// kind (a `Fresh` counter, a `Diag` list) is a real fold threaded through `state_local`.
    state_kind: Kind,
    /// The wasm local holding the current handler state, for a non-unit state fold. `None` for the
    /// unit-state fast path. Seeded to `<init>` by `gen_handle` before the body is emitted; each
    /// perform reads it (the arm's state binder) and, if the arm threads a `next-state`, writes it
    /// back — the mutation the value heap stays immutable under
    /// (capabilities-and-effects.md §A Handler Threads State Across The Operations It Discharges).
    state_local: Option<u32>,
}

/// An entrypoint `(host (Effect…) body)` delegation frame on the router stack: within `body`, the
/// named effects are routed to the component boundary as imported-function calls the host resolves
/// (capabilities-and-effects.md §Host-Binding Is A Routing Decision Made At The Entrypoint). The
/// host is their terminal handler.
#[derive(Clone)]
struct HostFrame {
    /// The effects this delegation names.
    effects: Vec<String>,
    /// Which named effects a reachable perform actually matched — a delegation naming an effect no
    /// perform reaches is CDZ0404 (latent authority). A `RefCell` because the perform site records
    /// a hit while emission holds `&self` on the enclosing router frame via `ctx` (which is `&mut`,
    /// but the frame is borrowed immutably during top-down resolution); recording through the cell
    /// avoids a borrow conflict.
    reached: std::cell::RefCell<std::collections::BTreeSet<String>>,
}

/// A frame on the router stack: a lexical `(handle …)` or an entrypoint `(host …)` delegation. Both
/// resolve by the same top-down nearest-enclosing rule, so a `handle` nearer a perform than an
/// enclosing `host` INTERPOSES on an otherwise-delegated effect.
#[derive(Clone)]
enum RouterFrame {
    Handler(HandlerFrame),
    Host(HostFrame),
}

impl FnCtx {
    fn alloc_local(&mut self, kind: Kind) -> u32 {
        let idx = self.next_local;
        self.next_local += 1;
        self.extra_locals.push(kind);
        idx
    }
}

// ─── Encoders for the pieces above ───────────────────────────────────────────────

/// A wasm functype: `0x60 <params-vec> <results-vec>`.
fn functype(params: &[Kind], result: Kind) -> Vec<u8> {
    let mut out = vec![0x60];
    let pbytes: Vec<u8> = params.iter().map(|k| k.core_valtype()).collect();
    out.extend_from_slice(&wasm_vec(params.len(), &pbytes));
    match result {
        Kind::Unit => out.extend_from_slice(&wasm_vec(0, &[])),
        k => out.extend_from_slice(&wasm_vec(1, &[k.core_valtype()])),
    }
    out
}

/// A functype for an effect-context SPECIALIZATION (Stage 3): parameters are the original params
/// followed by one hidden state param per threaded handler context; results are the original result
/// (if not Unit) followed by the same states — the multi-value return that threads each handler's
/// state on the call stack. A `Unit` original result contributes no result slot but the states
/// still return.
fn functype_spec(orig_params: &[Kind], state_kinds: &[Kind], result: Kind) -> Vec<u8> {
    let mut out = vec![0x60];
    // Params: orig params, then the state params.
    let mut params: Vec<u8> = orig_params.iter().map(|k| k.core_valtype()).collect();
    params.extend(state_kinds.iter().map(|k| k.core_valtype()));
    out.extend_from_slice(&wasm_vec(params.len(), &params));
    // Results: the original result (unless Unit), then each state.
    let mut results: Vec<u8> = Vec::new();
    if result != Kind::Unit {
        results.push(result.core_valtype());
    }
    results.extend(state_kinds.iter().map(|k| k.core_valtype()));
    out.extend_from_slice(&wasm_vec(results.len(), &results));
    out
}

// ─── Host-import component (capability lowering) ───────────────────────────────────────
//
// A program that imports host functions (`(import (host NAME (func …)))`) is emitted as a
// component that: imports each host function at the component boundary (its manifest = the set of
// imports, host-interface-binding.md §Imports Mirror The Manifest Exactly); lowers each into a
// core func the program module calls; and lifts the program's `run` export. The shape is derived
// byte-for-byte from a `wasm-tools`-validated reference (as the other envelopes are). It handles
// both scalar host funcs (no canon options) and string host funcs (canon options
// memory+realloc+utf8, marshalling the program's memory bytes into the host `string`), which is
// why the component instantiates a small SHARED-MEMORY core module first and threads its memory +
// realloc into both the lowers and the program module — breaking the canon-lowering circularity a
// single self-memory module would create.
//
// Core-func index layout inside the program module: the lowered host funcs occupy `0..n_host`
// (in declaration order), then `memory` is imported, then the user functions follow (shifted by
// `call_base = n_host`), then the arithmetic helpers, then `run` (= user func 0 = `main`, which
// is exported). So a host call emits `call <import-index>` (raw), a user call emits
// `call <index + n_host>`.

/// A component functype `0x40 <params-vec of (name, valtype)> <result>` for a host op. Params are
/// named `p0`, `p1`, … (Unit params are already stripped upstream); a Unit result is the
/// no-result form `0x01 0x00`, any other kind the single-result form `0x00 <valtype>`.
fn comp_functype(params: &[Kind], result: Kind) -> Vec<u8> {
    let mut ft = vec![0x40];
    uleb128(params.len() as u64, &mut ft);
    for (pi, pk) in params.iter().enumerate() {
        let pname = format!("p{pi}");
        uleb128(pname.len() as u64, &mut ft);
        ft.extend_from_slice(pname.as_bytes());
        ft.push(host_comp_valtype(*pk));
    }
    if result == Kind::Unit {
        ft.extend_from_slice(&[0x01, 0x00]); // no result
    } else {
        ft.push(0x00);
        ft.push(host_comp_valtype(result));
    }
    ft
}

/// The component-level valtype byte for a host boundary `Kind` (used in a component functype).
fn host_comp_valtype(k: Kind) -> u8 {
    match k {
        Kind::HostString => 0x73, // string
        Kind::Int64 => 0x78,      // s64
        Kind::Bool => 0x7f,       // bool
        Kind::Float64 => 0x75,    // f64
        _ => 0x7f,
    }
}

/// The core wasm valtypes a host boundary `Kind` lowers to (a string → two i32s: ptr, len).
fn host_core_valtypes(k: Kind) -> Vec<u8> {
    match k {
        Kind::HostString => vec![0x7f, 0x7f],
        Kind::Int64 => vec![0x7e],
        Kind::Bool => vec![0x7f],
        Kind::Float64 => vec![0x7c],
        Kind::Unit => vec![],
        _ => vec![0x7f],
    }
}

/// One delegated effect grouped for the host-import component: the effect's WIT-interface name and
/// its operations (each a boundary function within that interface). The component imports one
/// INSTANCE per effect (the interface = the component-model namespace, the op = a function in it),
/// so a dotted operation `log.emit` is `emit` inside interface `log` — dissolving the invalid
/// dotted extern-name and matching the value-heap runtime's `interface heap` shape
/// (host-interface-binding.md §A Host-Delegated Operation Imports Verbatim).
struct HostEffectGroup {
    effect: String,
    ops: Vec<HostImport>, // each HostImport.name is the FLAT "effect.op"; its bare op is after the dot
}

impl HostEffectGroup {
    /// The bare operation name (after the `effect.` prefix) — the function name inside the interface.
    fn op_name(hi: &HostImport) -> &str {
        hi.name.split_once('.').map(|(_, o)| o).unwrap_or(&hi.name)
    }
}

/// Group a flat `effect.op` host-import list by effect, preserving first-seen order for both the
/// effects and their operations, so the emitted interface-instance imports and the core-module
/// import indices agree with `gen_delegated_call`'s per-op index.
fn group_host_imports(host: &[HostImport]) -> Vec<HostEffectGroup> {
    let mut groups: Vec<HostEffectGroup> = Vec::new();
    for hi in host {
        let effect = hi.name.split_once('.').map(|(e, _)| e).unwrap_or(&hi.name).to_string();
        match groups.iter_mut().find(|g| g.effect == effect) {
            Some(g) => g.ops.push(hi.clone()),
            None => groups.push(HostEffectGroup { effect, ops: vec![hi.clone()] }),
        }
    }
    groups
}

/// Build the complete host-import component. `funcs`/`bodies` are the program's user functions
/// (bodies already emitted with `call_base = host.len()`); `helper_bodies` the arithmetic helpers;
/// `main_ret` the entry's boundary result kind. Each delegated effect is imported as an INSTANCE
/// (a WIT interface) whose exported functions are its operations — the effect is the component
/// namespace, the op a function within it (the value-heap runtime's `interface heap` shape).
fn host_import_component(
    host: &[HostImport],
    funcs: &[Func],
    bodies: &[Body],
    helper_bodies: &[Body],
    main_ret: Kind,
) -> Result<Vec<u8>, Decline> {
    let n_host = host.len();
    let groups = group_host_imports(host);
    let n_groups = groups.len();
    let mut comp = Vec::new();
    comp.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00]); // component magic

    // ── Per effect: a component type section (1 INSTANCE type declaring the effect's op funcs) then
    //    an import section importing an instance of that type under the effect's (kebab) name. The
    //    instance-type declares each op as a Type(Func …) followed by an Export naming it, and each
    //    op's Export references the op's func-type by its LOCAL index within the instance type. ──
    for (gi, g) in groups.iter().enumerate() {
        // Instance type: 0x42 <ndecls> [ 0x01 <functype> …per op ] [ 0x04 0x00 <name> 0x01 <local> …per op ]
        let mut it = vec![0x42];
        uleb128((g.ops.len() * 2) as u64, &mut it); // one Type decl + one Export decl per op
        for hi in &g.ops {
            it.push(0x01); // a Type declaration
            it.extend_from_slice(&comp_functype(&hi.params, hi.result));
        }
        for (oi, hi) in g.ops.iter().enumerate() {
            it.push(0x04); // an Export declaration
            it.push(0x00); // export-name kind (plain)
            let op = HostEffectGroup::op_name(hi);
            uleb128(op.len() as u64, &mut it);
            it.extend_from_slice(op.as_bytes());
            it.push(0x01); // sort: func
            uleb128(oi as u64, &mut it); // local func-type index within the instance type
        }
        comp.extend_from_slice(&section(7, &wasm_vec(1, &it)));
        // Import an instance of that type under the effect's name: 0x00 <name> 0x05 <instance-typeidx>.
        let mut imp = vec![0x00];
        uleb128(g.effect.len() as u64, &mut imp);
        imp.extend_from_slice(g.effect.as_bytes());
        imp.push(0x05); // import-kind: instance (type)
        uleb128(gi as u64, &mut imp); // the instance type just defined (component type index gi)
        comp.extend_from_slice(&section(10, &wasm_vec(1, &imp)));
    }

    // ── Shared-memory core module (module 0). ──
    comp.extend_from_slice(&section(1, HOST_MEM_MODULE));
    // ── Core instance 0: instantiate module 0 with no args. ──
    comp.extend_from_slice(&section(2, &wasm_vec(1, &[0x00, 0x00, 0x00])));
    // ── Component alias: memory (core mem 0) + cabi_realloc (core func 0) from core instance 0. ──
    let mut aliases = Vec::new();
    aliases.extend_from_slice(&[0x00, 0x02, 0x01, 0x00]); // alias core-instance-export memory, inst 0
    aliases.extend_from_slice(&[0x06]);
    aliases.extend_from_slice(b"memory");
    aliases.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]); // alias core-instance-export func, inst 0
    aliases.extend_from_slice(&[0x0c]);
    aliases.extend_from_slice(b"cabi_realloc");
    comp.extend_from_slice(&section(6, &wasm_vec(2, &aliases)));

    // ── Component aliases: pull each op function out of its imported instance, in host-import order
    //    (so the canon lowers below number them 0..n_host in the same order the program calls). An
    //    instance-export alias: <sort=0x01 func> <target=0x00 instance-export> <instance-idx> <name>. ──
    let mut fn_aliases = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for hi in &g.ops {
            fn_aliases.push(0x01); // sort: func (component)
            fn_aliases.push(0x00); // target: instance-export
            uleb128(gi as u64, &mut fn_aliases); // the imported instance index (== component type/import index)
            let op = HostEffectGroup::op_name(hi);
            uleb128(op.len() as u64, &mut fn_aliases);
            fn_aliases.extend_from_slice(op.as_bytes());
        }
    }
    comp.extend_from_slice(&section(6, &wasm_vec(n_host, &fn_aliases)));

    // ── Canon lowers: one per host op (in host-import order). A string-using op carries
    //    memory/realloc/utf8 options; the lowered core funcs follow cabi_realloc(core func 0). ──
    let mut lowers = Vec::new();
    for (i, hi) in host.iter().enumerate() {
        let uses_string =
            hi.params.iter().any(|p| *p == Kind::HostString) || hi.result == Kind::HostString;
        let mut low = vec![0x01, 0x00]; // lower, canon-lower tag
        uleb128(i as u64, &mut low); // component func index being lowered (the aliased op func i)
        if uses_string {
            // 3 options: memory(0x03) mem-idx 0, realloc(0x04) func-idx 0, utf8(0x00).
            low.extend_from_slice(&[0x03, 0x03, 0x00, 0x04, 0x00, 0x00]);
        } else {
            low.push(0x00); // 0 options
        }
        lowers.push(low);
    }
    comp.extend_from_slice(&section(8, &wasm_vec(n_host, &lowers.concat())));

    // ── The program core module (module 1), built here. Its imports are grouped by effect module
    //    (interface name) + memory. ──
    let prog = host_program_core_module(host, funcs, bodies, helper_bodies)?;
    comp.extend_from_slice(&section(1, &prog));

    // ── Core instance section: one FromExports instance PER effect (naming its op funcs), one for
    //    memory, then the instantiation of the program module wiring each effect module + memory. The
    //    lowered op core funcs are 1..=n_host (cabi_realloc is core func 0). ──
    let mut instances = Vec::new();
    let mut n_instances = 0usize;
    // Effect instances: FromExports listing each op func under its bare op name.
    let mut core_fn = 1u32; // lowered op funcs start at core func 1 (0 = cabi_realloc)
    for g in &groups {
        let mut fe = vec![0x01]; // kind: from-exports
        uleb128(g.ops.len() as u64, &mut fe);
        for hi in &g.ops {
            let op = HostEffectGroup::op_name(hi);
            uleb128(op.len() as u64, &mut fe);
            fe.extend_from_slice(op.as_bytes());
            fe.push(0x00); // kind: func
            uleb128(core_fn as u64, &mut fe);
            core_fn += 1;
        }
        instances.extend_from_slice(&fe);
        n_instances += 1;
    }
    // Memory instance (core instance index n_groups+1): exports the shared memory. The only core
    // memory in scope is core mem 0 (aliased from the shared-mem module instance above).
    let mem_inst_idx = (1 + n_groups) as u32; // core instances: 0 = mem module; 1..=n_groups = effects; then memory
    {
        let mut fe = vec![0x01];
        uleb128(1, &mut fe);
        fe.extend_from_slice(&[0x06]);
        fe.extend_from_slice(b"memory");
        fe.extend_from_slice(&[0x02, 0x00]); // kind memory, core mem 0
        instances.extend_from_slice(&fe);
        n_instances += 1;
    }
    // Instantiate the program module (module 1), wiring each effect module by interface name + "" memory.
    {
        let mut inst = vec![0x00];
        uleb128(1, &mut inst); // module index 1
        uleb128((n_groups + 1) as u64, &mut inst); // args: one per effect + memory
        for (gi, g) in groups.iter().enumerate() {
            uleb128(g.effect.len() as u64, &mut inst);
            inst.extend_from_slice(g.effect.as_bytes());
            inst.push(0x12); // kind: instance
            uleb128((1 + gi) as u64, &mut inst); // effect instance index (1.. after mem module inst 0)
        }
        inst.push(0x00); // arg name "" (memory)
        inst.push(0x12); // kind: instance
        uleb128(mem_inst_idx as u64, &mut inst);
        instances.extend_from_slice(&inst);
        n_instances += 1;
    }
    comp.extend_from_slice(&section(2, &wasm_vec(n_instances, &instances)));

    // The program core instance is the last one added.
    let prog_inst_idx = (1 + n_groups + 1) as u32; // mem(0) + effects(1..=n_groups) + memory-inst + program

    // ── Component type for the run export: () -> <result> (or () -> () for unit). ──
    let mut run_ty = vec![0x40, 0x00]; // 0 params
    if main_ret == Kind::Unit {
        run_ty.extend_from_slice(&[0x01, 0x00]);
    } else {
        run_ty.push(0x00);
        run_ty.push(host_comp_valtype(main_ret));
    }
    comp.extend_from_slice(&section(7, &wasm_vec(1, &run_ty)));

    // ── Component alias: core func `run` from the program core instance. ──
    let mut run_alias = vec![0x00, 0x00, 0x01]; // core sort, func kind, instance-export target
    uleb128(prog_inst_idx as u64, &mut run_alias);
    run_alias.push(0x03);
    run_alias.extend_from_slice(b"run");
    comp.extend_from_slice(&section(6, &wasm_vec(1, &run_alias)));

    // ── Canon lift: lift the core `run` func as a component func. The aliased core funcs are:
    //    cabi_realloc(0), the op lowers(1..=n_host), then the run alias (n_host+1). ──
    let run_core_idx = (n_host + 1) as u64;
    let mut lift = vec![0x00, 0x00]; // lift, tag
    uleb128(run_core_idx, &mut lift);
    lift.push(0x00); // 0 canon options
    // type index of the run component functype: the effect instance types are 0..n_groups, this is n_groups.
    uleb128(n_groups as u64, &mut lift);
    comp.extend_from_slice(&section(8, &wasm_vec(1, &lift)));

    // ── Component export "run". Component func index space: aliased op funcs 0..n_host, then the
    //    lifted `run` at n_host. ──
    let mut export = vec![0x00]; // export name kind
    uleb128(3, &mut export);
    export.extend_from_slice(b"run");
    export.push(0x01); // sort: func
    uleb128(n_host as u64, &mut export); // component func index of the lifted run
    export.push(0x00);
    comp.extend_from_slice(&section(11, &wasm_vec(1, &export)));

    Ok(comp)
}

/// Build the program core module for the host-import path: imports each host op (lowered) from its
/// EFFECT MODULE (the interface name) by its bare op name, plus memory, defines the user functions +
/// helpers, exports `run` (= main = user func 0). The two-level `(effect-module, op)` import name
/// mirrors the component's interface-instance imports; `groups` gives each op its interface module.
fn host_program_core_module(
    host: &[HostImport],
    funcs: &[Func],
    bodies: &[Body],
    helper_bodies: &[Body],
) -> Result<Vec<u8>, Decline> {
    let n_host = host.len();
    let n_user = funcs.len();
    let n_helpers = helper_bodies.len();

    // ── Type section: one per host import (its core signature), one per user func, one per helper. ──
    let mut types = Vec::new();
    let mut n_types = 0usize;
    for hi in host {
        // core signature: params flattened (string→i32,i32), result (string→i32, unit→none).
        let mut params = Vec::new();
        for p in &hi.params {
            params.extend_from_slice(&host_core_valtypes(*p));
        }
        let mut ft = vec![0x60];
        ft.extend_from_slice(&wasm_vec(params.len(), &params));
        let res = host_core_valtypes(hi.result);
        ft.extend_from_slice(&wasm_vec(res.len(), &res));
        types.extend_from_slice(&ft);
        n_types += 1;
    }
    let ty_user_base = n_types;
    for f in funcs {
        types.extend_from_slice(&functype(&f.param_kinds, f.ret_kind.externalized()));
        n_types += 1;
    }
    let ty_helper_base = n_types;
    let helper_ty = functype(&[Kind::Int64, Kind::Int64], Kind::Int64);
    for _ in 0..n_helpers {
        types.extend_from_slice(&helper_ty);
        n_types += 1;
    }
    let type_sec = section(1, &wasm_vec(n_types, &types));

    // ── Import section: each host op from its EFFECT MODULE (interface name) by its bare op name,
    //    in host order (so an op's core func index equals its host-import index — what
    //    `gen_delegated_call` emits), then memory from "". ──
    let mut imports = Vec::new();
    for (i, hi) in host.iter().enumerate() {
        let (module, op) = hi.name.split_once('.').unwrap_or(("", hi.name.as_str()));
        uleb128(module.len() as u64, &mut imports);
        imports.extend_from_slice(module.as_bytes());
        uleb128(op.len() as u64, &mut imports);
        imports.extend_from_slice(op.as_bytes());
        imports.push(0x00); // import kind: func
        uleb128(i as u64, &mut imports); // type index i
    }
    // memory import: module "", name "memory", kind mem, limits {min 1}.
    imports.push(0x00);
    imports.push(0x06);
    imports.extend_from_slice(b"memory");
    imports.extend_from_slice(&[0x02, 0x00, 0x01]); // kind mem, flags 0, min 1
    let import_sec = section(2, &wasm_vec(n_host + 1, &imports));

    // ── Function section: user funcs use type ty_user_base+i, helpers ty_helper_base+h. ──
    let mut func_items = Vec::new();
    for u in 0..n_user {
        uleb128((ty_user_base + u) as u64, &mut func_items);
    }
    for h in 0..n_helpers {
        uleb128((ty_helper_base + h) as u64, &mut func_items);
    }
    let func_sec = section(3, &wasm_vec(n_user + n_helpers, &func_items));

    // ── Export section: run = user func 0 (main), at core index n_host + 0. ──
    let run_idx = n_host as u32; // imports occupy 0..n_host, main is the first defined func
    let export_sec = section(7, &wasm_vec(1, &export_entry("run", 0x00, run_idx)));

    // ── Code section: user bodies then helper bodies. ──
    let mut code_items = Vec::new();
    for b in bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for hb in helper_bodies {
        code_items.extend_from_slice(&encode_body(hb));
    }
    let code_sec = section(10, &wasm_vec(n_user + n_helpers, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// Encode a function body for the code section: `<size-uleb> <locals> <code> end`.
/// Each extra local is declared as its own group (`1 <valtype>`) so mixed kinds are exact.
fn encode_body(b: &Body) -> Vec<u8> {
    let mut inner = Vec::new();
    // local declarations
    uleb128(b.extra_locals.len() as u64, &mut inner);
    for k in &b.extra_locals {
        uleb128(1, &mut inner);
        inner.push(k.core_valtype());
    }
    inner.extend_from_slice(&b.code);
    inner.push(op::END);
    let mut out = Vec::new();
    uleb128(inner.len() as u64, &mut out);
    out.extend_from_slice(&inner);
    out
}

/// Wrap a core module in the component-model envelope, presenting core func 0 (`run`) as a
/// component export `run : () -> <comp-valtype>`. These envelope bytes are the fixed shape
/// validated end-to-end at ignition; only the result encoding varies with the return kind.
fn wrap_component(core: &[u8], ret: Kind) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x0D, 0x00, 0x01, 0x00]); // component magic

    // Section 1: the embedded core module.
    out.push(1);
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(core);

    // Section 2 (core instance): instantiate module 0 with no args.
    out.extend_from_slice(&[2, 4, 1, 0, 0, 0]);

    // Section 7 (component type): one functype () -> result. The unit form has result-flag
    // `1` (no result) — verbatim from the wasm-tools reference `func()` component type.
    if ret == Kind::Unit {
        out.extend_from_slice(&[7, 5, 1, 0x40, 0, 1, 0]); // 0 params, result flag 1 (none)
    } else {
        out.extend_from_slice(&[7, 5, 1, 0x40, 0, 0, ret.comp_valtype()]); // 0 result: unnamed valtype
    }

    // Section 6 (canon lift of core export "run").
    out.extend_from_slice(&[6, 9, 1, 0, 0, 1, 0, 3, b'r', b'u', b'n']);
    // Section 8 (component func / start-less alias).
    out.extend_from_slice(&[8, 6, 1, 0, 0, 0, 0, 0]);
    // Section 11 (component export "run").
    out.extend_from_slice(&[11, 9, 1, 0, 3, b'r', b'u', b'n', 1, 0, 0]);
    out
}

// `RUNNABLE_ENVELOPE_TAIL` — the FIXED resource-with-display envelope (the wit-bindgen
// resource-linking pattern: a `value` resource, `make`/`[method]value.display` lifts, a nested
// inner component, and the `cadenza:run/run` export) — is GENERATED into `heap_envelope.rs` by
// `xtask build` (wasm-encoder-built + wasmparser-validated, split at the core-module boundary).
// The compiler writes the component preamble + its own core module, then appends this tail.

// ─── The compound value heap lives in the RUNTIME (M2) ──────────────────────────────
//
// The runtime value heap lives ENTIRELY in the runtime component, behind the opaque `u32`
// handle: the emitted program never dereferences a heap value, it only threads handles between
// the runtime's constructors/accessors. So the compiler holds NO heap-object layout — no tag
// word, no field offsets — that is the runtime's private, tag-free representation (which the
// runtime engineer may optimize freely without changing an emitted byte). The program's own
// linear memory is used ONLY to assemble the output string (the type-directed renderer writes
// bytes there), bump-allocated above `HEAP_BASE`.
//
// `HEAP_BASE` is still the const-string `runnable_component` path's scratch base (that path
// bakes a constant string and needs a small return-pair + data area); the runtime-compound path
// assembles its output string from offset 16 (see `emit_renderer`).

/// Base offset of the bump allocator's arena in the CONST-STRING path (and initial value of its
/// bump global). The low bytes below this are the `(ptr,len)` return-pair scratch and small
/// fixed scratch the const-string `display` uses; the string data grows upward from here.
const HEAP_BASE: i64 = 64;

// ─── The runtime-compound component shape (M2 Phase B) ───────────────────────────────
//
// A program whose `main` produces a runtime-built compound (a tuple carrying a runtime
// element, …) is emitted as a component that IMPORTS the value-heap runtime interface
// `cadenza:runtime/heap` and the host COMPOSES (component-abi.md §The Value-Heap Runtime).
// The runtime is a NAME-FREE structural store (constructors + accessors, no render); the
// program itself carries the type-directed renderer the compiler emits, so a compound result
// crosses the boundary as an ordinary `string` the program's `run` returns (§A Compound
// Result Is Rendered By Compiler-Emitted Code). The host reads that string back through the
// existing `() -> string` path.
//
// The component's fixed surround — the import of the 32 heap funcs, the canon-lowers, the
// core-instance instantiation threading the lowered funcs back in, and the `run -> string`
// lift/export — is emitted VERBATIM from a `wasm-tools`-validated reference (custom name
// sections stripped), exactly as the `runnable_component` envelope is. The compiler builds
// components ITSELF; `wasm-tools` is a dev-desk oracle used to derive these constants, never a
// compile-time dependency. Only the program core module's bodies vary. The envelope lowers 32
// of the runtime's non-`string` functions; `str-new`/`str-get` are NOT lowered (their `string`
// marshaling needs a heavier canon), so a runtime STRING leaf declines for now.
//
// Core-module function index layout the emitter below builds (imports occupy the low index
// space, so every DEFINED function's index is shifted by `RT_FUNC_BASE`):
//   0..=31  the imported heap funcs (the 32 non-string ops, in import order) — absolute, never offset
//   32      cabi_realloc   33  putu   34  itoa   (fixed helper bodies)
//   35..    the user functions (main = index 0 → wasm 35), then the arithmetic helpers
//   then    the per-shape render functions the compiler emits for THIS program
//   last    run (calls main @ RT_FUNC_BASE and the top-level render fn)
//
// There is NO fixed `render` import/helper anymore: the renderer is TYPE-DIRECTED code the
// compiler emits per program (one function per distinct `Shape` node), because a tag-free,
// name-free runtime cannot render (it holds neither a type tag nor field/variant names). Only
// HEAD/TAIL — the program-independent component-model surround — are baked byte constants.

// `mod himport` (the import indices), `RT_N_IMPORTS`, `RT_HEAD`/`RT_TAIL`/`RT_IMPORT_CONTENT`/
// `RT_MEM`/`RT_GLOBAL`, `RT_TAIL_PREFIX_LEN`, and `rt_import_types()` are all GENERATED from the
// runtime WIT into `heap_envelope.rs` (imported at the top of this module). They are the compiler's
// view of the runtime contract; edit the WIT + `xtask build`, never `heap_envelope.rs` by hand.

/// How many fixed helper funcs precede the user functions (realloc, putu, itoa).
const RT_FIXED_FUNCS: usize = 3;

/// The wasm index of the first DEFINED (non-import) USER function: 32 heap imports + 3 fixed
/// helpers (realloc, putu, itoa). A user function's wasm index is `RT_FUNC_BASE + its 0-based
/// index`; `main` (user index 0) is at `RT_FUNC_BASE`.
const RT_FUNC_BASE: u32 = RT_N_IMPORTS + RT_FIXED_FUNCS as u32; // 35


/// Build the complete runtime-compound component: a program that imports `cadenza:runtime/heap`
/// and whose `run` renders `main`'s heap value to a string by walking it through the runtime's
/// accessors (a TYPE-DIRECTED, tag-free renderer the compiler emits per program). The core
/// module is built ENTIRELY here (types/imports/funcs/code); only the HEAD/TAIL surround is a
/// baked constant. Function index layout: 24 imports, then realloc/putu/itoa (24/25/26), then
/// the user functions (main = 27), then the arithmetic helpers, then the per-shape render
/// functions, then `run` (last). `render_bodies` are the type-directed render functions (each
/// `(handle:i32, cursor:i32) -> i32`); `run_body` is `run` (`() -> i32`), both already emitted
/// with the correct absolute call indices by `emit_renderer`.
fn runtime_compound_component(
    funcs: &[Func],
    user_bodies: &[Body],
    helper_bodies: &[Body],
    spec_type_items: &[u8],
    n_specs: usize,
    spec_bodies: &[Body],
    render_bodies: &[Body],
    run_body: &Body,
) -> Vec<u8> {
    let n_user = user_bodies.len();
    let n_helpers = helper_bodies.len();
    let n_render = render_bodies.len();

    // ── Type section: 24 import types, then realloc/putu/itoa, then one per user func, then one
    // per helper, then one per render fn (all `(i32,i32)->i32`), then run (`()->i32`). ──
    let mut type_items = Vec::new();
    let mut n_types = 0usize;
    for t in rt_import_types() {
        type_items.extend_from_slice(&t);
        n_types += 1;
    }
    let ty_realloc = n_types as u32;
    type_items.extend_from_slice(&functype(&[Kind::Bool, Kind::Bool, Kind::Bool, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_putu = n_types as u32; // (i64,i32)->i32 ; itoa shares it
    type_items.extend_from_slice(&functype(&[Kind::Int64, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_user_base = n_types as u32;
    for f in funcs {
        type_items.extend_from_slice(&functype(&f.param_kinds, f.ret_kind.externalized()));
        n_types += 1;
    }
    let ty_helper_base = n_types as u32;
    let helper_ty = functype(&[Kind::Int64, Kind::Int64], Kind::Int64);
    for _ in 0..n_helpers {
        type_items.extend_from_slice(&helper_ty);
        n_types += 1;
    }
    // Effect-context specialization types (ask-49): after helpers, before render fns — matching the
    // func layout `spec_wasm_index` assumes (`[fixed][user][helpers][specs][render][run]`).
    let ty_spec_base = n_types as u32;
    type_items.extend_from_slice(spec_type_items);
    n_types += n_specs;
    let ty_render = n_types as u32; // (i32,i32)->i32, shared by every render fn
    let render_ty = functype(&[Kind::Bool, Kind::Bool], Kind::Bool);
    if n_render > 0 {
        type_items.extend_from_slice(&render_ty);
        n_types += 1;
    }
    let ty_run = n_types as u32; // ()->i32
    type_items.extend_from_slice(&functype(&[], Kind::Bool));
    n_types += 1;
    let type_sec = section(1, &wasm_vec(n_types, &type_items));

    // ── Import section (fixed 24 heap imports). ──
    let import_sec = section(2, RT_IMPORT_CONTENT);

    // ── Function section: realloc, putu, itoa, user types, helper types, render types, run. ──
    let mut func_items = Vec::new();
    uleb128(ty_realloc as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items); // itoa shares putu's shape
    for u in 0..n_user {
        uleb128((ty_user_base + u as u32) as u64, &mut func_items);
    }
    for h in 0..n_helpers {
        uleb128((ty_helper_base + h as u32) as u64, &mut func_items);
    }
    for s in 0..n_specs {
        uleb128((ty_spec_base + s as u32) as u64, &mut func_items);
    }
    for _ in 0..n_render {
        uleb128(ty_render as u64, &mut func_items);
    }
    uleb128(ty_run as u64, &mut func_items);
    let n_funcs = RT_FIXED_FUNCS + n_user + n_helpers + n_specs + n_render + 1; // +1 for run
    let func_sec = section(3, &wasm_vec(n_funcs, &func_items));

    // ── Memory + global (fixed). ──
    let mem_sec = section(5, RT_MEM);
    let glob_sec = section(6, RT_GLOBAL);

    // ── Export section: memory (mem 0), cabi_realloc (realloc func), run (last func, after specs+render). ──
    let realloc_idx = RT_REALLOC; // first defined func
    let run_idx = RT_FUNC_BASE + (n_user + n_helpers + n_specs + n_render) as u32;
    let mut exports = Vec::new();
    exports.extend_from_slice(&export_entry("memory", 0x02, 0));
    exports.extend_from_slice(&export_entry("cabi_realloc", 0x00, realloc_idx));
    exports.extend_from_slice(&export_entry("run", 0x00, run_idx));
    let export_sec = section(7, &wasm_vec(3, &exports));

    // ── Code section: realloc/putu/itoa, user bodies, helper bodies, render bodies, run. ──
    let mut code_items = Vec::new();
    code_items.extend_from_slice(&encode_body(&rt_realloc_body()));
    code_items.extend_from_slice(&encode_body(&rt_putu_body()));
    code_items.extend_from_slice(&encode_body(&rt_itoa_body()));
    for b in user_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in helper_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in spec_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in render_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    code_items.extend_from_slice(&encode_body(run_body));
    let code_sec = section(10, &wasm_vec(n_funcs, &code_items));

    // ── Assemble the core module. ──
    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&glob_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);

    // ── Component: fixed HEAD + embedded core module + fixed TAIL. ──
    let mut out = Vec::new();
    out.extend_from_slice(RT_HEAD);
    out.push(1); // core-module section id
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(&core);
    out.extend_from_slice(RT_TAIL);
    out
}

/// Build a `compile : list<u8> -> list<u8>` component from a `(def (compile b) …)` entry. Same
/// value-heap runtime import + core-module machinery as `runtime_compound_component`, but instead of
/// a nullary `run` + renderer, the entry is a `compile: (i32 ptr, i32 len) -> i32 retptr` wrapper that
/// marshals the canonical list ABI: read the incoming bytes into a runtime `Bytes` handle, call the
/// user `compile`, then copy the result handle's bytes into linear memory and return a `(ptr,len)`
/// retptr. Wrapped by the generated COMPILE_HEAD/COMPILE_TAIL (which lift it as
/// `cadenza:compiler/compile`). `user_bodies[0]` is the user `compile` (func 0).
/// Which `compile`-entry ABI to emit, chosen from the user body's static return shape.
#[derive(Clone, Copy)]
enum CompileAbi {
    /// `list<u8> -> list<u8>` — a body returning bare `Bytes` (the original seam).
    Bytes,
    /// `list<u8> -> result<list<u8>, list<diagnostic>>` — a body returning `Result` (carries the `Ok`
    /// discriminant). ask-40.
    Result(u32),
    /// `list<artifact> -> compile-output` — a body returning a `(record (artifacts…) (diagnostics…))`.
    /// ask-41 / Amendment 0.8.0.
    Artifacts,
}

fn compile_component(
    funcs: &[Func],
    user_bodies: &[Body],
    helper_bodies: &[Body],
    spec_type_items: &[u8],
    n_specs: usize,
    spec_bodies: &[Body],
    abi: CompileAbi,
) -> Vec<u8> {
    let n_user = user_bodies.len();
    let n_helpers = helper_bodies.len();

    // ── Types: heap imports, then realloc/putu/itoa (fixed helper trio), then one per user func,
    //    one per overflow helper, one per effect-context spec, then the wrapper `compile: (i32,i32)->i32`. ──
    let mut type_items = Vec::new();
    let mut n_types = 0usize;
    for t in rt_import_types() {
        type_items.extend_from_slice(&t);
        n_types += 1;
    }
    let ty_realloc = n_types as u32;
    type_items.extend_from_slice(&functype(&[Kind::Bool, Kind::Bool, Kind::Bool, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_putu = n_types as u32; // (i64,i32)->i32 ; itoa shares it
    type_items.extend_from_slice(&functype(&[Kind::Int64, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_user_base = n_types as u32;
    for f in funcs {
        type_items.extend_from_slice(&functype(&f.param_kinds, f.ret_kind.externalized()));
        n_types += 1;
    }
    let ty_helper_base = n_types as u32;
    let helper_ty = functype(&[Kind::Int64, Kind::Int64], Kind::Int64);
    for _ in 0..n_helpers {
        type_items.extend_from_slice(&helper_ty);
        n_types += 1;
    }
    // Effect-context specialization types (ask-46): after helpers, before the wrapper — matching the
    // func layout `spec_wasm_index` assumes (`[fixed][user][helpers][specs][wrapper]`).
    let ty_spec_base = n_types as u32;
    type_items.extend_from_slice(spec_type_items);
    n_types += n_specs;
    let ty_compile = n_types as u32; // (i32,i32)->i32
    type_items.extend_from_slice(&functype(&[Kind::Bool, Kind::Bool], Kind::Bool));
    n_types += 1;
    let type_sec = section(1, &wasm_vec(n_types, &type_items));

    // ── Import section (fixed heap imports). ──
    let import_sec = section(2, RT_IMPORT_CONTENT);

    // ── Function section: realloc, putu, itoa, user types, helper types, spec types, compile-wrapper type. ──
    let mut func_items = Vec::new();
    uleb128(ty_realloc as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    for u in 0..n_user {
        uleb128((ty_user_base + u as u32) as u64, &mut func_items);
    }
    for h in 0..n_helpers {
        uleb128((ty_helper_base + h as u32) as u64, &mut func_items);
    }
    for s in 0..n_specs {
        uleb128((ty_spec_base + s as u32) as u64, &mut func_items);
    }
    uleb128(ty_compile as u64, &mut func_items);
    let n_funcs = RT_FIXED_FUNCS + n_user + n_helpers + n_specs + 1; // +1 wrapper
    let func_sec = section(3, &wasm_vec(n_funcs, &func_items));

    // ── Memory + global (fixed). ──
    let mem_sec = section(5, RT_MEM);
    let glob_sec = section(6, RT_GLOBAL);

    // ── Export: memory, cabi_realloc, compile (the wrapper — last defined func, AFTER the specs). ──
    let realloc_idx = RT_REALLOC;
    let wrapper_idx = RT_FUNC_BASE + (n_user + n_helpers + n_specs) as u32;
    let mut exports = Vec::new();
    exports.extend_from_slice(&export_entry("memory", 0x02, 0));
    exports.extend_from_slice(&export_entry("cabi_realloc", 0x00, realloc_idx));
    exports.extend_from_slice(&export_entry("compile", 0x00, wrapper_idx));
    let export_sec = section(7, &wasm_vec(3, &exports));

    // ── Code: realloc/putu/itoa, user bodies, helper bodies, spec bodies, compile wrapper. ──
    let mut code_items = Vec::new();
    code_items.extend_from_slice(&encode_body(&rt_realloc_body()));
    code_items.extend_from_slice(&encode_body(&rt_putu_body()));
    code_items.extend_from_slice(&encode_body(&rt_itoa_body()));
    for b in user_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in helper_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in spec_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    // The wrapper marshals the chosen ABI: plain `list<u8> → list<u8>`, the diagnostics
    // `result<list<u8>, list<diagnostic>>`, or the kinded-artifact `list<artifact> → compile-output`.
    let wrapper = match abi {
        CompileAbi::Bytes => compile_wrapper_body(RT_FUNC_BASE),
        CompileAbi::Result(ok_disc) => compile_result_wrapper_body(RT_FUNC_BASE, ok_disc),
        CompileAbi::Artifacts => compile_artifacts_wrapper_body(RT_FUNC_BASE),
    };
    code_items.extend_from_slice(&encode_body(&wrapper));
    let code_sec = section(10, &wasm_vec(n_funcs, &code_items));

    // ── Assemble the core module. ──
    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&glob_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);

    // ── Component: the HEAD/TAIL surround for the chosen ABI + embedded core module. The core module
    //    is IDENTICAL across ABIs (same `compile: (i32,i32)->i32` retptr export); only the
    //    component-level lift differs (`list<u8>` vs `result<…>` vs `list<artifact>→compile-output`),
    //    which is what the HEAD/TAIL surround encodes. ──
    let (head, tail): (&[u8], &[u8]) = match abi {
        CompileAbi::Bytes => (COMPILE_HEAD, COMPILE_TAIL),
        CompileAbi::Result(_) => (COMPILE_RESULT_HEAD, COMPILE_RESULT_TAIL),
        CompileAbi::Artifacts => (COMPILE_ARTIFACTS_HEAD, COMPILE_ARTIFACTS_TAIL),
    };
    let mut out = Vec::new();
    out.extend_from_slice(head);
    out.push(1); // core-module section id
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(&core);
    out.extend_from_slice(tail);
    out
}

/// The `compile: (i32 ptr, i32 len) -> i32 retptr` wrapper body: marshals the canonical list ABI
/// around the user `compile` (core func `user_compile_idx`). Reads the input `(ptr,len)` bytes into a
/// runtime `Bytes` handle (`bytes-alloc` + a `bytes-set` per byte read via `i32.load8_u`), calls the
/// user body, then copies the result handle's bytes (`bytes-len`/`bytes-get`) into a `cabi_realloc`'d
/// output buffer, writes the `(out-ptr, out-len)` pair into the 8-byte retarea, and returns the
/// retarea pointer. Params: local0=ptr, local1=len. Extra locals (all i32): 2=in-buf handle,
/// 3=loop i, 4=result handle, 5=result len, 6=out-ptr, 7=retarea.
fn compile_wrapper_body(user_compile_idx: u32) -> Body {
    let (ptr, len) = (0u32, 1u32);
    let (inbuf, i, res, rlen, out, ret) = (2u32, 3u32, 4u32, 5u32, 6u32, 7u32);
    let extra_locals = vec![Kind::Bool; 6]; // locals 2..=7, all i32
    let mut c = Vec::new();

    // inbuf = bytes-alloc(len)
    c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::CALL]);
    uleb128(himport::BYTES_ALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, inbuf as u8]);
    // i = 0
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, i as u8]);
    // block { loop { if i >= len break; bytes-set(inbuf, i, load8(ptr+i)) drop ; i+=1 } }
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::LOCAL_GET, len as u8, 0x4E /*i32.ge_s*/, op::BR_IF, 1]);
    // bytes-set(inbuf, i, load8_u(ptr + i))
    c.extend_from_slice(&[op::LOCAL_GET, inbuf as u8, op::LOCAL_GET, i as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, ptr as u8, op::LOCAL_GET, i as u8, op::I32_ADD, op::I32_LOAD8_U, 0x00, 0x00]);
    c.push(op::CALL);
    uleb128(himport::BYTES_SET as u64, &mut c);
    c.push(op::DROP);
    // i += 1 ; continue
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, i as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block

    // res = user_compile(inbuf)
    c.extend_from_slice(&[op::LOCAL_GET, inbuf as u8, op::CALL]);
    uleb128(user_compile_idx as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, res as u8]);
    // rlen = bytes-len(res)
    c.extend_from_slice(&[op::LOCAL_GET, res as u8, op::CALL]);
    uleb128(himport::BYTES_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, rlen as u8]);
    // ret = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=8) — the 8-byte, 4-ALIGNED retarea for
    // the (ptr,len) pair the canonical `list<u8>` return requires. `cabi_realloc` HONORS the align
    // argument (rounds the bump pointer up to it), so this lands 4-aligned regardless of how many
    // input bytes the lift already consumed. ⚠ CANONICAL arg order: align is the 3RD arg (index 2),
    // matching wasmtime's own calls; a power of two there (the `& -align` mask needs it non-zero).
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4, op::I32_CONST, 8, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, ret as u8]);
    // out = cabi_realloc(orig=0, old_size=0, ALIGN=1, new_size=rlen) — a fresh list<u8> buffer, byte
    // alignment (1), sized rlen. Allocated AFTER the retarea, so `out = ret + 8` (they never overlap).
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 1, op::LOCAL_GET, rlen as u8, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, out as u8]);
    // i = 0 ; loop { if i >= rlen break; store8(out+i, bytes-get(res,i)) ; i+=1 }
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, i as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::LOCAL_GET, rlen as u8, 0x4E /*i32.ge_s*/, op::BR_IF, 1]);
    // store8(out + i, bytes-get(res, i))
    c.extend_from_slice(&[op::LOCAL_GET, out as u8, op::LOCAL_GET, i as u8, op::I32_ADD]);
    c.extend_from_slice(&[op::LOCAL_GET, res as u8, op::LOCAL_GET, i as u8, op::CALL]);
    uleb128(himport::BYTES_GET as u64, &mut c);
    c.extend_from_slice(&[op::I32_STORE8, 0x00, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, i as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]);

    // store(ret+0, out) ; store(ret+4, rlen)
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, out as u8, op::I32_STORE, 0x02, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, rlen as u8, op::I32_STORE, 0x02, 0x04]);
    // return ret
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8]);

    Body { extra_locals, code: c }
}

/// Emit `dst_handle = a runtime Bytes handle holding the `slen` bytes at linear-memory address
/// `sptr``: `bytes-alloc(slen)`, then per byte `bytes-set(buf, j, load8_u(sptr + j))`. The INPUT dual
/// of `emit_bytes_copy_loop` (which copies a runtime Bytes OUT to linear memory). `sptr`/`slen`/`j`
/// are i32 local indices holding the source (ptr,len); the fresh handle is left in local `dst_handle`.
/// Single-level `block`/`loop`. Appends to `c`.
fn emit_read_bytes_from_mem(dst_handle: u32, sptr: u32, slen: u32, j: u32, c: &mut Vec<u8>) {
    // dst = bytes-alloc(slen)
    c.extend_from_slice(&[op::LOCAL_GET, slen as u8, op::CALL]);
    uleb128(himport::BYTES_ALLOC as u64, c);
    c.extend_from_slice(&[op::LOCAL_SET, dst_handle as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, j as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, j as u8, op::LOCAL_GET, slen as u8, 0x4E /*i32.ge_s*/, op::BR_IF, 1]);
    // bytes-set(dst, j, load8_u(sptr + j)) ; drop
    c.extend_from_slice(&[op::LOCAL_GET, dst_handle as u8, op::LOCAL_GET, j as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, sptr as u8, op::LOCAL_GET, j as u8, op::I32_ADD, op::I32_LOAD8_U, 0x00, 0x00]);
    c.push(op::CALL);
    uleb128(himport::BYTES_SET as u64, c);
    c.push(op::DROP);
    c.extend_from_slice(&[op::LOCAL_GET, j as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, j as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block
}

/// Emit a self-contained byte-copy loop: `for j in 0..len { store8(dst + j, bytes-get(src, j)) }`,
/// where `dst`/`src`/`len`/`j` are i32 local indices (`src` a runtime Bytes handle, `dst` a linear-
/// memory pointer). Single-level `block`/`loop` with `br 1`/`br 0` to ITS OWN frames only — never a
/// multi-level branch (the hand-emitted-wasm rule). Appends to `c`.
fn emit_bytes_copy_loop(dst: u32, src: u32, len: u32, j: u32, c: &mut Vec<u8>) {
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, j as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, j as u8, op::LOCAL_GET, len as u8, 0x4E /*i32.ge_s*/, op::BR_IF, 1]);
    c.extend_from_slice(&[op::LOCAL_GET, dst as u8, op::LOCAL_GET, j as u8, op::I32_ADD]);
    c.extend_from_slice(&[op::LOCAL_GET, src as u8, op::LOCAL_GET, j as u8, op::CALL]);
    uleb128(himport::BYTES_GET as u64, c);
    c.extend_from_slice(&[op::I32_STORE8, 0x00, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, j as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, j as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block
}

/// Emit `(ptr,len) = marshal a runtime Bytes/String `handle` into a fresh cabi_realloc'd linear-memory
/// buffer`: `len = bytes-len(handle); buf = cabi_realloc(0,1,0,len); copy; store(base+off, buf);
/// store(base+off+4, len)` — the canonical-ABI `string`/`list<u8>` `(ptr,len)` pair written into the
/// record slot at `base+off`. `len_l`/`buf_l`/`j_l` are scratch i32 locals. Appends to `c`.
fn emit_marshal_string_into(
    base: u32,
    off: u32,
    handle: u32,
    len_l: u32,
    buf_l: u32,
    j_l: u32,
    c: &mut Vec<u8>,
) {
    // len = bytes-len(handle)
    c.extend_from_slice(&[op::LOCAL_GET, handle as u8, op::CALL]);
    uleb128(himport::BYTES_LEN as u64, c);
    c.extend_from_slice(&[op::LOCAL_SET, len_l as u8]);
    // buf = cabi_realloc(orig=0, old_size=0, ALIGN=1, new_size=len)  — byte-aligned fresh buffer
    // (canonical arg order: align is index 2).
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 1, op::LOCAL_GET, len_l as u8, op::CALL]);
    uleb128(RT_REALLOC as u64, c);
    c.extend_from_slice(&[op::LOCAL_SET, buf_l as u8]);
    emit_bytes_copy_loop(buf_l, handle, len_l, j_l, c);
    // store(base + off, buf) ; store(base + off + 4, len)
    c.extend_from_slice(&[op::LOCAL_GET, base as u8, op::I32_CONST]);
    sleb128(off as i64, c);
    c.extend_from_slice(&[op::I32_ADD, op::LOCAL_GET, buf_l as u8, op::I32_STORE, 0x02, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, base as u8, op::I32_CONST]);
    sleb128(off as i64 + 4, c);
    c.extend_from_slice(&[op::I32_ADD, op::LOCAL_GET, len_l as u8, op::I32_STORE, 0x02, 0x00]);
}

/// The core `compile` wrapper for the DIAGNOSTICS-ABI path: `compile: (i32 ptr, i32 len) -> i32 retptr`
/// lifting to `func(list<u8>) -> result<list<u8>, list<diagnostic>>`. Reads the input bytes into a
/// runtime `Bytes` handle, calls the user body (which returns a runtime `Result<Bytes,
/// list<diagnostic>>`), then marshals that Result into the canonical retptr layout
/// `[disc:i32 @0][ptr:i32 @4][len:i32 @8]`:
///   - disc 0 (Ok): payload is a Bytes handle → write it as `list<u8>` (out ptr, byte length);
///   - disc 1 (Err): payload is a `list<diagnostic>` (a runtime vec of records, each 2 String fields
///     sorted `code` < `message`) → allocate `n*16` element bytes and, per diagnostic, marshal both
///     strings into `[code(ptr,len) @0][message(ptr,len) @8]`, then write (elems ptr, n) as the list.
/// `some_disc`/`ok_disc` are the runtime discriminants of `Ok`/`Err` (from `Result`'s declared order).
fn compile_result_wrapper_body(user_compile_idx: u32, ok_disc: u32) -> Body {
    let (ptr, len) = (0u32, 1u32);
    // locals 2..=19 (all i32): input+result+retptr scratch, then Ok-arm and Err-arm scratch.
    let (inbuf, i, res, disc, payload, ret) = (2u32, 3u32, 4u32, 5u32, 6u32, 7u32);
    let (rlen, out) = (8u32, 9u32);
    let (ndiag, elems, d, diag, slen, sbuf, sj) = (10u32, 11u32, 12u32, 13u32, 14u32, 15u32, 16u32);
    let extra_locals = vec![Kind::Bool; 15]; // locals 2..=16, all i32
    let mut c = Vec::new();

    // inbuf = bytes-alloc(len) ; copy input[0..len] into it.
    c.extend_from_slice(&[op::LOCAL_GET, len as u8, op::CALL]);
    uleb128(himport::BYTES_ALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, inbuf as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, i as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::LOCAL_GET, len as u8, 0x4E, op::BR_IF, 1]);
    c.extend_from_slice(&[op::LOCAL_GET, inbuf as u8, op::LOCAL_GET, i as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, ptr as u8, op::LOCAL_GET, i as u8, op::I32_ADD, op::I32_LOAD8_U, 0x00, 0x00]);
    c.push(op::CALL);
    uleb128(himport::BYTES_SET as u64, &mut c);
    c.push(op::DROP);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, i as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]);

    // res = user_compile(inbuf)
    c.extend_from_slice(&[op::LOCAL_GET, inbuf as u8, op::CALL]);
    uleb128(user_compile_idx as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, res as u8]);
    // ret = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=12) — the 12-byte, 4-aligned retarea
    // [disc, ptr, len] (canonical arg order: align is index 2).
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4, op::I32_CONST, 12, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, ret as u8]);
    // disc = sum-disc(res) ; payload = sum-payload(res)
    c.extend_from_slice(&[op::LOCAL_GET, res as u8, op::CALL]);
    uleb128(himport::SUM_DISC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, disc as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, res as u8, op::CALL]);
    uleb128(himport::SUM_PAYLOAD as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, payload as u8]);

    // if disc == ok_disc { Ok arm } else { Err arm }
    c.extend_from_slice(&[op::LOCAL_GET, disc as u8, op::I32_CONST]);
    sleb128(ok_disc as i64, &mut c);
    c.extend_from_slice(&[0x46 /*i32.eq*/, op::IF, 0x40]);

    // ── Ok arm: payload is a Bytes handle → list<u8>. ──
    // rlen = bytes-len(payload) ; out = cabi_realloc(orig=0,old_size=0,ALIGN=1,new_size=rlen) ; copy ;
    // store [0,out,rlen] (canonical arg order: align is index 2).
    c.extend_from_slice(&[op::LOCAL_GET, payload as u8, op::CALL]);
    uleb128(himport::BYTES_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, rlen as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 1, op::LOCAL_GET, rlen as u8, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, out as u8]);
    emit_bytes_copy_loop(out, payload, rlen, i, &mut c);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::I32_CONST, 0, op::I32_STORE, 0x02, 0x00]); // disc=ok(0)
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, out as u8, op::I32_STORE, 0x02, 0x04]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, rlen as u8, op::I32_STORE, 0x02, 0x08]);

    c.push(op::ELSE);

    // ── Err arm: payload is a list<diagnostic> (runtime vec of record{code,message}). ──
    // ndiag = vec-len(payload) ; elems = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=ndiag*16)
    // (canonical arg order: align is index 2).
    c.extend_from_slice(&[op::LOCAL_GET, payload as u8, op::CALL]);
    uleb128(himport::VEC_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, ndiag as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4]);
    c.extend_from_slice(&[op::LOCAL_GET, ndiag as u8, op::I32_CONST, 16, op::I32_MUL, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, elems as u8]);
    // d = 0 ; loop over diagnostics
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, d as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::LOCAL_GET, ndiag as u8, 0x4E, op::BR_IF, 1]);
    // diag = vec-get(payload, d)  — a runtime record (arr of 2 slots).
    c.extend_from_slice(&[op::LOCAL_GET, payload as u8, op::LOCAL_GET, d as u8, op::CALL]);
    uleb128(himport::VEC_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, diag as u8]);
    // slot base for this element: sbuf reused = elems + d*16.
    c.extend_from_slice(&[op::LOCAL_GET, elems as u8, op::LOCAL_GET, d as u8, op::I32_CONST, 16, op::I32_MUL, op::I32_ADD, op::LOCAL_SET, sbuf as u8]);
    // code = arr-get(diag, 0) → marshal into [sbuf + 0].
    c.extend_from_slice(&[op::LOCAL_GET, diag as u8, op::I32_CONST, 0, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, diag as u8]); // temporarily hold the code handle in `diag`
    // (use a fresh scratch: reuse `res` is unsafe; use `out` which is Ok-arm-only.)
    emit_marshal_string_into(sbuf, 0, diag, slen, out, sj, &mut c);
    // message = arr-get(vec-get(payload,d), 1) → re-fetch the record (diag was overwritten).
    c.extend_from_slice(&[op::LOCAL_GET, payload as u8, op::LOCAL_GET, d as u8, op::CALL]);
    uleb128(himport::VEC_GET as u64, &mut c);
    c.extend_from_slice(&[op::I32_CONST, 1, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, diag as u8]);
    emit_marshal_string_into(sbuf, 8, diag, slen, out, sj, &mut c);
    // d += 1 ; continue
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, d as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block
    // store [1(err), elems, ndiag].
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::I32_CONST, 1, op::I32_STORE, 0x02, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, elems as u8, op::I32_STORE, 0x02, 0x04]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, ndiag as u8, op::I32_STORE, 0x02, 0x08]);

    c.push(op::END); // if

    // return ret
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8]);
    Body { extra_locals, code: c }
}

/// The core `compile` wrapper for the KINDED-ARTIFACT ABI (ask-41 / Amendment 0.8.0):
/// `compile: (i32 ptr, i32 len) -> i32 retptr` lifting to `func(list<artifact>) -> compile-output`.
///
/// INPUT — `(ptr, len)` is the canonical `list<artifact>`: `len` elements of 16 bytes each, laid out
/// `[bytes(mem_ptr,mem_len) @0][kind(str_ptr,str_len) @8]` (record fields SORTED by key: bytes < kind).
/// Build a runtime vec of runtime records `(record (bytes <Bytes>) (kind <String>))` — `arr-alloc(2)`,
/// slot0 = the bytes handle, slot1 = the kind handle (both read from linear memory), then `vec-push`.
///
/// OUTPUT — the user body returns a runtime `compile-output` record: a heap `arr` whose slots are the
/// fields sorted by key — slot0 `artifacts` (a runtime vec of `artifact` records), slot1 `diagnostics`
/// (a runtime vec of `diagnostic` records). Marshal to the canonical 16-byte retarea
/// `[artifacts(ptr,len) @0][diagnostics(ptr,len) @8]`, each list to a fresh `cabi_realloc`'d element
/// array: an artifact element is 16 bytes `[bytes(ptr,len)@0][kind(ptr,len)@8]`; a diagnostic element
/// is 20 bytes `[code(ptr,len)@0][message(ptr,len)@8][severity:i32 @16]` (severity read from the
/// record's slot2 as a boxed Int64: 0=error, 1=warning). Field order in every record matches the
/// SORTED WIT declaration, so a runtime slot i maps to canonical offset with no permutation.
fn compile_artifacts_wrapper_body(user_compile_idx: u32) -> Body {
    let (ptr, len) = (0u32, 1u32);
    // locals 2..=21 (all i32) — scratch for input build + output marshal.
    let inputs = 2u32; // runtime list<artifact> handle (built from the input)
    let i = 3u32; // outer loop counter
    let rec = 4u32; // a runtime artifact record handle
    let sp = 5u32; // element base ptr in linear memory
    let s0 = 6u32; // scratch ptr/len a
    let s1 = 7u32; // scratch ptr/len b
    let hbytes = 8u32; // read bytes handle
    let hkind = 9u32; // read kind handle
    let jj = 10u32; // inner copy counter
    let outrec = 11u32; // the compile-output record handle
    let arts = 12u32; // artifacts vec handle
    let diags = 13u32; // diagnostics vec handle
    let ret = 14u32; // 16-byte retarea
    let n = 15u32; // per-list count
    let elems = 16u32; // per-list element array ptr
    let d = 17u32; // per-list loop counter
    let item = 18u32; // a runtime record handle (artifact / diagnostic)
    let slen = 19u32; // string len scratch (for emit_marshal_string_into)
    let sbuf = 20u32; // string buf scratch
    let base = 21u32; // element slot base
    let extra_locals = vec![Kind::Bool; 20]; // locals 2..=21, all i32
    let mut c = Vec::new();

    // ── INPUT: build `inputs` = a runtime vec of artifact records from the canonical list<artifact>. ──
    // inputs = vec-empty()
    c.push(op::CALL);
    uleb128(himport::VEC_EMPTY as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, inputs as u8]);
    // i = 0 ; loop over the `len` input artifacts
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, i as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::LOCAL_GET, len as u8, 0x4E /*i32.ge_s*/, op::BR_IF, 1]);
    // sp = ptr + i*16
    c.extend_from_slice(&[op::LOCAL_GET, ptr as u8, op::LOCAL_GET, i as u8, op::I32_CONST, 16, op::I32_MUL, op::I32_ADD, op::LOCAL_SET, sp as u8]);
    // bytes field @ sp+0: s0=load(sp+0) mem-ptr, s1=load(sp+4) mem-len → hbytes.
    c.extend_from_slice(&[op::LOCAL_GET, sp as u8, op::I32_LOAD, 0x02, 0x00, op::LOCAL_SET, s0 as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, sp as u8, op::I32_LOAD, 0x02, 0x04, op::LOCAL_SET, s1 as u8]);
    emit_read_bytes_from_mem(hbytes, s0, s1, jj, &mut c);
    // kind field @ sp+8: s0=load(sp+8) str-ptr, s1=load(sp+12) str-len → hkind.
    c.extend_from_slice(&[op::LOCAL_GET, sp as u8, op::I32_LOAD, 0x02, 0x08, op::LOCAL_SET, s0 as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, sp as u8, op::I32_LOAD, 0x02, 0x0C, op::LOCAL_SET, s1 as u8]);
    emit_read_bytes_from_mem(hkind, s0, s1, jj, &mut c);
    // rec = arr-alloc(2) ; arr-set(rec,0,hbytes) ; arr-set(rec,1,hkind)
    c.extend_from_slice(&[op::I32_CONST, 2, op::CALL]);
    uleb128(himport::ARR_ALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, rec as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, rec as u8, op::I32_CONST, 0, op::LOCAL_GET, hbytes as u8, op::CALL]);
    uleb128(himport::ARR_SET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, rec as u8]); // arr-set returns the (possibly-moved) handle
    c.extend_from_slice(&[op::LOCAL_GET, rec as u8, op::I32_CONST, 1, op::LOCAL_GET, hkind as u8, op::CALL]);
    uleb128(himport::ARR_SET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, rec as u8]);
    // inputs = vec-push(inputs, rec)
    c.extend_from_slice(&[op::LOCAL_GET, inputs as u8, op::LOCAL_GET, rec as u8, op::CALL]);
    uleb128(himport::VEC_PUSH as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, inputs as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, i as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, i as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block

    // ── CALL the user body: outrec = compile(inputs). ──
    c.extend_from_slice(&[op::LOCAL_GET, inputs as u8, op::CALL]);
    uleb128(user_compile_idx as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, outrec as u8]);
    // arts = arr-get(outrec, 0) ; diags = arr-get(outrec, 1)
    c.extend_from_slice(&[op::LOCAL_GET, outrec as u8, op::I32_CONST, 0, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, arts as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, outrec as u8, op::I32_CONST, 1, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, diags as u8]);
    // ret = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=16) — the compile-output retarea
    // [artifacts(ptr,len), diags(ptr,len)] (canonical arg order: align is index 2).
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4, op::I32_CONST, 16, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, ret as u8]);

    // ── OUTPUT artifacts list → retarea offset 0. 16 bytes/element [bytes(ptr,len)@0][kind(ptr,len)@8]. ──
    // elems = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=n*16) (canonical arg order).
    c.extend_from_slice(&[op::LOCAL_GET, arts as u8, op::CALL]);
    uleb128(himport::VEC_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, n as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4]);
    c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 16, op::I32_MUL, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, elems as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, d as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::LOCAL_GET, n as u8, 0x4E, op::BR_IF, 1]);
    // base = elems + d*16 ; item = vec-get(arts, d)
    c.extend_from_slice(&[op::LOCAL_GET, elems as u8, op::LOCAL_GET, d as u8, op::I32_CONST, 16, op::I32_MUL, op::I32_ADD, op::LOCAL_SET, base as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, arts as u8, op::LOCAL_GET, d as u8, op::CALL]);
    uleb128(himport::VEC_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, item as u8]);
    // bytes = arr-get(item,0) → marshal into base+0 ; kind = arr-get(item,1) → base+8
    c.extend_from_slice(&[op::LOCAL_GET, item as u8, op::I32_CONST, 0, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, sbuf as u8]); // hold handle in sbuf temporarily
    emit_marshal_string_into(base, 0, sbuf, slen, hbytes, jj, &mut c);
    c.extend_from_slice(&[op::LOCAL_GET, item as u8, op::I32_CONST, 1, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, sbuf as u8]);
    emit_marshal_string_into(base, 8, sbuf, slen, hbytes, jj, &mut c);
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, d as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block
    // store (elems, n) into retarea @0/@4
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, elems as u8, op::I32_STORE, 0x02, 0x00]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, n as u8, op::I32_STORE, 0x02, 0x04]);

    // ── OUTPUT diagnostics list → retarea offset 8. 20 bytes/element
    //    [code(ptr,len)@0][message(ptr,len)@8][severity:i32 @16]. ──
    // elems = cabi_realloc(orig=0, old_size=0, ALIGN=4, new_size=n*20) (canonical arg order).
    c.extend_from_slice(&[op::LOCAL_GET, diags as u8, op::CALL]);
    uleb128(himport::VEC_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, n as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::I32_CONST, 0, op::I32_CONST, 4]);
    c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 20, op::I32_MUL, op::CALL]);
    uleb128(RT_REALLOC as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, elems as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, d as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::LOCAL_GET, n as u8, 0x4E, op::BR_IF, 1]);
    c.extend_from_slice(&[op::LOCAL_GET, elems as u8, op::LOCAL_GET, d as u8, op::I32_CONST, 20, op::I32_MUL, op::I32_ADD, op::LOCAL_SET, base as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, diags as u8, op::LOCAL_GET, d as u8, op::CALL]);
    uleb128(himport::VEC_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, item as u8]);
    // code = arr-get(item,0) → base+0 ; message = arr-get(item,1) → base+8
    c.extend_from_slice(&[op::LOCAL_GET, item as u8, op::I32_CONST, 0, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, sbuf as u8]);
    emit_marshal_string_into(base, 0, sbuf, slen, hbytes, jj, &mut c);
    c.extend_from_slice(&[op::LOCAL_GET, item as u8, op::I32_CONST, 1, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, sbuf as u8]);
    emit_marshal_string_into(base, 8, sbuf, slen, hbytes, jj, &mut c);
    // severity = get-int(arr-get(item,2)) → store i32 at base+16
    c.extend_from_slice(&[op::LOCAL_GET, base as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, item as u8, op::I32_CONST, 2, op::CALL]);
    uleb128(himport::ARR_GET as u64, &mut c);
    c.push(op::CALL);
    uleb128(himport::GET_INT as u64, &mut c); // i64
    c.push(0xA7); // i32.wrap_i64
    c.extend_from_slice(&[op::I32_STORE, 0x02, 0x10]); // @ base+16
    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, d as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // loop, block
    // store (elems, n) into retarea @8/@12
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, elems as u8, op::I32_STORE, 0x02, 0x08]);
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8, op::LOCAL_GET, n as u8, op::I32_STORE, 0x02, 0x0C]);

    // return ret
    c.extend_from_slice(&[op::LOCAL_GET, ret as u8]);
    Body { extra_locals, code: c }
}

// `RT_TAIL_PREFIX_LEN` (generated in `heap_envelope.rs`) splits RT_TAIL into a program-independent
// PREFIX — the heap core-instance instantiation, the memory/cabi_realloc component aliases,
// everything up to but not including the `run` export's component type — and a SUFFIX that
// types+lifts+exports `run`. The compound path's suffix types `run` as `() -> string` with a
// memory/realloc canon lift; the SCALAR path (below) needs `() -> <scalar>` with a zero-option lift,
// so it takes `RT_TAIL[..RT_TAIL_PREFIX_LEN]` and appends its own suffix. The generator derives the
// prefix length by locating the `7 5 1 0x40 0 0` run component-type section in the tail.

/// Build a runtime-scalar component: `main` returns a SCALAR (Int64/Bool) but the program computes
/// over runtime value-heap values (recursive `len`/`sum` over a linked list). Same value-heap
/// runtime import + core-module machinery as `runtime_compound_component`, but there is no renderer
/// — `run` calls `main` and returns its scalar directly, and the component's `run` export is
/// `() -> <scalar>` (a zero-option canon lift, no string marshaling). Reuses the RT_TAIL prefix
/// (heap wiring) and appends a scalar run-type/lift/export suffix.
fn runtime_scalar_component(
    funcs: &[Func],
    user_bodies: &[Body],
    helper_bodies: &[Body],
    main_ret: Kind,
    spec_type_items: &[u8],
    n_specs: usize,
    spec_bodies: &[Body],
) -> Result<Vec<u8>, Decline> {
    let n_user = user_bodies.len();
    let n_helpers = helper_bodies.len();

    // Types: 24 import types, realloc, putu, itoa (kept for a stable index layout though the
    // scalar path never renders), one per user func, one per helper, then run `() -> <scalar>`.
    let mut type_items = Vec::new();
    let mut n_types = 0usize;
    for t in rt_import_types() {
        type_items.extend_from_slice(&t);
        n_types += 1;
    }
    let ty_realloc = n_types as u32;
    type_items.extend_from_slice(&functype(&[Kind::Bool, Kind::Bool, Kind::Bool, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_putu = n_types as u32;
    type_items.extend_from_slice(&functype(&[Kind::Int64, Kind::Bool], Kind::Bool));
    n_types += 1;
    let ty_user_base = n_types as u32;
    for f in funcs {
        type_items.extend_from_slice(&functype(&f.param_kinds, f.ret_kind.externalized()));
        n_types += 1;
    }
    let ty_helper_base = n_types as u32;
    let helper_ty = functype(&[Kind::Int64, Kind::Int64], Kind::Int64);
    for _ in 0..n_helpers {
        type_items.extend_from_slice(&helper_ty);
        n_types += 1;
    }
    // Effect-context specialization types (ask-44): one per spec, threading handler state as
    // trailing params/returns. They sit AFTER helpers and BEFORE `run`, matching the func layout
    // `spec_wasm_index` assumes (`[fixed][user][helpers][specs][run]`).
    let ty_spec_base = n_types as u32;
    type_items.extend_from_slice(spec_type_items);
    n_types += n_specs;
    let ty_run = n_types as u32; // () -> <scalar>
    type_items.extend_from_slice(&functype(&[], main_ret));
    n_types += 1;
    let type_sec = section(1, &wasm_vec(n_types, &type_items));

    let import_sec = section(2, RT_IMPORT_CONTENT);

    // Function section: realloc, putu, itoa, user types, helper types, spec types, run.
    let mut func_items = Vec::new();
    uleb128(ty_realloc as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    for u in 0..n_user {
        uleb128((ty_user_base + u as u32) as u64, &mut func_items);
    }
    for h in 0..n_helpers {
        uleb128((ty_helper_base + h as u32) as u64, &mut func_items);
    }
    for s in 0..n_specs {
        uleb128((ty_spec_base + s as u32) as u64, &mut func_items);
    }
    uleb128(ty_run as u64, &mut func_items);
    let n_funcs = RT_FIXED_FUNCS + n_user + n_helpers + n_specs + 1; // +1 for run
    let func_sec = section(3, &wasm_vec(n_funcs, &func_items));

    let mem_sec = section(5, RT_MEM);
    let glob_sec = section(6, RT_GLOBAL);

    // Exports: memory, cabi_realloc, run (last defined func, AFTER the spec funcs).
    let realloc_idx = RT_REALLOC;
    let run_idx = RT_FUNC_BASE + (n_user + n_helpers + n_specs) as u32;
    let mut exports = Vec::new();
    exports.extend_from_slice(&export_entry("memory", 0x02, 0));
    exports.extend_from_slice(&export_entry("cabi_realloc", 0x00, realloc_idx));
    exports.extend_from_slice(&export_entry("run", 0x00, run_idx));
    let export_sec = section(7, &wasm_vec(3, &exports));

    // `run`'s body: call `main` (user func 0 → wasm index RT_FUNC_BASE) and return its scalar.
    let mut run_code = vec![op::CALL];
    uleb128(RT_FUNC_BASE as u64, &mut run_code);
    let run_body = Body { extra_locals: Vec::new(), code: run_code };

    // Code section: realloc/putu/itoa, user bodies, helper bodies, spec bodies, run.
    let mut code_items = Vec::new();
    code_items.extend_from_slice(&encode_body(&rt_realloc_body()));
    code_items.extend_from_slice(&encode_body(&rt_putu_body()));
    code_items.extend_from_slice(&encode_body(&rt_itoa_body()));
    for b in user_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in helper_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    for b in spec_bodies {
        code_items.extend_from_slice(&encode_body(b));
    }
    code_items.extend_from_slice(&encode_body(&run_body));
    let code_sec = section(10, &wasm_vec(n_funcs, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&glob_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);

    // Component: HEAD + core module + (RT_TAIL prefix) + scalar run type/lift/export.
    // The scalar valtype byte for the run result (s64 / bool / f64).
    let vt = main_ret.comp_valtype();
    let mut suffix = Vec::new();
    // component type section: 1 functype `() -> <scalar>` (0x40, 0 params, 0x00 result, valtype).
    // This becomes component type index 1 (the heap import interface is component type 0), matching
    // where the compound tail declares its `run` type.
    suffix.extend_from_slice(&[7, 5, 1, 0x40, 0, 0, vt]);
    // canonical function section: lift core func 29 (the core `run`, first func after the 29
    // lowered heap imports in the component's core-func space) as component func, ZERO canon
    // options (a scalar needs no memory/realloc/utf8), component TYPE index 1.
    //   08(canon sec) 06(len) 01(count) 00(lift) 00(tag) 29(core-func) 00(0 opts) 01(type idx)
    suffix.extend_from_slice(&[8, 6, 1, 0, 0, RT_N_IMPORTS as u8, 0, 1]);
    // component export "run" → the lifted component func (index 29 in the component func space:
    // the 29 heap-interface funcs are component funcs 0..28, then the lifted run is 29).
    suffix.extend_from_slice(&[11, 9, 1, 0, 3, b'r', b'u', b'n', 1, RT_N_IMPORTS as u8, 0]);

    let mut out = Vec::new();
    out.extend_from_slice(RT_HEAD);
    out.push(1);
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(&core);
    out.extend_from_slice(&RT_TAIL[..RT_TAIL_PREFIX_LEN]);
    out.extend_from_slice(&suffix);
    Ok(out)
}

/// The absolute wasm index of a fixed helper in the runtime-compound module.
const RT_REALLOC: u32 = RT_N_IMPORTS; // 32
const RT_PUTU: u32 = RT_N_IMPORTS + 1; // 33
const RT_ITOA: u32 = RT_N_IMPORTS + 2; // 34

/// `cabi_realloc(orig, old_size, align, new_size) -> ptr`: a bump allocator over global 0 that
/// returns the old top (aligned) and advances by `new_size` (param local 3), using local 4 for the ret.
fn rt_realloc_body() -> Body {
    // CANONICAL component-ABI signature `cabi_realloc(orig=0, old_size=1, ALIGN=2, new_size=3) -> ptr`.
    // ⚠ The alignment argument is param index **2**, NOT index 1 — this is the order wasmtime's
    // canonical ABI uses when it LOWERS host values into guest memory (input `list<artifact>`, its
    // inner `list<u8>`, its `kind` strings) and when the guest RETURNS a list. Reading align from the
    // wrong slot masks the bump pointer with `& -old_size` = `& -0` = `& 0` = 0, so every allocation
    // collapses to address 0 and clobbers the previous one — invisible for a single input allocation
    // (the bytes ABI) but corrupting for the nested lowering the kinded-artifact ABI drives (ask-41).
    // A bump allocator over global 0 that HONORS alignment: rounds the bump pointer UP to `align` (a
    // power of two) before taking it, then advances by `new_size`. `aligned = (bump + align - 1) &
    // -align` (`-align == ~(align-1)` in two's complement masks off the low bits). local 4 = the ret.
    const I32_AND: u8 = 0x71;
    Body {
        extra_locals: vec![Kind::Bool], // local 4 (i32)
        code: vec![
            // ret(local4) = (global0 + align - 1) & -align
            op::GLOBAL_GET, 0, op::LOCAL_GET, 2, op::I32_ADD, op::I32_CONST, 1, op::I32_SUB, // bump+align-1
            op::I32_CONST, 0, op::LOCAL_GET, 2, op::I32_SUB, // -align
            I32_AND,
            op::LOCAL_SET, 4,
            // global0 = ret + new_size
            op::LOCAL_GET, 4, op::LOCAL_GET, 3, op::I32_ADD, op::GLOBAL_SET, 0,
            // return ret
            op::LOCAL_GET, 4,
        ],
    }
}

/// `putu(v: u64, cursor: i32) -> i32`: write `v` as unsigned decimal at `cursor`, return the
/// cursor past the last digit. Recursive: emits the high digits first, then the low digit.
fn rt_putu_body() -> Body {
    const I64_GT_U: u8 = 0x56;
    const I64_DIV_U: u8 = 0x80;
    const I64_REM_U: u8 = 0x82;
    const I32_WRAP_I64: u8 = 0xA7;
    let mut c = Vec::new();
    // if v > 9 { cursor = putu(v/10, cursor) }
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 9, I64_GT_U, op::IF, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 10, I64_DIV_U, op::LOCAL_GET, 1, op::CALL]);
    uleb128(RT_PUTU as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, 1, op::END]);
    // cursor[0] = '0' + (v % 10) ; return cursor + 1
    c.extend_from_slice(&[
        op::LOCAL_GET, 1, op::I32_CONST, 48, op::LOCAL_GET, 0, op::I64_CONST, 10, I64_REM_U,
        I32_WRAP_I64, op::I32_ADD, op::I32_STORE8, 0, 0,
    ]);
    c.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 1, op::I32_ADD]);
    Body { extra_locals: vec![], code: c }
}

/// `itoa(v: i64, cursor: i32) -> i32`: write `v` as signed decimal (a leading `-` for negatives,
/// then the magnitude via `putu`), return the cursor past the last digit. Uses one i64 local
/// (index 2) for the magnitude.
fn rt_itoa_body() -> Body {
    let mut c = Vec::new();
    // if v < 0 { cursor[0]='-'; cursor+=1; mag = 0 - v } else { mag = v }
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 0, op::I64_LT_S, op::IF, 0x40]);
    c.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 45, op::I32_STORE8, 0, 0]);
    c.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, 1]);
    c.extend_from_slice(&[op::I64_CONST, 0, op::LOCAL_GET, 0, op::I64_SUB, op::LOCAL_SET, 2]);
    c.extend_from_slice(&[op::ELSE, op::LOCAL_GET, 0, op::LOCAL_SET, 2, op::END]);
    // return putu(mag, cursor)
    c.extend_from_slice(&[op::LOCAL_GET, 2, op::LOCAL_GET, 1, op::CALL]);
    uleb128(RT_PUTU as u64, &mut c);
    Body { extra_locals: vec![Kind::Int64], code: c }
}

/// Trap (`unreachable`) if the i64 byte value in local `val` is outside 0..=255. A byte range is
/// bounded on BOTH sides (10-bytes.sexp #constructing … out of range / negative traps), and the
/// runtime's `bytes-set` truncates `value as u8` — so the range check MUST happen here on Cadenza's
/// side, before the wrap, or -1 would silently become 255. Emitted as `if (val < 0 | val > 255) {
/// unreachable }` (leaves the value stack unchanged).
fn emit_byte_range_guard(val: u32, c: &mut Vec<u8>) {
    // (val < 0)  — an i32 result (0|1)
    c.push(op::LOCAL_GET);
    uleb128(val as u64, c);
    c.push(op::I64_CONST);
    sleb128(0, c);
    c.push(op::I64_LT_S);
    // (val > 255) — an i32 result (0|1)
    c.push(op::LOCAL_GET);
    uleb128(val as u64, c);
    c.push(op::I64_CONST);
    sleb128(255, c);
    c.push(op::I64_GT_S);
    // (val<0) | (val>255): OR the two i32 comparison results — if it holds, trap
    c.push(0x72); // i32.or
    c.push(op::IF);
    c.push(0x40);
    c.push(op::UNREACHABLE);
    c.push(op::END);
}

/// Emit `cursor[i] = byte` for each byte of `s` starting at `cursor` (local `cur`), then
/// `cur += s.len()`. `cur` is the i32 local holding the write cursor.
fn emit_write_lit(s: &[u8], cur: u32, c: &mut Vec<u8>) {
    for (i, b) in s.iter().enumerate() {
        c.push(op::LOCAL_GET);
        uleb128(cur as u64, c);
        c.push(op::I32_CONST);
        sleb128(*b as i64, c);
        c.extend_from_slice(&[op::I32_STORE8, 0]); // align=0
        uleb128(i as u64, c); // offset = i
    }
    c.push(op::LOCAL_GET);
    uleb128(cur as u64, c);
    c.push(op::I32_CONST);
    sleb128(s.len() as i64, c);
    c.push(op::I32_ADD);
    c.push(op::LOCAL_SET);
    uleb128(cur as u64, c);
}

/// Emit `cur[0] = <byte on stack>; cur += 1` — store the byte value currently on the wasm stack at
/// the write cursor and advance it by one. The caller pushes the byte value immediately before the
/// `LOCAL_GET cur` this emits, so the sequence is `local.get cur; <byte>; i32.store8; cur += 1`.
fn emit_store_byte_advance(cur: u32, c: &mut Vec<u8>) {
    c.extend_from_slice(&[op::I32_STORE8, 0, 0]); // align=0 offset=0
    c.push(op::LOCAL_GET);
    uleb128(cur as u64, c);
    c.push(op::I32_CONST);
    sleb128(1, c);
    c.push(op::I32_ADD);
    c.push(op::LOCAL_SET);
    uleb128(cur as u64, c);
}

/// Emit the `b"…"` display escape of ONE byte held in local `b` (an i32 in 0..=255), writing at
/// cursor local `cur` and using scratch local `nib` for a hex nibble. This is the emitted-wasm
/// mirror of the const-fold `escape_byte`, and the exact inverse of the `b"…"` reader — the three
/// renderers must agree byte-for-byte, so the escape order here matches `escape_byte`: the special
/// bytes first, then printable-ASCII passthrough, then `\xNN`.
fn emit_byte_escape(b: u32, nib: u32, cur: u32, c: &mut Vec<u8>) {
    const I32_EQ: u8 = 0x46;
    const I32_EQZ: u8 = 0x45;
    const I32_AND: u8 = 0x71;
    const I32_SHR_U: u8 = 0x76;
    // A `if b == <lit> { write <esc> } else …` step; leaves an open `else` for the caller to close.
    let eq_lit = |lit: u8, esc: &[u8], c: &mut Vec<u8>| {
        c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::I32_CONST]);
        sleb128(lit as i64, c);
        c.extend_from_slice(&[I32_EQ, op::IF, 0x40]);
        emit_write_lit(esc, cur, c);
        c.push(op::ELSE);
    };
    eq_lit(b'\n', b"\\n", c);
    eq_lit(b'\r', b"\\r", c);
    eq_lit(b'\t', b"\\t", c);
    eq_lit(b'\\', b"\\\\", c);
    eq_lit(b'"', b"\\\"", c);
    // if b == 0 { write "\0" } else …  (eqz, since 0 has no signed-literal quirk)
    c.extend_from_slice(&[op::LOCAL_GET, b as u8, I32_EQZ, op::IF, 0x40]);
    emit_write_lit(b"\\0", cur, c);
    c.push(op::ELSE);
    // if (b >= 0x20) & (b <= 0x7e) { write the raw byte } else { write \xNN }
    c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::I32_CONST]);
    sleb128(0x20, c);
    c.push(op::I32_GE_U);
    c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::I32_CONST]);
    sleb128(0x7e, c);
    c.push(op::I32_LE_U);
    c.extend_from_slice(&[I32_AND, op::IF, 0x40]);
    // printable ASCII stands for itself: cur[0] = b ; cur += 1
    c.extend_from_slice(&[op::LOCAL_GET, cur as u8, op::LOCAL_GET, b as u8]);
    emit_store_byte_advance(cur, c);
    c.push(op::ELSE);
    // "\xNN": write the `\x`, then each nibble as a lowercase hex ASCII digit. A nibble n in 0..=15
    // maps to ASCII by `n + 48 + (n >= 10 ? 39 : 0)` (48='0', +39 lifts 'a'..'f' from '0'..'5').
    emit_write_lit(b"\\x", cur, c);
    for shift in [4i64, 0] {
        // nib = (b >> shift) & 0xf
        c.extend_from_slice(&[op::LOCAL_GET, b as u8]);
        if shift != 0 {
            c.push(op::I32_CONST);
            sleb128(shift, c);
            c.push(I32_SHR_U);
        }
        c.push(op::I32_CONST);
        sleb128(0xf, c);
        c.extend_from_slice(&[I32_AND, op::LOCAL_SET, nib as u8]);
        // cur[0] = nib + 48 + (nib >= 10) * 39
        c.extend_from_slice(&[op::LOCAL_GET, cur as u8]);
        c.extend_from_slice(&[op::LOCAL_GET, nib as u8, op::I32_CONST]);
        sleb128(48, c);
        c.push(op::I32_ADD);
        c.extend_from_slice(&[op::LOCAL_GET, nib as u8, op::I32_CONST]);
        sleb128(10, c);
        c.push(op::I32_GE_U);
        c.push(op::I32_CONST);
        sleb128(39, c);
        c.push(op::I32_MUL);
        c.push(op::I32_ADD);
        emit_store_byte_advance(cur, c);
    }
    // Close the 7 nested ifs: 5 eq_lit + eqz + printable-range.
    for _ in 0..7 {
        c.push(op::END);
    }
}

/// Emit the `"…"` escape of ONE byte held in local `b` (an i32 in 0..=255) of a UTF-8 String, writing
/// at cursor local `cur`. This is the emitted-wasm mirror of the const path's `string_canonical_text`,
/// so the two renderers agree byte-for-byte. The rule uses ONLY the CLOSED escape set the reader can
/// read back (collections-and-text.md §A String Literal's Escapes Are A Closed Set: exactly `\n \r \t
/// \\ \"`); EVERY other byte passes through RAW. That covers printable ASCII, every multi-byte-UTF-8
/// byte (`>= 0x80`, so `café`/`☃`/`😀` reproduce verbatim), AND the non-printable control bytes
/// (`0x00`, `0x01..=0x1f`, `0x7f`) — the last are written raw rather than `\u{…}`/`\0` BECAUSE the
/// closed set has no numeric escape, so a `\u{…}`/`\0` would read back to a different value (the
/// round-trip fix, 13-strings.sexp §"a returned runtime string … renders the scalar verbatim"). Only
/// reachable for well-formed UTF-8 (the language guarantees it), so a raw non-printable byte is either
/// ASCII or part of a valid multi-byte sequence — never the lone-byte hazard that makes Bytes use
/// `\xNN`. The `nib` scratch is no longer needed (kept in the signature for call-site stability).
fn emit_string_byte_escape(b: u32, _nib: u32, cur: u32, c: &mut Vec<u8>) {
    const I32_EQ: u8 = 0x46;
    let eq_lit = |lit: u8, esc: &[u8], c: &mut Vec<u8>| {
        c.extend_from_slice(&[op::LOCAL_GET, b as u8, op::I32_CONST]);
        sleb128(lit as i64, c);
        c.extend_from_slice(&[I32_EQ, op::IF, 0x40]);
        emit_write_lit(esc, cur, c);
        c.push(op::ELSE);
    };
    eq_lit(b'\n', b"\\n", c);
    eq_lit(b'\r', b"\\r", c);
    eq_lit(b'\t', b"\\t", c);
    eq_lit(b'\\', b"\\\\", c);
    eq_lit(b'"', b"\\\"", c);
    // else: raw passthrough — cur[0] = b ; cur += 1. Every non-closed-set byte (printable, multi-byte
    // UTF-8, OR non-printable control) is written verbatim, the only round-trippable form.
    c.extend_from_slice(&[op::LOCAL_GET, cur as u8, op::LOCAL_GET, b as u8]);
    emit_store_byte_advance(cur, c);
    // Close the 5 nested `eq_lit` ifs.
    for _ in 0..5 {
        c.push(op::END);
    }
}

/// The distinct i32 scratch locals a byte-level UTF-8 validator needs (all separate — reuse is what
/// makes a hand-emitted validator subtly wrong).
struct Utf8Locals {
    buf: u32,   // Bytes buffer handle
    n: u32,     // scan index (start of the current scalar)
    len: u32,   // buffer byte length
    lead: u32,  // current lead byte
    seq: u32,   // number of continuation bytes for this scalar (1/2/3)
    k: u32,     // continuation loop counter (1..=seq)
    cb: u32,    // current continuation byte
    lo: u32,    // legal low bound for the FIRST continuation
    hi: u32,    // legal high bound for the FIRST continuation
    valid: u32, // running validity flag (1 = still well-formed)
}

/// Emit a byte-level UTF-8 well-formedness check over the Bytes buffer in local `v.buf`, leaving
/// `v.valid` = 1 (well-formed) or 0 (ill-formed). Rejects overlong encodings, surrogates
/// (U+D800..=U+DFFF), and code points > U+10FFFF, per the Unicode UTF-8 definition — matching Rust's
/// `str::from_utf8`, so the runtime `String.from-bytes` decode agrees with the const-fold path
/// (`std::str::from_utf8`). Emitted inline over the frozen `bytes-len`/`bytes-get` imports (no new
/// runtime op / envelope change).
///
/// Per scalar: read `lead`; ASCII (`<= 0x7F`) is a complete 1-byte scalar. Otherwise classify the
/// lead into `seq` (continuation count) and the LEGAL RANGE `[lo,hi]` of the FIRST continuation —
/// narrower than `0x80..=0xBF` for `E0`/`ED`/`F0`/`F4` (that range check excludes overlong /
/// surrogate / out-of-range). Then require `seq` more bytes exist and check each continuation (the
/// first against `[lo,hi]`, the rest against `0x80..=0xBF`).
///
/// NO multi-level `br` on failure (that depth bookkeeping is what makes a hand-emitted validator
/// wrong): instead a failure just sets `valid = 0`, and the loops are GUARDED by `valid` so they run
/// to a clean finish. Outer loop continues while `n < len & valid`; the continuation inner loop while
/// `k <= seq & valid`. `n` advances by `1 + seq` (well-defined even on a failing scalar — the outer
/// guard stops the next iteration). A single-level `br` (loop back / block exit) is all that's used.
fn emit_utf8_valid(v: &Utf8Locals, c: &mut Vec<u8>) {
    const AND: u8 = 0x71;
    const OR: u8 = 0x72;
    const EQ: u8 = 0x46;
    const GE_U: u8 = 0x4F;
    const LE_U: u8 = 0x4D;
    const GT_U: u8 = 0x4B;
    let get = |c: &mut Vec<u8>, idx: &dyn Fn(&mut Vec<u8>)| {
        c.extend_from_slice(&[op::LOCAL_GET, v.buf as u8]);
        idx(c);
        c.push(op::CALL);
        uleb128(himport::BYTES_GET as u64, c);
    };
    // valid = 1 ; n = 0 ; len = bytes-len(buf)
    c.extend_from_slice(&[op::I32_CONST, 1, op::LOCAL_SET, v.valid as u8]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, v.n as u8]);
    c.extend_from_slice(&[op::LOCAL_GET, v.buf as u8, op::CALL]);
    uleb128(himport::BYTES_LEN as u64, c);
    c.extend_from_slice(&[op::LOCAL_SET, v.len as u8]);
    // A range test `(x >= lo) & (x <= hi)`, x from `xget`, leaving an i32 on the stack.
    let in_range = |c: &mut Vec<u8>, xget: &dyn Fn(&mut Vec<u8>), lo: &dyn Fn(&mut Vec<u8>), hi: &dyn Fn(&mut Vec<u8>)| {
        xget(c);
        lo(c);
        c.push(GE_U);
        xget(c);
        hi(c);
        c.push(LE_U);
        c.push(AND);
    };
    let konst = |val: i64| move |c: &mut Vec<u8>| { c.push(op::I32_CONST); sleb128(val, c); };
    let lead_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, v.lead as u8]);
    let cb_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, v.cb as u8]);
    let lo_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, v.lo as u8]);
    let hi_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, v.hi as u8]);

    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    // continue only while n < len AND valid. Exit (br 1) otherwise.
    c.extend_from_slice(&[op::LOCAL_GET, v.n as u8, op::LOCAL_GET, v.len as u8, 0x49 /*i32.lt_u*/]);
    c.extend_from_slice(&[op::LOCAL_GET, v.valid as u8, AND, 0x45 /*i32.eqz*/, op::BR_IF, 1]);
    // lead = get(n)
    get(c, &|c| c.extend_from_slice(&[op::LOCAL_GET, v.n as u8]));
    c.extend_from_slice(&[op::LOCAL_SET, v.lead as u8]);
    // seq = lead<=0x7F ? 0 : lead>=0xF0 ? 3 : lead>=0xE0 ? 2 : 1
    lead_get(c);
    konst(0x7f)(c);
    c.push(LE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 0, op::ELSE]);
    lead_get(c); konst(0xf0)(c); c.push(GE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 3, op::ELSE]);
    lead_get(c); konst(0xe0)(c); c.push(GE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 2, op::ELSE, op::I32_CONST, 1, op::END, op::END, op::END]);
    c.extend_from_slice(&[op::LOCAL_SET, v.seq as u8]);
    // Default first-continuation range 0x80..0xBF; narrow for special leads.
    konst(0x80)(c); c.extend_from_slice(&[op::LOCAL_SET, v.lo as u8]);
    konst(0xbf)(c); c.extend_from_slice(&[op::LOCAL_SET, v.hi as u8]);
    let narrow = |c: &mut Vec<u8>, leadval: i64, lo: i64, hi: i64| {
        lead_get(c); konst(leadval)(c);
        c.extend_from_slice(&[EQ, op::IF, 0x40]);
        konst(lo)(c); c.extend_from_slice(&[op::LOCAL_SET, v.lo as u8]);
        konst(hi)(c); c.extend_from_slice(&[op::LOCAL_SET, v.hi as u8]);
        c.push(op::END);
    };
    narrow(c, 0xe0, 0xa0, 0xbf);
    narrow(c, 0xed, 0x80, 0x9f);
    narrow(c, 0xf0, 0x90, 0xbf);
    narrow(c, 0xf4, 0x80, 0x8f);
    // valid &= lead-not-invalid AND enough-bytes.
    //   lead-not-invalid = !((0x80<=lead<=0xC1) | (0xF5<=lead<=0xFF))
    //   enough-bytes     = (n + seq < len)
    // valid = valid & lead-not-invalid
    c.extend_from_slice(&[op::LOCAL_GET, v.valid as u8]);
    in_range(c, &lead_get, &konst(0x80), &konst(0xc1));
    in_range(c, &lead_get, &konst(0xf5), &konst(0xff));
    c.push(OR);
    c.push(0x45 /*eqz → not-invalid*/);
    c.push(AND);
    // & enough-bytes: (n + seq) < len
    c.extend_from_slice(&[op::LOCAL_GET, v.n as u8, op::LOCAL_GET, v.seq as u8, op::I32_ADD]);
    c.extend_from_slice(&[op::LOCAL_GET, v.len as u8, 0x49 /*lt_u*/, AND]);
    c.extend_from_slice(&[op::LOCAL_SET, v.valid as u8]);
    // Check continuations only if still valid AND there are enough bytes to read them safely.
    c.extend_from_slice(&[op::LOCAL_GET, v.valid as u8, op::IF, 0x40]);
    c.extend_from_slice(&[op::I32_CONST, 1, op::LOCAL_SET, v.k as u8]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    // continue inner while k <= seq AND valid. (k > seq | !valid) → exit.
    c.extend_from_slice(&[op::LOCAL_GET, v.k as u8, op::LOCAL_GET, v.seq as u8, GT_U]);
    c.extend_from_slice(&[op::LOCAL_GET, v.valid as u8, 0x45 /*eqz*/, OR, op::BR_IF, 1]);
    // cb = get(n + k)
    get(c, &|c| c.extend_from_slice(&[op::LOCAL_GET, v.n as u8, op::LOCAL_GET, v.k as u8, op::I32_ADD]));
    c.extend_from_slice(&[op::LOCAL_SET, v.cb as u8]);
    // cbok = (k==1) ? cb in [lo,hi] : cb in [0x80,0xBF]
    c.extend_from_slice(&[op::LOCAL_GET, v.k as u8, op::I32_CONST, 1, EQ, op::IF, 0x7F]);
    in_range(c, &cb_get, &lo_get, &hi_get);
    c.push(op::ELSE);
    in_range(c, &cb_get, &konst(0x80), &konst(0xbf));
    c.push(op::END);
    // valid = valid & cbok
    c.extend_from_slice(&[op::LOCAL_GET, v.valid as u8, AND, op::LOCAL_SET, v.valid as u8]);
    // k += 1 ; loop
    c.extend_from_slice(&[op::LOCAL_GET, v.k as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, v.k as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // inner loop, inner block
    c.push(op::END); // if valid (continuation check)
    // n += 1 + seq ; loop
    c.extend_from_slice(&[op::LOCAL_GET, v.n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_GET, v.seq as u8, op::I32_ADD, op::LOCAL_SET, v.n as u8, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // outer loop, outer block
}

/// The TYPE-DIRECTED, tag-free renderer. Given `main`'s result `Shape`, emit one render function
/// per distinct COMPOUND shape (each `(handle:i32, cursor:i32) -> i32` that writes the value's
/// canonical text at the cursor and returns the advanced cursor, walking the value through the
/// runtime's accessors) plus the `run` body. Returns `(render_bodies, run_body)`, or a decline
/// if the shape contains a leaf the renderer cannot produce yet (runtime float/string). The
/// render functions are laid out AFTER the user + helper functions, so their absolute wasm index
/// is `RT_FUNC_BASE + n_user + n_helpers + <position>`.
struct Renderer {
    /// Distinct compound shapes, in emission order; index in this vec = the render fn's position.
    shapes: Vec<Shape>,
    render_base: u32,
    /// A recursive sum type's NAME → its full `Sum` shape. A `Shape::Rec(T)` payload position
    /// (a recursive self-reference the finite shape tree cannot inline) renders by resolving to
    /// `type_shapes[T]` and calling ITS render fn — so a `Cons` whose tail is `Rec(IntList)` emits a
    /// CALL back into the IntList render fn, walking the runtime spine to its actual depth. Populated
    /// (before emission) by scanning the top shape for every `Sum`, keyed by its type name.
    type_shapes: std::collections::BTreeMap<String, Shape>,
}

impl Renderer {
    /// The absolute wasm index of the render fn for compound shape at position `pos`.
    fn fn_index(&self, pos: usize) -> u32 {
        self.render_base + pos as u32
    }

    /// Find or assign a render-fn position for a COMPOUND shape (scalars render inline, no fn).
    fn intern(&mut self, s: &Shape) -> usize {
        if let Some(p) = self.shapes.iter().position(|x| x == s) {
            return p;
        }
        self.shapes.push(s.clone());
        self.shapes.len() - 1
    }

    /// Emit code to render the value in `handle`-holding local `h`, writing at cursor local
    /// `cur`, for a shape that is either a scalar (inline) or a compound (call its render fn).
    /// Leaves the advanced cursor in `cur`. `c` is the code buffer.
    fn emit_render_into(&mut self, s: &Shape, h_expr: &[u8], cur: u32, c: &mut Vec<u8>) -> Result<(), Decline> {
        match s {
            Shape::Int => {
                // cur = itoa(get_int(h), cur)
                c.extend_from_slice(h_expr);
                c.push(op::CALL);
                uleb128(himport::GET_INT as u64, c);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(RT_ITOA as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
            Shape::Bool => {
                // if get_bool(h) { write "true" } else { write "false" }
                c.extend_from_slice(h_expr);
                c.push(op::CALL);
                uleb128(himport::GET_BOOL as u64, c);
                c.push(op::IF);
                c.push(0x40);
                emit_write_lit(b"true", cur, c);
                c.push(op::ELSE);
                emit_write_lit(b"false", cur, c);
                c.push(op::END);
                Ok(())
            }
            Shape::Unit => {
                let _ = h_expr; // unit has no payload to read
                emit_write_lit(b"unit", cur, c);
                Ok(())
            }
            Shape::Float => {
                decline("runtime float leaf rendering not yet emitted")
            }
            // A recursive back-reference: resolve to the named type's full `Sum` shape and render
            // THROUGH it — interning the resolved `Sum` (not the `Rec`), so this call and the top-
            // level occurrence of the same type resolve to ONE render fn. That fn's own `Rec(T)`
            // payload positions re-enter here and call the SAME fn, so the walk recurses to the
            // value's runtime depth (the render dual of runtime sum-match consumption). A dangling
            // `Rec` (type not registered — never happens for a shape reached from a real top value)
            // declines rather than emitting a wrong walk.
            Shape::Rec(type_name) => {
                let resolved = match self.type_shapes.get(type_name) {
                    Some(s) => s.clone(),
                    None => return decline("recursive render shape references an unregistered type"),
                };
                let pos = self.intern(&resolved);
                c.extend_from_slice(h_expr);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(self.fn_index(pos) as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
            // A String renders via its own render fn (a `"…"` escaping loop), like Bytes — it is not
            // an inline scalar. Fall through to the compound-shape call path below.
            // A compound: call its render fn with (h, cur), take the returned cursor.
            _ => {
                let pos = self.intern(s);
                c.extend_from_slice(h_expr);
                c.push(op::LOCAL_GET);
                uleb128(cur as u64, c);
                c.push(op::CALL);
                uleb128(self.fn_index(pos) as u64, c);
                c.push(op::LOCAL_SET);
                uleb128(cur as u64, c);
                Ok(())
            }
        }
    }

    /// Emit the body of the render fn for a COMPOUND `shape`. Locals: 0=handle, 1=cursor(param),
    /// 2=working cursor (i32). Returns the working cursor. For `List` it also needs a loop
    /// counter (local 3) — declared when needed.
    fn emit_render_fn(&mut self, shape: &Shape) -> Result<Body, Decline> {
        // Local 2 = working cursor, initialized from the cursor param (local 1).
        let cur = 2u32;
        let mut c = Vec::new();
        c.extend_from_slice(&[op::LOCAL_GET, 1, op::LOCAL_SET, cur as u8]);
        let mut extra_locals = vec![Kind::Bool]; // local 2
        // The handle is local 0; a sub-element handle is `arr_get(local0, i)`.
        let get_elem = |i: usize| -> Vec<u8> {
            let mut e = vec![op::LOCAL_GET, 0, op::I32_CONST];
            let mut t = Vec::new();
            sleb128(i as i64, &mut t);
            e.extend_from_slice(&t);
            e.push(op::CALL);
            let mut idx = Vec::new();
            uleb128(himport::ARR_GET as u64, &mut idx);
            e.extend_from_slice(&idx);
            e
        };
        match shape {
            Shape::Tuple(elems) => {
                emit_write_lit(b"(tuple", cur, &mut c);
                for (i, e) in elems.iter().enumerate() {
                    emit_write_lit(b" ", cur, &mut c);
                    let h = get_elem(i);
                    self.emit_render_into(e, &h, cur, &mut c)?;
                }
                emit_write_lit(b")", cur, &mut c);
            }
            Shape::Record(fields) => {
                emit_write_lit(b"(record", cur, &mut c);
                for (i, (k, v)) in fields.iter().enumerate() {
                    let mut open = Vec::from(&b" ("[..]);
                    open.extend_from_slice(k.as_bytes());
                    open.push(b' ');
                    emit_write_lit(&open, cur, &mut c);
                    let h = get_elem(i);
                    self.emit_render_into(v, &h, cur, &mut c)?;
                    emit_write_lit(b")", cur, &mut c);
                }
                emit_write_lit(b")", cur, &mut c);
            }
            Shape::List(elem) => {
                // A runtime list is backed by the value-heap runtime's 32-way radix trie, so the
                // renderer walks it via `vec-len`/`vec-get` (NOT the flat `arr-*` array, which backs
                // a fixed-arity tuple/record). It still renders `(list e0 e1 …)` — the representation
                // is unobservable (collections-and-text.md #A List's Representation Is Unspecified And
                // Unobservable). Reject a not-yet-renderable element up front (decline-don't-miscompile).
                if matches!(**elem, Shape::Float) {
                    return decline("runtime list of float not yet emitted");
                }
                let n = 3u32; // loop counter local
                extra_locals.push(Kind::Bool); // local 3
                emit_write_lit(b"(list", cur, &mut c);
                // i = 0
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
                // block { loop { if i >= vec-len(h) break; write ' ' ; render elem ; i+=1 } }
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                // i >= vec-len(h) → br 1
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::VEC_LEN as u64, &mut c);
                c.extend_from_slice(&[0x4E /*i32.ge_s*/, op::BR_IF, 1]);
                emit_write_lit(b" ", cur, &mut c);
                // elem handle = vec-get(h, i)
                let h = vec![op::LOCAL_GET, 0, op::LOCAL_GET, n as u8, op::CALL, himport::VEC_GET as u8];
                self.emit_render_into(elem, &h, cur, &mut c)?;
                // i += 1 ; continue
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]); // loop, block
                emit_write_lit(b")", cur, &mut c);
            }
            Shape::Bytes => {
                // `b"…"` — the byte-string display form (matching the `bytes` crate's `Debug`, and
                // the exact inverse of the `b"…"` reader). Loop i in 0..bytes-len(h), reading each
                // byte via the runtime `bytes-*` shape (imports 13/15) and escaping it per byte. The
                // text is byte-identical to the const `bytes_literal_text` render and the reader's
                // input form, so a rendered byte sequence reads back to the same value.
                let n = 3u32; // loop counter local
                let bv = 4u32; // current byte value (i32, 0..=255)
                let nib = 5u32; // hex-nibble scratch (i32)
                extra_locals.push(Kind::Bool); // local 3
                extra_locals.push(Kind::Bool); // local 4
                extra_locals.push(Kind::Bool); // local 5
                emit_write_lit(b"b\"", cur, &mut c);
                // i = 0
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
                // block { loop { if i >= bytes-len(h) break; bv = bytes-get(h,i); escape bv; i+=1 } }
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                // i >= bytes-len(h) → br 1
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::BYTES_LEN as u64, &mut c);
                c.extend_from_slice(&[0x4E /*i32.ge_s*/, op::BR_IF, 1]);
                // bv = bytes-get(h, i) — a byte is an i32 in 0..=255
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, n as u8, op::CALL]);
                uleb128(himport::BYTES_GET as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, bv as u8]);
                emit_byte_escape(bv, nib, cur, &mut c);
                // i += 1 ; continue
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]); // loop, block
                emit_write_lit(b"\"", cur, &mut c);
            }
            Shape::Str => {
                // `"…"` — the String display form, byte-identical to the const path's
                // `string_canonical_text`. A String is a Bytes-backed UTF-8 leaf, so loop over its
                // bytes writing the quoted, CLOSED-SET-escaped text. The escaping is applied PER BYTE
                // (`emit_string_byte_escape`): named escapes for the closed set `\n \r \t \\ \"` ONLY,
                // and raw passthrough for EVERY other byte — printable-ASCII, multi-byte-UTF-8 (≥
                // 0x80, so `café`/`☃`/`😀` reproduce verbatim), AND non-printable control bytes
                // (written raw, not `\u{…}`, since the closed set has no numeric escape — the
                // round-trip fix so a rendered string reads back to the same value).
                let n = 3u32; // loop counter local
                let bv = 4u32; // current byte value (i32, 0..=255)
                let nib = 5u32; // hex-nibble scratch (i32)
                extra_locals.push(Kind::Bool); // local 3
                extra_locals.push(Kind::Bool); // local 4
                extra_locals.push(Kind::Bool); // local 5
                emit_write_lit(b"\"", cur, &mut c);
                // i = 0
                c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n as u8]);
                c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
                // i >= bytes-len(h) → br 1
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::BYTES_LEN as u64, &mut c);
                c.extend_from_slice(&[0x4E /*i32.ge_s*/, op::BR_IF, 1]);
                // bv = bytes-get(h, i)
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, n as u8, op::CALL]);
                uleb128(himport::BYTES_GET as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, bv as u8]);
                emit_string_byte_escape(bv, nib, cur, &mut c);
                // i += 1 ; continue
                c.extend_from_slice(&[op::LOCAL_GET, n as u8, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, n as u8, op::BR, 0]);
                c.extend_from_slice(&[op::END, op::END]); // loop, block
                emit_write_lit(b"\"", cur, &mut c);
            }
            Shape::Sum(variants) => {
                // disc = sum-disc(handle) ; switch on it. Local 3 holds the discriminant.
                let d = 3u32;
                extra_locals.push(Kind::Bool); // local 3
                c.extend_from_slice(&[op::LOCAL_GET, 0, op::CALL]);
                uleb128(himport::SUM_DISC as u64, &mut c);
                c.extend_from_slice(&[op::LOCAL_SET, d as u8]);
                // payload handle = sum-payload(handle) — reused by every arm.
                let payload_h = vec![op::LOCAL_GET, 0, op::CALL, himport::SUM_PAYLOAD as u8];
                // Nested if/else: for each variant i, `if disc == i { "(Name " payload ")" }`.
                // A well-typed sum always matches one arm, so the innermost `else` is unreachable;
                // emit `unreachable` there to keep the value stack typed.
                for (i, (name, payload_shape)) in variants.iter().enumerate() {
                    c.extend_from_slice(&[op::LOCAL_GET, d as u8, op::I32_CONST]);
                    sleb128(i as i64, &mut c);
                    c.extend_from_slice(&[0x46 /*i32.eq*/, op::IF, 0x40]);
                    let mut open = Vec::from(&b"("[..]);
                    open.extend_from_slice(name.as_bytes());
                    open.push(b' ');
                    emit_write_lit(&open, cur, &mut c);
                    self.emit_render_into(payload_shape, &payload_h, cur, &mut c)?;
                    emit_write_lit(b")", cur, &mut c);
                    c.push(op::ELSE);
                }
                c.push(op::UNREACHABLE); // no discriminant matched — impossible for a well-typed sum
                for _ in variants.iter() {
                    c.push(op::END);
                }
            }
            _ => return decline("render fn requested for a non-compound shape"),
        }
        c.extend_from_slice(&[op::LOCAL_GET, cur as u8]);
        Ok(Body { extra_locals, code: c })
    }
}

/// Build the type-directed renderer for `main`'s result `top` shape: the per-shape render
/// functions and the `run` body. `n_user`/`n_helpers` fix where the render functions sit in the
/// index space (after the user + helper functions). Declines if a leaf is not yet renderable.
fn emit_renderer(
    top: &Shape,
    n_user: usize,
    n_helpers: usize,
    n_specs: usize,
    type_shapes: std::collections::BTreeMap<String, Shape>,
) -> Result<(Vec<Body>, Body), Decline> {
    // Render fns sit AFTER the effect-context specializations (which sit after helpers), so a
    // render fn's absolute index is `RT_FUNC_BASE + n_user + n_helpers + n_specs + position`
    // (`[fixed][user][helpers][SPECS][render][run]`). `main`'s call in `run` is still `RT_FUNC_BASE`
    // (user func 0, unchanged); only render fns shift past the specs. ask-49.
    let render_base = RT_FUNC_BASE + (n_user + n_helpers + n_specs) as u32;
    let mut r = Renderer { shapes: Vec::new(), render_base, type_shapes };

    // The top-level value's render — a scalar renders inline in `run`; a compound gets a fn we
    // call from `run`. Either way we produce the render code for `run` first, THEN drain the
    // worklist of compound shapes reached during it (and transitively).
    // Cursor buffer starts at HEAP_STR_BASE; the value heap grows above it. The realloc bump
    // global starts at 16, and the runtime's own heap is separate (in the runtime instance);
    // the program's memory only holds the output string, so we assemble it from offset 16.
    const STR_BASE: i64 = 16;

    // run: local 0 = handle (i32), local 1 = cursor (i32).
    let mut run = Vec::new();
    // handle = call main
    run.push(op::CALL);
    uleb128(RT_FUNC_BASE as u64, &mut run); // main is user func 0
    run.extend_from_slice(&[op::LOCAL_SET, 0]);
    // cursor = STR_BASE
    run.push(op::I32_CONST);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::LOCAL_SET, 1]);
    // render the top value (h = local 0) into cursor (local 1)
    let h_top = vec![op::LOCAL_GET, 0];
    r.emit_render_into(top, &h_top, 1, &mut run)?;
    // write the (ptr,len) return pair at offset 0: ptr = STR_BASE, len = cursor - STR_BASE
    run.push(op::I32_CONST);
    sleb128(0, &mut run);
    run.push(op::I32_CONST);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::I32_STORE, 2, 0]); // align=2 off=0
    run.push(op::I32_CONST);
    sleb128(4, &mut run);
    run.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST]);
    sleb128(STR_BASE, &mut run);
    run.extend_from_slice(&[op::I32_SUB, op::I32_STORE, 2, 0]);
    // return retptr 0
    run.extend_from_slice(&[op::I32_CONST, 0]);
    let run_body = Body { extra_locals: vec![Kind::Bool, Kind::Bool], code: run };

    // Drain the worklist: emit a body for each interned compound shape. Emitting a body may
    // intern further nested compound shapes, so iterate until the vec stops growing.
    let mut bodies: Vec<Body> = Vec::new();
    let mut i = 0;
    while i < r.shapes.len() {
        let shape = r.shapes[i].clone();
        let body = r.emit_render_fn(&shape)?;
        bodies.push(body);
        i += 1;
    }
    Ok((bodies, run_body))
}

/// The memory section a runnable core module needs: one memory, min 1 page, no maximum.
fn emit_memory_section() -> Vec<u8> {
    section(5, &wasm_vec(1, &[0x00, 0x01]))
}

/// The global section holding the bump pointer: one mutable i32 global initialized to `base`.
fn emit_bump_global(base: i64) -> Vec<u8> {
    let mut glob = vec![0x7F, 0x01, op::I32_CONST]; // valtype i32, mutable
    sleb128(base, &mut glob);
    glob.push(op::END);
    section(6, &wasm_vec(1, &glob))
}

/// The body code of `cabi_realloc(orig, align, old_size, new_size) -> ptr`: a bump allocator
/// over global 0 that returns the old top and advances by `new_size` (param local 3). It needs
/// one extra i32 local (index 4) to hold the returned old-top. This is the shared allocator the
/// heap constructors also call (by the same core-func index the envelope links).
fn realloc_body_code() -> Vec<u8> {
    vec![
        op::GLOBAL_GET, 0, // old top
        op::LOCAL_SET, 4,  // ret = old top
        op::GLOBAL_GET, 0,
        op::LOCAL_GET, 3,  // new_size
        op::I32_ADD,
        op::GLOBAL_SET, 0, // bump
        op::LOCAL_GET, 4,  // return old top
    ]
}

/// Canonical float text (matches the corpus value form and `host::display_float`).
pub fn display_float_text(f: f64) -> String {
    if f == 0.0 && f.is_sign_negative() {
        "-0.0".into()
    } else if f.is_nan() {
        "NaN".into()
    } else if f.fract() == 0.0 && f.is_finite() {
        // `{:.0}` prints the exact integer value of the whole float with no fractional digits,
        // then `.0` marks it a float. This is INJECTIVE across all finite whole floats — unlike
        // `f as i64`, which SATURATES at i64::MAX so every distinct whole float ≥ 2^63 (1e19,
        // 1e20, 1e100) collapsed to `9223372036854775807.0`, violating deterministic-value-form.md
        // §"Numeric Values Serialize Deterministically" (unequal floats MUST have distinct canonical
        // encodings). Kept in lock-step with host::display_float.
        format!("{f:.0}.0")
    } else {
        format!("{f}")
    }
}

/// The canonical text of an AST node value (what `quote` produced) — the `(Ast.Kind …)` form
/// the corpus records, e.g. `(Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))`.
fn ast_canonical_text(node: &Node) -> String {
    match node {
        Node::Int(n) => format!("(Ast.Int {n})"),
        Node::Float(f) => format!("(Ast.Float {})", display_float_text(*f)),
        Node::Str(s) => format!("(Ast.Str {s:?})"),
        Node::Bool(b) => format!("(Ast.Bool {b})"),
        Node::Name(n) => format!("(Ast.Name {n:?})"),
        Node::List(items) => {
            let parts: Vec<String> = items.iter().map(ast_canonical_text).collect();
            format!("(Ast.List (list {}))", parts.join(" "))
        }
    }
}

// ─── Compound value returns (the component value ABI) ──────────────────────────────
//
// A `main` returning a compound value (string, …) crosses the component boundary via the
// canonical value ABI: the core module exports linear `memory` and a `cabi_realloc`, and
// `run` returns a pointer to the lifted representation. Every compound result the realized
// corpus produces is a compile-time constant, so the core `run` simply writes constant bytes
// into memory and returns the retptr. The canonical lift in the component envelope reads it.

/// Build a complete component for a compile-time-constant compound `main` result, or None if
/// the value form is not yet lowered. A compound value crosses the boundary as its proper type
/// — a component RESOURCE owning a `display()` method that returns the value's canonical text
/// (constitution VII strict typed boundary; the host is value-agnostic and just calls
/// `.display()`). A String is a compound whose canonical text is the quoted literal; other
/// compounds (sum/tuple/bytes/AST) render via `print_cval`.
fn compound_component(v: &CVal) -> Option<Vec<u8>> {
    let text = canonical_text(v)?;
    Some(runnable_component(text.as_bytes()))
}

/// Is this a COMPOUND compile-time value (one with no scalar wasm representation, returned via
/// the resource-with-display ABI)? A scalar (Int/Bool/Float/unit) is NOT compound — it crosses
/// the boundary directly and must go through the normal scalar compile (which type-checks).
fn is_compound_cval(v: &CVal) -> bool {
    match v {
        CVal::Int(_) | CVal::Bool(_) | CVal::Float(_) => false,
        CVal::Tuple(t) if t.is_empty() => false, // unit is scalar-ish (empty result)
        _ => true,
    }
}

/// Append the `b"…"` display form of one byte to `out`, matching the `bytes` crate's `Debug`
/// impl — and the EXACT inverse of the `b"…"` reader escape (ast.rs `read_byte_string`), so a
/// rendered byte sequence reads back to the same value (round-trips). The escape order is
/// load-bearing: `\` and `"` fall inside the printable ASCII range `0x20..=0x7e`, so they MUST be
/// matched before the printable-passthrough arm.
pub fn escape_byte(b: u8, out: &mut String) {
    match b {
        b'\n' => out.push_str("\\n"),
        b'\r' => out.push_str("\\r"),
        b'\t' => out.push_str("\\t"),
        b'\\' => out.push_str("\\\\"),
        b'"' => out.push_str("\\\""),
        0 => out.push_str("\\0"),
        0x20..=0x7e => out.push(b as char), // printable ASCII stands for itself
        _ => {
            out.push_str("\\x");
            const HEX: &[u8; 16] = b"0123456789abcdef";
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
    }
}

/// The canonical `b"…"` display text of a byte sequence — the observable form the corpus records
/// and every renderer (const fold, emitted wasm, runtime crate) reproduces byte-for-byte. Shared
/// so the three renderers and the oracle cannot drift.
pub fn bytes_literal_text(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 3);
    out.push('b');
    out.push('"');
    for &b in bytes {
        escape_byte(b, &mut out);
    }
    out.push('"');
    out
}

/// The canonical TEXT form of a compile-time value — the exact rendering the corpus records as
/// the observable (`(Some 42)`, `(tuple 1 true)`, `"hello"`, …). Returns None for a value with
/// no defined canonical text yet.
/// A stable sort key for a map key VALUE — its canonical text, or a fallback so sorting never
/// panics on a key `canonical_text` cannot render (which does not arise for the scalar keys the
/// realized corpus uses, but keeps the sort total). Map entries are kept sorted by this so equality
/// and rendering are insertion-order-independent (collections-and-text.md §A Map Renders As Its
/// Entries In Canonical Key Order).
fn cval_canonical_key(k: &CVal) -> String {
    canonical_text(k).unwrap_or_default()
}

/// `Map.lookup` over a compile-time map: `(Some v)` when `key` (by value) is present, else
/// `(None unit)` — total (collections-and-text.md, the map clause of §Indexing And Lookup Are
/// Fallible, Not Trapping).
fn map_lookup_cval(m: &[(CVal, CVal)], key: &CVal) -> CVal {
    match m.iter().find(|(k, _)| cval_eq(k, key)) {
        Some((_, v)) => CVal::Sum { variant: "Some".into(), payload: Box::new(v.clone()) },
        None => CVal::Sum { variant: "None".into(), payload: Box::new(CVal::unit()) },
    }
}

/// `Map.insert` over a compile-time map: a NEW entry list with `key ↦ val`, REPLACING `key`'s value
/// if present (each key at most once) else adding it, re-sorted by canonical key form (order-
/// independent — collections-and-text.md §A Map Is Built By Functional Construction).
fn map_insert_cval(m: &[(CVal, CVal)], key: CVal, val: CVal) -> Vec<(CVal, CVal)> {
    let mut out: Vec<(CVal, CVal)> = m.to_vec();
    if let Some(slot) = out.iter_mut().find(|(k, _)| cval_eq(k, &key)) {
        slot.1 = val;
    } else {
        out.push((key, val));
    }
    out.sort_by(|a, b| cval_canonical_key(&a.0).cmp(&cval_canonical_key(&b.0)));
    out
}

/// `Map.remove` over a compile-time map: `(new-map-without-key, removed-value-as-Option)`. Removing
/// an absent key yields a map equal to the operand and `(None unit)` — removal is total
/// (collections-and-text.md §A Map Is Built By Functional Construction). The map's order is
/// preserved (already canonical-key-sorted; dropping one entry keeps the rest sorted).
fn map_remove_cval(m: &[(CVal, CVal)], key: &CVal) -> (Vec<(CVal, CVal)>, CVal) {
    let removed = match m.iter().find(|(k, _)| cval_eq(k, key)) {
        Some((_, v)) => CVal::Sum { variant: "Some".into(), payload: Box::new(v.clone()) },
        None => CVal::Sum { variant: "None".into(), payload: Box::new(CVal::unit()) },
    };
    let out: Vec<(CVal, CVal)> = m.iter().filter(|(k, _)| !cval_eq(k, key)).cloned().collect();
    (out, removed)
}

/// Render a String value's canonical `"…"` text using ONLY the closed escape set the reader can read
/// back (collections-and-text.md §A String Literal's Escapes Are A Closed Set: exactly `\n \t \r \\
/// \"`). Every other scalar — including non-printable control chars (U+0007 BEL, U+007F DEL,
/// zero-width U+200B) and NUL — is written VERBATIM as its raw UTF-8 bytes, because the closed set has
/// NO numeric escape (`\u{…}` / `\0` are not recognized), so escaping them would produce text that
/// reads back to a DIFFERENT value. This is the round-trip fix (13-strings.sexp §"a returned runtime
/// string … renders the scalar verbatim" — a rendered string MUST read back to the same value):
/// `format!("{s:?}")` emits `\u{7}`/`\0` for these, which `read("\u{7}")` decodes as the four
/// characters `u{7}` (or `\0` as `0`), breaking equality. The emitted-wasm renderer
/// (`emit_string_byte_escape`) mirrors this byte-for-byte. The bytes are always well-formed UTF-8 (the
/// language guarantees it), so writing a raw non-printable scalar's bytes is safe.
pub fn string_canonical_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // Everything else — printable OR non-printable — verbatim; the closed set has no other
            // escape, so a raw scalar is the only form that reads back to the same value.
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn canonical_text(v: &CVal) -> Option<String> {
    Some(match v {
        CVal::Int(n) => n.to_string(),
        CVal::Bool(b) => b.to_string(),
        CVal::Float(f) => crate::codegen::display_float_text(*f),
        CVal::Str(s) => string_canonical_text(s), // quoted, closed-set escapes only (round-trips)
        CVal::Tuple(t) if t.is_empty() => "unit".to_string(), // unit is the empty tuple
        CVal::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(canonical_text).collect::<Option<_>>()?;
            format!("(tuple {})", parts.join(" "))
        }
        CVal::List(elems) => {
            let parts: Vec<String> = elems.iter().map(canonical_text).collect::<Option<_>>()?;
            format!("(list {})", parts.join(" "))
        }
        CVal::Bytes(b) => bytes_literal_text(b),
        CVal::Record(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, val)| Some(format!("({k} {})", canonical_text(val)?)))
                .collect::<Option<_>>()?;
            format!("(record {})", parts.join(" "))
        }
        CVal::Sum { variant, payload } => {
            // A nullary variant carries unit and renders `(Variant unit)`; a payload variant
            // renders `(Variant <payload>)`. The variant name is as declared/qualified.
            format!("({variant} {})", canonical_text(payload)?)
        }
        CVal::Ast(node) => ast_canonical_text(node),
        // A map renders `(map (k v) …)` with its entries in CANONICAL KEY ORDER — the deterministic
        // order-independent form (collections-and-text.md §A Map Renders As Its Entries In Canonical
        // Key Order, §Map Iteration Is Deterministic). Keys are VALUES; each `(k v)` pair renders both
        // by `canonical_text`, and the entries are sorted by the KEY's canonical text so two equal maps
        // render identically regardless of insertion order. (The stored vec is already key-sorted by
        // the map ops, but sort here too so any construction path renders canonically.)
        CVal::Map(entries) => {
            let mut parts: Vec<String> = entries
                .iter()
                .map(|(k, val)| Some((canonical_text(k)?, canonical_text(val)?)))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .map(|(k, v)| format!("({k} {v})"))
                .collect();
            parts.sort();
            format!("(map {})", parts.join(" "))
        }
    })
}

/// A component exporting `run` as a resource `value` owning `display : () -> string`, whose
/// `display` returns the constant `text`. This is the resource-with-display boundary the spec
/// pins for a compound result. The core module (make + display + realloc + memory + a
/// resource.new import) embeds the text; the component envelope — the wit-bindgen inner-
/// component resource-linking pattern — is FIXED (independent of the text) and appended
/// verbatim from a wasm-tools-validated reference.
fn runnable_component(text: &[u8]) -> Vec<u8> {
    const RET: i64 = 0; // (ptr,len) pair for the returned string
    const DATA: i64 = 8; // the string bytes

    // display(rep) -> retptr : store the text bytes, write the (ptr,len) pair, return RET.
    let mut display = Vec::new();
    for (i, b) in text.iter().enumerate() {
        display.push(op::I32_CONST);
        sleb128(DATA + i as i64, &mut display);
        display.push(op::I32_CONST);
        sleb128(*b as i64, &mut display);
        display.extend_from_slice(&[op::I32_STORE8, 0x00, 0x00]); // i32.store8 align=0 off=0
    }
    display.push(op::I32_CONST);
    sleb128(RET, &mut display);
    display.push(op::I32_CONST);
    sleb128(DATA, &mut display);
    display.extend_from_slice(&[op::I32_STORE, 0x02, 0x00]); // i32.store (ptr) align=2 off=0
    display.push(op::I32_CONST);
    sleb128(RET + 4, &mut display);
    display.push(op::I32_CONST);
    sleb128(text.len() as i64, &mut display);
    display.extend_from_slice(&[op::I32_STORE, 0x02, 0x00]); // i32.store (len)
    display.push(op::I32_CONST);
    sleb128(RET, &mut display);

    // realloc: bump allocator on global 0 (local 4 = ret). Same as string_component's.
    let realloc = realloc_body_code();
    // make() -> rep : push 0, call the resource.new import (func 0).
    let make = vec![op::I32_CONST, 0, op::CALL, 0];

    // ── Core module (matches the reference layout) ──
    // Types: 0=(i32)->i32, 1=(i32×4)->i32, 2=()->i32.
    let mut types = vec![0x60];
    types.extend_from_slice(&wasm_vec(1, &[0x7F]));
    types.extend_from_slice(&wasm_vec(1, &[0x7F]));
    types.push(0x60);
    types.extend_from_slice(&wasm_vec(4, &[0x7F, 0x7F, 0x7F, 0x7F]));
    types.extend_from_slice(&wasm_vec(1, &[0x7F]));
    types.push(0x60);
    types.extend_from_slice(&wasm_vec(0, &[]));
    types.extend_from_slice(&wasm_vec(1, &[0x7F]));
    let type_sec = section(1, &wasm_vec(3, &types));

    // Import section: intr.new : type 0 → core func 0.
    let mut imp = Vec::new();
    uleb128(4, &mut imp);
    imp.extend_from_slice(b"intr");
    uleb128(3, &mut imp);
    imp.extend_from_slice(b"new");
    imp.extend_from_slice(&[0x00, 0]); // func, type 0
    let import_sec = section(2, &wasm_vec(1, &imp));

    // Function section: func1=realloc(type1), func2=make(type2), func3=display(type0).
    let func_sec = section(3, &wasm_vec(3, &[1, 2, 0]));

    // Memory + global (as in string_component).
    let mem_sec = emit_memory_section();
    let glob_sec = emit_bump_global(HEAP_BASE);

    // Export section: memory, cabi_realloc(f1), make(f2), display(f3).
    let mut exports = Vec::new();
    exports.extend_from_slice(&export_entry("memory", 0x02, 0));
    exports.extend_from_slice(&export_entry("cabi_realloc", 0x00, 1));
    exports.extend_from_slice(&export_entry("make", 0x00, 2));
    exports.extend_from_slice(&export_entry("display", 0x00, 3));
    let export_sec = section(7, &wasm_vec(4, &exports));

    // Code section: realloc (1 local i32), make (no locals), display (no locals).
    let realloc_body = encode_body(&Body { extra_locals: vec![Kind::Bool], code: realloc });
    let make_body = encode_body(&Body { extra_locals: vec![], code: make });
    let display_body = encode_body(&Body { extra_locals: vec![], code: display });
    let mut code_items = Vec::new();
    code_items.extend_from_slice(&realloc_body);
    code_items.extend_from_slice(&make_body);
    code_items.extend_from_slice(&display_body);
    let code_sec = section(10, &wasm_vec(3, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&glob_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);

    // ── Component: header + embedded core module + the FIXED resource-linking envelope. ──
    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x0D, 0x00, 0x01, 0x00]);
    out.push(1);
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(&core);
    out.extend_from_slice(RUNNABLE_ENVELOPE_TAIL);
    out
}

/// A component exporting `run : () -> string` that returns the constant UTF-8 `bytes`.
///
/// Memory layout the core `run` establishes: the string bytes live at offset `DATA` (8); the
/// `(ptr,len)` return pair the canonical ABI reads lives at offset `RET` (0). `run` returns
/// `RET`. A bump-allocator `cabi_realloc` (from offset 64) satisfies the ABI's realloc import

/// A core export entry: `<name-len> <name> <kind> <index>`.
fn export_entry(name: &str, kind: u8, index: u32) -> Vec<u8> {
    let mut out = Vec::new();
    uleb128(name.len() as u64, &mut out);
    out.extend_from_slice(name.as_bytes());
    out.push(kind);
    uleb128(index as u64, &mut out);
    out
}

/// Wrap a string-returning core module in the component envelope. The envelope is FIXED —
/// independent of the string's content (the bytes live in the embedded core module) — so it
/// is emitted verbatim from a reference produced by `wasm-tools` (a `() -> string` component
/// that instantiates the core module, aliases its `run`/`memory`/`cabi_realloc`, canon-lifts
/// `run` with `(memory 0) (realloc 1) utf8`, and exports it as `run`). Only the embedded core

// ─── Overflow-checked arithmetic helper bodies ─────────────────────────────────────
//
// Each helper is a wasm function (i64 a=local0, i64 b=local1) -> i64 that traps on signed
// overflow via `unreachable`, so call sites stay tiny and never collide on scratch locals.

/// `checked_add`: r = a+b (wrapping); overflow iff (a^r)&(b^r) < 0.
fn checked_add_body() -> Body {
    let mut c = Vec::new();
    // r (local 2) = a + b
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 1, op::I64_ADD, op::LOCAL_SET, 2]);
    // (a ^ r) & (b ^ r)
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 2, op::I64_XOR]);
    c.extend_from_slice(&[op::LOCAL_GET, 1, op::LOCAL_GET, 2, op::I64_XOR]);
    c.push(op::I64_AND);
    // < 0 ?  → trap
    c.push(op::I64_CONST);
    sleb128(0, &mut c);
    c.push(op::I64_LT_S);
    c.extend_from_slice(&[op::IF, 0x40, op::UNREACHABLE, op::END]);
    // return r
    c.extend_from_slice(&[op::LOCAL_GET, 2]);
    Body { extra_locals: vec![Kind::Int64], code: c }
}

/// `checked_sub`: r = a-b (wrapping); overflow iff (a^b)&(a^r) < 0.
fn checked_sub_body() -> Body {
    let mut c = Vec::new();
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 1, op::I64_SUB, op::LOCAL_SET, 2]);
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 1, op::I64_XOR]);
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 2, op::I64_XOR]);
    c.push(op::I64_AND);
    c.push(op::I64_CONST);
    sleb128(0, &mut c);
    c.push(op::I64_LT_S);
    c.extend_from_slice(&[op::IF, 0x40, op::UNREACHABLE, op::END]);
    c.extend_from_slice(&[op::LOCAL_GET, 2]);
    Body { extra_locals: vec![Kind::Int64], code: c }
}

/// `checked_mul`: r = a*b; if a==0 return 0; else if (r/a)!=b trap; else return r.
/// (The a==-1,b==MIN overflow is caught by i64.div_s trapping on MIN/-1.)
fn checked_mul_body() -> Body {
    let mut c = Vec::new();
    // r (local 2) = a * b
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::LOCAL_GET, 1, op::I64_MUL, op::LOCAL_SET, 2]);
    // if a == 0 → return r (which is 0)
    c.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_EQZ, op::IF, 0x40, op::LOCAL_GET, 2, 0x0F, op::END]);
    // (r / a) != b → trap
    c.extend_from_slice(&[op::LOCAL_GET, 2, op::LOCAL_GET, 0, op::I64_DIV_S, op::LOCAL_GET, 1, op::I64_NE]);
    c.extend_from_slice(&[op::IF, 0x40, op::UNREACHABLE, op::END]);
    // return r
    c.extend_from_slice(&[op::LOCAL_GET, 2]);
    Body { extra_locals: vec![Kind::Int64], code: c }
}

// ─── AST helpers ───────────────────────────────────────────────────────────────────

fn name_of(node: Option<&Node>) -> Option<&str> {
    match node {
        Some(Node::Name(n)) => Some(n.as_str()),
        _ => None,
    }
}

/// A compile-time scalar literal, for resolving literal match patterns.
enum ScalarLit {
    Int(i64),
    Bool(bool),
}

/// The COARSE static shape a value has, used only by the type-rejection checker to decide
/// the specific rejections the corpus exercises (numeric-mismatch, nominal comparison,
/// annotation contradiction). This is NOT the language's type universe — a full type carries
/// a sum's variant set, a record's fields, and the nominal tag; those live in the type system
/// a later generation realizes. An `Ast` value is a `Sum` (the AST is an ordinary sum type),
/// so it shares that shape rather than being its own.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum StaticType {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    Bytes,
    Tuple,
    List,
    Record,
    Map,
    Sum,
}

impl StaticType {
    fn is_numeric(self) -> bool {
        matches!(self, StaticType::Int | StaticType::Float)
    }
    fn of_cval(v: &CVal) -> StaticType {
        match v {
            CVal::Int(_) => StaticType::Int,
            CVal::Bool(_) => StaticType::Bool,
            CVal::Float(_) => StaticType::Float,
            CVal::Str(_) => StaticType::Str,
            CVal::Bytes(_) => StaticType::Bytes,
            CVal::Tuple(t) if t.is_empty() => StaticType::Unit,
            CVal::Tuple(_) => StaticType::Tuple,
            CVal::List(_) => StaticType::List,
            CVal::Record(_) => StaticType::Record,
            CVal::Map(_) => StaticType::Map,
            // The AST is an ordinary sum type; a sum value and an Ast value share a shape.
            CVal::Sum { .. } | CVal::Ast(_) => StaticType::Sum,
        }
    }
    /// Does a value of this type satisfy the annotation naming type `ann`?
    fn matches_annotation(self, ann: &str) -> bool {
        match self {
            // An integer value satisfies the realized `Int64`/`Int` AND any fixed-width or bignum
            // INTEGER type name (`UInt8`, `Int16`, `(UInt 48)`, `BigInt`, …). The width family is
            // not yet realized (all such corpus cases carry `(needs numeric-model)` and SKIP), so
            // annotating an integer literal with a width is NOT a contradiction — `(: 200 UInt8)` is
            // the well-typed unsigned-8-bit value 200, not a type error. Reporting CDZ0203 there was
            // a FALSE rejection (reject-don't-miscompile the WRONG way — rejecting a valid program).
            // The per-width RANGE check (`(: 300 UInt8)` → out of range, `(UInt 65)` → CDZ0302) is
            // the deferred numeric-model work; until then the annotation is accepted conservatively
            // (never wrong-valued — an in-range literal is correct at any width; an out-of-range one
            // is a not-yet-checked overflow, a decline-grade gap, not a miscompile).
            StaticType::Int => matches!(ann, "Int64" | "Int") || is_fixed_width_int_type_name(ann),
            StaticType::Float => matches!(ann, "Float64" | "Float"),
            StaticType::Bool => ann == "Bool",
            StaticType::Str => ann == "String",
            StaticType::Unit => ann == "Unit",
            StaticType::Bytes => ann == "Bytes",
            // A COMPOUND value (tuple, list, record, map, sum) annotated with a known SCALAR type
            // name — `(: (tuple 1 2) Int64)`, `(: (Some 5) Bool)` — is a contradiction (CDZ0203): a
            // compound value is not a scalar. A compound value annotated with a compound or unknown
            // annotation is not checked here (reject-don't-miscompile — a not-yet-checked rule is
            // not a rejection). 07-type-system.sexp §"a tuple/sum value annotated as a scalar type
            // is rejected".
            StaticType::Tuple | StaticType::List | StaticType::Record | StaticType::Map
            | StaticType::Sum => !is_scalar_type_name(ann),
        }
    }
}

/// Is `ann` the name of a known SCALAR type — one a compound value can never inhabit? Used to
/// reject a compound-value-vs-scalar-annotation contradiction (CDZ0203) while leaving a compound
/// or unknown annotation unchecked. Includes the fixed-width/bignum integer family: a COMPOUND
/// value annotated with `UInt8`/`(UInt 48)`/`BigInt` is still a contradiction (a tuple is not an
/// integer of any width), so those names must count as scalar here even though the widths are not
/// yet realized as values.
fn is_scalar_type_name(ann: &str) -> bool {
    matches!(ann, "Int64" | "Int" | "Float64" | "Float" | "Bool" | "String" | "Unit" | "Bytes")
        || is_fixed_width_int_type_name(ann)
}

/// Is `head` a structural (compound) TYPE-constructor head — one whose applied form names a
/// compound type whose KIND is determined by the head (`(Record …)`, `(Tuple …)`, `(List …)`,
/// `(Map …)`, `(Option …)`, `(Result …)`)? Used to check that a compound value's kind agrees with
/// a compound annotation's head (a `(Record …)` value annotated `(Tuple …)` is a contradiction).
fn is_structural_type_head(head: &str) -> bool {
    matches!(head, "Record" | "Tuple" | "List" | "Map" | "Option" | "Result")
}

/// Is `ty` a BUILT-IN (prelude-declared) sum type that is STRUCTURAL, not a nominal boundary? The
/// polymorphic `Option`/`Result` and the metaprogramming `Ast` are declared in the prelude the same
/// way a user `(type …)` is, but — unlike a user-declared sum — they are structural/polymorphic
/// values, not nominal types (comparing across their variant sets is the ordinary shape error
/// CDZ0201, not a nominal-boundary CDZ0202). `nominal_name` excludes them so a `(Some …)`/`(Ast.Int
/// …)` value is not mistaken for a nominal record over its structural shape. `Sign` is a user-style
/// enum kept out (its variants ARE a nominal sum), so only these three built-ins are excluded.
fn is_builtin_structural_sum(ty: &str) -> bool {
    matches!(ty, "Option" | "Result" | "Ast")
}

/// Does a value's coarse `StaticType` agree with the KIND named by a structural annotation head?
/// `Record`↔Record, `Tuple`↔Tuple, `List`↔List, `Map`↔Map, and `Option`/`Result`↔Sum (a built-in
/// sum is what an Option/Result annotation constrains). A disagreement (a Record value under a
/// `Tuple` head, a tuple value under a `List` head, …) is an annotation contradiction.
fn static_type_matches_structural_head(vt: StaticType, head: &str) -> bool {
    match head {
        "Record" => vt == StaticType::Record,
        "Tuple" => vt == StaticType::Tuple,
        "List" => vt == StaticType::List,
        "Map" => vt == StaticType::Map,
        "Option" | "Result" => vt == StaticType::Sum,
        _ => true, // an unrecognized head imposes nothing
    }
}

/// Is `ann` a fixed-width or bignum INTEGER type name — `UInt8`/`Int8`/…/`UInt64`/`Int64`, the
/// `(UInt N)`/`(Int N)` width-indexed forms (passed as their head `UInt`/`Int`), or `BigInt`? These
/// are the integer types beyond the realized 64-bit `Int64` core: an integer literal annotated with
/// one is well-typed (subject to a per-width range check the seed defers, `(needs numeric-model)`),
/// NOT a contradiction. `type_name` collapses `(UInt N)` to `"UInt"`, so the bare heads suffice.
fn is_fixed_width_int_type_name(ann: &str) -> bool {
    matches!(
        ann,
        "UInt" | "Int" | "BigInt"
            | "Int8" | "Int16" | "Int32" | "Int64"
            | "UInt8" | "UInt16" | "UInt32" | "UInt64"
    )
}

/// The type name an annotation node denotes: a bare `Int64`, or the head of a parameterized
/// annotation like `(Option Int64)` / `(Tuple …)`.
fn type_name(node: &Node) -> Option<&str> {
    match node {
        Node::Name(n) => Some(n.as_str()),
        Node::List(items) => name_of(items.first()),
        _ => None,
    }
}

/// A compile-time value produced by `eval_const` over the pure constant fragment. Scalars
/// have a wasm representation (`emit_const`); compound values exist only at compile time and
/// are consumed to produce scalars/traps.
#[derive(Clone, Debug)]
enum CVal {
    Int(i64),
    Bool(bool),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Tuple(Vec<CVal>),
    List(Vec<CVal>),
    Record(Vec<(String, CVal)>), // sorted by key
    /// A map — a key→value collection with DYNAMIC keys that are VALUES (not field labels): keys are
    /// arbitrary values compared by value (collections-and-text.md §Keys Are Compared By Value), so a
    /// key is a `CVal`, not a `String` (a record's field NAME is a compile-time label — `Record` above
    /// keeps `String` keys). A DISTINCT type from a record (never compares equal, not member-
    /// projectable). Sorted by CANONICAL KEY FORM for order-independent equality and canonical
    /// rendering (§A Map Renders As Its Entries In Canonical Key Order).
    Map(Vec<(CVal, CVal)>),
    Sum { variant: String, payload: Box<CVal> },
    /// An AST value (what `quote`/`quasiquote` produce and `Ast.decode` returns), carried as
    /// the canonical node it denotes so `Ast.encode`/`Ast.decode` round-trip exactly.
    Ast(Node),
}

impl CVal {
    /// The unit value is the empty tuple (core-semantics.md).
    fn unit() -> CVal {
        CVal::Tuple(Vec::new())
    }
}

/// A definite compile-time trap: an operation on constants whose dynamic result is a trap
/// (out-of-range byte, index out of bounds). Distinct from `Decline` (which means "not
/// compiled yet") — this means "the program traps," lowered to `unreachable`.
struct ConstTrap;

/// Structural equality of two compile-time values, matching the interpreter's value equality
/// (unit is the empty tuple; records compare by sorted fields; NaN handling via canonical
/// byte form for floats).
fn cval_eq(a: &CVal, b: &CVal) -> bool {
    match (a, b) {
        (CVal::Int(x), CVal::Int(y)) => x == y,
        (CVal::Bool(x), CVal::Bool(y)) => x == y,
        (CVal::Float(x), CVal::Float(y)) => float_canonical_eq(*x, *y),
        (CVal::Str(x), CVal::Str(y)) => x == y,
        (CVal::Bytes(x), CVal::Bytes(y)) => x == y,
        (CVal::Tuple(x), CVal::Tuple(y)) | (CVal::List(x), CVal::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| cval_eq(a, b))
        }
        // A record compares to a record by its sorted STRING field keys (never to a map — that
        // cross-type comparison is a compile-time rejection; this is the dynamic fallback).
        (CVal::Record(x), CVal::Record(y)) => {
            x.len() == y.len()
                && x.iter().zip(y).all(|((k1, v1), (k2, v2))| k1 == k2 && cval_eq(v1, v2))
        }
        // A map compares to a map by its associations, independent of insertion order
        // (collections-and-text.md §A Map Associates Keys With Values). Both are stored key-sorted by
        // canonical key form, so a positional zip over the sorted entries is order-independent: equal
        // key (by value) and equal value at each position.
        (CVal::Map(x), CVal::Map(y)) => {
            x.len() == y.len()
                && x.iter().zip(y).all(|((k1, v1), (k2, v2))| cval_eq(k1, k2) && cval_eq(v1, v2))
        }
        // AST equality bridges quote and the `Ast.*` constructors. When either operand is a
        // `quote` result (`CVal::Ast`), both operands are normalized to the raw program node they
        // denote (an `Ast.*`-constructor sum via `ast_sum_to_node`) and compared by canonical byte
        // form — the encoding is a bijection with one canonical byte form (ast-encoding.md), and
        // `match` already treats `(quote 42)` and `(Ast.Int 42)` as the one AST sum value, so `=`
        // MUST agree (12-metaprogramming.sexp §"a quoted integer equals the same node built by the
        // Ast.Int constructor"). An ordinary (non-Ast) sum has no AST node form and falls through
        // to the structural sum arm below.
        _ if matches!(a, CVal::Ast(_)) || matches!(b, CVal::Ast(_)) => {
            match (cval_to_ast_node(a), cval_to_ast_node(b)) {
                (Some(na), Some(nb)) => ast::encode(&na) == ast::encode(&nb),
                _ => false,
            }
        }
        (CVal::Sum { variant: v1, payload: p1 }, CVal::Sum { variant: v2, payload: p2 }) => {
            v1 == v2 && cval_eq(p1, p2)
        }
        _ => false,
    }
}

/// Convert a `(module name def…)` node to the record of its exports: each `(def (f p…) body)`
/// becomes a field `(f (fn (p…) body))` (a nullary def → `(fn (_) body)`), so a module used
/// as a value is an ordinary record of export functions reached by member access
/// (core-semantics.md §A Module Evaluates To A Record Of Its Exports). `use`/metadata forms
/// are not exports and are omitted.
fn module_to_record(mitems: &[Node]) -> Node {
    let mut fields = vec![Node::Name("record".into())];
    // The module's capability manifest is the UNION of its entrypoints' host delegations
    // (capabilities-and-effects.md §The Program Manifest Is The Union Of Its Entrypoints'
    // Delegations) — the delegation IS the grant; declaring or performing an effect grants
    // nothing. Computed by scanning each `def`'s body for `(host (Effect…) …)` delegation forms,
    // collecting the delegated effect names in first-seen order (deduplicated). Carried as
    // METADATA alongside the exports, not itself an export.
    let mut capabilities: Vec<Node> = vec![Node::Name("list".into())];
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for form in &mitems[2..] {
        if let Node::List(d) = form {
            if name_of(d.first()) == Some("def") {
                if let Some(body) = d.last() {
                    collect_host_delegations(body, &mut capabilities, &mut seen);
                }
            }
        }
    }
    for form in &mitems[2..] {
        if let Node::List(d) = form {
            if name_of(d.first()) == Some("def") {
                match d.get(1) {
                    // A FUNCTION definition `(def (f p…) body)` — signature is a `(name params…)`
                    // list. Register `f` as a field bound to the lambda `(fn (p…) body)` (a nullary
                    // def `(def (f) body)` takes unit).
                    Some(Node::List(sig)) => {
                        if let Some(Node::Name(fname)) = sig.first() {
                            let params: Vec<Node> = if sig.len() > 1 {
                                sig[1..].to_vec()
                            } else {
                                vec![Node::Name("_".into())] // nullary → takes unit
                            };
                            let body = d.last().cloned().unwrap_or(Node::Name("unit".into()));
                            let lambda = Node::List(vec![
                                Node::Name("fn".into()),
                                Node::List(params),
                                body,
                            ]);
                            fields.push(Node::List(vec![Node::Name(fname.clone()), lambda]));
                        }
                    }
                    // A VALUE definition `(def name value)` — signature is a bare NAME. Register
                    // `name` as a field bound DIRECTLY to its value node (no lambda wrap): a module
                    // Definition is "a value, function, type" (glossary), and each MUST register its
                    // name and value as a module-record field (core-semantics.md §A Module Evaluates
                    // To A Record Of Its Exports). So `(. m v)` PROJECTS the value (no `unit`
                    // application), matching how a `do`-scoped `(def x 5)` binds `x` to its value.
                    // Was dropped (only the function shape was collected), so `(. m v)` trapped on a
                    // missing field — a decline-don't-miscompile violation (11-modules.sexp §"a module
                    // value definition registers a reachable export field").
                    Some(Node::Name(vname)) => {
                        if let Some(value) = d.get(2) {
                            fields.push(Node::List(vec![Node::Name(vname.clone()), value.clone()]));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Carry the capability manifest under a RESERVED key that no export can spell. Metadata is
    // reached by `(meta KEY)`, which member access maps to `META_KEY_PREFIX+KEY` (see
    // `gen_member`); an export field is an identifier and cannot contain the prefix's separator,
    // so the two channels never collide — an export and a like-named metadata key are distinct
    // (11-modules.sexp §"an export and a like-named metadata key do not collide").
    fields.push(Node::List(vec![
        Node::Name(format!("{META_KEY_PREFIX}capabilities")),
        Node::List(capabilities),
    ]));
    Node::List(fields)
}

/// Collect every effect a `(host (Effect…) …)` delegation names within `node`, pushing each
/// first-seen effect name (as a `Str`) onto `caps` and recording it in `seen` for deduplication.
/// The module's capability manifest is the union of these across its entrypoints' bodies
/// (capabilities-and-effects.md §The Program Manifest Is The Union Of Its Entrypoints' Delegations).
fn collect_host_delegations(
    node: &Node,
    caps: &mut Vec<Node>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    if let Node::List(items) = node {
        if name_of(items.first()) == Some("host") {
            if let Some(Node::List(effs)) = items.get(1) {
                for e in effs {
                    if let Node::Name(name) = e {
                        if seen.insert(name.clone()) {
                            caps.push(Node::Str(name.clone()));
                        }
                    }
                }
            }
        }
        for child in items {
            collect_host_delegations(child, caps, seen);
        }
    }
}

/// Prefix of the reserved record key a `(meta KEY)` metadata access maps to. Contains a
/// character no identifier export name can, so a metadata key never collides with an export.
const META_KEY_PREFIX: &str = "@meta:";

/// The reserved metadata record key a member field `(meta KEY)` denotes, or None for an
/// ordinary field. `(. m (meta capabilities))` reads the module's manifest, distinct from the
/// export `(. m capabilities)`.
fn meta_field_key(field: &Node) -> Option<String> {
    if let Node::List(items) = field {
        if name_of(items.first()) == Some("meta") {
            if let Some(Node::Name(key)) = items.get(1) {
                return Some(format!("{META_KEY_PREFIX}{key}"));
            }
        }
    }
    None
}

/// Do two arithmetic operand kinds definitely mismatch (one Int64, one Float64)? Mixing
/// numeric types has no defined result in the dynamic semantics — it traps.
fn arith_type_mismatch(a: Kind, b: Kind) -> bool {
    matches!(
        (a, b),
        (Kind::Int64, Kind::Float64) | (Kind::Float64, Kind::Int64)
    )
}


/// Fold a binary integer op over two constants, matching the emitted wasm semantics: `+`/`-`/`*`
/// trap on signed overflow; `/`/`%` trap on divide-by-zero and MIN/-1; bitwise/shift wrap.
fn fold_int_op(op: &str, a: i64, b: i64) -> Result<i64, ConstTrap> {
    let r = match op {
        "+" => a.checked_add(b).ok_or(ConstTrap)?,
        "-" => a.checked_sub(b).ok_or(ConstTrap)?,
        "*" => a.checked_mul(b).ok_or(ConstTrap)?,
        "/" => a.checked_div(b).ok_or(ConstTrap)?,
        // `x % -1` is always 0 and never overflows — it forms no out-of-range quotient. Rust's
        // `checked_rem` conservatively returns None for `INT_MIN % -1` (mirroring division's
        // overflow), but the modulo does not overflow, so special-case it to 0 rather than trap.
        // This matches wasm's i64.rem_s (the runtime path), so the const and runtime paths agree
        // (06-numeric-model.sexp §"modulo by -1 is zero even at the minimum integer").
        "%" if b == -1 => 0,
        "%" => a.checked_rem(b).ok_or(ConstTrap)?,
        "&" => a & b,
        "|" => a | b,
        "^" => a ^ b,
        // Shifts obey #Overflow Is Defined: a shift count outside the type's bit width has no
        // defined value (wasm's shl/shr mask it mod 64 — a shift-by-64 becomes 0, a negative
        // count becomes 63 — undefined behavior the compiler must not emit), so an out-of-range
        // count traps. A left shift is exact multiplication by 2^count, so it overflows exactly
        // when that multiplication does — trapping like the checked `*` default.
        "<<" => {
            if !(0..64).contains(&b) {
                return Err(ConstTrap);
            }
            // Compute the exact value a·2^b in i128 (which cannot overflow — |a·2^b| < 2^127 for
            // b < 64) and trap if it falls outside the Int64 range. This is faithful to "a left
            // shift is exact multiplication by a power of two" even when 2^b is itself 2^63.
            let r = (a as i128) << (b as u32);
            if r < i64::MIN as i128 || r > i64::MAX as i128 {
                return Err(ConstTrap);
            }
            r as i64
        }
        ">>" => {
            if !(0..64).contains(&b) {
                return Err(ConstTrap);
            }
            a >> b
        }
        _ => return Err(ConstTrap),
    };
    Ok(r)
}

/// Convert a compile-time value to the program NODE that denotes it — used to embed an
/// unquoted value into a quasiquote (`,x` with x=2 → the node `2`). Scalars map to literals;
/// an Ast value is already a node.
fn cval_to_node(v: &CVal) -> Option<Node> {
    Some(match v {
        CVal::Int(n) => Node::Int(*n),
        CVal::Bool(b) => Node::Bool(*b),
        CVal::Float(f) => Node::Float(*f),
        CVal::Str(s) => Node::Str(s.clone()),
        // An AST VALUE's canonical program form is `(quote <node>)`, NOT the bare node — a bare
        // `Node::Int(7)` re-folds to `CVal::Int(7)`, losing that this was an `Ast` value (so a
        // decoded `(Ok a)` binder `a` would compare Int-vs-Ast). Wrapping in `quote` re-folds to
        // `CVal::Ast` exactly, so an Ast payload survives the CVal→Node→CVal round-trip that
        // `resolve`/`try_match`/`let`-memoization perform. (The RENDER path reads `CVal::Ast`
        // directly and is unaffected; this node is only for structural re-matching/re-folding.)
        CVal::Ast(n) => Node::List(vec![Node::Name("quote".into()), n.clone()]),
        CVal::List(elems) => {
            let mut items = vec![Node::Name("list".into())];
            for e in elems {
                items.push(cval_to_node(e)?);
            }
            Node::List(items)
        }
        CVal::Tuple(elems) => {
            let mut items = vec![Node::Name("tuple".into())];
            for e in elems {
                items.push(cval_to_node(e)?);
            }
            Node::List(items)
        }
        // A sum value `(Ctor payload)`. A qualified variant `Sign.Pos` becomes the canonical
        // member head `(. Sign Pos)` so `constructor_of` recovers the variant tag; a bare
        // `Some`/`Ok` stays a bare name. Reproduces the node a `match`/`try_match` walks.
        CVal::Sum { variant, payload } => {
            let head = match variant.split_once('.') {
                Some((ty, v)) => Node::List(vec![
                    Node::Name(".".into()),
                    Node::Name(ty.to_string()),
                    Node::Name(v.to_string()),
                ]),
                None => Node::Name(variant.clone()),
            };
            Node::List(vec![head, cval_to_node(payload)?])
        }
        // A folded byte buffer round-trips as `(Bytes.of (list b…))` — the literal form
        // `eval_const_dotted` folds straight back to `CVal::Bytes`. This lets a `let`-bound Bytes
        // value be memoized as a constant node (see the `let` arm of `eval_const`) so it is not
        // re-folded on every reference.
        CVal::Bytes(bytes) => {
            let mut list = vec![Node::Name("list".into())];
            for b in bytes {
                list.push(Node::Int(*b as i64));
            }
            Node::List(vec![
                Node::List(vec![
                    Node::Name(".".into()),
                    Node::Name("Bytes".into()),
                    Node::Name("of".into()),
                ]),
                Node::List(list),
            ])
        }
        _ => return None,
    })
}

/// The bare variant tag from a possibly-qualified variant name (`Sign.Pos` → `Pos`, `Some` →
/// `Some`), used to look a variant up in the declared sum-type map.
fn variant_tag(variant: &str) -> &str {
    variant.rsplit('.').next().unwrap_or(variant)
}

/// Convert an `Ast.*`-constructor sum value to the raw program NODE it denotes, so that an AST
/// built with `Ast.Int`/`Ast.Name`/`Ast.List`/… is recognized as the SAME value `quote` produces
/// (which the compiler stores as `CVal::Ast(node)`). This is the inverse of the quote→constructor
/// bridge `match` uses (`quote_to_ast`), so that `=` agrees with `match` on the one AST value form
/// (12-metaprogramming.sexp §"a quoted integer equals the same node built by the Ast.Int
/// constructor"). Returns None for a sum that is not an `Ast.*` constructor (an ordinary sum is
/// not an AST value and keeps the ordinary structural-equality path).
fn ast_sum_to_node(variant: &str, payload: &CVal) -> Option<Node> {
    let (ty, kind) = variant.split_once('.')?;
    if ty != "Ast" {
        return None;
    }
    Some(match (kind, payload) {
        ("Int", CVal::Int(n)) => Node::Int(*n),
        ("Float", CVal::Float(f)) => Node::Float(*f),
        ("Str", CVal::Str(s)) => Node::Str(s.clone()),
        ("Bool", CVal::Bool(b)) => Node::Bool(*b),
        // An `Ast.Name` carries the name as a String payload (quote_to_ast: a bare name quotes to
        // `(Ast.Name "n")`), so its node is `Node::Name(payload-string)`.
        ("Name", CVal::Str(s)) => Node::Name(s.clone()),
        // An `Ast.List` carries `(list child…)`; each child is itself an `Ast.*` value denoting a
        // child node.
        ("List", CVal::List(children)) => {
            let mut items = Vec::with_capacity(children.len());
            for c in children {
                items.push(cval_to_ast_node(c)?);
            }
            Node::List(items)
        }
        _ => return None,
    })
}

/// Normalize a compile-time value that denotes an AST to the raw program NODE it denotes: a
/// `quote` result (`CVal::Ast`) is already the node; an `Ast.*`-constructor value (`CVal::Sum`) is
/// bridged by `ast_sum_to_node`. Returns None if the value does not denote an AST.
fn cval_to_ast_node(v: &CVal) -> Option<Node> {
    match v {
        CVal::Ast(n) => Some(n.clone()),
        CVal::Sum { variant, payload } => ast_sum_to_node(variant, payload),
        _ => None,
    }
}

// ─── Effect declarations (routing-agnostic contracts) ─────────────────────────────────
//
// `(effect Name (op op-name (-> T… R)) …)` declares an effect and types its operations
// (capabilities-and-effects.md §An Effect Declaration Names The Effect And Types Its Operations).
// It is a ROUTING-AGNOSTIC contract — it says NOTHING about where the effect is discharged; the
// SAME declared effect may be handled in-program by a `(handle …)` or delegated to the host by an
// entrypoint `(host …)` (§Host-Binding Is A Routing Decision Made At The Entrypoint). There is NO
// `(host)` marker and NO separate import form; the declaration's operation signatures are used when
// an entrypoint delegates the effect, to emit the host imports verbatim.

/// One operation of an effect: its declared parameter kinds and result kind, read from the
/// `(-> T… R)` type. The last type in the arrow is the result; the rest are parameters.
#[derive(Clone, Debug)]
struct EffectOp {
    name: String,
    params: Vec<Kind>,
    result: Kind,
    /// The operation's declared PARAMETER type nodes (all but the last type in `(-> T… R)`), kept
    /// alongside the coarse `params` Kinds so a perform's arguments can be type-checked against the
    /// full declared types — a String or compound (`(List Int64)`) parameter, which the coarse Kind
    /// (`Heap`) cannot distinguish. Parallel to `result_type`.
    param_types: Vec<Node>,
    /// The operation's declared RESULT type node (the last type in the `(-> T… R)`), kept alongside
    /// the coarse `result` Kind so a compound result — `(List Int64)`, a tuple — can be turned into
    /// a render `Shape` (the coarse Kind loses the element/field types). Used by `shape_of` to give
    /// a perform's runtime value its shape (e.g. a `Diag.collect : Unit -> (List Int64)` read-out),
    /// AND to type-check a resume value against a compound result type.
    result_type: Node,
}

/// A declared effect: its name and its closed set of operations. The set is closed
/// (capabilities-and-effects.md §A Handler Arm Names An Operation Its Effect Declares), so a
/// handler arm naming an operation not here is CDZ0403.
#[derive(Clone, Debug)]
struct EffectDecl {
    ops: Vec<EffectOp>,
}

impl EffectDecl {
    fn op(&self, name: &str) -> Option<&EffectOp> {
        self.ops.iter().find(|o| o.name == name)
    }
}

/// Parse an operation-type node `(-> T… R)` into `(param-kinds, result-kind, param-type-nodes,
/// result-type-node)`, or None if it is malformed. A `(-> R)` (no params) is a nullary operation.
/// The effect's operation kinds reuse the host-boundary `Kind` mapping (`host_type_kind`): the
/// operation-declared types are exactly the WIT-typed signature a delegated operation imports
/// verbatim. A type the mapping does not recognize (a user compound, a list) maps to `Kind::Heap`
/// so the surface still parses — a delegated op with such a type would decline at the boundary, an
/// intra-program handler does not consult it. `Unit` (`unit`/`Unit`) is a valid operation type (a
/// Unit-returning `emit`). The full param TYPE NODES are returned alongside the coarse kinds so a
/// perform's args can be checked against a String/compound param the coarse `Heap` kind can't name.
fn parse_op_type(node: &Node) -> Option<(Vec<Kind>, Kind, Vec<Node>, Node)> {
    let items = match node {
        Node::List(items) if name_of(items.first()) == Some("->") => items,
        _ => return None,
    };
    let types = &items[1..];
    if types.is_empty() {
        return None;
    }
    let kind_of = |n: &Node| host_type_kind(n).unwrap_or(Kind::Heap);
    let result_node = types.last().unwrap().clone();
    let result = kind_of(&result_node);
    let param_nodes: Vec<Node> = types[..types.len() - 1].to_vec();
    let params = param_nodes.iter().map(|n| kind_of(n)).collect();
    Some((params, result, param_nodes, result_node))
}

/// Parse every top-level `(effect Name (op op-name (-> T… R)) …)` form into an
/// effect → declaration table. A malformed effect form is skipped (its later use — a perform or a
/// handler arm — declines or rejects on the missing entry), never a panic. Effect declarations are
/// top-level module forms (like `def`/`type`), so this scans the module's forms directly.
fn collect_effects(forms: &[Node]) -> std::collections::BTreeMap<String, EffectDecl> {
    let mut out = std::collections::BTreeMap::new();
    for form in forms {
        let items = match form {
            Node::List(items) => items,
            _ => continue,
        };
        if name_of(items.first()) != Some("effect") {
            continue;
        }
        let ename = match name_of(items.get(1)) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let mut ops = Vec::new();
        for op_form in &items[2..] {
            // Each operation is `(op op-name (-> T… R))`.
            if let Node::List(op_items) = op_form {
                if name_of(op_items.first()) != Some("op") {
                    continue;
                }
                let op_name = match name_of(op_items.get(1)) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if let Some(ty) = op_items.get(2) {
                    if let Some((params, result, param_types, result_type)) = parse_op_type(ty) {
                        ops.push(EffectOp { name: op_name, params, result, param_types, result_type });
                    }
                }
            }
        }
        out.insert(ename, EffectDecl { ops });
    }
    out
}

/// Collect every effect named by a `(host (Effect…) …)` delegation within `node`, in first-seen
/// order, into `out`. Used to compute the program manifest (the union of the entrypoint's
/// delegations).
fn collect_delegated_effects(node: &Node, out: &mut Vec<String>) {
    if let Node::List(items) = node {
        if name_of(items.first()) == Some("host") {
            if let Some(Node::List(effs)) = items.get(1) {
                for e in effs {
                    if let Node::Name(name) = e {
                        if !out.contains(name) {
                            out.push(name.clone());
                        }
                    }
                }
            }
        }
        for child in items {
            collect_delegated_effects(child, out);
        }
    }
}

/// Pull `(effect, op)` from a perform/arm head. The head `E.op` reads (reader dotted-name sugar)
/// as the member-access tree `(. E op)`, so a head node `(. E op)` yields `("E", "op")`. A bare
/// name or other shape is not a perform head.
fn effect_op_of_head(head: &Node) -> Option<(String, String)> {
    if let Node::List(items) = head {
        if name_of(items.first()) == Some(".") {
            if let (Some(e), Some(o)) = (name_of(items.get(1)), name_of(items.get(2))) {
                return Some((e.to_string(), o.to_string()));
            }
        }
    }
    None
}

/// Encode a state `Kind` as a small integer for the compiler-internal `(@state-local N tag)`
/// accessor node (the handler frame is off the router stack during arm emission, so the accessor
/// carries its own kind). Only the scalar/heap kinds a handler state can take are encoded.
fn state_kind_tag(k: Kind) -> i64 {
    match k {
        Kind::Int64 => 0,
        Kind::Bool => 1,
        Kind::Float64 => 2,
        Kind::Heap => 3,
        _ => 0,
    }
}

/// Inverse of `state_kind_tag`.
fn state_kind_untag(tag: i64) -> Kind {
    match tag {
        1 => Kind::Bool,
        2 => Kind::Float64,
        3 => Kind::Heap,
        _ => Kind::Int64,
    }
}

/// Classify a handler arm by how its body uses `resume` (options/effects-model/lowering-to-wasm.md
/// §Handler classification). Conservative: `Tail` requires `resume` exactly once in TAIL position
/// of every control path; no `resume` is `Abortive`; anything else is `GeneralOneShot`. The count
/// treats `resume` as bound only within this arm — it does NOT descend into a nested `(fn …)` or
/// nested `(handle …)`.
fn classify_arm(body: &Node) -> ArmClass {
    let count = count_resume(body);
    if count == 0 {
        return ArmClass::Abortive;
    }
    if count == 1 && resume_in_tail(body) {
        return ArmClass::Tail;
    }
    ArmClass::GeneralOneShot
}

/// Count `(resume …)` occurrences in `node`, NOT descending into a nested `(fn …)` (its own
/// resume scope) or nested `(handle …)` (a fresh handler binds a fresh continuation).
fn count_resume(node: &Node) -> usize {
    match node {
        Node::List(items) => {
            match name_of(items.first()) {
                Some("resume") => 1, // this node is a resume; its args cannot themselves resume in a well-formed arm
                // Do not descend into a nested lambda or nested handle.
                Some("fn") | Some("handle") => 0,
                _ => items.iter().map(count_resume).sum(),
            }
        }
        _ => 0,
    }
}

/// Is the single `resume` in `body` in TAIL position of every control path? Tail position is the
/// arm body itself, the taken branches of a tail `if`, each arm body of a tail `match`, and the
/// last form of a tail `do`/`let`. Mirrors the recursive tail-position definition the emitter uses.
fn resume_in_tail(node: &Node) -> bool {
    match node {
        Node::List(items) => match name_of(items.first()) {
            Some("resume") => true,
            Some("if") if items.len() == 4 => {
                // The condition is NOT tail; both branches are.
                resume_in_tail(&items[2]) && resume_in_tail(&items[3])
            }
            Some("do") if items.len() >= 2 => resume_in_tail(items.last().unwrap()),
            Some("let") if items.len() >= 3 => resume_in_tail(items.last().unwrap()),
            Some("match") => {
                // Each arm is `(pattern body)`; the body is tail. An arm with no resume in tail
                // fails the whole-path requirement.
                items[2..].iter().all(|arm| match arm {
                    Node::List(a) if a.len() == 2 => resume_in_tail(&a[1]),
                    _ => false,
                })
            }
            // Any other head is a leaf w.r.t. tail position: the resume, if present here, is an
            // ARGUMENT (a non-tail sub-expression like `(+ 1 (resume …))`), which is not tail.
            // But a FORWARDING arm `(resume (E.op …) s)` has the resume itself in tail, its
            // argument merely effectful — that is handled by the `Some("resume")` arm above (this
            // node's head is `resume`), so reaching here means resume is nested in a non-control
            // form and is not tail.
            _ => false,
        },
        _ => false,
    }
}

/// Visit the VALUE operand of every TAIL `(resume value next-state)` in a handler-arm body, calling
/// `f` on each. Walks the same tail positions as `unwrap_tail_resume` (`if`/`do`/`let`/`match`
/// tails), so a resume in each control-flow branch is visited. Used to type-check the resumed value
/// against the operation's declared result type (the resume value is what the op yields).
fn for_each_tail_resume_value(node: &Node, f: &mut impl FnMut(&Node)) {
    if let Node::List(items) = node {
        match name_of(items.first()) {
            Some("resume") if items.len() >= 2 => f(&items[1]),
            Some("if") if items.len() == 4 => {
                for_each_tail_resume_value(&items[2], f);
                for_each_tail_resume_value(&items[3], f);
            }
            Some("do") | Some("let") if items.len() >= 2 => {
                for_each_tail_resume_value(items.last().unwrap(), f);
            }
            Some("match") if items.len() >= 2 => {
                for arm in &items[2..] {
                    if let Node::List(a) = arm {
                        if a.len() == 2 {
                            for_each_tail_resume_value(&a[1], f);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Visit the STATE operand (the second argument) of every TAIL `(resume value next-state)` in a
/// handler-arm body, calling `f` on each. The state twin of `for_each_tail_resume_value` — same tail
/// walk (`if`/`do`/`let`/`match`), same positions. Used to scope-check the resume STATE for an unbound
/// name (the state is an ordinary expression subject to lexical scope, but — for a Unit-state arm — it
/// is unwrapped away and never emitted, so its scope errors are not caught downstream and must be
/// checked here). A resume with fewer than 3 items has no explicit state (the `s`-unchanged default),
/// so nothing to visit.
fn for_each_tail_resume_state(node: &Node, f: &mut impl FnMut(&Node)) {
    if let Node::List(items) = node {
        match name_of(items.first()) {
            Some("resume") if items.len() >= 3 => f(&items[2]),
            Some("if") if items.len() == 4 => {
                for_each_tail_resume_state(&items[2], f);
                for_each_tail_resume_state(&items[3], f);
            }
            Some("do") | Some("let") if items.len() >= 2 => {
                for_each_tail_resume_state(items.last().unwrap(), f);
            }
            Some("match") if items.len() >= 2 => {
                for arm in &items[2..] {
                    if let Node::List(a) = arm {
                        if a.len() == 2 {
                            for_each_tail_resume_state(&a[1], f);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Rewrite an arm body's tail `(resume value next-state)` into just `value` (the whole Tier-1
/// lowering: `resume` is not a call, it is the "return this value to the perform site" marker).
/// Recurses into the tail of `if`/`do`/`let`/`match` the same way tail position is computed, so a
/// resume in each tail branch is unwrapped. `next-state` is the second argument — its threading is
/// Stage 1's state fold; for the unit-state cases it is `s` unchanged (no bytes).
fn unwrap_tail_resume(node: &Node) -> Node {
    match node {
        Node::List(items) => match name_of(items.first()) {
            Some("resume") if items.len() >= 2 => items[1].clone(),
            Some("if") if items.len() == 4 => Node::List(vec![
                items[0].clone(),
                items[1].clone(),
                unwrap_tail_resume(&items[2]),
                unwrap_tail_resume(&items[3]),
            ]),
            Some("do") if items.len() >= 2 => {
                let mut out = items[..items.len() - 1].to_vec();
                out.push(unwrap_tail_resume(items.last().unwrap()));
                Node::List(out)
            }
            Some("let") if items.len() >= 3 => {
                let mut out = items[..items.len() - 1].to_vec();
                out.push(unwrap_tail_resume(items.last().unwrap()));
                Node::List(out)
            }
            Some("match") if items.len() >= 2 => {
                let mut out = items[..2].to_vec();
                for arm in &items[2..] {
                    match arm {
                        Node::List(a) if a.len() == 2 => {
                            out.push(Node::List(vec![a[0].clone(), unwrap_tail_resume(&a[1])]));
                        }
                        other => out.push(other.clone()),
                    }
                }
                Node::List(out)
            }
            _ => node.clone(),
        },
        _ => node.clone(),
    }
}

/// Scan `(type Name (V1 payload | V2 | …))` declarations, recording each variant tag → its
/// declared sum-type name. This is how the compiler learns that `Some`/`None` belong to
/// `Option` etc. — from a declaration (prelude or program), not from hardcoded names.
/// The first `(type-name, variant)` where a single `(type …)` declaration names the SAME variant
/// twice — `(type T (A Int64 | A Bool))` declares `A` twice — or None. A sum's variant names are a
/// SET (type-system.md #The Structural Types Are Record, Tuple, And Sum: a sum's shape is its variant
/// names with their payload types), so a duplicate makes the set ill-defined (CDZ0201), the fourth
/// closed name-set duplicate beside record fields, module definitions, and effect operations. The
/// duplicate is checked WITHIN one declaration — two DIFFERENT types sharing a variant name is a
/// separate (allowed, last-writer-wins) situation the reuse-override case exercises. Recurses into
/// non-quoted subtrees so a nested `(do (type …) …)` declaration is checked too, mirroring
/// `collect_sum_types`' walk and its `|`-segment split.
fn first_duplicate_variant_in_a_sum(forms: &[Node]) -> Option<(String, String)> {
    for form in forms {
        let items = match form {
            Node::List(items) => items,
            _ => continue,
        };
        if !matches!(name_of(items.first()), Some("quote") | Some("quasiquote")) {
            if let Some(dup) = first_duplicate_variant_in_a_sum(&items[..]) {
                return Some(dup);
            }
        }
        if name_of(items.first()) != Some("type") {
            continue;
        }
        let type_name = match name_of(items.get(1)) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Some(Node::List(body)) = items.get(2) {
            let mut seen: Vec<String> = Vec::new();
            for segment in body.split(|n| matches!(n, Node::Name(s) if s == "|")) {
                let tag = match segment.first() {
                    Some(Node::Name(v)) => v.clone(),
                    Some(Node::List(v)) => match name_of(v.first()) {
                        Some(h) => h.to_string(),
                        None => continue,
                    },
                    _ => continue,
                };
                if seen.contains(&tag) {
                    return Some((type_name, tag));
                }
                seen.push(tag);
            }
        }
    }
    None
}

fn collect_sum_types(
    forms: &[Node],
    out: &mut std::collections::BTreeMap<String, String>,
    nullary: &mut std::collections::BTreeSet<String>,
    variants: &mut std::collections::BTreeMap<String, Vec<String>>,
    payload_kinds: &mut std::collections::BTreeMap<String, Vec<Kind>>,
    payload_types: &mut std::collections::BTreeMap<String, Vec<Node>>,
) {
    for form in forms {
        let items = match form {
            Node::List(items) => items,
            _ => continue,
        };
        // A `(type …)` declaration is scoped to the forms that FOLLOW it in a sequencing block
        // (core-semantics.md §A Declaration In A Sequencing Block Is Scoped To The Forms That
        // Follow It), so it appears not only at a module's top level but NESTED inside a `def`
        // body's `do`/`let`. Recurse into every non-quoted subtree so a program-declared sum
        // (`(do (type Color (Red | Green | Blue)) …)`) registers its variants as constructors,
        // exactly as a top-level or prelude `(type …)` does. A `quote`/`quasiquote` body is
        // QUOTED DATA — a `(type …)` inside it is an AST value, not a declaration — so it is not
        // descended into (mirroring `check_tree`).
        if !matches!(name_of(items.first()), Some("quote") | Some("quasiquote")) {
            collect_sum_types(&items[..], out, nullary, variants, payload_kinds, payload_types);
        }
        if name_of(items.first()) != Some("type") {
            continue;
        }
        let type_name = match name_of(items.get(1)) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // The body `(V1 payload | V2 | …)` reads as ONE FLAT list of tokens with `|` as a bare-
        // name separator: `(Some a | None)` → `[Some, a, |, None]`, NOT `[(Some a), |, None]`.
        // Split it into `|`-separated SEGMENTS; each segment is one variant, its HEAD is the
        // variant tag, and it is NULLARY iff the segment is a single token (a payload token like
        // `a` in `Some a` means unary). This is why a type PARAMETER (`a`, `e`) must never be
        // recorded as a variant — it is a payload token inside a segment, not a segment head.
        if let Some(Node::List(body)) = items.get(2) {
            // The variant list in DECLARATION order (a variant's discriminant is its index here).
            let order = variants.entry(type_name.clone()).or_default();
            for segment in body.split(|n| matches!(n, Node::Name(s) if s == "|")) {
                let tag = match segment.first() {
                    Some(Node::Name(v)) => v.clone(),
                    // A parenthesized variant `(V payload)` (should it ever appear) — take its head.
                    Some(Node::List(v)) => match name_of(v.first()) {
                        Some(h) => h.to_string(),
                        None => continue,
                    },
                    _ => continue,
                };
                // A single-token segment is a nullary variant (argument type Unit); a segment with a
                // payload token (`Neg Expr`) is unary. LAST-WRITER-WINS, matching `payload_kinds` and
                // `sum_types` below: a program's `(type …)` reusing a prelude variant NAME overrides
                // the prelude's arity — a user `(type Expr (Lit Int64 | Neg Expr))` whose `Neg` shadows
                // the prelude `(type Sign (Neg | Zero | Pos))`'s NULLARY `Neg` must make `Neg` UNARY,
                // so the set must both ADD (nullary) and REMOVE (a now-unary tag), never only add —
                // else the stale prelude nullary entry misfires the CDZ0201 nullary-payload check on
                // the user's `(Expr.Neg x)`. (Variant tags are a global namespace here; per-type
                // disambiguation is the deeper fix, but arity is the property this check needs.)
                if segment.len() == 1 && matches!(segment.first(), Some(Node::Name(_))) {
                    nullary.insert(tag.clone());
                } else {
                    nullary.remove(&tag);
                }
                if !order.contains(&tag) {
                    order.push(tag.clone());
                }
                // Record the payload's tuple-slot kinds so a runtime match knows which slots to
                // unbox. The tokens after the tag are the payload type: `(Cons (Tuple Int64
                // IntList))` → segment `[Cons, (Tuple Int64 IntList)]`, payload `[(Tuple …)]`.
                // Use `.insert` (not `.or_insert_with`) so a program's `(type …)` OVERRIDES the
                // prelude on a reused tag — the same last-writer-wins `out.insert` uses below. A
                // program reusing a prelude variant name (`(type Res (Ok Int64 | Bad))` shadowing
                // Result's `Ok a`) must bind by ITS declared payload (`Int64`), not the prelude's
                // type parameter (which maps to an opaque Heap handle and mis-binds the scalar).
                payload_kinds.insert(tag.clone(), payload_slot_kinds(&segment[1..]));
                payload_types.insert(tag.clone(), payload_slot_types(&segment[1..]));
                out.insert(tag, type_name.clone());
            }
        }
    }
}

/// The SCALAR kinds of a sum variant's payload slots, for a runtime match to decide which slots to
/// unbox. A tuple payload `(Tuple T1 … Tn)` yields the tuple element kinds `[k1 … kn]` (a
/// `(tuple b1 … bn)` pattern binds each element from `arr-get(payload, i)`); a single non-tuple
/// payload type `T` yields one slot `[k(T)]` (a bare binder binds the payload directly); a nullary
/// variant yields `[]`. A type this does not recognize as a scalar (a user sum type, a type
/// parameter, a nested tuple) maps to `Kind::Heap` — an opaque handle bound without unboxing.
fn payload_slot_kinds(payload: &[Node]) -> Vec<Kind> {
    match payload {
        [] => Vec::new(),
        [one] => match one {
            Node::List(items) if name_of(items.first()) == Some("Tuple") => {
                items[1..].iter().map(type_node_to_kind).collect()
            }
            other => vec![type_node_to_kind(other)],
        },
        // A multi-token payload without a `Tuple` wrapper — treat each token as one slot.
        many => many.iter().map(type_node_to_kind).collect(),
    }
}

/// A per-variant payload-KIND map derived from a runtime value's inferred `Shape` — used to refine
/// a runtime sum match's payload binder to a CONCRETE kind when the declared type is polymorphic
/// (Option's `Some a` → opaque `Heap`, but a `(Bytes.at …)` value's `Some` payload is a `Int` byte).
/// Only a `Shape::Sum` yields entries; each variant maps to its payload's slot kinds — a `Tuple`
/// payload's element kinds, or a single-slot `[kind]` for a scalar/other payload, or `[]` for `Unit`.
/// A payload shape that is not a scalar leaf (a nested sum/list/record) maps its slot to `Heap`, so
/// the binder keeps the opaque handle (unchanged behavior). Non-Sum shapes → empty map.
fn shape_variant_payload_kinds(shape: &Shape) -> std::collections::BTreeMap<String, Vec<Kind>> {
    let mut out = std::collections::BTreeMap::new();
    if let Shape::Sum(variants) = shape {
        for (name, payload) in variants {
            let kinds = match payload {
                Shape::Unit => Vec::new(),
                Shape::Tuple(elems) => elems.iter().map(shape_leaf_kind).collect(),
                other => vec![shape_leaf_kind(other)],
            };
            // Key by the BARE variant tag (the match binds by `variant_tag`), matching how
            // `sum_payload_kinds` is keyed.
            out.insert(variant_tag(name).to_string(), kinds);
        }
    }
    out
}

/// A per-variant payload-SHAPE map derived from a runtime value's inferred `Shape` — the
/// shape-preserving companion of `shape_variant_payload_kinds`. Each variant maps to its payload's
/// `Shape` (a `Some`-payload `Record`, a `Cons`-payload `Tuple`, …), so a runtime sum match that
/// binds the payload to a bare name can attach that shape to the binder — letting a later
/// projection/access (`(. bound field)`, `(tuple.N bound)`) see through the opaque `Heap` handle.
/// Only a `Shape::Sum` yields entries; keyed by the bare variant tag. Non-Sum shapes → empty map.
fn shape_variant_payload_shapes(shape: &Shape) -> std::collections::BTreeMap<String, Shape> {
    let mut out = std::collections::BTreeMap::new();
    if let Shape::Sum(variants) = shape {
        for (name, payload) in variants {
            out.insert(variant_tag(name).to_string(), payload.clone());
        }
    }
    out
}

/// The scalar `Kind` a leaf `Shape` unboxes to (`Int`/`Bool`/`Float` → the scalar; anything
/// compound or non-scalar → `Heap`, an opaque handle bound without unboxing).
fn shape_leaf_kind(shape: &Shape) -> Kind {
    match shape {
        Shape::Int => Kind::Int64,
        Shape::Bool => Kind::Bool,
        Shape::Float => Kind::Float64,
        Shape::Unit => Kind::Unit,
        _ => Kind::Heap,
    }
}

/// The per-slot TYPE NODES of a sum variant's payload — the structure-preserving companion of
/// `payload_slot_kinds` (same slots, full type nodes instead of flattened kinds). A tuple payload
/// `(Tuple T1 … Tn)` yields `[T1 … Tn]` (each element type, so a nested `(Tuple …)` slot keeps its
/// node for `bind_sum_payload` to recurse into); a single non-tuple payload `T` yields `[T]`; a
/// nullary variant yields `[]`. Mirrors `payload_slot_kinds` slot-for-slot.
fn payload_slot_types(payload: &[Node]) -> Vec<Node> {
    match payload {
        [] => Vec::new(),
        [one] => match one {
            Node::List(items) if name_of(items.first()) == Some("Tuple") => items[1..].to_vec(),
            other => vec![other.clone()],
        },
        many => many.to_vec(),
    }
}

/// Map a payload TYPE node to the scalar `Kind` its runtime value unboxes to, or `Kind::Heap` for
/// anything not a recognized scalar (a user sum/product type, a type parameter, a nested tuple).
fn type_node_to_kind(n: &Node) -> Kind {
    match name_of(Some(n)) {
        Some("Int64") | Some("Int") => Kind::Int64,
        Some("Bool") => Kind::Bool,
        Some("Float64") | Some("Float") => Kind::Float64,
        _ => Kind::Heap,
    }
}

/// The render `Shape` of a declared TYPE node — a scalar (`Int64`, `Bool`, `Float64`, `Unit`), a
/// `(List T)`, or a `(Tuple T…)`. Used to give a perform's runtime value its shape from the
/// operation's declared result type (the coarse `Kind` loses the element/field types). Returns
/// None for a type this does not know how to render (a user sum, a bare type parameter) — the
/// caller then declines (decline-don't-miscompile).
/// The module RECORD for a built-in module NAME, or `None` if `name` is not a built-in module. A
/// built-in module is a record whose fields hold `(builtin <id>)` values — first-class built-in
/// operation refs (core-semantics.md §A Built-In Module Is A Record Of Its Operations, ask-58). This
/// makes `(. Bytes len)` an ordinary record projection: `resolve` returns this record, the `.` fold
/// projects the `len` field, yielding `(builtin bytes-len)`. The record is SYNTHETIC here (recognized
/// name → record); the end form is a prelude-as-source record, an incremental step away. Only `Bytes`
/// is modeled in this first increment (ask-58 phase-2a); the other modules follow the same shape.
///
/// The `<id>` is the frozen builtin identifier the apply-a-builtin-ref lowering keys on. For `Bytes`
/// the ids mirror the existing dotted-op names (`bytes-len`, `bytes-at`, …), so an applied builtin-ref
/// routes to the SAME lowering the dotted-application path already uses — zero change to how a Bytes
/// operation is emitted, only how the operation VALUE is reached.
fn builtin_module_record(name: &str) -> Option<Node> {
    let field = |f: &str, id: &str| {
        Node::List(vec![
            Node::Name(f.to_string()),
            Node::List(vec![Node::Name("builtin".to_string()), Node::Name(id.to_string())]),
        ])
    };
    let record = |fields: Vec<Node>| {
        let mut items = vec![Node::Name("record".to_string())];
        items.extend(fields);
        Some(Node::List(items))
    };
    match name {
        "Bytes" => record(vec![
            field("of", "bytes-of"),
            field("len", "bytes-len"),
            field("at", "bytes-at"),
            field("concat", "bytes-concat"),
            field("slice", "bytes-slice"),
            field("compact", "bytes-compact"),
        ]),
        "List" => record(vec![
            field("at", "list-at"),
            field("len", "list-len"),
            field("push", "list-push"),
            field("update", "list-update"),
        ]),
        "String" => record(vec![
            field("at", "string-at"),
            field("byte-len", "string-byte-len"),
            field("scalar-len", "string-scalar-len"),
            field("concat", "string-concat"),
            field("slice", "string-slice"),
            field("from-bytes", "string-from-bytes"),
            field("to-bytes", "string-to-bytes"),
        ]),
        "Ast" => record(vec![
            field("encode", "ast-encode"),
            field("decode", "ast-decode"),
        ]),
        // `Int64` carries VALUE CONSTANTS (`max`/`min`) alongside its function ops. Those constants
        // are projected by `gen_member`/`eval_const` BEFORE resolution (they short-circuit on the
        // `Int64.max`/`.min` field names), so they are unaffected by this record; the record supplies
        // only the FUNCTION fields as builtin-refs. A projection of a field not listed here (nor a
        // constant) falls through to the ordinary missing-field trap, as for any record.
        "Int64" => record(vec![
            field("checked-add", "i64-checked-add"),
            field("checked-sub", "i64-checked-sub"),
            field("checked-mul", "i64-checked-mul"),
            field("wrapping-add", "i64-wrapping-add"),
            field("wrapping-sub", "i64-wrapping-sub"),
            field("wrapping-mul", "i64-wrapping-mul"),
        ]),
        // `Option`/`Result` are DECLARED SUM TYPES (their names also denote the type in a type
        // position and their variants `Some`/`None`/`Ok`/`Err` are constructors); `.expect` is a
        // deeply-wired runtime-Option consumer. Not modeled as records here (phase-2b conservatism —
        // routing them through a synthetic record risks colliding with the sum-type/`expect` paths);
        // they stay on their existing dotted-application dispatch and their bare projection declines.
        _ => None,
    }
}

/// Is there an `unquote`/`unquote-splicing` in `node` that sits OUTSIDE any quasiquote — i.e. at
/// quasiquote-nesting `level == 0`? Used to reject a nested unquote inside a PLAIN quote (CDZ0401):
/// a plain quote starts the walk at `level 0`, so an unquote directly in its body is outside any
/// quasiquote and is the same syntax error a bare `,x` is. A nested `quasiquote` raises the level
/// (its body's unquotes are consumed by it, not stray); an `unquote` LOWERS the level for its inner
/// walk (so `,,x` inside one quasiquote — level 1 → the outer unquote is active at level 1, its inner
/// is at level 0, but we only flag an unquote ENCOUNTERED at level 0). The check mirrors `quote_node`'s
/// level accounting: an unquote is "active/stray" exactly at level 0.
fn unquote_outside_quasiquote(node: &Node, level: u32) -> bool {
    match node {
        Node::List(items) => {
            match name_of(items.first()) {
                Some("unquote") | Some("unquote-splicing") => {
                    // An unquote AT level 0 is outside any quasiquote → the stray-unquote error.
                    if level == 0 {
                        return true;
                    }
                    // Inside a quasiquote (level ≥ 1): this unquote is consumed by it; its operand
                    // is walked at one LOWER level (a `,,x` peels one level).
                    items.get(1).map_or(false, |inner| unquote_outside_quasiquote(inner, level - 1))
                }
                // A nested quasiquote raises the level for its body.
                Some("quasiquote") => {
                    items.get(1).map_or(false, |inner| unquote_outside_quasiquote(inner, level + 1))
                }
                // Any other list: an unquote could hide in any child at the SAME level.
                _ => items.iter().any(|c| unquote_outside_quasiquote(c, level)),
            }
        }
        _ => false,
    }
}

/// Does type node `n` name `ty` anywhere (as a bare name or nested inside `(Tuple …)`/`(List …)`/
/// any application)? Used to detect a recursive sum type (a variant payload that mentions the type
/// being declared). Descends every list element so `(Tuple Int64 IntList)` mentions `IntList`.
fn type_node_mentions(n: &Node, ty: &str) -> bool {
    match n {
        Node::Name(name) => name == ty,
        Node::List(items) => items.iter().any(|i| type_node_mentions(i, ty)),
        _ => false,
    }
}

fn shape_of_type_node(n: &Node) -> Option<Shape> {
    match n {
        Node::Name(name) => match name.as_str() {
            "Int64" | "Int" => Some(Shape::Int),
            "Bool" => Some(Shape::Bool),
            "Float64" | "Float" => Some(Shape::Float),
            "Unit" | "unit" => Some(Shape::Unit),
            "Bytes" => Some(Shape::Bytes),
            "String" => Some(Shape::Str),
            _ => None,
        },
        Node::List(items) => match name_of(items.first()) {
            Some("List") => {
                let elem = shape_of_type_node(items.get(1)?)?;
                Some(Shape::List(Box::new(elem)))
            }
            Some("Tuple") => {
                let shapes: Vec<Shape> =
                    items[1..].iter().map(shape_of_type_node).collect::<Option<_>>()?;
                Some(Shape::Tuple(shapes))
            }
            _ => None,
        },
        _ => None,
    }
}

/// The runtime accessor that unboxes a heap handle to a scalar of `kind`, or `None` when the value
/// stays an opaque handle (`Kind::Heap` — a nested sum/tuple/list bound without unboxing). The
/// inverse of `box_scalar`: `box-int`/`get-int`, `box-bool`/`get-bool`, `box-float`/`get-float`.
fn unbox_fn(kind: Kind) -> Option<u32> {
    match kind {
        Kind::Int64 => Some(himport::GET_INT),
        Kind::Bool => Some(himport::GET_BOOL),
        Kind::Float64 => Some(himport::GET_FLOAT),
        _ => None,
    }
}

/// The scalar `Kind` an `Option.expect`/`Result.expect` on `scrutinee` unboxes its payload to — a
/// PURELY SYNTACTIC classifier shared by codegen (`gen_option_expect`) and inference (`infer_list`)
/// so both report the SAME result kind for the same expression (a mismatch would emit a function
/// whose wasm signature disagrees with what its caller reads → INVALID component). Only the Option
/// producers whose payload is a KNOWN scalar unbox: `Int64.checked-*` and `Bytes.at` (and `List.at`
/// over an Int list) yield `Option<Int>`, so `expect` unwraps a plain `Int64`. Every other scrutinee
/// (a `(Some …)` of a compound, a parameter `o : Option`, a `String.at`/`Bytes.slice` result) keeps
/// the raw payload HANDLE (`Heap`), rendered via the scrutinee's inferred payload shape. Conservative
/// on purpose: unboxing only where the payload is unambiguously Int keeps this classifier a cheap
/// syntactic match, and a wrongly-kept handle merely renders (never miscompiles).
fn expect_payload_kind(scrutinee: &Node) -> Kind {
    if let Node::List(elems) = scrutinee {
        if let Some(Node::List(hd)) = elems.first() {
            if name_of(hd.first()) == Some(".") {
                match (name_of(hd.get(1)), name_of(hd.get(2))) {
                    (Some("Int64"), Some("checked-add"))
                    | (Some("Int64"), Some("checked-sub"))
                    | (Some("Int64"), Some("checked-mul"))
                    | (Some("Bytes"), Some("at")) => return Kind::Int64,
                    _ => {}
                }
            }
        }
    }
    Kind::Heap
}

/// Canonical-byte-form float equality: every NaN equals every NaN; otherwise equality is by
/// canonical bit pattern, so -0.0 ≠ 0.0 (distinct bits) even though they are numerically
/// equal (core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form).
fn float_canonical_eq(x: f64, y: f64) -> bool {
    if x.is_nan() && y.is_nan() {
        return true;
    }
    x.to_bits() == y.to_bits()
}

/// Is this application head a special form (not an ordinary callee)? Used to keep the
/// generic partial-application resolver from mistaking `(let …)`, `(if …)`, `(+ …)`, etc.
/// for a curried call.
fn is_special_form_head(head: Option<&Node>) -> bool {
    match name_of(head) {
        Some(h) => matches!(
            h,
            "let" | "if" | "do" | "match" | ":" | "fn" | "quote" | "quasiquote"
                | "unquote" | "record" | "tuple" | "list" | "map"
                | "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>"
                | "<" | ">" | "<=" | ">=" | "="
        ),
        None => false,
    }
}

/// Validate a special form's operand count so no later pass indexes past a short form and
/// panics (a compiler must never crash on malformed input). A form with the wrong arity is a
/// malformed program → rejected. `match`/`do`/`list`/`tuple`/`record`/applications are
/// variadic and validated where they are consumed.
fn check_arity(elems: &[Node]) -> Result<(), Decline> {
    let head = match elems.first() {
        Some(Node::Name(h)) => h.as_str(),
        _ => return Ok(()),
    };
    let n = elems.len() - 1; // operand count
    let ok = match head {
        "if" => n == 3,
        "=" | "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" | "<" | ">" | "<=" | ">=" => {
            n == 2
        }
        // Boolean connectives (core-semantics.md #Boolean Connectives Short-Circuit): `and`/`or`
        // are binary, `not` unary. They desugar to short-circuit `if`, so the arity is fixed.
        "and" | "or" => n == 2,
        "not" => n == 1,
        ":" | "." => n == 2,
        "quote" | "quasiquote" | "unquote" | "unquote-splicing" => n == 1,
        // `let` needs bindings + a body, and every binding must be a `(name value)` pair —
        // guarded here so a malformed binding like `(x)` (no value) rejects rather than
        // panicking when a later pass reads the value.
        "let" => {
            n >= 2
                && matches!(elems.get(1), Some(Node::List(binds))
                    if binds.iter().all(|b| matches!(b, Node::List(kv)
                        if kv.len() == 2 && matches!(kv.first(), Some(Node::Name(_))))))
        }
        _ if head.starts_with("tuple.") => n == 1,
        _ => true, // variadic or an application — validated at its use site
    };
    if ok {
        Ok(())
    } else {
        // An arity mismatch is an ill-typed application (the form is applied to the wrong
        // number of operands) — rejected with the type-mismatch diagnostic, never a panic.
        reject("CDZ0201", format!("malformed `{head}` form: arity mismatch"))
    }
}

/// Is this node a labeled field `(name value)` — a `List` whose head is a name and which has
/// exactly one value? Used to distinguish a nominal record constructor `(Point (x 0) (y 0))`
/// from a sum constructor application `(Some 42)`.
fn is_labeled_field(node: &Node) -> bool {
    // A record field `(name value)` is keyed by an ordinary (lowercase) field name. A
    // constructor application `(Some 5)` is ALSO a 2-element name-headed list, but its head is a
    // CONSTRUCTOR, not a field label — so it is a Sum payload, not a labeled field. Excluding a
    // constructor-named head keeps `(Some (Some 5))` a Sum whose payload is a Sum (rendered
    // `(Some (Some 5))`), rather than misreading the outer `Some` as a nominal record field set
    // `{Some: 5}` and dropping the tag (05-compound-types.sexp §"a constructor whose payload is
    // a constructor keeps the outer variant tag").
    matches!(node, Node::List(kv)
        if kv.len() == 2
        && matches!(kv.first(), Some(Node::Name(k)) if !is_constructor_name(k)))
}

/// The static type of a LITERAL pattern (a scalar literal in match-pattern position), or None if
/// the pattern is not a scalar literal (a name binder, `else`/`_`, or a compound/constructor
/// pattern — those are checked elsewhere). Used to reject a literal-pattern arm whose type
/// differs from the scrutinee's (a cross-type match can never succeed, CDZ0201).
fn literal_pattern_type(pattern: Option<&Node>) -> Option<StaticType> {
    match pattern {
        Some(Node::Int(_)) => Some(StaticType::Int),
        Some(Node::Bool(_)) => Some(StaticType::Bool),
        Some(Node::Float(_)) => Some(StaticType::Float),
        Some(Node::Str(_)) => Some(StaticType::Str),
        _ => None,
    }
}

/// The first field/key NAME that appears more than once among `(name value)…` entries, or None
/// if every name is distinct. Used to reject a record with a duplicate field (not a fixed field
/// SET) and a map with a repeated key (each key at most once) — both CDZ0201, over the whole
/// entry list (a non-adjacent duplicate counts). Entries that are not `(name …)` are ignored.
fn duplicate_field_name(entries: &[Node]) -> Option<String> {
    let mut seen: std::collections::BTreeSet<&str> = Default::default();
    for entry in entries {
        if let Node::List(kv) = entry {
            if let Some(Node::Name(k)) = kv.first() {
                if !seen.insert(k.as_str()) {
                    return Some(k.clone());
                }
            }
        }
    }
    None
}

/// A rendering of the first record/map entry that is NOT a well-formed `(name value)` pair, or
/// None if every entry is such a pair. A record/map entry MUST be a two-element list whose head
/// is a field/key name (core-semantics.md #A Record Has A Fixed Set Of Named Fields): a bare
/// `(a)` with no value, an over-long `(a v w)`, or a non-list entry is ill-formed. This is the
/// never-crash guard for the construction path — it fires before `eval_const` indexes an entry's
/// value node, turning `(record (a))` / `(map (a))` into a CDZ0201 rejection rather than a
/// codegen panic (07-type-system.sexp §"a record/map … with no value expression is rejected, not
/// a crash").
fn malformed_kv_entry(entries: &[Node]) -> bool {
    entries.iter().any(|entry| {
        !matches!(entry,
            Node::List(kv) if kv.len() == 2 && matches!(kv.first(), Some(Node::Name(_))))
    })
}

/// Scan a quasiquote body for an `unquote`/`unquote-splicing` form with the wrong operand count.
/// Both take EXACTLY ONE operand, so a form `(unquote 1 2)` (or `(unquote)`) is malformed and
/// yields the CDZ0201 code, so quasiquote expansion rejects it rather than dropping the extra
/// operand. Walks the whole quoted body since a malformed unquote can be nested anywhere in it.
fn malformed_unquote_arity(node: &Node) -> Option<&'static str> {
    if let Node::List(items) = node {
        if matches!(name_of(items.first()), Some("unquote") | Some("unquote-splicing"))
            && items.len() != 2
        {
            return Some("CDZ0201");
        }
        for child in items {
            if let Some(code) = malformed_unquote_arity(child) {
                return Some(code);
            }
        }
    }
    None
}

/// Distribute a projection (member access or tuple access) over a control form that chooses
/// the projected value at run time, pushing `wrap` down to the value-producing leaves:
///   `(if c a b)`          → `(if c (wrap a) (wrap b))`
///   `(let (…) body)`      → `(let (…) (wrap body))`
///   `(do … last)`         → `(do … (wrap last))`
///   `(match s (p e)…)`    → `(match s (p (wrap e))…)`
/// so a record/tuple selected by a conditional is projected in each branch (where it is
/// compile-time-known) rather than requiring the whole projection to fold at once. Returns
/// None if the object is not one of these control forms (the caller resolves it directly).
fn distribute_projection(obj: &Node, wrap: impl Fn(Node) -> Node + Copy) -> Option<Node> {
    let items = match obj {
        Node::List(items) => items,
        _ => return None,
    };
    match name_of(items.first()) {
        Some("if") if items.len() == 4 => Some(Node::List(vec![
            items[0].clone(),
            items[1].clone(),
            wrap(items[2].clone()),
            wrap(items[3].clone()),
        ])),
        Some("let") if items.len() >= 3 => {
            let mut out = items[..items.len() - 1].to_vec();
            out.push(wrap(items[items.len() - 1].clone()));
            Some(Node::List(out))
        }
        Some("do") if items.len() >= 2 => {
            let mut out = items[..items.len() - 1].to_vec();
            out.push(wrap(items[items.len() - 1].clone()));
            Some(Node::List(out))
        }
        Some("match") if items.len() >= 2 => {
            let mut out = vec![items[0].clone(), items[1].clone()];
            for arm in &items[2..] {
                match arm {
                    Node::List(a) if a.len() == 2 => {
                        out.push(Node::List(vec![a[0].clone(), wrap(a[1].clone())]));
                    }
                    other => out.push(other.clone()),
                }
            }
            Some(Node::List(out))
        }
        _ => None,
    }
}

/// The ground kind a literal match pattern constrains its scrutinee to, if the pattern is a
/// literal (an integer or a boolean). A binder/constructor/tuple pattern returns None.
fn literal_pattern_kind(pattern: &Node) -> Option<Kind> {
    match pattern {
        Node::Int(_) => Some(Kind::Int64),
        Node::Bool(_) => Some(Kind::Bool),
        _ => None,
    }
}

/// Is this match pattern a SUM constructor pattern — `(Ctor binder)` or `((. Ty Ctor) binder)`?
/// Such a pattern matches a runtime sum value, so it constrains its scrutinee to `Kind::Heap`. A
/// `(tuple …)` pattern is a product deconstruction (its scrutinee may be a compile-time tuple),
/// NOT a sum, so it is excluded — `constructor_of` already rejects `tuple` (lowercase, not a
/// constructor name).
fn is_constructor_pattern(pattern: &Node) -> bool {
    match pattern {
        Node::List(items) if items.len() == 2 => constructor_of(items.first()).is_some(),
        _ => false,
    }
}

/// Is a match ARM `(pattern body)` whose pattern is a `(tuple …)` — a tuple-destructuring arm? Used
/// to route a runtime HEAP scrutinee to the tuple-match path (a tuple is a heap array) rather than
/// the sum-match path (which needs a constructor arm).
fn arm_is_tuple_pattern(arm: &Node) -> bool {
    matches!(arm, Node::List(a) if a.len() == 2
        && matches!(&a[0], Node::List(p) if name_of(p.first()) == Some("tuple")))
}

/// The binders of an IRREFUTABLE tuple-destructuring arm — a `(tuple b0 … bn)` pattern EVERY slot of
/// which is a name or `_` (no literal, no constructor sub-pattern), so the arm always matches and each
/// slot merely BINDS its element. Returns the binder nodes, or `None` if the arm is not such a pattern
/// (a slot literal like `(tuple 0 y)` or a nested constructor makes it REFUTABLE — a value test, not a
/// pure bind — and must NOT be treated as an irrefutable binder for return-kind inference). Used by
/// the inference `match` arm to recover each slot binder's kind from a call-returned tuple scrutinee
/// (ask-77), distinct from `arm_is_tuple_pattern` (which accepts any `(tuple …)` head).
fn irrefutable_tuple_binders(arm: &Node) -> Option<&[Node]> {
    if let Node::List(a) = arm {
        if a.len() == 2 {
            if let Node::List(p) = &a[0] {
                if name_of(p.first()) == Some("tuple")
                    && p[1..].iter().all(|slot| matches!(slot, Node::Name(_)))
                {
                    return Some(&p[1..]);
                }
            }
        }
    }
    None
}

/// Is a match ARM a CATCH-ALL — `else` / `_` / a bare-name binder (binds the whole scrutinee)? A
/// catch-all arm is handled by the sum-match path (which already binds a bare name / falls through),
/// so a runtime tuple match routes there instead when any arm is a catch-all.
fn arm_is_catch_all(arm: &Node) -> bool {
    matches!(arm, Node::List(a) if a.len() == 2 && matches!(&a[0], Node::Name(_)))
}

/// The slot indices of a `(tuple …)` binder that are themselves REFUTABLE constructor patterns —
/// `(tuple (Inner.A v) k)` → `[0]`. `None` when `binder` is not a `(tuple …)` binder at all. A
/// constructor slot needs runtime discriminant DISPATCH (not just binding), so it is lowered by
/// recursing the sum-match dispatcher on that slot; the irrefutable slots (names/`_`) bind normally.
fn tuple_binder_refutable_slots(binder: &Node) -> Option<Vec<usize>> {
    match binder {
        Node::List(items) if name_of(items.first()) == Some("tuple") => Some(
            items[1..]
                .iter()
                .enumerate()
                .filter(|(_, b)| is_constructor_pattern(b))
                .map(|(i, _)| i)
                .collect(),
        ),
        _ => None,
    }
}

/// Is `name` a special-form keyword (a head that names a form, not a user variable)? Used to
/// distinguish a genuine unbound-name scope error from a not-yet-compiled form appearing in a
/// value position.
/// Desugar a boolean connective to its short-circuit `if` form (core-semantics.md #Boolean
/// Connectives Short-Circuit): `(and a b)` → `(if a b false)`, `(or a b)` → `(if a true b)`,
/// `(not a)` → `(if a false true)`. The single source the emit and const-fold paths share, so
/// they can never diverge on the desugaring. `items` is the whole `(and …)` form; arity was
/// checked by `check_arity` before this is reached.
fn desugar_connective(head: &str, items: &[Node]) -> Node {
    let t = || Node::Bool(true);
    let f = || Node::Bool(false);
    let if_ = |c: Node, a: Node, b: Node| {
        Node::List(vec![Node::Name("if".into()), c, a, b])
    };
    match head {
        "and" => if_(items[1].clone(), items[2].clone(), f()),
        "or" => if_(items[1].clone(), t(), items[2].clone()),
        // "not"
        _ => if_(items[1].clone(), f(), t()),
    }
}

fn is_form_keyword(name: &str) -> bool {
    matches!(
        name,
        "quote" | "quasiquote" | "unquote" | "unquote-splicing" | "tuple" | "record" | "map"
            | "list" | "match" | "let" | "if" | "do" | "fn" | "module" | "def" | "use" | "meta"
        // Boolean connectives desugar to short-circuit `if`; keywords so a malformed use declines
        // honestly rather than misfiring the ungranted-capability path in `gen_call`.
            | "and" | "or" | "not"
        // Effect special forms (options/code-shape/homoiconic-decoupled-display.md keyword table):
        // head-position keywords, so a bare `resume`/`handle`/`host`/`effect` declines as a
        // not-yet-lowered form rather than misfiring the ungranted-effect path in `gen_call`.
            | "effect" | "op" | "handle" | "host" | "resume"
    )
}

/// Is `tok` shaped like an integer LITERAL — a digit-led run of digits and `_` separators, with
/// an optional leading sign? Mirrors the reader's `looks_like_int`: such a token is a numeric
/// literal, so if it reached the compiler as a `Node::Name` it is an OUT-OF-RANGE integer (the
/// reader parses in-range ones to `Node::Int`), a malformed literal rather than a name.
fn looks_like_numeric_literal(tok: &str) -> bool {
    let body = tok.strip_prefix('-').or_else(|| tok.strip_prefix('+')).unwrap_or(tok);
    // A radix-prefixed token `0x…`/`0b…` is numeric in shape (a hex/binary literal); if it reached
    // here as a `Node::Name` it is out of the Int64 range — a malformed literal (CDZ0201), not an
    // unbound name (01-literals.sexp §"a hexadecimal literal past Int64.max"). Accept the radix
    // alphabet so an overflowing `0xFFFFFFFFFFFFFFFF` reports the honest out-of-range diagnostic.
    if let Some(radix_body) = body.strip_prefix("0x") {
        return !radix_body.is_empty()
            && radix_body.chars().all(|c| c.is_ascii_hexdigit() || c == '_');
    }
    if let Some(radix_body) = body.strip_prefix("0b") {
        return !radix_body.is_empty()
            && radix_body.chars().all(|c| c == '0' || c == '1' || c == '_');
    }
    // A FLOAT-shaped token that reached here as a `Node::Name` is a malformed float literal — the
    // reader (`looks_like_float`) refused it for a MISPLACED digit separator (`1._5`, `1.5_`,
    // `1_.5`, `1.5e_10`), so it must surface the honest CDZ0201 (malformed literal), NOT CDZ0101
    // (unbound name). A digit-led token containing a `.`/`e`/`E` (float shape) built only from the
    // float character set is numeric-in-shape (01-literals.sexp §"a digit separator adjacent to a
    // float's decimal point is a malformed literal"). Checked after the integer/radix shapes above.
    if body.chars().next().map_or(false, |c| c.is_ascii_digit())
        && (body.contains('.') || body.contains('e') || body.contains('E'))
        && body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
    {
        return true;
    }
    body.chars().next().map_or(false, |c| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_digit() || c == '_')
}

/// Is a runtime-Bool `match` exhaustive? A Bool has exactly the two values `true`/`false`, so
/// the match covers them iff it has a catch-all arm (a bare-name binder, `else`, or `_`) or it
/// names both boolean literals. Arms are `(pattern body)` lists.
fn bool_match_exhaustive(arms: &[Node]) -> bool {
    let mut has_true = false;
    let mut has_false = false;
    for arm in arms {
        let pattern = match arm {
            Node::List(a) if a.len() == 2 => &a[0],
            _ => continue,
        };
        match pattern {
            Node::Bool(true) => has_true = true,
            Node::Bool(false) => has_false = true,
            // A bare-name binder / `else` / `_` is a catch-all → exhaustive.
            Node::Name(_) => return true,
            _ => {}
        }
    }
    has_true && has_false
}

/// Does a match's arm set include a CATCH-ALL — a bare-name binder, `else`, or `_` pattern — that
/// matches any value? Used to decide exhaustiveness for an UNBOUNDED scalar scrutinee (Int64,
/// Float64, String, Bytes): no finite set of literal arms can cover such a type, so the match is
/// exhaustive only with a catch-all (core-semantics.md #Matching Is Exhaustive Or Rejected). A
/// bare-name pattern (`x`, `else`, `_`) is a catch-all; a literal / constructor / tuple pattern
/// constrains, so does not.
fn arms_have_catch_all(arms: &[Node]) -> bool {
    arms.iter().any(|arm| match arm {
        Node::List(a) if a.len() == 2 => matches!(&a[0], Node::Name(_)),
        _ => false,
    })
}

/// Is `name` a constructor (a variant tag)? Constructors are capitalized names, or the
/// prelude sum constructors. The canonical tree writes a qualified constructor `Sign.Zero`
/// as `(. Sign Zero)`, handled separately by `constructor_of`.
fn is_constructor_name(name: &str) -> bool {
    matches!(name, "Some" | "None" | "Ok" | "Err")
        || name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

/// The constructor a scrutinee's head denotes: a bare capitalized name (`Some`), or the
/// variant of a qualified member form `(. Sign Zero)` → `Zero`. The qualified variant must
/// itself be a constructor name (capitalized), so a lowercase method like `(. Ast decode)`
/// is NOT mistaken for a constructor.
fn constructor_of(head: Option<&Node>) -> Option<String> {
    match head {
        Some(Node::Name(n)) if is_constructor_name(n) => Some(n.clone()),
        Some(Node::List(items)) if name_of(items.first()) == Some(".") => {
            match name_of(items.get(2)) {
                Some(v) if is_constructor_name(v) => Some(v.to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse `(def (name p1 p2 …) body)` → (name, params, body). Non-def forms return None.
/// One top-level definition: a FUNCTION `(def (f p…) body)` or a VALUE `(def name value)`.
/// A value-def binds `name` to an ordinary expression at module scope (glossary: a definition is
/// "a value, function, type"); it registers a module field usable by sibling functions (ask-71),
/// distinct from a function-def which becomes a `Func` in `funcs`.
enum Def {
    Func(String, Vec<String>, Node),
    Value(String, Node),
}

fn parse_def(form: &Node) -> Result<Option<Def>, Decline> {
    let items = match form {
        Node::List(items) => items,
        _ => return Ok(None),
    };
    if name_of(items.first()) != Some("def") {
        return Ok(None);
    }
    match items.get(1) {
        // Function def: `(def (name param…) body)`.
        Some(Node::List(sig)) => {
            let name = match sig.first() {
                Some(Node::Name(n)) => n.clone(),
                _ => return decline("def signature without a name"),
            };
            let mut params = Vec::new();
            for p in &sig[1..] {
                match p {
                    Node::Name(n) => params.push(n.clone()),
                    _ => return decline("non-name parameter"),
                }
            }
            let body = match items.last() {
                Some(b) => b.clone(),
                None => return decline("def without a body"),
            };
            Ok(Some(Def::Func(name, params, body)))
        }
        // VALUE def: `(def name value)` — `name` is a bare Name, `value` an ordinary expression.
        // A module value-def binds a name usable by the module's functions (ask-71); e.g. a shared
        // `(def op (record …))` opcode table, or `(def answer 42)`. Like a function def, it MAY carry
        // DOCUMENTATION as a `(doc "…")` form between the name and the value (agent-authoring.md §Any
        // definition MUST be able to carry documentation) — `(def op (doc "…") (record …))`, the shape
        // the generated opcode table takes. The value is the LAST element (the doc, if present, is an
        // inert leading form the compiler ignores, exactly as the function path ignores a `(doc …)`
        // before its body via `items.last()`). A middle element that is NOT a `(doc …)` form is a
        // malformed def (a value-def has one value, optionally documented), not silently dropped.
        Some(Node::Name(name)) => {
            // Is `items[2]` a `(doc …)` form (a documented value-def `(def name (doc …) value)`)?
            let middle_is_doc = matches!(
                items.get(2),
                Some(Node::List(d)) if name_of(d.first()) == Some("doc")
            );
            let value = match items.len() {
                // `(def name value)`
                3 => items[2].clone(),
                // `(def name (doc …) value)` — the middle element MUST be a `(doc …)` form.
                4 if middle_is_doc => items[3].clone(),
                _ => return decline("value def without a single value expression"),
            };
            Ok(Some(Def::Value(name.clone(), value)))
        }
        _ => decline("def without a signature"),
    }
}

/// A host function a program imports: `(import (host NAME (func (PARAM-TYPE…) RESULT-TYPE)))`.
/// Its name is the capability the manifest enumerates and the import name at the component
/// boundary; its WIT-typed signature fixes how it lowers (host-interface-binding.md §A Host
/// Import Is A WIT-Typed Function The Manifest Enumerates). A capability and a boundary effect
/// are one concept (capabilities-and-effects.md §A Host Import Is A Boundary Effect And The
/// Manifest Is Its Row).
#[derive(Clone)]
struct HostImport {
    name: String,
    params: Vec<Kind>,
    result: Kind,
}

/// Parse the type name in a host import signature to a boundary `Kind`. Only the boundary types
/// a host function currently uses are recognized; anything else declines (the program is not yet
/// lowerable, not ill-typed).
fn host_type_kind(node: &Node) -> Option<Kind> {
    match node {
        Node::Name(n) => match n.as_str() {
            "Int64" => Some(Kind::Int64),
            "Bool" => Some(Kind::Bool),
            "Float64" => Some(Kind::Float64),
            "String" => Some(Kind::HostString),
            "unit" | "Unit" => Some(Kind::Unit),
            _ => None,
        },
        _ => None,
    }
}

/// Unify the two branch kinds of an `if` / the arm kinds of a `match` for RETURN-KIND inference,
/// order-independently. The tie-break on disagreement, in priority order:
///   1. `Heap` wins — a genuine runtime compound is "more defined" than a scalar (the Tier 00 rule:
///      a recursive compound builder whose base branch is Heap and whose recursive branch reports the
///      callee's still-default Int64 must converge to Heap, not lock Int64).
///   2. A CONCRETE scalar (`Bool`/`Float64`/`Unit`) beats `Int64`. `Int64` is the UNCONSTRAINED
///      default a still-unsolved recursive self-call reports, so when one branch is a Bool/Float
///      literal and the other is a defaulted self-call, the concrete kind must win regardless of
///      branch ORDER (the Tier 2d Bool-return asymmetry: `(if g (self-call) false)` locked Int64
///      because the then-biased `t.or(e)` saw the placeholder Int64 first). A genuine Int64-vs-Bool
///      conflict is still caught at emit; this only resolves the placeholder-vs-concrete race.
///   3. Otherwise `t.or(e)` (equal kinds, or one `None`).
/// `Never` (a diverging branch) yields to its sibling so the other branch's kind is taken.
/// The scalar `Kind` a leaf `Shape` unboxes to (Int/Bool/Float), else `None` (a compound element
/// stays an opaque Heap handle — the tuple-slot Heap default).
fn shape_scalar_kind(s: Shape) -> Option<Kind> {
    match s {
        Shape::Int => Some(Kind::Int64),
        Shape::Bool => Some(Kind::Bool),
        Shape::Float => Some(Kind::Float64),
        _ => None,
    }
}

/// Merge one tuple-branch's per-slot scalar kinds into the accumulator: absent → adopt it; present
/// → keep a slot only where both agree (a disagreement or an unknown drops that slot to `None`).
fn merge_slot_kinds(acc: &mut Option<Vec<Option<Kind>>>, this: Vec<Option<Kind>>) {
    match acc {
        None => *acc = Some(this),
        Some(cur) if cur.len() == this.len() => {
            for (c, t) in cur.iter_mut().zip(this) {
                if *c != t {
                    *c = None;
                }
            }
        }
        _ => {}
    }
}

fn unify_branch_kinds(t: Option<Kind>, e: Option<Kind>) -> Option<Kind> {
    match (t, e) {
        (Some(Kind::Never), other) | (other, Some(Kind::Never)) => other,
        (Some(Kind::Heap), _) | (_, Some(Kind::Heap)) => Some(Kind::Heap),
        // A concrete scalar beats the Int64 default (a placeholder for an unsolved recursive call).
        (Some(Kind::Int64), Some(k)) | (Some(k), Some(Kind::Int64)) if k != Kind::Int64 => Some(k),
        (a, b) => a.or(b),
    }
}

/// Scan for the arithmetic operators that need an overflow-checked helper.
/// Is this decline a HEAP-need — a body that could not lower on the scalar path because it needs
/// the runtime value heap (a runtime compound/sum constructor, or shape inference for one)? These
/// are the declines the runtime-mode retry clears; any other decline is a genuine unsupported
/// construct that the retry cannot fix.
fn is_heap_decline(d: &Decline) -> bool {
    let m = &d.0;
    m.contains("constant sum (folds or is dead)")
        || m.contains("constant compound (folds or is dead)")
        || m.contains("cannot infer runtime compound result shape")
        || m.contains("runtime compound constructor not yet emitted")
        || m.contains("runtime bytes value needs the value-heap runtime")
        || m.contains("runtime string value needs the value-heap runtime")
        || m.contains("runtime tuple access needs the value-heap runtime")
        || m.contains("runtime list value needs the value-heap runtime")
        || m.contains("runtime sum value needs the value-heap runtime")
        || m.contains("runtime sum match needs the value-heap runtime")
        || m.contains("runtime tuple match needs the value-heap runtime")
}

fn scan_helpers(node: &Node, h: &mut Helpers) {
    if let Node::List(items) = node {
        match name_of(items.first()) {
            Some("+") => h.add = true,
            Some("-") => h.sub = true,
            Some("*") => h.mul = true,
            _ => {}
        }
        for child in items {
            scan_helpers(child, h);
        }
    }
}


// ─── small tuple helper ──────────────────────────────────────────────────────────

trait CloneTuple {
    fn clone_tuple(&self) -> (String, Vec<String>, Node);
}
impl CloneTuple for (String, Vec<String>, Node) {
    fn clone_tuple(&self) -> (String, Vec<String>, Node) {
        (self.0.clone(), self.1.clone(), self.2.clone())
    }
}
