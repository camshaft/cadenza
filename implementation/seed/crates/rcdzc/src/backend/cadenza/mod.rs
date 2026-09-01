//! The Cadenza backend — lower the OPTIMIZED Core back to Cadenza surface, emitting the BINARY AST.
//!
//! Unlike the wasm/rust backends (which emit a runnable artifact), this backend emits the PROGRAM
//! ITSELF back out: it walks the same target-neutral columns everything above the seam produced
//! (`core_of`/`type_of`/[`Layout`]) — i.e. the program AFTER resolution, inference, const-folding, and
//! optimization — and reconstructs a Cadenza surface AST from it, then serializes that tree with the
//! binary-AST codec ([`crate::codec::encode`]). The result is a `kind == "ast"` artifact.
//!
//! Three properties this enables (operator's mandate):
//! - **round-trip idempotence** — feeding the emitted binary AST back through `compile` yields the
//!   identical optimized program, hence a byte-identical re-emit (the correctness + optimization-
//!   inspection signal). A deterministic build order is what makes this hold: `crate::codec::encode`
//!   serializes BUILD ORDER (no canon pass), so this backend builds head-first, left-to-right, and
//!   the same Core always produces the same bytes.
//! - **syntax-system pipe** — the bytes decode straight into `cadenza-syntax` for sexpr/ML rendering,
//!   so a human can inspect what lowering + folding did.
//! - **lean-oracle input** — the oracle consumes binary AST directly.
//!
//! Like the other backends it consumes the structured Core DIRECTLY (no flat wasm `Lir`) and uses only
//! `backend::common`, never the sibling backends' internals. It DECLINES (attributed to this target) a
//! construct it does not yet reconstruct — the same decline-don't-miscompile discipline the wasm/rust
//! backends follow. Coverage so far:
//! - **B0**: whole-program shape (`(do (def …)… (export …)…)`) with CONSTANT-bodied definitions — the
//!   PLAIN constant leaves (Int/Bool/Str/Char/Float/Unit) as literals, and the WRAPPER-typed numeric-
//!   tower / nominal-leaf constants via a re-compilable surface: `BigInt`→`(: n BigInt)` (the direct
//!   ascription — `BigInt.of` widens an `Int64` so it can't hold a beyond-`Int64` literal),
//!   `Rational`→`(Rational.of n d)` when num/den fit `Int64` (else declines), `Symbol`→`(Symbol.of "…")`
//!   (emitting the bare scalar would drop the type and miscompile the value). `Ty::Qty` still declines
//!   (needs unit reconstruction — a later slice).
//! - **B1a**: PARAMETERS — a def signature `(<name> (: <p> <Ty>)…)` (param types via lower's canonical
//!   `type_ast`) and a `Core::Param`/`LocalRef` reference (the bare binder name). A parameter of a type
//!   with no value-form surface (function/unsolved) declines.
//! - **B1b**: OPERATORS + CONTROL — the runtime binary operators (`Arith`/`Compare`/`StrCmp`/
//!   `FloatCompare`, re-emitted `(<op> l r)` via the `Prim`→surface reverse-map), boolean `Not`
//!   (`(not x)`), short-circuit `And`/`or` (`(and|or l r)`), and the conditional `If` (`(if c t e)`).
//! - **B2**: BINDING — a kept multi-use `Core::Let` re-emits as `(let ((<n> <v>)…) <body>)` with
//!   DETERMINISTIC synthesized binding names (the source name is discarded at lowering), and a
//!   `Core::LocalRef` resolves to its binding's synthesized name via the threaded [`BinderEnv`].
//! - **B3**: CALLS — a `Core::Call` (a non-inlinable, i.e. recursive, application) re-emits as
//!   `(<callee-name> <arg>…)`, naming the callee by its source name (it is in `layout.order`, so its
//!   `(def …)` is emitted too).
//!
//! - **M1**: scalar `Core::Match` (Int/Bool probes + wildcard/binder) → an `if`-chain of `(= scrut lit)`
//!   probes (value-equivalent; the scrutinee is a pure scalar). A GUARDED arm desugars into the `if`
//!   condition — `(if (and (= scrut lit) <guard>) body rest)`, or `(if <guard> body rest)` for a guarded
//!   wildcard — since a guard's fall-through IS the `if`/else chain. A non-scalar probe
//!   (Str/Char/Bytes/list/map) declines.
//! - **M4a**: sum `Core::MatchSum` → surface `(match <scrutinee> (<Variant> <binder>…) <body>)…`
//!   ([`emit_match_sum`]): a root switch on the scrutinee's OWN discriminant with EXPLICIT-variant, bare
//!   `Leaf`-body arms. Each arm mints a fresh `_cdz_m<n>` binder per payload slot (recorded in
//!   `env.payloads` under the `SumPayload` `(scrutinee, path)` key the body reads); a `Core::SumPayload`
//!   resolves to its binder. NESTED matches (a `Leaf` body that is itself a `MatchSum`) recurse naturally.
//!   A DEFAULT (`disc: None`) arm re-emits the wildcard `_` (a catch-all over the residual variants). A
//!   disc-FOLDED / nested-`Switch` / `Guarded` / `LitTest` decision tree still declines. A match over a
//!   user sum whose `(type …)` was not re-emitted declines; prelude sums (Option/Result) are ambient.
//! - **M4b**: list `Core::MatchList` → surface `(match <scrutinee> (<list-pattern> <body>)…)`
//!   ([`emit_match_list`]): a length-dispatch arm — `LenEq(n)`→`(list b0 … b_{n-1})`, `LenGe(lead)`→
//!   `(list b0 … b_{lead-1} .. rest)`, `Any`→`_`. Leading element binders register at `[Elem(i)]`, the
//!   rest binder at `[RestFrom(lead)]` (same `env.payloads` map M4a uses); a `Core::SumPayload` resolves
//!   to its binder, and nested list matches recurse. A GUARDED arm re-emits the `(guard <pattern> <cond>)`
//!   surface form (cond with the arm's binders in scope). A NESTED/variant element sub-pattern (a deeper
//!   `SumPayload` path this slice does not register) declines.
//! - **DATA**: runtime compound VALUES — via the M2 native ctor-leaf heads (`b.compound(CompoundCtor::…)`),
//!   not name heads: `Core::Tuple`/`ListNew`/`SetOf`→`Tuple`/`List`/`Set` ctor over the element children;
//!   `Core::Record`/`MapNew`→`Record`/`Map` ctor over `(= k v)` FieldPair leaves (record/map distinguished by
//!   the head, both field-pair children). Map/set entries emit in STORED order — the value is unordered, so
//!   the round-trip is VALUE-equivalence, order-independent; the keys are runtime, no canonical sort applies);
//!   and a `Core::SumNew` variant →
//!   `(: (<Variant> <payload-or-unit>) <sum-type>)` (the type ascription pins an under-determined sum,
//!   e.g. a bare `(None unit)`). When the value's OWN solved type is under-determined (a bare `(None)` at a
//!   join, whose own type is `Option<?>`), the `<sum-type>` is recovered from the `expected` type its
//!   container passed down (see `emit_expr`'s `expected` / [`body_ctx`]); a still-free type declines. The
//!   `expected` also threads into COMPOUND-VALUE element positions — a list/set element gets the
//!   collection's element type, a tuple element its slot type, a map entry its key/value types, a record
//!   field its field type — so a bare `(None)` element (`(list (Some n) (None) …)`) recovers its type.
//!   A bare `(None)` NESTED as a variant PAYLOAD (`(Some (None))`) recovers its type from the variant's
//!   INSTANTIATED payload type ([`sum_payload_expected`] via `infer::payload_ty_at_instantiation`), so a
//!   nested-`Option`/`Result` round-trips too. All mirror lower's value surface.
//!   A USER sum is re-declared: `emit` emits its `(type <Name> (<Variant> <PayloadTy>…)…)` decl (a CLOSED
//!   sum of any arity; a GENERIC sum too — head `(<Name> p0 p1…)`, a bare type-param payload re-emits its
//!   name, so `(type (Box a) (Mk a))` round-trips and its values/matches unblock) and its values then
//!   round-trip; a compound-with-param payload (`(List a)`) or an OPEN sum still declines. A
//!   SINGLE-variant sum is the ERASED `Ty::Nominal` newtype: its value re-emits the CONSTRUCTOR
//!   `(<Ctor> <payload>)` at the construction sites [`nominal_disposition`] classifies — a value-producing
//!   leaf/operator OR a COMPOUND-value builder (`(Mk (list …))`/`(tuple …)`/`(record …)`/`(map …)`/`Set.of`)
//!   OR a binder whose declared type is the inner — with the payload peeled to `inner` via a `view`
//!   recursion; pass-through positions (control flow, a binder already holding the nominal) emit unwrapped
//!   so the constructor is not doubled. An OPEN user sum (row tail) still DECLINES (no decl emitted). PRELUDE sums
//!   (Option/Result/…) are ambient (no decl). A user-sum/nominal value emits ⇔ its decl was emitted
//!   (`emitted` set), so there is never an unbound-type recompile.
//! - **WRAPPING ARITH**: `Core::Arith` with a `WrappingAdd`/`Sub`/`Mul` prim → `((. <IntType> wrapping-add)
//!   l r)` (opted-into BY NAME on the operand's int-type module — a plain `+` is the trapping form).
//! - **NUMERIC TOWER**: BigInt/Rational ops — `BigIntOfI64`→`(BigInt.of n)`, `BigIntBinOp`/`RationalBinOp`
//!   → the plain operator `(+ l r)` etc (tower op re-selected by operand type on recompile), `BigIntCmp`/
//!   `RationalCmp` → `(<op> l r)` via `prim_operator`; `RationalOfInts`→`(Rational.of n d)`,
//!   `RationalOfIntWiden`→`(Rational.of-int n)`, `RationalNum`/`RationalDen`→`(Rational.numerator/
//!   denominator r)`; `BigIntToI64`→`(<IntType>.of <bigint>)` (the checked narrow, target from result type).
//!   A `Core::SumNew` with MULTIPLE payloads re-emits `(<Variant> p0 p1 …)`.
//! - **TRAP**: `Core::Trap` (the unconditional `unreachable`) → `(trap "")` — the message is dropped at
//!   lowering (textless wasm trap), so a placeholder round-trips. `TrapDivZero`/`TrapOverflow` (distinct
//!   trap kinds) still decline.
//! - **CONVERT**: `Core::Convert` → `((. <ResultType> <member>) operand)` — the target type is the result
//!   type, the member is `of-int` for `FloatOfInt` (int→float), `wrap` for the truncating total `Wrap`
//!   (never traps), else `of` (float-width / checked int-width). A non-numeric (boolean-coercion) Convert
//!   declines; a CHECKED int→int narrow declines on its range-check `Trap` (a `Wrap` narrow is total, no
//!   such `Trap`, and re-emits as `.wrap` — mapping it to `.of` would be a trap-vs-value miscompile).
//! - **STRING.CONCAT / NFC**: `String.concat` shares `Core::BytesConcat` (a String is a UTF-8 byte leaf),
//!   disambiguated from `Bytes.concat` by the result type; its compiler-inserted `Core::NfcNormalize` (no
//!   surface member) emits TRANSPARENTLY (its inner string), the surface `String.concat`/`to-bytes`
//!   re-inserting the normalization on recompile.
//! - **ORDERING**: the three-way compare prim (`Compare`, scalar `Core::Compare` or compound `ValueCmp`)
//!   re-emits the member `(Ordering.of l r)` — `compare` is namespaced as `Ordering.of`, NOT a bare name.
//! - **PROJ / VALUE-EQ / VALUE-CMP**: a runtime tuple projection `Core::Proj` → `(. <operand> <index>)`;
//!   structural equality `Core::ValueEq`/`ValueEqShaped` → `(= l r)` and structural ordering
//!   `Core::ValueCmp{op}` → `(<op> l r)` on runtime compounds (the operands' type re-selects the
//!   `value-eq`/`value-cmp` path vs a scalar `Compare` on recompile).
//! - **LIST OPS**: the runtime list operations, each `((. List <member>) <op>…)` — `List.len`/`push`/
//!   `prepend`/`concat`/`update`/`at` (`at` re-reads to its `Option` result). A constant-list op folds in
//!   `lower`, so a surviving node is a runtime op.
//! - **MAP/SET OPS**: the runtime collection operations, each a prelude member-access application
//!   `((. <Module> <member>) <op>…)` — `Map.insert`/`lookup`/`remove`/`len`(`MapSize`)/`to-list`,
//!   `Set.insert`/`remove`/`contains`/`len`(`SetLen`)/`to-list`/`union`/`intersection`/`difference`. Operands
//!   emit left-to-right; the member re-resolves to the same op on recompile.
//! - **EXPECT**: `Core::SumExpect` → `((. Option|Result expect) <scrutinee> "")` — the unwrap-or-trap
//!   accessor. The module is recovered from the scrutinee's sum-decl name; the `"message"` operand was
//!   dropped at lowering (the trap is textless), so a placeholder `""` re-emits (byte-idempotent, and
//!   value-equivalent — present → the payload, absent → the same trap).
//!
//! Now EMITTED (this comment was long stale): closures (`Closure`/`Captured`/`CallClosure`, #5108),
//! `Bytes.of` / `Str` ops (#5020/#5032), the full SUM decision-tree family (guarded / literal-test /
//! nested-switch / default arms, #5146/#5155/#5159/#5184), multi-argument variant constructors (#5044/
//! #5063), the Qty-return control-flow family (If/Let/Match*, #5341/#5355/#5362), and runtime binary
//! CONSTRUCTION `(bin (uNN v)…)` (`BinBuild`, #5376).
//!
//! Still declining, for later increments: `HostCall` (a genuine host effect that does not erase),
//! sequencing (`Seq`/`Block`/`Break`), binary PATTERN reads (`BinIntRead`) + runtime bit-fields
//! (`BinBitsBuild`), the richer-sum LONG TAIL (multi-payload-OUTER nested-switch, deep literal-test),
//! non-scalar scalar-match probes, generic / open user sums, a Qty over a heap `BigInt`/`Rational`
//! magnitude (not boundary-representable), and runtime `Ast.print` / `Ast.encode`.

use crate::ast::{Builder, IntValue, Leaf, Radix, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::ty::Ty;
use std::collections::HashMap;

/// The in-scope binding environment threaded (as `&mut`) through an emit walk. Two DISJOINT namespaces,
/// both mapping an anonymous-in-Core binder to the SYNTHESIZED surface name this backend mints for it (the
/// source name was discarded at lowering, so names are minted DETERMINISTICALLY — the same Core always
/// yields the same names, so a re-emit round-trips):
/// - `lets`: a kept `let` binding's initializer occurrence (the `StructId` a `Core::LocalRef` resolves to)
///   → its synthesized name (by binding order, see [`synth_binding_name`]).
/// - `payloads`: a sum-match payload — keyed by `(scrutinee-node, path)` (the same key a `Core::SumPayload`
///   in an arm body carries) → the binder name a `Core::MatchSum` arm minted for that payload slot (see
///   [`synth_payload_name`]). A match payload binder is anonymous in Core (a body reads
///   `sum-payload(scrutinee)` at a path), so the arm mints one fresh name per payload slot and records it
///   here for the body; `next_payload` keeps names globally unique within the def so a nested match's
///   binders never shadow an outer match's.
#[derive(Default)]
struct BinderEnv {
    lets: HashMap<StructId, std::rc::Rc<str>>,
    payloads: HashMap<(StructId, Vec<crate::core::PathStep>), std::rc::Rc<str>>,
    /// The solved TYPE of a payload binder in `payloads` (SAME key), populated where cheaply known (the
    /// sum-match arm-loop's variant payload slot; a `build_arm_pat` leaf). Its ONLY use: a `Core::SumPayload`
    /// read whose binder type is an ERASED single-variant, single-payload newtype (`(type Box (Mk Int64))`)
    /// but whose OWN solved type is that newtype's INNER — the erasure ELIDES the newtype's `Payload` step,
    /// collapsing the inner-read's key onto the newtype binder's key, so the exact-binder lookup returns the
    /// newtype binder where the inner is required. Emit a newtype PEEL `(match <binder> ((<Ctor> x) x))` (the
    /// type-correct surface crossing) instead of the bare binder, which would recompile as the newtype where
    /// the inner scalar is needed (CDZ0203, e.g. a map-key `(Box.Mk n)` sub-pattern). A key ABSENT here → no
    /// peel (bare binder, prior behavior), so a missing / imprecise type degrades safely to today's emit.
    payload_tys: HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
    /// The set of effect NAMES a `Core::HostCall` performed while emitting the CURRENT definition's body.
    /// [`emit_def`] reads it after the body and, if non-empty, wraps the body in one `(host (E…) <body>)`
    /// delegation so each re-emitted perform `((. E o) …)` re-lowers to a host-delegated `HostCall` (else
    /// CDZ0401 "no home"). Cleared per definition. A generous per-def delegation scope is faithful — a
    /// HostCall effect is unhandled in-program, so delegating it over the whole def body shadows nothing.
    performed_effects: std::collections::HashSet<std::rc::Rc<str>>,
    /// The set of effect NAMES whose `(effect …)` declaration this backend re-emitted in the preamble (its
    /// `emit_effect_decl` returned `Some`). A `Core::HostCall` perform emits ONLY if its effect is here —
    /// else the decl could not round-trip (e.g. an op arrow with a non-copyable payload) and a bare
    /// `((. E o) …)` would recompile as `unbound name E`, so it DECLINES instead (the perform ⇔ decl
    /// coupling, mirroring a sum value ⇔ its `(type …)`). Set once per program (cloned into each def's env).
    emitted_effects: std::collections::HashSet<std::rc::Rc<str>>,
    next_payload: usize,
    /// The program's lambda-lifted lambdas (a cheap `Rc` copy of `layout.lifted`), so a `Core::Closure {
    /// code }` value resolves its lifted lambda by index and re-emits the surface `(fn (<params>) <body>)`.
    /// Set once per definition in [`emit_def`]; `None` (treated as empty) for a closure-free program.
    lifted: Option<std::rc::Rc<[crate::lower::LiftedLambda]>>,
    /// The captures of the lifted lambda whose body is CURRENTLY being emitted: a `Core::Captured { index }`
    /// read inside that body resolves `captures[index]` (the enclosing binding's binder occurrence) to its
    /// surface name — the closure re-captures it LEXICALLY, so the surface is just that name. Saved/restored
    /// around each `Core::Closure` body emit so a nested closure resolves its own captures.
    current_captures: Option<std::rc::Rc<[StructId]>>,
}

/// The deterministic synthesized surface name for the `i`th kept `let` binding encountered in an emit
/// walk. Positional (not derived from the binder's `StructId`, which differs between the two arenas of a
/// round-trip), so compile-then-recompile mints the SAME name for the structurally-same binding. The
/// `_cdz_let` prefix keeps it out of the way of ordinary source identifiers.
fn synth_binding_name(i: usize) -> std::rc::Rc<str> {
    format!("_cdz_let{i}").into()
}

/// The deterministic synthesized surface name for the `i`th sum-match PAYLOAD binder minted in an emit
/// walk (monotone across the whole def, so binders of a nested match never collide with an outer match's).
/// The `_cdz_m` prefix keeps it clear of source identifiers AND silences the unused-binding warning
/// (CDZ0306, "prefix with `_` to silence") for a payload slot the arm body never reads.
fn synth_payload_name(i: usize) -> std::rc::Rc<str> {
    format!("_cdz_m{i}").into()
}

/// Emit the binary-AST artifact for the program in `db` under `layout`. Reconstructs a Cadenza surface
/// tree `(do (def …)… (export …)…)` over the same reachable definition set (`layout.order`) the other
/// backends emit, then serializes it with the binary-AST codec.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    let mut b = Builder::new();

    // A top-level `(do …)` root: link unwraps a `do`/`module` root and contributes its children, so a
    // `(do <def>… <export>…)` re-reads to the same program `(module m <def>… <export>…)` would. Build
    // the head first (head-first order is the fleet-wide build convention the no-canon codec relies on).
    let do_head = b.name("do");
    let mut root_children = vec![do_head];

    // Emit the user-declared TYPE declarations FIRST — a user sum's value re-reads to `(: (V p) T)`,
    // which needs `(type T …)` in scope (a prelude sum like Option/Result is ambient, needs none).
    // `emit_type_decl` handles only a monomorphic, closed, multi-variant sum (generic / open / single-
    // variant-erased are declined at the value site); the set of decls that landed gates which sum values
    // may emit, so the two agree (a value emits ⇔ its decl was emitted — no unbound-type recompile).
    let mut emitted: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    for i in 0..db.type_decls.len() {
        let decl = db.type_decls[i].clone();
        if db.is_user_node(decl.occ)
            && let Some(node) = emit_type_decl(db, &mut b, &decl)
        {
            root_children.push(node);
            emitted.insert(decl.occ);
        }
    }

    // Then the user EFFECT declarations — a host-delegated perform `((. E o) …)` (a re-emitted
    // `Core::HostCall`) needs `(effect E (op o …))` in scope to re-lower to the same HostCall (as a sum
    // value needs its `(type …)`). `emitted_effects` records which effects' decls landed — a perform of an
    // effect whose decl could NOT be re-emitted (a non-copyable op arrow) DECLINES rather than emit an
    // unbound-effect surface.
    let mut emitted_effects: std::collections::HashSet<std::rc::Rc<str>> =
        std::collections::HashSet::new();
    for i in 0..db.effect_decls.len() {
        let decl = db.effect_decls[i].clone();
        if db.is_user_node(decl.occ)
            && let Some(node) = emit_effect_decl(db, &mut b, &decl)
        {
            root_children.push(node);
            emitted_effects.insert(decl.name.as_str().into());
        }
    }

    // The lambda-lifted lambdas, shared (by `Rc`) into each definition's binder environment so a
    // `Core::Closure { code }` body resolves its lifted lambda by index. Empty for a closure-free program.
    let lifted: std::rc::Rc<[crate::lower::LiftedLambda]> = layout.lifted.clone().into();

    // One `(def …)` per reachable definition, in layout order (a stable, target-neutral order).
    for &def in &layout.order {
        let dn = db.defs[def].name.clone();
        root_children.push(
            emit_def(db, &mut b, def, &emitted, &emitted_effects, &lifted)
                .map_err(|e| with_def_context(e, &dn))?,
        );
    }

    // Then the exports, so recompiling the emitted program reaches the SAME definition set (an export-
    // less program would close to an empty `layout.order` and re-emit `(do)` — not idempotent). Emit in
    // the same layout order for determinism.
    for &def in &layout.order {
        if let Some(e) = layout.export_plan(def) {
            root_children.push(emit_export(db, &mut b, def, e)?);
        }
    }

    let root = b.list(root_children);
    let arenas = b.finish(root);
    Ok(crate::codec::encode(&arenas))
}

/// Emit a NO-EXPORT `(do (def …)…)` FRAGMENT of ONLY the definitions whose source name is in `subset` —
/// the building block of the two-stage standalone test-shred (v-cdz-crate-split): a shared-closure library
/// and each per-test body lower to separate content-addressable fragments that a later splice concatenates
/// (`[closure-defs ++ test-defs ++ (export test-main)]`), so the closure lowers ONCE and each per-test build
/// pays only its own body.
///
/// Unlike [`emit`], this does NOT run the reachable-from-export DCE and emits NO `(export …)`: it emits
/// EXACTLY the named defs (no transitive expansion — a per-test fragment names just its `@test` def; the
/// closure it calls is a SEPARATE fragment, referenced by name via the ordinary `Core::Call` surface and
/// resolved at splice time). The FULL program must still be in scope (`db`/`layout` from a normal compile,
/// so the subset's bodies name-resolve + type against the closures they call, and `layout.lifted` carries
/// their lifted lambdas) — the caller passes the whole suite (its `@test`/`export` roots make `layout`
/// non-empty), and this picks the named subset out of it.
///
/// Iterating `layout.order` (not `subset` insertion order) keeps the emit order deterministic, so identical
/// (program, subset) input → BYTE-IDENTICAL output — the content-stability the CA-cache keys on.
///
/// `include_type_decls`: emit the user `(type …)` declarations too. Set it for the ONE fragment that owns
/// the shared decls (the closure) and clear it for the others, so the spliced program carries each `(type
/// …)` exactly once (a duplicate decl would re-define on recompile).
pub fn emit_fragment(
    db: &mut Db,
    layout: &Layout,
    subset: &std::collections::HashSet<String>,
    include_type_decls: bool,
) -> Result<Vec<u8>, Reject> {
    let mut b = Builder::new();
    let do_head = b.name("do");
    let mut root_children = vec![do_head];

    // Populate `emitted` with EVERY user `(type …)` this backend can emit — REGARDLESS of `include_type_decls`.
    // In the two-stage shred the CLOSURE fragment (`include_type_decls`) carries the decl NODES and the per-test
    // fragments do NOT (each per-test program SPLICES against the closure, which already declares the types). But
    // a per-test def that CONSTRUCTS or MATCHES a user sum/nominal VALUE still needs the decl marked emitted so
    // its ctor-referencing surface is allowed — the SPLICED program has the decl (from the closure), so it round-
    // trips. Without the mark, a per-test fragment declined every sum value/match "…`(type …)` declaration is not
    // emitted" (v-test-shred: the 3 LARGEST compiler-ml two-stage decline classes, all `emitted`-empty). So MARK
    // for all; PUSH the node only when this fragment OWNS the decls (`include_type_decls`). A decl this backend
    // CANNOT emit (open/empty/unrenderable payload) stays OUT of `emitted` in BOTH fragments (the closure omits it
    // too), so a value over it still declines — consistent with the whole-program `emit`.
    let mut emitted: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    for i in 0..db.type_decls.len() {
        let decl = db.type_decls[i].clone();
        if !db.is_user_node(decl.occ) {
            continue;
        }
        if include_type_decls {
            if let Some(node) = emit_type_decl(db, &mut b, &decl) {
                root_children.push(node);
                emitted.insert(decl.occ);
            }
        } else {
            // Mark-only: reuse the exact emittability test via a THROWAWAY builder (the decl node comes from
            // the closure fragment at splice time, so it must NOT be pushed here — that would double-declare it).
            let mut scratch = Builder::new();
            if emit_type_decl(db, &mut scratch, &decl).is_some() {
                emitted.insert(decl.occ);
            }
        }
    }

    // Effect decls, mirroring the type-decl treatment: PUSH the `(effect …)` node when this fragment OWNS
    // the decls (`include_type_decls`), else MARK-only (the closure fragment carries it at splice time).
    // `emitted_effects` gates which HostCall performs may emit (perform ⇔ decl, as the whole-program `emit`).
    let mut emitted_effects: std::collections::HashSet<std::rc::Rc<str>> =
        std::collections::HashSet::new();
    for i in 0..db.effect_decls.len() {
        let decl = db.effect_decls[i].clone();
        if !db.is_user_node(decl.occ) {
            continue;
        }
        if include_type_decls {
            if let Some(node) = emit_effect_decl(db, &mut b, &decl) {
                root_children.push(node);
                emitted_effects.insert(decl.name.as_str().into());
            }
        } else {
            let mut scratch = Builder::new();
            if emit_effect_decl(db, &mut scratch, &decl).is_some() {
                emitted_effects.insert(decl.name.as_str().into());
            }
        }
    }

    let lifted: std::rc::Rc<[crate::lower::LiftedLambda]> = layout.lifted.clone().into();
    // ONLY the named subset, in `layout.order` (deterministic) — NO exports (added at splice time).
    for &def in &layout.order {
        if subset.contains(&db.defs[def].name) {
            let dn = db.defs[def].name.clone();
            root_children.push(
                emit_def(db, &mut b, def, &emitted, &emitted_effects, &lifted)
                    .map_err(|e| with_def_context(e, &dn))?,
            );
        }
    }

    let root = b.list(root_children);
    let arenas = b.finish(root);
    Ok(crate::codec::encode(&arenas))
}

/// Reconstruct a user EFFECT's `(effect <Name> (op <o> (-> <Domain> <Result>))…)` declaration so a
/// re-emitted host-delegated perform `((. E o) …)` re-lowers to the same `Core::HostCall` (the effect
/// name + op signature must be in scope on recompile, exactly as a sum's `(type …)` must be). Each op's
/// arrow type is copied structurally from its declaration occurrence (`OpDecl.ty`) via
/// [`emit_type_surface`], like a variant payload. `None` if any op lacks a written arrow type (a malformed
/// decl the recompile could not re-type). A `@resource`-marked op re-emits its bare arrow here (the marker
/// is a hash-clean decl-sibling recovered on re-lower; a resource op that needs the marker for round-trip
/// is a later refinement).
fn emit_effect_decl(
    db: &mut Db,
    b: &mut Builder,
    decl: &crate::db::EffectDecl,
) -> Option<StructId> {
    let effect_head = b.name("effect");
    let name_node = b.name(decl.name.as_str());
    let mut children = vec![effect_head, name_node];
    for op in &decl.ops {
        let op_head = b.name("op");
        let op_name = b.name(op.name.as_str());
        let ty_node = emit_type_surface(db, b, op.ty?)?;
        children.push(b.list(vec![op_head, op_name, ty_node]));
    }
    Some(b.list(children))
}

/// Reconstruct a user sum's `(type <Name> (<Variant> <PayloadTy>…)…)` declaration, or `None` for a sum
/// this slice does not emit: an OPEN sum (row-variable tail). A MULTI-variant sum's values are `Ty::Sum`; a
/// SINGLE-variant sum's values are the erased `Ty::Nominal` newtype (re-emitted as `(<Ctor> <payload>)`) —
/// BOTH need this decl in scope, so both are emitted. An EMPTY sum (`(type V)`, ZERO variants, uninhabited)
/// is ALSO emitted — as the bare `(type V)`: no value ever flows through it, but it is a valid CLOSED type
/// that a live sum may carry as a payload (`(type W (Ok Int64) (Bad V))`), so its declaration must be in
/// scope for the dependent decl to re-resolve on recompile (else `unknown type V`). A variant's payload
/// types are recovered from their declaration occurrences via `typeval_of` + lower's `type_ast`; a nullary
/// variant is `(<Variant>)`. `decl` is an owned clone (so `typeval_of`'s `&mut db` does not alias a
/// `db.type_decls` borrow).
fn emit_type_decl(db: &mut Db, b: &mut Builder, decl: &crate::db::TypeDecl) -> Option<StructId> {
    // Emit any CLOSED sum, including the ZERO-variant (empty / uninhabited) sum as a bare `(type V)`. A
    // GENERIC sum (`decl.params` non-empty) is handled: its head is `(<Name> p0 p1…)` and a bare
    // type-parameter payload re-emits its name. An OPEN sum (row tail) still declines (its `.. r` surface
    // is a later slice).
    if decl.open_tail.is_some() {
        return None;
    }
    let type_head = b.name("type");
    // The type NAME position: bare `Name` for a monomorphic sum, or `(Name p0 p1…)` for a generic one (the
    // params are the sum's type parameters in first-appearance order, `(type (Box a) …)`).
    let name_node = if decl.params.is_empty() {
        b.name(decl.name.as_str())
    } else {
        let mut head = vec![b.name(decl.name.as_str())];
        for p in &decl.params {
            head.push(b.name(p.as_str()));
        }
        b.list(head)
    };
    let mut children = vec![type_head, name_node];
    for v in &decl.variants {
        let vname = b.name(v.name.as_str());
        let mut vchildren = vec![vname];
        for &p in &v.payloads {
            // Re-emit the payload's SOURCE type-surface directly (a structural copy — see
            // [`emit_type_surface`]). This reproduces a BARE type parameter (`a`) or concrete name
            // (`Int64`), AND a COMPOUND carrying a param (`(Vec3 a)`, `(List (PathSeg a))`) — the last of
            // which the previous `typeval_of`+`type_ast` path could not render (a param is a `Ty::Var`,
            // `type_ast` → `None`), which declined the whole generic sum and gated the cad / compiler-ml
            // generic-sum coverage for the two-stage shred (v-test-shred). Copying the source surface is
            // both simpler and avoids `type_ast` partial-building into `b` on a `None` return.
            let ty_node = emit_type_surface(db, b, p)?;
            vchildren.push(ty_node);
        }
        children.push(b.list(vchildren));
    }
    Some(b.list(children))
}

/// Re-emit a TYPE SURFACE (a variant payload's declared type) by structurally copying the source AST at
/// `occ` into the fresh builder `b`: a bare NAME atom (`a` / `Int64` / `Vec3`) → the name; an application
/// `(Head arg…)` (`(Vec3 a)`, `(List (PathSeg a))`) → each child re-emitted recursively. Unlike
/// `typeval_of` + [`crate::lower::type_ast`], this faithfully reproduces a type PARAMETER nested inside a
/// compound (a param is a `Ty::Var` that `type_ast` cannot render), which is what a generic sum's
/// compound payload needs. `None` for a non-name, non-application leaf (an int/symbol in a type surface —
/// e.g. the width in `(Int 24)` — is a rare later slice; declining it declines the whole decl, safe).
fn emit_type_surface(db: &Db, b: &mut Builder, occ: StructId) -> Option<StructId> {
    if let Some(nm) = db.ast.as_name(occ) {
        return Some(b.name(nm));
    }
    match db.ast.get(occ) {
        crate::ast::Struct::List(items) => {
            let items = items.clone();
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(emit_type_surface(db, b, it)?);
            }
            Some(b.list(out))
        }
        crate::ast::Struct::Atom(_) => None,
    }
}

/// Reconstruct `(def (<name> (: <p> <Ty>)…) <body>)` for definition `def`. B1a handles NULLARY defs and
/// parameterized defs whose parameters have a value-form-representable type; a parameter of a type with
/// no surface (a function/continuation/unsolved type — `type_ast` returns `None`) declines.
/// Prepend the owning def's name to a DECLINE's message so a `--target cadenza` / two-stage-shred emit
/// failure names WHICH def declined (the class message alone — e.g. "payload projection over a
/// non-tuple/record value" — does not say which def, which blocks per-def attribution + minimal-witness
/// reduction downstream, v-test-shred). A CODED rejection (`code.is_some()`) is already anchored to a node,
/// so it is left as-is; the prefix guard keeps a re-wrap from double-prefixing.
fn with_def_context(mut e: Reject, def_name: &str) -> Reject {
    if e.code.is_none() && !e.message.starts_with("def `") {
        e.message = format!("def `{def_name}`: {}", e.message);
    }
    e
}

fn emit_def(
    db: &mut Db,
    b: &mut Builder,
    def: usize,
    emitted: &std::collections::HashSet<StructId>,
    emitted_effects: &std::collections::HashSet<std::rc::Rc<str>>,
    lifted: &std::rc::Rc<[crate::lower::LiftedLambda]>,
) -> Result<StructId, Reject> {
    let name = db.defs[def].name.clone();
    let body = db.defs[def].body.ok_or_else(|| {
        Reject::decline(format!(
            "definition `{name}` has no body to lower to Cadenza"
        ))
    })?;

    // Build the signature `(<name> (: <p0> <Ty0>) …)`. `def_params` returns each parameter's binder
    // occurrence (the identity a `Core::Param` reference resolves to) paired with its SOLVED type. The
    // parameter's surface name is read off that binder occurrence; its type ascription reuses lower's
    // canonical `type_ast` so the re-emitted `(: p Ty)` is byte-identical to the type surface everything
    // else in the program uses (round-trip identity). `def_params` returns an owned Vec, so it is taken
    // FIRST (a `&mut db`), before the immutable `name_ctx()` borrow the type rendering needs.
    let params = crate::layout::def_params(db, def);
    let def_head = b.name("def");
    let sig_name = b.name(name.as_str());
    let mut sig_children = vec![sig_name];
    {
        // Within this scope only the immutable `NameCtx` (a `&db` borrow) and the builder are used — no
        // `&mut db` — so the parameter name reads (`as_name`) and `type_ast` calls compose.
        let ncx = db.name_ctx();
        for (binder, ty) in params.iter() {
            let pname = db.ast.as_name(*binder).ok_or_else(|| {
                Reject::decline(format!(
                    "the Cadenza backend cannot recover a parameter name for `{name}`"
                ))
            })?;
            let pname_node = b.name(pname);
            let ty_node = crate::lower::type_ast(b, ty, &ncx).ok_or_else(|| {
                Reject::unsupported(format!(
                    "the Cadenza backend does not support lowering a parameter of type `{}` (`{name}`) — no \
                     value-form type surface (a function / unsolved type)",
                    ty.render_name(&ncx)
                ))
            })?;
            // `(: <pname> <Ty>)` — the ascription head `:` is a Name atom, matching the surface reader
            // and `type_ast`'s own record-field ascriptions.
            let colon = b.name(":");
            sig_children.push(b.list(vec![colon, pname_node, ty_node]));
        }
    }
    let sig = b.list(sig_children);
    // A fresh binding environment per definition — a `let` / match arm in the body populates it; the
    // program's lifted lambdas are shared in so a `Core::Closure` body can resolve its lifted lambda.
    let mut env = BinderEnv {
        lifted: Some(lifted.clone()),
        emitted_effects: emitted_effects.clone(),
        ..BinderEnv::default()
    };
    // A def whose RESULT type is a concrete `Ty::Qty` AND whose body reduces to a BARE-INNER runtime
    // magnitude re-emits the quantity wrapper around the WHOLE body at the tail: `(def (main …) (Qty.of
    // (* n 2) u))` erases (`Qty.of` drops its compile-time unit) to a bare arithmetic node typed `Ty::Qty`,
    // so re-inserting `((. Qty of) <magnitude> <unit>)` reconstructs the genuine quantity return
    // (recompiling folds it back → value-eq + byte-idempotent). [`qty_leaf`] classifies the body's value
    // leaves: a `Core::Arith`/`Convert`/tower-binop is a bare-inner magnitude, and `If`/`Let` thread to
    // their tail leaves (a `Trap` diverges, neutral). WRAP-WHOLE fires only when EVERY value leaf is
    // bare-inner — this covers a computed magnitude AND a CHECKED-NARROW `(Qty.of (Int32.of n) u)` (which
    // erases to `(if range (trap) ((. Int32 wrap) n))`, whose bare-inner leaf would otherwise emit
    // unwrapped, silently DROPPING the Qty — a dual-path value miscompile; see `qty_leaf`).
    //
    // The escape type is the def RESULT type (`def_result_ty`), NOT the body node's own solved type — an
    // erased `Qty.value` peel (`(Qty.value (+ q r))`) leaves the same magnitude-typed-`Ty::Qty` body but
    // its result type is the bare inner numeric, so it is NOT matched here and still declines. A body whose
    // leaves are Param binders (`(if c (Qty.of x u)(Qty.of y u))` over inner params) is NOT bare-inner →
    // falls through to the normal emit, where each leaf SELF-constructs via `qty_disposition` (wrapping the
    // whole would DOUBLE-wrap). Restricting WRAP-WHOLE to the def tail keeps it from firing inconsistently
    // across nested match-arm / collection-element siblings (the `18-units-of-measure/0225` CDZ0203 hazard).
    let body_node = match def_result_ty(db, def, params.len()) {
        Some(Ty::Qty { inner, unit }) if qty_leaf(db, body) == LeafKind::BareInner => {
            let head = member_access(b, "Qty", "of");
            let mag =
                emit_expr_viewed(db, b, body, Some((*inner).clone()), None, &mut env, emitted)?;
            let unit_node = crate::lower::unit_value_ast(b, &unit);
            b.list(vec![head, mag, unit_node])
        }
        // A partial-bare-inner control-flow tail (a checked-narrow arm mixed with a Param/other arm) can be
        // neither wrapped-whole (the Param arm self-constructs → double-wrap) nor passed through (the bare-inner
        // arm silently drops its wrapper — the #5341 miscompile in an arm position). Decline it (a uniform-arm
        // reconstruction is a later slice); the direct-wasm path is what the corpus grades against.
        Some(Ty::Qty { .. }) if qty_leaf(db, body) == LeafKind::Mixed => {
            return Err(Reject::unsupported(
                "the Cadenza backend does not support re-emitting a quantity return whose control-flow arms MIX a \
                 bare-magnitude (e.g. checked-narrow) arm with a constructing arm"
                    .to_string(),
            ));
        }
        _ => emit_expr(db, b, body, None, &mut env, emitted)?,
    };
    // If the body PERFORMED any host-delegated effect (a `Core::HostCall`), wrap it in ONE
    // `(host (E1 E2 …) <body>)` delegation so each re-emitted perform `((. E o) …)` re-lowers to a
    // host-delegated `HostCall` on recompile (else CDZ0401 "no home"). Effects are emitted in SORTED order
    // for a deterministic (idempotent) surface. A generous whole-def-body scope is faithful — a HostCall
    // effect is unhandled in-program, so delegating it over the body shadows no handler.
    let body_node = if env.performed_effects.is_empty() {
        body_node
    } else {
        let mut names: Vec<std::rc::Rc<str>> = env.performed_effects.iter().cloned().collect();
        names.sort();
        let host_head = b.name("host");
        let effect_names: Vec<StructId> = names.iter().map(|n| b.name(n.clone())).collect();
        let effects_list = b.list(effect_names);
        b.list(vec![host_head, effects_list, body_node])
    };
    Ok(b.list(vec![def_head, sig, body_node]))
}

/// How a value leaf of a `Ty::Qty`-result def body classifies for the tail WRAP-WHOLE decision in
/// [`emit_def`] — see there for the miscompile it fixes.
#[derive(PartialEq, Clone, Copy)]
enum LeafKind {
    /// A bare-INNER runtime magnitude producer (`Arith`/`Convert`/tower binop) — emits the bare inner value,
    /// so the WHOLE body must be wrapped `(Qty.of … u)`.
    BareInner,
    /// A `Trap` — diverges, produces no value; NEUTRAL when merging sibling arms.
    Diverges,
    /// A leaf that SELF-constructs or declines — a `Param`/`LocalRef` (wraps via its own `qty_disposition`
    /// Construct), a const, a `Call`, a compound — keep the body on the normal pass-through/decline path;
    /// do NOT wrap-whole (wrapping a Param leaf would double-wrap).
    Other,
    /// A control-flow body whose leaves MIX `BareInner` and `Other` (`(if c (Qty.of (Int32.of a) u) (Qty.of x
    /// u))` — a checked-narrow arm + a Param arm). Wrap-whole is wrong (the Param arm self-constructs → would
    /// double-wrap) and the normal pass-through drops the bare-inner arm's wrapper (the #5341 miscompile in an
    /// arm position) → [`emit_def`] DECLINES it (decline-don't-miscompile; a uniform-arm slice is later).
    Mixed,
}

/// Classify a `Ty::Qty`-result def body by its value LEAVES, threading through ALL control flow
/// (`If`/`Let`/`Match`/`MatchList`/`MatchSum`) to the tail positions. `BareInner` iff every non-diverging
/// leaf is a bare-inner magnitude (so the whole body is a magnitude to wrap-whole); `Other` if any leaf
/// self-constructs / declines (`Param`/const/`Call`/compound); `Mixed` on a `BareInner`+`Other` split
/// (which [`emit_def`] declines). Covering `Match*` closes the same wrapper-drop miscompile [`emit_def`]
/// fixes for `If`/`Let`, when a match arm's body is a bare-inner magnitude (e.g. a checked-narrow).
fn qty_leaf(db: &mut Db, id: StructId) -> LeafKind {
    match core_of(db, id) {
        Core::Arith { .. }
        | Core::Convert { .. }
        | Core::BigIntBinOp { .. }
        | Core::RationalBinOp { .. }
        | Core::BigIntOfI64 { .. } => LeafKind::BareInner,
        Core::Trap => LeafKind::Diverges,
        Core::If { then_, else_, .. } => merge_leaf(qty_leaf(db, then_), qty_leaf(db, else_)),
        Core::Let { body, .. } => qty_leaf(db, body),
        // A match: the value leaves are the arm bodies (the guard/scrutinee are not value positions).
        Core::Match { arms, .. } => {
            let mut acc = LeafKind::Diverges;
            for a in arms.iter() {
                acc = merge_leaf(acc, qty_leaf(db, a.body));
            }
            acc
        }
        Core::MatchList { arms, .. } => {
            let mut acc = LeafKind::Diverges;
            for a in arms.iter() {
                acc = merge_leaf(acc, qty_leaf(db, a.body));
            }
            acc
        }
        Core::MatchSum { root, .. } => qty_cont_leaf(db, &root),
        _ => LeafKind::Other,
    }
}

/// Fold [`qty_leaf`] over every body position of a sum-match continuation tree (the `MatchSum` twin of the
/// arm-body walk in [`qty_leaf`]): a `Leaf` body, a `Guarded`/`LitTest` body threaded with its `els`
/// fall-through, and each `Switch` arm's nested cont.
fn qty_cont_leaf(db: &mut Db, cont: &crate::core::SumCont) -> LeafKind {
    use crate::core::SumCont;
    match cont {
        SumCont::Leaf(body) => qty_leaf(db, *body),
        SumCont::Guarded { body, els, .. } => {
            merge_leaf(qty_leaf(db, *body), qty_cont_leaf(db, els))
        }
        SumCont::LitTest { then_, els, .. } => {
            merge_leaf(qty_cont_leaf(db, then_), qty_cont_leaf(db, els))
        }
        SumCont::Switch { arms, .. } => {
            let mut acc = LeafKind::Diverges;
            for a in arms.iter() {
                acc = merge_leaf(acc, qty_cont_leaf(db, &a.cont));
            }
            acc
        }
    }
}

/// Merge two sibling-arm [`LeafKind`]s: `Diverges` is neutral (a trapping arm carries no value); two
/// `BareInner` stay `BareInner`; two `Other` stay `Other`; a `BareInner`+`Other` split (or anything already
/// `Mixed`) is `Mixed` (a partial-bare-inner control flow that must decline, not silently drop an arm).
fn merge_leaf(a: LeafKind, b: LeafKind) -> LeafKind {
    match (a, b) {
        (LeafKind::Diverges, x) | (x, LeafKind::Diverges) => x,
        (LeafKind::Mixed, _) | (_, LeafKind::Mixed) => LeafKind::Mixed,
        (LeafKind::BareInner, LeafKind::BareInner) => LeafKind::BareInner,
        (LeafKind::Other, LeafKind::Other) => LeafKind::Other,
        // one BareInner + one Other → a mixed control flow.
        _ => LeafKind::Mixed,
    }
}

/// The def's RESULT type — its inferred scheme type ([`crate::infer::def_scheme`]) with `n_params`
/// parameter arrows (`Ty::Fn`) stripped off. `None` when the scheme is unavailable or the arrow spine is
/// shorter than the parameter count (a malformed / under-determined signature). This is the body's ESCAPE
/// type, distinct from the body NODE's own solved type when a unit-erasing op (`Qty.value`) left the node
/// carrying `Ty::Qty` while the real escape is the bare inner numeric — see the Qty tail re-emit in
/// [`emit_def`].
fn def_result_ty(db: &mut Db, def: usize, n_params: usize) -> Option<Ty> {
    let mut ty = crate::infer::def_scheme(db, def)?.ty;
    for _ in 0..n_params {
        match ty {
            Ty::Fn(_, ret) => ty = (*ret).clone(),
            _ => return None,
        }
    }
    Some(ty)
}

/// Reconstruct `(export <name>)` for an exported definition. B0 handles only an export whose boundary
/// name equals the definition's source name (an unrenamed export); a renamed export (`export … as …`)
/// declines, since dropping the rename would not round-trip.
fn emit_export(
    db: &mut Db,
    b: &mut Builder,
    def: usize,
    e: &crate::layout::ExportPlan,
) -> Result<StructId, Reject> {
    let source_name = db.defs[def].name.clone();
    if e.name != source_name {
        return Err(Reject::unsupported(format!(
            "the Cadenza backend does not support lowering a RENAMED export (`{source_name}` exported as \
             `{}`) — B0 emits unrenamed exports only",
            e.name
        )));
    }
    let export_head = b.name("export");
    let name_ref = b.name(source_name.as_str());
    Ok(b.list(vec![export_head, name_ref]))
}

/// Reconstruct a Cadenza surface expression from the optimized `Core` at `id`. Delegates to
/// [`emit_expr_viewed`] with no type view (the node's own solved type governs). See it for `expected`.
fn emit_expr(
    db: &mut Db,
    b: &mut Builder,
    id: StructId,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    emit_expr_viewed(db, b, id, None, expected, env, emitted)
}

/// The body of [`emit_expr`], parameterized by an optional type `view` that OVERRIDES the node's own solved
/// type for its OWN-type decisions — the user-sum/nominal guard and the constant-scalar arms (which pick the
/// literal-vs-wrapper surface by type). `view` is `Some` only when a NEWTYPE value is being peeled: an erased
/// single-variant sum has solved type `Ty::Nominal { inner }` but its core IS the bare payload (a `ConstInt`
/// typed as the nominal, a `Param`, …). To re-emit `(Mk <payload>)`, the guard recurses on the SAME node with
/// `view = Some(inner)` so the payload is emitted AS its inner scalar type (the `ConstInt` arm fires on
/// `Ty::Int`, not on the nominal); a nested newtype peels again. The 29 recursive child emits call the
/// `view = None` wrapper [`emit_expr`], so only this node's own-type reads consult `view`.
///
/// `expected` is the type this expression is REQUIRED to have by its surrounding context (the branch/body
/// position it occupies) — `Some` when a container passed one down (an `if` gives its branches the join
/// type; a `let`/match body inherits the whole form's type), else `None`. It is a FALLBACK, used only where
/// the node's own solved type is under-determined: a nullary/partially-applied `Core::SumNew` whose solved
/// type has a FREE type argument (`(None)` in `(if c (Some 1) (None))`, whose own type is `Option<?>`) reads
/// the CONCRETE join type from `expected` to ascribe `(: (None unit) (Option Int64))` — otherwise it would
/// decline. Threaded only to value/tail positions (branches, bodies); operand/scrutinee/guard positions
/// pass `None` (they impose no outer type). Passing `None`/`None` everywhere reproduces pre-thread behavior.
fn emit_expr_viewed(
    db: &mut Db,
    b: &mut Builder,
    id: StructId,
    view: Option<Ty>,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    // The EFFECTIVE type for this node's own-type decisions: the `view` override when peeling a newtype,
    // else the node's own solved type. Used by the sum/nominal guard below and the constant-scalar arms.
    let eff_ty = match view {
        Some(v) => v,
        None => crate::infer::type_of(db, id),
    };
    // A value whose solved type is a USER-declared sum/nominal round-trips only if its `(type …)`
    // declaration is re-emitted. `emit` emits (and records in `emitted`) the declarations it can — a
    // MONOMORPHIC, CLOSED sum (multi-variant → `Ty::Sum` values; SINGLE-variant → erased `Ty::Nominal`
    // values). A user-sum/nominal value whose decl IS in `emitted` proceeds; otherwise DECLINE (a GENERIC
    // / OPEN sum, no decl emitted). Prelude sums (Option/Result — `is_user_node` false) are ambient and
    // always proceed. (breaker-reported; decline-don't-miscompile.)
    match &eff_ty {
        // A TYPE-VALUE (`Ty::Type`) — a first-class type used as a runtime value (`(let ((t Int64)) t)`,
        // `(: Int64 Type)`). Its Core is intentionally ERASED (a type is FULLY compile-time-known, not a
        // runtime value, so the erased core is a `Poison` — `lower/value_form.rs:1998`), so decide by the
        // TYPE, not the core: recover the concrete `Ty` with `eval::typeval_of` (the same reducer the SHARED
        // value-form renderer bakes the boundary form with at `value_form.rs:1999`) and re-emit the explicit
        // type-value ascription `(: <type-surface> Type)` — the recompilable source form (the corpus program
        // `(: Int64 Type)`), `type_ast` rendering the type surface. SAFE discriminator (confirmed by
        // v-metaprogramming, the type-reflection owner): keyed on `Ty::Type` AND `typeval_of` SUCCESS, which
        // holds ONLY for a genuinely compile-time-known type-value — a real rejection has a non-`Type` type or
        // `typeval_of` fails, so this never misfires on a carried `Poison`. A parameterized / not-fully-
        // determined type has no boundary form → `typeval_of` / `type_ast` yield `None` → falls through to the
        // core match and declines (correct — per the 07-type-system corpus).
        Ty::Type => {
            if let Some(concrete) = crate::eval::typeval_of(db, id) {
                let ncx = db.name_ctx();
                if let Some(ty_surface) = crate::lower::type_ast(b, &concrete, &ncx) {
                    let colon = b.name(":");
                    let type_kw = b.name("Type");
                    return Ok(b.list(vec![colon, ty_surface, type_kw]));
                }
            }
        }
        Ty::Sum { decl, .. } if db.is_user_node(*decl) && !emitted.contains(decl) => {
            return Err(Reject::unsupported(
                "the Cadenza backend does not support re-emitting a generic / open user sum value; only a \
                 monomorphic, closed sum re-emits its `(type …)` declaration"
                    .to_string(),
            ));
        }
        // An ERASED single-variant sum (`Ty::Nominal`) — the newtype box is erased, so a nominal value and
        // its inner value SHARE one Core node; the surface `(<Ctor> …)` must be re-inserted at EXACTLY the
        // construction sites (where an inner value becomes the nominal), which the erased Core no longer
        // marks. [`nominal_disposition`] classifies THIS node: a CONSTRUCTION site (a leaf/operator that
        // yields the inner value — a literal, arithmetic, or a binder whose declared type is the inner) →
        // re-emit `(<Ctor> <payload>)`, the payload peeled to `inner` via a `view` recursion; a PASS-THROUGH
        // (control flow / a binder already holding the nominal — the `(<Ctor> …)` sits at the leaves inside,
        // which the child emits handle) → fall through and emit the core AS-IS (no wrap — wrapping it would
        // DOUBLE the constructor); anything else declines (an ambiguous site, e.g. a `Call` that could
        // return either the inner or the nominal — decline-don't-miscompile).
        Ty::Nominal { decl, inner, .. } if db.is_user_node(*decl) => {
            let decl = *decl;
            let inner = (**inner).clone();
            if !emitted.contains(&decl) {
                return Err(Reject::unsupported(
                    "the Cadenza backend does not support re-emitting this user nominal (newtype) value (its \
                     `(type …)` declaration is not emitted — a generic / open newtype)"
                        .to_string(),
                ));
            }
            match nominal_disposition(db, id, decl) {
                NominalDisp::Construct => {
                    let head = crate::lower::variant_head_ast(db, b, decl, 0).ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend could not recover the constructor name for a \
                                 newtype value"
                                .to_string(),
                        )
                    })?;
                    // A MULTI-payload single-variant sum (`(type P2 (Both Int64 Int64))`) erases its
                    // payload to a TUPLE (`inner` is `Ty::Tuple`), so wrapping that tuple as ONE argument
                    // (`(Both (tuple 5 6))`) is NON-re-compilable against the two-slot constructor
                    // (CDZ0201 — a tuple applied where two Int64 slots are declared). Emit the payload
                    // SLOT-WISE (`(Both 5 6)`) when the erased value is a statically-visible `Core::Tuple`
                    // literal of the declared arity; otherwise decline (a runtime tuple — a call/proj
                    // result — cannot be statically unpacked into the constructor's slots).
                    let arity = db
                        .type_decl_by_occ(decl)
                        .and_then(|t| t.variants.first())
                        .map(|v| v.payloads.len())
                        .unwrap_or(1);
                    if arity >= 2 {
                        let elems = match core_of(db, id) {
                            Core::Tuple { elems } if elems.len() == arity => elems,
                            _ => {
                                return Err(Reject::unsupported(
                                    "the Cadenza backend does not support re-emitting a multi-payload \
                                     newtype value whose erased payload is not a statically-visible \
                                     tuple literal of the declared arity"
                                        .to_string(),
                                ));
                            }
                        };
                        let slot_tys = match &inner {
                            Ty::Tuple(ts) => Some(ts.clone()),
                            _ => None,
                        };
                        let mut children = Vec::with_capacity(1 + elems.len());
                        children.push(head);
                        for (i, &e) in elems.iter().enumerate() {
                            let ex = slot_tys.as_ref().and_then(|ts| ts.get(i).cloned());
                            children.push(emit_expr(db, b, e, ex, env, emitted)?);
                        }
                        return Ok(b.list(children));
                    }
                    let payload = emit_expr_viewed(db, b, id, Some(inner), None, env, emitted)?;
                    return Ok(b.list(vec![head, payload]));
                }
                // PASS-THROUGH: fall through to the core match (emit the core unwrapped — its own
                // sub-values carry the nominal and re-insert the constructor at the true leaves).
                NominalDisp::PassThrough => {}
                NominalDisp::Decline => {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support re-emitting a newtype value from this \
                         construction site (an ambiguous inner-vs-nominal position)"
                            .to_string(),
                    ));
                }
            }
        }
        // A unit-bearing QUANTITY (`Ty::Qty`) — the unit is compile-time + ERASED, so the value's Core IS the
        // bare magnitude (a scalar typed `Qty`); the surface `((. Qty of) <mag> <unit>)` must be re-inserted
        // where the value genuinely escapes AS a quantity, like an erased nominal. `qty_disposition` (see it
        // for the erased-`Qty.value` trap) classifies conservatively: a CONSTRUCT site (a bare magnitude
        // binder — a `def` returning `(Qty.of <param> <unit>)`) → wrap `((. Qty of) <mag-peeled-to-inner>
        // <unit>)`, the unit reconstructed from the type's unit map via lower's `unit_value_ast` (the
        // canonical value form); a PASS-THROUGH (control flow / a binder already `Ty::Qty`) → emit the core
        // as-is (the wrap sits at the true leaves); anything else declines (decline-don't-miscompile).
        Ty::Qty { inner, unit } => {
            let inner = (**inner).clone();
            let unit = unit.clone();
            match qty_disposition(db, id) {
                NominalDisp::Construct => {
                    let head = member_access(b, "Qty", "of");
                    let mag = emit_expr_viewed(db, b, id, Some(inner), None, env, emitted)?;
                    let unit_node = crate::lower::unit_value_ast(b, &unit);
                    return Ok(b.list(vec![head, mag, unit_node]));
                }
                NominalDisp::PassThrough => {}
                NominalDisp::Decline => {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support re-emitting a quantity value from this \
                         construction site (an ambiguous magnitude-vs-quantity position)"
                            .to_string(),
                    ));
                }
            }
        }
        _ => {}
    }
    match core_of(db, id) {
        // A CONSTANT scalar leaf re-reads to its plain value+type when its solved type is the PLAIN scalar
        // type; a numeric-tower / nominal-leaf WRAPPER (`BigInt`/`Rational`/`Symbol`) shares a bare scalar
        // core (`ConstInt`/`ConstStr`/`ConstRational`) but re-reads to the WRAPPER only through its
        // CONSTRUCTOR surface — emitting the bare scalar would drop the type and MISCOMPILE the value (a
        // `Ty::Symbol` value came back a `String`, confirmed). So a plain scalar emits its literal and a
        // wrapper constant emits `(X.of …)`. (`Ty::Qty` — a scaled/unit-bearing wrapper — needs unit
        // reconstruction and still declines, a later slice.) `radix` is display-only (Core drops it).
        // A NON-default-Int64 integer constant must ASCRIBE its type: a bare integer literal re-grounds to
        // the DEFAULT signed `Int64` on recompile, so an UNSIGNED value (e.g. `UInt64`) or one BEYOND i64
        // range re-reads as out-of-range for Int64 (CDZ0201, "it fits `UInt64`; annotate it (: … UInt64)") —
        // a round-trip break the bare literal caused (breaker-confirmed: `(: 18446744073709551615 UInt64)`).
        // Emit the DIRECT ascription `(: <v> <IntTy>)` (mirrors the `BigInt` arm just below + the `ConstFloat`
        // non-64-width ascription). Scoped to the VERIFIED classes — unsigned OR value out of i64 range; an
        // in-range signed value keeps the bare literal via the next arm (default Int64 needs no ascription).
        Core::ConstInt(v) if matches!(&eff_ty, Ty::Int(it) if !it.ground_signed() || v.to_i64().is_none()) =>
        {
            // The ascription type is built by `int_module_ast` (NOT `render_name`): it yields the RECOMPILABLE
            // surface — a bare `UInt64`/`Int32` for a standard width, but the CTOR form `(UInt 48)` for a
            // non-standard width (a bare `UInt48` is an unbound name → CDZ0101). The corpus writes exactly
            // `(: <v> (UInt 48))`, so this round-trips at every width.
            let it = match &eff_ty {
                Ty::Int(it) => *it,
                _ => unreachable!("guarded Ty::Int"),
            };
            let colon = b.name(":");
            let n = b.atom_leaf(Leaf::Int {
                value: v,
                radix: Radix::Dec,
            });
            let ty = int_module_ast(b, it);
            Ok(b.list(vec![colon, n, ty]))
        }
        Core::ConstInt(v) if matches!(eff_ty, Ty::Int(_)) => Ok(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        // A BigInt constant is a `ConstInt` typed `Ty::BigInt` — re-emit the DIRECT ascription
        // `(: <n> BigInt)`, NOT `(BigInt.of <n>)`: `BigInt.of` WIDENS a fixed-size `Int64`, so it cannot
        // hold a beyond-`Int64` literal (`(BigInt.of 9223372036854775808)` fails CDZ0201 "out of range
        // for Int64 … write the literal directly as a BigInt with (: … BigInt)"). The ascription form
        // takes the literal directly as a BigInt and round-trips at every magnitude.
        Core::ConstInt(v) if matches!(eff_ty, Ty::BigInt) => {
            let colon = b.name(":");
            let n = b.atom_leaf(Leaf::Int {
                value: v,
                radix: Radix::Dec,
            });
            let ty = b.name("BigInt");
            Ok(b.list(vec![colon, n, ty]))
        }
        Core::ConstStr(s) if matches!(eff_ty, Ty::String) => Ok(b.atom_leaf(Leaf::Str(s))),
        // A Symbol constant shares a `ConstStr` core typed `Ty::Symbol` — re-emit `(Symbol.of "…")` so it
        // re-reads as a Symbol (a bare string would come back a `String`).
        Core::ConstStr(s) if matches!(eff_ty, Ty::Symbol) => {
            let head = member_access(b, "Symbol", "of");
            let text = b.atom_leaf(Leaf::Str(s));
            Ok(b.list(vec![head, text]))
        }
        // A BYTES constant re-emits its canonical value-form leaf `b"…"` (`Leaf::Bytes` — the raw bytes flow
        // through the shared `Arc<[u8]>` with no copy), the surface a byte sequence is written as; it
        // recompiles straight back to a `Core::ConstBytes` of the same bytes (the reader folds a `b"…"`
        // literal to `ConstBytes` in `lower`, the twin of the `ConstStr`↔`"…"` path above).
        Core::ConstBytes(bytes) if matches!(eff_ty, Ty::Bytes) => {
            Ok(b.atom_leaf(Leaf::Bytes(bytes)))
        }
        // A float constant. A bare decimal leaf grounds to the DEFAULT `Float64` on recompile, so a
        // `Float32` constant (or any non-64 width) must be ASCRIBED with its width — otherwise the value
        // CHANGES (`0.1` at `Float32` = `0.10000000149…` ≠ `0.1` at `Float64`, a value miscompile). A
        // `Float64` const emits the bare literal (the default width needs no ascription).
        Core::ConstFloat(d) if matches!(eff_ty, Ty::Float(_)) => {
            let width = match &eff_ty {
                Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            let lit = b.atom_leaf(Leaf::Float(d));
            if width == 64 {
                Ok(lit)
            } else {
                let tyname = eff_ty.render_name(&db.name_ctx());
                let colon = b.name(":");
                let ty_node = b.name(tyname.as_str());
                Ok(b.list(vec![colon, lit, ty_node]))
            }
        }
        // The non-finite Float constants re-emit their canonical WRITTEN value forms — the member-access
        // constants `(. Float<width> nan)` / `(. Float<width> Infinity)` (prelude.rs §float module: a
        // `float-nan` / `float-inf` intrinsic ANNOTATED with the module width), which fold straight back to
        // `Core::ConstFloatNan` / `Core::ConstFloatInf` on recompile. NOT a bare `Leaf::FloatNan`/`FloatInf`:
        // those are VALUE-RENDER / `Ast.encode` leaves the FRONT-END REFUSES in expression position
        // (`resolve.rs` poisons them → CDZ0201 "non-finite float value has no source literal form"), so a bare
        // leaf breaks the recompile contract as soon as the const survives to a runtime VALUE position (a
        // runtime-vs-const compare, not an all-const compare that folds away) — the adv-hop2 finding (breaker)
        // #7298 shipped. The per-width `Float<width>` module name carries the width, so this now handles EVERY
        // float width (the earlier `== 64` guard is subsumed). `Core::ConstFloatInf` is the POSITIVE infinity
        // (a negative ∞ is `(- Float<width>.Infinity)`).
        Core::ConstFloatNan if matches!(&eff_ty, Ty::Float(_)) => {
            let width = match &eff_ty {
                Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            Ok(member_access(b, &format!("Float{width}"), "nan"))
        }
        Core::ConstFloatInf if matches!(&eff_ty, Ty::Float(_)) => {
            let width = match &eff_ty {
                Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            Ok(member_access(b, &format!("Float{width}"), "Infinity"))
        }
        // An exact RATIONAL constant — its value-form `num/den` is not valid expression syntax, so
        // re-emit the CONSTRUCTOR `(Rational.of <num> <den>)` over the normalized pair. `Rational.of`
        // takes two `Int64` arguments, so a numerator/denominator BEYOND `Int64` cannot be expressed this
        // way (same limit as `BigInt.of`); such a constant DECLINES (a beyond-`Int64` rational literal
        // surface is a later slice) rather than emit a non-re-compilable `(Rational.of <huge> …)`.
        Core::ConstRational(n, d) if n.to_i64().is_some() && d.to_i64().is_some() => {
            let head = member_access(b, "Rational", "of");
            let num = b.atom_leaf(Leaf::Int {
                value: n,
                radix: Radix::Dec,
            });
            let den = b.atom_leaf(Leaf::Int {
                value: d,
                radix: Radix::Dec,
            });
            Ok(b.list(vec![head, num, den]))
        }
        // Bool / Char / Unit have no wrapping type, so they always emit their one literal form.
        Core::ConstBool(bo) => Ok(b.atom_leaf(Leaf::Bool(bo))),
        Core::ConstChar(c) => Ok(b.atom_leaf(Leaf::Char(c))),
        Core::Unit => Ok(b.name("unit")),
        // A reference to a function PARAMETER — its surface is the bare name of the parameter's binder
        // occurrence (a `Name`), which re-resolves to the same parameter on recompile.
        Core::Param { binder } => {
            let nm = db.ast.as_name(binder).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend cannot recover the name of a parameter reference"
                        .to_string(),
                )
            })?;
            Ok(b.name(nm))
        }
        // A reference to a kept `let` binding — its binder is the initializer occurrence (NOT a `Name`),
        // so its surface name comes from the environment the enclosing `Let` populated with the
        // synthesized binding name. A `LocalRef` always lives inside its `Let`'s body, so the binder is
        // in scope; an absent entry would be an emit bug (a `LocalRef` reached without its `Let`).
        Core::LocalRef { binder } => {
            let nm = env.lets.get(&binder).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend reached a `let`-binding reference with no binding in scope"
                        .to_string(),
                )
            })?;
            Ok(b.name(nm.clone()))
        }
        // A runtime binary operator — arithmetic, integer/bool comparison, string ordering, or float
        // comparison. All four carry `{op, lhs, rhs}` (FloatCompare also a width, ignored — the surface
        // operator is width-agnostic), and all re-emit as `(<operator> <lhs> <rhs>)`. The surface
        // operator is recovered from the prim; the INTERNAL float prims (`FAdd`/`FEq`/…) share the same
        // one surface operator as their integer counterparts (the author writes one `+`/`=`/`<`, `lower`
        // picks the prim by solved type), so re-emitting the shared operator re-solves to the same prim.
        Core::Arith { op, lhs, rhs }
        | Core::Compare { op, lhs, rhs }
        | Core::StrCmp { op, lhs, rhs }
        | Core::FloatCompare {
            op,
            lhs,
            rhs,
            width: _,
        } => {
            // The WRAPPING arithmetic prims are opted into BY NAME on the int-type module —
            // `(Int64.wrapping-add …)` = `((. <IntType> wrapping-add) l r)`, NOT a bare operator (a plain
            // `+` is the trapping form). The module is the operand's int type (`render_name`), so
            // `UInt8.wrapping-mul` round-trips to the same prim. (These reach here as `Core::Arith` — a
            // wrapping prim lowers to a raw machine op — so `prim_operator` has no symbol for them.)
            use crate::resolved::Prim::{WrappingAdd, WrappingMul, WrappingSub};
            if let WrappingAdd | WrappingSub | WrappingMul = op {
                let member = match op {
                    WrappingAdd => "wrapping-add",
                    WrappingSub => "wrapping-sub",
                    WrappingMul => "wrapping-mul",
                    _ => unreachable!(),
                };
                let operand_ty = crate::infer::type_of(db, lhs);
                let module = match &operand_ty {
                    Ty::Int(it) => int_module_ast(b, *it),
                    _ => {
                        return Err(Reject::decline(
                            "the Cadenza backend cannot recover the int type for a wrapping op"
                                .to_string(),
                        ));
                    }
                };
                let head = member_access_node(b, module, member);
                let l = emit_expr(db, b, lhs, None, env, emitted)?;
                let r = emit_expr(db, b, rhs, None, env, emitted)?;
                return Ok(b.list(vec![head, l, r]));
            }
            // THREE-WAY compare — its surface is the member `(Ordering.of l r)` (the `compare` prim is
            // namespaced as `Ordering.of`, NOT a bare `compare` name — which is unbound). Applies to a
            // scalar `Core::Compare { op: Compare }`; the compound `Core::ValueCmp { Compare }` mirrors it.
            if op == crate::resolved::Prim::Compare {
                let head = member_access(b, "Ordering", "of");
                let l = emit_expr(db, b, lhs, None, env, emitted)?;
                let r = emit_expr(db, b, rhs, None, env, emitted)?;
                return Ok(b.list(vec![head, l, r]));
            }
            let sym = prim_operator(op).ok_or_else(|| {
                Reject::unsupported(format!(
                    "the Cadenza backend does not support lowering the operator prim {op:?}"
                ))
            })?;
            let head = b.name(sym);
            // Operands FIRST would reverse head-first order — build the head atom, then each operand
            // sub-tree left-to-right, then the list (children hold the ids; the head is already pushed).
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // RATIONAL constructors / accessors — member-access ops `((. Rational <member>) <op>…)` (member
        // names per `prelude.rs`: `of`/`of-int`/`numerator`/`denominator`). Present when an operand is
        // runtime (a constant `Rational.of` folds to `Core::ConstRational`). `Rational.of` takes two
        // fixed-width ints; `of-int` widens one int to `n/1`; `numerator`/`denominator` project a `BigInt`.
        // `BigInt.of n` on a RUNTIME fixed-width int — widen to a `BigInt`. Member-access `((. BigInt of)
        // <value>)` (a constant folds to a `ConstInt`-typed-`BigInt` in `lower`, handled as a value above).
        Core::BigIntOfI64 { value } => {
            let head = member_access(b, "BigInt", "of");
            let v = emit_expr(db, b, value, None, env, emitted)?;
            Ok(b.list(vec![head, v]))
        }
        // `<IntType>.of <bigint>` — the CHECKED narrowing of a BigInt to a fixed-width int (traps out of
        // range). Member-access `((. <IntType> of) <operand>)`; the target int module is this node's OWN
        // result type (`render_name` → `Int64`/`Int8`/…), so a narrow target round-trips to the same op.
        Core::BigIntToI64 { operand } => {
            let ty = crate::infer::type_of(db, id);
            let module = match &ty {
                Ty::Int(it) => int_module_ast(b, *it),
                _ => {
                    return Err(Reject::decline(
                        "the Cadenza backend cannot recover the target int type for a BigInt narrow"
                            .to_string(),
                    ));
                }
            };
            let head = member_access_node(b, module, "of");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // A numeric CONVERSION `(<TargetType>.<member> <operand>)` — the TARGET type is this node's OWN
        // result type (`render_name` → `Int8`/`Float64`/…); the MEMBER depends on the conversion kind:
        // `FloatOfInt` (int→float) is written `Float64.of-int`; the TRUNCATING integer conversion `Wrap`
        // is `<Type>.wrap` (total — keeps the low bits, never traps); a float→float WIDTH change (`FloatOf`)
        // and the CHECKED int→int width/sign change (`CheckedOf`) are `<Type>.of`. The operand's type +
        // target re-select the exact op on recompile. TRAP: `Wrap` and `CheckedOf` share the `Core::Convert`
        // node but have DIFFERENT semantics — `.wrap` truncates, `.of` range-CHECKS (traps out of range) —
        // so a `Wrap` re-emitted as `.of` is a VALUE-changing miscompile (hop traps where direct wraps),
        // NOT a decline. A non-numeric result (the boolean-coercion `!`) declines (a later slice). (An
        // int→int CHECKED narrow carries a range-check `Trap` above the `Convert`, which declines on `Trap`
        // first; a `Wrap` narrow is total, has no such `Trap`, and reaches here.)
        Core::Convert { op, operand } => {
            // Use `eff_ty` (the view-aware type), not `type_of(id)`: a Convert that IS a `Qty` magnitude is
            // reached with `view = Some(inner)` (the peeled numeric type), so its own node type is `Qty`
            // while `eff_ty` is the real result type (`Float64`/…). For a non-peeled Convert `eff_ty ==
            // type_of(id)`, so this is unchanged there.
            let ty = eff_ty.clone();
            let member = match op {
                crate::resolved::Prim::FloatOfInt => "of-int",
                crate::resolved::Prim::Wrap => "wrap",
                _ => "of",
            };
            // The target module head: a fixed-width int renders to its recompilable surface (a bare
            // alias name, or the ctor application `(Int 24)` for an odd width); a float width is always
            // aliased (`Float32`/`Float64`), so its bare name is fine.
            let head = match &ty {
                Ty::Int(it) => {
                    let module = int_module_ast(b, *it);
                    member_access_node(b, module, member)
                }
                Ty::Float(_) => {
                    let module = ty.render_name(&db.name_ctx());
                    member_access(b, &module, member)
                }
                _ => {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support lowering a non-numeric Convert (e.g. a boolean \
                         coercion); only numeric conversions are lowered"
                            .to_string(),
                    ));
                }
            };
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::RationalOfInts { num, den } => {
            let head = member_access(b, "Rational", "of");
            let n = emit_expr(db, b, num, None, env, emitted)?;
            let d = emit_expr(db, b, den, None, env, emitted)?;
            Ok(b.list(vec![head, n, d]))
        }
        Core::RationalOfIntWiden { value } => {
            let head = member_access(b, "Rational", "of-int");
            let v = emit_expr(db, b, value, None, env, emitted)?;
            Ok(b.list(vec![head, v]))
        }
        Core::RationalNum { operand } => {
            let head = member_access(b, "Rational", "numerator");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::RationalDen { operand } => {
            let head = member_access(b, "Rational", "denominator");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // BigInt / Rational ARITHMETIC + COMPARISON — the numeric-tower twins of `Arith`/`Compare`, present
        // when an operand is a RUNTIME `BigInt`/`Rational` (a constant pair folds in `lower`). All re-emit
        // the plain surface operator `(<op> l r)`: on recompile the operands' `BigInt`/`Rational` type
        // re-selects the same tower op (the author writes one `+`/`<`, `lower` picks the op by solved type).
        Core::BigIntBinOp { op, lhs, rhs } => {
            let sym = match op {
                crate::core::BigIntOp::Add => "+",
                crate::core::BigIntOp::Sub => "-",
                crate::core::BigIntOp::Mul => "*",
                crate::core::BigIntOp::Div => "/",
                crate::core::BigIntOp::Rem => "%",
            };
            let head = b.name(sym);
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        Core::RationalBinOp { op, lhs, rhs } => {
            let sym = match op {
                crate::core::RationalOp::Add => "+",
                crate::core::RationalOp::Sub => "-",
                crate::core::RationalOp::Mul => "*",
                crate::core::RationalOp::Div => "/",
            };
            let head = b.name(sym);
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        Core::BigIntCmp { op, lhs, rhs } | Core::RationalCmp { op, lhs, rhs } => {
            let sym = prim_operator(op).ok_or_else(|| {
                Reject::unsupported(format!(
                    "the Cadenza backend does not support lowering the numeric-tower compare prim {op:?}"
                ))
            })?;
            let head = b.name(sym);
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // Boolean negation `(not x)`.
        Core::Not { operand } => {
            let head = b.name("not");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // Short-circuiting conjunction / disjunction — `is_and` picks `and` vs `or`.
        Core::And { lhs, rhs, is_and } => {
            let head = b.name(if is_and { "and" } else { "or" });
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // A two-way conditional `(if cond then else)`. Both BRANCHES are value/tail positions of the same
        // solved type — the `if`'s own type (the join of the branches). Pass it down as `expected` so a
        // branch that is an under-determined `Core::SumNew` (a bare `(None)` whose own type is `Option<?>`)
        // recovers the concrete join type to ascribe against. The condition is a Bool operand — no expected.
        Core::If { cond, then_, else_ } => {
            let head = b.name("if");
            let ctx = body_ctx(db, id, expected);
            let c = emit_expr(db, b, cond, None, env, emitted)?;
            let t = emit_expr(db, b, then_, ctx.clone(), env, emitted)?;
            let e = emit_expr(db, b, else_, ctx, env, emitted)?;
            Ok(b.list(vec![head, c, t, e]))
        }
        // A kept multi-use binding sequence `(let ((<n0> <v0>) …) <body>)`. Each binding is `(init, init)`
        // — keyed only by its initializer occurrence — so a fresh surface name is minted deterministically
        // (by binding order) and recorded in `env` for the `LocalRef`s that read it. Bindings are
        // SEQUENTIAL (`let*`): a later binding's value may reference an earlier binding, so each name is
        // registered right AFTER its own value is emitted and BEFORE the next; the body is emitted with
        // every binding in scope. Head-first: the `let` head and each binding's name atom are pushed
        // before their sub-trees.
        Core::Let { bindings, body } => {
            let let_head = b.name("let");
            let mut binding_nodes = Vec::with_capacity(bindings.len());
            for &(binder, value) in bindings.iter() {
                let name = synth_binding_name(env.lets.len());
                let name_atom = b.name(name.clone());
                // The value is emitted with only the PRIOR bindings in scope (a binding's initializer
                // cannot reference itself), then this binding is registered for the rest of the sequence.
                let value_node = emit_expr(db, b, value, None, env, emitted)?;
                env.lets.insert(binder, name);
                binding_nodes.push(b.list(vec![name_atom, value_node]));
            }
            let bindings_list = b.list(binding_nodes);
            // The body is the `let`'s value/tail position — it has the whole `let`'s type, so it inherits
            // this `let`'s `expected`; the bindings' initializers are operands (no expected).
            let body_node = emit_expr(db, b, body, expected, env, emitted)?;
            Ok(b.list(vec![let_head, bindings_list, body_node]))
        }
        // A runtime CALL to a top-level function — `(<callee-name> <arg>…)`. `Core::Call` is present only
        // for an application that could NOT be inlined-and-folded at compile time (i.e. a RECURSIVE
        // callee); `callee` is the `db.defs` index, whose source name re-resolves to the same definition
        // (it is in `layout.order`, so this backend also emits its `(def …)`). Args are lowered in the
        // caller's frame, left-to-right. Head-first: the callee name atom is pushed before the args.
        Core::Call { callee, args } => {
            let callee_name = db.defs[callee].name.clone();
            let head = b.name(callee_name.as_str());
            let mut children = Vec::with_capacity(1 + args.len());
            children.push(head);
            for &arg in args.iter() {
                children.push(emit_expr(db, b, arg, None, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // A scalar MATCH over a runtime Int/Bool/Char scrutinee — re-emit as an `if`-CHAIN of literal-equality
        // probes: `(match s (l0 b0) … (_ bn))` → `(if (= s l0) b0 (if … bn))`. This is VALUE-equivalent
        // (the backend itself lowers a scalar match to a probe chain), reusing the `if`/`=` emit; the
        // scrutinee is a pure scalar, so re-emitting it per probe is side-effect-free. A `Char` is a Unicode
        // scalar value with a runtime `=` (like `Int`) — its literal re-emits as `#\c` and recompiles to the
        // same char-equality probe. M1 handles LITERAL probes (`Int`/`Bool`/`Char`) + a wildcard tail,
        // UNGUARDED; a guarded arm or a still-non-scalar probe (`Str`/`Bytes`/`ListLen`/`MapHasKeys` — a
        // runtime `Str`/`Bytes` match desugars to an equality if-chain in `lower` before the backend, so it
        // reaches here only in an un-desugared residue) declines (later slices).
        Core::Match { scrutinee, arms } => {
            if arms.is_empty() {
                return Err(Reject::decline(
                    "the Cadenza backend does not lower a zero-arm match".to_string(),
                ));
            }
            for arm in &arms {
                if !matches!(
                    arm.probe,
                    crate::core::Probe::Int(_)
                        | crate::core::Probe::Bool(_)
                        | crate::core::Probe::Char(_)
                        | crate::core::Probe::Wild
                ) {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support lowering a non-scalar match probe \
                         (Str/Bytes/list/map)"
                            .to_string(),
                    ));
                }
            }
            let ctx = body_ctx(db, id, expected);
            emit_match_chain(db, b, scrutinee, &arms, 0, ctx, env, emitted)
        }
        // A runtime TUPLE value `(tuple <e>…)` — a fixed-arity positional product built from runtime
        // operands (a projection of a compile-time-visible tuple folds away in `lower`, so a surviving
        // `Core::Tuple` is a runtime value). Mirrors lower's constant value surface.
        Core::Tuple { elems } => {
            // M2: a native TUPLE ctor-leaf head (`b.compound`), not a `tuple` name head. Mirrors
            // `const_value_ast`.
            let mut children = Vec::with_capacity(elems.len());
            // Each element's `expected` is the tuple's own type at that position — so an under-determined
            // element (a bare `(None)` in `(tuple (None) 3)`) recovers its type from the tuple's slot type.
            for (i, e) in elems.iter().copied().enumerate() {
                let ex = match &eff_ty {
                    Ty::Tuple(ts) => ts.get(i).cloned(),
                    _ => None,
                };
                children.push(emit_expr(db, b, e, ex, env, emitted)?);
            }
            Ok(b.compound(crate::ast::CompoundCtor::Tuple, &children))
        }
        // A runtime RECORD value `(record (= <k> <v>)…)` — fields in canonical (name-sorted `BTreeMap`)
        // order, each an `(= name value)` ascription pair (matching lower's `const_value_ast` surface).
        Core::Record { fields } => {
            // M2: a native RECORD ctor-leaf head; fields are `(= k v)` FieldPair leaves (`b.field_pair`).
            let mut children = Vec::with_capacity(fields.len());
            for (name, &v) in fields.iter() {
                let fname = b.name(&*name.name);
                // The field's `expected` is the record type's field type (an under-determined field value
                // recovers it from there).
                let ex = match &eff_ty {
                    Ty::Record(ftys) => ftys.get(name).cloned(),
                    _ => None,
                };
                let fval = emit_expr(db, b, v, ex, env, emitted)?;
                children.push(b.field_pair(fname, fval));
            }
            Ok(b.compound(crate::ast::CompoundCtor::Record, &children))
        }
        // A runtime LIST value `(list <e>…)` — an ordered homogeneous sequence built from runtime
        // operands; the walk preserves element order.
        Core::ListNew { elems } => {
            // M2: a native LIST ctor-leaf head, not a `list` name head.
            let mut children = Vec::with_capacity(elems.len());
            // Every element's `expected` is the list's element type — so a bare `(None)` element (in
            // `(list (Some n) (None) …)`, whose own type is `Option<?>`) recovers `Option Int64` from it.
            let elem_ty = match &eff_ty {
                Ty::List(e) => Some((**e).clone()),
                _ => None,
            };
            for e in elems.iter().copied() {
                children.push(emit_expr(db, b, e, elem_ty.clone(), env, emitted)?);
            }
            Ok(b.compound(crate::ast::CompoundCtor::List, &children))
        }
        // A runtime MAP value `(map (<k> <v>)…)` — the entries are runtime operands (a fully-constant map
        // bakes via lower's constant escape, so a surviving `Core::MapNew` is a runtime value). Entries are
        // emitted in their STORED order, NOT re-sorted into canonical key order: a map is UNORDERED, so the
        // reconstructed value equals the original regardless of entry order (and the keys are runtime, so no
        // compile-time canonical sort is available anyway) — the round-trip is VALUE-equivalence, which a
        // map's order-independent identity satisfies. Each entry is the pair-list `(<k> <v>)` (distinct from
        // a record's `(= k v)`), key then value emitted left-to-right. Mirrors lower's constant map surface.
        Core::MapNew { entries, .. } => {
            // M2: a native MAP ctor-leaf head; entries are `(= k v)` FieldPair leaves (distinguished from a
            // record only by the MAP head). Stored order (a map is unordered → value-eq is order-independent).
            // Key/value `expected` are the map type's key/value types (an under-determined key or value —
            // e.g. a `(None)` value — recovers its type from there).
            let (key_ty, val_ty) = match &eff_ty {
                Ty::Map(k, v) => (Some((**k).clone()), Some((**v).clone())),
                _ => (None, None),
            };
            // LAST-WINS dedup of FOLDED-CONSTANT keys. The optimizer folds a bound key NAME to a literal (a
            // `(let ((a 5)) #map((= a 1) (= 5 2)))` folds to `#map((= 5 1) (= 5 2))`), so two entries can
            // collapse to the SAME literal key — and the front-end REJECTS a `#map` with duplicate literal keys
            // (CDZ0201), rejecting the backend's own output (breaker-minimized). Runtime map construction is
            // last-wins (a later entry overwrites an equal key), so keep only the LAST entry per equal CONSTANT
            // key: value-identical to the original AND recompilable. A non-constant (RUNTIME) key is never a
            // literal (so never triggers the duplicate-literal check) and is genuinely distinct, so it is always
            // kept — the runtime resolves any equal-at-runtime collision exactly as the original did.
            let mut last_for_sig: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (i, &(k, _)) in entries.iter().enumerate() {
                if let Some(sig) = map_key_const_sig(db, k) {
                    last_for_sig.insert(sig, i);
                }
            }
            let mut children = Vec::with_capacity(entries.len());
            for (i, &(k, v)) in entries.iter().enumerate() {
                if let Some(sig) = map_key_const_sig(db, k) {
                    // An earlier duplicate of a constant key that recurs later → drop (last-wins).
                    if last_for_sig.get(&sig) != Some(&i) {
                        continue;
                    }
                }
                let kv = emit_expr(db, b, k, key_ty.clone(), env, emitted)?;
                let vv = emit_expr(db, b, v, val_ty.clone(), env, emitted)?;
                children.push(b.field_pair(kv, vv));
            }
            Ok(b.compound(crate::ast::CompoundCtor::Map, &children))
        }
        // A runtime SET value `((. Set of) (list <e>…))` — the `Set.of` application over a `(list …)` of the
        // elements (a fully-constant set bakes via lower's constant escape; a surviving `Core::SetOf` is a
        // runtime value). Like the map, elements emit in STORED order (a set is unordered, so value-identity
        // is order-independent). `Set.of` is the member access `(. Set of)`, matching lower's set surface.
        Core::SetOf { elems, .. } => {
            // M2: a native SET ctor-leaf head (`#set(…)`), REPLACING the old `((. Set of) (list …))`
            // member-path. Stored order (a set is unordered → value-eq is order-independent).
            let mut children = Vec::with_capacity(elems.len());
            let elem_ty = match &eff_ty {
                Ty::Set(e) => Some((**e).clone()),
                _ => None,
            };
            for e in elems.iter().copied() {
                children.push(emit_expr(db, b, e, elem_ty.clone(), env, emitted)?);
            }
            Ok(b.compound(crate::ast::CompoundCtor::Set, &children))
        }
        // A runtime SUM (variant) value `(<Variant> <payload>)` — a constructed variant built from a
        // runtime payload. The variant NAME is recovered from the discriminant against the node's solved
        // sum type (`variant_head_ast` — bare, or `(. Type Variant)` when the name would collide with a
        // non-ctor prelude binding). A nullary variant carries `unit` (`(None unit)`), a single-payload
        // variant its payload; a multi-argument variant surface is not canonical and declines. Mirrors
        // lower's constant value surface.
        Core::SumNew { disc, payloads } => {
            // The value's own solved type. When it is UNDER-DETERMINED (a free type argument — a bare
            // nullary `(None)` at a join whose element type only the sibling branch fixes, so this node's
            // own type is `Option<?>`), fall back to the `expected` type the surrounding context supplied
            // (the `if`/`let`/match position this value fills). Both are the SAME sum declaration; `expected`
            // just carries the RESOLVED type arguments, which is what the `(: … <sum-type>)` ascription needs.
            let own_ty = crate::infer::type_of(db, id);
            let ty = match (&own_ty, &expected) {
                // Under-determined own type + a concrete expected of the same sum decl → use expected.
                (Ty::Sum { decl: od, .. }, Some(ex @ Ty::Sum { decl: ed, .. }))
                    if od == ed && ty_has_free_arg(&own_ty) && !ty_has_free_arg(ex) =>
                {
                    ex.clone()
                }
                _ => own_ty,
            };
            let decl = match &ty {
                Ty::Sum { decl, .. } => *decl,
                _ => {
                    return Err(Reject::decline(
                        "the Cadenza backend cannot recover a variant head for a non-sum SumNew"
                            .to_string(),
                    ));
                }
            };
            // `(: <variant> <sum-type>)` — the ASCRIPTION is required: the optimizer often folds a sum
            // value to a bare variant with no surrounding type context (e.g. `main` = `(None unit)`), and
            // a nullary or partially-parameterized variant under-determines the sum's type parameters
            // (`(Option _)` / `(Result Int64 _)`) → CDZ0203 on recompile. Annotating with the full solved
            // sum type (via lower's `type_ast`) pins it. `type_ast` returns `None` for an under-determined
            // sum (a free type-arg), so a genuinely-ambiguous value DECLINES rather than emit a bad surface.
            let colon = b.name(":");
            let head = crate::lower::variant_head_ast(db, b, decl, disc).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend could not recover the variant name for a SumNew"
                        .to_string(),
                )
            })?;
            let mut variant_children = vec![head];
            match payloads.len() {
                // A NULLARY variant carries `unit` (`(None unit)`).
                0 => variant_children.push(b.name("unit")),
                // SINGLE payload — its `expected` is the variant's INSTANTIATED payload type at this sum
                // type, so a bare `(None)` nested as a payload (`(Some (None))` : `Option (Option Int64)`,
                // whose inner own type is `Option<?>`) recovers `Option Int64` from the outer instantiation.
                1 => {
                    let pexp = sum_payload_expected(db, decl, disc, &ty);
                    variant_children.push(emit_expr(db, b, payloads[0], pexp, env, emitted)?);
                }
                // MULTI-argument variant `(<Variant> p0 p1 …)` — each payload emitted left-to-right (their
                // types are the variant's slot types, determined by the operands; no under-determined
                // fallback needed for the common case).
                _ => {
                    for &p in payloads.iter() {
                        variant_children.push(emit_expr(db, b, p, None, env, emitted)?);
                    }
                }
            }
            let variant = b.list(variant_children);
            let ncx = db.name_ctx();
            let ty_node = crate::lower::type_ast(b, &ty, &ncx).ok_or_else(|| {
                Reject::unsupported(
                    "the Cadenza backend does not support lowering a variant of an under-determined sum type"
                        .to_string(),
                )
            })?;
            Ok(b.list(vec![colon, variant, ty_node]))
        }
        // A match over a runtime SUM scrutinee — re-emit the surface `(match <scrutinee> (<pat> <body>)…)`.
        // M4a handles the SIMPLE decision-tree shape (delegated to [`emit_match_sum`]): a root switch on the
        // scrutinee's OWN discriminant, every arm an explicit variant with a bare LEAF body; a disc-folded /
        // nested / guarded / literal-test tree, or a default (wildcard) arm, declines (a later slice).
        Core::MatchSum { scrutinee, root } => {
            let ctx = body_ctx(db, id, expected);
            emit_match_sum(db, b, scrutinee, &root, ctx, env, emitted)
        }
        // A match over a runtime LIST scrutinee — re-emit `(match <scrutinee> (<list-pattern> <body>)…)`
        // ([`emit_match_list`]): a length-`LenEq`/`LenGe`/`Any` arm with PLAIN leading-element + rest binders.
        // A guarded arm, or a nested/variant element sub-pattern (a deeper `SumPayload` path), declines.
        Core::MatchList { scrutinee, arms } => {
            let ctx = body_ctx(db, id, expected);
            emit_match_list(db, b, scrutinee, &arms, ctx, env, emitted)
        }
        // A match PAYLOAD read — its surface is the binder name the enclosing `MatchSum`/`MatchList` arm
        // minted for this `(scrutinee, path)` and recorded in `env.payloads` (a sum variant payload at
        // `[Payload]`/`[Payload, Elem(i)]`, or a list element/rest at `[Elem(i)]`/`[RestFrom(k)]`). Reached
        // ONLY inside the arm body that bound it; a read whose binder is not in scope (a nested sub-pattern
        // this slice does not emit) declines.
        Core::SumPayload { scrutinee, path } => {
            // Exact registered binder (a match arm's own payload slot).
            if let Some(nm) = env.payloads.get(&(scrutinee, path.to_vec())).cloned() {
                // TYPE-AWARE NEWTYPE PEEL: the binder may hold an ERASED single-variant, single-payload
                // newtype (`(type Box (Mk Int64))`) whose `Payload` step the optimizer elided — collapsing an
                // inner-payload read's key onto this binder's key. If THIS read's OWN solved type is the
                // newtype's INNER (not the newtype), the bare binder recompiles as the newtype where the inner
                // is required (CDZ0203, e.g. a map-key `(Box.Mk n)` sub-pattern read). Emit the type-correct
                // unwrap `(match <binder> ((<Ctor> x) x))` (irrefutable single-variant, value-eq — recompile
                // re-erases). Gated on a stored binder type that is an EMITTED single-variant/single-payload
                // nominal whose inner EQUALS the node's solved type; absent / non-matching → bare binder.
                if let Some(binder_ty) = env.payload_tys.get(&(scrutinee, path.to_vec())).cloned()
                    && let Ty::Nominal {
                        decl: nd, inner, ..
                    } = &binder_ty
                    && emitted.contains(nd)
                    && db
                        .type_decl_by_occ(*nd)
                        .is_some_and(|t| t.variants.len() == 1 && t.variants[0].payloads.len() == 1)
                {
                    let node_ty = crate::infer::type_of(db, id);
                    if **inner == node_ty && node_ty != binder_ty {
                        let nd = *nd;
                        let scrut = b.name(nm.clone());
                        let ctor =
                            crate::lower::variant_head_ast(db, b, nd, 0).ok_or_else(|| {
                                Reject::decline(
                                    "the Cadenza backend could not recover the newtype ctor for a \
                                     payload peel"
                                        .to_string(),
                                )
                            })?;
                        let x = synth_payload_name(env.next_payload);
                        env.next_payload += 1;
                        let x_pat = b.name(x.clone());
                        let pat = b.list(vec![ctor, x_pat]);
                        let body = b.name(x);
                        let arm = b.list(vec![pat, body]);
                        let match_head = b.name("match");
                        return Ok(b.list(vec![match_head, scrut, arm]));
                    }
                }
                return Ok(b.name(nm));
            }
            // Else: a NESTED compound destructure — a `(tuple a c)` / `(record (= f p))` pattern inside a
            // match arm reads its element as `SumPayload { scrutinee = <the enclosing bound compound>, path =
            // [Elem(i)…] }` (relative to that scrutinee, not the root). Emit the SCRUTINEE's surface (it
            // recurses — the scrutinee is itself a registered payload / binder), then re-emit the `Elem` path
            // as nested PROJECTIONS: index for a `Ty::Tuple`, field name for a `Ty::Record` (mirrors the
            // `Core::Proj` arm, and its type-driven key). A `Payload` / `RestFrom` step (a refutable nested
            // sum/list — needs a real nested match) is not projectable and declines. An irrefutable
            // tuple/record destructure is value-eq to these projections (recompile re-lowers them identically).
            let mut cur_ty = crate::infer::type_of(db, scrutinee);
            let mut node = emit_expr(db, b, scrutinee, None, env, emitted)?;
            for step in path.iter() {
                let crate::core::PathStep::Elem(i) = *step else {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support lowering a nested match sub-pattern with a \
                         non-tuple/record (sum / list-rest) step"
                            .to_string(),
                    ));
                };
                // Clone the type cursor so the arms can REASSIGN `cur_ty` (the single-variant peel below
                // descends the type) without holding the `&cur_ty` match borrow.
                let ct = cur_ty.clone();
                match &ct {
                    Ty::Tuple(ts) => {
                        let key = b.atom_leaf(Leaf::Int {
                            value: IntValue::from_i64(i as i64),
                            radix: Radix::Dec,
                        });
                        let dot = b.name(".");
                        node = b.list(vec![dot, node, key]);
                        cur_ty = ts.get(i).cloned().unwrap_or(Ty::Any);
                    }
                    Ty::Record(fields) => {
                        let fname = fields.keys().nth(i).ok_or_else(|| {
                            Reject::decline(
                                "the Cadenza backend could not recover a record field name for a \
                                 nested projection"
                                    .to_string(),
                            )
                        })?;
                        let key = b.name(&*fname.name);
                        let dot = b.name(".");
                        node = b.list(vec![dot, node, key]);
                        cur_ty = fields.values().nth(i).cloned().unwrap_or(Ty::Any);
                    }
                    // A SINGLE-VARIANT sum, typed as an ERASED `Ty::Nominal`, whose `Payload` step the
                    // optimizer ELIDED (erasure): the runtime rep IS the sole variant's payload, so `Elem(i)`
                    // indexes payload slot `i` (a field accessor `let Ctor(a,_,_)=x in a` inlined to a direct
                    // element read). The surface `.` operator does NOT type-check on a nominal (the erasure
                    // wall), so re-emit an inline single-arm `(match <node> ((<Ctor> b0 … b_{n-1}) b_i))` that
                    // NAMES the ctor — the TYPE-CORRECT surface crossing (v-wasm-opt review #2). IRREFUTABLE
                    // (one variant → exhaustive, no CDZ0210) and value-eq (recompile re-erases the newtype).
                    // Gated to an EMITTED user single-variant sum (its `(type …)` must resolve the ctor on
                    // recompile). `inner` is the erased payload machine-rep (a `Tuple` of slots for a
                    // multi-payload variant, or the sole type for arity 1) — the source of slot `i`'s type.
                    Ty::Nominal { decl, inner, .. }
                        if emitted.contains(decl)
                            && db
                                .type_decl_by_occ(*decl)
                                .is_some_and(|t| t.variants.len() == 1) =>
                    {
                        let decl = *decl;
                        let arity = db
                            .type_decl_by_occ(decl)
                            .and_then(|t| t.variants.first())
                            .map(|v| v.payloads.len())
                            .unwrap_or(0);
                        // The erased `inner` rep IS what `Elem(i)` indexes. TWO layouts, discriminated by
                        // arity vs the inner tuple's arity (NOT `i < arity`):
                        //  - MULTI-payload variant (`arity == inner_len`, e.g. Subst(Map,Map,Map)): each
                        //    payload IS a tuple element → `(match node ((Ctor b0…b_{n-1}) b_i))`, body `b_i`.
                        //  - ARITY-1 TUPLE-payload NEWTYPE (`arity == 1`, inner a `Tuple` of len > 1, e.g.
                        //    WrapT(Tuple(Int64,Int64)) — breaker): the sole payload IS the tuple, so `Elem(i)`
                        //    projects INTO it → `(match node ((Ctor t) (. t i)))`, body `(. t i)`. (Emitting a
                        //    bare `(. node i)` would project the NOMINAL — the CDZ0201 recompile bug.)
                        let inner_len = match &**inner {
                            Ty::Tuple(ts) => Some(ts.len()),
                            _ => None,
                        };
                        let head =
                            crate::lower::variant_head_ast(db, b, decl, 0).ok_or_else(|| {
                                Reject::decline(
                                    "the Cadenza backend could not recover the single-variant ctor name for \
                                     a payload projection"
                                        .to_string(),
                                )
                            })?;
                        if inner_len == Some(arity) && i < arity {
                            // Multi-payload: bind every slot, return slot `i`.
                            let slot_ty = match &**inner {
                                Ty::Tuple(ts) => ts.get(i).cloned().unwrap_or(Ty::Any),
                                _ => Ty::Any,
                            };
                            let mut pat_children = vec![head];
                            let mut bi = None;
                            for slot in 0..arity {
                                let nm = synth_payload_name(env.next_payload);
                                env.next_payload += 1;
                                if slot == i {
                                    bi = Some(nm.clone());
                                }
                                pat_children.push(b.name(nm));
                            }
                            let pat = b.list(pat_children);
                            let body = b.name(bi.expect("slot i < arity is bound"));
                            let match_head = b.name("match");
                            let arm = b.list(vec![pat, body]);
                            node = b.list(vec![match_head, node, arm]);
                            cur_ty = slot_ty;
                        } else if arity == 1 && inner_len.is_some_and(|l| i < l) {
                            // Arity-1 tuple-payload newtype: bind the sole payload `t`, project `(. t i)`.
                            let slot_ty = match &**inner {
                                Ty::Tuple(ts) => ts.get(i).cloned().unwrap_or(Ty::Any),
                                _ => Ty::Any,
                            };
                            let t = synth_payload_name(env.next_payload);
                            env.next_payload += 1;
                            let t_pat = b.name(t.clone());
                            let pat = b.list(vec![head, t_pat]);
                            let dot = b.name(".");
                            let idx = b.atom_leaf(Leaf::Int {
                                value: IntValue::from_i64(i as i64),
                                radix: Radix::Dec,
                            });
                            let t_ref = b.name(t);
                            let proj = b.list(vec![dot, t_ref, idx]);
                            let match_head = b.name("match");
                            let arm = b.list(vec![pat, proj]);
                            node = b.list(vec![match_head, node, arm]);
                            cur_ty = slot_ty;
                        } else {
                            return Err(Reject::unsupported(
                                "the Cadenza backend does not support a payload projection over a \
                                 single-variant sum whose erased payload layout it cannot index"
                                    .to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(Reject::unsupported(
                            "the Cadenza backend does not support a payload projection over a \
                             non-tuple/record value (e.g. a newtype ctor sub-pattern nested under a \
                             list/tuple pattern over Map.to-list output)"
                                .to_string(),
                        ));
                    }
                }
            }
            Ok(node)
        }
        // MAP / SET OPERATIONS — each a prelude member-access application `((. <Module> <member>) <op>…)`
        // (the member name is the prelude field the op resolves from — `Map.len` is `map-size`, `Set.len` is
        // `set-len`; see `prelude.rs`). The operands are runtime values (a fully-constant op folds in
        // `lower`), emitted left-to-right with no expected. `Map.lookup` re-reads to its `Option` result and
        // `Map.len`/`Set.len` to `Int64`, exactly as the surface member does, so the round-trip re-lowers to
        // the same op node. (`Map.empty`/`Set.of`/the constant maps are handled as VALUES above.)
        Core::MapInsert { map, key, val, .. } => {
            let head = member_access(b, "Map", "insert");
            let m = emit_expr(db, b, map, None, env, emitted)?;
            let k = emit_expr(db, b, key, None, env, emitted)?;
            let v = emit_expr(db, b, val, None, env, emitted)?;
            Ok(b.list(vec![head, m, k, v]))
        }
        Core::MapLookup { map, key, .. } => {
            let head = member_access(b, "Map", "lookup");
            let m = emit_expr(db, b, map, None, env, emitted)?;
            let k = emit_expr(db, b, key, None, env, emitted)?;
            Ok(b.list(vec![head, m, k]))
        }
        Core::MapRemove { map, key, .. } => {
            let head = member_access(b, "Map", "remove");
            let m = emit_expr(db, b, map, None, env, emitted)?;
            let k = emit_expr(db, b, key, None, env, emitted)?;
            Ok(b.list(vec![head, m, k]))
        }
        // RUNTIME `Ast` reflection ops — `Ast.encode`/`Ast.print`/`Ast.decode` over a RUNTIME `Ast`/`Bytes`
        // value (a compile-time-visible `Ast` folds to a constant in `ast_reflect`, so a SURVIVING node is
        // genuinely runtime). Each re-emits its member-access perform `((. Ast <op>) <operand>)`; the node's
        // `discs`/`disc_ok`/`disc_err` are the Ast-sum discriminant metadata, RE-DERIVED from the operand's
        // solved type on recompile (not carried on the surface), so the bare member-access round-trips to the
        // same node. (v-metaprogramming owns the `Ast` codec/shape; this is the backend emit arm.)
        Core::AstEncode { operand, .. } => {
            let head = member_access(b, "Ast", "encode");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::AstPrint { operand, .. } => {
            let head = member_access(b, "Ast", "print");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::AstDecode { operand, .. } => {
            let head = member_access(b, "Ast", "decode");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // A PERFORM of a host-delegated / escaping effect op — `Core::HostCall { effect: E, op: o, args }`
        // re-emits the member-access perform `((. E o) <args>)` (`E.o` is ordinary member access, v-effects).
        // For this to re-lower to the SAME HostCall on recompile, TWO context pieces are emitted elsewhere:
        // the `(effect E (op o …))` DECL (preamble, [`emit_effect_decls`]) and ONE entrypoint-level
        // `(host (E…) <body>)` delegation ([`emit_def`]) — a generous entrypoint scope is faithful because a
        // HostCall effect is unhandled in-program (a handled op reduces away before the backend), so
        // delegating it over a larger region shadows nothing. NO lexical host-scope reconstruction needed.
        Core::HostCall {
            effect, op, args, ..
        } => {
            // Perform ⇔ decl coupling: only emit if the `(effect E …)` decl re-emitted (else `((. E o) …)`
            // recompiles as `unbound name E` — an op arrow with a non-copyable payload declines the decl).
            if !env.emitted_effects.contains(&effect) {
                return Err(Reject::unsupported(
                    "the Cadenza backend does not support re-emitting a perform of this effect (its \
                     `(effect …)` declaration is not re-emittable — an op signature with a non-copyable \
                     payload)"
                        .to_string(),
                ));
            }
            let head = member_access(b, &effect, &op);
            let mut children = vec![head];
            for &a in args.iter() {
                children.push(emit_expr(db, b, a, None, env, emitted)?);
            }
            // Record the effect so `emit_def` wraps this def's body in `(host (E…) …)` (the delegation).
            env.performed_effects.insert(effect.clone());
            Ok(b.list(children))
        }
        Core::MapSize { map } => {
            let head = member_access(b, "Map", "len");
            let m = emit_expr(db, b, map, None, env, emitted)?;
            Ok(b.list(vec![head, m]))
        }
        Core::MapToList { map, .. } => {
            let head = member_access(b, "Map", "to-list");
            let m = emit_expr(db, b, map, None, env, emitted)?;
            Ok(b.list(vec![head, m]))
        }
        Core::SetContains { set, elem, .. } => {
            let head = member_access(b, "Set", "contains");
            let s = emit_expr(db, b, set, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, s, e]))
        }
        Core::SetInsert { set, elem, .. } => {
            let head = member_access(b, "Set", "insert");
            let s = emit_expr(db, b, set, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, s, e]))
        }
        Core::SetRemove { set, elem, .. } => {
            let head = member_access(b, "Set", "remove");
            let s = emit_expr(db, b, set, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, s, e]))
        }
        Core::SetLen { set } => {
            let head = member_access(b, "Set", "len");
            let s = emit_expr(db, b, set, None, env, emitted)?;
            Ok(b.list(vec![head, s]))
        }
        Core::SetToList { set, .. } => {
            let head = member_access(b, "Set", "to-list");
            let s = emit_expr(db, b, set, None, env, emitted)?;
            Ok(b.list(vec![head, s]))
        }
        Core::SetAlgebra { op, lhs, rhs } => {
            let member = match op {
                crate::core::SetAlgebraOp::Union => "union",
                crate::core::SetAlgebraOp::Intersection => "intersection",
                crate::core::SetAlgebraOp::Difference => "difference",
            };
            let head = member_access(b, "Set", member);
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // LIST OPERATIONS — the runtime list ops, each a prelude member-access application
        // `((. List <member>) <op>…)` (`List.len`/`push`/`prepend`/`concat`/`update`/`at`; see `prelude.rs`).
        // A constant-list op folds in `lower`, so a surviving node is a runtime list op. `List.len` re-reads
        // to `Int64`, `List.at` to its `Option` result — the member re-resolves to the same op on recompile.
        Core::ListLen { operand } => {
            let head = member_access(b, "List", "len");
            let l = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, l]))
        }
        Core::ListPush { list, elem } => {
            let head = member_access(b, "List", "push");
            let l = emit_expr(db, b, list, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, l, e]))
        }
        Core::ListPrepend { list, elem } => {
            let head = member_access(b, "List", "prepend");
            let l = emit_expr(db, b, list, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, l, e]))
        }
        Core::ListConcat { lhs, rhs } => {
            let head = member_access(b, "List", "concat");
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        Core::ListUpdate { list, index, elem } => {
            let head = member_access(b, "List", "update");
            let l = emit_expr(db, b, list, None, env, emitted)?;
            let i = emit_expr(db, b, index, None, env, emitted)?;
            let e = emit_expr(db, b, elem, None, env, emitted)?;
            Ok(b.list(vec![head, l, i, e]))
        }
        Core::ListAt { list, index, .. } => {
            let head = member_access(b, "List", "at");
            let l = emit_expr(db, b, list, None, env, emitted)?;
            let i = emit_expr(db, b, index, None, env, emitted)?;
            Ok(b.list(vec![head, l, i]))
        }
        // BYTES OPERATIONS — `Bytes.of` builds a byte sequence from a `(list …)` of `Int64` bytes; the rest
        // are member-access ops `((. Bytes <member>) <op>…)` (`len`/`at`/`concat`/`slice`/`compact`; see
        // `prelude.rs`). A fully-constant `Bytes.of` bakes in `lower`, so a surviving node is a runtime value.
        Core::BytesOf { elems } => {
            let list_head = b.name("list");
            let mut list_children = Vec::with_capacity(1 + elems.len());
            list_children.push(list_head);
            for e in elems.iter().copied() {
                list_children.push(emit_expr(db, b, e, None, env, emitted)?);
            }
            let inner_list = b.list(list_children);
            let head = member_access(b, "Bytes", "of");
            Ok(b.list(vec![head, inner_list]))
        }
        // A runtime BINARY CONSTRUCTION `(bin (u16 v) (u8 v) (u16 v le) …)` — each `Core::BinSeg` is a
        // fixed-width integer segment: surface head `u<bits>`/`i<bits>` (bits = 8·width; `u`/`i` by
        // `signed`), the runtime value, and a trailing `le` name atom when `little_endian` (big-endian is
        // the modifier-free default). A constant `bin` folds to a `Core::ConstBytes` in `lower`, so a
        // surviving `BinBuild` is genuinely runtime. (BN4b carries INT segments only — a bits/`(bytes …)`
        // splice with a runtime value declines at `lower`, so it never reaches here.)
        Core::BinBuild { segs } => {
            let mut children = Vec::with_capacity(1 + segs.len());
            children.push(b.name("bin"));
            for seg in segs.iter() {
                let bits = u32::from(seg.width) * 8;
                let ty = b.name(format!("{}{bits}", if seg.signed { "i" } else { "u" }));
                let val = emit_expr(db, b, seg.value, None, env, emitted)?;
                let mut seg_children = vec![ty, val];
                if seg.little_endian {
                    seg_children.push(b.name("le"));
                }
                children.push(b.list(seg_children));
            }
            Ok(b.list(children))
        }
        // A runtime BIT-FIELD binary construction `(bin (bits v k) …)` — the `Core::BinBitsBuild` sibling of
        // `BinBuild`: each `BinBitsField` packs the low `k` bits of a runtime value (`k` a compile-time
        // constant read at resolve). Re-emit `(bin (bits <value> <k>) …)` (same `bin` head; the segment head
        // is the `bits` keyword, then the runtime value, then the constant width `k`). A constant bit-field
        // `bin` folds to `Core::ConstBytes` in `lower`, so a surviving `BinBitsBuild` is genuinely runtime.
        Core::BinBitsBuild { fields } => {
            let mut children = Vec::with_capacity(1 + fields.len());
            children.push(b.name("bin"));
            for f in fields.iter() {
                let bits_head = b.name("bits");
                let val = emit_expr(db, b, f.value, None, env, emitted)?;
                let k = b.atom_leaf(Leaf::Int {
                    value: IntValue::from_i64(i64::from(f.k)),
                    radix: Radix::Dec,
                });
                children.push(b.list(vec![bits_head, val, k]));
            }
            Ok(b.list(children))
        }
        // `Bytes.len` and `String.byte-len` share `Core::BytesLen` (a String is a flat UTF-8 byte leaf, so
        // its byte length reads that leaf's length), and the RESULT is `Int64` either way — so unlike
        // `BytesConcat` (disambiguated by result type) this must disambiguate by the OPERAND type: a
        // `Ty::String` operand re-emits `(. String byte-len)`, else `(. Bytes len)`. Emitting `Bytes.len`
        // over a String operand mis-types on recompile (CDZ0203 String-vs-Bytes) — the recompilability
        // break breaker found (a runtime, non-const-foldable `String.byte-len`).
        Core::BytesLen { operand } => {
            let (module, member) = match crate::infer::type_of(db, operand) {
                Ty::String => ("String", "byte-len"),
                _ => ("Bytes", "len"),
            };
            let head = member_access(b, module, member);
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::BytesAt { bytes, index, .. } => {
            let head = member_access(b, "Bytes", "at");
            let by = emit_expr(db, b, bytes, None, env, emitted)?;
            let i = emit_expr(db, b, index, None, env, emitted)?;
            Ok(b.list(vec![head, by, i]))
        }
        // `Core::BytesConcat` backs BOTH `Bytes.concat` AND `String.concat` — a String is a flat UTF-8 byte
        // leaf, so `String.concat` lowers to the SAME `bytes-concat` op (see `lower.rs`). Recover which
        // surface from the result type: a `Ty::String` node re-emits `(. String concat)`, else `(. Bytes
        // concat)` — emitting `Bytes.concat` over String operands would mis-type on recompile.
        Core::BytesConcat { lhs, rhs } => {
            let module = match crate::infer::type_of(db, id) {
                Ty::String => "String",
                _ => "Bytes",
            };
            let head = member_access(b, module, "concat");
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            let head = member_access(b, "Bytes", "slice");
            let by = emit_expr(db, b, bytes, None, env, emitted)?;
            let s = emit_expr(db, b, start, None, env, emitted)?;
            let l = emit_expr(db, b, len, None, env, emitted)?;
            Ok(b.list(vec![head, by, s, l]))
        }
        Core::BytesCompact { operand } => {
            let head = member_access(b, "Bytes", "compact");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // `Value.encode v` — the in-fold canonical binary-AST value-form encode (`∀a. a → Bytes`, TOTAL).
        // Surface `((. Value encode) <value>)`. The `desc` byte string is DERIVED from `value`'s type at
        // lowering, so it is NOT in the surface — recompile rebuilds the identical descriptor from the
        // re-inferred type. `Value.encode` is THE single public canonical encoder (also backs `Ast.encode`).
        Core::ValueEncode { value, .. } => {
            let head = member_access(b, "Value", "encode");
            let v = emit_expr(db, b, value, None, env, emitted)?;
            Ok(b.list(vec![head, v]))
        }
        // `Value.decode b` — the PARTIAL inverse (`∀a. Bytes → (Option a)`). Surface
        // `(: ((. Value decode) <bytes>) (Option <T>))`. The target `a` is grounded by the CALL-SITE
        // expected type; the optimizer can fold a decode to a position that under-determines `a` on
        // recompile, so we ASCRIBE the node's own solved `(Option T)` result type (via `type_ast`). An
        // under-determined result type (`type_ast` returns `None`) DECLINES rather than emit an unsolved
        // decode. `desc`/`disc_some`/`disc_none` are all rebuilt from that type on recompile.
        Core::ValueDecode { bytes, .. } => {
            let ty = crate::infer::type_of(db, id);
            let ncx = db.name_ctx();
            let ty_node = crate::lower::type_ast(b, &ty, &ncx).ok_or_else(|| {
                Reject::unsupported(
                    "the Cadenza backend does not support lowering a `Value.decode` whose result type is \
                     under-determined (an unsolved decode target)"
                        .to_string(),
                )
            })?;
            let head = member_access(b, "Value", "decode");
            let by = emit_expr(db, b, bytes, None, env, emitted)?;
            let call = b.list(vec![head, by]);
            let colon = b.name(":");
            Ok(b.list(vec![colon, call, ty_node]))
        }
        // `Blake3.of b` on a RUNTIME `Bytes` — the blake3 content hash (`Bytes → Bytes`, a 32-byte digest).
        // Surface `((. Blake3 of) <operand>)`. (A CONSTANT `Bytes` folds to a `Core::ConstBytes` in `lower`
        // and never reaches here; this is the runtime path.)
        Core::Blake3Of { operand } => {
            let head = member_access(b, "Blake3", "of");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // A CLOSURE value — a lambda that was LAMBDA-LIFTED (`code` indexes `layout.lifted`, the captures made
        // explicit). Re-emit the ORIGINAL surface `(fn ((: <p> <Ty>)…) <body>)`: the params come from the
        // lifted lambda's `params` (name via `as_name`, type via `type_ast` — the same signature shape a
        // `def` uses), and the body is the lifted body emitted with this lambda's captures in scope. A
        // `Core::Captured` in that body re-emits the enclosing binder's LEXICAL NAME (below), so the
        // reconstructed `(fn …)` re-captures the same bindings; re-lowering re-lifts them (value-eq — a
        // re-lift may reorder, so byte-idempotence is not guaranteed, but the value is preserved). The node's
        // own `captures` value-expressions are NOT needed — the lexical name suffices.
        Core::Closure { code, .. } => {
            let lifted = env
                .lifted
                .as_ref()
                .and_then(|l| l.get(code))
                .cloned()
                .ok_or_else(|| {
                    Reject::decline(
                        "the Cadenza backend has no lifted lambda for a closure code".to_string(),
                    )
                })?;
            let fn_head = b.name("fn");
            // The param signature `((: p0 T0) (: p1 T1) …)` — mirrors `emit_def`'s (immutable `NameCtx`
            // scope, no `&mut db`, so the name reads + `type_ast` compose). A param with no value-form type
            // surface (a function/unsolved type) declines, exactly as a def parameter does.
            let mut sig_children = Vec::with_capacity(lifted.params.len());
            {
                let ncx = db.name_ctx();
                for (binder, ty) in lifted.params.iter() {
                    let pname = db.ast.as_name(*binder).ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend cannot recover a closure parameter name"
                                .to_string(),
                        )
                    })?;
                    let pname_node = b.name(pname);
                    let ty_node = crate::lower::type_ast(b, ty, &ncx).ok_or_else(|| {
                        Reject::unsupported(
                            "the Cadenza backend does not support lowering a closure parameter type (no \
                             value-form surface)"
                                .to_string(),
                        )
                    })?;
                    let colon = b.name(":");
                    sig_children.push(b.list(vec![colon, pname_node, ty_node]));
                }
            }
            let sig = b.list(sig_children);
            // A CAPTURED value that is neither a parameter NAME nor an in-scope `let` (e.g. a runtime Call
            // the optimizer inlined into the capture list without let-binding it) has NO surface name for
            // the body's `Core::Captured` read → the read would decline. HOIST each such capture into a
            // `let` binding wrapped around the `(fn …)`: emit its value ONCE, bind a fresh name, register
            // (capture-node → name) so `Core::Captured` resolves — the surface analogue of the wasm
            // backend's implicit capture (evaluate at closure creation, store in the cell). The result
            // `(let ((c0 <cap0-value>) …) (fn (params) body))` re-lowers to the same closure (c0 is now a
            // let-binder the fn captures). A capture already resolvable (a param / in-scope let) is left
            // as-is. The hoisted names are removed after this closure's body so they do not leak to siblings.
            let mut hoisted: Vec<(std::rc::Rc<str>, StructId)> = Vec::new();
            let mut inserted_keys: Vec<StructId> = Vec::new();
            for &cap in lifted.captures.iter() {
                let resolvable = db.ast.as_name(cap).is_some() || env.lets.contains_key(&cap);
                // SAFETY (why hoisting the capture VALUE is sound, not an effect-doubling miscompile): the ANF
                // effect-sequencing invariant guarantees the optimizer never inlines an EFFECTFUL expression
                // into a capture list — a `perform`/`HostCall` is a SEQUENCED statement (let-bound / in a
                // `Seq`), so a closure captures the (pure) RESULT binder (resolvable via `env.lets`, not
                // hoisted), never a live effectful value. Thus a capture reaching THIS branch (un-resolvable →
                // an inlined value) is a PURE expression; evaluating it once in the hoisted `let` matches the
                // wasm backend's evaluate-once-at-creation capture. (Value-eq; sharing across sibling closures
                // is lost — each hoists its own copy — a benign double-eval, not a wrong value.)
                if !resolvable {
                    let cname = synth_binding_name(env.lets.len());
                    let cap_val = emit_expr(db, b, cap, None, env, emitted)?;
                    env.lets.insert(cap, cname.clone());
                    inserted_keys.push(cap);
                    hoisted.push((cname, cap_val));
                }
            }
            // Emit the lifted body with THIS lambda's captures in scope (save/restore for nesting).
            let saved = env.current_captures.take();
            env.current_captures = Some(lifted.captures.clone().into());
            let body_res = emit_expr(db, b, lifted.body, None, env, emitted);
            env.current_captures = saved;
            for k in &inserted_keys {
                env.lets.remove(k);
            }
            let body_node = body_res?;
            let fn_node = b.list(vec![fn_head, sig, body_node]);
            if hoisted.is_empty() {
                Ok(fn_node)
            } else {
                let let_head = b.name("let");
                let mut binding_nodes = Vec::with_capacity(hoisted.len());
                for (cname, cval) in hoisted {
                    let name_atom = b.name(cname);
                    binding_nodes.push(b.list(vec![name_atom, cval]));
                }
                let bindings_list = b.list(binding_nodes);
                Ok(b.list(vec![let_head, bindings_list, fn_node]))
            }
        }
        // A read of the k-th CAPTURED free variable inside a lifted closure body — re-emit the enclosing
        // binding's LEXICAL surface name (`lifted.captures[index]` is that binding's binder occurrence). A
        // captured PARAMETER resolves via `as_name`; a captured kept-`let` resolves via `env.lets`; anything
        // else declines. Reached only inside the `Core::Closure` body that set `current_captures`.
        Core::Captured { index, .. } => {
            let caps = env.current_captures.clone().ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend reached a captured-variable read outside a closure body"
                        .to_string(),
                )
            })?;
            let binder = *caps.get(index).ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend captured-variable index is out of range".to_string(),
                )
            })?;
            if let Some(nm) = db.ast.as_name(binder) {
                Ok(b.name(nm))
            } else if let Some(nm) = env.lets.get(&binder) {
                Ok(b.name(nm.clone()))
            } else {
                Err(Reject::decline(
                    "the Cadenza backend cannot resolve a captured variable to an in-scope name"
                        .to_string(),
                ))
            }
        }
        // Apply a runtime CLOSURE value at full arity — `(<closure> <arg>…)`. The head emits to a `(fn …)`
        // value or a binder holding one; recompile re-selects the runtime `call_indirect` path.
        Core::CallClosure { closure, args } => {
            let head = emit_expr(db, b, closure, None, env, emitted)?;
            let mut children = Vec::with_capacity(1 + args.len());
            children.push(head);
            for &a in args.iter() {
                children.push(emit_expr(db, b, a, None, env, emitted)?);
            }
            Ok(b.list(children))
        }
        // STRING OPERATIONS — member-access ops `((. String <member>) <op>…)`. `String.at`/`scalar-at`
        // share ONE `Core::StrAt` (both walk the scalar buffer), distinguished by the RESULT's `Option`
        // payload — a `Char` payload came from `scalar-at`, a `String` payload from `at`. The others are 1:1.
        Core::StrScalarLen { operand } => {
            let head = member_access(b, "String", "scalar-len");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::StrAt { string, index, .. } => {
            // `at` (payload `String`) vs `scalar-at` (payload `Char`) — recover from the result Option's arg.
            let member = match crate::infer::type_of(db, id) {
                Ty::Sum { args, .. } if matches!(args.first(), Some(Ty::Char)) => "scalar-at",
                _ => "at",
            };
            let head = member_access(b, "String", member);
            let s = emit_expr(db, b, string, None, env, emitted)?;
            let i = emit_expr(db, b, index, None, env, emitted)?;
            Ok(b.list(vec![head, s, i]))
        }
        // `String.scalar-at` split into its OWN `Core::StrScalarAt` (post-#5928/#5932; distinct from the
        // shared `Core::StrAt` above). It reads the scalar at `index` and returns `Option Char` (the baked
        // `disc_some`/`disc_none` are the Option discriminants — the surface member re-lowers to the same, so
        // they need no re-emit). Re-emit the member `((. String scalar-at) operand index)`: recompiling it
        // re-lowers to the same scalar read + `Option Char`, value-equivalent. Mirrors the `StrAt`/`scalar-at`
        // arm; the direct wasm/rust backends emit this node green (breaker census post-#5932).
        Core::StrScalarAt { operand, index, .. } => {
            let head = member_access(b, "String", "scalar-at");
            let s = emit_expr(db, b, operand, None, env, emitted)?;
            let i = emit_expr(db, b, index, None, env, emitted)?;
            Ok(b.list(vec![head, s, i]))
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            let head = member_access(b, "String", "slice");
            let s = emit_expr(db, b, string, None, env, emitted)?;
            let st = emit_expr(db, b, start, None, env, emitted)?;
            let en = emit_expr(db, b, end, None, env, emitted)?;
            Ok(b.list(vec![head, s, st, en]))
        }
        Core::StrToBytes { string } => {
            // A `StrToBytes` node TYPED `Symbol` is the optimizer's lowering of `Symbol.of(s)` — a symbol's
            // identity IS its normalized bytes, so `Symbol.of` folds to `StrToBytes` while the node keeps its
            // `Ty::Symbol`. Re-emit it as `(Symbol.of s)`, NOT `(String.to-bytes s)`: emitting `to-bytes` would
            // yield a `Bytes` value, breaking arm unification in an enclosing match (or any Symbol-typed
            // context) so the re-emit fails to type-check (13-strings/0057: a `Symbol.of(s)` arm sibling to a
            // `Symbol` constant arm emitted as Bytes → hop² CDZ0203 "match arms differ: Bytes vs Symbol").
            let member = if matches!(eff_ty, Ty::Symbol) {
                ("Symbol", "of")
            } else {
                ("String", "to-bytes")
            };
            let head = member_access(b, member.0, member.1);
            let s = emit_expr(db, b, string, None, env, emitted)?;
            Ok(b.list(vec![head, s]))
        }
        // NFC normalization is COMPILER-INSERTED (no surface member) — `String.concat` lowers to
        // `NfcNormalize(bytes-concat …)` and `String.to-bytes` to `StrToBytes(NfcNormalize s)` (see
        // `lower.rs`; for a constant/ASCII operand NFC is identity and folds away, so this only reaches the
        // backend for a RUNTIME string op). Emit the inner string TRANSPARENTLY: the surrounding surface op
        // (`String.concat`/`to-bytes`) RE-INSERTS the same normalization on recompile, so dropping the
        // explicit node round-trips (NFC is deterministic + idempotent — same op, same normalize).
        Core::NfcNormalize { string } => emit_expr(db, b, string, expected, env, emitted),
        Core::StrFromBytes { bytes, .. } => {
            let head = member_access(b, "String", "from-bytes");
            let by = emit_expr(db, b, bytes, None, env, emitted)?;
            Ok(b.list(vec![head, by]))
        }
        // CHAR CONVERSIONS — member-access ops `((. Char <member>) <operand>)`. `Char.to-int : Char →
        // Int64` (total scalar-value read); `Char.from-int : Int64 → (Option Char)` (fallible). A CONSTANT
        // operand folds in `lower` (to a `ConstInt` / `Some #\c`|`None`) and never reaches here, so a
        // surviving node is a genuinely-runtime char/int (a param/local/`if`-join). Both surfaces have a
        // FIXED result type from the member signature, so no ascription is needed — recompile re-derives
        // the same node (byte-idempotent). The dropped `disc_some`/`disc_none` on `from-int` are the
        // built-in `Option` discriminants, rebuilt on recompile from the prelude.
        Core::CharToInt { operand } => {
            let head = member_access(b, "Char", "to-int");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        Core::IntToCharChecked { operand, .. } => {
            let head = member_access(b, "Char", "from-int");
            let x = emit_expr(db, b, operand, None, env, emitted)?;
            Ok(b.list(vec![head, x]))
        }
        // `Option.expect` / `Result.expect` — unwrap the present variant's payload or TRAP on absence. The
        // surface `((. <Module> expect) <scrutinee> <message>)`; the MODULE (`Option`/`Result`) is recovered
        // from the scrutinee's solved sum declaration NAME. The `"message"` operand was DROPPED at lowering
        // (`Core::SumExpect` carries no text — the wasm trap is textless and the corpus grades on the TRAP,
        // not its message), so a placeholder `""` is re-emitted: it round-trips (re-lowers to the same
        // `SumExpect`, message dropped again — byte-idempotent) and is value-equivalent (present → the
        // payload, unaffected by the message; absent → the same textless trap).
        Core::SumExpect { scrutinee, .. } => {
            let sty = crate::infer::type_of(db, scrutinee);
            let module = match &sty {
                Ty::Sum { decl, .. } => match db.type_decl_by_occ(*decl).map(|t| t.name.as_str()) {
                    Some("Option") => "Option",
                    Some("Result") => "Result",
                    _ => {
                        return Err(Reject::decline(
                            "the Cadenza backend lowers `expect` only over `Option` / `Result`"
                                .to_string(),
                        ));
                    }
                },
                _ => {
                    return Err(Reject::decline(
                        "the Cadenza backend cannot recover the module for an `expect` over a non-sum"
                            .to_string(),
                    ));
                }
            };
            let head = member_access(b, module, "expect");
            let s = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let msg = b.atom_leaf(Leaf::Str("".into()));
            Ok(b.list(vec![head, s, msg]))
        }
        // A tuple PROJECTION `(. <operand> <index>)` — read element `index` of a runtime tuple (a
        // projection of a compile-time tuple folds in `lower`). The index is a compile-time constant, an
        // `Int` leaf; the member-access reader accepts an integer key for a positional tuple read.
        Core::Proj { operand, index } => {
            // If the operand is a compile-time-visible compound LITERAL, resolve the projection at the CORE
            // level and emit the element node directly — the FOLDED form recompile's projection-fold
            // produces. Emitting `(. <literal> field)` would leave hop¹ un-folded but hop² folded (a
            // byte-idempotence break: breaker #5137, a nested record destructure whose scrutinee inlined to
            // a record literal, so the projection is materialized AFTER the fold pass). A runtime operand
            // (a binder / call / …) falls to the `(. <operand> <key>)` surface projection below.
            match core_of(db, operand) {
                Core::Tuple { elems } => {
                    if let Some(&e) = elems.get(index) {
                        return emit_expr(db, b, e, expected, env, emitted);
                    }
                }
                Core::Record { fields } => {
                    if let Some((_, &v)) = fields.iter().nth(index) {
                        return emit_expr(db, b, v, expected, env, emitted);
                    }
                }
                _ => {}
            }
            // A projection's surface KEY depends on the operand's compound kind: a TUPLE projects by the
            // positional slot `(. t 1)`, but a RECORD projects by the FIELD NAME `(. r val)`. Records are
            // keyed by name (canonical `BTreeMap` sorted order), so the field name at slot `index` is the
            // `index`-th key. Emitting the positional index for a record is NON-re-compilable — `(. r 1)`
            // re-reads as a tuple projection → CDZ0201 "tuple projection requires a tuple, found (Record …)".
            let operand_ty = crate::infer::type_of(db, operand);
            // A projection whose OPERAND is a BINDER declared a single-variant NOMINAL (`(. w i)` where
            // `w : WrapT`, `type WrapT = | MkWT (Tuple …)`): the optimizer folds the irrefutable `(MkWT t)`
            // match away, so the projection reads `w` directly. The Core node type is the ERASED inner tuple,
            // but `w`'s SURFACE-declared type is the nominal, and surface `.` cannot cross it (`(. w i)`
            // recompiles to CDZ0201 "tuple projection requires a tuple, found WrapT" — breaker). Detect it by
            // the BINDER's declared type (not the erased node type), and peel via a ctor pattern
            // ([`emit_nominal_elem_peel`]): `(match w ((MkWT t) (. t i)))`.
            if let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, operand)
                && let Ty::Nominal { decl, inner, .. } = crate::infer::type_of(db, binder)
                && emitted.contains(&decl)
                && db
                    .type_decl_by_occ(decl)
                    .is_some_and(|t| t.variants.len() == 1)
            {
                let op = emit_expr(db, b, operand, None, env, emitted)?;
                let (n, _) = emit_nominal_elem_peel(db, b, op, index, decl, &inner, env, emitted)?;
                return Ok(n);
            }
            let dot = b.name(".");
            let op = emit_expr(db, b, operand, None, env, emitted)?;
            let key = match &operand_ty {
                Ty::Record(fields) => {
                    let name = fields.keys().nth(index).ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend could not recover the record field name for a \
                             projection (slot out of range)"
                                .to_string(),
                        )
                    })?;
                    b.name(&*name.name)
                }
                _ => b.atom_leaf(Leaf::Int {
                    value: IntValue::from_i64(index as i64),
                    radix: Radix::Dec,
                }),
            };
            Ok(b.list(vec![dot, op, key]))
        }
        // Structural EQUALITY on a runtime compound — `(= <lhs> <rhs>)`. `ValueEq`/`ValueEqShaped` (the
        // shaped form carries a descriptor `ty`, display-neutral) both re-emit the surface `=`; on recompile
        // the operands' compound type re-selects the `value-eq` path (a scalar `=` would re-select `Compare`,
        // a constant pair would fold — all value-equivalent). Mirrors the operator arm's `=`.
        Core::ValueEq { lhs, rhs } | Core::ValueEqShaped { lhs, rhs, .. } => {
            let head = b.name("=");
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // Structural ORDERING on a runtime compound — `(<op> <lhs> <rhs>)` where `op` is an ordering prim
        // (`Lt`/`Le`/`Gt`/`Ge`), re-emitted via the same `prim_operator` reverse-map the scalar comparisons
        // use; the operands' compound type re-selects the `value-cmp` path on recompile.
        Core::ValueCmp { op, lhs, rhs, .. } => {
            // A THREE-WAY compound compare is the member `(Ordering.of l r)` (like the scalar case); a
            // boolean compound ordering (`Lt`/`Le`/`Gt`/`Ge`) is the plain operator via `prim_operator`.
            let head = if op == crate::resolved::Prim::Compare {
                member_access(b, "Ordering", "of")
            } else {
                let sym = prim_operator(op).ok_or_else(|| {
                    Reject::unsupported(format!(
                        "the Cadenza backend does not support lowering the value-compare prim {op:?}"
                    ))
                })?;
                b.name(sym)
            };
            let l = emit_expr(db, b, lhs, None, env, emitted)?;
            let r = emit_expr(db, b, rhs, None, env, emitted)?;
            Ok(b.list(vec![head, l, r]))
        }
        // An unconditional TRAP — `(trap "")`. `Core::Trap` is the generic `unreachable` (from `Prim::Trap`
        // with its message dropped, an unreachable match, or a demoted provable trap). The message operand
        // is dropped at lowering (the wasm trap is textless, graded on the trap not its text), so a
        // placeholder `""` re-emits: `(trap "")` re-lowers to `Core::Trap` (byte-idempotent) and traps the
        // same. (`TrapOverflow` is a distinct wasm trap kind — a later slice — declined below; `TrapDivZero`
        // is re-emitted kind-preservingly just after.)
        Core::Trap => {
            let head = b.name("trap");
            let msg = b.atom_leaf(Leaf::Str("".into()));
            Ok(b.list(vec![head, msg]))
        }
        // A KIND-PRESERVING const divide-by-zero trap. `Core::TrapDivZero` is the demote target for a
        // fold-provable const `(/ x 0)` / `(% x 0)` in a CONDITIONALLY-reached `if` branch / `match` arm
        // (`lower::demote_conditional_trap`) — the operator ruled (2026-08-27) the demote MUST preserve the
        // "divide by zero" kind, not collapse to a generic `unreachable`. So re-emit the SOURCE-shape const
        // div-by-zero `(: (/ 1 0) <IntTy>)` ascribed to the node's fixed-width int type: it lands in the SAME
        // conditional position (a TrapDivZero is ONLY produced there), where `demote_conditional_trap`
        // re-demotes it to `Core::TrapDivZero` of the SAME kind + width — so the round-tripped program traps
        // identically (a bare `(trap "")` would trap "unreachable", the WRONG kind, and — since a decline is
        // SKIPPED by the gate but a mismatched trap FAILS it — would be worse than declining). A non-fixed-width
        // integer type (e.g. `BigInt`) declines (a later slice).
        Core::TrapDivZero if matches!(eff_ty, Ty::Int(_)) => {
            let tyname = eff_ty.render_name(&db.name_ctx());
            let slash = b.name("/");
            let one = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(1),
                radix: Radix::Dec,
            });
            let zero = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(0),
                radix: Radix::Dec,
            });
            let div = b.list(vec![slash, one, zero]);
            let colon = b.name(":");
            let ty_node = b.name(tyname.as_str());
            Ok(b.list(vec![colon, div, ty_node]))
        }
        // A KIND-PRESERVING const INTEGER-OVERFLOW trap — the overflow twin of `TrapDivZero` (same operator
        // ruling). Re-emit the canonical const overflow `(: (/ <MIN> -1) <IntTy>)`: the minimum signed value
        // divided by -1 overflows at EVERY signed width (`MIN / -1 = 2^(w-1)`, one past `MAX`), and in the SAME
        // conditionally-reached position `demote_conditional_trap` re-demotes it to `Core::TrapOverflow` of the
        // same kind — the round-tripped program traps "integer overflow" identically (this is exactly the corpus
        // shape `(/ -9223372036854775808 -1)`). Guarded to a SIGNED, ≤64-bit int (the shape `(/ MIN -1)` needs a
        // signed MIN, and the i128 `MIN` literal is exact for w ≤ 64); an unsigned / wider / non-int type
        // declines (a later slice — unsigned overflow needs a different const shape).
        Core::TrapOverflow if matches!(&eff_ty, Ty::Int(it) if it.ground_signed() && it.ground_width() <= 64) =>
        {
            let width = match &eff_ty {
                Ty::Int(it) => it.ground_width(),
                _ => 64,
            };
            let min = -(1i128 << (width - 1));
            let tyname = eff_ty.render_name(&db.name_ctx());
            let slash = b.name("/");
            let min_lit = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i128(min),
                radix: Radix::Dec,
            });
            let neg_one = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(-1),
                radix: Radix::Dec,
            });
            let div = b.list(vec![slash, min_lit, neg_one]);
            let colon = b.name(":");
            let ty_node = b.name(tyname.as_str());
            Ok(b.list(vec![colon, div, ty_node]))
        }
        // A SEQUENCING BLOCK — statements evaluated in written order for their observable host effects, then
        // the TAIL is the block's value. Re-emit `(do <stmt>… <tail>)`. A `Core::Seq` is built ONLY when a
        // non-final statement reaches a host call (`lower/compute.rs` §needs_seq — otherwise the do-block folds
        // to its tail), and the surface `do` re-lowers to a `Core::Seq` under that SAME condition; since the
        // re-emitted statements still reach their host calls, the block round-trips (re-forms an equivalent
        // Seq). Statements emit at `None` (their value is discarded); the tail carries the block's `expected`
        // type. The per-def `(host …)` wrapper + perform⇔decl coupling (#7268) carry the statements' effects —
        // an effect whose decl is not re-emittable declines its perform, so the whole Seq declines cleanly.
        Core::Seq { stmts, tail } => {
            let do_head = b.name("do");
            let mut children = Vec::with_capacity(stmts.len() + 2);
            children.push(do_head);
            for &s in stmts.iter() {
                children.push(emit_expr(db, b, s, None, env, emitted)?);
            }
            children.push(emit_expr(db, b, tail, expected.clone(), env, emitted)?);
            Ok(b.list(children))
        }
        other => Err(Reject::unsupported(format!(
            "the Cadenza backend does not support lowering this Core node back to Cadenza: {}",
            core_node_kind(&other)
        ))),
    }
}

/// Build ONE surface arm PATTERN over the value of type `ty` at `path` (from the root scrutinee), given the
/// per-path variant `choices` a decision-tree leaf-path fixed (v-wasm-opt review #4 — the general un-flatten).
/// `choices[P] = Some(disc)` = the value at P was switched to variant `disc` (refutable → `(<V> <sub-pats>)`);
/// `choices[P] = None` = the DEFAULT arm at that switch (any other variant → `_`). No entry at `path` = no
/// switch here: if a DEEPER choice exists under `path`, destructure the IRREFUTABLE structure to reach it (a
/// single-variant sum/newtype → its ctor; a tuple → `#tuple(…)`; a record → `#record(…)`) — the recipe's
/// irrefutable-in-pattern step; else it is a LEAF, bound to a fresh name (keyed at the FULL `path` — the exact
/// `Core::SumPayload` key the body reads). A payload slot `k` of variant `disc` sits at `path ++ [Payload {,
/// Elem(k)}]` — the same keying `emit_match_sum` / `emit_nested_switch_chain` use.
#[allow(clippy::too_many_arguments)]
fn build_arm_pat(
    db: &mut Db,
    b: &mut Builder,
    root_scrut: StructId,
    ty: &Ty,
    // `path` = the SWITCH path (keys the `choices` map + the `Core::Switch` paths — a `Payload` into a
    // single-variant sum IS kept here, matching the decision tree). `read_path` = the BODY-read path (keys
    // `env.payloads` + `Core::SumPayload` reads — a `Payload` into a single-variant sum is ELIDED here, because
    // the newtype box erases at runtime, so a body reads its payload directly). The two DIFFER by exactly the
    // erased-newtype `Payload` steps — keying a leaf under the wrong one = a missed binder → a false decline.
    path: &[crate::core::PathStep],
    read_path: &[crate::core::PathStep],
    choices: &std::collections::HashMap<Vec<crate::core::PathStep>, Option<u32>>,
    // A LITERAL-at-a-path refinement from a `LitTest` slot. `lit_choices[P]` is:
    //   `Some(lit)` — emit the pre-built literal PATTERN node at path `P` (the `7` in `#tuple(a 7)`): the
    //     matched `then_` sub-tree, where the slot is fixed to the literal.
    //   `None` — a FREED slot: the fall-through `els` sub-tree does NOT fix this slot, but the tuple/record
    //     around it must still be DESTRUCTURED so the freed slot (and its siblings) get fresh binders the
    //     body reads (`#tuple(a k)`). `has_deeper` scans these keys, so an ancestor path destructures down
    //     to reach it, and the slot itself falls through to a fresh leaf-bind.
    // Kept SEPARATE from `choices` (variant discriminants) — a LitTest tests a scalar value at a leaf, a
    // Switch tests a discriminant; the two path sets are disjoint. Emitting the literal directly (vs a
    // `(= b lit)` guard) keeps the round-trip IDEMPOTENT: `#tuple(a 7)` re-lowers straight back to a
    // `LitTest` (a guard would re-lower to a `Guarded`, which this tree-walk declines → a hop1≠hop2 split).
    lit_choices: &std::collections::HashMap<Vec<crate::core::PathStep>, Option<StructId>>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    use crate::core::PathStep;
    // Emit `(<Ctor of decl@disc> <slot0-pat> … )`, recursing each payload slot. SWITCH path always adds `[Payload
    // {,Elem(k)}]`; READ path adds `[Payload {,Elem(k)}]` for a MULTI-variant sum but ELIDES the `Payload` for a
    // SINGLE-variant sum (erased newtype — the body reads the payload without a sum-payload step).
    #[allow(clippy::too_many_arguments)]
    fn ctor_pat(
        db: &mut Db,
        b: &mut Builder,
        root_scrut: StructId,
        parent_ty: &Ty,
        decl: StructId,
        disc: u32,
        path: &[PathStep],
        read_path: &[PathStep],
        choices: &std::collections::HashMap<Vec<PathStep>, Option<u32>>,
        lit_choices: &std::collections::HashMap<Vec<PathStep>, Option<StructId>>,
        env: &mut BinderEnv,
        emitted: &std::collections::HashSet<StructId>,
    ) -> Result<StructId, Reject> {
        if db.is_user_node(decl) && !emitted.contains(&decl) {
            return Err(Reject::unsupported(
                "the Cadenza backend does not support re-emitting a deep nested match over an \
                 un-emitted user sum"
                    .to_string(),
            ));
        }
        let single = db
            .type_decl_by_occ(decl)
            .is_some_and(|t| t.variants.len() == 1);
        let arity = db
            .type_decl_by_occ(decl)
            .and_then(|t| t.variants.get(disc as usize))
            .map(|v| v.payloads.len())
            .ok_or_else(|| {
                Reject::decline("the Cadenza backend could not recover a variant arity".to_string())
            })?;
        // Payload types: `sum_payload_expected` returns a `Tuple` of the slots (multi) / the sole type (arity 1).
        let pay = sum_payload_expected(db, decl, disc, parent_ty);
        let head = crate::lower::variant_head_ast(db, b, decl, disc).ok_or_else(|| {
            Reject::decline("the Cadenza backend could not recover a variant name".to_string())
        })?;
        let mut children = vec![head];
        for k in 0..arity {
            let mut kpath = path.to_vec();
            kpath.push(PathStep::Payload);
            if arity > 1 {
                kpath.push(PathStep::Elem(k));
            }
            let mut kread = read_path.to_vec();
            if !single {
                kread.push(PathStep::Payload);
            }
            if arity > 1 {
                kread.push(PathStep::Elem(k));
            }
            let kty = match (&pay, arity) {
                (Some(Ty::Tuple(ts)), n) if n > 1 => ts.get(k).cloned().unwrap_or(Ty::Any),
                (Some(t), 1) => t.clone(),
                _ => Ty::Any,
            };
            children.push(build_arm_pat(
                db,
                b,
                root_scrut,
                &kty,
                &kpath,
                &kread,
                choices,
                lit_choices,
                env,
                emitted,
            )?);
        }
        Ok(b.list(children))
    }

    // A LITERAL refinement at THIS path (a `LitTest` slot fixed to a literal) — emit the literal pattern
    // itself (`7` in `#tuple(a 7)`); no binder, the value is fixed. A `None` entry (a freed `els` slot)
    // does NOT return here — it falls through to the leaf-bind below (a fresh binder). Checked before
    // `choices`: a LitTest path is a scalar leaf, never a discriminant switch, so the two never collide.
    if let Some(Some(lit)) = lit_choices.get(path) {
        return Ok(*lit);
    }
    if let Some(choice) = choices.get(path) {
        return match choice {
            None => Ok(b.name("_")),
            Some(disc) => {
                let decl = match ty {
                    Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => *decl,
                    _ => {
                        return Err(Reject::decline(
                            "the Cadenza backend deep-match choice is over a non-sum value"
                                .to_string(),
                        ));
                    }
                };
                ctor_pat(
                    db,
                    b,
                    root_scrut,
                    ty,
                    decl,
                    *disc,
                    path,
                    read_path,
                    choices,
                    lit_choices,
                    env,
                    emitted,
                )
            }
        };
    }
    // No switch at THIS path. Reach a deeper choice (a discriminant Switch OR a literal LitTest below
    // here) by destructuring the irrefutable structure, else bind a leaf.
    let has_deeper = choices
        .keys()
        .chain(lit_choices.keys())
        .any(|k| k.len() > path.len() && k.starts_with(path));
    if has_deeper {
        match ty {
            // A SINGLE-VARIANT sum / newtype is irrefutable — its ctor crosses to the payload (no branch).
            Ty::Nominal { decl, .. } => {
                let decl = *decl;
                return ctor_pat(
                    db,
                    b,
                    root_scrut,
                    ty,
                    decl,
                    0,
                    path,
                    read_path,
                    choices,
                    lit_choices,
                    env,
                    emitted,
                );
            }
            Ty::Sum { decl, .. }
                if db
                    .type_decl_by_occ(*decl)
                    .is_some_and(|t| t.variants.len() == 1) =>
            {
                let decl = *decl;
                return ctor_pat(
                    db,
                    b,
                    root_scrut,
                    ty,
                    decl,
                    0,
                    path,
                    read_path,
                    choices,
                    lit_choices,
                    env,
                    emitted,
                );
            }
            Ty::Tuple(ts) => {
                let ts = ts.clone();
                let mut children = Vec::with_capacity(ts.len());
                for (i, et) in ts.iter().enumerate() {
                    let mut ep = path.to_vec();
                    ep.push(PathStep::Elem(i));
                    let mut er = read_path.to_vec();
                    er.push(PathStep::Elem(i));
                    children.push(build_arm_pat(
                        db,
                        b,
                        root_scrut,
                        et,
                        &ep,
                        &er,
                        choices,
                        lit_choices,
                        env,
                        emitted,
                    )?);
                }
                return Ok(b.compound(crate::ast::CompoundCtor::Tuple, &children));
            }
            Ty::Record(fields) => {
                let items: Vec<(String, Ty)> = fields
                    .iter()
                    .map(|(k, v)| (k.name.to_string(), v.clone()))
                    .collect();
                let mut children = Vec::with_capacity(items.len());
                for (i, (fname, fty)) in items.iter().enumerate() {
                    let mut ep = path.to_vec();
                    ep.push(PathStep::Elem(i));
                    let mut er = read_path.to_vec();
                    er.push(PathStep::Elem(i));
                    let vpat = build_arm_pat(
                        db,
                        b,
                        root_scrut,
                        fty,
                        &ep,
                        &er,
                        choices,
                        lit_choices,
                        env,
                        emitted,
                    )?;
                    let kn = b.name(fname.as_str());
                    children.push(b.field_pair(kn, vpat));
                }
                return Ok(b.compound(crate::ast::CompoundCtor::Record, &children));
            }
            _ => {
                return Err(Reject::decline(
                    "the Cadenza backend cannot destructure this value to reach a deep-match constraint"
                        .to_string(),
                ));
            }
        }
    }
    // A LEAF position — bind a fresh name at the READ path (the exact key the body's `Core::SumPayload` uses).
    let nm = synth_payload_name(env.next_payload);
    env.next_payload += 1;
    // Record the leaf's TYPE too (for the erased-newtype read peel — see `BinderEnv::payload_tys`).
    env.payload_tys
        .insert((root_scrut, read_path.to_vec()), ty.clone());
    env.payloads
        .insert((root_scrut, read_path.to_vec()), nm.clone());
    Ok(b.name(nm))
}

/// Reconstruct surface arms for a deep decision-`SumCont` tree under one outer variant (v-wasm-opt review #4):
/// walk each `Switch` arm, threading a `path -> variant-choice` map; at each `Leaf`, [`build_arm_pat`] emits
/// ONE surface arm `(<deep-pattern> <body>)` reflecting that leaf-path's choices (an explicit variant becomes a
/// sub-pattern, a default becomes `_`). A `LitTest` refines a scalar slot to a literal: it emits the literal
/// IN the pattern (`#tuple(a 7)`) for the `then_` sub-tree and threads the fall-through `els` unrefined, so the
/// round-trip stays idempotent (a literal pattern re-lowers to a `LitTest`, whereas a `(= b lit)` guard would
/// re-lower to a `Guarded` this walk declines). Enumerating every leaf keeps the emitted match exhaustive
/// without a synthetic wildcard. A `Guarded` node, or a non-scalar `LitTest` probe, still declines (a later slice).
#[allow(clippy::too_many_arguments)]
fn emit_switch_tree(
    db: &mut Db,
    b: &mut Builder,
    root_scrut: StructId,
    root_ty: &Ty,
    cont: &crate::core::SumCont,
    choices: std::collections::HashMap<Vec<crate::core::PathStep>, Option<u32>>,
    lit_choices: std::collections::HashMap<Vec<crate::core::PathStep>, Option<StructId>>,
    expected: &Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
    children: &mut Vec<StructId>,
) -> Result<(), Reject> {
    use crate::core::SumCont;
    match cont {
        SumCont::Leaf(body) => {
            let pat = build_arm_pat(
                db,
                b,
                root_scrut,
                root_ty,
                &[],
                &[],
                &choices,
                &lit_choices,
                env,
                emitted,
            )?;
            let body_node = emit_expr(db, b, *body, expected.clone(), env, emitted)?;
            children.push(b.list(vec![pat, body_node]));
            Ok(())
        }
        SumCont::Switch { path, arms } => {
            for arm in arms {
                let mut c = choices.clone();
                c.insert(path.to_vec(), arm.disc);
                emit_switch_tree(
                    db,
                    b,
                    root_scrut,
                    root_ty,
                    &arm.cont,
                    c,
                    lit_choices.clone(),
                    expected,
                    env,
                    emitted,
                    children,
                )?;
            }
            Ok(())
        }
        // A LITERAL-PAYLOAD test on a scalar slot at `path` — `(#tuple(a 7) …)` / `(#record((= f 7)) …)`.
        // Emit the literal IN the pattern for the matched `then_` sub-tree (fixing the slot to the literal),
        // then the fall-through `els` sub-tree UNREFINED (the slot binds / is further tested there). The two
        // become adjacent surface arms — the specific literal arm first, the general fall-through after — so
        // the surface matcher's top-to-bottom order reproduces the LitTest's then/else, and the fall-through
        // keeps the match exhaustive. Only a scalar Int/Bool probe reconstructs (mirrors the own-payload
        // LitTest arm in `emit_match_sum`); any other probe kind, or a deeper nested `then_`/`els`, is handled
        // by the recursion or declines there.
        SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            let lit = match probe {
                crate::core::Probe::Int(v) => b.atom_leaf(Leaf::Int {
                    value: v.clone(),
                    radix: Radix::Dec,
                }),
                crate::core::Probe::Bool(x) => b.atom_leaf(Leaf::Bool(*x)),
                // A STRING / CHAR literal probe (`(#tuple(a "hi") …)` / `(#tuple(a #\z) …)`): emit the literal
                // IN the pattern, exactly as Int/Bool — the recompile re-lowers it to the same `LitTest` and
                // (for the constant-foldable scrutinee that survives to a backend) re-folds it, matching the
                // direct wasm path (which folds the constant string/char match). A runtime Str/Char scrutinee
                // never reaches a backend (folds or declines in lowering, both backends alike), so this only
                // reconstructs what actually arrives here. Keeps the round-trip idempotent (literal → LitTest).
                crate::core::Probe::Str(s) => b.atom_leaf(Leaf::Str(s.as_str().into())),
                crate::core::Probe::Char(c) => b.atom_leaf(Leaf::Char(*c)),
                _ => {
                    // Reconstructing a Bytes / ListLen / MapHasKeys slot probe is a future slice; that
                    // not-yet intent stays in this comment, NOT the user-facing message (operator seq-280).
                    return Err(Reject::unsupported(
                        "the Cadenza backend reconstructs a literal-at-slot test only for an Int / Bool / \
                         Str / Char probe (a Bytes / ListLen / MapHasKeys slot probe is not supported)"
                            .to_string(),
                    ));
                }
            };
            // then_: fix the slot to the literal (`Some(lit)` → emitted in the pattern).
            let mut lc_then = lit_choices.clone();
            lc_then.insert(path.to_vec(), Some(lit));
            emit_switch_tree(
                db,
                b,
                root_scrut,
                root_ty,
                then_,
                choices.clone(),
                lc_then,
                expected,
                env,
                emitted,
                children,
            )?;
            // els: the slot is FREED (`None` → destructure-to + fresh binder, so the fall-through pattern
            // `#tuple(a k)` binds it and its siblings; the arm follows the literal arm, keeping order +
            // exhaustiveness).
            let mut lc_els = lit_choices.clone();
            lc_els.insert(path.to_vec(), None);
            emit_switch_tree(
                db, b, root_scrut, root_ty, els, choices, lc_els, expected, env, emitted, children,
            )?;
            Ok(())
        }
        // A GUARDED arm on the compound destructure at this leaf — `((guard #tuple(a k) <cond>) <body>)`.
        // Build the arm pattern for the choices/lits fixed so far (the compound destructure, which REGISTERS
        // the slot binders `a`/`k`), THEN emit `<cond>` (it reads those binders — in scope now) wrapped as
        // `(guard <pattern> <cond>)`, then the matched `<body>`; FOLLOWED by the fall-through `els` arms (the
        // remaining rows of the SAME compound shape — a later guard or the covering unguarded arm), exactly as
        // the own-discriminant arm-loop threads a false guard's else. The unguarded fall-through keeps the
        // match exhaustive (a guarded arm does not count). Idempotent: recompiling `(guard <pat> <cond>)`
        // re-lowers to a `Guarded` → this same arm.
        SumCont::Guarded { cond, body, els } => {
            // A guard's cond + body read the destructured SLOT binders, but there is no switch/lit choice
            // forcing `build_arm_pat` to destructure the root compound — so seed a FREED-slot (`None`) marker
            // for each top-level slot of a Tuple/Record root, exactly as the LitTest `els` fall-through does.
            // That destructures `#tuple(a k)` / `#record((= f a)…)` and binds every slot, so the cond/body's
            // `SumPayload[Elem(i)]` reads resolve to their binders (else they re-project the scrutinee — a
            // value-correct but double-eval emit). Threaded to BOTH the guard arm's pattern AND the `els`
            // fall-through (its arms read the same slots). A non-compound root (bare sum) seeds nothing → the
            // whole value binds as one name (the guard reads it whole).
            use crate::core::PathStep;
            let mut lc = lit_choices.clone();
            match root_ty {
                Ty::Tuple(ts) => {
                    for i in 0..ts.len() {
                        lc.entry(vec![PathStep::Elem(i)]).or_insert(None);
                    }
                }
                Ty::Record(fs) => {
                    for i in 0..fs.len() {
                        lc.entry(vec![PathStep::Elem(i)]).or_insert(None);
                    }
                }
                _ => {}
            }
            let pat = build_arm_pat(
                db,
                b,
                root_scrut,
                root_ty,
                &[],
                &[],
                &choices,
                &lc,
                env,
                emitted,
            )?;
            let cond_node = emit_expr(db, b, *cond, None, env, emitted)?;
            let guard_head = b.name("guard");
            let guard_pat = b.list(vec![guard_head, pat, cond_node]);
            let body_node = emit_expr(db, b, *body, expected.clone(), env, emitted)?;
            children.push(b.list(vec![guard_pat, body_node]));
            emit_switch_tree(
                db, b, root_scrut, root_ty, els, choices, lc, expected, env, emitted, children,
            )?;
            Ok(())
        }
    }
}

/// Reconstruct the surface `(match <scrutinee> (<pattern> <body>)…)` for a `Core::MatchSum`. M4a lowers the
/// SIMPLE decision-tree shape: the `root` is a [`SumCont::Switch`] on the scrutinee's OWN discriminant
/// (empty `path`), every arm dispatches on an EXPLICIT variant (`disc: Some`) to a bare [`SumCont::Leaf`]
/// body. A root `Switch` at a NON-EMPTY path — a COMPOUND scrutinee (Tuple/Record/newtype) dispatching on a
/// SLOT — routes through [`emit_switch_tree`]/[`build_arm_pat`] (destructure the compound to reach + dispatch
/// on the switched slot). Anything richer declines (a later slice): a disc-FOLDED / GUARDED / LITERAL-TEST
/// root continuation. Each arm's `(<Variant> <binder>…)` pattern mints
/// one fresh `_cdz_m<n>` binder per payload slot (recorded in `env.payloads` under the same `(scrutinee,
/// path)` key a `Core::SumPayload` in the body carries — `[Payload]` for a single-payload variant,
/// `[Payload, Elem(i)]` for slot `i` of a multi-payload variant, mirroring `select.rs`), so a payload read
/// resolves to its binder. A match over a USER sum whose `(type …)` was not re-emitted declines (its variant
/// heads must resolve on recompile); a prelude sum (`Option`/`Result`) is ambient. The scrutinee is emitted
/// ONCE (a match evaluates it once). Because every arm is an explicit variant, the emitted match covers the
/// same variant set the original did — it stays exhaustive (no CDZ0210), no synthesized wildcard needed.
fn emit_match_sum(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    root: &crate::core::SumCont,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    use crate::core::{PathStep, SumCont};
    // LEAF ROOT whose body reads the scrutinee's ELEMENTS by position. The optimizer collapsed the match to
    // one irrefutable arm but kept the `MatchSum` wrapper (`lower.rs` §evaluate-once: the scrutinee is a
    // non-reusable Call / recursive result); the body reads `Core::SumPayload{scrutinee, [Elem(i)]}` — a BARE
    // `[Elem(i)]` (traced: 18×[Elem(0)]+7×[Elem(1)], NO `Payload` step → NOT a variant read, so no `folded`
    // needed). Reconstruct the ONE arm destructuring the scrutinee at each `[Elem(i)]`, so the body's reads
    // resolve to pattern binders off the ONCE-evaluated scrutinee (no re-emit → no double-eval / exponential
    // re-eval the wrapper exists to prevent). Two irrefutable scrutinee shapes:
    //   · `Ty::Tuple`  → `(match s (#tuple(_b0 _b1 …) <body>))`, bind slot i at `[Elem(i)]`.
    //   · a SINGLE-variant `Ty::Sum`/`Ty::Nominal` (erased newtype; its payload is read by `[Elem(i)]`, the
    //     `Payload` step elided) → `(match s ((Ctor _b0 _b1 …) <body>))`, bind slot i at `[Elem(i)]`.
    // (A MULTI-variant sum or a Record falls through — a bare `[Elem]` over a multi-variant sum isn't
    // destructurable without a variant, and Records key by field not position.) The traced reads are all
    // length-1, so one-level binding is EXACT; a deeper read would fall through to re-emit — the corpus-cadenza
    // A/B gate would flag any such value regression.
    if let SumCont::Leaf(body) = root {
        let sty = crate::infer::type_of(db, scrutinee);
        // The number of positional slots to bind, and the surface CTOR HEAD (`Some` = a single-variant sum
        // ctor `(Ctor …)`; `None` = a native `#tuple(…)`). `None` for a non-destructurable shape (multi-variant
        // sum / record / scalar) → fall through to the normal dispatch/decline.
        let plan: Option<(usize, Option<StructId>)> = match &sty {
            Ty::Tuple(ts) if !ts.is_empty() => Some((ts.len(), None)),
            Ty::Sum { decl, .. } | Ty::Nominal { decl, .. }
                if db
                    .type_decl_by_occ(*decl)
                    .is_some_and(|t| t.variants.len() == 1) =>
            {
                let decl = *decl;
                let arity = db
                    .type_decl_by_occ(decl)
                    .and_then(|t| t.variants.first())
                    .map(|v| v.payloads.len())
                    .unwrap_or(0);
                match (arity, crate::lower::variant_head_ast(db, b, decl, 0)) {
                    (0, _) | (_, None) => None,
                    (n, Some(head)) => Some((n, Some(head))),
                }
            }
            _ => None,
        };
        if let Some((n_slots, ctor_head)) = plan {
            let match_head = b.name("match");
            let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let mut binders = Vec::with_capacity(n_slots);
            for i in 0..n_slots {
                let nm = synth_payload_name(env.next_payload);
                env.next_payload += 1;
                env.payloads
                    .insert((scrutinee, vec![PathStep::Elem(i)]), nm.clone());
                binders.push(b.name(nm));
            }
            // `(Ctor b0 b1 …)` for a single-variant sum, else the native `#tuple(b0 b1 …)`.
            let pat = match ctor_head {
                Some(head) => {
                    let mut children = vec![head];
                    children.extend(binders);
                    b.list(children)
                }
                None => b.compound(crate::ast::CompoundCtor::Tuple, &binders),
            };
            let body_node = emit_expr(db, b, *body, expected.clone(), env, emitted)?;
            let arm = b.list(vec![pat, body_node]);
            return Ok(b.list(vec![match_head, scrut_node, arm]));
        }
    }
    let arms = match root {
        SumCont::Switch { path, arms } if path.is_empty() => arms,
        // A COMPOUND scrutinee dispatching on a SLOT (not its own discriminant): the root `Switch` sits at a
        // NON-EMPTY path (e.g. `[Elem(1)]` — the 2nd tuple slot, a record field, a newtype payload). The
        // scrutinee's own type is a Tuple/Record/newtype, NOT a `Sum`, so there is no root discriminant to
        // switch on (and the `Ty::Sum` `decl` recovery below would decline it); instead reconstruct the surface
        // arms with the SAME general tree-walk the deep-cont arm uses ([`emit_switch_tree`] + [`build_arm_pat`]):
        // destructure the irrefutable compound structure down to the switched slot and dispatch THERE —
        // `(#tuple(a (Some b)) …)` / `(#record((= f (V …))) …)` (the variant-at-slot shape, e.g. #6967's
        // variant-below-record, and inner-sum-at-a-tuple-slot). A root `LitTest` (a LITERAL at a compound slot —
        // `(#tuple(a 7) …)`) routes the same way (the tree-walk emits the literal in the pattern), as does a root
        // `Guarded` (a GUARD on the destructured slots — `((guard #tuple(a k) <cond>) …)`; the tree-walk builds
        // the compound pattern, binds the slots, and wraps the arm in `(guard … <cond>)`). The per-`decl`
        // un-emitted-user-sum guard lives inside `build_arm_pat`/`ctor_pat`, and every tree leaf is enumerated so
        // the emitted match stays exhaustive (no synthesized wildcard). A non-scalar `LitTest` probe still
        // declines (its surface literal is a later slice).
        SumCont::Switch { .. } | SumCont::LitTest { .. } | SumCont::Guarded { .. } => {
            let root_ty = crate::infer::type_of(db, scrutinee);
            let match_head = b.name("match");
            let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let mut children = vec![match_head, scrut_node];
            emit_switch_tree(
                db,
                b,
                scrutinee,
                &root_ty,
                root,
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                &expected,
                env,
                emitted,
                &mut children,
            )?;
            return Ok(b.list(children));
        }
        _ => {
            return Err(Reject::unsupported(
                "the Cadenza backend does not support lowering this sum match (a disc-folded / nested / \
                 guarded root — only a switch on the scrutinee's own discriminant, or a switch on a \
                 compound slot)"
                    .to_string(),
            ));
        }
    };
    // The scrutinee's solved sum declaration — the source of each arm's variant name + payload arity.
    let decl = match crate::infer::type_of(db, scrutinee) {
        Ty::Sum { decl, .. } => decl,
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend cannot recover a sum declaration for a MatchSum scrutinee"
                    .to_string(),
            ));
        }
    };
    if db.is_user_node(decl) && !emitted.contains(&decl) {
        return Err(Reject::unsupported(
            "the Cadenza backend does not support re-emitting a match over this user sum (its `(type …)` \
             declaration is not emitted — a generic / open / single-variant sum)"
                .to_string(),
        ));
    }
    let match_head = b.name("match");
    let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
    let mut children = vec![match_head, scrut_node];
    for arm in arms {
        match arm.disc {
            // The DEFAULT (wildcard) tail `_` — binds nothing; a body reading the whole scrutinee does so
            // through its own name. Only a bare `Leaf` default is emitted (a guarded default declines).
            None => {
                let SumCont::Leaf(body) = &arm.cont else {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support lowering a guarded / literal-test default \
                         sum-match arm"
                            .to_string(),
                    ));
                };
                let pat = b.name("_");
                let body_node = emit_expr(db, b, *body, expected.clone(), env, emitted)?;
                children.push(b.list(vec![pat, body_node]));
            }
            // An EXPLICIT variant. Register its payload binders ONCE (single-payload reads `[Payload]`, a
            // multi-payload variant's tuple slot `i` reads `[Payload, Elem(i)]`), then emit one surface arm
            // per step of the cont chain: a `Leaf` is `(<Variant> <binder>…) <body>`; a `Guarded { cond,
            // body, els }` is `((guard <Variant-pattern> <cond>) <body>)` FOLLOWED BY the arms of `els` (the
            // fall-through rows of the SAME variant — a later guard, or the covering unguarded arm), exactly
            // as the Core threads a false guard's else. The shared binders are in scope for every cond+body;
            // the covering unguarded fall-through keeps the emitted match exhaustive. A literal-test /
            // nested-switch continuation declines (a later slice).
            Some(disc) => {
                let arity = db
                    .type_decl_by_occ(decl)
                    .and_then(|t| t.variants.get(disc as usize))
                    .map(|v| v.payloads.len())
                    .ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend could not recover the variant arity for a sum-match arm"
                                .to_string(),
                        )
                    })?;
                let mut binder_names = Vec::with_capacity(arity);
                // The variant's payload type(s) — the source of each slot binder's TYPE (for the erased-newtype
                // read peel; see `BinderEnv::payload_tys`). `sum_payload_expected` returns a `Tuple` of the
                // slots (multi-payload) or the sole type (arity 1); index slot `i` accordingly.
                let sum_ty = crate::infer::type_of(db, scrutinee);
                let payload_expected = sum_payload_expected(db, decl, disc, &sum_ty);
                for slot in 0..arity {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    let path: Vec<PathStep> = if arity == 1 {
                        vec![PathStep::Payload]
                    } else {
                        vec![PathStep::Payload, PathStep::Elem(slot)]
                    };
                    if let Some(slot_ty) = match (&payload_expected, arity) {
                        (Some(Ty::Tuple(ts)), n) if n > 1 => ts.get(slot).cloned(),
                        (Some(t), 1) => Some(t.clone()),
                        _ => None,
                    } {
                        env.payload_tys.insert((scrutinee, path.clone()), slot_ty);
                    }
                    env.payloads.insert((scrutinee, path), name.clone());
                    binder_names.push(name);
                }
                let mut cont = &arm.cont;
                loop {
                    // Rebuild the `(<Variant> <binder>…)` pattern fresh for THIS surface arm.
                    let head =
                        crate::lower::variant_head_ast(db, b, decl, disc).ok_or_else(|| {
                            Reject::decline(
                                "the Cadenza backend could not recover the variant name for a \
                                 sum-match arm"
                                    .to_string(),
                            )
                        })?;
                    let mut pat_children = vec![head];
                    for n in &binder_names {
                        pat_children.push(b.name(n.clone()));
                    }
                    let var_pat = b.list(pat_children);
                    match cont {
                        SumCont::Leaf(body) => {
                            let body_node =
                                emit_expr(db, b, *body, expected.clone(), env, emitted)?;
                            children.push(b.list(vec![var_pat, body_node]));
                            break;
                        }
                        SumCont::Guarded { cond, body, els } => {
                            let guard_head = b.name("guard");
                            let cond_node = emit_expr(db, b, *cond, None, env, emitted)?;
                            let guard_pat = b.list(vec![guard_head, var_pat, cond_node]);
                            let body_node =
                                emit_expr(db, b, *body, expected.clone(), env, emitted)?;
                            children.push(b.list(vec![guard_pat, body_node]));
                            cont = els.as_ref();
                        }
                        // A LITERAL-PAYLOAD test `(<Variant> <lit>)` on the WHOLE single payload — translate
                        // to the equivalent GUARD `(guard (<Variant> <binder>) (= <binder> <lit>))`: the
                        // literal test IS an equality refinement of the bound payload, so reusing the guard
                        // machinery makes a `then_` that reads the payload work (it reads `<binder>`, which
                        // == the literal on a match). Only a scalar (`Int`/`Bool`) probe over the single
                        // payload `[Payload]` is realized (a runtime `Str`/`Char`/`Bytes`/`ListLen`/
                        // `MapHasKeys` probe never reaches a backend; a deeper `path` — a literal in a
                        // multi-payload tuple slot — is a later slice), else decline.
                        SumCont::LitTest {
                            path,
                            probe,
                            then_,
                            els,
                        } => {
                            use crate::core::PathStep;
                            if !matches!(
                                probe,
                                crate::core::Probe::Int(_) | crate::core::Probe::Bool(_)
                            ) {
                                return Err(Reject::decline(
                                    "the Cadenza backend lowers a literal-payload test only for a \
                                     scalar (Int/Bool) probe"
                                        .to_string(),
                                ));
                            }
                            // Which payload binder the literal refines: the WHOLE single payload
                            // (`[Payload]`), or slot `i` of a multi-payload variant (`[Payload, Elem(i)]`).
                            // A deeper path (a literal nested inside a payload tuple/record) is a later slice.
                            let bidx = match &path[..] {
                                [PathStep::Payload] if binder_names.len() == 1 => 0usize,
                                [PathStep::Payload, PathStep::Elem(i)]
                                    if *i < binder_names.len() =>
                                {
                                    *i
                                }
                                _ => {
                                    return Err(Reject::unsupported(
                                        "the Cadenza backend does not support lowering a literal-payload test at \
                                         a deep / non-payload path"
                                            .to_string(),
                                    ));
                                }
                            };
                            // The matched continuation `then_` must be a plain body (`Leaf`); a nested
                            // continuation after the literal test (another guard / test) is a later slice.
                            let SumCont::Leaf(then_body) = then_.as_ref() else {
                                return Err(Reject::unsupported(
                                    "the Cadenza backend does not support lowering a literal-payload test with \
                                     a nested `then` continuation"
                                        .to_string(),
                                ));
                            };
                            // Translate to `(guard (<V> b…) (= <b_bidx> <lit>))`, built in the SAME
                            // leaf-insertion order the `Guarded` arm uses for a re-read guard — guard head,
                            // then the cond `=`/binder/lit — so hop¹ (this arm) and hop² (recompile re-reads
                            // the guard → the `Guarded` arm) emit BYTE-IDENTICAL trees (the no-canon codec
                            // serializes build order).
                            let guard_head = b.name("guard");
                            let eq_head = b.name("=");
                            let binder_ref = b.name(binder_names[bidx].clone());
                            let lit = match probe {
                                crate::core::Probe::Int(v) => b.atom_leaf(Leaf::Int {
                                    value: v.clone(),
                                    radix: Radix::Dec,
                                }),
                                crate::core::Probe::Bool(x) => b.atom_leaf(Leaf::Bool(*x)),
                                _ => unreachable!("probe kind checked above"),
                            };
                            let cond_node = b.list(vec![eq_head, binder_ref, lit]);
                            let guard_pat = b.list(vec![guard_head, var_pat, cond_node]);
                            let body_node =
                                emit_expr(db, b, *then_body, expected.clone(), env, emitted)?;
                            children.push(b.list(vec![guard_pat, body_node]));
                            cont = els.as_ref();
                        }
                        // A NESTED variant match on the immediate payload — `(Some (Some x))` / `(Some
                        // (None))` and DEEPER (`(Wrap (Some (Some x)))`, arbitrary depth). The outer arm's
                        // cont is a nested `Switch` on the payload's discriminant (path `[Payload]`), sharing
                        // the outer probe (Maranget). Scope: a LINEAR chain of SINGLE-payload variants
                        // (`arity == 1` at every intermediate level) ending in a `Leaf` body whose variant may
                        // be multi-payload. [`emit_nested_switch_chain`] recurses the chain to arbitrary depth,
                        // reconstructing one flattened surface arm `(<V0> (<V1> … (<Vk> b…)))` per leaf and
                        // registering the leaf binder under the FULL path from the ROOT scrutinee
                        // (`[Payload, Payload, …]`, `… Elem(i)` for a multi-payload leaf) — the exact key the
                        // body's `Core::SumPayload` reads (keying by the full step vector, NEVER a truncated
                        // suffix, is the correctness crux: a length-only key would collide distinct same-length
                        // prefixes). A MULTI-payload INTERMEDIATE variant (a nested switch on a tuple slot, e.g.
                        // `(Pair (Some a) (Some b))`), a deeper/non-payload nested path, a guarded/literal inner
                        // cont, or a DEFAULT inner arm still declines (later slices). Preserves the depth-1
                        // shape byte-for-byte (the single-level case is this recursion of depth 1).
                        SumCont::Switch {
                            path: npath,
                            arms: inner_arms,
                        } if arity == 1 && matches!(npath.as_ref(), [PathStep::Payload]) => {
                            let sum_ty = crate::infer::type_of(db, scrutinee);
                            let payload_ty =
                                sum_payload_expected(db, decl, disc, &sum_ty).ok_or_else(|| {
                                    Reject::decline(
                                        "the Cadenza backend could not recover the nested-match payload \
                                         type"
                                            .to_string(),
                                    )
                                })?;
                            let wrap = vec![(decl, disc)];
                            emit_nested_switch_chain(
                                db,
                                b,
                                scrutinee,
                                &payload_ty,
                                npath.as_ref(),
                                inner_arms,
                                &wrap,
                                &expected,
                                env,
                                emitted,
                                &mut children,
                            )?;
                            break;
                        }
                        // A DEEP decision-tree cont (a nested Switch at an arbitrary path — through a
                        // newtype/tuple, or multi-payload — beyond the linear-chain [`emit_nested_switch_chain`]
                        // handles). Reconstruct the surface arms by walking the tree ([`emit_switch_tree`] +
                        // [`build_arm_pat`], v-wasm-opt review #4): the outer variant `disc` is the choice at the
                        // root path `[]`, and each nested `Switch` arm adds its `path -> disc` choice; each leaf
                        // becomes one deep surface arm.
                        SumCont::Switch { .. } => {
                            let root_ty = crate::infer::type_of(db, scrutinee);
                            let mut choices: std::collections::HashMap<
                                Vec<crate::core::PathStep>,
                                Option<u32>,
                            > = std::collections::HashMap::new();
                            choices.insert(Vec::new(), Some(disc));
                            emit_switch_tree(
                                db,
                                b,
                                scrutinee,
                                &root_ty,
                                cont,
                                choices,
                                std::collections::HashMap::new(),
                                &expected,
                                env,
                                emitted,
                                &mut children,
                            )?;
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(b.list(children))
}

/// Recurse a LINEAR chain of nested single-payload variant switches, emitting one FLATTENED surface arm
/// `(<V0> (<V1> … (<Vk> b…)))` per leaf into `children`. Called from [`emit_match_sum`] when an outer arm's
/// cont is a nested `Switch` on the immediate payload; recurses to arbitrary depth.
///
/// - `root_scrutinee` — the ROOT match scrutinee, the key under which every payload binder is registered.
/// - `switch_ty` — the SOLVED `Ty::Sum` of the value THIS switch dispatches on (the value at `path`).
/// - `path` — the FULL path from the root to this switch's scrutinee value (`[Payload]`, `[Payload, Payload]`,
///   …). Payload binders are registered under `(root_scrutinee, path ++ [Payload {, Elem(slot)}])` — the exact
///   key the body's `Core::SumPayload` carries. Keying by the FULL step vector (never a length/suffix) is the
///   correctness crux: a truncated key would collide two distinct same-length prefixes → a silent wrong-payload
///   read (the wasm `select.rs` lesson).
/// - `wrap` — the outer variant `(decl, disc)` wrappers, OUTERMOST-first, rebuilt (fresh head per surface arm)
///   around the built inner pattern so `(<Vk> b…)` becomes `(<V0> (<V1> … (<Vk> b…)))`.
///
/// A LEAF arm emits (its variant may be multi-payload — each slot binds at `… Elem(slot)`). A `Switch` arm on a
/// SINGLE-payload inner variant (`arity == 1`) at the IMMEDIATE payload path recurses one level deeper. Anything
/// else — a multi-payload intermediate (a switch on a tuple slot), a non-immediate-payload nested path, a
/// guarded/literal inner cont, a DEFAULT (wildcard) inner arm, or an un-emitted user sum at ANY level — declines
/// (later slices), so the emit never produces a pattern the recompile cannot re-lower identically.
#[allow(clippy::too_many_arguments)]
fn emit_nested_switch_chain(
    db: &mut Db,
    b: &mut Builder,
    root_scrutinee: StructId,
    switch_ty: &Ty,
    path: &[crate::core::PathStep],
    inner_arms: &[crate::core::SumArm],
    wrap: &[(StructId, u32)],
    expected: &Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
    children: &mut Vec<StructId>,
) -> Result<(), Reject> {
    use crate::core::{PathStep, SumCont};
    let decl = match switch_ty {
        Ty::Sum { decl, .. } => *decl,
        _ => {
            return Err(Reject::decline(
                "the Cadenza backend nested-match payload is not a sum".to_string(),
            ));
        }
    };
    // (per-level) A user sum whose `(type …)` was not re-emitted must decline HERE, not only at the root —
    // its variant heads must resolve on recompile. Option/Result are ambient (prelude), so they proceed.
    if db.is_user_node(decl) && !emitted.contains(&decl) {
        return Err(Reject::unsupported(
            "the Cadenza backend does not support re-emitting a nested match over an un-emitted user sum"
                .to_string(),
        ));
    }
    for inner in inner_arms {
        let Some(inner_disc) = inner.disc else {
            // DEFAULT (wildcard) inner arm — matches ANY inner variant not covered by an explicit arm above.
            // Bind the SWITCHED value (the whole inner sum at `path`) to a fresh binder and wrap it with the
            // outer heads → `(<V0> ivar)`; the body reads the switched value whole via `SumPayload{root, path}`
            // (it cannot read a specific variant's slot — the variant is unknown here). Emitted AFTER the
            // explicit variant arms (inner-arm order preserved), so the surface matcher takes an explicit arm
            // first and this catches the rest — exhaustiveness holds. A GUARDED/nested default cont declines.
            let SumCont::Leaf(default_body) = &inner.cont else {
                return Err(Reject::unsupported(
                    "the Cadenza backend does not support lowering a guarded / nested nested-switch default arm"
                        .to_string(),
                ));
            };
            let ivar = synth_payload_name(env.next_payload);
            env.next_payload += 1;
            env.payloads
                .insert((root_scrutinee, path.to_vec()), ivar.clone());
            let mut pat = b.name(ivar.clone());
            for (wd, wdisc) in wrap.iter().rev() {
                let wh = crate::lower::variant_head_ast(db, b, *wd, *wdisc).ok_or_else(|| {
                    Reject::decline(
                        "the Cadenza backend could not recover an outer variant name".to_string(),
                    )
                })?;
                pat = b.list(vec![wh, pat]);
            }
            let body_node = emit_expr(db, b, *default_body, expected.clone(), env, emitted)?;
            children.push(b.list(vec![pat, body_node]));
            continue;
        };
        let inner_arity = db
            .type_decl_by_occ(decl)
            .and_then(|t| t.variants.get(inner_disc as usize))
            .map(|v| v.payloads.len())
            .ok_or_else(|| {
                Reject::decline(
                    "the Cadenza backend could not recover the inner variant arity".to_string(),
                )
            })?;
        match &inner.cont {
            // LEAF — the deepest level. Bind each payload slot under the FULL path from the root, build
            // `(<Vk> b…)`, then wrap outward with the accumulated outer heads (innermost wrap first).
            SumCont::Leaf(inner_body) => {
                let mut binders = Vec::with_capacity(inner_arity);
                for slot in 0..inner_arity {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    let mut bpath: Vec<PathStep> = path.to_vec();
                    bpath.push(PathStep::Payload);
                    if inner_arity != 1 {
                        bpath.push(PathStep::Elem(slot));
                    }
                    env.payloads.insert((root_scrutinee, bpath), name.clone());
                    binders.push(name);
                }
                let leaf_head = crate::lower::variant_head_ast(db, b, decl, inner_disc)
                    .ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend could not recover the inner variant name"
                                .to_string(),
                        )
                    })?;
                let mut pat_children = vec![leaf_head];
                for n in &binders {
                    pat_children.push(b.name(n.clone()));
                }
                let mut pat = b.list(pat_children);
                for (wd, wdisc) in wrap.iter().rev() {
                    let wh =
                        crate::lower::variant_head_ast(db, b, *wd, *wdisc).ok_or_else(|| {
                            Reject::decline(
                                "the Cadenza backend could not recover an outer variant name"
                                    .to_string(),
                            )
                        })?;
                    pat = b.list(vec![wh, pat]);
                }
                let body_node = emit_expr(db, b, *inner_body, expected.clone(), env, emitted)?;
                children.push(b.list(vec![pat, body_node]));
            }
            // A DEEPER switch on this inner variant's SINGLE payload (`arity == 1`) at the immediate payload
            // path `path ++ [Payload]` — recurse, extending the wrap with this variant and the path by one.
            SumCont::Switch {
                path: np,
                arms: deeper,
            } if inner_arity == 1 => {
                let mut want: Vec<PathStep> = path.to_vec();
                want.push(PathStep::Payload);
                if np.as_ref() != want.as_slice() {
                    return Err(Reject::unsupported(
                        "the Cadenza backend does not support lowering a nested switch at a non-immediate-payload \
                         path"
                            .to_string(),
                    ));
                }
                let payload_ty =
                    sum_payload_expected(db, decl, inner_disc, switch_ty).ok_or_else(|| {
                        Reject::decline(
                            "the Cadenza backend could not recover the nested-match payload type"
                                .to_string(),
                        )
                    })?;
                let mut wrap2 = wrap.to_vec();
                wrap2.push((decl, inner_disc));
                emit_nested_switch_chain(
                    db,
                    b,
                    root_scrutinee,
                    &payload_ty,
                    &want,
                    deeper,
                    &wrap2,
                    expected,
                    env,
                    emitted,
                    children,
                )?;
            }
            _ => {
                return Err(Reject::unsupported(
                    "the Cadenza backend does not support lowering a guarded/literal/multi-payload/deeper \
                     nested-switch inner arm"
                        .to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// Reconstruct the surface `(match <scrutinee> (<list-pattern> <body>)…)` for a `Core::MatchList` — a match
/// dispatched by the list's LENGTH. Each arm's [`ListArmCond`] maps to a surface list pattern: `LenEq(n)` →
/// `(list b0 … b_{n-1})` (a fixed-arity pattern binding exactly `n` elements), `LenGe(lead)` →
/// `(list b0 … b_{lead-1} .. rest)` (a rest pattern binding `lead` leading elements + the tail sublist), and
/// `Any` → the bare wildcard `_` (a whole-list catch-all; a body that reads the whole list does so through
/// the scrutinee's OWN name, which A-normal form guarantees is a binder). A leading element binder is
/// registered under `[Elem(i)]` and the rest binder under `[RestFrom(lead)]` (the same `SumPayload` key the
/// body carries — see `resolve.rs`), so a `Core::SumPayload` read resolves to its binder. Only PLAIN binders
/// are emitted: a NESTED element sub-pattern (`(list (Mk x) …)` / `(list (list a ..) ..)`) resolves its
/// binder at a DEEPER path (`[Elem(i), Payload]` / `[Elem(i), Elem(j)]`) that this slice does not register,
/// so its body read misses the env and DECLINES rather than emit a wrong pattern. A GUARDED arm declines.
/// Arm order + conditions mirror the Core exactly, so the emitted match stays exhaustive (no CDZ0210).
fn emit_match_list(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    use crate::core::{ListArmCond, PathStep};
    // RECOMPILABILITY FENCE (breaker mfp1/mfp2, #5472 class): a list-match whose scrutinee is a `ListNew`
    // carrying a RUNTIME-VALUED (non-constant) `MapNew` element does NOT round-trip. The optimizer inlines
    // the list literal here, but a MAP-KEY sub-pattern into a runtime-valued map element (`(list #map((= 5
    // v)) _r)`) is desugared to a runtime presence-chain (the #5472 fence). Re-emitting that de-sugared arm
    // and RECOMPILING it re-fuses into a nested map-key match whose map sits at a nested `[Elem(i), …]` path
    // inside the (then runtime-treated) list — a shape the lowering itself declines (`lower_map_field`:
    // "a nested map pattern over a runtime/non-constant scrutinee is not yet matched"). That yields an
    // UN-compilable `program1` (a corpus-cadenza RED, not a skip). Until the runtime nested-map matcher is
    // wired (option (a): re-emit the map match against the BOUND element so it round-trips), DECLINE the
    // shape at emit time so the case skips rather than emitting a program the compiler cannot re-lower. A
    // CONSTANT map element folds fine (`fold_sum_path`), so only a NON-const map element trips this.
    if let Core::ListNew { elems } = core_of(db, scrutinee)
        && elems.iter().any(|&e| {
            matches!(core_of(db, e), Core::MapNew { .. }) && !crate::lower::is_const_value(db, e)
        })
    {
        return Err(Reject::unsupported(
            "the Cadenza backend does not support re-emitting a list match whose scrutinee carries a \
             runtime-valued map element (a nested map-key sub-pattern over a runtime map does not \
             round-trip — the #5472 fence)"
                .to_string(),
        ));
    }
    let match_head = b.name("match");
    let scrut_node = emit_expr(db, b, scrutinee, None, env, emitted)?;
    let mut children = vec![match_head, scrut_node];
    // The list's ELEMENT type — recorded for each element binder (for the erased-newtype read peel, see
    // `BinderEnv::payload_tys`): a `#list((Mk a) …)` element that is an erased single-variant newtype otherwise
    // binds the whole `Box` and a body read of its inner scalar emits the bare `Box` where the inner is required
    // (CDZ0201 "arithmetic is not defined on Box" on recompile — a round-trip BREAK, not just a decline).
    let elem_ty = match crate::infer::type_of(db, scrutinee) {
        Ty::List(e) => Some(*e),
        _ => None,
    };
    for arm in arms {
        // Build the arm's surface pattern, registering each binder's `SumPayload` path for the body.
        let pattern = match arm.cond {
            ListArmCond::LenEq(n) => {
                let list_head = b.name("list");
                let mut pat = vec![list_head];
                for i in 0..n {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    if let Some(et) = &elem_ty {
                        env.payload_tys
                            .insert((scrutinee, vec![PathStep::Elem(i)]), et.clone());
                    }
                    env.payloads
                        .insert((scrutinee, vec![PathStep::Elem(i)]), name.clone());
                    pat.push(b.name(name));
                }
                b.list(pat)
            }
            ListArmCond::LenGe(lead) => {
                let list_head = b.name("list");
                let mut pat = vec![list_head];
                for i in 0..lead {
                    let name = synth_payload_name(env.next_payload);
                    env.next_payload += 1;
                    if let Some(et) = &elem_ty {
                        env.payload_tys
                            .insert((scrutinee, vec![PathStep::Elem(i)]), et.clone());
                    }
                    env.payloads
                        .insert((scrutinee, vec![PathStep::Elem(i)]), name.clone());
                    pat.push(b.name(name));
                }
                // The `..` separator, then the rest binder (the tail sublist from `lead` onward).
                pat.push(b.name(".."));
                let rest = synth_payload_name(env.next_payload);
                env.next_payload += 1;
                env.payloads
                    .insert((scrutinee, vec![PathStep::RestFrom(lead)]), rest.clone());
                pat.push(b.name(rest));
                b.list(pat)
            }
            // A whole-list catch-all — the bare wildcard `_`. The whole-list value, if the body reads it,
            // comes through the scrutinee's own name (not a `SumPayload`), so no binder is registered.
            ListArmCond::Any => b.name("_"),
        };
        // A GUARDED arm wraps its pattern in the `(guard <pattern> <cond>)` surface form (`resolve.rs`
        // Case 6lg): the arm fires only when its length condition AND `cond` hold, and otherwise FALLS
        // THROUGH to the next arm — the surface reader re-lowers `(guard …)` with that same fall-through.
        // The cond is emitted with this arm's element/rest binders IN SCOPE (registered above), so a guard
        // reading a bound element resolves to its binder. A guarded arm does not count toward exhaustiveness
        // (upstream guarantees an unguarded covering tail), so mirroring the arm keeps the match exhaustive.
        let pattern = match arm.guard {
            Some(g) => {
                let guard_head = b.name("guard");
                let cond = emit_expr(db, b, g, None, env, emitted)?;
                b.list(vec![guard_head, pattern, cond])
            }
            None => pattern,
        };
        let body_node = emit_expr(db, b, arm.body, expected.clone(), env, emitted)?;
        children.push(b.list(vec![pattern, body_node]));
    }
    Ok(b.list(children))
}

/// Re-emit a scalar match's arms as a nested `if`-chain, from arm `i` onward. The LAST arm (or an
/// UNGUARDED wildcard) is unconditional — its body IS the else: a scalar `Core::Match` is exhaustive
/// (checked upstream), so its final/wildcard arm covers the residual case. Each earlier arm wraps
/// `(if <cond> <body> <rest>)`, where `<cond>` is the probe test `(= <scrutinee> <lit>)`, the arm's GUARD,
/// or their conjunction `(and (= <scrutinee> <lit>) <guard>)`: a GUARDED arm fires only when its probe AND
/// guard hold and otherwise FALLS THROUGH to the rest — exactly the `if`/`else` chain, so a guard needs no
/// surface `match`, it desugars into the condition (a guarded Wild arm is just `(if <guard> body rest)`).
/// The guard's binder is the scrutinee (a bare-binder pattern binds the whole scalar), which lowering
/// resolves to the scrutinee's own core, so emitting the guard re-emits the scrutinee reference in scope.
/// The scrutinee is a pure scalar, re-emitted per probe. Precondition (caller): every probe is
/// `Int`/`Bool`/`Wild`. (A guarded arm does not count toward exhaustiveness, so the final covering arm is
/// always unguarded — a guarded LAST arm would be a non-exhaustive shape and declines defensively.)
// The recursion threads the shared emit state (db/builder/scrutinee/arms/index) plus the `expected` type
// and the binder env — each is load-bearing, so the arg count is intrinsic, not a bundling opportunity.
#[allow(clippy::too_many_arguments)]
fn emit_match_chain(
    db: &mut Db,
    b: &mut Builder,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    i: usize,
    expected: Option<Ty>,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<StructId, Reject> {
    let arm = &arms[i];
    let is_last = i + 1 == arms.len();
    // An UNGUARDED last arm, or an UNGUARDED wildcard (always matches → any later arm is dead): the
    // unconditional else. A wildcard arm may BIND the scrutinee; its body reads that binder, which lowering
    // resolves to the scrutinee's own core, so emitting the body re-emits the scrutinee reference in scope.
    // The body is the match's value/tail position, so it inherits the match's `expected` type.
    let unguarded_wild = matches!(arm.probe, crate::core::Probe::Wild) && arm.guard.is_none();
    if (is_last && arm.guard.is_none()) || unguarded_wild {
        return emit_expr(db, b, arm.body, expected, env, emitted);
    }
    if is_last {
        // A guarded final arm cannot cover the residual case (its guard may fail) — a non-exhaustive shape
        // that should not arise from a well-formed match; decline rather than build a chain with no tail.
        return Err(Reject::decline(
            "the Cadenza backend does not lower a GUARDED final scalar-match arm (no covering tail)"
                .to_string(),
        ));
    }
    // The probe test `(= <scrutinee> <lit>)`, if this arm probes a literal (a Wild arm has no probe test —
    // only its guard gates it).
    let probe_cond = match &arm.probe {
        crate::core::Probe::Int(v) => {
            let eq = b.name("=");
            let scrut = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let lit = b.atom_leaf(Leaf::Int {
                value: v.clone(),
                radix: Radix::Dec,
            });
            Some(b.list(vec![eq, scrut, lit]))
        }
        crate::core::Probe::Bool(x) => {
            let eq = b.name("=");
            let scrut = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let lit = b.atom_leaf(Leaf::Bool(*x));
            Some(b.list(vec![eq, scrut, lit]))
        }
        crate::core::Probe::Char(c) => {
            // A char is a Unicode scalar value with a runtime `=`; `#\c` re-emits the literal (the same
            // leaf `Core::ConstChar` emits), so the probe recompiles to the identical char-equality test.
            let eq = b.name("=");
            let scrut = emit_expr(db, b, scrutinee, None, env, emitted)?;
            let lit = b.atom_leaf(Leaf::Char(*c));
            Some(b.list(vec![eq, scrut, lit]))
        }
        crate::core::Probe::Wild => None,
        // The caller pre-scanned the arms to only Int/Bool/Char/Wild.
        _ => {
            return Err(Reject::unsupported(
                "the Cadenza backend does not support lowering this match probe".to_string(),
            ));
        }
    };
    // The arm's guard (a boolean the scrutinee-binder is in scope for), if present.
    let guard_cond = match arm.guard {
        Some(g) => Some(emit_expr(db, b, g, None, env, emitted)?),
        None => None,
    };
    // The full condition: probe alone, guard alone, or `(and probe guard)`. At least one is present here
    // (an unguarded Wild was returned above as the unconditional else).
    let cond = match (probe_cond, guard_cond) {
        (Some(p), Some(g)) => {
            let and = b.name("and");
            b.list(vec![and, p, g])
        }
        (Some(p), None) => p,
        (None, Some(g)) => g,
        (None, None) => {
            unreachable!("an unguarded Wild arm is the unconditional else, handled above")
        }
    };
    let if_head = b.name("if");
    let body = emit_expr(db, b, arm.body, expected.clone(), env, emitted)?;
    let rest = emit_match_chain(db, b, scrutinee, arms, i + 1, expected, env, emitted)?;
    Ok(b.list(vec![if_head, cond, body, rest]))
}

/// The `expected` type to pass down to the value/tail children (branches, arm bodies) of a container node
/// `id` (an `if` / `match`): PREFER the container's OWN solved type (the join of its branches — usually
/// concrete, e.g. an `if` returning `Option<Int64>`), falling back to the `incoming` expected only when the
/// container's own type is itself under-determined (e.g. a match whose arms are all `(None)`, whose join is
/// `Option<?>` — then the concrete type comes from further out). This is what lets a bare `(None)` in a
/// branch/arm body recover its element type. Cheap `Ty` clone; called once per container.
fn body_ctx(db: &mut Db, id: StructId, incoming: Option<Ty>) -> Option<Ty> {
    let own = crate::infer::type_of(db, id);
    if ty_has_free_arg(&own) {
        incoming
    } else {
        Some(own)
    }
}

/// How a `Ty::Nominal` (erased single-variant sum) value at a node should be re-emitted — see the guard in
/// [`emit_expr_viewed`].
enum NominalDisp {
    /// This node is a CONSTRUCTION site: emit `(<Ctor> <payload>)`, the payload peeled to `inner`.
    Construct,
    /// This node PASSES THROUGH the nominal from its sub-values: emit its core unwrapped (the constructor
    /// re-inserts at the true leaves the children emit).
    PassThrough,
    /// An ambiguous site this slice cannot classify — decline.
    Decline,
}

/// Classify how to re-emit a `Ty::Nominal` value at node `id` of the newtype `decl` (see [`NominalDisp`]).
/// The rule follows where the inner→nominal transition can be: a value-producing LEAF/OPERATOR (a literal,
/// arithmetic, boolean) intrinsically yields the INNER value, so it is the construction site (`Construct`);
/// a binder (`Param`/`LocalRef`) is a construction only when its DECLARED type is the inner (a wrapped
/// parameter) and a pass-through when it already holds the nominal; control flow (`If`/`Let`/`Match*`)
/// passes the nominal through from its branches/body (`PassThrough` — the children construct at the leaves);
/// any other core (`Call`, compound builders, …) is ambiguous (`Decline`).
fn nominal_disposition(db: &mut Db, id: StructId, decl: StructId) -> NominalDisp {
    match core_of(db, id) {
        // Intrinsically inner-valued producers → the constructor is erased HERE. A scalar constant/operator
        // yields its scalar; a COMPOUND-VALUE builder (`(list …)`/`(tuple …)`/`(record …)`/`(map …)`/
        // `Set.of`) yields its own collection/tuple/record — NEVER a pre-existing nominal — so a newtype
        // over a compound (`(Mk (list n …))` : `(type LW (Mk (List Int64)))`) is a construction here: wrap
        // `(Mk <compound>)`, the payload peeled to `inner` (a List/Tuple/…) via the `view` recursion.
        Core::ConstInt(_)
        | Core::ConstRational(..)
        | Core::ConstBool(_)
        | Core::ConstChar(_)
        | Core::ConstStr(_)
        | Core::ConstFloat(_)
        | Core::Unit
        | Core::Arith { .. }
        | Core::Compare { .. }
        | Core::StrCmp { .. }
        | Core::FloatCompare { .. }
        | Core::Not { .. }
        | Core::And { .. }
        | Core::Tuple { .. }
        | Core::Record { .. }
        | Core::ListNew { .. }
        | Core::MapNew { .. }
        | Core::SetOf { .. } => NominalDisp::Construct,
        // A binder: construction iff its DECLARED type is NOT already this nominal (a wrapped inner value);
        // a binder already typed as the nominal is a pass-through (emit the bare name).
        Core::Param { binder } | Core::LocalRef { binder } => {
            match crate::infer::type_of(db, binder) {
                Ty::Nominal { decl: bd, .. } if bd == decl => NominalDisp::PassThrough,
                _ => NominalDisp::Construct,
            }
        }
        // A payload READ — a stored sum/list slot read back. `nominal_disposition` is reached ONLY when this
        // node's OWN solved type is the nominal (the caller matched `eff_ty` = `Ty::Nominal`), so the slot
        // stored the nominal and the read yields it directly: PASS-THROUGH (emit the read; it carries the
        // nominal, no re-wrap). NOT the ambiguous `Call` case — a `SumPayload`'s value is fixed by the slot
        // type, not inferred from a return. (compiler-ml: the run-src interpreter reads Ty/Subst/… nominal
        // fields out of its node/state records — every one of these was the `_ => Decline` newtype gap.)
        Core::SumPayload { .. } => NominalDisp::PassThrough,
        // Control flow / binding carry the nominal through from their sub-expressions.
        Core::If { .. }
        | Core::Let { .. }
        | Core::Match { .. }
        | Core::MatchSum { .. }
        | Core::MatchList { .. } => NominalDisp::PassThrough,
        // Anything else (a `Call` that may return inner OR nominal, a compound builder, …) is ambiguous.
        _ => NominalDisp::Decline,
    }
}

/// How a `Ty::Qty` (unit-bearing quantity) value at a node should be re-emitted — the QUANTITY twin of
/// [`nominal_disposition`]. A quantity erases its (compile-time) unit, so its value's Core IS the bare
/// magnitude; the `((. Qty of) <mag> <unit>)` surface must be re-inserted only where the value GENUINELY
/// escapes AS a quantity — never over a pass-through, and (the subtle part) never over an erased-op leaf
/// that only LOOKS like a quantity node.
///
/// TRAP: The trap [`Prim::QtyValue`] sets: `Qty.value` (and other unit-erasing ops) lower to NOTHING — the
/// magnitude Core node is left in place, still carrying its `Ty::Qty` solved type, but the ENCLOSING
/// context (the `Qty.value` result, the def's inferred result type) is the bare inner numeric. So an
/// `Arith`/`Compare`/`Convert`/const leaf reached here with `eff_ty == Ty::Qty` is almost always an
/// erased-peel magnitude whose real escape type is numeric — wrapping it re-inserts a `Qty.of` the direct
/// path never renders (a same-unit runtime sum `(Qty.value (+ (Qty.of n m) (Qty.of 5 m)))` returns bare
/// `Int64 8`, NOT `(Qty.of 8 m)`), and a constant magnitude additionally drops the reference-unit display
/// scale that [`const_value_ast`] owns. Both were confirmed corpus regressions. So this classifier is
/// CONSERVATIVE: it Constructs ONLY at a bare magnitude BINDER (`Core::Param`/`LocalRef` whose declared
/// type is the inner numeric) — the one shape that genuinely re-emits `(Qty.of <name> <unit>)` and round-
/// trips (a `def` returning `(Qty.of v u)` over a param `v`). A binder already typed `Ty::Qty` is a wrapped
/// quantity → pass-through. Control flow carries the quantity through (its true leaves handle the wrap).
/// EVERYTHING ELSE declines — an erased-op / constant / call magnitude cannot be soundly re-wrapped here
/// (decline-don't-miscompile); those are a later slice (needing the escape type threaded, not the erased
/// node's own `Ty::Qty`).
fn qty_disposition(db: &mut Db, id: StructId) -> NominalDisp {
    match core_of(db, id) {
        // A binder already typed `Ty::Qty` is a wrapped quantity (pass-through, emit the bare name); a
        // binder holding the inner magnitude (declared numeric) is the ONE sound construction site — a
        // `def` returning `(Qty.of <param> <unit>)` re-emits exactly this.
        Core::Param { binder } | Core::LocalRef { binder } => {
            match crate::infer::type_of(db, binder) {
                Ty::Qty { .. } => NominalDisp::PassThrough,
                _ => NominalDisp::Construct,
            }
        }
        // Control flow / binding carry the quantity through from their sub-expressions.
        Core::If { .. }
        | Core::Let { .. }
        | Core::Match { .. }
        | Core::MatchSum { .. }
        | Core::MatchList { .. } => NominalDisp::PassThrough,
        // An erased-op / constant / call magnitude reached with `eff_ty == Ty::Qty` is (almost always) an
        // erased `Qty.value`-peel whose real escape type is numeric — do NOT re-wrap (the direct path emits
        // it bare); a genuine quantity-returning construction from such a site needs the escape type
        // threaded, a later slice. Decline-don't-miscompile.
        _ => NominalDisp::Decline,
    }
}

/// The `expected` type for a single-payload variant's payload at a CONCRETE sum instantiation: the variant's
/// synthesized constructor scheme (`∀…. payload → Sum …`) unified against the concrete sum type via
/// [`crate::infer::payload_ty_at_instantiation`], yielding the payload's instantiated type (`Option Int64`
/// for `Some` at `Option (Option Int64)`). `None` when there is no ctor, no payload, or the result is STILL
/// under-determined (a genuinely-ambiguous nesting) — the payload then emits with no expected. `sum_ty` must
/// be the SumNew's own `Ty::Sum` (already `expected`-recovered by the caller).
/// Re-emit "element `index` of the single-variant NOMINAL value whose emitted surface is `node`". The
/// surface `.` operator cannot cross a nominal (the erasure wall: `(. <nominal> i)` recompiles to CDZ0201
/// "tuple projection requires a tuple, found <Nominal>"), so NAME the ctor via an inline single-arm match,
/// then index the ERASED payload. TWO layouts, discriminated by the newtype ARITY vs the inner tuple's arity.
/// A MULTI-payload variant (`arity == inner_len`, e.g. `Subst(Map,Map,Map)`) has each payload AS a tuple
/// element → `(match node ((Ctor b0 … b_{n-1}) b_index))`, body the `index`-th binder. An ARITY-1
/// TUPLE-payload newtype (`arity == 1`, inner a `Tuple` of len > 1, e.g. `WrapT(Tuple(a,b))`) has the sole
/// payload AS the tuple, so `index` projects INTO it → `(match node ((Ctor t) (. t index)))`.
/// Returns the surface node + the element's type. IRREFUTABLE (one variant → exhaustive) and value-eq
/// (recompile re-erases the newtype). CALLER gates to an EMITTED single-variant nominal (the `(type …)` must
/// resolve the ctor on recompile). Used by the `Core::Proj` arm (a folded `(MkWT t)` match leaving `(. w i)`
/// on the nominal-declared binder).
#[allow(clippy::too_many_arguments)]
fn emit_nominal_elem_peel(
    db: &mut Db,
    b: &mut Builder,
    node: StructId,
    index: usize,
    decl: StructId,
    inner: &Ty,
    env: &mut BinderEnv,
    emitted: &std::collections::HashSet<StructId>,
) -> Result<(StructId, Ty), Reject> {
    let _ = emitted; // caller gates; kept for call-site symmetry.
    let arity = db
        .type_decl_by_occ(decl)
        .and_then(|t| t.variants.first())
        .map(|v| v.payloads.len())
        .unwrap_or(0);
    let inner_len = match inner {
        Ty::Tuple(ts) => Some(ts.len()),
        _ => None,
    };
    let slot_ty = match inner {
        Ty::Tuple(ts) => ts.get(index).cloned().unwrap_or(Ty::Any),
        _ => Ty::Any,
    };
    let head = crate::lower::variant_head_ast(db, b, decl, 0).ok_or_else(|| {
        Reject::decline(
            "the Cadenza backend could not recover the single-variant ctor name for a payload projection"
                .to_string(),
        )
    })?;
    let match_head = b.name("match");
    if inner_len == Some(arity) && index < arity {
        // Multi-payload: bind every slot, return slot `index`.
        let mut pat_children = vec![head];
        let mut bi = None;
        for slot in 0..arity {
            let nm = synth_payload_name(env.next_payload);
            env.next_payload += 1;
            if slot == index {
                bi = Some(nm.clone());
            }
            pat_children.push(b.name(nm));
        }
        let pat = b.list(pat_children);
        let body = b.name(bi.expect("index < arity is bound"));
        let arm = b.list(vec![pat, body]);
        Ok((b.list(vec![match_head, node, arm]), slot_ty))
    } else if arity == 1 && inner_len.is_some_and(|l| index < l) {
        // Arity-1 tuple-payload newtype: bind the sole payload `t`, project `(. t index)`.
        let t = synth_payload_name(env.next_payload);
        env.next_payload += 1;
        let t_pat = b.name(t.clone());
        let pat = b.list(vec![head, t_pat]);
        let dot = b.name(".");
        let idx = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(index as i64),
            radix: Radix::Dec,
        });
        let t_ref = b.name(t);
        let proj = b.list(vec![dot, t_ref, idx]);
        let arm = b.list(vec![pat, proj]);
        Ok((b.list(vec![match_head, node, arm]), slot_ty))
    } else {
        Err(Reject::decline(
            "the Cadenza backend reached a payload projection over a single-variant sum whose erased \
             payload layout it does not support indexing"
                .to_string(),
        ))
    }
}

fn sum_payload_expected(db: &mut Db, decl: StructId, disc: u32, sum_ty: &Ty) -> Option<Ty> {
    let ctor = db
        .type_decl_by_occ(decl)?
        .variants
        .get(disc as usize)?
        .ctor?;
    let pty = crate::infer::payload_ty_at_instantiation(db, ctor, sum_ty)?;
    // Only use a CONCRETE result — a still-free payload type is no better than the payload's own.
    if ty_has_free_arg(&pty) {
        None
    } else {
        Some(pty)
    }
}

/// Whether a solved type is UNDER-DETERMINED — it contains a free type variable (`Ty::Var`) or the
/// unconstrained `Ty::Any`, so `lower::type_ast` cannot render it (it returns `None` for those). Used by the
/// `Core::SumNew` emit to decide whether the node's OWN solved type is usable, or whether it must fall back
/// to the `expected` type its context supplied (e.g. a bare `(None)` whose own type is `Option<?>`). Walks
/// the type's structure so a free arg NESTED inside a compound (`Option<List<?>>`) is caught too.
/// A dedup SIGNATURE for a map KEY that folded to a compile-time CONSTANT (the only keys that can collide as
/// duplicate LITERALS in the re-emitted `#map` and trip the front-end's CDZ0201 duplicate-literal-key check).
/// `Some(sig)` iff the key's core is a constant leaf — `sig` is equal iff the key VALUE is equal (type-prefixed
/// so `ConstInt(5)` and `ConstStr("5")` never collide). `None` for a RUNTIME key (never a literal, always kept).
fn map_key_const_sig(db: &mut Db, k: StructId) -> Option<String> {
    match core_of(db, k) {
        Core::ConstInt(v) => Some(format!("I{v:?}")),
        Core::ConstStr(s) => Some(format!("S{s}")),
        Core::ConstBool(bo) => Some(format!("B{bo}")),
        Core::ConstChar(c) => Some(format!("C{c}")),
        Core::ConstBytes(b) => Some(format!("Y{b:?}")),
        _ => None,
    }
}

fn ty_has_free_arg(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) | Ty::Any => true,
        Ty::List(t) | Ty::Set(t) => ty_has_free_arg(t),
        Ty::Map(k, v) => ty_has_free_arg(k) || ty_has_free_arg(v),
        Ty::Tuple(ts) => ts.iter().any(ty_has_free_arg),
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args.iter().any(ty_has_free_arg),
        Ty::Record(fs) => fs.values().any(ty_has_free_arg),
        Ty::Qty { inner, .. } => ty_has_free_arg(inner),
        Ty::Fn(a, r) => ty_has_free_arg(a) || ty_has_free_arg(r),
        Ty::Cont { resume, answer } => ty_has_free_arg(resume) || ty_has_free_arg(answer),
        _ => false,
    }
}

/// `(. <operand> <key>)` — the member-access form the reader normalizes a dotted `X.key` to. Used to
/// re-emit a wrapper constant's CONSTRUCTOR (`(Symbol.of …)`, `(BigInt.of …)`, `(Rational.of …)`), whose
/// value-form is not valid expression syntax. Mirrors `lower::member_access`.
fn member_access(b: &mut Builder, operand: &str, key: &str) -> StructId {
    let dot = b.name(".");
    let op = b.name(operand);
    let k = b.name(key);
    b.list(vec![dot, op, k])
}

/// A member access `(. <module> <key>)` where the module head is an already-built AST node (not a bare
/// name) — used for a fixed-width int module whose surface may be the ctor application `(Int 24)`.
fn member_access_node(b: &mut Builder, module: StructId, key: &str) -> StructId {
    let dot = b.name(".");
    let k = b.name(key);
    b.list(vec![dot, module, k])
}

/// The recompilable SURFACE for a fixed-width integer type used as a MEMBER-ACCESS module head
/// (`(. <module> wrap)` / `(. <module> of)` / `(. <module> wrapping-add)`). An ALIASED width
/// (8/16/32/64) has a bound type name (`Int32`/`UInt8`), so the module is that bare name. An ODD width
/// has NO alias — its surface is the constructor application `(Int 24)` / `(UInt 48)` (the `Int`/`UInt`
/// ctor applied to the width), so a bare `Int24`/`UInt48` name would be UNBOUND (CDZ0101) on recompile.
fn int_module_ast(b: &mut Builder, it: crate::ty::IntTy) -> StructId {
    let width = it.ground_width();
    let stem = if it.ground_signed() { "Int" } else { "UInt" };
    if matches!(width, 8 | 16 | 32 | 64) {
        b.name(format!("{stem}{width}"))
    } else {
        let ctor = b.name(stem);
        let w = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(i64::from(width)),
            radix: Radix::Dec,
        });
        b.list(vec![ctor, w])
    }
}

/// The SURFACE operator a runtime-operator prim re-emits as, or `None` for a prim that is not a binary
/// operator (defensive — such a prim never appears in `Arith`/`Compare`/`StrCmp`/`FloatCompare`). The
/// reverse of `Prim::from_name` for the operator subset. Each INTERNAL float prim maps to the SAME
/// surface operator as its integer twin (`FAdd`→`+`, `FEq`→`=`, `FLt`→`<`, …): the author writes one
/// operator and `lower` selects the prim by the operands' solved type, so re-emitting the shared surface
/// operator re-solves to the same prim on recompile — the property round-trip idempotence rests on.
fn prim_operator(op: crate::resolved::Prim) -> Option<&'static str> {
    use crate::resolved::Prim::*;
    Some(match op {
        Add | FAdd => "+",
        Sub | FSub => "-",
        Mul | FMul => "*",
        Div | FDiv => "/",
        Rem => "%",
        Shl => "<<",
        Shr => ">>",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        Lt | FLt => "<",
        Gt | FGt => ">",
        Le | FLe => "<=",
        Ge | FGe => ">=",
        Eq | FEq => "=",
        // `Compare` (three-way ordering) is NOT a bare operator — its surface is the member `Ordering.of`
        // (`compare` is no longer a top-level name), so it is handled by the caller, not here.
        _ => return None,
    })
}

/// A short human-readable kind name for a `Core` node, for the decline message (so a decline says WHICH
/// construct is not yet lowered rather than an opaque debug dump).
fn core_node_kind(c: &Core) -> String {
    // The variant NAME, derived from `Core`'s `Debug` (`Foo { .. }` / `Bar(..)` / `Baz` → `Foo`/`Bar`/`Baz`)
    // — so a decline names EXACTLY which Core node blocked, without a hand-maintained match to drift or a
    // debug build to inspect (breaker's diagnostic nit). Cheap: one `Debug` format on the decline path only.
    let dbg = format!("{c:?}");
    dbg.split([' ', '(', '{'])
        .next()
        .unwrap_or("<unknown>")
        .to_string()
}
// Behavioral coverage for this backend lives in the CORPUS round-trip check through the nix per-case
// pipeline (operator directive: e2e behavior belongs in the conformance/corpus suite, NEVER a Rust
// `#[test]`), not here — see the `corpus-cadenza` target (coordinated with v-nix).
