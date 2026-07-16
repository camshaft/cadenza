//! `select` — instruction selection for the wasm backend: the core (A-normal, structured) form of a
//! definition body linearized into a flat `Vec<Lir>`.
//!
//! This is the wasm backend's linearization of the core (`backends-and-targets.md` §A Backend
//! Linearizes The Core Only If Its Target Is Linear). It reads a node's core form (via
//! [`crate::lower::core_of`]) and its solved type (via [`crate::infer::type_of`]) — the machine
//! representation is a READ-OFF of the solved type (`reference-compiler.md` §A Value's Machine
//! Representation Follows Its Solved Type At Selection), not a guess from the node's shape. It is
//! where a deferred integer width GROUNDS to its machine width, and where a literal that does not fit
//! its solved width DECLINES rather than emitting a truncated value.
//!
//! A construct the flat rung cannot express declines (`reference-compiler.md` §A Guarded Operation
//! Reserves Bounded Scratch Or Declines). What is selected: constant pushes, a structured
//! `if`/`else`/`end`, checked arithmetic and comparisons (guarded scratch locals), truncating
//! conversions, a `match` as a probe chain, a runtime `Core::Call`, and value-heap construction/
//! projection for tuples, records, and sums. A construct without a machine form here declines (e.g. a
//! runtime compound of a type that cannot yet cross the boundary).
//!
//! Selection reads an ALREADY-RESOLVED representation: it consumes the core form (`core_of`, itself a
//! read of the resolved column `resolved_of`), where every name reference is already resolved to the
//! binding it denotes — so this pass reads a resolved binding rather than searching a scope.
//= spec/capabilities/compiler-pipeline.md#the-compiler-resolves-names-before-it-selects-instructions
//# The compiler MUST lower the AST to an intermediate representation in which every name reference is resolved to the binding it denotes before it selects the instructions to emit, so that instruction selection reads a resolved binding rather than searching a scope.

use crate::ast::StructId;
use crate::backend::wasm::lir::{BlockType, Lir, ValType, valtype_of};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Ty};
use std::collections::{HashMap, HashSet};
use tracing::trace;

/// The emit buffer — the flat `Vec<Lir>` a body linearizes into, PLUS a per-construct source-line map
/// for debug info (`DESIGN-debug-line-granularity-rcdzc.md`). Wrapping the vector (rather than threading
/// a second `&mut` param through the ~28-function emit family) means every existing `out.push(…)` /
/// `out.contains(…)` / `out.last()` site works UNCHANGED via `Deref`/`DerefMut` — the wrapper adds a
/// channel, not a rewrite.
///
/// `lines` records `(instruction index, source StructId)` at each point a distinct source construct's
/// evaluation BEGINS — marked by `mark(id)` at every `StructId`-consuming emit point (the coverage the
/// first attempt lacked). The backend turns these into `.debug_line` rows (mapping code offset → source
/// line), dedups a repeated offset (keeps the first — the outer construct), and collapses consecutive
/// same-line rows so the table has one row per LINE the code visits. Indices are into `code` as emitted;
/// `peephole_emit` remaps them when it fuses `set;get`→`tee` (which shifts later indices down).
#[derive(Default)]
pub struct Emit {
    code: Vec<Lir>,
    lines: Vec<(u32, StructId)>,
    /// Named SCALAR `let`-binding locals discovered during emit (D3 variable inspection extended to
    /// locals — `DESIGN-debug-info-rcdzc.md` §2.4). A kept multi-use scalar binding lives in a stable
    /// slot; recorded here at the `Core::Let` arm so `DW_TAG_variable` DIEs describe it, letting a
    /// debugger `print x` for a local, not just a parameter. Params are collected separately in
    /// `select_function_of` (slots `0..n`); these are the bindings above `base`.
    binding_locals: Vec<LocalVar>,
    /// Scalar MATCH-BINDER lexical scopes (D3 locals for `(match e (x body)…)`). A scalar match spills
    /// its scrutinee to ONE slot for the whole match; a bare-binder arm binds that slot's value. Unlike a
    /// param/let (function-scoped), a match binder is live ONLY within its match expression — and its
    /// slot is a REUSED scratch slot the rest of the function repurposes — so a flat function-scoped
    /// `DW_TAG_variable` would MISLEAD. Recorded as a scope `(Lir range, vars)` so the backend emits a
    /// `DW_TAG_lexical_block` with a PC range that fences the binder to its arms. Indices are into `code`
    /// as emitted; `peephole_emit` remaps them alongside `lines`.
    match_scopes: Vec<MatchScope>,
    /// SHARED SUM-PAYLOAD-PREFIX slots (a per-arm-body CSE). A match arm reading MULTIPLE elements of one
    /// payload tuple — `(Node (tuple l r))` → `l`/`r` each a `SumPayload{s, [Payload, Elem(i)]}` — would
    /// re-walk the `sum-payload(s)` prefix per element. Before emitting such an arm body, the shared
    /// prefix is computed ONCE into a slot and recorded here keyed by `(scrutinee-id, the prefix STEPS)`;
    /// the `Core::SumPayload` emit then reads the slot + walks only the SUFFIX. Populated ONLY at an arm-
    /// body top (a save/restore fences it to that arm), and ONLY for a prefix whose shared extensions are
    /// all BORROWING `Elem` reads — sound because `op_sum-payload` is TOTAL (never traps) and BORROWING (no
    /// refcount change), so computing it once when the arm is entered matches per-element re-walks exactly.
    ///
    /// 🩸 The key carries the FULL prefix STEPS, NOT just its length: a TUPLE-OF-TWO-SUMS match
    /// (`match (a, b) with (TArrow(a1,a2), TArrow(b1,b2)) => …`) produces TWO distinct prefixes of the SAME
    /// length off the SAME tuple scrutinee — `[Elem(0), Payload]` (a's payload) and `[Elem(1), Payload]`
    /// (b's payload). A length-only key `(scrutinee, 2)` COLLIDED them, so the second overwrote the first and
    /// the emit fast-path read `b`'s payload from `a`'s slot — a SILENT MISCOMPILE (`unify(a2,b2)` reading
    /// `unify(a2,a2)`). The steps discriminate the two, so each gets its own slot.
    payload_prefix_slots: HashMap<(StructId, Vec<crate::core::PathStep>), u32>,
    /// Perceus RETAIN sites (`collect_dup_sites`): the `Core::LocalRef`/`Core::Param` OCCURRENCE ids whose
    /// reference is consumed while the binding has a later live use — a `dup` is emitted after the
    /// `LocalGet` at each so the consumer gets its own reference and the later use reads the original.
    /// Computed ONCE at function entry over all heap binders (params + `let`-binders); empty for a body
    /// with no shared-then-consumed heap binding (the common case), so the fast path is untouched.
    dup_sites: HashSet<StructId>,
    /// ENTERED-VARIANT PAYLOAD TYPES for a sum decision tree — `switch_path + [Payload]` → the payload type
    /// of the variant an ENCLOSING switch arm entered. A nested switch / literal-test / disc-walk resolves
    /// a `Payload` step's sub-value type from here, so it descends the ACTUAL entered variant, not variant 0
    /// (which `sum_single_payload_ty` blindly reads). Without this, a `Payload` step into a non-variant-0
    /// variant whose payload is a `List` mis-picked `arr-get` over an RRB vec (a SILENT miscompile: reading
    /// a list element's discriminant to dispatch a nested pattern `Ast.List([Ast.Name n, ..])`). Recorded
    /// with SCOPED save/restore as each switch arm is emitted (like `payload_prefix_slots`), so a sibling
    /// arm's `Payload` at the same path sees ITS own variant's type, not this arm's. This mirrors the Rust
    /// backend's `Ctx::sum_path_types`. Empty at the root/top level (the walk falls back to variant 0 there,
    /// which IS the root scrutinee's own type via `type_of`).
    sum_path_types: HashMap<Vec<crate::core::PathStep>, Ty>,
}

/// A scalar match's binder scope: the `[start, end)` Lir range spanning its arm bodies, and the binder
/// locals visible there (one per distinct binder name across the arms, all aliasing the scrutinee's
/// spill slot). Becomes a `DW_TAG_lexical_block` in the DWARF (`DESIGN-debug-info-rcdzc.md` §2.4). The
/// `start_ix`/`end_ix` are Lir indices (remapped by `peephole_emit`); `dwarf_funcs_for` turns them into
/// absolute code offsets for the block's `DW_AT_low_pc`/`high_pc`.
#[derive(Clone, Debug)]
pub struct MatchScope {
    pub start_ix: u32,
    pub end_ix: u32,
    pub vars: Vec<LocalVar>,
}

impl Emit {
    fn new() -> Emit {
        Emit::default()
    }
    /// Mark that the source construct `id` begins at the CURRENT instruction position — its first
    /// emitted instruction is the next `push`. Dedups a repeated offset (two marks at the same index
    /// keep the FIRST, i.e. the outer/earlier construct's line). The caller guards to user nodes (a
    /// prelude/synthesized node has no source span, so a mark for it would map to a garbage line).
    fn mark(&mut self, id: StructId) {
        let at = self.code.len() as u32;
        if self.lines.last().map(|&(i, _)| i) != Some(at) {
            self.lines.push((at, id));
        }
    }
    /// Record a named scalar `let`-binding local at its persistent slot (D3 locals). Called at the
    /// `Core::Let` arm for each SCALAR binding whose binder occurrence has a source name.
    fn binding_local(&mut self, slot: u32, name: String, ty: Ty) {
        self.binding_locals.push(LocalVar {
            slot,
            name,
            ty,
            is_param: false,
        });
    }
    /// The CURRENT instruction position — the start/end anchor for a match-binder scope (`match_scope`).
    fn here(&self) -> u32 {
        self.code.len() as u32
    }
    /// Record a scalar match-binder lexical scope: the `[start, end)` Lir range over its arm bodies plus
    /// the binder locals visible there (D3 locals). Skips an empty scope (no named binder / no code).
    fn match_scope(&mut self, start_ix: u32, end_ix: u32, vars: Vec<LocalVar>) {
        if !vars.is_empty() && end_ix > start_ix {
            self.match_scopes.push(MatchScope {
                start_ix,
                end_ix,
                vars,
            });
        }
    }
}

impl std::ops::Deref for Emit {
    type Target = Vec<Lir>;
    fn deref(&self) -> &Vec<Lir> {
        &self.code
    }
}
impl std::ops::DerefMut for Emit {
    fn deref_mut(&mut self) -> &mut Vec<Lir> {
        &mut self.code
    }
}

// The value-heap runtime ops the tuple path emits, referenced by their WIT names (the same names the
// generated `runtime_abi` table + the import section resolve by). Named here so the emit reads clearly
// and `collect_used_ops` and `emit` agree on exactly one spelling per op.
const OP_ARR_ALLOC: &str = "arr-alloc";
const OP_ARR_SET: &str = "arr-set";
const OP_ARR_GET: &str = "arr-get";
const OP_BOX_INT: &str = "box-int";
const OP_GET_INT: &str = "get-int";
const OP_BOX_BOOL: &str = "box-bool";
const OP_GET_BOOL: &str = "get-bool";
const OP_BOX_FLOAT: &str = "box-float";
const OP_GET_FLOAT: &str = "get-float";
const OP_BOX_FLOAT32: &str = "box-float32";
const OP_GET_FLOAT32: &str = "get-float32";
/// `sum-new(disc, payload) -> handle` — build a sum value from its discriminant and a single payload
/// handle (`value-heap-runtime.md` §Sum). The payload is: an empty array for a nullary variant, the
/// boxed value for a one-payload variant, or a tuple handle for a multi-payload variant.
const OP_SUM_NEW: &str = "sum-new";
/// `sum-disc(handle) -> u32` — read a sum value's discriminant (which variant), driving a match's
/// dispatch. `sum-payload(handle) -> u32` — the sum's payload handle, unboxed to the bound value.
const OP_SUM_DISC: &str = "sum-disc";
const OP_SUM_PAYLOAD: &str = "sum-payload";
/// Persistent-vector (list) ops. `vec-empty() -> handle` — a fresh empty list; `vec-push(handle, elem)
/// -> handle` — append an element (returns the new list, threading the handle); `vec-len(handle) -> u32`
/// — the length. A list value is built `vec-empty` then a `vec-push` per element.
const OP_VEC_PUSH: &str = "vec-push";
const OP_VEC_LEN: &str = "vec-len";
/// `bytes-alloc(len) -> handle` — a fresh mutable byte buffer of `len` zero bytes (filled by `bytes-set`).
const OP_BYTES_ALLOC: &str = "bytes-alloc";
/// `bytes-set(buf, index, byte)` — set the byte at `index` (the byte is an i32 in `0..=255`; the caller
/// range-checks). Used to fill a `bytes-alloc` buffer element by element at construction.
const OP_BYTES_SET: &str = "bytes-set";
/// `bytes-len(b) -> u32` — the byte count of a byte sequence (extended to `Int64` at the boundary).
const OP_BYTES_LEN: &str = "bytes-len";
/// `bytes-get(b, index) -> u32` — the byte at `index`, a RAW value in `0..=255` (NOT a heap handle,
/// unlike `vec-get`), so no `dup` is needed; the caller bounds-checks (an OOB index TRAPS).
const OP_BYTES_GET: &str = "bytes-get";
/// `bytes-concat(a, b) -> handle` — a then b (consumes both, empty is the identity).
const OP_BYTES_CONCAT: &str = "bytes-concat";
/// The runtime BigInt ops (B3a) the compiler emits for RUNTIME-valued BigInt (a constant folds in
/// `lower`). Boxed sign-magnitude heap leaves; add/sub/mul never trap, div traps on zero, to-i64-checked
/// traps out of range. Spellings MUST match `runtime.wit` / the generated `runtime_abi.rs` table.
const OP_BIGINT_OF_I64: &str = "bigint-of-i64";
/// `bigint-of-bytes(buf) -> u32` — a BigInt leaf from a Bytes leaf holding the canonical sign-magnitude
/// bytes; the beyond-i64 CONSTANT materialization (`bigint-of-i64` handles only an i64-fitting constant).
const OP_BIGINT_OF_BYTES: &str = "bigint-of-bytes";
const OP_BIGINT_TO_I64_CHECKED: &str = "bigint-to-i64-checked";
const OP_BIGINT_ADD: &str = "bigint-add";
const OP_BIGINT_SUB: &str = "bigint-sub";
const OP_BIGINT_MUL: &str = "bigint-mul";
const OP_BIGINT_DIV: &str = "bigint-div";
const OP_BIGINT_REM: &str = "bigint-rem";
/// `bigint-cmp(a, b) -> s64` — the three-way compare (`-1`/`0`/`1` for `a<b`/`a=b`/`a>b`), which the
/// BigInt comparison operators `<`/`>`/`<=`/`>=`/`=` lower to + a fixed signed compare-with-zero (B3c).
const OP_BIGINT_CMP: &str = "bigint-cmp";
/// The runtime Rational ops (R3a) the compiler emits for RUNTIME-valued Rational (a constant folds in
/// `lower`). A Rational is a normalized 2-BigInt-handle node. `rational-of` CONSUMES its two BigInt
/// operand handles; the arithmetic/compare BORROW. Spellings MUST match `runtime.wit`/`runtime_abi.rs`.
const OP_RATIONAL_OF: &str = "rational-of";
const OP_RATIONAL_ADD: &str = "rational-add";
const OP_RATIONAL_SUB: &str = "rational-sub";
const OP_RATIONAL_MUL: &str = "rational-mul";
const OP_RATIONAL_DIV: &str = "rational-div";
const OP_RATIONAL_CMP: &str = "rational-cmp";
/// `bytes-slice(buf, start, len) -> handle` — `len` bytes from `start` (consumes buf; `start+len >
/// bytes-len` TRAPS, so the caller bounds-checks first and returns `None` instead).
const OP_BYTES_SLICE: &str = "bytes-slice";
/// `bytes-compact(buf) -> handle` — a content-equal sequence with independent storage (consumes buf).
const OP_BYTES_COMPACT: &str = "bytes-compact";
/// `str-from-bytes(buf) -> handle` — the runtime TOTAL UTF-8 decode: strictly validate `buf` as
/// well-formed UTF-8 and return it AS a String (a String IS a UTF-8 Bytes leaf, so a valid buffer is
/// re-tagged with no copy), or `NULL` when invalid. CONSUMES `buf`. The compiler wraps the handle-or-NULL
/// into the `(Option String)` sum (`Some buf` / `None`). Never traps.
const OP_STR_FROM_BYTES: &str = "str-from-bytes";
/// `vec-concat(a, b) -> handle` — concatenate two lists into one.
const OP_VEC_CONCAT: &str = "vec-concat";
/// `vec-update(v, index, elem) -> handle` — replace the element at `index` (returns the new list; an
/// out-of-bounds `index` traps).
const OP_VEC_UPDATE: &str = "vec-update";
/// `vec-get(v, index) -> handle` — the element at `index`, BORROWED (rc unchanged; the list still owns
/// it). An out-of-bounds index TRAPS, so `List.at` bounds-checks BEFORE calling it.
const OP_VEC_GET: &str = "vec-get";
/// `vec-drop(v, index) -> handle` — the TAIL `[index, len)` of the RRB vector, dropping the prefix
/// `[0, index)`, CONSUMING `v`. A single-u32 result (unlike `vec-split`'s tuple retarea). A list REST
/// binder `(list p… .. rest)` binds `rest` = `vec-drop(list, leading-count)`.
const OP_VEC_DROP: &str = "vec-drop";
/// `vec-of-arr(arr) -> handle` — build a persistent vector from an already-built flat `arr` in ONE call
/// (CONSUMES the arr). The bulk-construct lowering target for a `(list …)` literal: `arr-alloc N` + N×
/// `arr-set` then one `vec-of-arr`, instead of `vec-empty` + N× consuming `vec-push`. `arr-len 0` yields
/// the empty vector, so it covers `(list)` too.
const OP_VEC_OF_ARR: &str = "vec-of-arr";
/// `drop` — release a reference to a heap handle (the Perceus calling convention). At refcount 0 the
/// runtime frees the node and recursively releases its children (the boxed elements), so a single
/// `drop` of a dead tuple reclaims the whole value.
///
/// Reclamation is this emitted reference-count discipline — the compiler places `drop`/`dup` at the
/// source-determined points its escape analysis fixes — NOT a tracing garbage collector the runnable
/// form depends on, and because the release points are a static function of the source, the timing of
/// reclamation is not a source of observable nondeterminism.
//= spec/capabilities/memory-and-resource-model.md#the-runnable-form-needs-no-collector
//# The runnable form of a program MUST NOT depend on a tracing garbage collector for correctness.
//= spec/capabilities/memory-and-resource-model.md#the-runnable-form-needs-no-collector
//# The timing of memory reclamation MUST NOT be a source of nondeterminism in a program's observable behavior.
const OP_DROP: &str = "drop";
/// `dup(handle)` — increment a heap handle's refcount (the Perceus retain). Emitted where a construct
/// takes ownership of a handle it only BORROWED — `List.at` `dup`s the `vec-get` element before the
/// `Some` payload consumes it, so the list keeps its own reference.
const OP_DUP: &str = "dup";
/// `value-eq(a, b) -> bool` — deep STRUCTURAL equality over two compound heap values (the `champ_eq`
/// walk). BORROWS both operands (an inspector, like `sum-disc`/`vec-len`): it changes neither refcount,
/// so an owned-temporary operand is `drop`ped by the emit AFTER the compare. The runtime `=` on two
/// runtime compounds neither of which the compiler folded.
const OP_VALUE_EQ: &str = "value-eq";
/// Persistent CHAMP map ops. `map-empty() -> handle` — the canonical empty map; `map-insert(m, key, val)
/// -> handle` — add-or-replace (CONSUMES m, key, val; returns the new map); `map-lookup(m, key) -> handle`
/// — the value for `key` or NULL when absent (BORROWS m + key); `map-remove(m, key) -> handle` — m without
/// `key` (CONSUMES m; BORROWS key); `map-size(m) -> u32` — the entry count (BORROWS, O(1)). Keys and values
/// cross as plain handles; the runtime compares keys by a tagless structural walk.
const OP_MAP_EMPTY: &str = "map-empty";
const OP_MAP_INSERT: &str = "map-insert";
const OP_MAP_LOOKUP: &str = "map-lookup";
const OP_MAP_REMOVE: &str = "map-remove";
const OP_MAP_SIZE: &str = "map-size";
const OP_MAP_TO_LIST: &str = "map-to-list";
/// Persistent CHAMP set ops (CHAMP-minus-value-column). `set-empty() -> handle`; `set-insert(s, elem) ->
/// handle` (consumes s, elem); `set-contains(s, elem) -> bool` (BORROWS both); `set-remove(s, elem) ->
/// handle` (consumes s; borrows elem); `set-size(s) -> u32` (borrows, O(1)); `set-union`/`set-intersection`/
/// `set-difference(a, b) -> handle` (consume both). Elements cross as plain handles, compared structurally.
const OP_SET_EMPTY: &str = "set-empty";
const OP_SET_INSERT: &str = "set-insert";
const OP_SET_CONTAINS: &str = "set-contains";
const OP_SET_REMOVE: &str = "set-remove";
const OP_SET_SIZE: &str = "set-size";
const OP_SET_TO_LIST: &str = "set-to-list";
const OP_SET_UNION: &str = "set-union";
const OP_SET_INTERSECTION: &str = "set-intersection";
const OP_SET_DIFFERENCE: &str = "set-difference";
/// NULL — the absent-value handle `map-lookup` returns for a key the map does not contain (the runtime's
/// canonical null handle, 0). `Map.lookup` tests the returned handle against it to build `None` vs `Some`.
const NULL_HANDLE: i32 = 0;

/// Whether a solved type is a HEAP VALUE — one held as an owned runtime handle that the Perceus
/// contract reclaims (a tuple, record, sum, or list). A scalar (integer/bool/unit) owns no heap cell,
/// so it is never dup'd/drop'd. This is what decides which `let` bindings get a closing `drop`, and it
/// gates the branchless-`select` `if` lowering OUT for a heap result (a `select` on a handle would be
/// ill-formed). A `Ty::List` is an owned `vec-*` handle exactly like a tuple/record/sum — it MUST be
/// listed here, and `valtype_of` already agrees it is an i32 handle; omitting it let an `if` over a
/// list take the scalar `select` path and emit a module that failed wasm validation (i64/i32 mismatch).
///
/// This predicate is where the reference-count reclamation the emitted component CARRIES is decided: a
/// heap-typed `let` binding gets a `drop` emitted after the body (see `emit`), so the runnable form
/// releases each value's storage after its last use — the release point being a static consequence of
/// the source, not a later collector sweep — and the runtime it targets need supply only raw memory
/// (the `alloc`/`drop`/`dup` refcount discipline is emitted BY the component, imported by name).
//= spec/capabilities/memory-and-resource-model.md#reclamation-is-carried-by-the-runnable-form
//# The runnable form of a program MUST carry its own allocation and reclamation of values, so that the runtime it targets need provide only raw memory rather than a memory manager.
//= spec/capabilities/memory-and-resource-model.md#cleanup-is-source-determined
//# A value's storage MUST be released after its last use in a way the executable semantics defines, rather than at an unspecified later time.
fn is_heap_type(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        // A String / Symbol is a heap ROPE at run time exactly as Bytes is (see `elem_needs_rope_
        // compaction`, which treats all three alike). It MUST count as heap here so a String/Symbol
        // binder/param threaded past a consuming use (`String.concat(acc, s)` where `s` is also passed to
        // a self-recursive call) is a Perceus RETAIN candidate — else no `dup` is emitted, the shared rope
        // is freed while still referenced, and the rope walk reads OUT OF BOUNDS past a depth threshold (a
        // wasm trap). Omitting String/Symbol here was the gap the List ops did not hit (List was included).
        | Ty::String
        | Ty::Symbol
        | Ty::Bytes => true,
        // A quantity ERASES to its inner numeric type before the backend (`lower` strips the `Qty`), so
        // a `Ty::Qty` should not reach selection. Defensively classify it by its inner type — a quantity
        // over a heap numeric would be heap, but Layer 1's numerics are all scalars (int/float).
        Ty::Qty { inner, .. } => is_heap_type(inner),
        // A NOMINAL tag "adds nothing to the value's runtime representation" (type-system.md §156) — at run
        // time a `Ty::Nominal` IS its `inner` shape (a single-variant newtype like `(type Box (B (List T)))`
        // ERASES to the bare list handle). So a nominal binder wrapping a heap shape (`bx : Box` = a list) is
        // a heap value and MUST be a Perceus RETAIN candidate — else `bx` threaded past a consuming use of
        // its erased payload gets no `dup` and the shared handle is FBIP-mutated while still referenced
        // (drift). Classify by the erased inner shape, exactly like `Qty`.
        Ty::Nominal { inner, .. } => is_heap_type(inner),
        _ => false,
    }
}

/// Whether the reference to the `let` binding `binder` ESCAPES the node at `id` — i.e. its reference
/// flows into a value that OUTLIVES the `let` (the returned result, an element of a constructed tuple,
/// or a call argument — all CONSUMING positions), as opposed to being used only to BORROW (a
/// `Core::Proj` operand: `arr-get` borrows and does not transfer ownership). An escaped binding must
/// NOT be dropped — its ownership transfers to the consumer (ownership-transfer-on-return). This is a
/// CONSERVATIVE analysis: any occurrence that is not provably a borrow is treated as an escape, so a
/// value is never wrongly reclaimed (a false "escapes" only leaks in a case we do not yet emit; a false
/// "does not escape" would be a use-after-free, which this avoids). `tail` marks whether `id` is in the
/// body's TAIL (result) position — a bare `LocalRef` in tail position is the return, an escape.
///
/// This is the aliasing discipline the compiler applies INTERNALLY: the escape/borrow classification is
/// computed here from the source, deciding where a `dup` retains and where a `drop` reclaims — the
/// program's author writes no use-count and no aliasing annotation to be memory-safe. Because the
/// analysis is conservative (only a provable borrow is treated as non-escaping), a live value is never
/// reclaimed under one reference while another still reads it, so the emitted component has no
/// unspecified aliasing behavior.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# The aliasing discipline MUST be one the compiler applies internally to reclaim and reuse storage, rather than a use-counting obligation the program's author writes, so that a program's author states no aliasing annotation to be memory-safe.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# A value MUST NOT be observably mutated through one reference while it is read through another in a way the executable semantics leaves unspecified.
fn binding_escapes(db: &mut Db, id: StructId, binder: StructId, tail_borrowed: bool) -> bool {
    match core_of(db, id) {
        // A reference to the binding: it escapes UNLESS this occurrence is a borrow (the operand of a
        // `Proj`, which `arr-get`-borrows). `tail_borrowed` is set by the `Proj` arm below for its
        // operand; every other occurrence (the result, a tuple element, a call arg) is consuming.
        Core::LocalRef { binder: b } => b == binder && !tail_borrowed,
        // A projection of a SCALAR element BORROWS its operand — `arr-get` then `get-int`/`get-bool` COPIES
        // the value out, retaining nothing from the aggregate — so a `LocalRef` directly under such a `Proj`
        // does not escape through it (recurse with the borrow flag). But a projection of a NESTED-COMPOUND
        // element returns the CHILD HANDLE, a live reference INTO the aggregate that TRANSFERS OUT to the
        // consumer (a call arg, a constructor element, or the return); if the aggregate were then dropped,
        // that drop would cascade to free the extracted child — a use-after-free (the byte-decode `(let ((r
        // (one …))) (loop … (. r 0)))` threading a boxed-sum `(. r 0)` into a param returned garbage). So a
        // nested-compound projection ESCAPES its operand: the operand must NOT be reclaimed (its child left
        // through the projection). Conservative — the aggregate's array + its other children leak rather
        // than risk the UAF (the analysis's stated bias: a false "escapes" only leaks). `get_op(id)` is
        // `Some` for a scalar element (borrow), `None` for a nested-compound (escape). `List.len`/`Bytes.len`
        // (`vec-len`/`bytes-len`) read a scalar count — always a borrow.
        Core::ListLen { operand } | Core::BytesLen { operand } => {
            binding_escapes(db, operand, binder, true)
        }
        Core::Proj { operand, .. } => {
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            binding_escapes(db, operand, binder, scalar_element)
        }
        // `List.at` BORROWS its list (`vec-len`/`vec-get` both borrow; the read element is DUP'd into the
        // `Some` payload rather than moved) — so a list bound here does not escape through `List.at`. The
        // index is a scalar. Recurse borrowing the list; the index cannot hold a heap reference.
        Core::ListAt { list, index, .. } => {
            binding_escapes(db, list, binder, true) || binding_escapes(db, index, binder, false)
        }
        // `Bytes.at` BORROWS its bytes (`bytes-len`/`bytes-get` both borrow; the byte read is a raw i32
        // VALUE, not a heap handle, so nothing is retained from the sequence). The index is a scalar.
        Core::BytesAt { bytes, index, .. } => {
            binding_escapes(db, bytes, binder, true) || binding_escapes(db, index, binder, false)
        }
        // `String.at` BORROWS its string — the `Some` branch `dup`s it before the `bytes-slice` consumes
        // the copy (so the returned slice owns an INDEPENDENT reference, not part of the source), and the
        // `None` branch takes no reference. So a binding used as the string operand does NOT escape through
        // `String.at` — the enclosing `let`/owner still reclaims it — exactly like `List.at`/`Bytes.at`.
        // The index is a scalar. (This borrow discipline is why `String.at` composes in a recursive char
        // scan that threads the same string through both `String.at` and the recursive call.)
        Core::StrAt { string, index, .. } => {
            binding_escapes(db, string, binder, true) || binding_escapes(db, index, binder, false)
        }
        // `String.slice` BORROWS its string operand (the Some branch `dup`s it before the consuming
        // `bytes-slice`, the None branch takes no reference — same discipline as `String.at`), so a binding
        // used as the string does NOT escape (its owner reclaims it). start/end are scalars.
        Core::StrSlice {
            string, start, end, ..
        } => {
            binding_escapes(db, string, binder, true)
                || binding_escapes(db, start, binder, false)
                || binding_escapes(db, end, binder, false)
        }
        // `Bytes.concat`/`slice`/`compact` all CONSUME their bytes operand(s) into the new sequence
        // (`bytes-concat`/`bytes-slice`/`bytes-compact` consume, per `value-heap-runtime.md §Constructors
        // Consume`). A binding used as an operand escapes into the result. `slice`'s start/len are scalars.
        Core::BytesConcat { lhs, rhs } => {
            binding_escapes(db, lhs, binder, false) || binding_escapes(db, rhs, binder, false)
        }
        // The runtime BigInt ops BORROW their operand handles (`bigint-add`/…/`to-i64-checked` `unbox_
        // bigint`-read without consuming, then the `emit_bigint_borrow_*` helpers drop only an OWNED
        // temporary), so — like `value-eq` — a binding used DIRECTLY as an operand does NOT escape (the
        // enclosing `let` still drops it). A binding that flows into a CONSTRUCTED/owned operand (e.g.
        // `(+ (BigInt.of x) y)` where `x` feeds a `BigInt.of`) DOES escape into that owned temporary,
        // which the op then drops — the `tail_borrowed: true` borrow-in-tail computes exactly this (a
        // direct `LocalRef` borrows; a producer arm resets to consuming). `bigint-of-i64`'s operand is an
        // i64 scalar (no heap ref) — always consuming, `false`.
        Core::BigIntBinOp { lhs, rhs, .. } | Core::BigIntCmp { lhs, rhs, .. } => {
            binding_escapes(db, lhs, binder, true) || binding_escapes(db, rhs, binder, true)
        }
        Core::BigIntOfI64 { value } => binding_escapes(db, value, binder, false),
        Core::BigIntToI64 { operand } => binding_escapes(db, operand, binder, true),
        // The runtime Rational arithmetic/comparison ops BORROW their operand handles (`rational-add`/…/
        // `rational-cmp` unbox-read without consuming; the borrow helpers drop only an OWNED temporary), so
        // a binding used DIRECTLY as an operand does NOT escape (`tail_borrowed: true`, like the BigInt
        // arith). `RationalOfInts`'s num/den + `RationalOfIntWiden`'s value are i64 SCALARS (no heap ref) —
        // always consuming, `false`.
        Core::RationalBinOp { lhs, rhs, .. } | Core::RationalCmp { lhs, rhs, .. } => {
            binding_escapes(db, lhs, binder, true) || binding_escapes(db, rhs, binder, true)
        }
        Core::RationalOfInts { num, den } => {
            binding_escapes(db, num, binder, false) || binding_escapes(db, den, binder, false)
        }
        Core::RationalOfIntWiden { value } => binding_escapes(db, value, binder, false),
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            binding_escapes(db, bytes, binder, false)
                || binding_escapes(db, start, binder, false)
                || binding_escapes(db, len, binder, false)
        }
        Core::BytesCompact { operand } => binding_escapes(db, operand, binder, false),
        // `String.from-bytes` CONSUMES its bytes operand (`str-from-bytes` transfers ownership out as the
        // String on success, drops it on failure), so a binding used as the operand escapes into the result.
        Core::StrFromBytes { bytes, .. } => binding_escapes(db, bytes, binder, false),
        // `String.to-bytes` CONSUMES its string operand (`bytes-compact` transfers the handle out as the
        // Bytes result), so a binding used as the operand escapes into the result.
        Core::StrToBytes { string } => binding_escapes(db, string, binder, false),
        // A constructed tuple/list CONSUMES each element — a binding used as an element escapes into it.
        // `Bytes.of`'s elements are scalar bytes (Int64 0..=255), consumed into the sequence like a list's.
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            elems.iter().any(|&e| binding_escapes(db, e, binder, false))
        }
        // A runtime `(bin …)` construction consumes each segment's scalar int value into the built bytes.
        Core::BinBuild { segs } => segs
            .iter()
            .any(|s| binding_escapes(db, s.value, binder, false)),
        // A runtime bit-field run consumes each field's scalar value (packed into the built bytes).
        Core::BinBitsBuild { fields } => fields
            .iter()
            .any(|f| binding_escapes(db, f.value, binder, false)),
        // A `BinIntRead` reads (borrows) its bytes operand to decode a segment — a binding used as the
        // scrutinee flows in; treat like a projection operand (does not consume-escape).
        Core::BinIntRead { bytes, .. } | Core::BinRestRead { bytes, .. } => {
            binding_escapes(db, bytes, binder, false)
        }
        // `List.push`/`concat` CONSUME both operands (the persistent op takes ownership of the list and
        // the pushed/concatenated value into the result).
        Core::ListPush { list, elem } => {
            binding_escapes(db, list, binder, false) || binding_escapes(db, elem, binder, false)
        }
        Core::ListConcat { lhs, rhs } => {
            binding_escapes(db, lhs, binder, false) || binding_escapes(db, rhs, binder, false)
        }
        // `List.update` CONSUMES the list and the replacement element into the new list; the `index` is a
        // scalar (passed by value, never a heap handle) so it cannot escape into the result.
        Core::ListUpdate { list, elem, .. } => {
            binding_escapes(db, list, binder, false) || binding_escapes(db, elem, binder, false)
        }
        // A map construction CONSUMES each entry's key AND value into the built map — a binding used as a
        // key or value escapes into it (like a tuple/list element).
        Core::MapNew { entries, .. } => entries.iter().any(|&(k, v)| {
            binding_escapes(db, k, binder, false) || binding_escapes(db, v, binder, false)
        }),
        // `Map.insert` CONSUMES the map, the key, and the value into the new map (the persistent op takes
        // ownership of all three) — any of them used here escapes into the result.
        Core::MapInsert { map, key, val, .. } => {
            binding_escapes(db, map, binder, false)
                || binding_escapes(db, key, binder, false)
                || binding_escapes(db, val, binder, false)
        }
        // `Map.lookup` BORROWS the map (returns a fresh Option; the boxed key is an owned temporary the
        // emit drops), so a map bound here does NOT escape through the lookup. The key flows into an owned
        // temporary — consuming — so it escapes if used there.
        Core::MapLookup { map, key, .. } => {
            binding_escapes(db, map, binder, true) || binding_escapes(db, key, binder, false)
        }
        // `Map.remove` CONSUMES the map into the new map (persistent op takes ownership); the key is boxed
        // into an owned temporary (consuming), dropped by the emit after the borrow-compare.
        Core::MapRemove { map, key, .. } => {
            binding_escapes(db, map, binder, false) || binding_escapes(db, key, binder, false)
        }
        // `Map.size` BORROWS its map operand (`map-size` reads the root without consuming) — like `List.len`.
        Core::MapSize { map } => binding_escapes(db, map, binder, true),
        // A set construction CONSUMES each element into the built set — a binding used as an element
        // escapes into it (like a list element / a map key).
        Core::SetOf { elems, .. } => elems.iter().any(|&e| binding_escapes(db, e, binder, false)),
        // `Set.insert` CONSUMES the set and the element into the new set (persistent op takes ownership) —
        // both escape if used here. `Set.remove` CONSUMES the set; its element is boxed into an owned
        // temporary (consuming), dropped by the emit after the borrow-compare.
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            binding_escapes(db, set, binder, false) || binding_escapes(db, elem, binder, false)
        }
        // `Set.contains` BORROWS the set (returns a bool; the boxed element is an owned temporary the emit
        // drops), so a set bound here does NOT escape; the element flows into an owned temporary (consuming).
        Core::SetContains { set, elem, .. } => {
            binding_escapes(db, set, binder, true) || binding_escapes(db, elem, binder, false)
        }
        // `Set.len` BORROWS its set operand (`set-size` reads the root without consuming) — like `Map.size`.
        Core::SetLen { set } => binding_escapes(db, set, binder, true),
        Core::SetToList { set, .. } => binding_escapes(db, set, binder, true),
        Core::MapToList { map, .. } => binding_escapes(db, map, binder, true),
        // A set-algebra op CONSUMES both operand sets into the result — either escapes if used here.
        Core::SetAlgebra { lhs, rhs, .. } => {
            binding_escapes(db, lhs, binder, false) || binding_escapes(db, rhs, binder, false)
        }
        // A call CONSUMES its arguments; a host call OR a cross-component call likewise consumes its
        // arguments across the boundary.
        Core::Call { args, .. } | Core::HostCall { args, .. } => {
            args.iter().any(|&a| binding_escapes(db, a, binder, false))
        }
        // A sequencing block: the binding escapes if it escapes any statement or the tail.
        Core::Seq { stmts, tail } => {
            stmts.iter().any(|&s| binding_escapes(db, s, binder, false))
                || binding_escapes(db, tail, binder, false)
        }
        // A boundary block / break — the binding escapes if it escapes the body / break value.
        Core::Block { body, .. } => binding_escapes(db, body, binder, false),
        Core::Break { value } => binding_escapes(db, value, binder, false),
        // Control flow: the binding escapes if it escapes any reachable sub-position.
        Core::If { cond, then_, else_ } => {
            binding_escapes(db, cond, binder, false)
                || binding_escapes(db, then_, binder, false)
                || binding_escapes(db, else_, binder, false)
        }
        Core::Match { scrutinee, arms } => {
            binding_escapes(db, scrutinee, binder, false)
                || arms.iter().any(|a| {
                    a.guard
                        .is_some_and(|g| binding_escapes(db, g, binder, false))
                        || binding_escapes(db, a.body, binder, false)
                })
        }
        Core::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| binding_escapes(db, *v, binder, false))
                || binding_escapes(db, body, binder, false)
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            binding_escapes(db, lhs, binder, false) || binding_escapes(db, rhs, binder, false)
        }
        // `value-eq` BORROWS both operands (it drops only an OWNED temporary, never a `LocalRef`), so a
        // binding used DIRECTLY as an operand does NOT escape — the enclosing `let` still drops it. A
        // binding that flows into a CONSTRUCTED operand (`(= (Wrap x) …)`) DOES escape: it is consumed
        // into that owned temporary, which `value-eq` then drops (so the `let` must not double-drop). The
        // borrow-in-tail recursion (`tail_borrowed: true`) computes exactly this — a direct `LocalRef`
        // borrows, a constructor/call arm resets to consuming — mirroring the `Proj`/`ListLen` arm above.
        Core::ValueEq { lhs, rhs } => {
            binding_escapes(db, lhs, binder, true) || binding_escapes(db, rhs, binder, true)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            binding_escapes(db, operand, binder, false)
        }
        Core::Record { fields } => fields
            .values()
            .any(|&v| binding_escapes(db, v, binder, false)),
        // A sum construction CONSUMES each payload (it becomes part of the heap sum value).
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .any(|&p| binding_escapes(db, p, binder, false)),
        // A sum match: the binding escapes if it escapes the scrutinee or the root continuation (a leaf
        // body, a guarded arm, or a switch's arms — recursed via `cont_binding_escapes`).
        Core::MatchSum { scrutinee, root } => {
            binding_escapes(db, scrutinee, binder, false) || cont_binding_escapes(db, &root, binder)
        }
        // A list match: escapes if the binding escapes the scrutinee (CONSUMING — a rest arm's `vec-split`
        // consumes the list handle) or any arm body.
        Core::MatchList { scrutinee, arms } => {
            binding_escapes(db, scrutinee, binder, false)
                || arms
                    .iter()
                    .any(|a| binding_escapes(db, a.body, binder, false))
        }
        // A sum-payload read BORROWS the scrutinee (`sum-payload` reads without consuming), like a
        // projection operand — so a `LocalRef` reached through it does not escape.
        Core::SumPayload { scrutinee, .. } => binding_escapes(db, scrutinee, binder, true),
        // `expect` reads the scrutinee's payload (a borrow, like `SumPayload`) — a `LocalRef` reached
        // through it does not escape (the payload is unboxed/used in place, not moved out).
        Core::SumExpect { scrutinee, .. } => binding_escapes(db, scrutinee, binder, true),
        // A closure CONSUMES each captured value (it becomes part of the closure cell); a closure
        // application consumes both the closure value and its argument. (This increment's no-capture
        // closure has an empty `captures`, so it references no binding — but the arm is written for the
        // general case so a captured binding is correctly seen as escaping when captures land.)
        Core::Closure { captures, .. } => captures
            .iter()
            .any(|&c| binding_escapes(db, c, binder, false)),
        Core::CallClosure { closure, args } => {
            binding_escapes(db, closure, binder, false)
                || args.iter().any(|&a| binding_escapes(db, a, binder, false))
        }
        // Leaves reference no binding (a `Captured` read reads the env cell, not a body binding). `trap`
        // diverges with no operand, so it holds no binding to escape.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        | Core::Trap
        | Core::Param { .. }
        | Core::Captured { .. }
        | Core::Poison(_) => false,
    }
}

/// Whether `binder` escapes through a sum-match CONTINUATION — a leaf's body, or a nested switch's arms
/// (each recursed). The `Payload`/`Elem` path steps are heap reads that carry no binding, so only the arm
/// continuations matter (mirrors the `MatchSum` arm walk in `binding_escapes`).
fn cont_binding_escapes(db: &mut Db, cont: &crate::core::SumCont, binder: StructId) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => binding_escapes(db, *body, binder, false),
        // A guarded arm's binder can escape through either the guarded body or the fall-through
        // continuation (the guard cond only reads, never escapes a binding).
        crate::core::SumCont::Guarded { body, els, .. } => {
            binding_escapes(db, *body, binder, false) || cont_binding_escapes(db, els, binder)
        }
        // A literal test's binder can escape through either continuation (the `path` walk only reads).
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_binding_escapes(db, then_, binder) || cont_binding_escapes(db, els, binder)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| cont_binding_escapes(db, &a.cont, binder)),
    }
}

/// Perceus RETAIN placement: the set of `Core::LocalRef`/`Core::Param` OCCURRENCES (keyed by their own
/// node id) whose reference is CONSUMED at that occurrence while the binding has a LATER live use on the
/// same control-flow path — so the occurrence must be `dup`'d (rc++) before the consuming op runs, or the
/// op's in-place FBIP reuse (a uniquely-owned `vec-push`/`map-insert`/… mutates its operand) corrupts the
/// value the later use reads.
///
/// The single closing `drop` a `Core::Let` emits (gated by `!binding_escapes`) reclaims a binding whose
/// LAST use borrows; it does NOT account for a binding consumed EARLY and read again. Without a dup there,
/// `(let ((e L)) (+ (List.len (List.push e 9)) (List.len e)))` mutates `e` through the push and the right
/// `List.len e` reads the grown list — a silent wrong value (the same defect for `Map.insert`/`Set.insert`
/// and for a shared PARAMETER across two recursive-call operands). A dup at the consuming occurrence gives
/// the consumer its OWN reference and leaves the binding's reference intact for the later use; the existing
/// escape-gated drop still reclaims the survivor exactly once. The single-use consume (the FBIP fast path,
/// `(List.len (List.push e 9))` with `e` used once) is untouched — no later use, so no dup.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# A value MUST NOT be observably mutated through one reference while it is read through another in a way the executable semantics leaves unspecified.
// The dup/reuse decision this computes is a function of the SOURCE STRUCTURE ALONE — the consuming
// occurrences, their control-flow paths, and the escape/borrow classification — never of any runtime or
// nondeterministic input, so whether an op reuses its operand's storage in place (FBIP) or a dup forces
// fresh storage is deterministic and cannot introduce nondeterminism into observable behavior:
//= spec/capabilities/memory-and-resource-model.md#reuse-is-not-observable
//# A decision to reuse a value's storage or to allocate fresh storage MUST be a deterministic function of the source, so that reuse does not introduce nondeterminism into a program's observable behavior.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# A decision to share a value's storage or to copy it MUST be a deterministic function of the source, so that sharing does not introduce nondeterminism into a program's observable behavior.
fn collect_dup_sites(
    db: &mut Db,
    body: StructId,
    binders: &[StructId],
    sites: &mut HashSet<StructId>,
) {
    for &binder in binders {
        // The body's result position CONSUMES (the value is returned / escapes), so the top-level call is
        // `consuming: true`; nothing is used after the whole body, so `live_after: false`.
        mark_binder_dups(db, body, binder, true, false, sites);
    }
}

/// Collect every HEAP-typed binder whose multi-use inside `id` could need a retain: each `Core::Let`
/// BINDER declared in the subtree, and each PARAMETER referenced by a `Core::Param` occurrence (a scalar
/// binding owns no heap cell, so its multi-use needs no dup — a scalar is re-read from its slot freely).
/// De-dups a parameter referenced more than once. Used to seed `collect_dup_sites` — from `select_function`
/// (which then emits the dups) AND from `collect_used_ops` (which must import `OP_DUP` iff a dup site
/// exists), so the two agree on the retain set. Walks every child (a binding/reference nests anywhere).
fn collect_retain_candidate_binders(db: &mut Db, id: StructId, out: &mut Vec<StructId>) {
    match core_of(db, id) {
        Core::Let { bindings, .. } => {
            for (binder, _) in &bindings {
                if is_heap_type(&type_of(db, *binder)) {
                    out.push(*binder);
                }
            }
        }
        Core::Param { binder } if is_heap_type(&type_of(db, binder)) && !out.contains(&binder) => {
            out.push(binder);
        }
        _ => {}
    }
    for child in core_child_ids(db, id) {
        collect_retain_candidate_binders(db, child, out);
    }
}

/// Every immediate child NODE id of a Core node (all sub-expression occurrences, regardless of position).
/// Used by `collect_heap_let_binders` to find nested `let`s; positions do not matter here. Also drives
/// `layout::collect_closure_call_sigs` (the extra closure-application functype collection) — hence `pub`.
pub fn core_child_ids(db: &mut Db, id: StructId) -> Vec<StructId> {
    let mut cs: Vec<StructId> = Vec::new();
    match core_of(db, id) {
        Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::BytesCompact { operand }
        | Core::MapSize { map: operand }
        | Core::SetLen { set: operand }
        | Core::SetToList { set: operand, .. }
        | Core::MapToList { map: operand, .. }
        | Core::Proj { operand, .. }
        | Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::SumExpect {
            scrutinee: operand, ..
        }
        | Core::BigIntOfI64 { value: operand }
        | Core::RationalOfIntWiden { value: operand }
        | Core::BigIntToI64 { operand }
        | Core::BinIntRead { bytes: operand, .. }
        | Core::BinRestRead { bytes: operand, .. }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand } => cs.push(operand),
        Core::ListAt {
            list: a, index: b, ..
        }
        | Core::BytesAt {
            bytes: a, index: b, ..
        }
        | Core::StrAt {
            string: a,
            index: b,
            ..
        }
        | Core::BigIntBinOp { lhs: a, rhs: b, .. }
        | Core::BigIntCmp { lhs: a, rhs: b, .. }
        | Core::RationalBinOp { lhs: a, rhs: b, .. }
        | Core::RationalCmp { lhs: a, rhs: b, .. }
        | Core::RationalOfInts { num: a, den: b }
        | Core::ValueEq { lhs: a, rhs: b }
        | Core::BytesConcat { lhs: a, rhs: b }
        | Core::ListConcat { lhs: a, rhs: b }
        | Core::ListPush { list: a, elem: b }
        | Core::MapLookup { map: a, key: b, .. }
        | Core::MapRemove { map: a, key: b, .. }
        | Core::SetInsert {
            set: a, elem: b, ..
        }
        | Core::SetRemove {
            set: a, elem: b, ..
        }
        | Core::SetContains {
            set: a, elem: b, ..
        }
        | Core::SetAlgebra { lhs: a, rhs: b, .. }
        | Core::Arith { lhs: a, rhs: b, .. }
        | Core::Compare { lhs: a, rhs: b, .. }
        | Core::FloatCompare { lhs: a, rhs: b, .. }
        | Core::And { lhs: a, rhs: b, .. } => {
            cs.push(a);
            cs.push(b);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            cs.push(bytes);
            cs.push(start);
            cs.push(len);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            cs.push(string);
            cs.push(start);
            cs.push(end);
        }
        Core::ListUpdate { list, index, elem } => {
            cs.push(list);
            cs.push(index);
            cs.push(elem);
        }
        Core::MapInsert { map, key, val, .. } => {
            cs.push(map);
            cs.push(key);
            cs.push(val);
        }
        Core::Tuple { elems }
        | Core::ListNew { elems }
        | Core::BytesOf { elems }
        | Core::SetOf { elems, .. } => cs.extend(elems),
        Core::SumNew { payloads, .. } => cs.extend(payloads),
        Core::Record { fields } => cs.extend(fields.values().copied()),
        Core::MapNew { entries, .. } => {
            for (k, v) in entries {
                cs.push(k);
                cs.push(v);
            }
        }
        Core::BinBuild { segs } => cs.extend(segs.iter().map(|s| s.value)),
        Core::BinBitsBuild { fields } => cs.extend(fields.iter().map(|f| f.value)),
        Core::Call { args, .. } | Core::HostCall { args, .. } => cs.extend(args),
        Core::CallClosure { closure, args } => {
            cs.push(closure);
            cs.extend(args);
        }
        Core::Closure { captures, .. } => cs.extend(captures),
        Core::Seq { stmts, tail } => {
            cs.extend(stmts);
            cs.push(tail);
        }
        // A boundary block's child is its body; a break's child is its value.
        Core::Block { body, .. } => cs.push(body),
        Core::Break { value } => cs.push(value),
        Core::Let { bindings, body } => {
            cs.extend(bindings.iter().map(|(_, v)| *v));
            cs.push(body);
        }
        Core::If { cond, then_, else_ } => {
            cs.push(cond);
            cs.push(then_);
            cs.push(else_);
        }
        Core::Match { scrutinee, arms } => {
            cs.push(scrutinee);
            for a in &arms {
                if let Some(g) = a.guard {
                    cs.push(g);
                }
                cs.push(a.body);
            }
        }
        Core::MatchList { scrutinee, arms } => {
            cs.push(scrutinee);
            for a in &arms {
                if let Some(g) = a.guard {
                    cs.push(g);
                }
                cs.push(a.body);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            cs.push(scrutinee);
            cont_child_ids(&root, &mut cs);
        }
        Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        | Core::Trap
        | Core::Captured { .. }
        | Core::Poison(_) => {}
    }
    cs
}

/// Collect the body/guard/`cond` occurrence ids of a sum-match continuation (the arms `core_child_ids`
/// reaches through a `MatchSum`). The `path` steps carry no occurrence.
fn cont_child_ids(cont: &crate::core::SumCont, cs: &mut Vec<StructId>) {
    match cont {
        crate::core::SumCont::Leaf(body) => cs.push(*body),
        crate::core::SumCont::Guarded { cond, body, els } => {
            cs.push(*cond);
            cs.push(*body);
            cont_child_ids(els, cs);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_child_ids(then_, cs);
            cont_child_ids(els, cs);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms.iter() {
                cont_child_ids(&a.cont, cs);
            }
        }
    }
}

/// Walk `id` in EVALUATION order marking dup sites for `binder` (see [`collect_dup_sites`]), and RETURN
/// whether `binder` occurs anywhere in the subtree. `consuming` is whether the reference reaching `id` is
/// in a consuming position (a constructor element, a call argument, a persistent-collection operand, the
/// escaping result) vs a borrow (a `Proj`/`ListLen`/`Map.size`/… read operand); `live_after` is whether
/// `binder` has a use AFTER `id` completes on the current path. A consuming `LocalRef`/`Param` occurrence
/// of `binder` with `live_after` is a dup site. Sequential operands are processed RIGHT-TO-LEFT, folding
/// each returned "occurred" into `live_after` for its earlier siblings (so an earlier consuming operand
/// sees a later sibling's use); branches (`if`/`match` arms) are independent paths, each processed with
/// the SAME incoming `live_after` (a use in a sibling arm is not "later" on this arm's path). The position
/// (borrow-vs-consume) of each child mirrors [`binding_escapes`] exactly (its `tail_borrowed` = borrow).
/// Does `binder` occur anywhere in the subtree at `id`? A CHEAP occurrence-only walk (a plain membership
/// scan over `core_child_ids` — NO site marking, NO borrow/consume/liveness logic), used by `seq`'s
/// pre-pass to decide whether a sibling references the binder. ⚠ It must NOT call `mark_binder_dups`: that
/// full two-pass walk, invoked from every `seq` level's pre-pass, is EXPONENTIAL on a deeply-nested term
/// (a `push(push(push(xs)))` chain re-walks its inner subtree once per enclosing level, 2^depth). This
/// occurrence scan visits each node of the subtree ONCE per call (the enclosing `seq` levels make it
/// O(depth × subtree) overall — polynomial, not exponential). Memoized via `cache` so a shared subterm
/// (a DAG re-walk) is not re-scanned within one query.
fn binder_occurs(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    cache: &mut HashMap<StructId, bool>,
) -> bool {
    if let Some(&hit) = cache.get(&id) {
        return hit;
    }
    let here = match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| binder_occurs(db, c, binder, cache)),
    };
    cache.insert(id, here);
    here
}

fn mark_binder_dups(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    // Thin entry: every position EXCEPT a `Proj`'s own operand is a "top" position for child-dup marking.
    mark_binder_dups_inner(db, id, binder, consuming, live_after, false, sites)
}

/// Whether `id` is a chain of nested-compound `Core::Proj`s ultimately rooted at `binder` — `binder`
/// itself (`(. binder k)`), or a projection of such a chain (`(. (. binder j) k)`, arbitrarily deep). Each
/// intermediate `Proj` is a BORROW (`arr-get` returns a child handle into the parent), so every child in
/// the chain aliases a cell that lives inside `binder`; a consuming op on the innermost child would
/// FBIP-mutate it while `binder` still owns it. Used by [`mark_binder_dups_inner`] to decide a child-retain
/// (`dup`) site. Only follows `Proj` links (not `SumPayload`/`ListAt`/… — those have their own retain
/// paths); bottoms out at the `LocalRef`/`Param` for `binder`.
fn proj_chain_roots_at_binder(db: &mut Db, id: StructId, binder: StructId) -> bool {
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        Core::Proj { operand, .. } => proj_chain_roots_at_binder(db, operand, binder),
        _ => false,
    }
}

/// Whether `id` is a chain of BORROWING heap-child extractions (`Core::Proj` `arr-get`, `Core::SumPayload`
/// `sum-payload`/`arr-get`, OR `Core::SumExpect` `sum-payload`, in any mix) ultimately rooted at `binder`.
/// Each intermediate step is a BORROW that returns a handle to a cell living INSIDE `binder` (no rc++), so
/// the extracted leaf aliases `binder`'s storage under its single refcount. A consuming op on the leaf would
/// FBIP-mutate it while `binder` still owns it — the [`Core::SumPayload`]/[`Core::SumExpect`]/[`Core::Proj`]
/// child-retain sites in [`mark_binder_dups_inner`] use this to decide a `dup`. The `SumPayload`/`SumExpect`
/// analogue of [`proj_chain_roots_at_binder`]; bottoms out at the `LocalRef`/`Param` for `binder`. Following
/// `SumExpect` too is load-bearing for a CHAINED extraction — `(Option.expect (Option.expect s))` over a
/// threaded `(Option (Option (List …)))`: the outer expect's scrutinee is the inner expect, which must
/// resolve through to the root `s` so the consuming op on the leaf retains.
fn payload_or_proj_chain_roots_at_binder(db: &mut Db, id: StructId, binder: StructId) -> bool {
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        Core::Proj { operand, .. }
        | Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::SumExpect {
            scrutinee: operand, ..
        } => payload_or_proj_chain_roots_at_binder(db, operand, binder),
        _ => false,
    }
}

/// The worker of [`mark_binder_dups`]. `in_proj_operand` is set ONLY when `id` is the aggregate operand of
/// an enclosing `Core::Proj` (an `arr-get`-borrowed intermediate) — used to suppress a redundant child-dup
/// mark on a nested projection in a chain (only the OUTERMOST consuming projection dups its child). Every
/// other recursion resets it to `false` (via the `mark_binder_dups` wrapper the closures call).
fn mark_binder_dups_inner(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    in_proj_operand: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    // A borrowing child position — recurse borrowing, threading `live_after` unchanged.
    let borrow = |db: &mut Db, c: StructId, la: bool, s: &mut HashSet<StructId>| {
        mark_binder_dups(db, c, binder, false, la, s)
    };
    // A consuming child position — recurse consuming.
    let consume = |db: &mut Db, c: StructId, la: bool, s: &mut HashSet<StructId>| {
        mark_binder_dups(db, c, binder, true, la, s)
    };
    // A SEQUENTIAL group of (child, is_borrow) evaluated left-to-right, ALL SIMULTANEOUSLY LIVE before the
    // enclosing op runs (a call pushes every arg onto the stack, then consumes them; a constructor likewise
    // holds all elements). Because they are simultaneously live, a CONSUMING occurrence of `binder` in one
    // child must retain (`dup`) if `binder` also occurs in ANY OTHER child — left OR right — not only a
    // later (right) one: an EARLIER (left) child's `local.get binder` leaves a handle on the stack that a
    // later child's consuming op (e.g. `List.push binder`) would FBIP-mutate in place at rc==1, corrupting
    // the earlier child's already-stacked handle (the self-recursive-call `(f … base … (push base) …)`
    // shape — base threaded unchanged in one arg AND consumed in a sibling). So a child's incoming
    // `live_after` must include whether `binder` occurs in any OTHER child of THIS group. Two-pass: first
    // detect occurrence in every child (a cheap pre-walk that marks no sites — `probe_only`), then process
    // each child with `la || (binder occurs in some other child)`. Still fold right-to-left within the pass
    // (preserves the later-sibling propagation), but seed each child's `la` from the group-wide occurrence
    // EXCLUDING itself. Returns whether `binder` occurred in any of them.
    let seq = |db: &mut Db,
               children: &[(StructId, bool)],
               la_in: bool,
               s: &mut HashSet<StructId>|
     -> bool {
        // Pre-pass: does `binder` occur in each child? Use the CHEAP occurrence scan (`binder_occurs`), NOT
        // `mark_binder_dups` — the latter's full two-pass walk, invoked from every nested `seq`'s pre-pass,
        // is EXPONENTIAL on a deep term (a `push(push(push(xs)))` chain: each level re-walks its inner
        // subtree, 2^depth — a real cdz-compile timeout at depth ~30). The occurrence scan is memoized and
        // marks no sites; the real site marking happens in the main pass below with the correct `live_after`.
        let mut occ_cache: HashMap<StructId, bool> = HashMap::new();
        let mut occurs: Vec<bool> = Vec::with_capacity(children.len());
        for &(c, _) in children.iter() {
            occurs.push(binder_occurs(db, c, binder, &mut occ_cache));
        }
        let any = occurs.iter().any(|&o| o);
        // Main pass, right-to-left so a later sibling's use still flows into an earlier one's `live_after`;
        // additionally seed each child's `la` with "binder occurs in some OTHER child" (the left-sibling
        // case the one-directional fold misses for simultaneously-live operands).
        let mut la = la_in;
        for i in (0..children.len()).rev() {
            let (c, is_borrow) = children[i];
            let other = any && occurs.iter().enumerate().any(|(k, &o)| k != i && o);
            let here = mark_binder_dups(db, c, binder, !is_borrow, la || other, s);
            la = la || here;
        }
        any
    };
    // A BRANCH group: a leading sequential prefix (cond/scrutinee, evaluated before the arms) then N arms,
    // each an independent path with the SAME incoming `live_after`. The prefix's `live_after` includes any
    // arm's use (an arm runs after the prefix). Returns whether `binder` occurred anywhere.
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => {
            if b == binder {
                if consuming && live_after {
                    sites.insert(id);
                }
                return true;
            }
            false
        }
        // Borrowing reads: the operand is borrowed (a scalar element `Proj`, a length, a lookup's map, …).
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::BytesCompact { operand } => {
            borrow(db, operand, live_after, sites)
        }
        Core::Proj { operand, .. } => {
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            // A NESTED-COMPOUND projection (`get_op` None — `arr-get` returns the child HANDLE, a BORROW of
            // the parent) in a CONSUMING position, whose parent `binder` is STILL LIVE afterward, needs a
            // `dup` of the CHILD — not of the binder. The parent is already retained by its own occurrence's
            // dup, but that bumps the AGGREGATE's rc, not the child's: `arr-get` does not rc++ the child, so
            // the child has rc 1 (only the parent's array cell refs it) and the consuming op (e.g. `vec-push`)
            // FBIP-mutates it in place, corrupting a LATER re-projection `(. binder k)` that reads the same
            // child. Dup the child here so the consumer takes the persistent (copy) path and the parent's
            // array stays intact. Marked at THIS Proj node (its own id); the emit `dup`s the arr-get result.
            // The operand must resolve to the (live) `binder` through a CHAIN of nested-compound projections
            // (each an intermediate BORROW — `(. binder k)`, or `(. (. binder j) k)` two deep, …), which all
            // alias the SAME leaf child living inside `binder`'s cells. A projection off a nested/COMPUTED
            // operand (a call result, a fresh constructor) is a different owned handle handled by the
            // `reclaim` path. Scalar elements COPY out, so they never alias — no dup (the FBIP fast path and
            // scalar reads stay untouched). Only the OUTERMOST consuming projection marks (a chain's
            // intermediate projection is reached below as an `arr-get`-borrowed operand — `in_proj_operand`
            // suppresses a redundant child-dup there).
            if consuming
                && !scalar_element
                && !in_proj_operand
                && live_after
                && proj_chain_roots_at_binder(db, operand, binder)
            {
                sites.insert(id);
            }
            // Recurse for BINDER-marking (the aggregate's own dup) as before, flagging that `operand` is a
            // projection operand (borrowed) so a nested `Proj` there does not re-mark a child-dup site.
            mark_binder_dups_inner(
                db,
                operand,
                binder,
                !scalar_element,
                live_after,
                true,
                sites,
            )
        }
        Core::MapSize { map } => borrow(db, map, live_after, sites),
        Core::SetLen { set } => borrow(db, set, live_after, sites),
        Core::SetToList { set, .. } => borrow(db, set, live_after, sites),
        Core::MapToList { map, .. } => borrow(db, map, live_after, sites),
        Core::SumPayload { scrutinee, .. } => {
            // A sum-match payload binder lowers to `Core::SumPayload` at EACH use (lower.rs), and
            // `sum-payload`/`arr-get` BORROW the scrutinee's payload (no rc++). A payload that is a COMPOUND
            // heap child (`get_op` None — the leaf is a handle, not an unboxed scalar) in a CONSUMING position,
            // whose scrutinee `binder` is STILL LIVE afterward, needs a `dup` of the CHILD — exactly like the
            // nested-compound `Proj` case above (and the `RestFrom` step's dup). Without it, a consuming op
            // (`List.push`/`Bytes.concat`/…) FBIP-mutates the child at rc==1 while the still-live scrutinee
            // (matched again, or threaded to a self-call) still references it → the scrutinee reads the grown
            // value (drift). `proj_chain_roots_at_binder`'s SumPayload analogue confirms the scrutinee resolves
            // to the live `binder` through a chain of borrowing payload/proj extractions. A scalar payload
            // COPIES out (no alias) so it never dups — the FBIP fast path and scalar reads stay untouched.
            // Marked at THIS node's id; the emit `dup`s the extracted child.
            let scalar_leaf = matches!(get_op(db, id), Ok(Some(_)));
            // A path ending in `RestFrom` is a list-tail slice (`vec-drop`) — the emit's `RestFrom` step
            // ALREADY dups the scrutinee before consuming (see the emit), so this node must NOT also mark a
            // child-dup (a double-dup + a slot conflict). Only a `Payload`/`Elem` COMPOUND leaf extraction
            // (a borrowing `sum-payload`/`arr-get` returning a handle) needs this retain.
            let ends_in_rest = matches!(
                core_of(db, id),
                Core::SumPayload { path, .. }
                    if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
            );
            if consuming
                && !scalar_leaf
                && !ends_in_rest
                && !in_proj_operand
                && live_after
                && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            {
                sites.insert(id);
            }
            // Recurse for BINDER-marking on the scrutinee (borrowed), flagging it as a projection operand so a
            // nested payload/proj there does not re-mark a redundant child-dup (only the outermost consuming
            // extraction dups).
            mark_binder_dups_inner(db, scrutinee, binder, false, live_after, true, sites)
        }
        Core::SumExpect { scrutinee, .. } => {
            // The `SumExpect` twin of the `SumPayload` child-retain above: `Option.expect`/`Result.expect`
            // reads `sum-payload` (a BORROW, no rc++) of the present variant. A COMPOUND payload
            // (`get_op` None) consumed while the scrutinee `binder` is STILL LIVE (a self-recursive call
            // threading the Option, or a re-expect in the same expression) must `dup` the extracted child —
            // else the consuming op FBIP-mutates the shared payload at rc==1 and the still-live scrutinee
            // reads the grown value (drift). Same shape as `SumPayload`; no `RestFrom` case (a `SumExpect`
            // reads exactly one payload). A scalar payload COPIES out → never a site (FBIP fast path intact).
            let scalar_leaf = matches!(get_op(db, id), Ok(Some(_)));
            if consuming
                && !scalar_leaf
                && !in_proj_operand
                && live_after
                && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            {
                sites.insert(id);
            }
            mark_binder_dups_inner(db, scrutinee, binder, false, live_after, true, sites)
        }
        // `List.at`/`Bytes.at` BORROW the sequence; the index is a scalar (consume position, no heap).
        Core::ListAt { list, index, .. } => {
            seq(db, &[(list, true), (index, false)], live_after, sites)
        }
        Core::BytesAt { bytes, index, .. } => {
            seq(db, &[(bytes, true), (index, false)], live_after, sites)
        }
        // `String.at` CONSUMES its string (the Some branch slices out of it); the index is scalar.
        Core::StrAt { string, index, .. } => {
            seq(db, &[(string, false), (index, false)], live_after, sites)
        }
        // `String.slice` likewise CONSUMES its string (the Some branch `dup`s + slices out of it); the
        // start/end bounds are scalars.
        Core::StrSlice {
            string, start, end, ..
        } => seq(
            db,
            &[(string, false), (start, false), (end, false)],
            live_after,
            sites,
        ),
        // `String.from-bytes` CONSUMES its bytes operand (`str-from-bytes` transfers it out as the String).
        Core::StrFromBytes { bytes, .. } => consume(db, bytes, live_after, sites),
        // `String.to-bytes` CONSUMES its string operand (`bytes-compact` transfers it out as the Bytes).
        Core::StrToBytes { string } => consume(db, string, live_after, sites),
        // BigInt/Rational arith/cmp BORROW their handle operands (`tail_borrowed: true` in `binding_escapes`).
        Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            seq(db, &[(lhs, true), (rhs, true)], live_after, sites)
        }
        Core::BigIntOfI64 { value } | Core::RationalOfIntWiden { value } => {
            consume(db, value, live_after, sites)
        }
        Core::BigIntToI64 { operand } => borrow(db, operand, live_after, sites),
        Core::RationalOfInts { num, den } => {
            seq(db, &[(num, false), (den, false)], live_after, sites)
        }
        // `value-eq` BORROWS both operands.
        Core::ValueEq { lhs, rhs } => seq(db, &[(lhs, true), (rhs, true)], live_after, sites),
        // Consuming constructors / ops: every operand is consumed into the result.
        Core::BytesConcat { lhs, rhs } | Core::ListConcat { lhs, rhs } => {
            seq(db, &[(lhs, false), (rhs, false)], live_after, sites)
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => seq(
            db,
            &[(bytes, false), (start, false), (len, false)],
            live_after,
            sites,
        ),
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            let cs: Vec<(StructId, bool)> = elems.iter().map(|&e| (e, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::SumNew { payloads, .. } => {
            let cs: Vec<(StructId, bool)> = payloads.iter().map(|&p| (p, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::Record { fields } => {
            let cs: Vec<(StructId, bool)> = fields.values().map(|&v| (v, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::BinBuild { segs } => {
            let cs: Vec<(StructId, bool)> = segs.iter().map(|s| (s.value, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::BinBitsBuild { fields } => {
            let cs: Vec<(StructId, bool)> = fields.iter().map(|f| (f.value, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::BinIntRead { bytes, .. } | Core::BinRestRead { bytes, .. } => {
            consume(db, bytes, live_after, sites)
        }
        Core::ListPush { list, elem } => {
            seq(db, &[(list, false), (elem, false)], live_after, sites)
        }
        Core::ListUpdate { list, index, elem } => seq(
            db,
            &[(list, false), (index, false), (elem, false)],
            live_after,
            sites,
        ),
        Core::MapNew { entries, .. } => {
            let mut cs: Vec<(StructId, bool)> = Vec::with_capacity(entries.len() * 2);
            for &(k, v) in entries.iter() {
                cs.push((k, false));
                cs.push((v, false));
            }
            seq(db, &cs, live_after, sites)
        }
        Core::MapInsert { map, key, val, .. } => seq(
            db,
            &[(map, false), (key, false), (val, false)],
            live_after,
            sites,
        ),
        // `Map.lookup` BORROWS the map; the key is consumed into an owned temporary.
        Core::MapLookup { map, key, .. } => {
            seq(db, &[(map, true), (key, false)], live_after, sites)
        }
        Core::MapRemove { map, key, .. } => {
            seq(db, &[(map, false), (key, false)], live_after, sites)
        }
        Core::SetOf { elems, .. } => {
            let cs: Vec<(StructId, bool)> = elems.iter().map(|&e| (e, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            seq(db, &[(set, false), (elem, false)], live_after, sites)
        }
        // `Set.contains` BORROWS the set; the element is consumed into an owned temporary.
        Core::SetContains { set, elem, .. } => {
            seq(db, &[(set, true), (elem, false)], live_after, sites)
        }
        Core::SetAlgebra { lhs, rhs, .. } => {
            seq(db, &[(lhs, false), (rhs, false)], live_after, sites)
        }
        // Arithmetic / logical: both operands consumed positions (scalars anyway; a heap binding can only
        // reach here through a producer, which resets to consuming — matches `binding_escapes`'s `false`).
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => seq(db, &[(lhs, false), (rhs, false)], live_after, sites),
        Core::Convert { operand, .. } | Core::Not { operand } => {
            consume(db, operand, live_after, sites)
        }
        // A runtime call / host call CONSUMES each argument (callee-owns-args). Args evaluate left-to-right.
        Core::Call { args, .. } | Core::HostCall { args, .. } => {
            let cs: Vec<(StructId, bool)> = args.iter().map(|&a| (a, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::CallClosure { closure, args } => {
            let mut cs: Vec<(StructId, bool)> = Vec::with_capacity(args.len() + 1);
            cs.push((closure, false));
            cs.extend(args.iter().map(|&a| (a, false)));
            seq(db, &cs, live_after, sites)
        }
        Core::Closure { captures, .. } => {
            let cs: Vec<(StructId, bool)> = captures.iter().map(|&c| (c, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        // A sequencing block: statements then the tail, all sequential (each statement's value is dropped,
        // so a bare consuming statement is a consume position).
        Core::Seq { stmts, tail } => {
            let mut cs: Vec<(StructId, bool)> = stmts.iter().map(|&s| (s, false)).collect();
            cs.push((tail, false));
            seq(db, &cs, live_after, sites)
        }
        // A boundary block's body / a break's value is a single sequential value position (the body's
        // value is the block's; the break value flows out as the block's value on the abortive path).
        Core::Block { body, .. } => {
            mark_binder_dups(db, body, binder, consuming, live_after, sites)
        }
        Core::Break { value } => mark_binder_dups(db, value, binder, consuming, live_after, sites),
        // A `let`: the initializers are sequential-before the body (a `let*` later init may name an earlier
        // one). The body's position is the enclosing `consuming` (the let's value flows to where the let is
        // used). NOTE the INNER binder shadows nothing here — we track ONE outer `binder`; an inner binding
        // with the same id is impossible (each binder is a distinct node).
        Core::Let { bindings, body } => {
            let body_occurs = mark_binder_dups(db, body, binder, consuming, live_after, sites);
            // Each initializer is evaluated before the body; the body (and later inits) may use `binder`.
            let mut la = live_after || body_occurs;
            let mut any = body_occurs;
            for (_, v) in bindings.iter().rev() {
                let here = mark_binder_dups(db, *v, binder, false, la, sites);
                la = la || here;
                any = any || here;
            }
            any
        }
        // `if`: the condition is evaluated first (borrow — a bool test never consumes a heap ref into the
        // result); the two branches are INDEPENDENT paths, each with the incoming `live_after`.
        Core::If { cond, then_, else_ } => {
            let then_occurs = mark_binder_dups(db, then_, binder, consuming, live_after, sites);
            let else_occurs = mark_binder_dups(db, else_, binder, consuming, live_after, sites);
            let cond_la = live_after || then_occurs || else_occurs;
            let cond_occurs = mark_binder_dups(db, cond, binder, false, cond_la, sites);
            cond_occurs || then_occurs || else_occurs
        }
        // A scalar `match`: the scrutinee is evaluated first; each arm (guard + body) is an independent
        // path. A guard is evaluated before its body (both on the arm's path), so within an arm the guard's
        // `live_after` includes the body's use.
        Core::Match { scrutinee, arms } => {
            let mut arms_occur = false;
            for a in arms.iter() {
                let body_occurs =
                    mark_binder_dups(db, a.body, binder, consuming, live_after, sites);
                if let Some(g) = a.guard {
                    let g_occurs =
                        mark_binder_dups(db, g, binder, false, live_after || body_occurs, sites);
                    arms_occur = arms_occur || g_occurs;
                }
                arms_occur = arms_occur || body_occurs;
            }
            let scrutinee_occurs = mark_binder_dups(
                db,
                scrutinee,
                binder,
                false,
                live_after || arms_occur,
                sites,
            );
            scrutinee_occurs || arms_occur
        }
        // A LIST match: the scrutinee is consumed (a rest arm's `vec-split` consumes the handle); each arm
        // body/guard is an independent path.
        Core::MatchList { scrutinee, arms } => {
            let mut arms_occur = false;
            for a in arms.iter() {
                let body_occurs =
                    mark_binder_dups(db, a.body, binder, consuming, live_after, sites);
                if let Some(g) = a.guard {
                    let g_occurs =
                        mark_binder_dups(db, g, binder, false, live_after || body_occurs, sites);
                    arms_occur = arms_occur || g_occurs;
                }
                arms_occur = arms_occur || body_occurs;
            }
            let scrutinee_occurs =
                mark_binder_dups(db, scrutinee, binder, true, live_after || arms_occur, sites);
            scrutinee_occurs || arms_occur
        }
        // A SUM match: the scrutinee is evaluated first; the continuation's arms are independent paths.
        Core::MatchSum { scrutinee, root } => {
            let cont_occurs = mark_cont_dups(db, &root, binder, consuming, live_after, sites);
            let scrutinee_occurs = mark_binder_dups(
                db,
                scrutinee,
                binder,
                false,
                live_after || cont_occurs,
                sites,
            );
            scrutinee_occurs || cont_occurs
        }
        // Leaves / non-binding nodes.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        | Core::Trap
        | Core::Captured { .. }
        | Core::Poison(_) => false,
    }
}

/// Mark dup sites through a sum-match CONTINUATION (mirrors `cont_binding_escapes`): every leaf body /
/// guarded arm / literal-test / nested switch is an independent path, each processed with the incoming
/// `consuming`/`live_after`. The `path` steps (`Payload`/`Elem`) are heap reads carrying no binding.
/// Returns whether `binder` occurs anywhere in the continuation.
fn mark_cont_dups(
    db: &mut Db,
    cont: &crate::core::SumCont,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            mark_binder_dups(db, *body, binder, consuming, live_after, sites)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            let body_occurs = mark_binder_dups(db, *body, binder, consuming, live_after, sites);
            let els_occurs = mark_cont_dups(db, els, binder, consuming, live_after, sites);
            // The guard is evaluated before the guarded body (same path); the fall-through `els` is a
            // separate path. The guard only reads (never consumes into the result).
            let cond_occurs =
                mark_binder_dups(db, *cond, binder, false, live_after || body_occurs, sites);
            body_occurs || els_occurs || cond_occurs
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            let then_occurs = mark_cont_dups(db, then_, binder, consuming, live_after, sites);
            let els_occurs = mark_cont_dups(db, els, binder, consuming, live_after, sites);
            then_occurs || els_occurs
        }
        crate::core::SumCont::Switch { arms, .. } => {
            let mut occurs = false;
            for a in arms.iter() {
                occurs =
                    mark_cont_dups(db, &a.cont, binder, consuming, live_after, sites) || occurs;
            }
            occurs
        }
    }
}

/// The runtime op that BOXES the node at `id` (a tuple/record element) into a u32 heap handle, by its
/// solved type: an integer → `box-int` (an i64 payload), a boolean → `box-bool`. A COMPOUND element (a
/// nested tuple/record) is ALREADY a u32 handle — it is `arr-set` into the parent array as-is, with no
/// box op — so this returns `Ok(None)` for a compound (the caller skips the box). A type with no heap
/// representation at all (a function/type-value) DECLINES. Reads the solved type.
fn box_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    let ty = type_of(db, id);
    box_op_ty(db, &ty)
}

/// The box op for a collection ELEMENT/KEY/VALUE node, given the collection's DECLARED slot type. Prefers
/// the collection's declared type (which grounds a bare-literal element to the collection's width), but
/// FALLS BACK to the element NODE's own concrete type when the declared type is an unresolved `Var`/`Any`.
/// A collection built EMPTY — `(Set.of (list))`, `(Map.empty)` — leaves its element/key type an
/// unconstrained var: inference never unifies a later `Set.insert`/`Map.insert`'s element type back onto
/// the empty collection's element var, so the declared type stays `Var`. `box_op_ty` defaults a `Var` to
/// `box-int` (the uniform heap cell — correct for a genuinely-dead phantom position), which WRONGLY boxes
/// a live HEAP-HANDLE element (a String, a nested compound) as an integer → an invalid module. The
/// inserted element is a live value with a SOLVED type, so box it by that — exactly as `Core::Tuple` boxes
/// each element by its own node type via `box_op`. (A resolved declared type is authoritative and used
/// as-is; only an unresolved declared type defers to the element node.)
fn box_op_for(db: &mut Db, node: StructId, declared: &Ty) -> Result<Option<&'static str>, Reject> {
    if matches!(declared, Ty::Var(_) | Ty::Any) {
        box_op(db, node)
    } else {
        box_op_ty(db, declared)
    }
}

/// The box op for a solved TYPE directly (not a node) — used where a map's key/value type is known but
/// no representative node is at hand (a `Map.lookup` value unbox reads `val_ty`). Mirrors [`box_op`].
fn box_op_ty(db: &Db, ty: &Ty) -> Result<Option<&'static str>, Reject> {
    // An ENUM-DISCRIMINANT sum is a bare i32 discriminant, NOT a heap handle, so as a nested element it
    // boxes exactly like an integer (`box-int`, with the i32→i64 extend the caller applies) — checked
    // before the `Ty::Sum` "already a handle" arm below.
    if ty_is_enum_disc(db, ty) {
        return Ok(Some(OP_BOX_INT));
    }
    match ty {
        Ty::Int(_) => Ok(Some(OP_BOX_INT)),
        Ty::Bool => Ok(Some(OP_BOX_BOOL)),
        // A FLOAT boxes into its width's dedicated leaf: Float64 → `box-float` (an `f64` slot, `valtype_of`
        // → F64, IS `box-float`'s arg), Float32 → `box-float32` (an `f32` slot IS `box-float32`'s arg). So
        // NEITHER needs coercion — the shared `emit_box_i32_to_i64_extend` before the box op is a no-op for
        // a non-int/non-enum-disc value. A Float32 gets its OWN 4-byte leaf (not a promoted f64) so its
        // canonical byte form + value-encode render are the f32's, not an f64's (`0.1f32` → `0.1`).
        Ty::Float(ft) if ft.ground_width() == 64 => Ok(Some(OP_BOX_FLOAT)),
        Ty::Float(ft) if ft.ground_width() == 32 => Ok(Some(OP_BOX_FLOAT32)),
        // A nested compound — a tuple/record, a SUM (its `sum-new` handle), a LIST (`vec-*` handle), a
        // MAP (its CHAMP `map-*` handle), a BYTES sequence (`bytes-*` handle), or a STRING (a UTF-8
        // byte-leaf handle) — is already a u32 handle, so it is `arr-set` into the parent array (or used
        // as a sum payload / a map key or value) as-is, no box op. A CLOSURE (`Ty::Fn`) is likewise a u32
        // cell handle (`valtype_of(Ty::Fn) = I32`) — a closure captured BY another closure is stored
        // as-is. A `Ty::Map` here is what lets a MAP be a KEY or VALUE of another map — its handle is
        // CANONICAL by construction (order-independent CHAMP), so the outer map's `champ_hash`/`champ_eq`
        // walk over the nested map key is exact, exactly as a nested tuple/record key already is.
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::Bytes
        | Ty::String
        // A BigInt is already a heap handle (its sign-magnitude leaf — `bigint-of-i64`/arithmetic return
        // handles), so as a nested element it is stored as-is, exactly like a String/List/Map handle.
        | Ty::BigInt
        // A Rational is likewise already a heap handle (a normalized 2-BigInt-handle node — `rational-of`/
        // the arithmetic return handles), so as a map key / set element / tuple element it is stored
        // as-is; its `champ_eq`/`champ_hash` descend the two child leaves (value equality by component).
        | Ty::Rational
        | Ty::Fn(_, _) => Ok(None),
        // A quantity erases to its inner numeric type (`lower` strips the `Qty`), so box it by that inner
        // type — a `(Qty Int64 u)` element boxes exactly as an `Int64` element.
        Ty::Qty { inner, .. } => box_op_ty(db, inner),
        // A NOMINAL newtype erases to its inner (the tag adds nothing to the runtime representation) — box
        // by the inner type. A scalar-inner newtype boxes as its scalar; a recursive newtype's inner is a
        // finite `Ty::Sum`-back-edged compound (a handle), boxed as-is. This is what lets a newtype value
        // (incl. a recursive one's self-referential cell) sit in a tuple/sum/collection slot.
        Ty::Nominal { inner, .. } => box_op_ty(db, inner),
        // A FREE var / `Any` element — inference never determined this position's type. That happens ONLY
        // when NO value ever flows through it: a DEAD match arm (`(match (Some (Ok x)) … ((Err e) e) …)`
        // where no `Err` is ever built leaves the Err type a free var, and its `e` read is unreachable), or
        // a phantom parameter. Ground it to the uniform i64 heap cell (`box-int`) — the SAME default a
        // deferred integer width takes: since no value observably flows, any consistent representation is
        // correct, and the unreachable read/store just needs SOME valid op to emit. This lets a program
        // that MATCHES an un-built variant (a total match on a `Result` only ever `Ok`) compile, rather
        // than declining the dead arm's phantom-typed payload read. (A LIVE value never has a free-var
        // type here — inference would have solved it — so this cannot mask a real unresolved-type bug.)
        Ty::Var(_) | Ty::Any => Ok(Some(OP_BOX_INT)),
        Ty::Unit => Ok(None),
        other => Err(Reject::decline(format!(
            "a tuple element of type {} needs the value heap (not yet built)",
            other.render_name()
        ))),
    }
}

/// The runtime op that UNBOXES a u32 heap handle back to the value the node at `id` projects — the dual
/// of [`box_op`], keyed by this projection's solved type: an integer → `get-int`, a boolean →
/// `get-bool`. A COMPOUND projection (the element is itself a nested tuple/record) needs NO unbox — the
/// handle `arr-get` yields IS the nested compound — so this returns `Ok(None)` (the caller uses the
/// handle as-is). A projection of a type with no heap representation declines.
fn get_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    let ty = type_of(db, id);
    get_op_ty(db, &ty)
}

/// The unbox op for a solved TYPE directly (not a node) — the dual of [`box_op_ty`], used where a value
/// type is known but no node is at hand (a `Map.lookup` reads its `Some` payload back by `val_ty`).
fn get_op_ty(db: &Db, ty: &Ty) -> Result<Option<&'static str>, Reject> {
    // An ENUM-DISCRIMINANT sum was boxed as an integer (see `box_op_ty`), so it is read back with
    // `get-int` (and the caller narrows i64→i32) — NOT used as a handle. Checked before the `Ty::Sum` arm.
    if ty_is_enum_disc(db, ty) {
        return Ok(Some(OP_GET_INT));
    }
    match ty {
        Ty::Int(_) => Ok(Some(OP_GET_INT)),
        Ty::Bool => Ok(Some(OP_GET_BOOL)),
        // A FLOAT reads back with its width's op: Float64 → `get-float` (an `f64`), Float32 → `get-float32`
        // (an `f32`, the value's machine slot) — the duals of `box_op_ty`'s Float arms, both coercion-free.
        Ty::Float(ft) if ft.ground_width() == 64 => Ok(Some(OP_GET_FLOAT)),
        Ty::Float(ft) if ft.ground_width() == 32 => Ok(Some(OP_GET_FLOAT32)),
        // A nested compound / SUM / LIST / MAP / BYTES / STRING handle `arr-get` (or `sum-payload`) yields
        // is used as-is — no unbox. A CLOSURE (`Ty::Fn`) is a u32 cell handle too — a captured fn-typed
        // value (`Core::Captured` of a closure) reads back the handle directly, ready for a `call_indirect`.
        // A `Ty::Map` here is the dual of `box_op_ty`'s Map arm: a map read back from a heap slot (a map
        // stored as another map's key/value, or a tuple/sum element) is used as its handle directly.
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::Bytes
        | Ty::String
        // A BigInt handle read back from a heap slot is used as-is (dual of `box_op_ty`'s BigInt arm).
        | Ty::BigInt
        // A Rational handle read back from a heap slot is used as-is (dual of `box_op_ty`'s Rational arm).
        | Ty::Rational
        | Ty::Fn(_, _) => Ok(None),
        // A quantity erases to its inner numeric type — unbox by that inner type (the dual of `box_op_ty`).
        Ty::Qty { inner, .. } => get_op_ty(db, inner),
        // A NOMINAL newtype erases to its inner — unbox by the inner type (the dual of `box_op_ty`'s
        // Nominal arm), so a newtype value read back from a heap slot uses the right unbox.
        Ty::Nominal { inner, .. } => get_op_ty(db, inner),
        // A FREE var / `Any` projection — a DEAD arm reading an un-built variant's phantom payload (the dual
        // of `box_op_ty`'s free-var arm). Ground to `get-int` (the i64 cell): the read is unreachable (no
        // value of this type ever flows), so the op only needs to be VALID, not value-correct. This lets a
        // total match on a partly-un-built sum (`(Result C ?)` only ever `Ok`) compile its dead `Err` arm.
        Ty::Var(_) | Ty::Any => Ok(Some(OP_GET_INT)),
        Ty::Unit => Ok(None),
        other => Err(Reject::decline(format!(
            "projecting a tuple element of type {} needs the value heap (not yet built)",
            other.render_name()
        ))),
    }
}

/// Push the inline-unit sentinel a UNIT value occupies when it crosses into a value-heap slot. A `Unit`
/// has NO machine slot (`valtype_of(Unit) = None`), so the value itself (`Core::Unit`) emits NOTHING — but
/// a heap slot (a tuple/record element, a sum payload, a collection key/value/element, a closure capture)
/// still needs SOME handle to keep its positional index aligned, so it holds `IMM_UNIT`: a low-bit-tagged
/// immediate (RC-noop, no runtime import), the SAME sentinel a nullary/Unit-payload sum uses (see the
/// `Core::SumNew` single-payload arm). Without this a Unit element pushed nothing and the following
/// `arr-set`/`sum-new` underflowed the stack → an INVALID module.
fn emit_unit_slot(out: &mut Emit) {
    out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32));
}

/// Emit the heap-STORE tail AFTER a value node has been emitted (its machine slot is on the stack, or —
/// for a `Unit` — NOTHING was pushed). Leaves exactly ONE handle for the following `arr-set`/`sum-new`/
/// insert: a SCALAR boxes (extending a narrow int i32→i64 first, as `box-int` takes an i64 cell); a
/// COMPOUND is already a u32 handle, left as-is; a UNIT substitutes the inline-unit sentinel (the value
/// pushed nothing). `boxed` is the caller's `box_op`/`box_op_for` classification — passed, not recomputed,
/// because a collection slot's box type may come from a DECLARED type, not the node — and both a compound
/// handle AND a Unit map to `None`, so the Unit case is distinguished by the node's own solved type
/// (stripping a nominal, exactly as `box_op_ty`'s recursion does). Any rope compaction the caller applies
/// stays AFTER this call (compaction is String/Bytes-only, never a Unit).
fn emit_heap_store_tail(db: &mut Db, node: StructId, boxed: Option<&'static str>, out: &mut Emit) {
    match boxed {
        Some(op) => {
            emit_box_i32_to_i64_extend(db, node, out);
            out.push(Lir::CallImport(op));
        }
        None if matches!(type_of(db, node).strip_nominal(), Ty::Unit) => emit_unit_slot(out),
        None => {}
    }
}

/// Emit the heap-READ tail AFTER `arr-get`/`sum-payload`/`vec-get` has pushed the slot's handle — the dual
/// of [`emit_heap_store_tail`]. A SCALAR unboxes (`get-int`/`get-bool`/…, then a narrow int narrows
/// i64→i32); a COMPOUND handle is used as-is; a UNIT DROPS the sentinel handle the read yielded, because a
/// projected `Unit` leaves NO machine value (`valtype_of(Unit) = None`). Without the drop the `IMM_UNIT`
/// handle stayed on the stack where nothing was expected → a stack-type mismatch (an INVALID module).
/// `unboxed` is the caller's `get_op`/`get_op_ty` result (`None` for BOTH a compound and a Unit; the Unit
/// case is distinguished by `id`'s solved type).
fn emit_heap_read_tail(db: &mut Db, id: StructId, unboxed: Option<&'static str>, out: &mut Emit) {
    match unboxed {
        Some(op) => {
            out.push(Lir::CallImport(op));
            if needs_get_int_narrow(db, id) {
                out.push(Lir::I32WrapI64);
            }
        }
        None if matches!(type_of(db, id).strip_nominal(), Ty::Unit) => out.push(Lir::Drop),
        None => {}
    }
}

// The heap stores an integer as an i64 cell (`box-int` takes `s64`, `get-int` returns `s64`), but a
// NARROW-width integer (`Int8`/`Int16`/`Int32`/`UInt8`…) lives in an i32 machine slot. So a narrow
// element must be EXTENDED i32→i64 before `box-int`, and a narrow projection NARROWED i64→i32 after
// `get-int` — otherwise the emitted `box-int`/op has a mismatched operand slot (i32 vs the i64 the heap
// ABI expects, or i64 vs the i32 a narrow op expects) and wasm rejects the function. A full-width
// integer (Int64/UInt64) already occupies the i64 slot; a boolean is an i32 the heap boxes as-is. This
// is the heap-boundary analogue of `emit_wrap`'s slot move (the heap holds a width-erased i64 cell; the
// compiler normalizes at the box/unbox edge).

/// True iff the node at `id` is a NARROW integer (an i32-slot integer: width ≤ 32). Such a value must
/// cross the i64 heap-cell boundary with an explicit slot conversion.
fn is_narrow_int(db: &mut Db, id: StructId) -> Option<Machine> {
    match type_of(db, id) {
        Ty::Int(it) => {
            let m = Machine::of(it);
            m.slot32.then_some(m)
        }
        _ => None,
    }
}

/// After `get-int`ing a value BACK from an i64 heap cell, does it need narrowing i64→i32 to its machine
/// slot? True for a NARROW int (its slot is i32) OR an ENUM-DISC value (a bare i32 discriminant boxed as
/// an i64 cell — the exact dual of `emit_box_i32_to_i64_extend`'s extend-on-store). Without the enum-disc
/// case an enum reached through a heap slot (a tuple/record element, a sum payload, an expect payload, a
/// closure capture) reads back as an i64 where its i32 discriminant slot is expected → wasm rejects the
/// module (`type mismatch: expected i32, found i64`). `get_op_ty`'s enum-disc arm returns `get-int` on the
/// promise that "the caller narrows i64→i32" — this is that narrow.
fn needs_get_int_narrow(db: &mut Db, id: StructId) -> bool {
    is_narrow_int(db, id).is_some() || node_is_enum_disc(db, id)
}

/// Before `box-int`ing a value into a heap cell, widen it from an i32 slot to the i64 `box-int` expects.
/// Fires for a NARROW int (extended by ITS sign) OR an ENUM-DISC value (a bare i32 discriminant, extended
/// UNSIGNED — a discriminant is a small non-negative index). A full-width i64 int, or a non-scalar, needs
/// no extend. Shared by every `box-int` payload/element site (a sum payload, a tuple/record element, a
/// closure capture, a map value) — an enum-disc payload (`(Some (Green))`, a `Color` element in a tuple)
/// must widen exactly like a narrow int, or an i32 reaches the i64 `box-int` and wasm rejects the module.
fn emit_box_i32_to_i64_extend(db: &mut Db, id: StructId, out: &mut Emit) {
    if let Some(m) = is_narrow_int(db, id) {
        out.push(if m.signed {
            Lir::I64ExtendI32S
        } else {
            Lir::I64ExtendI32U
        });
    } else if node_is_enum_disc(db, id) {
        // An enum-disc value is a non-negative i32 discriminant → zero-extend to the i64 cell.
        out.push(Lir::I64ExtendI32U);
    }
}

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). A body may take parameters and declare locals (the scratch a guarded operation
/// reserves, and any persistent slot a kept `let` binding holds).
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
    /// The AST occurrence this function's body was selected from — the source-attribution anchor for
    /// debug info (`DESIGN-debug-info-rcdzc.md` §2.1b). Carried so `serialize` can pair each function's
    /// emitted code byte range with the source `StructId` (→ span, via the `spans` sidecar), which is
    /// what a DWARF `DW_TAG_subprogram`'s `low_pc`/`high_pc` + line-program rows need. This is
    /// FUNCTION-granularity attribution; per-statement (per-`Lir`-run) is a later refinement that needs
    /// threading the current node through the emit family. `None` for a synthesized function (an escape
    /// walker) with no single source body.
    pub src_body: Option<StructId>,
    /// Named SCALAR locals for debug-info variable inspection (D3, `DESIGN-debug-info-rcdzc.md` §2.4) —
    /// each a `(wasm local slot, source name, solved scalar type)`. Populated with the function's scalar
    /// PARAMETERS (slots `0..n`, the common `print n` target). A `DW_TAG_variable` DIE emits from each so
    /// a debugger can read the value. A compound (heap-handle) param is omitted — DWARF cannot walk the
    /// tagless heap (§3), so only scalars appear. Function-scoped `let`-bindings also land here; MATCH
    /// binders — live only within one match expression, in a REUSED scratch slot — are NOT flat locals
    /// but scoped `scopes` below. Empty unless debug is requested.
    pub locals: Vec<LocalVar>,
    /// Scalar MATCH-BINDER lexical scopes (D3, `DESIGN-debug-info-rcdzc.md` §2.4): each a `[Lir start,
    /// end)` range + the binder locals visible there. Distinct from `locals` because a match binder is
    /// live only within its match (its slot is later reused), so the backend fences it in a
    /// `DW_TAG_lexical_block` with a PC range rather than a function-scoped `DW_TAG_variable`. Ranges are
    /// post-peephole Lir indices; `dwarf_funcs_for` maps them to code offsets. Empty unless debug.
    pub scopes: Vec<MatchScope>,
    /// Per-CONSTRUCT source line markers (`DESIGN-debug-line-granularity-rcdzc.md`): `(Lir index,
    /// source occurrence)` at each point a distinct source construct's evaluation begins, in emission
    /// order, remapped through the peephole pass. The backend turns each into a `.debug_line` row (one
    /// per source LINE the code visits, after dedup/collapse), so a debugger steps line-by-line instead
    /// of resting on the function's opening line. Empty for a single-construct body → the line program
    /// falls back to one function-entry row (the function-granularity behavior, preserved). Always
    /// collected (cheap); only READ under a debug target.
    pub stmt_lines: Vec<(u32, StructId)>,
}

/// A named scalar local for debug info (D3): its wasm local slot, source name, solved scalar type, and
/// whether it is a function PARAMETER (vs a `let`-binding local) — so the backend picks
/// `DW_TAG_formal_parameter` vs `DW_TAG_variable`.
#[derive(Clone, Debug)]
pub struct LocalVar {
    pub slot: u32,
    pub name: String,
    pub ty: Ty,
    pub is_param: bool,
}

/// An inert STUB function with the given parameter types and result type `ret` — its body is a single
/// zero of the result's machine type. Used for an UNREACHED lambda-lifted closure (a dead lift the
/// emitted code folds away and never calls): the stub keeps the function-index + type section consistent
/// with the funcref table's slot numbering without carrying the dead lambda's (possibly ill-formed) body.
/// It is never invoked (its table entry is omitted), so returning a zero is safe. `params` is the
/// `(binder, type)` list the real selection would use; only the value types matter here.
pub fn stub_function(params: &[(StructId, Ty)], ret: &Ty) -> SelectedFunc {
    let param_vts: Vec<ValType> = params.iter().filter_map(|(_, t)| valtype_of(t)).collect();
    // A zero of the result's machine slot. A result with no machine rep (should not happen for a lifted
    // lambda, whose result type was checked at lift time) defaults to an i32 zero — harmless in a
    // never-called stub.
    let zero = match valtype_of(ret) {
        Some(ValType::I64) => Lir::ConstI64(0),
        Some(ValType::F64) => Lir::F64ConstBits(0),
        _ => Lir::ConstI32(0),
    };
    SelectedFunc {
        params: param_vts,
        ret: ret.clone(),
        code: vec![zero],
        declared: Vec::new(),
        src_body: None,
        locals: Vec::new(),
        scopes: Vec::new(),
        stmt_lines: Vec::new(),
    }
}

/// Select one NULLARY definition body (rooted at AST occurrence `body`) into its flat instruction
/// sequence. The return type is the body's solved type. Reads the core + type columns lazily.
pub fn select_body(db: &mut Db, body: StructId, layout: &Layout) -> Result<SelectedFunc, Reject> {
    select_function(db, body, &[], layout)
}

/// Collect the value-heap runtime OP NAMES the body (rooted at core node `id`) will emit, into `out`.
/// This mirrors `emit`'s op choices EXACTLY (the same `box_op`/`get_op` per element/projection type), so
/// the program's per-program import set is precisely the ops it calls — no more, no less. Run over every
/// reachable body BEFORE selection, so the used-set (hence `layout.import_base` and the import section)
/// is fixed before a `Lir::CallImport` is resolved to an index.
///
/// The entry point ALSO imports `dup` iff the body has any Perceus RETAIN site (`collect_dup_sites` — a
/// heap binding/param consumed while it has a later live use, emitted by `emit_binder_ref`). Computed ONCE
/// over the whole body here, not per-node in the recursive walk, so a PARAM retain site (whose scope is the
/// whole function, not one `let`) is covered — the emit places its `dup` and the import must match.
pub fn collect_used_ops(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    // The retain-site `dup` import: mirror `select_function_of`'s `collect_dup_sites` over ALL heap binders
    // (params + `let`s) reachable in this body, and import `dup` if any occurrence needs a retain. Precise —
    // the FBIP single-use consume produces no site, so a body that never shares-then-consumes imports no dup.
    let mut retain_binders: Vec<StructId> = Vec::new();
    collect_retain_candidate_binders(db, id, &mut retain_binders);
    let mut sites: HashSet<StructId> = HashSet::new();
    collect_dup_sites(db, id, &retain_binders, &mut sites);
    if !sites.is_empty() {
        out.insert(OP_DUP);
    }
    collect_used_ops_into(db, id, out);
}

/// The recursive worker of [`collect_used_ops`] — descends every sub-position (both `if` branches, every
/// arm body — an op used only under a branch is still imported, since the branch may run). A box/get op
/// that would decline (a non-scalar element) is simply not added here; the decline surfaces at `emit`.
fn collect_used_ops_into(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match core_of(db, id) {
        Core::Tuple { elems } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            for elem in &elems {
                // A scalar element boxes (`box-int`/`box-bool`); a nested compound is already a handle
                // (`box_op` → `Ok(None)`), stored as-is. Recurse either way so a nested compound's own
                // construction ops (`arr-alloc`/`arr-set`/its element boxes) are collected.
                if let Ok(Some(op)) = box_op(db, *elem) {
                    out.insert(op);
                }
                // A rope-capable String/Bytes element is `bytes-compact`ed on construction (the emit
                // arm's nested-rope canonicalization), so import it.
                if elem_needs_rope_compaction(db, *elem) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into(db, *elem, out);
            }
        }
        Core::Proj { operand, .. } => {
            out.insert(OP_ARR_GET);
            let scalar_elem = get_op(db, id);
            if let Ok(Some(op)) = scalar_elem {
                out.insert(op);
            }
            // RECLAMATION (U13/U14): a projection off an OWNED-temporary aggregate reclaims it after the
            // borrowing read — mirror the emit's reclaim condition so the ops are imported. A SCALAR element
            // `drop`s the parent; a NESTED-COMPOUND element `dup`s the returned child then `drop`s the parent.
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
                if matches!(scalar_elem, Ok(None)) {
                    out.insert(OP_DUP);
                }
            }
            collect_used_ops_into(db, operand, out);
        }
        // A list construction is a BULK build: a flat `arr` (`arr-alloc` + a boxed `arr-set` per element,
        // like a tuple) then one `vec-of-arr`. So it imports the arr ops + `vec-of-arr`, not the old
        // `vec-empty`/`vec-push` chain.
        Core::ListNew { elems } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            out.insert(OP_VEC_OF_ARR);
            for elem in &elems {
                if let Ok(Some(op)) = box_op(db, *elem) {
                    out.insert(op);
                }
                if elem_needs_rope_compaction(db, *elem) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into(db, *elem, out);
            }
        }
        // `List.len` uses `vec-len` and evaluates its operand.
        Core::ListLen { operand } => {
            out.insert(OP_VEC_LEN);
            // RECLAMATION: a `vec-len` over an OWNED-temporary list drops it after the borrowing read
            // (mirror the emit's reclaim condition so `drop` is imported). A borrowed param/local is not
            // dropped (its owner reclaims).
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, operand, out);
        }
        // `Bytes.of` uses `bytes-alloc` + a `bytes-set` per element (each element is a raw byte — an
        // i32 in `0..=255`, NOT boxed to a handle, unlike a list element). Evaluate each element.
        Core::BytesOf { elems } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for elem in &elems {
                collect_used_ops_into(db, *elem, out);
            }
        }
        // A runtime `(bin …)` build allocs the byte buffer + writes each segment byte with `bytes-set`.
        Core::BinBuild { segs } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for s in &segs {
                collect_used_ops_into(db, s.value, out);
            }
        }
        // A runtime bit-field run allocs the buffer + writes each packed byte with `bytes-set`.
        Core::BinBitsBuild { fields } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for f in &fields {
                collect_used_ops_into(db, f.value, out);
            }
        }
        // A `BinIntRead` reads its segment bytes with `bytes-get`.
        Core::BinIntRead { bytes, .. } => {
            out.insert(OP_BYTES_GET);
            collect_used_ops_into(db, bytes, out);
        }
        // A `BinRestRead` slices the tail: `dup` the shared scrutinee, then `bytes-slice(bytes, off,
        // bytes-len - off)` on the copy.
        Core::BinRestRead { bytes, .. } => {
            out.insert(OP_DUP);
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_SLICE);
            collect_used_ops_into(db, bytes, out);
        }
        // `Bytes.len` uses `bytes-len` and evaluates its operand.
        Core::BytesLen { operand } => {
            out.insert(OP_BYTES_LEN);
            // RECLAMATION: a `bytes-len` over an OWNED-temporary bytes drops it after the borrow (mirror the
            // emit); a borrowed param/local is not dropped (its owner reclaims).
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, operand, out);
        }
        // `List.push` uses `vec-push` (the pushed element boxed by its type); `List.concat` uses `vec-concat`.
        Core::ListPush { list, elem } => {
            out.insert(OP_VEC_PUSH);
            if let Ok(Some(op)) = box_op(db, elem) {
                out.insert(op);
            }
            collect_used_ops_into(db, list, out);
            collect_used_ops_into(db, elem, out);
        }
        Core::ListConcat { lhs, rhs } => {
            out.insert(OP_VEC_CONCAT);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // `List.update` uses `vec-update` (the replacement element boxed by its type, like a push).
        Core::ListUpdate { list, index, elem } => {
            out.insert(OP_VEC_UPDATE);
            if let Ok(Some(op)) = box_op(db, elem) {
                out.insert(op);
            }
            collect_used_ops_into(db, list, out);
            collect_used_ops_into(db, index, out);
            collect_used_ops_into(db, elem, out);
        }
        // A RUNTIME `List.at` reads the length (`vec-len`) for the bounds test and, in bounds, the
        // element (`vec-get`, which BORROWS → `dup` before the `Some` consumes it), then builds
        // `Some`/`None` (`sum-new`, with `arr-alloc(0)` for `None`'s unit payload). The element stays
        // BOXED (the handle `vec-get` returns feeds `sum-new` directly; a downstream match unboxes it),
        // so no `box-*`/`get-*` here — mirrors the `emit` arm's op choices exactly.
        Core::ListAt { list, index, .. } => {
            // Some(elem) via `vec-get` (dup'd) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_VEC_LEN);
            out.insert(OP_VEC_GET);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            // RECLAMATION: an OWNED-temporary list operand is dropped after the borrowing len/get (the emit
            // reclaims it). This collect pass has no `slots` to test `reusable_handle_slot`, so import `drop`
            // whenever the operand's ownership is Owned — a superset of the emit's condition (a declared-but-
            // unused import is harmless; the emit only actually emits the drop for a non-reused owned list).
            if matches!(heap_operand_ownership(db, list), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, list, out);
            collect_used_ops_into(db, index, out);
        }
        // A map construction is `map-empty` then a `map-insert` per entry (each key/value boxed by its
        // type). Mirrors the emit arm's op choices.
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            out.insert(OP_MAP_EMPTY);
            if !entries.is_empty() {
                out.insert(OP_MAP_INSERT);
            }
            for (k, v) in &entries {
                // NODE-AWARE box op (mirror the emit's `box_op_for`, not `box_op_ty`) — a Float key/value into
                // an empty (`Var`-typed) map must import the `box-float` the emit calls, not the `box-int`
                // `box_op_ty` defaults an unresolved `Var` to (the empty-collection float-element gap).
                if let Ok(Some(op)) = box_op_for(db, *k, &key_ty) {
                    out.insert(op);
                }
                if let Ok(Some(op)) = box_op_for(db, *v, &val_ty) {
                    out.insert(op);
                }
                if key_needs_compaction(db, *k) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into(db, *k, out);
                collect_used_ops_into(db, *v, out);
            }
        }
        // `Map.insert` = `map-insert`, boxing the key and value by their types (NODE-AWARE `box_op_for` so a
        // Float key/value into an empty `Var`-typed map imports the `box-float` the emit calls — see `MapNew`).
        Core::MapInsert {
            map,
            key,
            val,
            key_ty,
            val_ty,
        } => {
            out.insert(OP_MAP_INSERT);
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if let Ok(Some(op)) = box_op_for(db, val, &val_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, map, out);
            collect_used_ops_into(db, key, out);
            collect_used_ops_into(db, val, out);
        }
        // A RUNTIME `Map.lookup`: box the key, `map-lookup` (→ the stored value handle, or NULL when
        // absent), then build `Some(value)` / `None` (`sum-new`, `arr-alloc(0)` for None's unit). The
        // stored value is a BOXED handle (like a list element), used DIRECTLY as the `Some` payload —
        // `dup`'d so the map keeps its own reference (mirrors `ListAt`) — no unbox. The boxed key is an
        // owned temporary the emit `drop`s after the borrow-lookup.
        Core::MapLookup {
            map, key, key_ty, ..
        } => {
            // Some(value) via `map-lookup` (dup'd) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_MAP_LOOKUP);
            out.insert(OP_DUP);
            out.insert(OP_DROP);
            out.insert(OP_SUM_NEW);
            // NODE-AWARE box op for the looked-up KEY (mirror the emit) — a Float key into an empty `Var`-typed
            // map imports the `box-float` the emit calls, not `box_op_ty`'s `box-int` default.
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, map, out);
            collect_used_ops_into(db, key, out);
        }
        // `Map.remove` = `map-remove`, boxing the key by its type (NODE-AWARE `box_op_for` — see `MapInsert`).
        Core::MapRemove { map, key, key_ty } => {
            out.insert(OP_MAP_REMOVE);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, map, out);
            collect_used_ops_into(db, key, out);
        }
        // `Map.size` = `map-size` (→ u32, extended to i64) — reads the map operand.
        Core::MapSize { map } => {
            out.insert(OP_MAP_SIZE);
            // RECLAMATION: a `map-size` over an OWNED-temporary map drops it after the borrow (mirror emit).
            if matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, map, out);
        }
        // A set construction is `set-empty` then a `set-insert` per element (each boxed by its type).
        Core::SetOf { elems, elem_ty } => {
            out.insert(OP_SET_EMPTY);
            if !elems.is_empty() {
                out.insert(OP_SET_INSERT);
            }
            for &e in &elems {
                // NODE-AWARE box op (mirror the emit's `box_op_for`, NOT `box_op_ty`): when `elem_ty` is an
                // unresolved `Var`/`Any` (an empty-base set fixed no element type), `box_op_ty` DEFAULTS to
                // `box-int`, but the emit's `box_op_for` falls back to the ELEMENT NODE's real type — a Float
                // element emits `box-float`. Collecting `box-int` while the emit calls `box-float` leaves
                // `box-float` un-imported → `call u32::MAX` → INVALID WASM (the empty-set float-element gap,
                // the historical empty-set String box-int bug's twin). Use the node so the two agree.
                if let Ok(Some(op)) = box_op_for(db, e, &elem_ty) {
                    out.insert(op);
                }
                if key_needs_compaction(db, e) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into(db, e, out);
            }
        }
        // `Set.contains` = `set-contains` (→ bool), boxing the element (an owned temporary the emit drops).
        Core::SetContains { set, elem, elem_ty } => {
            out.insert(OP_SET_CONTAINS);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, set, out);
            collect_used_ops_into(db, elem, out);
        }
        // `Set.insert`/`Set.remove` = `set-insert`/`set-remove`, boxing the element by its type. NODE-AWARE
        // `box_op_for` (not `box_op_ty`) so a Float element into an empty (`Var`-typed) set imports the
        // `box-float` the emit calls, not the `box-int` `box_op_ty` defaults a `Var` to — see `SetOf`.
        Core::SetInsert { set, elem, elem_ty } => {
            out.insert(OP_SET_INSERT);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, set, out);
            collect_used_ops_into(db, elem, out);
        }
        // `set-remove` BORROWS the element, so the emit drops an OWNED-TEMPORARY element after the borrow
        // (the ownership gate) — hence `drop`.
        Core::SetRemove { set, elem, elem_ty } => {
            out.insert(OP_SET_REMOVE);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, set, out);
            collect_used_ops_into(db, elem, out);
        }
        // `Set.to-list` = `set-to-list` + the inline descriptor `Bytes` build (`bytes-alloc`/`bytes-set`).
        Core::SetToList { set, .. } => {
            out.insert(OP_SET_TO_LIST);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // the borrowed-only descriptor Bytes is dropped after the op
            collect_used_ops_into(db, set, out);
        }
        // `Map.to-list` = `map-to-list` + the inline descriptor `Bytes` build (`bytes-alloc`/`bytes-set`).
        Core::MapToList { map, .. } => {
            out.insert(OP_MAP_TO_LIST);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // the borrowed-only descriptor Bytes is dropped after the op
            collect_used_ops_into(db, map, out);
        }
        // `Set.len` = `set-size` (→ u32, extended to i64) — reads the set operand.
        Core::SetLen { set } => {
            out.insert(OP_SET_SIZE);
            // RECLAMATION: a `set-size` over an OWNED-temporary set drops it after the borrow (mirror emit).
            if matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, set, out);
        }
        // A set-algebra op = the matching runtime op (consumes both operand sets).
        Core::SetAlgebra { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::SetAlgebraOp::Union => OP_SET_UNION,
                crate::core::SetAlgebraOp::Intersection => OP_SET_INTERSECTION,
                crate::core::SetAlgebraOp::Difference => OP_SET_DIFFERENCE,
            });
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // A RUNTIME `Bytes.at`: `bytes-len` (bounds test) + `bytes-get` (the raw byte VALUE, in bounds),
        // then `box-int` the byte into the `Some` payload (`sum-new`), or `arr-alloc(0)` for `None`'s
        // unit payload. No `dup` — `bytes-get` returns a value, not a borrowed handle. Mirrors `emit`.
        Core::BytesAt { bytes, index, .. } => {
            // Some(byte) via `bytes-get` (box-int) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BOX_INT);
            out.insert(OP_SUM_NEW);
            // RECLAMATION: an OWNED-temporary bytes operand is dropped after the borrowing len/get (see the
            // emit + the `ListAt` collect arm — import `drop` for an Owned operand; harmless if the emit's
            // reused-slot condition suppresses the actual drop).
            if matches!(
                heap_operand_ownership(db, bytes),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into(db, bytes, out);
            collect_used_ops_into(db, index, out);
        }
        // `String.at` on a runtime string walks the UTF-8 buffer (`bytes-len`/`bytes-get`), slices the
        // scalar span (`bytes-slice`, which CONSUMES the string handle → the borrowed scan `dup`s first,
        // and the None branch `drop`s the un-consumed handle), and builds `Some`/`None` (`sum-new`,
        // `arr-alloc` for the unit payload).
        Core::StrAt { string, index, .. } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BYTES_SLICE);
            // The Some-branch DUPs the string (`dup`), the slice CONSUMES that dup'd copy, then COMPACTS
            // the fresh slice to an independent flat leaf (see the emit) so a `String.at` result's
            // content-equality / key-hashing compares by content, not rope offset. The original string is
            // NOT dropped here — its owner (an enclosing let/param) reclaims it (see the emit comment), so
            // no `drop` is imported (unlike Map.lookup/Set.contains, whose boxed KEY is an owned temporary
            // they must drop). None is the inline-unit constant (`IMM_UNIT`), NOT an allocation — no
            // `arr-alloc` either.
            out.insert(OP_BYTES_COMPACT);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into(db, string, out);
            collect_used_ops_into(db, index, out);
        }
        // `String.slice` walks the UTF-8 buffer to the start/end scalar byte positions (`bytes-len`/`bytes-
        // get`), slices that span (`bytes-slice`, which CONSUMES the string handle → the Some branch `dup`s
        // first), COMPACTS the fresh slice to an independent flat leaf (content-equality/key-hashing), and
        // builds `Some`/`None` (`sum-new`; `None` is the inline `IMM_UNIT`, no `arr-alloc`). Same op set as
        // `String.at`. The source string is NOT dropped here (its owner reclaims it), so no `drop` import.
        Core::StrSlice {
            string, start, end, ..
        } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BYTES_SLICE);
            out.insert(OP_BYTES_COMPACT);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into(db, string, out);
            collect_used_ops_into(db, start, out);
            collect_used_ops_into(db, end, out);
        }
        // `Bytes.concat` = `bytes-concat`; `Bytes.compact` = `bytes-compact`; `Bytes.slice` bounds-checks
        // via `bytes-len` then builds `Some(bytes-slice)` (a Bytes HANDLE, no box) / `None` (`arr-alloc(0)`).
        Core::BytesConcat { lhs, rhs } => {
            out.insert(OP_BYTES_CONCAT);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // `bigint-of-i64` mints a leaf from an i64 scalar — no handle operand, no drop. An UNSIGNED source
        // instead materializes a sign-magnitude byte leaf (`bytes-alloc`/`bytes-set` → `bigint-of-bytes`),
        // so declare those imports to match the emit's signedness branch (see the `Core::BigIntOfI64` emit).
        Core::BigIntOfI64 { value } => {
            if int_ty_of(db, value).ground_signed() {
                out.insert(OP_BIGINT_OF_I64);
            } else {
                out.insert(OP_BYTES_ALLOC);
                out.insert(OP_BYTES_SET);
                out.insert(OP_BIGINT_OF_BYTES);
            }
            collect_used_ops_into(db, value, out);
        }
        // The borrowing BigInt ops also import `drop` (to reclaim an OWNED-temporary handle operand after
        // the borrowing call — see the `emit_bigint_borrow_*` helpers), plus `bigint-of-i64` when an
        // operand is a CONSTANT BigInt materialized inline (`const_bigint_materializes`).
        Core::BigIntToI64 { operand } => {
            out.insert(OP_BIGINT_TO_I64_CHECKED);
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, operand, out);
            collect_used_ops_into(db, operand, out);
        }
        Core::BigIntBinOp { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::BigIntOp::Add => OP_BIGINT_ADD,
                crate::core::BigIntOp::Sub => OP_BIGINT_SUB,
                crate::core::BigIntOp::Mul => OP_BIGINT_MUL,
                crate::core::BigIntOp::Div => OP_BIGINT_DIV,
                crate::core::BigIntOp::Rem => OP_BIGINT_REM,
            });
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, lhs, out);
            insert_const_bigint_materialize_ops(db, rhs, out);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // A BigInt comparison imports `bigint-cmp` (the three-way primitive) AND `drop` (to reclaim an
        // owned-temporary operand after the borrowing compare — the `emit_bigint_borrow_binary` helper),
        // plus the materialization ops for an inline-materialized constant operand.
        Core::BigIntCmp { lhs, rhs, .. } => {
            out.insert(OP_BIGINT_CMP);
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, lhs, out);
            insert_const_bigint_materialize_ops(db, rhs, out);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // `Rational.of n d` on runtime ints — widen each to a BigInt (`bigint-of-i64`) then `rational-of`.
        Core::RationalOfInts { num, den } => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
            collect_used_ops_into(db, num, out);
            collect_used_ops_into(db, den, out);
        }
        // `Rational.of-int n` — widen `n` + the constant `1` to BigInt, then `rational-of`.
        Core::RationalOfIntWiden { value } => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
            collect_used_ops_into(db, value, out);
        }
        // The borrowing Rational arithmetic ops import their op + `drop` (reclaim an owned-temporary
        // operand after the borrowing call — the `emit_rational_borrow_binary` helper).
        Core::RationalBinOp { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::RationalOp::Add => OP_RATIONAL_ADD,
                crate::core::RationalOp::Sub => OP_RATIONAL_SUB,
                crate::core::RationalOp::Mul => OP_RATIONAL_MUL,
                crate::core::RationalOp::Div => OP_RATIONAL_DIV,
            });
            out.insert(OP_DROP);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        Core::RationalCmp { lhs, rhs, .. } => {
            out.insert(OP_RATIONAL_CMP);
            out.insert(OP_DROP);
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_SLICE);
            out.insert(OP_DROP); // the None branch drops the un-consumed bytes reference
            out.insert(OP_SUM_NEW);
            // None is the inline-unit constant (`IMM_UNIT`), NOT an allocation — no `arr-alloc` (see the
            // emit arm; PR #404 class).
            collect_used_ops_into(db, bytes, out);
            collect_used_ops_into(db, start, out);
            collect_used_ops_into(db, len, out);
        }
        Core::BytesCompact { operand } => {
            out.insert(OP_BYTES_COMPACT);
            collect_used_ops_into(db, operand, out);
        }
        // `String.from-bytes` on a runtime Bytes: `str-from-bytes` (→ the String handle, or NULL when the
        // buffer is invalid UTF-8), then build `Some(handle)` / `None` via `sum-new`. The returned handle is
        // already OWNED (str-from-bytes consumes the buffer and transfers it out), so it is used DIRECTLY as
        // the `Some` payload — no `dup`. The None branch's unit payload is the inline `IMM_UNIT` CONSTANT
        // (matching the emit, exactly as `Map.lookup`/`Bytes.at` build their None), NOT an `arr-alloc(0)`
        // call — so `arr-alloc` must NOT be imported here (the emit never calls it; importing it declares a
        // DEAD runtime op for a program using only str-from-bytes + sum-new). The None branch has no handle
        // to drop (the runtime consumed the buffer on failure).
        Core::StrFromBytes { bytes, .. } => {
            // `str-from-bytes` decodes the buffer to a String handle or NULL; the emit then builds
            // `Some(handle)` / `None` via `sum-new`. The `None` payload is the INLINE-unit constant
            // (`IMM_UNIT`), not an allocated cell — so `sum-new` is the only heap op, no `arr-alloc`.
            // (An earlier version over-declared `OP_ARR_ALLOC` "for None's unit"; None uses no alloc, so
            // importing it forced an unnecessary runtime import — PR #404 Copilot review.)
            out.insert(OP_STR_FROM_BYTES);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into(db, bytes, out);
        }
        // `String.to-bytes` on a runtime String: `bytes-compact` flattens the string's byte-rope to a
        // canonical flat leaf (a String IS a UTF-8 Bytes leaf, so no conversion) and transfers it out as the
        // Bytes result — the total encoding needs no `sum-new`/validation, just the one flatten op.
        Core::StrToBytes { string } => {
            out.insert(OP_BYTES_COMPACT);
            collect_used_ops_into(db, string, out);
        }
        Core::If { cond, then_, else_ } => {
            collect_used_ops_into(db, cond, out);
            collect_used_ops_into(db, then_, out);
            collect_used_ops_into(db, else_, out);
        }
        Core::Match { scrutinee, arms } => {
            collect_used_ops_into(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_used_ops_into(db, g, out);
                }
                collect_used_ops_into(db, arm.body, out);
            }
        }
        Core::Let { bindings, body } => {
            for (binder, value) in &bindings {
                // A HEAP-typed binding is `drop`'d after the body (Perceus) — so the program imports
                // `drop`. (A scalar binding owns no heap cell → no drop, matching `emit`.) The `dup` a
                // consumed-then-reused binding needs is imported ONCE at the `collect_used_ops` entry
                // (over the whole body, covering params too), not per-binding here.
                if is_heap_type(&type_of(db, *binder)) {
                    out.insert(OP_DROP);
                }
                collect_used_ops_into(db, *value, out);
            }
            collect_used_ops_into(db, body, out);
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        // Runtime structural equality imports `value-eq` (the compare) AND `drop` (to reclaim an owned
        // temporary operand after the borrowing compare — see the `Core::ValueEq` emit). A STRING/BYTES
        // operand is canonicalized with `bytes-compact` before the compare (a rope vs its flat twin — see
        // the emit), so import `bytes-compact` when either operand is a String or Bytes.
        Core::ValueEq { lhs, rhs } => {
            out.insert(OP_VALUE_EQ);
            out.insert(OP_DROP);
            if operand_is_string_or_bytes(db, lhs) || operand_is_string_or_bytes(db, rhs) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into(db, lhs, out);
            collect_used_ops_into(db, rhs, out);
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            collect_used_ops_into(db, operand, out)
        }
        Core::Call { args, .. } => {
            // A CONSTANT-BigInt argument to a BigInt param materializes via `bigint-of-i64` in the
            // `Core::ConstInt` collect arm (matching its emit) — no per-call special-case needed here.
            for arg in args {
                collect_used_ops_into(db, arg, out);
            }
        }
        // A HOST CALL vs a PEER CALL — mirror the `emit` arm's arg handling EXACTLY, and the two boundaries
        // treat a String argument DIFFERENTLY:
        //  - A HOST call marshals a `Ty::String` arg as `(ptr, len)` into the data segment (the emit arm
        //    consumes the `Core::ConstStr` there) — it is NOT built as a runtime byte-leaf, so it uses NO
        //    runtime op; descending into it via the generic walk would wrongly import `bytes-alloc`/
        //    `bytes-set` (the ConstStr arm), making the runtime-import set non-empty and tripping the
        //    "host + runtime imports don't yet compose" decline for what is a host-ONLY program.
        //  - A PEER call (a peer-BOUND effect, `db.effect_bindings`) crosses a String/Bytes arg as a runtime
        //    HANDLE — the emit arm calls `emit(arg)`, which for a `Core::ConstStr` builds the rope on the
        //    value heap (`bytes-alloc`/`bytes-set`). So a peer String/Bytes arg DOES use runtime ops and MUST
        //    recurse, or the import section would omit an op the body calls (→ an invalid consumer component).
        // A `Unit` arg carries no value on either boundary. So: skip a String/Unit HOST arg; recurse into a
        // peer arg (and any scalar/compound host arg) as before.
        Core::HostCall { args, effect, .. } => {
            let peer_bound = db.effect_bindings.contains_key(&effect);
            for arg in args {
                match crate::infer::type_of(db, arg) {
                    Ty::Unit => {}
                    Ty::String | Ty::Bytes if !peer_bound => {}
                    _ => collect_used_ops_into(db, arg, out),
                }
            }
        }
        Core::Seq { stmts, tail } => {
            for s in stmts {
                collect_used_ops_into(db, s, out);
            }
            collect_used_ops_into(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value to reach any op inside.
        Core::Block { body, .. } => collect_used_ops_into(db, body, out),
        Core::Break { value } => collect_used_ops_into(db, value, out),
        Core::Record { fields } => {
            // A runtime record builds on the heap exactly as a tuple — `arr-alloc` + per-field
            // `box-*`/`arr-set` (the same ops `emit`'s `Core::Record` arm lays down), so the used-set
            // must include them or the import section would omit an op the body calls.
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            for value in fields.values() {
                if let Ok(Some(op)) = box_op(db, *value) {
                    out.insert(op);
                }
                if elem_needs_rope_compaction(db, *value) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into(db, *value, out);
            }
        }
        // A sum construction always calls `sum-new`; the payload build mirrors `emit`'s `Core::SumNew`:
        //  - nullary → the inline-unit CONSTANT (`IMM_UNIT`), no runtime op (see `emit`);
        //  - single → `box-*` the one payload (a compound payload is already a handle, no box);
        //  - multi → a tuple handle (`arr-alloc` + per-payload `box-*`/`arr-set`).
        Core::SumNew { payloads, .. } => {
            // An ENUM-DISC sum (all variants nullary) is built as a bare `i32.const disc` — NO `sum-new`,
            // no payload op (mirrors `emit`'s `node_is_enum_disc` fast path). Over-reporting `sum-new`
            // here declares a dead runtime import.
            if node_is_enum_disc(db, id) {
                return;
            }
            out.insert(OP_SUM_NEW);
            match payloads.len() {
                0 => {
                    // The unit payload is the inline-unit constant — no `arr-alloc` import.
                }
                1 => {
                    if let Ok(Some(op)) = box_op(db, payloads[0]) {
                        out.insert(op);
                    }
                    if elem_needs_rope_compaction(db, payloads[0]) {
                        out.insert(OP_BYTES_COMPACT);
                    }
                    collect_used_ops_into(db, payloads[0], out);
                }
                _ => {
                    out.insert(OP_ARR_ALLOC);
                    out.insert(OP_ARR_SET);
                    for p in &payloads {
                        if let Ok(Some(op)) = box_op(db, *p) {
                            out.insert(op);
                        }
                        if elem_needs_rope_compaction(db, *p) {
                            out.insert(OP_BYTES_COMPACT);
                        }
                        collect_used_ops_into(db, *p, out);
                    }
                }
            }
        }
        // A sum match calls `sum-disc` to dispatch at each switch; a switch on a deeper sub-value (a
        // non-empty `path`) first WALKS there (`sum-payload`/`arr-get` per step) before the disc. The
        // scrutinee + the root continuation are emitted (any op reachable in the tree must be imported) —
        // `collect_cont_ops` recurses switches/guards, inserting each switch's disc + walk ops.
        Core::MatchSum { scrutinee, root } => {
            collect_used_ops_into(db, scrutinee, out);
            collect_cont_ops(db, scrutinee, &root, out);
        }
        // A list match reads `vec-len` to dispatch by length; arm bodies' element/rest binders bring in
        // `vec-get`/`vec-split` via their own `SumPayload` occurrences. A guarded arm's GUARD is also
        // emitted (its ops must be collected too).
        Core::MatchList { scrutinee, arms } => {
            out.insert(OP_VEC_LEN);
            collect_used_ops_into(db, scrutinee, out);
            for arm in &arms {
                if let Some(g) = arm.guard {
                    collect_used_ops_into(db, g, out);
                }
                collect_used_ops_into(db, arm.body, out);
            }
        }
        // A sum-payload read walks its `path` (`sum-payload`/`arr-get` per step) then unboxes the leaf
        // by THIS node's solved type (`get-*`).
        Core::SumPayload { scrutinee, path } => {
            for step in &path {
                match step {
                    crate::core::PathStep::Payload => {
                        out.insert(OP_SUM_PAYLOAD);
                    }
                    // An `Elem` may read a tuple `arr` OR a list `vec`; insert both (emit picks by type).
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET);
                    }
                    // A list REST binder slices the tail with `vec-drop` (single-result tail), preceded by
                    // a `dup` to RETAIN the shared arm handle across the consuming slice (so a sibling
                    // element binder in the same arm still reads a live handle — see the `RestFrom` emit).
                    crate::core::PathStep::RestFrom(_) => {
                        out.insert(OP_VEC_DROP);
                        out.insert(OP_DUP);
                    }
                };
            }
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            collect_used_ops_into(db, scrutinee, out);
        }
        // `expect` probes the discriminant (`sum-disc`) and, on the present arm, reads the payload
        // (`sum-payload`) then unboxes by the result type (`get-*`); the absent arm traps (no op). It also
        // emits the scrutinee once.
        Core::SumExpect { scrutinee, .. } => {
            out.insert(OP_SUM_DISC);
            out.insert(OP_SUM_PAYLOAD);
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            collect_used_ops_into(db, scrutinee, out);
        }
        // A closure VALUE is a heap CELL — `arr-alloc(1 + captures)` then `arr-set` of `box-int(code)`
        // (slot 0) and each boxed capture. So it uses `arr-alloc`/`arr-set`/`box-int` always, plus the
        // per-capture box op. A closure APPLICATION reads the code slot (`arr-get`+`get-int`) then
        // `call_indirect` (a core instruction, not a runtime import), plus its operands.
        Core::Closure { captures, .. } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            out.insert(OP_BOX_INT); // slot 0 = box-int(code)
            for &c in &captures {
                if let Ok(Some(op)) = box_op(db, c) {
                    out.insert(op);
                }
                collect_used_ops_into(db, c, out);
            }
        }
        Core::CallClosure { closure, args } => {
            out.insert(OP_ARR_GET); // read the code slot from the cell
            out.insert(OP_GET_INT); // unbox it to the table index
            collect_used_ops_into(db, closure, out);
            for arg in args {
                collect_used_ops_into(db, arg, out);
            }
        }
        // A CAPTURED-variable read: `arr-get(env, 1+index)` then unbox by the captured value's type.
        Core::Captured { .. } => {
            out.insert(OP_ARR_GET);
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
        }
        // A constant STRING used as an in-body runtime value builds a flat UTF-8 byte leaf via
        // `bytes-alloc` + a `bytes-set` per byte (byte-identical to `str-new`'s rep — see the `emit`
        // arm). So it imports those two ops. (A constant string that only FOLDS — its equality, or an
        // escape's baked bytes — never reaches the emit path, so this is a superset that is harmless if
        // the string folds away; `collect_used_ops` mirrors `emit`'s op choices for the values that DO
        // reach emission.)
        Core::ConstStr(_) => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
        }
        // A CONSTANT typed `BigInt` used as an in-body runtime value MATERIALIZES at `emit` (a map key/value,
        // set element, call arg, op operand — it must be an i32 handle, not a raw i64): `bigint-of-i64` for
        // an i64-fitting value, or `bytes-alloc`/`bytes-set`/`bigint-of-bytes` for a beyond-i64 one — declare
        // whichever it needs here to match `emit_const_bigint_leaf`. (A whole-export constant BigInt takes
        // the baked-bytes path and never reaches `emit`, but an unused import would be harmless if it did.)
        Core::ConstInt(_) if is_bigint_valued(db, id) => {
            insert_const_bigint_materialize_ops(db, id, out);
        }
        // A constant Rational used as an in-body runtime value MATERIALIZES via `bigint-of-i64` (×2 the
        // components) + `rational-of` at `emit` — declare those imports to match. (A component beyond i64
        // declines at emit; a whole-export constant Rational takes the baked path and doesn't reach here.)
        Core::ConstRational(n, d) if n.to_i64().is_some() && d.to_i64().is_some() => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
        }
        // Leaves and references emit no runtime op. `trap` emits `unreachable` (a core instruction, not a
        // runtime import), so it adds nothing.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        | Core::Trap
        | Core::Param { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// Collect the runtime ops a sum-match CONTINUATION uses — a leaf's body, or a nested switch (its own
/// `sum-disc` + path walk ops + its arms, recursed). Mirrors the `MatchSum` arm walk in `collect_used_ops`
/// so an op used only deep in the tree is still imported.
fn collect_cont_ops(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    // The entered-variant payload types, threaded exactly as the EMIT threads `Emit::sum_path_types`, so the
    // `sub_is_enum` disc-op choice here agrees with `push_discriminant`'s (which now resolves a `Payload`
    // step to the ACTUAL entered variant, not variant 0). Starts empty at the root.
    let mut recorded: HashMap<Vec<crate::core::PathStep>, Ty> = HashMap::new();
    collect_cont_ops_rec(db, scrutinee, cont, &mut recorded, out);
}

fn collect_cont_ops_rec(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    recorded: &mut HashMap<Vec<crate::core::PathStep>, Ty>,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_used_ops(db, *body, out),
        // A guarded arm uses the ops of its guard cond, its body, AND the fall-through continuation.
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_used_ops(db, *cond, out);
            collect_used_ops(db, *body, out);
            collect_cont_ops_rec(db, scrutinee, els, recorded, out);
        }
        // A literal test walks its `path` (sum-payload/arr-get|vec-get) then reads the leaf scalar to compare
        // it; an Int probe reads `get-int`, a Bool probe `get-bool`. Then both continuations' ops.
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            for step in path {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    // An `Elem` may read a tuple `arr` OR a list `vec` — insert both; emit picks by type.
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET)
                    }
                    crate::core::PathStep::RestFrom(_) => false, // never on a sum-disc path
                };
            }
            match probe {
                crate::core::Probe::Int(_) => out.insert(OP_GET_INT),
                crate::core::Probe::Bool(_) => out.insert(OP_GET_BOOL),
                // A string-literal probe over a RUNTIME payload emits a `value-eq` content compare against a
                // freshly-built literal byte-leaf (`bytes-alloc`+`bytes-set`), after `bytes-compact`ing the
                // leaf handle to canonical flat form and dropping the owned literal — so import all four (the
                // emit arm's ops). A CONSTANT string sub-value still folds in `build_tree` and never reaches
                // here; this covers the runtime case (`(Ast.Name "+")` over a runtime Ast).
                crate::core::Probe::Str(_) => {
                    out.insert(OP_BYTES_COMPACT);
                    out.insert(OP_BYTES_ALLOC);
                    out.insert(OP_BYTES_SET);
                    out.insert(OP_VALUE_EQ);
                    out.insert(OP_DROP)
                }
                // A char-literal probe only FOLDS (a constant char payload) — a runtime char has no machine
                // rep, so a runtime-char payload declines at emit rather than reaching here; no op to collect.
                crate::core::Probe::Char(_) => false,
                // A `ListLen` probe over a runtime list payload reads `vec-len` of the sub-list handle to
                // gate the arm (a constant list folds instead, never reaching here).
                crate::core::Probe::ListLen { .. } => out.insert(OP_VEC_LEN),
                // A `MapHasKeys` probe only ever FOLDS (a constant map sub-value); a runtime map declines at
                // `build_lit_test` before a decision tree emits, so it never reaches a runtime LitTest — no
                // op to collect.
                crate::core::Probe::MapHasKeys { .. } => false,
                crate::core::Probe::Wild => false,
            };
            collect_cont_ops_rec(db, scrutinee, then_, recorded, out);
            collect_cont_ops_rec(db, scrutinee, els, recorded, out);
        }
        crate::core::SumCont::Switch { path, arms } => {
            // Mirror `push_discriminant` EXACTLY: the switched sub-value's discriminant is read via
            // `sum-disc` for a BOXED sum, but an ENUM-DISC value needs none at the top level (it IS the
            // raw i32) and `get-int` (+ `i32.wrap_i64`, a core op) at a NESTED position. Over-reporting
            // `sum-disc` here — as the old unconditional insert did — declares a DEAD runtime import for a
            // program whose only match is on an all-nullary enum (now a bare i32), forcing a needless
            // `heap` linkage. Resolve the sub-value's type at `path` USING the recorded entered-variant
            // types (so a non-variant-0 payload agrees with the emit) and branch as the emit does.
            let root = type_of(db, scrutinee);
            let sub = ty_at_path_recorded(db, &root, path, recorded);
            let sub_is_enum = ty_is_enum_disc(db, &sub);
            for step in path {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    // An `Elem` may read a tuple `arr` OR a list `vec` — insert both; emit picks by type.
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET)
                    }
                    crate::core::PathStep::RestFrom(_) => false, // never on a sum-disc path
                };
            }
            if sub_is_enum {
                // A NESTED enum-disc was boxed as an int → `get-int` recovers it (the wrap is a core op).
                if !path.is_empty() {
                    out.insert(OP_GET_INT);
                }
            } else {
                out.insert(OP_SUM_DISC);
            }
            for arm in arms {
                // Thread each arm's entered-variant payload type (scoped save/restore), mirroring the emit.
                let restore = arm
                    .disc
                    .and_then(|d| record_entered_payload_ty_into(db, scrutinee, path, d, recorded));
                collect_cont_ops_rec(db, scrutinee, &arm.cont, recorded, out);
                restore_entered_payload_ty_into(path, restore, recorded);
            }
        }
    }
}

/// Whether the body at `id` PROVABLY diverges — its core reduces to an unconditional `Core::Trap`,
/// possibly THROUGH a sequencing/binding wrapper whose value is that trap. A diverging body's
/// `unreachable` is stack-polymorphic (validates in any result position), so its function is emitted
/// with a UNIT (0-result) signature rather than declining "return type has no machine representation".
///
/// Peers through the two value-position wrappers whose value is their TAIL: `Core::Seq { tail }` (an
/// effect-statement run then a value — the `(do (log.emit …) (trap …))` shape a test-failure path takes)
/// and `Core::Let { body }` (a binding then a body). A bare `Core::Trap` is the base case. Every other
/// core shape is NOT provably diverging (returns `false`) — so a genuine value-returning body keeps its
/// solved type and a real "no machine representation" decline still fires for it. Conservative: it proves
/// divergence only through these value-forwarding wrappers, never guesses.
///
/// `pub(crate)` because the component-boundary layer (`wasm::mod`) makes the same "diverging export → a
/// unit (no-result) boundary entry" decision and must recognize the same shapes (a bare trap AND a
/// trap-through-`Seq`/`Let`), so both the core-signature site here and the boundary site there share this.
pub(crate) fn body_diverges(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Trap => true,
        // A sequence's value is its tail; the statements run for effect. Diverges iff the tail does.
        Core::Seq { tail, .. } => body_diverges(db, tail),
        // A `let`'s value is its body; the bindings are evaluated first. Diverges iff the body does.
        Core::Let { body, .. } => body_diverges(db, body),
        _ => false,
    }
}

/// Select a function body with `params` — each a `(name-occurrence, solved-type)`, in signature order.
/// The parameters occupy wasm local slots `0..n` in order; a `Core::Param` reference to a parameter
/// emits `local.get <slot>`. The return type is the body's solved type. A parameter whose type has no
/// machine representation (an unresolved/compound type) DECLINES here — an exported parameter needs a
/// definite scalar type (which an annotation supplies).
pub fn select_function(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    layout: &Layout,
) -> Result<SelectedFunc, Reject> {
    select_function_of(db, body, params, layout, None)
}

/// [`select_function`] plus the emitting function's OWN `db.defs` index (`self_def`) when known — used
/// to compile a SELF-tail-recursive function as a `loop` (its self-tail-calls iterate in place rather
/// than `return_call`). `None` (the `select_function` entry, and `select_body`) disables the loop
/// transform, so a self-call stays a `return_call`. A nullary or unknown-index function never loops.
pub fn select_function_of(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    layout: &Layout,
    self_def: Option<usize>,
) -> Result<SelectedFunc, Reject> {
    // Assign each parameter a local slot in order, and its wasm value type (its machine rep).
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_vts: Vec<ValType> = Vec::new();
    let mut param_slots: Vec<u32> = Vec::new();
    // Named SCALAR params for debug info (D3): slot `i` holds param `i`; record its source name + type
    // when it is a scalar (int width / bool). A compound (heap-handle) param is skipped — DWARF cannot
    // walk the tagless heap, so only scalars get a `DW_TAG_variable`. Cheap (a name lookup per param);
    // the emit path only reads it under a debug request.
    let mut locals: Vec<LocalVar> = Vec::new();
    for (binder, ty) in params.iter() {
        // A `Unit` parameter occupies NO wasm slot — Unit is zero-width (`valtype_of(Unit) = None`), so it
        // is ELIDED from the functype's params, exactly as a Unit RESULT is elided to a zero-result
        // functype and a Unit ARGUMENT (`Core::Unit`) pushes nothing. The slot counter advances only for
        // represented params, so the remaining params + scratch keep a dense `0..n` numbering. A
        // `Core::Param` reference to this binder emits nothing (see the `Core::Param` arm), the read
        // analogue of a Unit value carrying no machine content. This is what lets a `(-> Unit T)` closure
        // (the canonical lazy THUNK `Susp(Unit -> …)`) box + dispatch through `call_indirect`.
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        let slot = param_vts.len() as u32;
        let vt = valtype_of(ty).ok_or_else(|| {
            Reject::decline("a function parameter's type has no machine representation")
        })?;
        slot_of.insert(*binder, slot);
        param_vts.push(vt);
        param_slots.push(slot);
        if matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
            && let Some(name) = db.ast.as_name(*binder)
        {
            locals.push(LocalVar {
                slot,
                name: name.to_string(),
                ty: ty.clone(),
                is_param: true,
            });
        }
    }
    let mut ret = type_of(db, body);
    // A body that provably DIVERGES has a `Never` result type (a fresh var / `Any`) with no machine
    // representation, but it never RETURNS a value — its `unreachable` is stack-polymorphic and validates
    // in any result position. So a diverging function is emitted with a UNIT (0-result) signature rather
    // than declining "return type has no machine representation": `(def (main) (trap …))`, a zero-arm
    // match on a `Never` scrutinee (`(match (never-returns))` → `Core::Trap`), or a body that runs some
    // effect statements and THEN traps (`(host (log) (do (log.emit "m") (trap …)))` — a `Core::Seq` whose
    // tail is the trap, the shape a unit-test failure path takes). Only rewrite when `ret` has NO valtype
    // AND the body PROVABLY diverges (`body_diverges`) — a genuine value-returning body keeps its type (a
    // real "no machine rep" decline still fires for those).
    if valtype_of(&ret).is_none() && !matches!(ret, Ty::Unit) && body_diverges(db, body) {
        ret = Ty::Unit;
    }
    let mut code = Emit::new();
    // Perceus RETAIN placement (soundness): find every occurrence that CONSUMES a heap binding (a param or
    // a nested `let`) while that binding has a LATER live use, and record it so the emit `dup`s it. Without
    // this a value consumed by `List.push`/`Map.insert`/… in one operand and read again in a later operand
    // (or shared across two recursive-call operands) is mutated in place by the consuming op — a silent
    // wrong value. Computed ONCE here over all heap binders; the set is empty for the common single-use
    // body, so the FBIP fast path is unchanged. (See `collect_dup_sites`.)
    {
        let mut heap_binders: Vec<StructId> = Vec::new();
        collect_retain_candidate_binders(db, body, &mut heap_binders);
        collect_dup_sites(db, body, &heap_binders, &mut code.dup_sites);
    }
    // Scratch locals start PAST the parameters (slots `0..n` are the params); a guarded op claims scratch
    // slots from `base` up. `high` tracks the highest scratch slot used, and `scratch_ty` records each
    // scratch slot's VALUE TYPE (i32 for a ≤32-bit op, i64 otherwise) — a slot must be DECLARED at the
    // type it is `local.set` with, or wasm rejects the module. (A given scratch slot is used at one
    // width within one op's guarded sequence: arithmetic preserves type and a width conversion `emit_wrap`
    // moves through the value stack rather than stashing across widths — so the map records the slot's
    // type rather than assuming i64.)
    let base = param_vts.len() as u32;
    let mut high = base;
    let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
    // If this function tail-calls itself (or a mutually-recursive PEER of the same signature) through
    // `if`/`let`/`match` result positions, and has parameters, compile it as a LOOP: a member tail-call
    // updates the parameter locals and `br`s to the loop top instead of a `return_call` — no wasm call
    // frame per iteration. `loop_members` is the tail-recursive group this function belongs to (just
    // `[self_def]` for plain self-recursion; `even`,`odd` for a mutual pair). Detection is conservative
    // — see `body_has_member_tail_call` (only the `if`/`let`/`match` tail positions the transform handles).
    let loop_members: Vec<usize> = match self_def {
        Some(d) if !param_slots.is_empty() => mutual_loop_group(db, d),
        _ => Vec::new(),
    };
    let loops = !loop_members.is_empty();
    // A MUTUAL group (more than one member) dispatches on a `which` state local: the first scratch slot
    // (i32, holding a member discriminant). A plain self-loop needs no dispatch (`which = None`). The
    // `which` slot is claimed above `base`, so scratch for the bodies starts one higher.
    let mutual = loop_members.len() > 1;
    let which_slot = base;
    // The body's scratch floor. It rises past the `which` state slot (mutual) and past any LICM-hoisted
    // invariant slots (assigned below) — all of which live ACROSS the loop, so the body must not reuse them.
    let mut body_base = if mutual { base + 1 } else { base };
    if mutual {
        scratch_ty.insert(which_slot, ValType::I32);
        high = high.max(body_base);
    }
    // Every member's body references ITS OWN parameter occurrences; since the signatures are identical,
    // member `m`'s parameter at position `i` shares slot `i` with this function's. Map each member's
    // param binders onto the shared slots so `Core::Param` in a peer's body resolves (a peer body is
    // emitted inline under the dispatch below).
    let mut shared_slots = slot_of.clone();
    if mutual {
        for &m in &loop_members {
            for (i, p) in db.defs[m].params.clone().into_iter().enumerate() {
                let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                    Some(name_occ) => name_occ,
                    None => p,
                };
                shared_slots.insert(binder, i as u32);
            }
        }
    }
    let tl = loops.then(|| TailLoop {
        members: &loop_members,
        param_slots: &param_slots,
        which: mutual.then_some(which_slot),
        depth: 0,
    });
    // Initialize `which` to this function's OWN discriminant BEFORE the loop opens — it selects which
    // member body runs on the FIRST iteration (this function's own). A member cross-call updates `which`
    // for the next iteration; putting the init inside the loop would re-run it every iteration and
    // clobber that update (the entry would be re-selected forever — a correctness bug). So it is a
    // one-time setup outside the loop.
    if mutual {
        let self_which = loop_members
            .iter()
            .position(|&m| m == self_def.unwrap())
            .expect("self is a member of its own loop group") as i32;
        code.push(Lir::ConstI32(self_which));
        code.push(Lir::LocalSet(which_slot));
    }
    // LOOP-INVARIANT CODE MOTION: for a PLAIN self-loop (a single member), hoist trap-free, loop-invariant,
    // non-trivial subexpressions of the body — computed ONCE here (before the loop opens) into a fresh slot
    // and read back inside the body via `emit`'s node-keyed `slots.get(&id)` fast path. The classic win is
    // `(List.len xs)` in an index loop `(if (< i (List.len xs)) …)`: a `vec-len` import CALL, invariant
    // because `xs` is threaded unchanged, now runs once instead of per iteration. A mutual group is skipped
    // (its members share slots, so back-edge invariance is per-peer — deferred). Runs only when looping.
    if loops && !mutual {
        let self_d = self_def.expect("a loop has a self_def");
        let inv_params = invariant_param_binders(db, body, params, &slot_of, &loop_members, self_d);
        // The body's DOMINATING FRONTIER — the always-evaluated positions (the loop condition, a match
        // scrutinee, an always-run prefix). A trapping invariant in the frontier is hoisted (trap-
        // equivalent, since it runs on entry either way); one buried in a conditional branch is not.
        let mut frontier: std::collections::HashSet<StructId> = std::collections::HashSet::new();
        collect_dominating_frontier(db, body, &mut frontier);
        let mut hoist: Vec<StructId> = Vec::new();
        collect_hoistable(db, body, &inv_params, &frontier, &mut hoist);
        // Every DISTINCT node occurrence in the body, in first-seen order — the pool we scan for other
        // occurrences VALUE-EQUAL to a hoisted node (so a loop-invariant subexpression written in BOTH the
        // condition AND the body — `(if (< i (* n 2)) … (+ acc (* n 2)) …)` — shares the ONE hoist rather
        // than recomputing the body copy each iteration; the two `(* n 2)` are distinct StructIds but
        // `core_eq`). Counts are unused here; we only need the id list.
        let mut counts: HashMap<StructId, u32> = HashMap::new();
        let mut body_nodes: Vec<StructId> = Vec::new();
        collect_node_refs(db, body, &mut counts, &mut body_nodes);
        for node in hoist {
            // The hoisted value's machine slot. Skip anything without a machine rep (a heap-handle
            // invariant is fine — it is an i32 handle — but a rep-less type cannot be stashed).
            let Some(vt) = valtype_of(&type_of(db, node)) else {
                continue;
            };
            // Claim a PERSISTENT slot for the hoisted value at the body-scratch floor, and raise the floor
            // past it so the loop body's transient scratch never reuses it (the value must survive every
            // iteration). This mirrors how `which` reserves `base` for a mutual group.
            let slot = body_base;
            body_base += 1;
            high = high.max(body_base);
            scratch_ty.insert(slot, vt);
            // Emit the invariant computation ONCE (its own transient scratch floats above the reserved
            // slots, from `body_base`), store it, and register `(node → slot)` so every occurrence inside
            // the loop body reads the slot instead of recomputing.
            emit(
                db,
                node,
                &slot_of,
                body_base,
                &mut high,
                &mut scratch_ty,
                layout,
                &mut code,
            )?;
            code.push(Lir::LocalSet(slot));
            // Raise the body floor past ANY transient scratch the invariant's `emit` touched, not just the
            // persistent slot. A non-trivial hoisted invariant can spend its own scratch above `body_base`
            // (a checked `(+ n 1)` tees the sum into a guard slot to compare against `n` for overflow), and
            // that slot is recorded in `scratch_ty` at the invariant's width (i64). If the body then reused
            // it — a `match` scrutinee dispatch reuses the next free slot for the i32 bool discriminant —
            // the one wasm local would be declared at two widths and the module fails to validate
            // (`type mismatch: expected i32, found i64`). Mirrors the `let`-initializer floor at the `Let`
            // arm below. Only the persistent hoist slot must survive the loop; the guard scratch is dead
            // after the `local.set`, but its recorded TYPE forbids a width-changing reuse, so we skip past it.
            body_base = body_base.max(high);
            slot_of.insert(node, slot);
            // VALUE-NUMBER the hoist: point every OTHER body occurrence that is `core_eq` to this one (and
            // itself loop-invariant, so its value is identical every iteration) at the SAME slot. Without
            // this, a second textual copy of the invariant in the body (a distinct StructId) would
            // recompute it per iteration despite the hoist already holding the value. Sound: the slot holds
            // the value computed once before the loop from invariant params, and a `core_eq` invariant
            // occurrence denotes that same value on every iteration. Skip an already-slotted node (a nested
            // hoist / param) — it already reads a correct slot.
            for &m in &body_nodes {
                if m != node
                    && !slot_of.contains_key(&m)
                    && licm_invariant(db, m, &inv_params)
                    && core_eq(db, node, m)
                {
                    slot_of.insert(m, slot);
                }
            }
        }
    }
    if loops {
        let block_ty = match &ret {
            Ty::Unit => BlockType::Empty,
            other => match valtype_of(other) {
                Some(vt) => BlockType::Val(vt),
                None => return Err(Reject::decline("looped function result has no machine rep")),
            },
        };
        code.push(Lir::Loop(block_ty));
    }
    // DOMINATOR CSE: for a NON-looping, NON-mutual body, compute each shared scalar subexpression that is
    // ALWAYS EVALUATED (in the dominating frontier — the body if straight-line, or an `if` condition /
    // match scrutinee that runs before any branch) ONCE into a slot up-front, so `emit`'s node-keyed
    // `slots.get(&id)` fast path reads the slot at each use (in the cond AND both branches) instead of
    // re-emitting. `collect_cse_candidate_groups` requires a dominating member per class, so a value shared
    // only across branches is NOT hoisted (that would speculate work/a trap onto a path that skips it).
    // Skipped for a looping body (the loop transform owns its slots) and the mutual dispatch.
    if !loops && !mutual {
        for group in collect_cse_candidate_groups(db, body) {
            // A group is a VALUE-EQUIVALENCE class (all members `core_eq` — the same computation). Emit ONE
            // representative into a slot and point every member at it. Pick a representative NOT already
            // slotted (a member could be a sub-node of an earlier, larger class's representative that got
            // its slot first — its uses already read that slot).
            let Some(&rep) = group.iter().find(|&&m| !slot_of.contains_key(&m)) else {
                continue; // every member already reads a slot (nested in an earlier class) — nothing to do.
            };
            let Some(vt) = valtype_of(&type_of(db, rep)) else {
                continue;
            };
            let slot = body_base;
            body_base += 1;
            high = high.max(body_base);
            scratch_ty.insert(slot, vt);
            // Emit the representative's computation ONCE (transient scratch above the reserved slots). A
            // nested class was slotted earlier (inner-first), so this emit reads ITS slot — no recompute.
            // A CHECKED-ARITH rep writes into ITS OWN `$r` then needs a `local.get $r ; local.set slot`
            // move; route it through `emit_operand_into` (result dest = `slot`) so `$r` IS the slot and the
            // store is direct — no temp/copy (the same win as the arith-operand and let-binding paths).
            // Every other rep keeps `emit ; LocalSet` (byte-identical).
            let rep_int = match type_of(db, rep).strip_nominal() {
                Ty::Int(it) if it.width_is_fixed() => Some(*it),
                _ => None,
            };
            let arith_rep = rep_int.is_some()
                && matches!(
                    core_of(db, rep),
                    Core::Arith {
                        op: Prim::Add | Prim::Sub | Prim::Mul,
                        ..
                    }
                );
            if let Some(it) = rep_int.filter(|_| arith_rep) {
                emit_operand_into(
                    db,
                    rep,
                    it,
                    slot,
                    &slot_of,
                    body_base,
                    &mut high,
                    &mut scratch_ty,
                    layout,
                    &mut code,
                )?;
            } else {
                emit(
                    db,
                    rep,
                    &slot_of,
                    body_base,
                    &mut high,
                    &mut scratch_ty,
                    layout,
                    &mut code,
                )?;
                code.push(Lir::LocalSet(slot));
            }
            // Raise the scratch floor past ANY transient slot the rep's emit touched (not just the persistent
            // CSE slot), exactly like the LICM-hoist arm above. A rep with its OWN scratch — a const-divisor
            // `%`/`/` stashes the dividend `$a` at an i64 slot, a checked-arith tees a guard — records that
            // slot in `scratch_ty` at the rep's width. If a LATER allocation (the next CSE class, or the body
            // emit) reused it at a DIFFERENT width — the i32 Bool slot of a `(= (% s 2) 0)` element beside the
            // i64 `%` scratch, the tuple-`=` const-divisor miscompile — one wasm local would be declared at
            // two widths → `type mismatch: expected i32, found i64`, an invalid module. Skipping past `high`
            // hands every later slot a fresh, single-width local.
            body_base = body_base.max(high);
            // Point EVERY member of the class at this one slot — each occurrence, wherever it is in the
            // body, now reads the slot via `emit`'s node-keyed `slots.get(&id)` fast path instead of
            // recomputing. (Members already slotted keep their own slot — harmless; they are `core_eq` so
            // the value is identical, and re-inserting would only redirect a read to an equal value.)
            for &member in &group {
                slot_of.entry(member).or_insert(slot);
            }
        }
    }
    // The body is emitted in TAIL position: a `Core::Call` in the body's result position becomes a
    // `return_call` (or, in a looped function, a member call becomes a loop iteration). `emit_tail`
    // propagates tail-ness through `if`/`match`/`let` result positions and delegates every non-tail
    // position to `emit`.
    if mutual {
        // Dispatch on `which`: an if-chain over the members runs the one whose discriminant is current.
        // Each member's body runs at `depth = dispatch-if-nesting + 1` (the extra +1 is the loop).
        emit_mutual_dispatch(
            db,
            &loop_members,
            which_slot,
            &shared_slots,
            body_base,
            &mut high,
            &mut scratch_ty,
            layout,
            &mut code,
            tl.unwrap(),
        )?;
    } else {
        emit_tail(
            db,
            body,
            &slot_of,
            body_base,
            &mut high,
            &mut scratch_ty,
            layout,
            &mut code,
            tl,
        )?;
    }
    if loops {
        // Close the loop block. Control reaches here only via a non-looping tail leaf, which left the
        // result value on the stack — that value is the loop's (and the function's) result.
        code.push(Lir::End);
    }
    // Declare scratch slots `base..high` in slot order, each at its recorded type (default i64 for a slot
    // that was counted in the high-water mark but never explicitly typed — a defensive fallback).
    let declared: Vec<ValType> = (base..high)
        .map(|s| scratch_ty.get(&s).copied().unwrap_or(ValType::I64))
        .collect();
    peephole_emit(&mut code);
    // Named scalar locals (D3): the function's PARAMETERS (slots `0..n`, collected above) plus the
    // scalar `let`-bindings discovered during emit (`Emit::binding_local`). Both become `DW_TAG_variable`
    // DIEs, so a debugger can `print` an argument OR a local.
    locals.extend(code.binding_locals);
    Ok(SelectedFunc {
        params: param_vts,
        ret,
        code: code.code,
        declared,
        // The body occurrence is this function's source anchor for debug info (§2.1b).
        src_body: Some(body),
        // Scalar params + `let`-binding locals for debug-info variable inspection (§2.4, D3).
        locals,
        // Scalar match-binder lexical scopes (§2.4, D3) — a `DW_TAG_lexical_block` per match.
        scopes: code.match_scopes,
        // Per-construct source line markers (per-statement granularity), remapped through the peephole.
        stmt_lines: code.lines,
    })
}

/// A local peephole pass over the linearized body: fold `local.set N ; local.get N` (store then
/// immediately re-read the SAME local) into a single `local.tee N` (store AND leave the value on the
/// stack, one opcode). This is ALWAYS valid — `local.tee` is defined as exactly that set-then-leave —
/// so no liveness analysis is needed; the two forms have identical stack and local effects. The pattern
/// is emitted wherever a value is stashed into a scratch slot and read back immediately: a nested
/// checked op's result flowing into the enclosing op's operand slot (`… local.set $r_inner ;
/// local.get $r_inner ; local.set $a`), and a runtime `let` value stored then used. Block markers
/// (`If`/`Else`/`End`) are their own `Lir` entries, so "adjacent in the vec" means adjacent WITHIN a
/// block — a `local.get` that opens a different block never fuses with a `local.set` closing another.
///
/// This is the plain-`Vec<Lir>` fusion, kept as the unit-tested reference for the fusion RULE; the emit
/// path uses [`peephole_emit`] (same fusion, plus a remap of the debug line-table indices).
#[cfg(test)]
fn peephole(code: &mut Vec<Lir>) {
    let mut out: Vec<Lir> = Vec::with_capacity(code.len());
    let mut i = 0;
    while i < code.len() {
        if let Lir::LocalSet(n) = code[i]
            && let Some(Lir::LocalGet(m)) = code.get(i + 1)
            && n == *m
        {
            out.push(Lir::LocalTee(n));
            i += 2;
            continue;
        }
        out.push(code[i].clone());
        i += 1;
    }
    *code = out;
}

/// The peephole pass over an [`Emit`] — fuses `set;get`→`tee` in the code (as [`peephole`]) AND remaps
/// the debug `lines` indices, since a fusion shifts every later instruction down by one. Builds an
/// `old_index → new_index` map as it walks (both instructions of a fused pair map to the single `tee`'s
/// new index), then rewrites each line entry, so a `.debug_line` row still lands on the instruction it
/// names after the transform.
fn peephole_emit(emit: &mut Emit) {
    let old = std::mem::take(&mut emit.code);
    let mut out: Vec<Lir> = Vec::with_capacity(old.len());
    let mut remap: Vec<u32> = Vec::with_capacity(old.len());
    let mut i = 0;
    while i < old.len() {
        if let Lir::LocalSet(n) = old[i]
            && let Some(Lir::LocalGet(m)) = old.get(i + 1)
            && n == *m
        {
            let new_i = out.len() as u32;
            out.push(Lir::LocalTee(n));
            remap.push(new_i); // the `set` maps to the tee
            remap.push(new_i); // the fused `get` maps to the SAME tee
            i += 2;
            continue;
        }
        remap.push(out.len() as u32);
        out.push(old[i].clone());
        i += 1;
    }
    for (idx, _) in emit.lines.iter_mut() {
        // A marker whose only instructions all fused away clamps to the code end (a valid offset).
        *idx = remap
            .get(*idx as usize)
            .copied()
            .unwrap_or(out.len() as u32);
    }
    // Match-binder scope ranges shift with the same remap (an EXCLUSIVE end at `old.len()` maps to the
    // new code end). Both endpoints go through `remap`, keeping the range covering the same instructions.
    let remap_ix = |ix: u32| remap.get(ix as usize).copied().unwrap_or(out.len() as u32);
    for sc in emit.match_scopes.iter_mut() {
        sc.start_ix = remap_ix(sc.start_ix);
        sc.end_ix = remap_ix(sc.end_ix);
    }
    emit.code = out;
}

// ── LOOP-INVARIANT CODE MOTION (LICM) ────────────────────────────────────────────────────────────
//
// Once the loop transform has turned a tail-recursive function into a `loop`, a subexpression of the
// body that depends ONLY on loop-INVARIANT parameters (and constants) recomputes the SAME value every
// iteration — a waste, especially when it is a runtime CALL like `(List.len xs)` (a `vec-len` import) in
// the classic index loop `(if (< i (List.len xs)) …)`. LICM computes such a subexpression ONCE before
// the loop into a slot and reads the slot inside the body (via `emit`'s `slots.get(&id)` fast path).
//
// A parameter is loop-INVARIANT iff EVERY self-recursive back-edge (a member tail call) passes it back
// UNCHANGED — the exact `is_identity` test `emit_loop_iteration` already applies per arg. A subexpression
// is HOISTABLE iff it is (a) TRAP-FREE (`is_trap_free` — hoisting a trapping op ahead of a possibly-zero-
// iteration loop would introduce a trap the body ran conditionally/never), (b) INVARIANT (built only from
// invariant params + constants through pure operators — no call/effect/control-flow, no varying param or
// let-local), and (c) WORTH IT (a non-trivial computation, not a bare param/const, which are already free
// `local.get`/immediate). Only self-loops (a single member) are handled here — a mutual group shares
// slots across peers, so per-member invariance would need per-peer back-edge analysis (deferred).

/// The set of loop-invariant PARAMETER BINDERS of a self-loop: those a member tail call NEVER reassigns
/// (every self-call passes the parameter back to its own slot — the `is_identity` shape). Starts with ALL
/// params invariant and REMOVES any that some back-edge changes; a param not threaded identically on even
/// one edge is variant. `slots` maps each param binder to its slot; `param_slots[i]` is param `i`'s slot.
fn invariant_param_binders(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
) -> std::collections::HashSet<StructId> {
    // Begin optimistic: every parameter binder is invariant.
    let mut invariant: std::collections::HashSet<StructId> =
        params.iter().map(|(b, _)| *b).collect();
    let param_slots: Vec<u32> = params
        .iter()
        .map(|(b, _)| *slots.get(b).expect("param binder has a slot"))
        .collect();
    // Walk every SELF tail call (a back-edge) and demote any param its arg does not pass through unchanged.
    invalidate_varying_params(
        db,
        body,
        &param_slots,
        slots,
        members,
        self_def,
        &mut invariant,
        params,
    );
    invariant
}

/// Descend the TAIL positions (the same ones `emit_tail`/`tail_callees` thread) and, at each SELF tail
/// call, drop from `invariant` any parameter whose argument is not exactly its own identity pass-through
/// (`Core::Param{binder}` bound to the same slot). A non-self tail call (`return_call` to a peer/other
/// def) is NOT a back-edge of THIS loop for a single-member group, so it is not walked for invalidation —
/// but a single-member self-loop only has self back-edges anyway (`members == [self_def]`).
#[allow(clippy::too_many_arguments)]
fn invalidate_varying_params(
    db: &mut Db,
    id: StructId,
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
    invariant: &mut std::collections::HashSet<StructId>,
    params: &[(StructId, Ty)],
) {
    match core_of(db, id) {
        Core::Call { callee, args } if members.contains(&callee) => {
            // A back-edge: param `i` stays invariant only if arg `i` is its own identity pass-through.
            for (i, &arg) in args.iter().enumerate() {
                if i >= param_slots.len() {
                    continue;
                }
                let is_identity = matches!(core_of(db, arg), Core::Param { binder }
                    if slots.get(&binder) == Some(&param_slots[i]));
                if !is_identity {
                    invariant.remove(&params[i].0);
                }
            }
        }
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            invalidate_varying_params(
                db,
                then_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params(
                db,
                else_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        Core::Let { body, .. } => invalidate_varying_params(
            db,
            body,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        Core::Match { arms, .. } => {
            for arm in arms {
                invalidate_varying_params(
                    db,
                    arm.body,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
        Core::MatchList { arms, .. } => {
            for arm in arms {
                invalidate_varying_params(
                    db,
                    arm.body,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
        Core::MatchSum { root, .. } => invalidate_varying_params_sum(
            db,
            &root,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        _ => {}
    }
}

/// `invalidate_varying_params` over a sum decision tree — the `SumCont` analogue, descending the same
/// `Leaf`/`Guarded`/`LitTest`/`Switch` tail continuations `sum_cont_tail_callees` does.
#[allow(clippy::too_many_arguments)]
fn invalidate_varying_params_sum(
    db: &mut Db,
    cont: &crate::core::SumCont,
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    members: &[usize],
    self_def: usize,
    invariant: &mut std::collections::HashSet<StructId>,
    params: &[(StructId, Ty)],
) {
    match cont {
        crate::core::SumCont::Leaf(body) => invalidate_varying_params(
            db,
            *body,
            param_slots,
            slots,
            members,
            self_def,
            invariant,
            params,
        ),
        crate::core::SumCont::Guarded { body, els, .. } => {
            invalidate_varying_params(
                db,
                *body,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params_sum(
                db,
                els,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            invalidate_varying_params_sum(
                db,
                then_,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
            invalidate_varying_params_sum(
                db,
                els,
                param_slots,
                slots,
                members,
                self_def,
                invariant,
                params,
            );
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                invalidate_varying_params_sum(
                    db,
                    &arm.cont,
                    param_slots,
                    slots,
                    members,
                    self_def,
                    invariant,
                    params,
                );
            }
        }
    }
}

/// Whether the node at `id` is LOOP-INVARIANT given the set of invariant param binders — it is built
/// ONLY from invariant params and constants through PURE, side-effect-free operators. CONSERVATIVE: only
/// the enumerated pure scalar/collection-read variants qualify (arithmetic, comparison, conversion,
/// negation, a collection COUNT, a projection / sum-payload read); every other kind — a call, control
/// flow, a heap CONSTRUCTION, a `let`/`LocalRef` (a loop-varying local), a `Captured`/closure — is
/// treated as variant (returns false), so LICM never hoists something it cannot prove invariant. A bare
/// `Param` is invariant iff in the set; a `ConstInt`/`ConstBool`/`Unit` is always invariant.
fn licm_invariant(
    db: &mut Db,
    id: StructId,
    inv_params: &std::collections::HashSet<StructId>,
) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit => true,
        Core::Param { binder } => inv_params.contains(&binder),
        // Pure scalar operators — invariant iff every operand is.
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
            licm_invariant(db, lhs, inv_params) && licm_invariant(db, rhs, inv_params)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            licm_invariant(db, operand, inv_params)
        }
        // A collection COUNT / a projection / a sum-payload read is a pure borrowing read — invariant iff
        // the container is. (Its trap-freedom is decided separately by `is_trap_free`.)
        Core::ListLen { operand } | Core::BytesLen { operand } => {
            licm_invariant(db, operand, inv_params)
        }
        Core::MapSize { map } => licm_invariant(db, map, inv_params),
        Core::SetLen { set } => licm_invariant(db, set, inv_params),
        Core::Proj { operand, .. } => licm_invariant(db, operand, inv_params),
        Core::SumPayload { scrutinee, .. } => licm_invariant(db, scrutinee, inv_params),
        // Everything else — calls, control flow, heap builds, LocalRef (a loop-varying let), closures,
        // effects — is conservatively variant. LICM does not hoist it.
        _ => false,
    }
}

/// Whether a node is TRIVIAL to (re)materialize — a bare parameter or a constant. Such a node is already
/// a single `local.get` / immediate at each use, so hoisting it into a slot would only ADD a redundant
/// slot + move; LICM skips it and hoists only NON-trivial invariant computations.
fn licm_trivial(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::Param { .. } | Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit
    )
}

/// Collect the MAXIMAL hoistable subexpressions of a loop body: trap-free, loop-invariant, non-trivial
/// nodes, taking the OUTERMOST such node on each path (a maximal invariant subtree is hoisted as ONE
/// slot; its invariant sub-parts ride along inside it, needing no separate slot). Descends the body; at a
/// node that is hoistable it records the node and does NOT descend (maximal); otherwise it recurses into
/// the child positions that can CONTAIN a hoistable operand. Returns the node ids in DISCOVERY order
/// (deduplicated), so each is emitted once before the loop. Only pure/analyzable parents are descended —
/// which is sufficient because a hoistable node under an unanalyzed parent is still found when the walk
/// reaches it through the parent's enumerated child positions.
fn collect_hoistable(
    db: &mut Db,
    id: StructId,
    inv_params: &std::collections::HashSet<StructId>,
    frontier: &std::collections::HashSet<StructId>,
    out: &mut Vec<StructId>,
) {
    // A non-trivial INVARIANT node is a maximal hoist root when hoisting it before the loop adds no trap.
    // Two ways that holds:
    //   • it is TRAP-FREE — hoisting can add no trap regardless of position; OR
    //   • it is in the loop body's DOMINATING FRONTIER — an ALWAYS-EVALUATED position (the loop condition
    //     `(< i (* n 2))` runs on entry AND on every exit check, even for a 0-iteration loop). Such a node
    //     is evaluated ≥1 time whenever the loop is reached, so pulling it before the loop is TRAP-
    //     EQUIVALENT: a trapping invariant (a checked `(* n 2)`) traps on the first condition check either
    //     way. (A trapping invariant BURIED IN A BRANCH is NOT in the frontier — it might run zero times —
    //     so it stays put, keeping the `is_trap_free` guard for those.)
    // Record it and don't descend (maximal — its invariant sub-parts ride along in the one slot).
    if !licm_trivial(db, id)
        && licm_invariant(db, id, inv_params)
        && (crate::lower::is_trap_free(db, id) || frontier.contains(&id))
        // HEAP-HANDLE HOIST GUARD (Perceus soundness): a hoisted value is materialized ONCE before the loop
        // into a persistent slot and read back each iteration via `slots.get(&id)` — with the refcounts it
        // had at hoist time. That is correct for a SCALAR result (a count/index — copying an i64 is free and
        // rc-neutral). But a heap-HANDLE hoist root emits its dup/retain ONCE in the prologue, while the body
        // may CONSUME it (a `List.push`/`Bytes.concat`/`Map.insert` of the projected handle) once PER
        // ITERATION — so a single hoisted dup covers only the first consume; the second iteration consumes a
        // shared handle at rc==1 and FBIP-mutates it in place, and the loop-carried value DRIFTS. (Repro:
        // `(loop … pr … (List.len (List.push (. pr 0) 99)))` with `pr` a threaded tuple carrying the list —
        // per-iter len drifts 3,3,4,5,… .) A heap invariant that is only BORROWED in the body is safe, but
        // its maximal hoist root is then the enclosing SCALAR read (`List.len (. pr 0)` hoists as one i64
        // slot, the projection riding inside), so refusing a heap-TYPED root loses only the dangerous
        // handle-alone hoist, never the scalar borrow-read wins. A missed hoist is a slower loop, never wrong.
        && !is_heap_type(&type_of(db, id))
    {
        if !out.contains(&id) {
            out.push(id);
        }
        return;
    }
    // Otherwise descend the child positions that can hold a hoistable operand. Enumerated conservatively:
    // exactly the pure operator operands + the control-flow / match sub-positions + call args + the common
    // heap-op operands. An unlisted variant simply is not descended (a missed hoist, never a wrong one).
    for child in licm_children(db, id) {
        collect_hoistable(db, child, inv_params, frontier, out);
    }
}

/// The child occurrences of `id` LICM descends looking for hoistable operands — the operand positions of
/// the pure operators, the branches/arms of control flow, and the operands of calls/heap ops. Kept
/// deliberately broad on the READ side (finding a hoist under any parent is sound), but it never returns a
/// binder/pattern occurrence (only value positions). A variant not listed yields no children (its
/// subexpressions are simply not searched — a missed opportunity, never an unsound hoist).
fn licm_children(db: &mut Db, id: StructId) -> Vec<StructId> {
    match core_of(db, id) {
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::BytesConcat { lhs, rhs } => vec![lhs, rhs],
        Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::Proj { operand, .. }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::BytesCompact { operand } => vec![operand],
        Core::MapSize { map } => vec![map],
        Core::SetLen { set } => vec![set],
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => vec![scrutinee],
        Core::If {
            cond, then_, else_, ..
        } => vec![cond, then_, else_],
        Core::Let { body, bindings } => {
            // The bindings' INIT expressions are value positions (their binders are not); the body too.
            let mut v: Vec<StructId> = bindings.iter().map(|(_, init)| *init).collect();
            v.push(body);
            v
        }
        Core::Match { scrutinee, arms } => {
            let mut v = vec![scrutinee];
            v.extend(arms.iter().map(|a| a.body));
            v
        }
        Core::MatchList { scrutinee, arms } => {
            let mut v = vec![scrutinee];
            v.extend(arms.iter().map(|a| a.body));
            v
        }
        Core::Call { args, .. } => args,
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => elems,
        Core::ListPush { list, elem } => vec![list, elem],
        Core::ListAt { list, index, .. } => vec![list, index],
        // A variant with binders/patterns or an unanalyzed shape yields no searchable children.
        _ => vec![],
    }
}

// ── SHARED SUM-PAYLOAD-PREFIX CSE (per-arm-body) ──────────────────────────────────────────────────
//
// A match arm reading MULTIPLE elements of one payload tuple — `(Node (tuple l r))` binds `l` =
// `SumPayload{s, [Payload, Elem(0)]}` and `r` = `SumPayload{s, [Payload, Elem(1)]}` — re-walks the shared
// `sum-payload(s)` PREFIX per element (the two nodes are not `core_eq`, so the value-numbering CSE does not
// share them; only their prefix is common, and a prefix is a sub-PATH, not a `Core` node). This is the
// canonical AST-walker / linked-list-fold shape (`(Cons (tuple h t))`, `(Node (tuple l r))`).
//
// Fix: before emitting an arm body, compute each such shared prefix ONCE into a slot and record it (keyed
// by `(scrutinee-id, prefix step count)`); the `Core::SumPayload` emit then reads the slot and walks only
// the SUFFIX. SOUND: `op_sum_payload` is TOTAL (never traps — a mismatched node yields NULL, not a trap)
// and BORROWING (returns a handle from `handles.first()` with NO refcount change), so materializing it at
// the arm-body top is trap- and refcount-equivalent to the per-element re-walks, regardless of any control
// flow inside the arm body. Restricted to a prefix ending in `Payload` and shared by ≥2 `SumPayload` nodes
// that extend it with a BORROWING `Elem` step (an `arr-get`/`vec-get`, not a `RestFrom` `vec-drop`, which
// consumes) — so the materialized handle is only ever borrowed, never consumed.

/// Collect the shared SUM-PAYLOAD PREFIXES of `body` worth materializing: each returned
/// `(scrutinee, prefix)` is a path ending in `Payload` that ≥2 distinct `SumPayload` nodes in `body`
/// extend with a further `Elem` step (so both re-walk `<scrutinee>…prefix`). Walks the whole body
/// (through control flow — the arm body may nest `if`/`match`); groups by `(scrutinee, prefix)`.
fn collect_sum_payload_prefixes(
    db: &mut Db,
    body: StructId,
) -> Vec<(StructId, Vec<crate::core::PathStep>)> {
    // Every distinct SumPayload node in the body, as (scrutinee, path).
    let mut seen: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    let mut payloads: Vec<(StructId, Vec<crate::core::PathStep>)> = Vec::new();
    fn walk(
        db: &mut Db,
        id: StructId,
        seen: &mut std::collections::HashSet<StructId>,
        payloads: &mut Vec<(StructId, Vec<crate::core::PathStep>)>,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::SumPayload { scrutinee, path } = core_of(db, id) {
            payloads.push((scrutinee, path));
        }
        for child in licm_children(db, id) {
            walk(db, child, seen, payloads);
        }
    }
    walk(db, body, &mut seen, &mut payloads);
    // Tally each PREFIX (a path truncated after a `Payload` step) by how many payload nodes extend it with
    // a following `Elem`. `(scrutinee, prefix)` with a count ≥2 is a shared prefix worth hoisting.
    let mut counts: HashMap<(StructId, usize), usize> = HashMap::new();
    let mut key_path: HashMap<(StructId, usize), (StructId, Vec<crate::core::PathStep>)> =
        HashMap::new();
    for (scrutinee, path) in &payloads {
        // Consider every prefix `path[..k]` that ENDS in `Payload` and is FOLLOWED by an `Elem` (a
        // borrowing read). `RestFrom` never appears mid-path (it is a sole step), so a followed step is
        // always `Elem`/`Payload`; require `Elem` so the materialized prefix handle is only borrowed.
        for k in 1..path.len() {
            if matches!(path[k - 1], crate::core::PathStep::Payload)
                && matches!(path[k], crate::core::PathStep::Elem(_))
            {
                let key = (*scrutinee, k);
                *counts.entry(key).or_insert(0) += 1;
                key_path
                    .entry(key)
                    .or_insert_with(|| (*scrutinee, path[..k].to_vec()));
            }
        }
    }
    // The shared prefixes, unordered (the caller `materialize_payload_prefixes` sorts them shortest-first
    // so a nested prefix's walk can read a shorter already-materialized one).
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .filter_map(|(key, _)| key_path.remove(&key))
        .collect()
}

/// Materialize the shared SUM-PAYLOAD prefixes of an arm `body` into fresh slots (a per-arm-body CSE) and
/// register them in `out.payload_prefix_slots` keyed by `(scrutinee, prefix step count)`. Each prefix is
/// emitted ONCE (`<scrutinee> …prefix` — reusing a shorter already-registered prefix via the `SumPayload`
/// emit's fast path, since shorter prefixes are materialized first) and stored into its slot. Returns the
/// keys registered so the caller can REMOVE them after the arm body — fencing the slots to this arm so a
/// sibling arm never reads a payload its own scrutinee value did not produce. Slots are claimed from
/// `*high` upward (never `base`), so the arm body (which emits above `*high`) never clashes with them.
#[allow(clippy::too_many_arguments)]
fn materialize_payload_prefixes(
    db: &mut Db,
    body: StructId,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    slots: &HashMap<StructId, u32>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<Vec<(StructId, Vec<crate::core::PathStep>)>, Reject> {
    let mut prefixes = collect_sum_payload_prefixes(db, body);
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }
    // SHORTEST-first: a longer prefix's own walk then reads a shorter already-slotted prefix (the emit's
    // longest-matching-prefix fast path), so a nested payload chain materializes each level once.
    prefixes.sort_by_key(|(_, p)| p.len());
    let mut keys = Vec::new();
    for (scrutinee, prefix) in prefixes {
        let slot = *high;
        *high = slot + 1;
        scratch_ty.insert(slot, ValType::I32); // a payload handle is an i32
        // Emit the prefix as a BARE HANDLE WALK — `<start> …steps` with NO trailing unbox (`get_op`); a
        // prefix ends in `Payload`, so its value is a tuple/record HANDLE, used as-is. Start from the
        // longest ALREADY-registered shorter prefix if one exists (shortest-first order guarantees it is
        // materialized), else from the scrutinee. Only `Payload`/`Elem` steps appear in a prefix (a
        // `RestFrom` is a sole step, never followed → never in a prefix), and every `Elem` here is an
        // `arr-get` (a payload tuple, not a list — a list element would be a lone `Elem` off a list
        // scrutinee, not under a `Payload`).
        let start = (0..prefix.len()).rev().find_map(|k| {
            out.payload_prefix_slots
                .get(&(scrutinee, prefix[..k].to_vec()))
                .map(|&s| (k, s))
        });
        let from = if let Some((k, s)) = start {
            out.push(Lir::LocalGet(s)); // [handle] — the shorter shared prefix
            k
        } else {
            emit(
                db,
                scrutinee,
                slots,
                slot + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?; // [handle]
            0
        };
        for step in &prefix[from..] {
            match step {
                crate::core::PathStep::Payload => {
                    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                }
                crate::core::PathStep::Elem(i) => {
                    out.push(Lir::ConstI32(*i as i32));
                    out.push(Lir::CallImport(OP_ARR_GET));
                }
                crate::core::PathStep::RestFrom(_) => {
                    return Err(Reject::decline(
                        "a payload prefix cannot contain a RestFrom step",
                    ));
                }
            }
        }
        out.push(Lir::LocalSet(slot));
        let key = (scrutinee, prefix.clone());
        out.payload_prefix_slots.insert(key.clone(), slot);
        keys.push(key);
    }
    let _ = base;
    Ok(keys)
}

// ── STRAIGHT-LINE COMMON-SUBEXPRESSION ELIMINATION (CSE) ──────────────────────────────────────────
//
// β-reduction SHARES an argument occurrence at every parameter use site (`beta_reduce` returns the SAME
// `StructId`), so an inlined helper `(def (g s) (+ (+ s s) s))` applied to a non-trivial argument leaves
// the ONE argument node referenced multiple times in the reduced body. `emit` is then called once PER
// reference and re-emits the whole computation each time — `g (* a b)` emits `(* a b)` twice; a heap-
// building argument (`(len xs)` twice over `xs = (build …)`) rebuilds the list at each use. The intra-op
// arith-CSE (`core_eq` in `emit_checked_arith`) only shares the two operands of ONE op, so a node used
// across DIFFERENT ops (or ≥3 times) still duplicates.
//
// This pass computes such a shared node ONCE into a slot and reads the slot at each use (via `emit`'s
// node-keyed `slots.get(&id)` fast path — the same mechanism LICM / the match-scrutinee materialization
// use). It is deliberately SCOPED to the provably-sound subset:
//  • STRAIGHT-LINE body only (no `if`/`match` anywhere) — so every use of a shared node is unconditionally
//    executed; computing it up-front never speculates past a branch (no added trap, no branch-only heap
//    build hoisted, no refcount imbalance from a value live on only one path).
//  • TRAP-FREE shared node (`is_trap_free`) — computing it before the rest can add no trap.
//  • SCALAR result (a non-heap machine value) — a scalar has no refcount, so compute-once-read-N is
//    unconditionally sound; a heap handle would need dup/drop accounting per use (deferred).
//  • NON-TRIVIAL (`!licm_trivial`) — a bare param/const is already a free `local.get`/immediate.
// Emitted INNER-FIRST (smaller subtrees first) so a nested shared node's slot is registered before an
// enclosing shared node reads it.

/// Collect the DOMINATING FRONTIER of the body at `id` — the set of node occurrences that are ALWAYS
/// EVALUATED on entry, regardless of which branch any control flow takes. This is the emit-position
/// dominance set a CSE hoist to the top is sound against: a node in it runs before the rest of the body no
/// matter what, so computing it once up-front adds no work on any path and moves no trap (its trap, if any,
/// fires at the same first-occurrence point). The walk descends UNCONDITIONALLY-reached positions only: a
/// pure operator's operands and a `let`'s bindings+body are always evaluated, but an `If` conditionally
/// runs its branches — so descend ONLY its `cond` (always evaluated); likewise a `Match`/`MatchList`/
/// `MatchSum` runs only the selected arm, so descend ONLY its scrutinee. (A whole straight-line body is its
/// own frontier — no control flow prunes anything — so this subsumes the old `body_is_straight_line` gate.)
fn collect_dominating_frontier(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::HashSet<StructId>,
) {
    if !out.insert(id) {
        return; // already visited this occurrence
    }
    let unconditional: Vec<StructId> = match core_of(db, id) {
        // Control flow: only the DECIDING sub-value is always evaluated; the branches/arms are conditional.
        Core::If { cond, .. } => vec![cond],
        Core::Match { scrutinee, .. }
        | Core::MatchList { scrutinee, .. }
        | Core::MatchSum { scrutinee, .. } => vec![scrutinee],
        // Everything else `licm_children` enumerates evaluates ALL its children unconditionally (a pure
        // operator's operands, a `let`'s bindings + body, a call's args, a compound's elements).
        _ => licm_children(db, id),
    };
    for child in unconditional {
        collect_dominating_frontier(db, child, out);
    }
}

/// Walk the value-position tree at `id` (via `licm_children`), recording per-StructId a REFERENCE COUNT
/// (how many parent edges point at it — a node reached twice counts 2) into `counts`, and the distinct
/// StructIds in first-seen order into `order`. A shared subtree's interior is walked ONCE (the count above
/// captures the node's own multiplicity); descending per visit would over-count nested nodes / blow up on
/// a deep DAG.
fn collect_node_refs(
    db: &mut Db,
    id: StructId,
    counts: &mut HashMap<StructId, u32>,
    order: &mut Vec<StructId>,
) {
    let n = counts.entry(id).or_insert(0);
    *n += 1;
    if *n == 1 {
        order.push(id);
        for child in licm_children(db, id) {
            collect_node_refs(db, child, counts, order);
        }
    }
}

/// Collect the CSE candidate GROUPS of the body `id`: each returned `Vec<StructId>` is a VALUE-EQUIVALENCE
/// CLASS (all members pairwise `core_eq` — the SAME computation) of shareable, non-trivial, SCALAR nodes
/// whose TOTAL reference count across the class is ≥2 AND that has ≥1 member in the DOMINATING FRONTIER
/// (an always-evaluated position). The dominance requirement is what makes hoisting sound across control
/// flow: the class is computed anyway on entry (its dominating occurrence), so pulling it to a slot up-
/// front adds no work on any path and moves no trap — the other occurrences (in branches / anywhere) then
/// read the slot. `(if (> (* a b) 0) (* a b) (- 0 (* a b)))`: the `(* a b)` in the cond dominates, so the
/// two branch copies collapse to slot reads (3 muls → 1). A class shared ONLY across branches (no
/// dominating member) is NOT hoisted — that would speculate work / a trap onto a path that skips it.
/// Two sources of ≥2 refs both qualify (a single β-shared node ref'd twice, or distinct `core_eq`
/// occurrences), value-numbering unifies them. Groups INNER-FIRST (ascending representative subtree size)
/// so a nested class's slot is registered before an enclosing class's representative reads it.
fn collect_cse_candidate_groups(db: &mut Db, body: StructId) -> Vec<Vec<StructId>> {
    let mut counts: HashMap<StructId, u32> = HashMap::new();
    let mut order: Vec<StructId> = Vec::new();
    collect_node_refs(db, body, &mut counts, &mut order);
    let mut dominating: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    collect_dominating_frontier(db, body, &mut dominating);
    // Keep only the shareable / non-trivial / scalar distinct nodes (in first-seen order for determinism).
    let mut cands: Vec<StructId> = Vec::new();
    for id in order {
        if licm_trivial(db, id) || !is_cse_shareable(db, id) {
            continue;
        }
        let ty = type_of(db, id);
        if is_heap_type(&ty) || valtype_of(&ty).is_none() {
            continue;
        }
        cands.push(id);
    }
    // Partition into value-equivalence classes by `core_eq` (a small O(n²) pairwise scan — a body has few
    // CSE candidates). A distinct node joins the first class it is `core_eq` to.
    let mut classes: Vec<Vec<StructId>> = Vec::new();
    for id in cands {
        let mut placed = false;
        for class in classes.iter_mut() {
            if core_eq(db, class[0], id) {
                class.push(id);
                placed = true;
                break;
            }
        }
        if !placed {
            classes.push(vec![id]);
        }
    }
    // Keep a class iff (a) its TOTAL reference count (summing each distinct member's multiplicity) is ≥2 —
    // an actual repeat worth naming — AND (b) ≥1 member is in the DOMINATING FRONTIER (always evaluated),
    // so hoisting it to the top is sound on every path. INNER-FIRST by representative size so emitting a
    // class's representative reads any already-slotted nested class instead of recomputing.
    let mut groups: Vec<Vec<StructId>> = classes
        .into_iter()
        .filter(|c| {
            c.iter().map(|m| counts[m]).sum::<u32>() >= 2
                && c.iter().any(|m| dominating.contains(m))
        })
        .collect();
    groups.sort_by_key(|c| subtree_size(db, c[0]));
    groups
}

/// The number of nodes in the value-position subtree at `id` (via `licm_children`) — the CSE ordering key
/// (inner-first). A shared node is counted structurally; the absolute value only needs to be MONOTONE in
/// containment (a subtree is strictly larger than any subtree it contains), which this is.
fn subtree_size(db: &mut Db, id: StructId) -> u32 {
    1 + licm_children(db, id)
        .into_iter()
        .map(|c| subtree_size(db, c))
        .sum::<u32>()
}

/// Whether the node at `id` is a PURE, DETERMINISTIC SCALAR computation whose sharing is observably
/// identical to recomputing it — the UNARY analogue of the pairwise [`core_eq`] pure set (arith incl.
/// CHECKED `+`/`-`/`*`, compare, convert, not, proj, sum-payload, a nested pure `if`, or a leaf). A CALL,
/// a heap CONSTRUCT, control flow with an impure sub-part, an effect — anything else — is NOT shareable
/// (returns false). Used by straight-line CSE: sharing such a node computes it ONCE at the point that
/// dominates all its uses (the body is straight-line, so the first use dominates the rest), which
/// preserves its value AND its trap behavior — a trapping subexpression traps at the same first-occurrence
/// point whether shared or duplicated (the exact `core_eq` rationale). Distinct from `is_trap_free` (which
/// EXCLUDES a checked op because hoisting it past a BRANCH could add a trap): here there is no branch, so a
/// checked op is shareable too. NOTE: not restricted to scalar HERE — the caller applies the scalar filter
/// (`is_heap_type`); this predicate is purely about determinism/effect-freedom.
fn is_cse_shareable(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit | Core::Param { .. } => true,
        // A `let`-LOCAL reference is NOT shareable by this pass: its slot is established only when the
        // `let` binding is emitted INSIDE the body, but CSE hoists a candidate to BEFORE the body — so a
        // hoisted `(* k k)` over a let-local `k` would read an unbound slot ("let-binding reference has no
        // local slot"). Params (slots `0..n`, live up front) are fine; a let-local is excluded so its
        // enclosing subexpression is never hoisted. (The `let`-binding-level CSE — `should_keep_binding`
        // — already names a multiply-used let value; a computation OVER a let-local stays in place.)
        Core::LocalRef { .. } => false,
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
            is_cse_shareable(db, lhs) && is_cse_shareable(db, rhs)
        }
        Core::Convert { operand, .. } | Core::Not { operand } | Core::Proj { operand, .. } => {
            is_cse_shareable(db, operand)
        }
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) BORROWING read
        // returning a SCALAR (a `vec-len`/`bytes-len`/`champ-size` runtime import — no refcount change, no
        // effect, deterministic). Sharing two identical counts of the same collection is observably
        // identical to reading twice (same value, no trap), and the RESULT is a scalar so the caller's
        // `is_heap_type` filter admits it (we CSE the count, not the collection handle). The operand must
        // itself be shareable (a param handle / another shareable read) so the read is well-formed at the
        // hoist point. Mirrors `is_trap_free`'s treatment of these counts.
        Core::ListLen { operand } | Core::BytesLen { operand } => is_cse_shareable(db, operand),
        Core::MapSize { map } => is_cse_shareable(db, map),
        Core::SetLen { set } => is_cse_shareable(db, set),
        Core::SumPayload { scrutinee, .. } => is_cse_shareable(db, scrutinee),
        // A `List.at`/`Bytes.at` indexed read (`vec-get`/`bytes-get` after a bounds check) BORROWS the
        // sequence and is DETERMINISTIC — the same (list, index) yields the same element, no rc change on
        // the sequence, no effect. It produces an `Option` (a heap sum), so `ListAt`/`BytesAt` never
        // qualify as a CSE candidate THEMSELVES (the caller's `is_heap_type` filter drops them); they are
        // shareable only as the SCRUTINEE of a scalar-unwrapping `SumExpect` below. Both operands must be
        // shareable so the read is well-formed at the hoist point.
        Core::ListAt { list, index, .. } => {
            is_cse_shareable(db, list) && is_cse_shareable(db, index)
        }
        Core::BytesAt { bytes, index, .. } => {
            is_cse_shareable(db, bytes) && is_cse_shareable(db, index)
        }
        // `Map.lookup` (`map-lookup`) BORROWS the map and is DETERMINISTIC — the same (map, key) yields the
        // same result, no rc change on the map, no effect. It returns an `Option` (a heap sum), so like
        // `ListAt` it never qualifies as a CSE candidate itself (the caller's `is_heap_type` filter drops
        // it); it is shareable only as the SCRUTINEE of a scalar-unwrapping `SumExpect` — so a repeated
        // `(Option.expect (Map.lookup m k))` reading a scalar value shares ONE `map-lookup` (an O(log n)
        // CHAMP walk) instead of two. Both operands must be shareable so the read is well-formed at the
        // hoist point (the key is consumed into an owned temporary; a constant/param key qualifies).
        // (`Set.contains` returns a bare Bool but boxes its element into a fixed scratch slot the CSE hoist
        // can't relocate, so it does not share today — not admitted here to avoid a dead arm.)
        Core::MapLookup { map, key, .. } => is_cse_shareable(db, map) && is_cse_shareable(db, key),
        // `Option.expect`/`Result.expect` on a runtime sum (`SumExpect`) BORROWS its scrutinee and is a
        // deterministic unwrap-or-trap: the same present sum yields the same payload, and an absent one
        // traps — sharing preserves both (the CSE driver only hoists a class with a DOMINATING-frontier
        // member, so the trap fires at the same first-occurrence point whether shared or duplicated, the
        // standard checked-op CSE rationale). When the unwrapped payload is SCALAR (the common
        // `(Option.expect (List.at xs i))` reading an `Int64` element) the whole `SumExpect(ListAt …)` is a
        // scalar-valued borrowing read the caller's `is_heap_type` filter admits — so two identical such
        // reads share one bounds-check + `vec-get` + unbox instead of duplicating the ~20-instr sequence.
        // A heap-payload `SumExpect` is filtered out by the scalar gate, so this arm needs no type guard.
        Core::SumExpect { scrutinee, .. } => is_cse_shareable(db, scrutinee),
        Core::If {
            cond, then_, else_, ..
        } => {
            is_cse_shareable(db, cond) && is_cse_shareable(db, then_) && is_cse_shareable(db, else_)
        }
        _ => false,
    }
}

/// Whether the body at `id` makes a tail call to any def in `members` through the tail positions the
/// loop transform HANDLES — the body itself, an `if`'s two branches, a `let`'s body, or a `match`'s arm
/// bodies. NOT a non-tail position (an operand — that is a non-tail call). Mirrors `emit_tail`'s
/// propagation for exactly the `Call`/`If`/`Let`/`Match` cases so detection and emission agree. For a
/// plain self-loop `members = [self_def]`; for a mutual group it is every member (a tail call to any of
/// them iterates the shared loop).
fn body_has_member_tail_call(db: &mut Db, id: StructId, members: &[usize]) -> bool {
    match core_of(db, id) {
        Core::Call { callee, .. } => members.contains(&callee),
        Core::If { then_, else_, .. } => {
            body_has_member_tail_call(db, then_, members)
                || body_has_member_tail_call(db, else_, members)
        }
        Core::Let { bindings, body } => {
            // Match `emit_tail`: a `let` keeps its body's tail position only when no heap drop is pending
            // (a drop after the body would fall back to non-tail `emit`). A scalar-only `let` (the loop
            // shapes) has no drop, so this simply recurses the body.
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            !any_drop && body_has_member_tail_call(db, body, members)
        }
        // A `match`'s arm bodies are tail positions (the probe chain threads the loop context into each),
        // so a member tail-call in any arm makes the function loopable. (A guard is NOT a tail position —
        // it is a predicate evaluated before the body, so it is not considered here.)
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| body_has_member_tail_call(db, a.body, members)),
        // A LIST match's arm bodies are tail positions too — `emit_tail` threads the loop context into
        // each (a tail self-call in a `(list …)` arm iterates the loop), so a member tail-call in any arm
        // makes the function loopable. This is what lets a tail list fold `(sa xs acc) = (match xs ((list)
        // acc) ((list x .. rest) (sa rest (+ acc x))))` become a constant-stack loop.
        Core::MatchList { arms, .. } => arms
            .iter()
            .any(|a| body_has_member_tail_call(db, a.body, members)),
        // A SUM match's decision tree has tail positions at its LEAF/GUARDED bodies — `emit_tail` threads
        // the loop context into each (a tail self-call in a `(Succ m) → (count m …)` arm iterates the
        // loop), so a member tail-call in any leaf makes the function loopable. This is what lets a
        // tail-recursive sum-type consumer `(count n acc) = (match n ((Zero) acc) ((Succ m) (count m (+
        // acc 1))))` become a constant-stack loop.
        Core::MatchSum { root, .. } => sum_cont_has_member_tail_call(db, &root, members),
        _ => false,
    }
}

/// The `body_has_member_tail_call` recursion over a sum decision tree ([`SumCont`]): a `Leaf`/`Guarded`
/// BODY is a tail position (a member tail-call there loops); the `Guarded.els`, `LitTest.then_`/`els`, and
/// `Switch` arm continuations are the remaining sub-matrix, all in the same tail position, so recurse
/// through them. The guard `cond` / literal `probe` are predicates evaluated BEFORE the body, not tail
/// positions, so they are not considered.
fn sum_cont_has_member_tail_call(
    db: &mut Db,
    cont: &crate::core::SumCont,
    members: &[usize],
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => body_has_member_tail_call(db, *body, members),
        crate::core::SumCont::Guarded { body, els, .. } => {
            body_has_member_tail_call(db, *body, members)
                || sum_cont_has_member_tail_call(db, els, members)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_has_member_tail_call(db, then_, members)
                || sum_cont_has_member_tail_call(db, els, members)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_has_member_tail_call(db, &a.cont, members)),
    }
}

/// The def indices called in TAIL position from the body at `id` — the recursion edges the loop
/// transform can turn into a `br`. Descends exactly the tail positions `emit_tail` propagates through
/// (`if` branches, `let` body without a pending drop, `match` arms); a call in a NON-tail position (an
/// operand) is NOT a tail edge (it must stay a real call) and is skipped. This is the tail-call analogue
/// of `body_has_member_tail_call`, collecting the callees rather than testing one set.
fn tail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match core_of(db, id) {
        Core::Call { callee, .. } if !out.contains(&callee) => out.push(callee),
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            tail_callees(db, then_, out);
            tail_callees(db, else_, out);
        }
        Core::Let { bindings, body } => {
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            if !any_drop {
                tail_callees(db, body, out);
            }
        }
        Core::Match { arms, .. } => {
            for arm in arms {
                tail_callees(db, arm.body, out);
            }
        }
        Core::MatchList { arms, .. } => {
            for arm in arms {
                tail_callees(db, arm.body, out);
            }
        }
        Core::MatchSum { root, .. } => sum_cont_tail_callees(db, &root, out),
        _ => {}
    }
}

/// The `tail_callees` recursion over a sum decision tree ([`SumCont`]): collect the callees in TAIL
/// position (the `Leaf`/`Guarded` bodies), descending the same continuations `sum_cont_has_member_tail_call`
/// tests. The tail-call analogue of that predicate.
fn sum_cont_tail_callees(db: &mut Db, cont: &crate::core::SumCont, out: &mut Vec<usize>) {
    match cont {
        crate::core::SumCont::Leaf(body) => tail_callees(db, *body, out),
        crate::core::SumCont::Guarded { body, els, .. } => {
            tail_callees(db, *body, out);
            sum_cont_tail_callees(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_tail_callees(db, then_, out);
            sum_cont_tail_callees(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                sum_cont_tail_callees(db, &arm.cont, out);
            }
        }
    }
}

/// The wasm value types of def `d`'s parameters, in order — its machine SIGNATURE. `None` if any
/// parameter type has no machine representation (that def can't be a loop member). Two defs share a
/// signature (the requirement for a shared mutual loop, which reuses one set of parameter slots) iff
/// their `sig_valtypes` are equal.
fn sig_valtypes(db: &mut Db, d: usize) -> Option<Vec<ValType>> {
    crate::layout::def_params(db, d)
        .iter()
        .map(|(_, ty)| valtype_of(ty))
        .collect()
}

/// The TAIL-RECURSIVE LOOP GROUP that def `self_def` belongs to — the set of defs compiled into ONE
/// shared `loop`. Returns `[self_def]` for plain self-recursion (a single-member loop, no dispatch), a
/// LARGER set for a mutually-tail-recursive group of SAME-SIGNATURE functions (`even`/`odd`), or empty
/// when `self_def` is not tail-recursive at all (so it stays ordinary `return_call`s).
///
/// The group is the strongly-connected component of `self_def` in the TAIL-call graph, restricted to
/// members that (a) share `self_def`'s machine signature — the shared loop reuses one set of parameter
/// slots, so members must agree on arity and per-slot type — and (b) are reachable in a tail cycle back
/// to `self_def`. A def whose signature differs, or that only calls `self_def` NON-tail, is excluded (a
/// non-tail call must stay a real call; a differing signature can't share the frame). Deterministic:
/// members are returned with `self_def` first, the rest in ascending def order, so the emitted `which`
/// discriminants are stable across runs.
///
/// MEMOIZED across a whole GROUP: `select_function_of` calls this for EVERY def, and the body is a
/// double BFS over the tail-call graph (forward reach + a reach-back-to-self per member), so a group of
/// N mutually tail-recursive same-signature defs cost O(N²) per def → O(N³) over the group (measured:
/// 200 mutual defs = 687ms before this). Every member of one SCC produces the SAME member SET (differing
/// only in the `self_def`-first ordering), so the expensive set is computed ONCE and cached by the
/// group's canonical representative (its minimum member index) — the N members of a group then share
/// that one computation, and each derives its self-first order cheaply. Keying by `self_def` directly
/// would MISS (each def is queried once), so the cache keys on the SORTED set's min element.
fn mutual_loop_group(db: &mut Db, self_def: usize) -> Vec<usize> {
    let sorted = mutual_loop_members_sorted(db, self_def);
    // Reorder to this member's view: `self_def` first (it enters the loop at its own discriminant), the
    // rest ascending. `sorted` is already ascending, so this is a cheap rotate of `self_def` to front.
    if sorted.len() <= 1 {
        return sorted; // a plain self-loop (or empty) needs no reorder
    }
    let mut members = Vec::with_capacity(sorted.len());
    members.push(self_def);
    members.extend(sorted.iter().copied().filter(|&d| d != self_def));
    members
}

/// The SORTED member set of `self_def`'s tail-recursive SCC (ascending; empty if not a loop). Cached
/// PER MEMBER: since every member of one group produces the SAME sorted set, the first member to be
/// queried computes it (the O(N²) BFS) and then caches it for EVERY member of the group at once — so
/// the other N-1 members hit the cache and never recompute. That collapses the group's total cost from
/// O(N³) to O(N²) (one compute) + O(N) lookups. A non-loop def caches its own empty set.
fn mutual_loop_members_sorted(db: &mut Db, self_def: usize) -> Vec<usize> {
    if let Some(cached) = db.mutual_loop_cache.get(&self_def) {
        return cached.clone();
    }
    let sorted = mutual_loop_group_uncached(db, self_def);
    // Cache for EVERY member of the discovered group (they all share this set) — so a co-member queried
    // later is an O(1) hit, not another O(N²) BFS. A non-loop def (empty set) caches just itself.
    if sorted.is_empty() {
        db.mutual_loop_cache.insert(self_def, Vec::new());
    } else {
        for &m in &sorted {
            db.mutual_loop_cache.insert(m, sorted.clone());
        }
    }
    sorted
}

/// The uncached core — computes the SORTED SCC member set (ascending), see [`mutual_loop_group`] docs.
fn mutual_loop_group_uncached(db: &mut Db, self_def: usize) -> Vec<usize> {
    let Some(self_sig) = sig_valtypes(db, self_def) else {
        return Vec::new();
    };
    // Forward tail-reachability from `self_def`, staying within same-signature defs. A def enters the
    // frontier only if it shares the signature (else the edge can't be a shared-loop iteration).
    let mut reach: Vec<usize> = vec![self_def];
    let mut i = 0;
    while i < reach.len() {
        let d = reach[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if !reach.contains(&c) && sig_valtypes(db, c).as_ref() == Some(&self_sig) {
                reach.push(c);
            }
        }
    }
    // Keep only the members that tail-reach BACK to `self_def` (a genuine cycle) — the SCC. A def in
    // `reach` that never tail-calls back is a one-way tail callee (a helper `self_def` tail-calls but
    // which does not recurse into the group); it is not part of the loop and stays a `return_call`.
    // `self_def` is always in (it seeds the group; a lone `self_def` with a self-edge loops as before,
    // and even without one an empty group falls through to no-loop via the `loops` check upstream).
    let mut members: Vec<usize> = reach
        .iter()
        .copied()
        .filter(|&d| d == self_def || tail_reaches(db, d, self_def, &reach))
        .collect();
    // Deterministic order: `self_def` first (this function enters the loop at its own discriminant),
    // the rest ascending — so the emitted `which` discriminants are stable. (Discriminants are LOCAL to
    // each member function's own loop, so `self`-first differing per function is fine — control never
    // crosses between the two functions' loops.)
    members.sort_unstable();
    members.retain(|&d| d != self_def);
    members.insert(0, self_def);
    // A single member is a plain self-loop ONLY if it actually self-tail-calls; otherwise no loop.
    if members.len() == 1 {
        let body = match db.defs[self_def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        if body_has_member_tail_call(db, body, &members) {
            return members;
        }
        return Vec::new();
    }
    members
}

/// Whether def `from` tail-reaches `target` within the candidate set `within` (a path of tail calls,
/// each hop staying inside `within`). Used to keep only the SCC members in `mutual_loop_group`.
fn tail_reaches(db: &mut Db, from: usize, target: usize, within: &[usize]) -> bool {
    let mut seen: Vec<usize> = vec![from];
    let mut i = 0;
    while i < seen.len() {
        let d = seen[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if c == target {
                return true;
            }
            if within.contains(&c) && !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    false
}

/// The context for a SELF-TAIL-RECURSIVE function being compiled as a `loop`: which def index a tail
/// call must recognize as a loop iteration (`members` — the def indices compiled into this shared
/// loop), the SHARED parameter slots a tail call updates in place, the `which` local's slot (the state
/// variable a mutual group dispatches on — `None` for a plain SELF-loop, which needs no dispatch), and
/// the current branch `depth` from the loop (how many `if`/loop blocks enclose this position — the `br`
/// target). Threaded through `emit_tail`; `None` when the function neither self- nor mutually-loops, so
/// a tail call stays a `return_call`.
///
/// A plain self-tail-recursive function is the degenerate case `members = [self_def]`, `which = None`.
/// A mutually-tail-recursive group of same-signature functions (`even`/`odd`) shares ONE loop: each
/// member's function runs the loop entered at its own discriminant, and a tail call to ANY member sets
/// the shared params, sets `which` to that member's discriminant (its index in `members`), and `br`s to
/// the loop top — a branch, not a wasm call. A tail call to a def OUTSIDE `members` stays `return_call`.
#[derive(Clone, Copy)]
struct TailLoop<'a> {
    members: &'a [usize],
    param_slots: &'a [u32],
    which: Option<u32>,
    depth: u32,
}

impl TailLoop<'_> {
    /// The discriminant (index in `members`) of a tail-call callee that is a loop member, or `None` if
    /// the callee is not in this loop's group (so the call stays a `return_call`).
    fn member_which(&self, callee: usize) -> Option<usize> {
        self.members.iter().position(|&m| m == callee)
    }
}

/// Whether a match's arm bodies are in TAIL position (and, if so, the enclosing self-loop context so a
/// self-tail-call in an arm iterates the loop rather than emitting `return_call`). `NonTail` = an
/// ordinary value match (arm bodies emit via `emit`); `Tail(tl)` = a match in tail position (arm bodies
/// via `emit_tail`, threading `tl` — `None` inside `tl` means tail-but-not-self-recursive).
#[derive(Clone, Copy)]
enum TailPos<'a> {
    NonTail,
    Tail(Option<TailLoop<'a>>),
}

/// Emit the node at `id` in TAIL position — the body's result, whose value becomes the function's
/// return. A `Core::Call` here is emitted as `return_call` (a TAIL call: it replaces the caller's frame
/// rather than pushing a new one), so a tail-recursive loop runs in O(1) stack instead of trapping by
/// stack exhaustion at ~35k frames. When `tl` marks this function self-recursive, a SELF tail call is
/// instead compiled as an in-place LOOP iteration (update the parameter locals, `br` to the loop top) —
/// no wasm call frame per step. Tail-ness PROPAGATES through the result-producing sub-positions: an
/// `if`'s two branches, a `let`'s body (only when no heap `drop` must run AFTER it — a drop is code that
/// executes on return, so the call can't be the last instruction), and a `match`'s arm bodies. Every
/// other node (an operand, an operation, a plain value) is not a tail call, so it delegates to `emit`.
/// This mirrors `emit`'s structure for exactly the propagating cases; everything else is one delegation.
#[allow(clippy::too_many_arguments)]
fn emit_tail(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tl: Option<TailLoop>,
) -> Result<(), Reject> {
    match core_of(db, id) {
        // A tail call. When it targets a MEMBER of the loop group being compiled, iterate in place:
        // evaluate the new argument values, move them into the parameter locals, set `which` to the
        // callee's discriminant (mutual group only), and `br` back to the loop top — no call frame.
        // Otherwise it is a `return_call` (a real tail call: recursion to a def outside this loop group,
        // or a function not compiled as a loop at all).
        Core::Call { callee, args } => {
            if let Some(tl) = tl
                && let Some(which) = tl.member_which(callee)
                && args.len() == tl.param_slots.len()
            {
                emit_loop_iteration(
                    db, which, &args, tl, slots, base, high, scratch_ty, layout, out,
                )?;
                return Ok(());
            }
            emit_call_args(
                db, callee, &args, slots, base, high, scratch_ty, layout, out,
            )?;
            match layout.abs(callee) {
                Some(idx) => {
                    trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit TAIL call (return_call)");
                    out.push(Lir::ReturnCall(idx));
                    Ok(())
                }
                None => Err(Reject::decline(
                    "tail call to a definition with no emission index",
                )),
            }
        }
        // An `if` in tail position: its condition is not tail (a value the branch selects on), but BOTH
        // branches are — a tail call in either branch is the function's result.
        Core::If { cond, then_, else_ } => {
            let result = type_of(db, id);
            // FLOW-SENSITIVE DEAD-BRANCH ELIMINATION (see the non-tail arm): when the active refinement
            // decides this `if`'s condition, emit ONLY the taken branch — in TAIL position (so a tail call
            // in it stays a `return_call`/loop `br`). The condition is a trap-free refined comparison, so
            // dropping it preserves behavior.
            if let Core::Compare { op, lhs, rhs } = core_of(db, cond)
                && let Some(taken) = crate::lower::refined_comparison_const(db, op, lhs, rhs)
            {
                let branch = if taken { then_ } else { else_ };
                trace!(target: "rcdzc::select", node = id.0, taken, "tail if condition decided by branch refinement — emit only the taken branch");
                return emit_tail(db, branch, slots, base, high, scratch_ty, layout, out, tl);
            }
            // FLOW-SENSITIVE EQUAL-BRANCH COLLAPSE (see the non-tail arm): both branches reduce to the SAME
            // constant under their branch refinements + a trap-free cond → emit that constant (in tail
            // position). The emit-time analogue of `lower`'s `core_equiv(then, else)` fold.
            if crate::lower::is_trap_free(db, cond) {
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let tc = refined_const_value(db, then_);
                db.pop_range_refinements();
                if let Some(tc) = tc {
                    let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                    db.push_range_refinements(else_frame);
                    let ec = refined_const_value(db, else_);
                    db.pop_range_refinements();
                    if ec.as_ref() == Some(&tc) {
                        trace!(target: "rcdzc::select", node = id.0, "tail if with equal refined-constant branches → the constant");
                        let cid = crate::lower::synth_core(db, tc, result.clone());
                        return emit_tail(db, cid, slots, base, high, scratch_ty, layout, out, tl);
                    }
                }
            }
            // BRANCHLESS SELECT (see the non-tail `emit` arm for the full rationale): when both branches
            // are cheap trap-free scalar computations (`is_select_arm`) and the result is a non-heap
            // scalar, a `select` beats an `if`. A trap-free arm is never a tail call (a call is not
            // trap-free), so dropping the tail context here loses no `return_call`/loop-`br` — the whole
            // `if` becomes one value expression the caller consumes. (An exported body emitted in tail
            // position — `(def (f p a b) (if p a b))` — reaches HERE, not the non-tail arm, so the select
            // must be handled in both places.)
            // BOOLEAN MATERIALIZATION: `(if c 1 0)`/`(if c 0 1)` → the condition coerced to the result
            // width (a leaf `if` can reach tail position — an exported `(def (f p) (if p 1 0))` body).
            if let Some(r) = try_bool_materialization(
                db, cond, then_, else_, &result, slots, base, high, scratch_ty, layout, out,
            ) {
                return r;
            }
            if !matches!(result, Ty::Unit)
                && (!is_heap_type(&result) || ty_is_enum_disc(db, &result))
                && valtype_of(&result).is_some()
                && is_select_arm(db, then_)
                && is_select_arm(db, else_)
            {
                // An ENUM-DISC result is admitted alongside a scalar: its runtime rep IS an i32
                // discriminant (`valtype_of` = i32), and each enum-disc `select` arm emits just that
                // constant — no allocation, no drop — so `select` between two discriminants is as sound as
                // between two scalars (`(if c (Dir.North) (Dir.South))` = `(if c 0 1)` on the disc).
                // Each arm is emitted UNDER its branch-refinement frame (see the non-tail `Core::If` arm's
                // select block for the full rationale) — a `select` arm computes the same value the `if`
                // arm would, so a refinement that simplifies the arm (elides a redundant mask under a
                // proven range) must still apply. Sound: a trap-free arm has no guard to wrongly elide, the
                // taken arm's refinement holds, and the untaken arm's value is discarded regardless.
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let then_res = emit_branch(
                    db, then_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                then_res?;
                let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                db.push_range_refinements(else_frame);
                let else_res = emit_branch(
                    db, else_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                else_res?;
                emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Select);
                return Ok(());
            }
            emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
            // The branches start scratch ABOVE the high-water the COND reached, NOT at `base` — see the
            // non-tail `Core::If` arm for the full rationale: a cond that stashes an i32 HEAP HANDLE (a
            // runtime `value-eq`/`MatchSum` on constructed sums) types a slot for the whole function, and
            // a branch's i64 arith temp (`(if (= (mk n) (mk 3)) n (find (+ n 1)))`) reusing that slot
            // number at a different width fails validation. A scalar cond leaves `*high == base`, so this
            // is a no-op (byte-identical) for the common case.
            let branch_base = *high;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            // Inside the `if` block a self-loop `br` must jump one MORE level out to reach the loop top.
            let inner_tl = tl.map(|t| TailLoop {
                depth: t.depth + 1,
                ..t
            });
            // Each branch is TAIL (a tail call becomes `return_call`, a self-call a loop `br`), EXCEPT a
            // bare-literal branch, which must be GROUNDED to the `if`'s result width (a bare literal is
            // never a tail call, so grounding is safe): a default-Int64 literal opposite a narrow branch
            // would push a mismatched machine slot into the block. Ground via `emit_operand`, else emit
            // in tail pos.
            let emit_tail_branch = |db: &mut Db,
                                    b: StructId,
                                    bbase: u32,
                                    high: &mut u32,
                                    st: &mut HashMap<u32, ValType>,
                                    out: &mut Emit|
             -> Result<(), Reject> {
                if matches!(core_of(db, b), Core::ConstInt(_))
                    && let Ty::Int(rit) = &result
                {
                    emit_operand(db, b, *rit, slots, bbase, high, st, layout, out)
                } else {
                    emit_tail(db, b, slots, bbase, high, st, layout, out, inner_tl)
                }
            };
            // FLOW-SENSITIVE RANGE REFINEMENT (see the non-tail `Core::If` arm): push the branch's
            // condition-derived variable bound while emitting each branch, so a guard-elision check inside
            // sees the narrowed range (`(- n 1)` under `(> n 0)` sheds its underflow guard). Pop even on an
            // early `?` return. Fires here too because an exported/tail-position `if` reaches THIS arm.
            let base_frame = db.current_refinements();
            let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
            db.push_range_refinements(then_frame);
            let then_res = emit_tail_branch(db, then_, branch_base, high, scratch_ty, out);
            db.pop_range_refinements();
            then_res?;
            out.push(Lir::Else);
            // The else branch starts its scratch ABOVE the then branch's high-water, NOT back at
            // `branch_base`. The two branches are mutually exclusive, so REUSING slot indices would be sound
            // for a wasm STACK value — but a scratch slot's TYPE is recorded once in `scratch_ty`, and the
            // two arms can want the SAME index at DIFFERENT widths: a collection-carrying recursion's base
            // arm materializes a fallible-read Option HANDLE (i32) while its recursive arm's `(- n 1)` uses
            // an i64 temp — sharing `branch_base` sets one local at both types → the validator rejects it
            // (`expected i32, found i64`). Advancing past the then branch's `*high` hands the else branch
            // fresh, never-typed slots — the same disjoint-by-width discipline call args / tuple elements /
            // match arms already apply. (When the then branch used no scratch, `*high == branch_base`, so
            // this is byte-identical for the common scalar-`if`.)
            let else_base = branch_base.max(*high);
            let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
            db.push_range_refinements(else_frame);
            let else_res = emit_tail_branch(db, else_, else_base, high, scratch_ty, out);
            db.pop_range_refinements();
            else_res?;
            out.push(Lir::End);
            Ok(())
        }
        // A `let` in tail position: its body is tail — BUT only if no heap binding must be `drop`ped
        // AFTER the body. A drop is code that runs on the way out, so a `return_call` (which does not
        // return here) would skip it; when a drop is pending, fall back to the non-tail `emit` (the
        // body's call pushes an ordinary frame that returns, then the drops run). A body with no
        // pending drop (every heap binding escapes, or there are none) keeps the tail position.
        Core::Let { bindings, body } => {
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            if any_drop {
                return emit(db, id, slots, base, high, scratch_ty, layout, out);
            }
            // Re-emit the bindings exactly as `emit` does, then the body in TAIL position. (No drop
            // epilogue is needed — the `any_drop` check above guaranteed none.)
            let mut extended = slots.clone();
            let mut floor = base;
            for (binder, value) in &bindings {
                let slot = floor;
                let ty = type_of(db, *binder);
                let vt = valtype_of(&ty).ok_or_else(|| {
                    Reject::decline("a let binding's type has no machine representation")
                })?;
                // RESERVE the binding slot BEFORE the initializer emits — see the non-tail `Core::Let` arm:
                // the initializer emits at `slot + 1`, and its inner scratch floats off `*high`, so `*high`
                // must already cover the binding slot or a compound/`if` initializer reuses the binding's
                // own slot at the wrong width (the let-bound-if-tuple invalid-wasm miscompile).
                scratch_ty.insert(slot, vt);
                if slot + 1 > *high {
                    *high = slot + 1;
                }
                emit(
                    db,
                    *value,
                    &extended,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                // DEBUG (D3 locals): a SCALAR binding with a source name lives in this slot for its whole
                // scope — record it so a `DW_TAG_variable` DIE lets a debugger `print` the local. The
                // binder key is the initializer occurrence, so recover the name from its `(name init)`
                // pair (`let_binding_name`), not from the binder itself.
                if matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
                    && let Some(name) = db.let_binding_name(*binder)
                {
                    out.binding_local(slot, name.to_string(), ty.clone());
                }
                extended.insert(*binder, slot);
                // The body emits ABOVE both this binding slot AND any scratch the INITIALIZER used (its
                // transient slots are recorded in `scratch_ty` at a fixed TYPE; a body reusing one at a
                // different type would re-type a wasm local → invalid module — e.g. a runtime-`(bin …)`
                // scrutinee initializer uses an i64 `val` slot, and the match body reuses it as an i32).
                // `*high` tracks the top slot touched so far. For a scalar/handle initializer with no
                // scratch, `*high == slot+1`, so this is byte-identical to before.
                floor = (slot + 1).max(*high);
            }
            // A `let` adds no wasm block (its bindings are plain `local.set`s), so the loop-branch depth
            // is unchanged — the body's tail position is at the same nesting as the `let`.
            emit_tail(
                db, body, &extended, floor, high, scratch_ty, layout, out, tl,
            )
        }
        // A `match` in tail position: each arm body is tail. Delegated with a tail-aware arm emitter.
        Core::Match { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "match result type has no machine representation",
                        ));
                    }
                },
            };
            let it = int_ty_of(db, scrutinee);
            // The match's RESULT integer type (its arms' joined width), so a bare-literal arm body is
            // grounded to it (like an operand of a binary op) — otherwise an arm that is a default-Int64
            // literal beside an arm at a NARROW width would push a mismatched machine slot and wasm
            // rejects the block. `None` for a non-integer result (e.g. Bool arms — a ConstBool is always
            // i32, no width to reconcile).
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_match_arms_tailable(
                db,
                scrutinee,
                &arms,
                it,
                result_it,
                block_ty,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::Tail(tl),
            )
        }
        // A LIST match in tail position: dispatch by length, each ARM BODY in tail position (a self-tail
        // call in a `(list …)` arm becomes a `return_call` / loop iteration). Mirrors the scalar `Match`
        // arm — materialize the handle + `vec-len` once, then `emit_list_arms_tailable` with `Tail(tl)`.
        // Without this, `MatchList` fell through to non-tail `emit`, so a tail list fold never looped
        // (`(sa xs acc) = (match xs ((list) acc) ((list x .. rest) (sa rest (+ acc x))))` stack-recursed).
        Core::MatchList { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "list match result type has no machine representation",
                        ));
                    }
                },
            };
            let (arm_slots, len_slot, arm_base) = materialize_list_match_scrutinee(
                db, scrutinee, slots, high, scratch_ty, layout, out,
            )?;
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_list_arms_tailable(
                db,
                &arms,
                len_slot,
                block_ty,
                result_it,
                &arm_slots,
                arm_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::Tail(tl),
            )
        }
        // A SUM match in tail position: dispatch on the discriminant decision tree, each LEAF/GUARDED body
        // in tail position (a self-tail-call in a `(Succ m) → (count m …)` arm becomes a `return_call` /
        // loop iteration). Mirrors the non-tail `MatchSum` emit — materialize the scrutinee handle once (a
        // reusable param/local is re-read cheaply per probe; a computed scrutinee is stashed in a fresh
        // i32 slot so it is evaluated ONCE) — then `emit_sum_cont_tailable` with `Tail(tl)`. Without this,
        // `MatchSum` fell through to non-tail `emit`, so a tail-recursive sum consumer never looped (`(count
        // n acc) = (match n ((Zero) acc) ((Succ m) (count m (+ acc 1))))` stack-recursed).
        Core::MatchSum { scrutinee, root } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "sum match result type has no machine representation",
                        ));
                    }
                },
            };
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            // Same scrutinee discipline as the non-tail `MatchSum` emit: a reusable handle (a param/local
            // already in a slot) is re-read per probe; a computed one is materialized ONCE into a fresh i32
            // slot above the high-water so every re-read hits the slot (and its transient scratch never
            // clashes with the arm bodies at `base`).
            let (arms_slots, arms_base) = if reusable_handle_src(db, scrutinee, slots) {
                (slots.clone(), base)
            } else {
                let slot = *high;
                *high = slot + 1;
                scratch_ty.insert(slot, ValType::I32);
                emit(
                    db,
                    scrutinee,
                    slots,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                let mut m = slots.clone();
                m.insert(scrutinee, slot);
                (m, (*high).max(slot + 1))
            };
            emit_sum_cont(
                db,
                scrutinee,
                &root,
                result_it,
                block_ty,
                &arms_slots,
                arms_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::Tail(tl),
            )
        }
        // Everything else in tail position is an ordinary value (no tail call inside it) — emit normally.
        _ => emit(db, id, slots, base, high, scratch_ty, layout, out),
    }
}

/// Emit a member tail-call as a LOOP iteration: update the parameter locals with the new argument
/// values, set the `which` state local (for a mutual group) to the callee's discriminant, and `br` back
/// to the loop top — no wasm call frame. The new args are ALL evaluated onto the stack FIRST (each
/// reading the OLD parameter values), then popped into the param slots in REVERSE order (the stack is
/// LIFO, so the last-pushed arg is on top and stores into the last param). This is the standard parallel
/// move: it avoids the clobber where storing arg 0 into `$0` would corrupt a later arg that reads `$0`
/// (`sum(n-1, acc+n)` — arg 1 `acc+n` reads the OLD `n`, evaluated before `$0` is written). `which` is
/// set AFTER the params (its slot is above the params, never an arg source, so order is free). `tl.depth`
/// is the number of enclosing `if`/loop blocks, so `br depth` targets the loop top.
#[allow(clippy::too_many_arguments)]
fn emit_loop_iteration(
    db: &mut Db,
    which: usize,
    args: &[StructId],
    tl: TailLoop,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    trace!(target: "rcdzc::select", which, depth = tl.depth, args = args.len(), "emit member tail-call as loop iteration");
    // Evaluate each new argument value onto the stack, grounding a bare-literal arg to its OWN solved
    // width (unification already set it to the parameter's type at the call site — the same
    // reconciliation an operand/branch literal gets, so a default-Int64 literal into a narrow param slot
    // does not mismatch). All args are evaluated BEFORE any store, so each reads the OLD param values.
    //
    // Each arg after the first starts its scratch ABOVE the running high-water (`arg_base = *high`), so
    // sibling args never SHARE a scratch slot. All args are simultaneously live on the operand stack for
    // the parallel move, and a wasm local has ONE type — a later arg's i32 heap-match handle reusing an
    // earlier arg's i64 arith-guard slot (`(f (- n 1) (match <heap-Option> …))`) would force one slot to
    // two types and the module fails validation. `*high` is the max slot ever touched, so advancing to it
    // hands each arg fresh, never-typed slots (the `MatchSum` arm applies the same discipline internally).
    // IDENTITY-MOVE ELISION: an argument that is exactly the parameter it is stored back into — the
    // pass-through `(go (- n 1) k (+ acc k))` re-passes `k` to `k`'s own slot — is a no-op `local.get s ;
    // local.set s`. Since EVERY arg is read onto the stack BEFORE ANY store (the parallel move reads all
    // OLD param values first), such a slot keeps its old value throughout, so both the push and the store
    // can be dropped with no effect on the other args (they already read their sources onto the stack).
    // This strips the per-iteration self-move that a carried-through parameter (a limit/config/closure)
    // would otherwise run every loop. Guard `i < param_slots.len()` for safety (arg count matches the
    // callee's arity, so this always holds).
    let is_identity: Vec<bool> = args
        .iter()
        .enumerate()
        .map(|(i, &arg)| {
            i < tl.param_slots.len()
                && matches!(core_of(db, arg), Core::Param { binder }
                    if slots.get(&binder) == Some(&tl.param_slots[i]))
        })
        .collect();
    let mut arg_base = base;
    for (i, &arg) in args.iter().enumerate() {
        if is_identity[i] {
            continue; // pass-through to its own slot — no push, no store.
        }
        if let Core::ConstInt(_) = core_of(db, arg)
            && let Ty::Int(ait) = type_of(db, arg)
        {
            emit_operand(db, arg, ait, slots, arg_base, high, scratch_ty, layout, out)?;
        } else {
            emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?;
        }
        arg_base = *high;
    }
    // Pop the values into the parameter slots, last-arg-first (stack is LIFO). An identity-move slot was
    // never pushed, so it is not popped either — its old value stands.
    for (i, &slot) in tl.param_slots.iter().enumerate().rev() {
        if i < is_identity.len() && is_identity[i] {
            continue;
        }
        out.push(Lir::LocalSet(slot));
    }
    // For a mutual group, set the `which` state so the next iteration dispatches into the callee's body.
    // (A plain self-loop has one member, `which = None`, and skips this.)
    if let Some(w) = tl.which {
        out.push(Lir::ConstI32(which as i32));
        out.push(Lir::LocalSet(w));
    }
    // Jump to the loop top to iterate.
    out.push(Lir::Br(tl.depth));
    Ok(())
}

/// Emit the mutual-recursion DISPATCH inside the shared loop: an if-chain on the `which` state local
/// that runs the matching member's body in tail position. For k members, `k-1` `if`s test
/// `which == 0, 1, …` and the final `else` is the last member (its discriminant by elimination). Each
/// member body is emitted in TAIL position so a member tail-call inside it iterates the loop; the body
/// sits one `if` deeper than the position handed in, so the threaded `TailLoop.depth` bumps +1 per
/// enclosing dispatch `if` (mirroring how `emit_tail`'s `if` arm bumps depth). `tl.depth` on entry is
/// the loop-relative depth of the dispatch (0 — the loop is the immediately enclosing block).
#[allow(clippy::too_many_arguments)]
fn emit_mutual_dispatch(
    db: &mut Db,
    members: &[usize],
    which_slot: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tl: TailLoop,
) -> Result<(), Reject> {
    // Emit member `idx`'s body at branch-depth `depth` (loop-relative), then the rest as the `else` tail.
    fn emit_from(
        db: &mut Db,
        members: &[usize],
        idx: usize,
        which_slot: u32,
        slots: &HashMap<StructId, u32>,
        base: u32,
        high: &mut u32,
        scratch_ty: &mut HashMap<u32, ValType>,
        layout: &Layout,
        out: &mut Emit,
        tl: TailLoop,
        block_ty: BlockType,
    ) -> Result<(), Reject> {
        let member = members[idx];
        let body = db.defs[member]
            .body
            .ok_or_else(|| Reject::decline("a loop member has no body"))?;
        if idx + 1 == members.len() {
            // Last member — the unconditional tail (no probe; reached by elimination).
            return emit_tail(
                db,
                body,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                Some(tl),
            );
        }
        // `which == idx` ? run this member's body : fall through to the next. The body/else sit one `if`
        // deeper, so the loop `br` target grows by one.
        out.push(Lir::LocalGet(which_slot));
        if idx > 0 {
            out.push(Lir::ConstI32(idx as i32));
            out.push(Lir::I32Eq);
        } else {
            // `which == 0` is `i32.eqz` (one instruction; the discriminant 0 is the common entry).
            out.push(Lir::I32Eqz);
        }
        out.push(Lir::If(block_ty));
        let deeper = TailLoop {
            depth: tl.depth + 1,
            ..tl
        };
        emit_tail(
            db,
            body,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            Some(deeper),
        )?;
        out.push(Lir::Else);
        emit_from(
            db,
            members,
            idx + 1,
            which_slot,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            deeper,
            block_ty,
        )?;
        out.push(Lir::End);
        Ok(())
    }
    let ret = type_of(db, tl_body_of(db, members[0])?);
    let block_ty = match &ret {
        Ty::Unit => BlockType::Empty,
        other => match valtype_of(other) {
            Some(vt) => BlockType::Val(vt),
            None => return Err(Reject::decline("looped member result has no machine rep")),
        },
    };
    emit_from(
        db, members, 0, which_slot, slots, base, high, scratch_ty, layout, out, tl, block_ty,
    )
}

/// A loop member's body occurrence (helper for `emit_mutual_dispatch`'s block-type read).
fn tl_body_of(db: &Db, member: usize) -> Result<StructId, Reject> {
    db.defs[member]
        .body
        .ok_or_else(|| Reject::decline("a loop member has no body"))
}

/// The `call_indirect` TYPE-section index for applying the closure value at `closure` to `args` (at
/// FULL arity) — resolved by finding the lambda-lifted function whose `(env, params…) -> result`
/// signature matches the call's machine shape, and returning ITS functype's type index
/// (`layout.lifted_type_index`). The match is by MACHINE valtype: the lifted lambda must have exactly
/// `args.len()` params whose valtypes equal the call args' valtypes, and its result valtype must equal
/// the whole application's result valtype. Structural functypes mean any type index with the same shape
/// validates; using a matching lifted lambda's keeps it exact. `None` if no lifted lambda matches (a
/// runtime closure with no lifted body — e.g. a partial application / runtime currying, not yet built).
fn closure_type_index(
    db: &mut Db,
    closure: StructId,
    args: &[StructId],
    layout: &Layout,
) -> Option<u32> {
    // Each argument's machine valtype, in order — a `Unit` argument is ELIDED (it occupies no wasm slot,
    // pushes nothing, and the lifted lambda's Unit param is elided from its functype too), so it is
    // dropped here rather than making the whole collection `None`. A non-Unit arg with no machine rep
    // (should not reach a runtime application) makes the shape unrepresentable → `None` (caller declines).
    let mut arg_vts: Vec<crate::backend::wasm::lir::ValType> = Vec::new();
    for &a in args {
        let ty = type_of(db, a);
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        arg_vts.push(valtype_of(&ty)?);
    }
    let mut result_ty = type_of(db, closure);
    for _ in 0..args.len() {
        result_ty = match result_ty {
            Ty::Fn(_, r) => *r,
            _ => return None,
        };
    }
    // The application's result valtype — `None` for a `Unit` result, which crosses as a ZERO-RESULT
    // functype (the serializer emits a Unit-returning lifted lambda as `0x60 <params> <>`). A result
    // that is neither machine-repr NOR Unit is unrepresentable, so no type matches (the caller declines).
    let is_unit_result = matches!(result_ty, Ty::Unit);
    let rv = if is_unit_result {
        None
    } else {
        Some(valtype_of(&result_ty)?)
    };
    // A lifted lambda's result MATCHES this application's result shape — a Unit result matches a lift
    // whose own result is Unit (both zero-result functypes); a scalar result matches by valtype.
    let ret_matches = |l: &crate::lower::LiftedLambda| {
        if is_unit_result {
            matches!(l.ret_ty, Ty::Unit)
        } else {
            valtype_of(&l.ret_ty) == rv
        }
    };
    // A lifted lambda's REPRESENTED param valtypes (in order) — a `Unit` param is elided (it occupies no
    // wasm slot), mirroring the `arg_vts` elision above, so the two lists compare like-for-like.
    let lift_param_vts =
        |l: &crate::lower::LiftedLambda| -> Vec<crate::backend::wasm::lir::ValType> {
            l.params
                .iter()
                .filter(|(_, pt)| !matches!(pt.strip_nominal(), Ty::Unit))
                .filter_map(|(_, pt)| valtype_of(pt))
                .collect()
        };
    // Find a lifted lambda with the same represented-param valtypes (in order) + result shape.
    if let Some(slot) = layout
        .lifted
        .iter()
        .position(|l| lift_param_vts(l) == arg_vts && ret_matches(l))
    {
        return Some(layout.lifted_type_index(slot, layout.import_base));
    }
    // No lifted lambda supplies this shape — the applied closure is of a type NO `Core::Closure` in this
    // program builds (a statically-reachable but dynamically-dead `match` arm applying a variant's boxed
    // closure). `layout.closure_call_types` registered an EXTRA functype of the needed `(env:i32, args…)
    // ->result` shape; find it and use its type-section index. The lifted lambda's functype prepends an
    // i32 env, so the extra functype's params are `[i32, arg_vts…]` — match on that full param list. A
    // Unit result is a zero-result functype (`ret` is `Ty::Unit`), matched the same way as the lift path.
    let want_params: Vec<crate::backend::wasm::lir::ValType> =
        core::iter::once(crate::backend::wasm::lir::ValType::I32)
            .chain(arg_vts.iter().copied())
            .collect();
    let i = layout.closure_call_types.iter().position(|(pvts, ret)| {
        *pvts == want_params
            && if is_unit_result {
                matches!(ret, Ty::Unit)
            } else {
                valtype_of(ret) == rv
            }
    })?;
    Some(layout.closure_call_type_index(i, layout.import_base))
}

/// The MACHINE signature of a closure whose TYPE is `ty` — every curried parameter's valtype (in order)
/// and the final non-function result's valtype, peeling ALL arrows. `None` iff `ty` is not a function
/// type or any parameter/result has no machine representation. This is the type-level companion of a
/// lifted lambda's own signature ([`lifted_full_machine_sig`]): a runtime closure VALUE's machine shape
/// is exactly its lift's, so two closures share this signature iff one lift could produce a value of the
/// other's type. Used to decide whether a `Core::CallClosure` whose application arity finds no matching
/// lift is PROVABLY DEAD (no lift inhabits the operand's type) or merely UNSUPPORTED (a lift does, but
/// the application shape — a curried nested-unary lift applied at flattened higher arity — is one the
/// backend cannot lower).
fn ty_full_machine_sig(
    ty: &Ty,
) -> Option<(
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::lir::ValType,
)> {
    let mut params = Vec::new();
    let mut cur = ty.clone();
    while let Ty::Fn(p, r) = cur {
        params.push(valtype_of(&p)?);
        cur = *r;
    }
    if params.is_empty() {
        return None; // not a function type — no closure value lives here.
    }
    let rv = valtype_of(&cur)?;
    Some((params, rv))
}

/// A lifted lambda's FULL curried machine signature — every parameter's valtype (in order) THEN, if its
/// result is itself a function (a nested-unary lift `(fn a (fn x …))` returns a closure), that result's
/// parameters, ending at the first non-function result's valtype. So a 2-param sugar lift `(fn (a x) …)`
/// and a nested-unary `(fn a (fn x …))` of the same type both flatten to the identical `([i64,i64], i64)`
/// — a closure value's machine shape does not record HOW it was curried. Compared against a closure
/// operand's [`ty_full_machine_sig`] to test whether a lift can produce a value of the operand's type.
fn lifted_full_machine_sig(
    lift: &crate::lower::LiftedLambda,
) -> Option<(
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::lir::ValType,
)> {
    let mut params: Vec<crate::backend::wasm::lir::ValType> = lift
        .params
        .iter()
        .map(|(_, t)| valtype_of(t))
        .collect::<Option<_>>()?;
    match ty_full_machine_sig(&lift.ret_ty) {
        // The result is itself a function — extend with its curried params and take its final result.
        Some((rest, rv)) => {
            params.extend(rest);
            Some((params, rv))
        }
        // The result is a plain value — its valtype is the signature's result.
        None => Some((params, valtype_of(&lift.ret_ty)?)),
    }
}

/// Whether NO lifted lambda in `layout` could produce a runtime closure value of type `operand_ty` — the
/// operand's full curried machine signature matches no lift's. When true, a `Core::CallClosure` on an
/// operand of this type is PROVABLY DEAD: a closure value arises only from a lift, so an operand no lift
/// can inhabit holds no callable value and the application can never execute. Requires `operand_ty` to be
/// a representable function type (else `None` → not provably dead, so the caller declines rather than
/// silently emitting an `unreachable` for a shape it merely cannot represent).
fn closure_operand_is_dead(operand_ty: &Ty, layout: &Layout) -> bool {
    let Some(want) = ty_full_machine_sig(operand_ty) else {
        return false;
    };
    !layout
        .lifted
        .iter()
        .any(|l| lifted_full_machine_sig(l) == Some(want.clone()))
}

/// Whether the value at node `id` has an ENUM-DISCRIMINANT type — a C-style enum represented directly as
/// its discriminant `i32`, with no heap box (`Db::is_enum_disc`). Reads the node's SOLVED type, peels a
/// nominal wrapper (a nominal-over-enum shares the enum's representation), and asks the decl. A non-sum
/// (or a boxed mixed sum) is `false`, so every backend site can gate the unboxed path on this one query.
fn node_is_enum_disc(db: &mut Db, id: StructId) -> bool {
    let ty = crate::infer::type_of(db, id);
    ty_is_enum_disc(db, &ty)
}

/// Whether the SOLVED type `ty` is an enum-discriminant sum — the type-level companion of
/// [`node_is_enum_disc`], used where a type (a scrutinee's, an operand's) is in hand rather than a node.
fn ty_is_enum_disc(db: &Db, ty: &crate::ty::Ty) -> bool {
    match ty.strip_nominal() {
        crate::ty::Ty::Sum { decl, .. } => db.is_enum_disc(*decl),
        _ => false,
    }
}

/// The payload type of a sum's variant 0 (the shape a `Payload` path step descends into) — `None` for a
/// nullary or unresolvable variant. A helper for [`ty_at_path_recorded`]; reads the decl's first variant's
/// payload occurrences and decodes them (a single payload IS the type, multiple box as a tuple). Used only
/// as the FALLBACK for an unrecorded `Payload` step (the root switch, whose current type IS the scrutinee's
/// own — so variant 0 is correct there); a nested switch resolves the ACTUAL entered variant via the
/// recorded `sum_path_types`.
fn sum_single_payload_ty(db: &mut Db, sum: &crate::ty::Ty) -> Option<crate::ty::Ty> {
    let stripped = sum.strip_nominal().clone();
    let crate::ty::Ty::Sum { decl, .. } = &stripped else {
        return None;
    };
    let ctor = {
        let td = db.type_decl_by_occ(*decl)?;
        let v0 = td.variants.first()?;
        v0.ctor?
    };
    // Substitute the sum's ACTUAL type ARGS into the variant's generic payload: `Option Color`'s `Some`
    // payload is `Color`, NOT the unsubstituted parameter `?0`. `payload_ty_at_instantiation` unifies the
    // ctor's result (`Option ?a`) against the concrete scrutinee type, so a nested enum-disc payload
    // (`(Option Color)`) resolves to `Color` and `ty_is_enum_disc` sees it — without this, the payload
    // read as `?0` mis-selected `sum-disc` over the `get-int` a boxed enum-disc needs (invalid wasm).
    crate::infer::payload_ty_at_instantiation(db, ctor, &stripped)
}

/// The payload type of a sum's variant `disc` at THIS instantiation — the generalization of
/// [`sum_single_payload_ty`] (which is `disc == 0`) to ANY discriminant. A nested switch on a variant at
/// disc ≥ 1 (`(type Ast (Int Int64) (Name String) (List (List Ast)))` matched by `Ast.List([Ast.Name n,
/// ..])`) must read the payload of the ACTUAL entered variant (`List` → `List Ast`), not variant 0's (`Int`
/// → `Int64`). Recorded in `Emit::sum_path_types` as a switch descends, then read by the `Payload`-step
/// type resolution below. `None` for a nullary/unresolvable variant. Mirrors the Rust backend's
/// `variant_payload_ty`.
fn variant_payload_ty_at(db: &mut Db, sum: &Ty, disc: u32) -> Option<Ty> {
    let stripped = sum.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return None;
    };
    let ctor = {
        let td = db.type_decl_by_occ(*decl)?;
        td.variants.get(disc as usize)?.ctor?
    };
    crate::infer::payload_ty_at_instantiation(db, ctor, &stripped)
}

/// The type reached by a `Payload` step whose FULL path (from the root, INCLUDING this `Payload`) is
/// `prefix`, given the current sub-value type `cur`. Prefer the RECORDED entered-variant payload type in
/// `recorded` (keyed by the absolute path — written as an enclosing switch descended into a specific
/// variant); this is authoritative because it carries WHICH variant was entered, which the flat path alone
/// cannot. Fall back to variant 0 (`sum_single_payload_ty`) only when nothing is recorded (the root switch,
/// whose `cur` IS the scrutinee's type). A NOMINAL newtype `Payload` is a static unwrap to its inner type.
fn payload_step_ty(
    db: &mut Db,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<Vec<crate::core::PathStep>, Ty>,
) -> Ty {
    payload_step_ty_of(db, None, cur, prefix, recorded)
}

/// [`payload_step_ty`] with an optional SCRUTINEE node, so a `Payload` step whose entered variant was NOT
/// recorded (an enclosing `Switch` was FOLDED AWAY by the `known_disc` optimization — its emit never ran
/// `record_entered_payload_ty`) can recover the ACTUAL entered variant's payload type from the scrutinee's
/// CONSTANT value at this path, instead of falling back to VARIANT 0. When a switch is folded, the sub-value
/// at `prefix[..len-1]` is a compile-time `SumNew{disc}` (that is exactly what `const_at_path`/`known_disc`
/// proved to fold it), so its discriminant is known — and its payload type is `variant_payload_ty_at(sum,
/// disc)`, not variant 0's. Falling back to variant 0 read a nested self-recursive-sum payload at the wrong
/// depth (a `(W (I 7))` over `(type T (I …) (W T))` with a known outer `W` disc: the inner `I` payload was
/// resolved as `I`'s `Int64` from variant 0, erasing the second `Payload` step → a silent MISCOMPILE). Only
/// used where the scrutinee node is in scope (the emit walks); the type-only `payload_step_ty` keeps the
/// variant-0 fallback (its callers already thread `recorded` from an emitted switch, so a miss there is the
/// genuine root/variant-0 case).
fn payload_step_ty_of(
    db: &mut Db,
    scrutinee: Option<StructId>,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<Vec<crate::core::PathStep>, Ty>,
) -> Ty {
    if let Some(t) = recorded.get(prefix) {
        return t.clone();
    }
    match cur.strip_nominal() {
        Ty::Sum { .. } => {
            // Recover the entered variant from the scrutinee's CONSTANT value at the parent path (the box
            // this `Payload` unwraps). `prefix` ends in `Payload`; its parent is `prefix[..len-1]`.
            if let Some(s) = scrutinee
                && let Some(parent) = prefix.split_last().map(|(_, rest)| rest)
                && let Some(disc) = const_disc_at(db, s, parent)
                && let Some(pt) = variant_payload_ty_at(db, cur, disc)
            {
                return pt;
            }
            sum_single_payload_ty(db, cur).unwrap_or(Ty::Any)
        }
        inner => inner.clone(),
    }
}

/// The statically-known discriminant of the sub-value at `path` from `scrutinee`, when that sub-value is a
/// compile-time `Core::SumNew` (its tag is fixed even if its payload is a runtime value) — the backend twin
/// of `lower`'s `const_at_path` disc read. Walks `Payload`/`Elem` steps through constant `SumNew`/`Tuple`
/// cores; `None` at the first runtime step (then the caller keeps the variant-0 fallback, correct because a
/// runtime disc means an enclosing switch WAS emitted and recorded the type). Used only to repair a
/// folded-switch `Payload` type (see [`payload_step_ty_of`]).
fn const_disc_at(db: &mut Db, scrutinee: StructId, path: &[crate::core::PathStep]) -> Option<u32> {
    let mut cur = scrutinee;
    for step in path {
        // Mirror `lower::const_at_path`: an erased nominal `Payload` is a no-op; a boxed `SumNew` payload
        // unwraps to its single payload; a `Tuple`/`ListNew` `Elem` indexes.
        if matches!(step, crate::core::PathStep::Payload) && crate::infer::type_is_nominal(db, cur)
        {
            continue;
        }
        // A `Payload` step over a MULTI-payload `SumNew` is a NO-OP that lands on the payload TUPLE — the
        // following `Elem(i)` then indexes `payloads[i]` (the `(Elem, SumNew)` arm below). This mirrors the
        // RUNTIME walk (`sum-payload` yields the payload array, `arr-get i` indexes it). Without this a path
        // into a multi-payload variant's payload (`Payload` THEN `Elem`) hit the single-payload `len == 1`
        // guard, fell through to `None`, LOST the constant discriminant, and the caller defaulted to variant
        // 0 → a wrong-payload-depth miscompile (Copilot PR#457). A single-payload variant's path is just
        // `[Payload]` (no following `Elem`), so it still unwraps to `payloads[0]` in the arm below.
        if matches!(step, crate::core::PathStep::Payload)
            && let Core::SumNew { payloads, .. } = core_of(db, cur)
            && payloads.len() > 1
        {
            continue;
        }
        cur = match (step, core_of(db, cur)) {
            (crate::core::PathStep::Payload, Core::SumNew { payloads, .. })
                if payloads.len() == 1 =>
            {
                payloads[0]
            }
            (crate::core::PathStep::Elem(i), Core::Tuple { elems })
            | (crate::core::PathStep::Elem(i), Core::ListNew { elems })
            // A multi-payload variant's payloads: after the `Payload` no-op above, `cur` is the `SumNew`
            // and `Elem(i)` selects the i-th payload — the constant twin of `sum-payload` + `arr-get i`.
            | (crate::core::PathStep::Elem(i), Core::SumNew { payloads: elems, .. }) => *elems.get(*i)?,
            _ => return None,
        };
    }
    match core_of(db, cur) {
        Core::SumNew { disc, .. } => Some(disc),
        _ => None,
    }
}

/// Walk `path` from `root` to the sub-value's type, using `recorded` (the enclosing-switch entered-variant
/// payload types) to resolve each `Payload` step's variant — the type-only companion of the emit walk in
/// `push_discriminant`. Used to decide the discriminant REPRESENTATION (`sum-disc` vs a raw enum-disc i32)
/// at the sub-value. Falls back to variant 0 for an unrecorded `Payload` (the root). `Ty::Any` on a
/// malformed/unresolvable step (the caller then takes the safe boxed-sum path).
fn ty_at_path_recorded(
    db: &mut Db,
    root: &Ty,
    path: &[crate::core::PathStep],
    recorded: &HashMap<Vec<crate::core::PathStep>, Ty>,
) -> Ty {
    let mut cur = root.clone();
    let mut prefix: Vec<crate::core::PathStep> = Vec::with_capacity(path.len());
    for step in path {
        prefix.push(*step);
        cur = match step {
            crate::core::PathStep::Payload => payload_step_ty(db, &cur, &prefix, recorded),
            crate::core::PathStep::Elem(i) => match cur.strip_nominal() {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(e) => e.clone(),
                    None => return Ty::Any,
                },
                Ty::List(elem) => (**elem).clone(),
                _ => return Ty::Any,
            },
            crate::core::PathStep::RestFrom(_) => match cur.strip_nominal() {
                Ty::List(_) => cur.clone(),
                _ => return Ty::Any,
            },
        };
    }
    cur
}

/// Emit the scrutinee at `scrutinee`, walk `path` to the sub-value, and leave its DISCRIMINANT (an i32)
/// on the stack — the shared front of every sum switch/probe. A boxed sum reads `sum-disc`; an ENUM-DISC
/// sub-value carries its discriminant AS its representation, so at the top level (empty path) the emitted
/// i32 IS the discriminant (no op) and at a nested position it was boxed as an int, read back with
/// `get-int` (then narrowed to i32). This is the ONE place the discriminant-extraction representation
/// choice lives, so the br-table switch, the linear switch, and the `expect` probe all agree.
#[allow(clippy::too_many_arguments)]
fn push_discriminant(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let root = type_of(db, scrutinee);
    let sub = ty_at_path_recorded(db, &root, path, &out.sum_path_types);
    let sub_is_enum = ty_is_enum_disc(db, &sub);
    emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?;
    // Track the CURRENT sub-value's type as the walk descends so an `Elem` step picks the right accessor:
    // a tuple/record/sum-payload is a flat `arr` (`arr-get`), but a `List` is an RRB `vec` (`vec-get`). The
    // `Payload` step's variant is resolved from `sum_path_types` (recorded as the enclosing switch descended
    // into a specific variant) — falling back to variant 0 only at the root. A `Payload` into a non-variant-0
    // variant whose payload is a `List` (`Ast.List(List Ast)` matched by `Ast.List([Ast.Name n, ..])`) then
    // reads element 0 with `vec-get` (was `arr-get` on a vec — garbage disc, a silent mis-dispatch).
    let mut cur = root.clone();
    let mut prefix: Vec<crate::core::PathStep> = Vec::with_capacity(path.len());
    for step in path {
        prefix.push(*step);
        match step {
            crate::core::PathStep::Payload => {
                out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                cur = payload_step_ty_of(db, Some(scrutinee), &cur, &prefix, &out.sum_path_types);
            }
            crate::core::PathStep::Elem(i) => {
                out.push(Lir::ConstI32(*i as i32));
                if matches!(cur.strip_nominal(), Ty::List(_)) {
                    out.push(Lir::CallImport(OP_VEC_GET)); // list element → vec-get
                    cur = match cur.strip_nominal() {
                        Ty::List(e) => (**e).clone(),
                        _ => Ty::Any,
                    };
                } else {
                    out.push(Lir::CallImport(OP_ARR_GET));
                    cur = match cur.strip_nominal() {
                        Ty::Tuple(elems) => elems.get(*i).cloned().unwrap_or(Ty::Any),
                        _ => Ty::Any,
                    };
                }
            }
            crate::core::PathStep::RestFrom(_) => {} // never on a sum-disc path
        }
    }
    if sub_is_enum {
        // The sub-value is an enum-disc value. At the TOP level it is already the raw discriminant i32.
        // At a NESTED position (a non-empty path ending in a Payload/Elem read) it was boxed as an int, so
        // `get-int` recovers the i64 cell and `i32.wrap_i64` narrows it to the discriminant i32.
        if !path.is_empty() {
            out.push(Lir::CallImport(OP_GET_INT));
            out.push(Lir::I32WrapI64);
        }
    } else {
        out.push(Lir::CallImport(OP_SUM_DISC));
    }
    Ok(())
}

/// Emit a reference to the binder at wasm `slot` (a `Core::Param`/`Core::LocalRef` occurrence `id`). Reads
/// the persistent slot with `local.get`. If `id` is a RETAIN site (`collect_dup_sites` — this occurrence
/// CONSUMES the binding while it has a later live use), a `dup` (rc++) is emitted FIRST so the consuming op
/// spends a fresh reference and the binding's own reference survives for the later use. `dup` POPS its
/// argument and returns nothing, so it reads the slot itself (`local.get slot; dup`) — leaving the stack
/// unchanged — then the value is pushed for the consumer (`local.get slot`). A non-retain occurrence emits
/// the single `local.get`, byte-identical to before (the common case; `dup_sites` is empty for most bodies).
fn emit_binder_ref(id: StructId, slot: u32, out: &mut Emit) {
    if out.dup_sites.contains(&id) {
        out.push(Lir::LocalGet(slot));
        out.push(Lir::CallImport(OP_DUP)); // rc++ — pops this copy, returns nothing
    }
    out.push(Lir::LocalGet(slot));
}

/// Emit the flat instructions for the node at `id`, appending to `out`. `slots` maps a parameter's
/// name occurrence to its wasm local slot; `base` is the next free SCRATCH slot (a guarded op claims
/// `[base, base+1, base+2]` and recurses operands at `base+3`); `high` is the running high-water mark of
/// scratch slots used (so `select_function` declares exactly that many); `scratch_ty` records each
/// scratch slot's value type (so it is declared at the type it is set with). Exhaustive over `Core`.
#[allow(clippy::too_many_arguments)]
fn emit(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A node MATERIALIZED into a scratch slot reads back as a `local.get`, not a recomputation. A
    // sum-match evaluates a non-reusable scrutinee ONCE into a slot (`emit_sum_match_arms`) and records
    // `(scrutinee-id → slot)` here, so every per-probe / per-payload re-reference of the scrutinee reads
    // the slot instead of rebuilding the value (which for a `List.at`/call scrutinee would both recompute
    // AND collide with arm-body scratch — an invalid module). Keyed by the node's OWN occurrence, distinct
    // from the binder-keyed `Param`/`LocalRef` entries, so this never shadows a binding.
    if let Some(&slot) = slots.get(&id) {
        out.push(Lir::LocalGet(slot));
        return Ok(());
    }
    // DEBUG (per-construct line rows): mark this node's first instruction with its source occurrence,
    // so a `.debug_line` row maps the code offset to its line. Recorded for every USER node (a prelude/
    // synthesized node has no span; `Emit::mark` dedups a repeated offset). The operand-consuming
    // helpers (`emit_operand`/`emit_checked_arith`/…) mark their own child ids too, so a construct whose
    // operands this `emit` never re-enters (an inline `Param`/`ConstInt`) still gets attributed there.
    if db.is_user_node(id) {
        out.mark(id);
    }
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // A CONSTANT typed `BigInt` reaching `emit` is used as a RUNTIME VALUE (a map key/value, a set
            // element, a call argument, an op operand) — every such context wants an i32 HANDLE, not a raw
            // `i64.const`. A folded `(BigInt.of <const>)` is a `Core::ConstInt` typed BigInt; emitting it
            // as a bare scalar pushes an i64 where a handle is expected → an invalid module (the map-key /
            // call-arg miscompiles). MATERIALIZE it as a fresh BigInt leaf via `bigint-of-i64` (the value
            // fits i64 — a beyond-i64 constant BigInt is not yet built and declines earlier). A CONSTANT
            // BigInt that is a whole nullary EXPORT takes the baked-bytes `constant_value_form` path and
            // never reaches here; this is only the in-body runtime-value use.
            if is_bigint_valued(db, id) {
                // Materialize the constant as a heap leaf: fits-i64 via `bigint-of-i64`, beyond-i64 via
                // `bigint-of-bytes` on its baked canonical sign-magnitude bytes (`emit_const_bigint_leaf`).
                // `is_bigint_valued` also fires for a BigInt-inner quantity (`(Qty.of (BigInt.of k) u)`) —
                // it erases to this ConstInt and equally needs the handle, not a raw `i64.const`.
                emit_const_bigint_leaf(&v, out);
                return Ok(());
            }
            // Ground the literal to the machine width its solved type fixes. The constant must FIT the
            // width (checked at annotation time; a value that does not fit never reaches here for an
            // annotated literal), then it is emitted as the two's-complement BIT PATTERN of that width.
            // For an UNSIGNED value at/above the signed max (`UInt64.max = 2^64-1`), the bit pattern is
            // a negative i64 (`-1`) — correct at the machine level; the boundary lifts it back as u64.
            let it = int_ty_of(db, id);
            let width = it.ground_width();
            trace!(target: "rcdzc::select", node = id.0, signed = it.ground_signed(), width, "ground integer literal to its machine width");
            if !v.fits_width(it.ground_signed(), width) {
                trace!(target: "rcdzc::select", node = id.0, width, "literal does not fit its width (CDZ0302)");
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            if width <= 32 {
                out.push(Lir::ConstI32(v.to_i32_bits(width)));
            } else {
                out.push(Lir::ConstI64(v.to_i64_bits()));
            }
            Ok(())
        }
        Core::ConstBool(b) => {
            out.push(Lir::ConstI32(if b { 1 } else { 0 }));
            Ok(())
        }
        // A constant string reaching `emit` as an in-body runtime VALUE — a string used where a heap
        // handle is needed (a map KEY/VALUE, a boxed element, a value threaded to a consumer), NOT folded
        // away. Build it as a FLAT UTF-8 BYTE LEAF, exactly as `Core::BytesOf` builds a byte sequence:
        // `bytes-alloc(len)` then a `bytes-set` per byte. This is BYTE-IDENTICAL to the runtime `str-new`
        // op (`op_str_new(s) = alloc(Vec::new(), s.into_bytes())` — a flat arity-0 leaf whose raw is the
        // UTF-8 bytes), so a String value built here is CANONICAL and `champ_eq`/`value-eq` compares two
        // equal strings correctly (raw-byte compare). We build via the lowerable `bytes-*` ops rather than
        // `str-new` because `str-new` takes a component-model `string` (canonical-ABI ptr+len+realloc), so
        // it is NOT callable from a core function body (`lowerable: false`); the byte-leaf build reaches the
        // identical rep with core-scalar ops. The reader already NFC-normalized the string, so the bytes
        // are canonical. (A CONSTANT string still folds in `lower` — a `= "a" "a"` never reaches here; this
        // is the path for a string that must become a runtime handle, e.g. `(map ("a" 1))`'s key.)
        Core::ConstStr(s) => {
            let bytes = s.as_bytes();
            out.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf] (bytes-set returns the buffer)
            }
            Ok(()) // leaves [buf] — the string's flat UTF-8 byte-leaf handle (== str-new's rep)
        }
        // A constant char reaching `emit` as an in-body VALUE has no runtime slot form yet — its
        // equality/ordering FOLD in `lower` (never reaching here), and it does not yet cross the boundary.
        // So a char value used inside a body declines cleanly (the scalar runtime rep is a later increment).
        Core::ConstChar(_) => Err(Reject::decline(
            "a runtime char value is not yet built (only a constant char folds; boundary crossing is later)",
        )),
        // A constant `Rational` reaching `emit` as an in-body RUNTIME VALUE (a call arg, a map/set element,
        // an operand of a runtime rational op) MATERIALIZES to a runtime rational node: box each component
        // (num, den) as a BigInt leaf via `bigint-of-i64`, then `rational-of` (which consumes the two
        // handles + normalizes — the pair is already normalized, so this is idempotent). Both components
        // fit i64 for a materializable constant; a component beyond i64 declines (the arbitrary-magnitude
        // rational-component leaf is a later slice — no current case builds one). The whole-export constant
        // Rational takes the baked-bytes `constant_value_form` path and never reaches here — this is the
        // in-body runtime-value use, the analogue of the `Core::ConstInt`-typed-BigInt materialization.
        Core::ConstRational(n, d) => match (n.to_i64(), d.to_i64()) {
            (Some(nv), Some(dv)) => {
                out.push(Lir::ConstI64(nv));
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big]
                out.push(Lir::ConstI64(dv));
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big, den-big]
                out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle]
                Ok(())
            }
            _ => Err(Reject::decline(
                "a constant Rational with a component beyond i64 is not yet materialized at run time",
            )),
        },
        // The canonical NaN emits an `f64.const`/`f32.const` of the canonical NaN bit pattern at the
        // node's solved width — the same machine-slot value a returned NaN leaves on the stack (a NaN is a
        // real Float value that crosses the boundary, unlike a char). `f32::NAN`/`f64::NAN` are the one
        // canonical quiet NaN, matching the fold's `to_f64_bits` comparison basis.
        Core::ConstFloatNan => {
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            if width == 32 {
                out.push(Lir::F32ConstBits(f32::NAN.to_bits()));
            } else {
                out.push(Lir::F64ConstBits(f64::NAN.to_bits()));
            }
            Ok(())
        }
        // A float CONSTANT emits an `f64.const`/`f32.const` of its canonical bit pattern at the node's
        // SOLVED width — the value a float occupies in its machine slot, and what an export returning a
        // float leaves on the stack (the boundary lifts it to the component `f64`/`f32`). A `Float32`
        // constant rounds the exact `Decimal` through binary32 (`as f32`) and emits `f32.const`. The width
        // is read off the solved type (the same read the boundary valtype uses).
        Core::ConstFloat(d) => {
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            if width == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                out.push(Lir::F32ConstBits(bits));
            } else {
                out.push(Lir::F64ConstBits(d.to_f64_bits()));
            }
            Ok(())
        }
        Core::Unit => {
            // Unit occupies no slot and pushes nothing.
            Ok(())
        }
        // A runtime RECORD — the SAME positional heap array as a tuple, its fields in canonical
        // (key-sorted) order (the `BTreeMap` iteration order, which the value-form renderer and every
        // `arr-get` index agree on). Field names are compile-time information the runtime does not hold;
        // at run time a record IS a tuple. So build it exactly as a tuple: `arr-alloc(n)`, then each
        // field value boxed by its type and `arr-set` into its sorted position. Leaves the record's u32
        // handle on the stack. (A record consumed only to read a field folds away in `lower`; a record
        // that survives to selection is a genuine runtime value — e.g. one that escapes to the host.)
        Core::Record { fields } => {
            out.push(Lir::ConstI32(fields.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            for (i, (_, &value)) in fields.iter().enumerate() {
                // Each field starts its scratch ABOVE the running high-water, NOT at a fixed `base` — the
                // same disjoint-slot discipline `Core::Tuple`/`Core::ListNew` apply. A field initialized by
                // a checked-arith op stashes an i64 into a scratch slot; if a sibling (or an enclosing
                // call-boundary) slot at that number was already typed i32, reusing it re-types one wasm
                // local to two widths → an invalid module (`expected i64, found i32`). This is the
                // composed-call miscompile: `(f (f (record (a (+ (. r a) 1)) (b (. r b)))))` had field
                // `a`'s i64 arith `$r` collide with a record-assembler i32 slot. Advancing `field_base`
                // past each field's high-water hands each field fresh, never-typed slots.
                let field_base = base.max(*high);
                // [arr] ; push index ; push (box, if scalar) the field value ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, value, slots, field_base, high, scratch_ty, layout, out)?; // [arr, i, value]
                // A scalar element boxes to a handle (a NARROW int first extends i32→i64, as box-int
                // takes an i64 cell); a nested compound is ALREADY a u32 handle → `arr-set` it directly;
                // a UNIT field pushed nothing → its slot holds the inline-unit sentinel.
                let boxed = box_op(db, value)?;
                emit_heap_store_tail(db, value, boxed, out); // [arr, i, handle]
                // Canonicalize a rope-capable String/Bytes field to a flat leaf on construction (see the
                // `Core::Tuple` arm) — a record IS a tuple at run time, so the same nested-rope face.
                if elem_needs_rope_compaction(db, value) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            Ok(()) // leaves [arr] — the record handle
        }
        // A runtime TUPLE — build it on the value heap: `arr-alloc(n)` leaves the array handle on the
        // stack, then for each element push `(handle, index, boxed-elem)` and `arr-set` (which returns
        // the handle, threading it to the next element). The handle stays on the operand stack across
        // elements — no scratch local — because `arr-set` returns it. Each element is BOXED to a u32
        // handle by its type (`box-int`/`box-bool`); the tuple itself is a u32 handle.
        Core::Tuple { elems } => {
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            // Each element starts its scratch ABOVE the high-water the PREVIOUS elements reached, NOT at a
            // fixed `base`. An element that stashes a value in a scratch slot at a given TYPE (a
            // `SumExpect`/match materializing an i32 heap handle) fixes that slot's declared type; a LATER
            // element reusing the same slot number at a DIFFERENT width (`(+ i 1)` → i64) would re-type it,
            // an invalid module (`expected i64, found i32`). Advancing `elem_base` past each element's
            // high-water keeps sibling elements on disjoint slots. (A scalar element leaves `*high` where it
            // was, so this is a no-op for the common all-scalar tuple — byte-identical there.)
            for (i, &elem) in elems.iter().enumerate() {
                let elem_base = *high;
                // [arr] ; push index ; push (box, if scalar) the element ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [arr, i, elem]
                // A scalar element boxes (a NARROW int extends i32→i64 first, box-int takes i64); a
                // nested compound is ALREADY a u32 handle → `arr-set` it directly, no box; a UNIT element
                // pushed nothing → its slot holds the inline-unit sentinel.
                let boxed = box_op(db, elem)?;
                emit_heap_store_tail(db, elem, boxed, out); // [arr, i, handle]
                // CANONICALIZE a rope-capable String/Bytes element to a flat leaf on construction (the
                // nested-leaf twin of the `op_box_float` normalize-on-construct + the top-level `=`
                // compaction), so the tagless `champ_eq`/`champ_hash` walk compares a nested string by
                // content, not rope-physical bytes. rc-neutral (see `elem_needs_rope_compaction`).
                if elem_needs_rope_compaction(db, elem) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            Ok(()) // leaves [arr] — the tuple handle
        }
        // A runtime LIST — BULK BUILD: lay the elements into a flat `arr` (exactly as a tuple:
        // `arr-alloc N` + a boxed `arr-set` per element), then ONE `vec-of-arr` to turn that array into
        // the persistent vector. This replaces the old `vec-empty` + N× `vec-push` — N persistent-trie
        // CONSTRUCTORS each consuming+rebuilding the whole vector (O(N) handle allocs) — with a single
        // bulk build (`vec-of-arr` is zero-copy for a ≤32-element list: the arr node IS the trie leaf,
        // reused by move). Each scalar element is BOXED to a u32 handle (a narrow int extended i32→i64
        // first); a nested compound is already a handle. `arr-len 0` yields the empty vector, so `(list)`
        // is `arr-alloc 0` + `vec-of-arr` (no push-chain special case).
        Core::ListNew { elems } => {
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            // Per-element scratch above the running high-water (see `Core::Tuple` — sibling elements of
            // different widths must not share a slot number).
            for (i, &elem) in elems.iter().enumerate() {
                let elem_base = *high;
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [arr, i, elem]
                let boxed = box_op(db, elem)?;
                emit_heap_store_tail(db, elem, boxed, out); // [arr, i, handle]
                // Canonicalize a rope-capable String/Bytes element to a flat leaf on construction (see
                // the `Core::Tuple` arm) — a list element nested in a value-eq'd/keyed compound is the
                // same nested-rope face.
                if elem_needs_rope_compaction(db, elem) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            out.push(Lir::CallImport(OP_VEC_OF_ARR)); // [arr] → [list]
            Ok(()) // leaves [list] — the list handle
        }
        // `List.len` — emit the list handle, then `vec-len` (→ u32 length, an i32 slot). `List.len`'s
        // type is `Int64` (an i64 slot), so EXTEND the i32 length to i64 (unsigned — a length is
        // non-negative). Without this the op left an i32 on the stack where an i64 is expected (the
        // function result, or any enclosing i64 context), so a `List.len` whose list came through a
        // runtime `if` (not folded to a constant) emitted a module that FAILED wasm validation
        // ("expected i64, found i32"). A folded `List.len` over a literal never reaches here (it becomes
        // a `ConstInt`), which is why the constant control validated while the runtime case did not.
        Core::ListLen { operand } => {
            // RECLAMATION (mirror the scalar-element `Core::Proj` reclaim): `vec-len` only BORROWS the list
            // and returns a scalar COUNT, retaining nothing from the sequence. If the operand is a fresh
            // OWNED TEMPORARY (a call result, a constructor — `heap_operand_ownership == Owned`) rather than
            // a BORROW of a live binding (a param/local the owner reclaims), nothing else drops it, so it
            // LEAKS one heap cell per call (`(List.len (build …))`). Stash it in a scratch slot across the
            // borrowing `vec-len`, then `drop` it — the count is already a scalar on the stack. A BORROWED
            // operand is left to its owner (declines to Owned only on a proven-fresh producer, else Borrowed
            // — leak-safe: an unproven ownership just leaves it un-dropped, never double-frees).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let list_slot = base;
                if list_slot + 1 > *high {
                    *high = list_slot + 1;
                }
                scratch_ty.insert(list_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [list]
                out.push(Lir::LocalTee(list_slot)); // [list], list_slot = the owned list
                out.push(Lir::CallImport(OP_VEC_LEN)); // → [len:i32] (borrows the list)
                out.push(Lir::LocalGet(list_slot)); // [len, list]
                out.push(Lir::CallImport(OP_DROP)); // → [len] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [len:i64] — List.len : Int64
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [list]
            out.push(Lir::CallImport(OP_VEC_LEN)); // → [len:i32]
            out.push(Lir::I64ExtendI32U); // → [len:i64] — List.len : Int64
            Ok(())
        }
        // `Bytes.of` — build the byte sequence on the rope heap. `bytes-alloc(len)` leaves a fresh buffer;
        // then for each element `[buf] ; index ; byte ; bytes-set` — and `bytes-set(buf,index,value) ->
        // buf` RETURNS the buffer (FBIP in-place), so the buffer threads through with no scratch local. A
        // byte is a RAW i32 in `0..=255` (NOT boxed like a list element); every element folded to a
        // constant in range at lowering (`lower_bytes_of`), so each is pushed as an `i32.const`. ⚠ the
        // byte value uses `Lir::ConstI32`, which the serializer writes as a SIGNED LEB — a raw byte ≥ 64
        // would sign-extend negative if hand-emitted, but `Lir::ConstI32` handles the signed encoding, so
        // there is no raw-opcode hazard here (the seed's `sleb128` bug was in hand-written opcode bytes).
        Core::BytesOf { elems } => {
            out.push(Lir::ConstI32(elems.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                // Push the element's BYTE VALUE (an i32 in 0..=255). A CONSTANT folds to an inline
                // `i32.const`; a RUNTIME element (a `UInt8` param, or `(UInt8.wrap n)`) is emitted — its
                // solved type is a narrow UInt8, so it already lives in an i32 machine slot (no extend,
                // no box: `bytes-set` takes a raw i32 byte). This is what lets the LEB128 encoder's
                // `(Bytes.of (list (UInt8.wrap n)))` build a byte from a runtime value.
                match core_of(db, elem) {
                    Core::ConstInt(v) => {
                        let byte =
                            v.to_i64()
                                .filter(|n| (0..=255).contains(n))
                                .ok_or_else(|| {
                                    Reject::coded(
                                        Code::IntOutOfRange,
                                        "a Bytes.of element is not a UInt8 (0..=255)",
                                    )
                                })? as i32;
                        out.push(Lir::ConstI32(byte)); // [buf, index, byte]
                    }
                    _ => {
                        // A runtime UInt8 — emit its value (an i32 slot); it is in 0..=255 by its type.
                        emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [buf, index, byte]
                    }
                }
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]  (bytes-set returns the buffer)
            }
            Ok(()) // leaves [buf] — the bytes handle
        }
        // A runtime `(bin …)` construction of fixed-width INTEGER segments. Alloc a buffer of the total
        // static width, then per segment: materialize its int value in an i64 scratch slot and write its `w`
        // bytes big-endian (`le` reversed) via `bytes-set` (which returns the buffer, so it threads on the
        // stack). A per-segment range-check is emitted below, but it is a DEFENSIVE BACKSTOP that is normally
        // DEAD: a `(uN v)`/`(iN v)` segment REQUIRES `v` to have the segment's exact width type (infer.rs
        // `seg_value_ty`), so a value that does not fit is a COMPILE-TIME type error (CDZ0203), never a
        // runtime trap — the value reaching here provably fits. Uses TWO scratch slots: `buf` (the byte
        // buffer handle) and `val` (the current segment's i64 value); both above `base`.
        Core::BinBuild { segs } => {
            let total: u32 = segs.iter().map(|s| s.width as u32).sum();
            // The current segment's value lives in an i64 scratch slot (range-checked, then its bytes
            // extracted by shift/mask). The byte buffer handle is THREADED ON THE STACK: `bytes-set`
            // returns the buffer, so each write leaves `[buf]` for the next — exactly like `BytesOf`.
            let val_slot = base;
            if base + 1 > *high {
                *high = base + 1;
            }
            scratch_ty.insert(val_slot, ValType::I64);
            out.push(Lir::ConstI32(total as i32)); // [total]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            let mut offset: u32 = 0;
            for s in &segs {
                let w = s.width as u32;
                let bits = w * 8;
                // Materialize the segment value in the i64 slot (a narrow int emits an i32 → extend by
                // its OWN signedness; an Int64 is already i64). Stack still just `[buf]` after the set.
                // The value sub-expression's transient scratch must FLOAT above the high-water mark, not
                // reuse a fixed `base + 1`: two segments each with a `(g x)` closure application (or any
                // slot-typed temp) would otherwise alias one wasm local at two widths — segment 1 an i32
                // closure cell, segment 2 an i64 arith stash — re-typing it → "expected i64, found i32"
                // (the disjoint-slot discipline `emit_checked_arith`/`emit_call_args` follow for siblings).
                let seg_base = (val_slot + 1).max(*high);
                emit(db, s.value, slots, seg_base, high, scratch_ty, layout, out)?; // [buf, val:i32|i64]
                emit_box_i32_to_i64_extend(db, s.value, out);
                out.push(Lir::LocalSet(val_slot)); // val := value:i64  → [buf]
                // RANGE CHECK (defensive backstop — normally DEAD, since the segment's width type already
                // bounds `val`; a value that does not fit is a compile-time CDZ0203): the value must fit the
                // segment's (signed, bits) width, else trap. Width 8 (an i64 holds every i64) needs no check.
                // Signed: `-(2^(bits-1)) <= val < 2^(bits-1)`; unsigned: `0 <= val < 2^bits`. Emitted as
                // `(low-fail | high-fail) → trap`.
                if bits < 64 {
                    if s.signed {
                        let hi = (1i64 << (bits - 1)) - 1;
                        let lo = -(1i64 << (bits - 1));
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(hi));
                        out.push(Lir::I64GtS); // val > hi
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(lo));
                        out.push(Lir::I64LtS); // val < lo
                        out.push(Lir::I32Or);
                        out.push(Lir::IfUnreachableEnd); // → trap "binary value does not fit segment"
                    } else {
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(0));
                        out.push(Lir::I64LtS); // val < 0
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(1i64 << bits)); // 2^bits
                        out.push(Lir::I64GeS); // val >= 2^bits (val < 2^63 since bits<64, so signed cmp ok)
                        out.push(Lir::I32Or);
                        out.push(Lir::IfUnreachableEnd); // → trap
                    }
                }
                // Write the `w` bytes MSB-first; `le` reverses the buffer position. Each `bytes-set`
                // consumes `[buf, pos, byte]` and returns `[buf]`, threading the buffer.
                for p in 0..w {
                    let shift = (w - 1 - p) * 8;
                    let pos = if s.little_endian {
                        offset + (w - 1 - p)
                    } else {
                        offset + p
                    };
                    // stack is [buf]
                    out.push(Lir::ConstI32(pos as i32)); // [buf, pos]
                    out.push(Lir::LocalGet(val_slot)); // [buf, pos, val:i64]
                    if shift > 0 {
                        out.push(Lir::ConstI64(shift as i64));
                        out.push(Lir::I64ShrU);
                    }
                    out.push(Lir::I32WrapI64);
                    out.push(Lir::ConstI32(0xff));
                    out.push(Lir::I32And); // [buf, pos, byte:i32]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
                }
                offset += w;
            }
            Ok(()) // leaves [buf] — the bytes handle
        }
        // A RUN of `(bits v k)` bit-fields with a runtime value, packed MSB-first into a fresh `Bytes`. The
        // run is byte-aligned (CDZ0220), so the total byte count + every flush position + the bit-cursor are
        // STATIC (all `k` are compile-time constants) — only the field values are runtime. `acc` (an i64
        // slot) accumulates the open bits MSB-first, flushing whole bytes from its top as they close, exactly
        // like the constant packer in `lower_bin_build`. The byte buffer is THREADED ON THE STACK like
        // `BinBuild`. Each field emits a range-check (`0 <= v < 2^k`) as a DEFENSIVE BACKSTOP that is
        // normally DEAD: a `(bits v k)` field REQUIRES `v : (UInt k)` (infer.rs `seg_value_ty`), so a value
        // that does not fit is a compile-time CDZ0203, not a runtime trap — the value reaching here fits.
        Core::BinBitsBuild { fields } => {
            let total_bits: u32 = fields.iter().map(|f| f.k).sum();
            let total_bytes = total_bits / 8; // byte-aligned (CDZ0220) — exact
            let val_slot = base;
            let acc_slot = base + 1;
            if base + 2 > *high {
                *high = base + 2;
            }
            scratch_ty.insert(val_slot, ValType::I64);
            scratch_ty.insert(acc_slot, ValType::I64);
            out.push(Lir::ConstI32(total_bytes as i32)); // [total]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(acc_slot)); // acc := 0  → [buf]
            let mut nbits: u32 = 0; // open bits since the last byte boundary (STATIC)
            let mut out_pos: u32 = 0; // running byte position in the buffer (STATIC)
            for f in &fields {
                let k = f.k; // 1..=56 (guarded at lower)
                // Materialize the field value in the i64 slot (a narrow int extends by its own signedness).
                emit(db, f.value, slots, base + 2, high, scratch_ty, layout, out)?; // [buf, val:i32|i64]
                emit_box_i32_to_i64_extend(db, f.value, out);
                out.push(Lir::LocalSet(val_slot)); // val := value:i64  → [buf]
                // RANGE CHECK (defensive backstop — normally DEAD, the `(UInt k)` field type already bounds
                // `val`): a k-bit UNSIGNED field, so `0 <= val < 2^k` (k ≤ 56 → 2^k is a positive i64).
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64(0));
                out.push(Lir::I64LtS); // val < 0
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64(1i64 << k));
                out.push(Lir::I64GeS); // val >= 2^k
                out.push(Lir::I32Or);
                out.push(Lir::IfUnreachableEnd); // → trap "binary value does not fit segment"  → [buf]
                // acc = (acc << k) | (val & ((1<<k)-1))
                out.push(Lir::LocalGet(acc_slot));
                out.push(Lir::ConstI64(k as i64));
                out.push(Lir::I64Shl); // acc << k
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64((1i64 << k) - 1));
                out.push(Lir::I64And); // val & mask
                out.push(Lir::I64Or);
                out.push(Lir::LocalSet(acc_slot)); // → [buf]
                nbits += k;
                // Flush every whole byte from the TOP of the accumulator (MSB-first), masking the flushed
                // high bits off `acc` after each byte (identical to the constant packer).
                while nbits >= 8 {
                    let shift = nbits - 8;
                    out.push(Lir::ConstI32(out_pos as i32)); // [buf, pos]
                    out.push(Lir::LocalGet(acc_slot));
                    if shift > 0 {
                        out.push(Lir::ConstI64(shift as i64));
                        out.push(Lir::I64ShrU); // acc >> shift
                    }
                    out.push(Lir::I32WrapI64);
                    out.push(Lir::ConstI32(0xff));
                    out.push(Lir::I32And); // [buf, pos, byte:i32]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
                    out_pos += 1;
                    nbits -= 8;
                    // acc &= (1<<nbits)-1 — drop the just-flushed high bits (nbits==0 → acc := 0).
                    if nbits == 0 {
                        out.push(Lir::ConstI64(0));
                    } else {
                        out.push(Lir::LocalGet(acc_slot));
                        out.push(Lir::ConstI64((1i64 << nbits) - 1));
                        out.push(Lir::I64And);
                    }
                    out.push(Lir::LocalSet(acc_slot)); // → [buf]
                }
            }
            debug_assert_eq!(
                nbits, 0,
                "a runtime bit-field run must be byte-aligned (CDZ0220)"
            );
            Ok(()) // leaves [buf] — the bytes handle
        }
        // Read a fixed-width int segment out of a runtime `Bytes` scrutinee (a `bin`-pattern binder). The
        // scrutinee handle is stashed in a slot, then the `w` bytes at `byte_offset` are `bytes-get`'d and
        // assembled into an i64 accumulator MSB-first (`le` reversed), and sign/zero-extended per `signed`.
        // The caller's length probe guarantees the read is in bounds. Result: [value:i64].
        Core::BinIntRead {
            bytes,
            byte_offset,
            width,
            signed,
            little_endian,
        } => {
            let w = width as u32;
            // The `bytes` operand is the materialized scrutinee (a `LocalRef` — a cheap `local.get`), so
            // it is RE-EMITTED per `bytes-get` rather than stashed in a scratch slot. Claiming a scratch
            // slot here (typed i32) collided with an i64 slot in a nested-if match chain; re-emitting the
            // handle avoids any scratch of our own, so nothing this arm emits can re-type a shared slot.
            out.push(Lir::ConstI64(0)); // [acc:i64]
            for p in 0..w {
                let shift = (w - 1 - p) * 8; // MSB-first bit position
                let pos = if little_endian {
                    byte_offset + (w - 1 - p)
                } else {
                    byte_offset + p
                };
                emit(db, bytes, slots, base, high, scratch_ty, layout, out)?; // [acc, bytes]
                out.push(Lir::ConstI32(pos as i32)); // [acc, bytes, pos]
                out.push(Lir::CallImport(OP_BYTES_GET)); // [acc, byte:i32]
                out.push(Lir::I64ExtendI32U); // [acc, byte:i64] (0..=255)
                if shift > 0 {
                    out.push(Lir::ConstI64(shift as i64));
                    out.push(Lir::I64Shl); // [acc, byte << shift]
                }
                out.push(Lir::I64Or); // [acc']
            }
            // Sign-extend a SIGNED segment narrower than 64 bits from its top bit; an unsigned segment is
            // already zero-extended (each byte was zero-extended). Shift left then arithmetic-shift right.
            if signed && w < 8 {
                let sh = ((8 - w) * 8) as i64; // bits above the value
                out.push(Lir::ConstI64(sh));
                out.push(Lir::I64Shl);
                out.push(Lir::ConstI64(sh));
                out.push(Lir::I64ShrS); // arithmetic → sign-extended
            }
            Ok(()) // leaves [value:i64]
        }
        // A `BinRestRead` binds a FINAL unsized `(bytes rest)` segment: the tail of the scrutinee after
        // the fixed int prefix, as a fresh `Bytes` handle. Emit `bytes-slice(bytes, off, bytes-len - off)`.
        // `bytes` is the materialized scrutinee (a `LocalRef` — a borrow shared by every arm), but
        // `bytes-slice` CONSUMES its source handle, so DUP the scrutinee first (rc++) and slice the copy;
        // the original stays live for the enclosing `let`'s scope-end drop. `off` is a static u32 (the sum
        // of the preceding int widths); the length is `bytes-len - off`, computed at i32 width (both are
        // non-negative and `off <= bytes-len` since the arm's length probe already required `len >= off`).
        Core::BinRestRead { bytes, byte_offset } => {
            // dup(handle) pops the handle and rc++'s it, returning nothing — so `tee` it into a scratch
            // slot, dup that copy, then get it back as the slice source. The slot is typed i32 (a handle).
            let handle_slot = base;
            if handle_slot + 1 > *high {
                *high = handle_slot + 1;
            }
            scratch_ty.insert(handle_slot, ValType::I32);
            emit(db, bytes, slots, base + 1, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalTee(handle_slot)); // [bytes], slot = bytes
            out.push(Lir::CallImport(OP_DUP)); // pops the copy, rc++ → []
            // Slice source (the retained, rc-incremented handle), then start = off, then len = bytes-len - off.
            out.push(Lir::LocalGet(handle_slot)); // [bytes] (owned copy for bytes-slice to consume)
            out.push(Lir::ConstI32(byte_offset as i32)); // [bytes, off]
            out.push(Lir::LocalGet(handle_slot)); // [bytes, off, bytes]
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [bytes, off, len:i32] (borrows)
            out.push(Lir::ConstI32(byte_offset as i32));
            out.push(Lir::I32Sub); // [bytes, off, len - off]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [slice-handle] (consumes the copied bytes)
            Ok(()) // leaves [rest:bytes-handle]
        }
        // `Bytes.len` — emit the bytes handle, then `bytes-len` (→ u32, an i32 slot), then extend to i64
        // (a length is non-negative), since `Bytes.len : Int64`. Mirrors `List.len` exactly.
        Core::BytesLen { operand } => {
            // RECLAMATION (same as `Core::ListLen`): `bytes-len` BORROWS the bytes and returns a scalar
            // count, so an OWNED-TEMPORARY operand must be dropped after the borrow or it leaks a heap cell.
            // A BORROWED param/local is left to its owner (leak-safe: Owned only on a proven-fresh producer).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let bytes_slot = base;
                if bytes_slot + 1 > *high {
                    *high = bytes_slot + 1;
                }
                scratch_ty.insert(bytes_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [bytes]
                out.push(Lir::LocalTee(bytes_slot)); // [bytes], bytes_slot = the owned bytes
                out.push(Lir::CallImport(OP_BYTES_LEN)); // → [len:i32] (borrows the bytes)
                out.push(Lir::LocalGet(bytes_slot)); // [len, bytes]
                out.push(Lir::CallImport(OP_DROP)); // → [len] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [len:i64] — Bytes.len : Int64
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::CallImport(OP_BYTES_LEN)); // → [len:i32]
            out.push(Lir::I64ExtendI32U); // → [len:i64] — Bytes.len : Int64
            Ok(())
        }
        // `List.push(l, x)` — emit the list handle, then the element boxed to a u32 handle by its type
        // (a narrow int extended i32→i64 first), then `vec-push` (RETURNS the new list handle). Nested
        // compound elements are already handles (`box_op` → None), pushed directly.
        Core::ListPush { list, elem } => {
            emit(db, list, slots, base, high, scratch_ty, layout, out)?; // [list]
            emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [list, elem]
            let boxed = box_op(db, elem)?;
            emit_heap_store_tail(db, elem, boxed, out); // [list, handle]
            out.push(Lir::CallImport(OP_VEC_PUSH)); // → [list']
            Ok(())
        }
        // `List.concat(a, b)` — emit both list handles, then `vec-concat` (→ the joined list handle).
        Core::ListConcat { lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, base, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(OP_VEC_CONCAT)); // → [a++b]
            Ok(())
        }
        // `List.update(l, i, x)` — emit the list handle, the index WRAPPED to the `u32` the op takes (the
        // language index is `Int64`, an i64 slot), then the element boxed to a u32 handle by its type (a
        // narrow int extended i32→i64 first, exactly as a push), then `vec-update` (RETURNS the new list
        // handle; an out-of-bounds index traps). Order matches `vec-update(v, index, elem)`.
        Core::ListUpdate { list, index, elem } => {
            emit(db, list, slots, base, high, scratch_ty, layout, out)?; // [list]
            emit(db, index, slots, base, high, scratch_ty, layout, out)?; // [list, index:i64]
            // HIGH-BITS BOUNDS GUARD before the i64→i32 wrap. `vec-update` takes a u32 index and checks it
            // against the length, but `i32.wrap_i64` discards the high 32 bits FIRST — so a huge index
            // `>= 2^32` that truncates BELOW the length would silently update the wrong slot instead of
            // trapping (an OOB update aliasing a valid element — a safety hole). Trap if the i64 index does
            // not fit u32 (`(index as u64) >= 2^32`); a value in `[0, 2^32)` wraps losslessly and the
            // runtime's own length check catches a real OOB. A NEGATIVE index is a huge u64 (≥ 2^32) so it
            // is caught here too (and is ≥ length regardless). Mirrors the `br_if` wrap-alias guard the
            // scalar `br_table` dispatch emits for an i64 scrutinee, but traps (`IfUnreachableEnd`) rather
            // than routing to a default. The index sub-value is kept in a scratch local across the test.
            let idx_slot = base;
            if idx_slot + 1 > *high {
                *high = idx_slot + 1;
            }
            scratch_ty.insert(idx_slot, ValType::I64);
            out.push(Lir::LocalTee(idx_slot)); // [list, index] — keep a copy in the slot
            out.push(Lir::ConstI64(0x1_0000_0000)); // 2^32
            out.push(Lir::I64GeU); // [list, (index as u64) >= 2^32]
            out.push(Lir::IfUnreachableEnd); // out of u32 range → trap (index out of bounds)
            out.push(Lir::LocalGet(idx_slot)); // [list, index:i64]
            out.push(Lir::I32WrapI64); // [list, index:i32] — now known to fit u32
            emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [list, index, elem]
            let boxed = box_op(db, elem)?;
            emit_heap_store_tail(db, elem, boxed, out); // [list, index, handle]
            out.push(Lir::CallImport(OP_VEC_UPDATE)); // → [list']
            Ok(())
        }
        // A runtime SUM construction — `(Option.Some 5)` or a nullary `None`. Build the PAYLOAD handle,
        // then `sum-new(disc, payload)`. The payload is (`value-heap-runtime.md` §Sum):
        //  - NULLARY (no payloads) → the unit value, an empty array `arr-alloc(0)`;
        //  - SINGLE payload → the value boxed to a handle (`box-int`/`box-bool`; a compound payload is
        //    already a handle, no box) — a NARROW int extends i32→i64 first, as `box-int` takes an i64;
        //  - MULTIPLE payloads → a tuple handle built exactly as `Core::Tuple` (`arr-alloc(n)` + per-
        //    payload box + `arr-set`).
        // Leaves the sum's u32 handle on the stack.
        Core::SumNew { disc, payloads } => {
            // ENUM-DISCRIMINANT sum (every variant nullary, ≥2 variants): the value IS its discriminant,
            // so construction is JUST the constant — no `sum-new` box, no unit payload. The i32 slot holds
            // the discriminant directly; a match switches on it and equality is `i32.eq` (see those sites).
            if node_is_enum_disc(db, id) {
                out.push(Lir::ConstI32(disc as i32));
                return Ok(());
            }
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            match payloads.len() {
                0 => {
                    // Unit payload: the inline-unit handle. `arr-alloc(0)` RETURNS exactly this constant
                    // (a compile-time-known handle, no heap node), so push it directly rather than
                    // emitting an `arr-alloc(0)` CALL — one const instead of `const 0 ; call arr-alloc`,
                    // and the nullary construction imports no runtime op for its payload. `IMM_UNIT` is
                    // DERIVED from the runtime's `cdz-abi` section by codegen (never hand-coded).
                    out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc, unit]
                }
                1 => {
                    let p = payloads[0];
                    // A UNIT-TYPED single payload — a single-variant nullary sum (`(type E EX)`) erases to
                    // `Ty::Nominal { inner: Unit }`, so a `(Result A E)`'s `Err` carries an `E` payload whose
                    // value is `Core::Unit` (it EMITS NOTHING — `valtype_of(Unit) = None`). This is the SAME
                    // shape as the 0-payload case above (a genuinely nullary variant): the payload handle is
                    // the inline-unit sentinel `IMM_UNIT`, which `emit_heap_store_tail` substitutes when the
                    // value pushed nothing. A SCALAR boxes; a compound payload is already a handle. Without
                    // this the payload was absent and `sum-new` underflowed the stack (invalid wasm).
                    emit(db, p, slots, base, high, scratch_ty, layout, out)?; // [disc, value | nothing]
                    let boxed = box_op(db, p)?;
                    emit_heap_store_tail(db, p, boxed, out); // [disc, payload-handle]
                    // Canonicalize a rope-capable String/Bytes payload to a flat leaf on construction (see
                    // the `Core::Tuple` arm) — a rope in a sum payload (e.g. `(Some (concat …))`) is the
                    // sum-payload face of the nested-rope value-eq/key miss.
                    if elem_needs_rope_compaction(db, p) {
                        out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [disc, flat-leaf]
                    }
                }
                n => {
                    // Multiple payloads: build a tuple `arr` and box each into its position. A UNIT payload
                    // occupies its slot with the inline-unit sentinel (`emit_heap_store_tail`), keeping the
                    // positional indices aligned — this is the `(A Int64 Unit)` shape whose Unit slot pushed
                    // NOTHING before, underflowing the per-payload `arr-set` into invalid wasm.
                    out.push(Lir::ConstI32(n as i32)); // [disc, n]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, arr]
                    for (i, &p) in payloads.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32)); // [disc, arr, i]
                        emit(db, p, slots, base, high, scratch_ty, layout, out)?; // [disc, arr, i, value]
                        let boxed = box_op(db, p)?;
                        emit_heap_store_tail(db, p, boxed, out); // [disc, arr, i, handle]
                        // Canonicalize a rope-capable payload element on construction (see above).
                        if elem_needs_rope_compaction(db, p) {
                            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [disc, arr, i, flat-leaf]
                        }
                        out.push(Lir::CallImport(OP_ARR_SET)); // [disc, arr]
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_NEW)); // → [sum-handle]
            Ok(())
        }
        // A RUNTIME `List.at(list, index)` — the bounds-checked fallible read. Evaluate the list handle
        // and the Int64 index ONCE into scratch slots (each is read more than once: the list for
        // `vec-len` AND `vec-get`; the index for the low/high bounds tests AND `vec-get`). Then test
        // `0 <= index < vec-len(list)` (an i64 compare — the u32 length is extended to i64, and the
        // signed index's low side catches a NEGATIVE index without it wrapping to a huge unsigned
        // offset), and emit `if in-bounds (Some <element>) else None`:
        //  - IN BOUNDS: `vec-get(list, wrap(index))` yields the element handle BORROWED (rc unchanged);
        //    `dup` it (the `Some` payload CONSUMES its handle, but the list still owns the element —
        //    value-heap-runtime.md §Constructors Consume And Accessors Borrow), then `sum-new(disc_some,
        //    handle)`. The element stays BOXED (a downstream match unboxes it), so no box/unbox here.
        //  - OUT OF BOUNDS: `sum-new(disc_none, arr-alloc(0))` — the nullary `None` (unit payload).
        // Leaves the Option's u32 handle on the stack.
        Core::ListAt {
            list,
            index,
            disc_some,
            disc_none,
        } => {
            // HANDLE SLOT REUSE: the list handle is read TWICE (the bounds-check `vec-len` and the in-bounds
            // `vec-get`), both BORROWING (no refcount change, never consume). When the list is a reusable
            // handle already resident in a stable slot (a param / kept `let`-local), read THAT slot directly
            // at each use instead of copying it into a fresh scratch slot — the heap analogue of the scalar
            // operand-slot reuse. A computed list is stashed in scratch once, as before.
            let reuse_list = reusable_handle_slot(db, list, slots);
            // RECLAMATION: `vec-len`/`vec-get` only BORROW the list; the read element is `dup`'d into the
            // `Some` payload (so it survives independently). If the list operand is a fresh OWNED TEMPORARY
            // (`List.at (build …) i` — not a reused param/kept-local slot), nothing else drops it → it LEAKS.
            // Drop `list_slot` after the if/else (the Option result is on top; `drop` pops only the list).
            // A reused-slot (param/kept-local) list is BORROWED — its owner reclaims; never drop it here.
            let reclaim_list = reuse_list.is_none()
                && matches!(heap_operand_ownership(db, list), Ok(HandleOwnership::Owned));
            // Scratch above `base`: the list handle (i32, only when NOT reusing an owner slot), the index
            // (i64), and — in the in-bounds arm — the borrowed element handle (i32). Reusing the list slot
            // frees one scratch slot (the index/elem shift down), shrinking the high-water. The operand
            // recursions float above all claimed scratch so they never clobber a live slot.
            let (list_slot, index_slot, elem_slot, floor) = match reuse_list {
                Some(s) => (s, base, base + 1, base + 2),
                None => (base, base + 1, base + 2, base + 3),
            };
            if elem_slot + 1 > *high {
                *high = elem_slot + 1;
            }
            if reuse_list.is_none() {
                scratch_ty.insert(list_slot, ValType::I32);
            }
            scratch_ty.insert(index_slot, ValType::I64);
            scratch_ty.insert(elem_slot, ValType::I32);
            if reuse_list.is_none() {
                emit(db, list, slots, floor, high, scratch_ty, layout, out)?; // [list]
                out.push(Lir::LocalSet(list_slot));
            }
            emit(db, index, slots, floor, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // in_bounds = (index >= 0) & (index < len), all in i64. LOWER-BOUND ELISION: when the index is
            // provably NON-NEGATIVE (a masked/length/unsigned/refined value), the `index >= 0` half is a
            // compile-time `true`, so drop it and test only `index < len` — a masked index (`(& i 15)`), a
            // `List.len`, or a loop counter refined `≥ 0` sheds the redundant lower check (3 ops).
            let index_nonneg = crate::lower::value_provably_nonneg(db, index);
            if !index_nonneg {
                out.push(Lir::LocalGet(index_slot));
                out.push(Lir::ConstI64(0));
                out.push(Lir::I64GeS); // [index >= 0]
            }
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::LocalGet(list_slot));
            out.push(Lir::CallImport(OP_VEC_LEN)); // [.., index, len:i32]
            out.push(Lir::I64ExtendI32U); // [.., index, len:i64]
            out.push(Lir::I64LtS); // [(index >= 0,) index < len]
            if !index_nonneg {
                out.push(Lir::I32And); // [in_bounds]
            }
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(element). `vec-get` yields the element handle BORROWED; `dup` retains it (rc++)
            // so the `Some` payload can own a reference while the list keeps its own. `dup(handle)`
            // RETURNS NOTHING (it pops the handle and increments its count), so the handle is stashed in
            // a scratch slot: `tee` (store + keep a copy for `dup`), `dup` (consume that copy, rc++), then
            // `get` it back as the payload under `disc_some` for `sum-new`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(list_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I32WrapI64); // [disc_some, list, index:i32] — vec-get takes a u32
            out.push(Lir::CallImport(OP_VEC_GET)); // [disc_some, elem-handle] (borrowed)
            out.push(Lir::LocalTee(elem_slot)); // [disc_some, elem], elem_slot = elem
            out.push(Lir::CallImport(OP_DUP)); // pops elem, rc++ → [disc_some]
            out.push(Lir::LocalGet(elem_slot)); // [disc_some, elem] (the retained handle)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: the unit payload is an empty array.
            out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
            // The `None` (nullary) variant's unit payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a
            // runtime `arr-alloc(0)` CALL — the runtime's `arr-alloc(0)` returns exactly `imm_unit()`, so
            // pushing the derived constant is equivalent and drops one import call per `None` (the same
            // optimization the `SumNew` nullary path already uses; this brings the `List.at`/`Map.lookup`/
            // `String.at`/`Bytes.at` None arms to parity).
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            if reclaim_list {
                // [Option] — drop the owned-temporary list now that both borrows (len + get) are done.
                out.push(Lir::LocalGet(list_slot));
                out.push(Lir::CallImport(OP_DROP)); // → [Option] (list reclaimed)
            }
            Ok(())
        }
        // A MAP construction — `(map …)` or `Map.empty`. `map-empty` leaves a fresh empty map; then for
        // each entry, box the key and value by their types (a narrow int extended i32→i64 first) and
        // `map-insert(map, key, val)` — which CONSUMES the map handle + key + value and RETURNS the new
        // map, threading the handle through with no scratch local (like `bytes-set`). Entries insert in
        // SOURCE order, so a later duplicate key overwrites (keys compared by value). Leaves the map handle.
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            out.push(Lir::CallImport(OP_MAP_EMPTY)); // → [map]
            for &(k, v) in &entries {
                emit(db, k, slots, base, high, scratch_ty, layout, out)?; // [map, key]
                let key_boxed = box_op_for(db, k, &key_ty)?;
                emit_heap_store_tail(db, k, key_boxed, out); // [map, key-handle]
                if key_needs_compaction(db, k) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
                }
                emit(db, v, slots, base, high, scratch_ty, layout, out)?; // [map, key, val]
                let val_boxed = box_op_for(db, v, &val_ty)?;
                emit_heap_store_tail(db, v, val_boxed, out); // [map, key, val-handle]
                out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map'] (consumes map, key, val)
            }
            Ok(()) // leaves [map] — the map handle
        }
        // `Map.insert(m, k, v)` — emit the map handle, the key boxed by its type, the value boxed by its
        // type, then `map-insert` (RETURNS the new map handle; consumes all three). Mirrors `MapNew`'s
        // per-entry insert.
        Core::MapInsert {
            map,
            key,
            val,
            key_ty,
            val_ty,
        } => {
            emit(db, map, slots, base, high, scratch_ty, layout, out)?; // [map]
            emit(db, key, slots, base, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf (champ contract)
            }
            emit(db, val, slots, base, high, scratch_ty, layout, out)?; // [map, key, val]
            let val_boxed = box_op_for(db, val, &val_ty)?;
            emit_heap_store_tail(db, val, val_boxed, out); // [map, key, val-handle]
            out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map']
            Ok(())
        }
        // `Map.remove(m, k)` — emit the map handle, the key boxed by its type, then `map-remove` (RETURNS
        // the new map; consumes the map, BORROWS the key). Removing an absent key yields a map equal to the
        // operand (total). The op only reads the key (via hash/eq) and drops the map's OWN stored columns,
        // never the passed-in key — so an OWNED-TEMPORARY key (boxed scalar / compacted rope / const-String
        // leaf) must be `drop`ped by the emit after the borrow, exactly like `MapLookup`. Scratch: the key
        // handle (i32), teed before the op and dropped after when owned.
        Core::MapRemove { map, key, key_ty } => {
            let key_slot = base;
            if key_slot + 1 > *high {
                *high = key_slot + 1;
            }
            scratch_ty.insert(key_slot, ValType::I32);
            emit(db, map, slots, base + 1, high, scratch_ty, layout, out)?; // [map]
            emit(db, key, slots, base + 1, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                // Compact BEFORE the tee so key_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
            }
            // OWNERSHIP GATE (mirrors `MapLookup`): `map-remove` BORROWS the key, so drop it AFTER only when
            // it is an OWNED TEMPORARY. A BORROWED String/compound key (param / kept-local / a live
            // sum-payload projection) is left to its owner — dropping it would free a live reference.
            let key_owned = key_handle_is_owned_temporary(db, key, &key_ty)?;
            out.push(Lir::LocalTee(key_slot)); // [map, key], key_slot = key (for the later drop)
            out.push(Lir::CallImport(OP_MAP_REMOVE)); // → [map'] (consumes map, borrows key)
            if key_owned {
                // Drop the owned key temporary now that the borrow-remove is done.
                out.push(Lir::LocalGet(key_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // `Map.size(m)` — emit the map handle, `map-size` (→ u32, an i32 slot), then extend to i64 (a
        // count is non-negative), since `Map.size : Int64`. Mirrors `List.len`/`Bytes.len`.
        Core::MapSize { map } => {
            // RECLAMATION (same as `Core::ListLen`/`Core::BytesLen`): `map-size` BORROWS the map and returns
            // a scalar count, so an OWNED-TEMPORARY operand (`Map.len (build …)`) must be dropped after the
            // borrow or it leaks a heap cell. A BORROWED param/local is left to its owner (Owned only on a
            // proven-fresh producer, else Borrowed — leak-safe).
            let reclaim = matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned));
            if reclaim {
                let map_slot = base;
                if map_slot + 1 > *high {
                    *high = map_slot + 1;
                }
                scratch_ty.insert(map_slot, ValType::I32);
                emit(db, map, slots, base + 1, high, scratch_ty, layout, out)?; // [map]
                out.push(Lir::LocalTee(map_slot)); // [map], map_slot = the owned map
                out.push(Lir::CallImport(OP_MAP_SIZE)); // → [size:i32] (borrows the map)
                out.push(Lir::LocalGet(map_slot)); // [size, map]
                out.push(Lir::CallImport(OP_DROP)); // → [size] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [size:i64] — Map.size : Int64
                return Ok(());
            }
            emit(db, map, slots, base, high, scratch_ty, layout, out)?; // [map]
            out.push(Lir::CallImport(OP_MAP_SIZE)); // → [size:i32]
            out.push(Lir::I64ExtendI32U); // → [size:i64] — Map.size : Int64
            Ok(())
        }
        // A SET construction — `(Set.of (list …))`. `set-empty` leaves a fresh empty set; then for each
        // element, box it by its type (a narrow int extended i32→i64 first) and `set-insert(set, elem)` —
        // which CONSUMES the set + element and RETURNS the new set, threading the handle through (like a
        // map insert). A duplicate element is a no-op at insert (the set dedups). Leaves the set handle.
        Core::SetOf { elems, elem_ty } => {
            out.push(Lir::CallImport(OP_SET_EMPTY)); // → [set]
            for &e in &elems {
                emit(db, e, slots, base, high, scratch_ty, layout, out)?; // [set, elem]
                let elem_boxed = box_op_for(db, e, &elem_ty)?;
                emit_heap_store_tail(db, e, elem_boxed, out); // [set, elem-handle]
                if key_needs_compaction(db, e) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
                }
                out.push(Lir::CallImport(OP_SET_INSERT)); // → [set'] (consumes set, elem)
            }
            Ok(()) // leaves [set] — the set handle
        }
        // `Set.insert(s, e)` — emit the set handle, the element boxed by its type, then `set-insert`
        // (RETURNS the new set; consumes both). Mirrors `MapInsert` without the value column.
        Core::SetInsert { set, elem, elem_ty } => {
            emit(db, set, slots, base, high, scratch_ty, layout, out)?; // [set]
            emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            out.push(Lir::CallImport(OP_SET_INSERT)); // → [set']
            Ok(())
        }
        // `Set.remove(s, e)` — emit the set handle, the element boxed by its type, then `set-remove`
        // (RETURNS the new set; consumes the set, BORROWS the element). Removing an absent element yields an
        // equal set (total). Like `map-remove`, the op only reads the element and drops the set's OWN stored
        // columns, never the passed-in element — so an OWNED-TEMPORARY element must be `drop`ped by the emit
        // after the borrow, exactly like `SetContains`. Scratch: the element handle (i32).
        Core::SetRemove { set, elem, elem_ty } => {
            let elem_slot = base;
            if elem_slot + 1 > *high {
                *high = elem_slot + 1;
            }
            scratch_ty.insert(elem_slot, ValType::I32);
            emit(db, set, slots, base + 1, high, scratch_ty, layout, out)?; // [set]
            emit(db, elem, slots, base + 1, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                // Compact BEFORE the tee so elem_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            // OWNERSHIP GATE (mirrors `SetContains`): `set-remove` BORROWS the element, so drop it AFTER only
            // when it is an OWNED TEMPORARY. A BORROWED element (param / kept-local / a live sum-payload
            // projection) is left to its owner — dropping it would free a live reference.
            let elem_owned = key_handle_is_owned_temporary(db, elem, &elem_ty)?;
            out.push(Lir::LocalTee(elem_slot)); // [set, elem], elem_slot = elem (for the later drop)
            out.push(Lir::CallImport(OP_SET_REMOVE)); // → [set'] (consumes set, borrows elem)
            if elem_owned {
                // Drop the owned element temporary now that the borrow-remove is done.
                out.push(Lir::LocalGet(elem_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // `Set.len(s)` — emit the set handle, `set-size` (→ u32), extend to i64 (`Set.len : Int64`).
        Core::SetLen { set } => {
            // RECLAMATION (same as `Core::MapSize`/`Core::ListLen`): `set-size` BORROWS the set, so an
            // OWNED-TEMPORARY operand must be dropped after the borrow or it leaks a heap cell. A borrowed
            // param/local is left to its owner.
            let reclaim = matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned));
            if reclaim {
                let set_slot = base;
                if set_slot + 1 > *high {
                    *high = set_slot + 1;
                }
                scratch_ty.insert(set_slot, ValType::I32);
                emit(db, set, slots, base + 1, high, scratch_ty, layout, out)?; // [set]
                out.push(Lir::LocalTee(set_slot)); // [set], set_slot = the owned set
                out.push(Lir::CallImport(OP_SET_SIZE)); // → [size:i32] (borrows the set)
                out.push(Lir::LocalGet(set_slot)); // [size, set]
                out.push(Lir::CallImport(OP_DROP)); // → [size] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [size:i64]
                return Ok(());
            }
            emit(db, set, slots, base, high, scratch_ty, layout, out)?; // [set]
            out.push(Lir::CallImport(OP_SET_SIZE)); // → [size:i32]
            out.push(Lir::I64ExtendI32U); // → [size:i64]
            Ok(())
        }
        // `Set.to-list(s)` — enumerate the set's elements as a `List` in canonical element order. Emit the
        // set handle, then build the compiler-baked element-shape descriptor as a constant `Bytes` handle
        // inline (`bytes-alloc(n)` then a `bytes-set` per descriptor byte — `bytes-set` returns the buffer,
        // so the desc handle threads on the stack), then `set-to-list(s, desc)` (BORROWS both; the runtime
        // reads the desc to order by element value and returns a fresh owned `List`). The descriptor is a
        // `Set`-rooted shape (NOT the value-form `Framed` wrapper), which is what `op_set_to_list` resolves.
        Core::SetToList { set, elem_ty } => {
            let Some(desc) = crate::lower::set_shape_descriptor(db, &elem_ty) else {
                return Err(Reject::decline(
                    "Set.to-list element shape has no orderable descriptor",
                ));
            };
            // The baked descriptor `Bytes` is an OWNED TEMPORARY that `set-to-list` only BORROWS (the
            // runtime reads it as an inspector; see `op_set_to_list` — "BORROWS `s` and `desc`"). So it must
            // be dropped after the op, or every `Set.to-list` call LEAKS the descriptor heap cell. Stash its
            // handle in a scratch slot across the (set, desc)-consuming op call, then drop it.
            let desc_slot = base;
            if desc_slot + 1 > *high {
                *high = desc_slot + 1;
            }
            scratch_ty.insert(desc_slot, ValType::I32);
            emit(db, set, slots, base + 1, high, scratch_ty, layout, out)?; // [set]
            out.push(Lir::ConstI32(desc.len() as i32)); // [set, len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [set, desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32)); // [set, buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [set, buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [set, buf] (bytes-set returns the buffer)
            }
            out.push(Lir::LocalTee(desc_slot)); // [set, desc], desc_slot = desc (for the later drop)
            out.push(Lir::CallImport(OP_SET_TO_LIST)); // [set, desc] → [list] (borrows both)
            out.push(Lir::LocalGet(desc_slot)); // [list, desc]
            out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed-only descriptor Bytes)
            Ok(()) // leaves [list]
        }
        // `Map.to-list(m)` — the map companion: emit the map, bake a MAP-rooted key/value shape descriptor
        // inline, then `map-to-list(m, desc)` → a `List (Tuple k v)` in canonical KEY order.
        Core::MapToList {
            map,
            key_ty,
            val_ty,
        } => {
            let Some(desc) = crate::lower::map_shape_descriptor(db, &key_ty, &val_ty) else {
                return Err(Reject::decline(
                    "Map.to-list key/value shape has no orderable descriptor",
                ));
            };
            // As in `Set.to-list`: the baked descriptor `Bytes` is an owned temporary `map-to-list` only
            // BORROWS (`op_map_to_list` — "BORROWS `m` and `desc`"), so it must be dropped after the op or
            // every `Map.to-list` call leaks the descriptor heap cell. Stash + drop across the op call.
            let desc_slot = base;
            if desc_slot + 1 > *high {
                *high = desc_slot + 1;
            }
            scratch_ty.insert(desc_slot, ValType::I32);
            emit(db, map, slots, base + 1, high, scratch_ty, layout, out)?; // [map]
            out.push(Lir::ConstI32(desc.len() as i32)); // [map, len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [map, desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32)); // [map, buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [map, buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [map, buf]
            }
            out.push(Lir::LocalTee(desc_slot)); // [map, desc], desc_slot = desc (for the later drop)
            out.push(Lir::CallImport(OP_MAP_TO_LIST)); // [map, desc] → [list] (borrows both)
            out.push(Lir::LocalGet(desc_slot)); // [list, desc]
            out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed-only descriptor Bytes)
            Ok(()) // leaves [list]
        }
        // A runtime `Set.contains(s, e)` — the TOTAL membership predicate. Box the element, `set-contains(s,
        // key)` (BORROWS both; returns a `bool` directly — UNLIKE `Map.lookup`'s NULL-or-handle → Option).
        // The boxed element is an owned temporary the emit must `drop` after the borrow — stash it in a
        // scratch slot, box, contains, then drop the stashed element. Leaves the bool on the stack.
        Core::SetContains { set, elem, elem_ty } => {
            let set_slot = base;
            let elem_slot = base + 1;
            if elem_slot + 1 > *high {
                *high = elem_slot + 1;
            }
            scratch_ty.insert(set_slot, ValType::I32);
            scratch_ty.insert(elem_slot, ValType::I32);
            // `set-contains` BORROWS BOTH the set and the element and returns a bool (nothing borrows out of
            // the set). Reclaim EACH iff an OWNED TEMPORARY: the SET when it is a fresh computed value
            // (`Set.contains (build …) x` — else it leaks, the collection twin of the Set.len owned-temp
            // reclaim), the ELEMENT via the existing `key_handle_is_owned_temporary` gate. A borrowed
            // param/kept-local set/element is left to its owner.
            let set_owned = matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned));
            emit(db, set, slots, base + 2, high, scratch_ty, layout, out)?; // [set]
            if set_owned {
                out.push(Lir::LocalTee(set_slot)); // [set], set_slot = the owned set (for the later drop)
            }
            emit(db, elem, slots, base + 2, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                // Compact BEFORE the tee so elem_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            // OWNERSHIP GATE (mirrors `MapLookup`): `set-contains` BORROWS the element, so drop it AFTER
            // only when it is an OWNED TEMPORARY (a boxed scalar, a compacted rope, or a fresh owned
            // compound). A BORROWED String/compound element — a param / kept-local / a live sum-payload or
            // element projection — is used as-is; dropping it would free a reference its owner still holds.
            let elem_owned = key_handle_is_owned_temporary(db, elem, &elem_ty)?;
            out.push(Lir::LocalTee(elem_slot)); // [set, elem], elem_slot = elem (for the later drop)
            out.push(Lir::CallImport(OP_SET_CONTAINS)); // [bool] (borrows set + elem)
            if elem_owned {
                // Drop the owned element temporary now that the borrow-contains is done.
                out.push(Lir::LocalGet(elem_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            if set_owned {
                // Drop the owned-temporary SET (set-contains only borrowed it; the bool result is a scalar).
                out.push(Lir::LocalGet(set_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(()) // leaves [bool]
        }
        // A set-algebra op — emit both operand set handles, then the matching runtime op (consumes both,
        // returns the result set). `Set.union`/`intersection`/`difference` share this shape.
        Core::SetAlgebra { op, lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, base, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(match op {
                crate::core::SetAlgebraOp::Union => OP_SET_UNION,
                crate::core::SetAlgebraOp::Intersection => OP_SET_INTERSECTION,
                crate::core::SetAlgebraOp::Difference => OP_SET_DIFFERENCE,
            })); // → [result]
            Ok(())
        }
        // A runtime `Map.lookup(m, k)` — the fallible keyed read. Box the key, `map-lookup(m, key)` (BORROWS
        // both; returns the STORED VALUE HANDLE, or NULL when the key is absent). If the returned handle is
        // non-null build `Some(value)` — the value is a boxed handle used DIRECTLY as the `Some` payload,
        // `dup`'d so the map keeps its own reference (mirrors `ListAt`'s borrowed `vec-get`) — else `None`.
        // The boxed key is an owned temporary `drop`ped after the borrow. Scratch: the key handle (i32,
        // dropped after lookup) and the looked-up value handle (i32).
        Core::MapLookup {
            map,
            key,
            key_ty,
            disc_some,
            disc_none,
            ..
        } => {
            let key_slot = base;
            let val_slot = base + 1;
            let map_slot = base + 2;
            if map_slot + 1 > *high {
                *high = map_slot + 1;
            }
            scratch_ty.insert(key_slot, ValType::I32);
            scratch_ty.insert(val_slot, ValType::I32);
            scratch_ty.insert(map_slot, ValType::I32);
            // `map-lookup` BORROWS the map; if the map is an OWNED TEMPORARY (`Map.lookup (build …) k` — not
            // a reused param/kept-local) it must be reclaimed or it LEAKS. ⚠ DELICATE ORDERING: the looked-up
            // VALUE is stored borrowed in `val_slot` and `dup`'d in the Some arm — so the map's drop must come
            // AFTER that dup (THEN), and in the None arm (val is NULL, nothing to preserve). Dropping right
            // after `map-lookup` would free the value `val_slot` still borrows → a UAF. Stash the map now.
            let map_owned = matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned));
            emit(db, map, slots, base + 3, high, scratch_ty, layout, out)?; // [map]
            if map_owned {
                out.push(Lir::LocalTee(map_slot)); // [map], map_slot = the owned map (for the arm drops)
            }
            emit(db, key, slots, base + 3, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                // Compact BEFORE the tee so key_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
            }
            // OWNERSHIP GATE: `map-lookup` BORROWS the key (never consumes it), so we drop it AFTER only
            // when it is an OWNED TEMPORARY (a boxed scalar, a compacted rope, or a fresh owned compound).
            // A BORROWED String/compound key — a param / kept-local / a `sum-payload`/`arr-get` projection
            // of a still-live value — is used as-is; dropping it would free a reference its owner still
            // holds (a use-after-free). This is the two-live-matched-String-payloads miscompile: a
            // tree-walker looking up a node's OWN key AND its child's key (both live sum-payload String
            // projections) had the second borrowed key freed, flipping its comparison (a silent wrong
            // count). See `key_handle_is_owned_temporary`.
            let key_owned = key_handle_is_owned_temporary(db, key, &key_ty)?;
            out.push(Lir::LocalTee(key_slot)); // [map, key], key_slot = key (for the later drop)
            out.push(Lir::CallImport(OP_MAP_LOOKUP)); // [value-or-null] (borrows map + key)
            out.push(Lir::LocalSet(val_slot)); // val_slot = value-or-null, stack empty
            if key_owned {
                // Drop the owned key temporary now that the borrow-lookup is done (map-lookup borrows it).
                out.push(Lir::LocalGet(key_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            // present = (value != NULL).
            out.push(Lir::LocalGet(val_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [present]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(value). The stored value handle is BORROWED (the map still owns it); `dup` it so
            // the `Some` payload owns its own reference, then use it as the payload under `disc_some`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(val_slot)); // [disc_some, value]
            out.push(Lir::CallImport(OP_DUP)); // pops value, rc++ → [disc_some]
            out.push(Lir::LocalGet(val_slot)); // [disc_some, value] (retained)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            if map_owned {
                // The value is now independently retained (dup'd above), so it is SAFE to drop the
                // owned-temporary map here — AFTER the dup, not right after `map-lookup` (that would free
                // the value the `val_slot` borrow points at → UAF). `drop` pops only the map. [Some-handle]
                out.push(Lir::LocalGet(map_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            out.push(Lir::Else);
            // ELSE — None: the unit payload is an empty array.
            out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
            // The `None` (nullary) variant's unit payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a
            // runtime `arr-alloc(0)` CALL — the runtime's `arr-alloc(0)` returns exactly `imm_unit()`, so
            // pushing the derived constant is equivalent and drops one import call per `None` (the same
            // optimization the `SumNew` nullary path already uses; this brings the `List.at`/`Map.lookup`/
            // `String.at`/`Bytes.at` None arms to parity).
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            if map_owned {
                // None arm: `val_slot` is NULL (no value borrowed out), so the owned-temporary map is dropped
                // here with nothing to preserve. [None-handle]
                out.push(Lir::LocalGet(map_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            out.push(Lir::End);
            Ok(())
        }
        // A runtime `Bytes.at` — the fallible byte read. Bounds-check `0 <= index < bytes-len`, then in
        // bounds build `Some(box-int(bytes-get))`, else `None`. Unlike `ListAt`, `bytes-get` returns a
        // RAW byte VALUE (i32 `0..=255`), so there is no borrowed handle to `dup`: zero-extend it to i64
        // and `box-int` it into an `Int64` for the `Some` payload. Two scratch slots (bytes handle i32,
        // index i64) above `base`; operand recursions float above them.
        Core::BytesAt {
            bytes,
            index,
            disc_some,
            disc_none,
        } => {
            // HANDLE SLOT REUSE (see `ListAt`): the bytes handle is read twice (bounds-check `bytes-len` +
            // the in-bounds `bytes-get`), both BORROWING. A reusable handle already in a stable slot is read
            // directly; a computed one is stashed in scratch once.
            let reuse_bytes = reusable_handle_slot(db, bytes, slots);
            // RECLAMATION (see `ListAt`): an OWNED-temporary bytes operand (`Bytes.at (build …) i`, not a
            // reused param/kept-local) is dropped after the borrowing len/get — the read byte is a COPIED
            // i32 value (nothing borrows from the bytes), so the sequence can be freed. A borrowed
            // param/kept-local is left to its owner.
            let reclaim_bytes = reuse_bytes.is_none()
                && matches!(
                    heap_operand_ownership(db, bytes),
                    Ok(HandleOwnership::Owned)
                );
            let (bytes_slot, index_slot, floor) = match reuse_bytes {
                Some(s) => (s, base, base + 1),
                None => (base, base + 1, base + 2),
            };
            if index_slot + 1 > *high {
                *high = index_slot + 1;
            }
            if reuse_bytes.is_none() {
                scratch_ty.insert(bytes_slot, ValType::I32);
            }
            scratch_ty.insert(index_slot, ValType::I64);
            if reuse_bytes.is_none() {
                emit(db, bytes, slots, floor, high, scratch_ty, layout, out)?; // [bytes]
                out.push(Lir::LocalSet(bytes_slot));
            }
            emit(db, index, slots, floor, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // in_bounds = (index >= 0) & (index < len), all in i64. LOWER-BOUND ELISION (see `ListAt`): a
            // provably NON-NEGATIVE index (a masked/length/unsigned/refined value) makes `index >= 0` a
            // compile-time `true`, so drop it and test only `index < len`.
            let index_nonneg = crate::lower::value_provably_nonneg(db, index);
            if !index_nonneg {
                out.push(Lir::LocalGet(index_slot));
                out.push(Lir::ConstI64(0));
                out.push(Lir::I64GeS); // [index >= 0]
            }
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [.., index, len:i32]
            out.push(Lir::I64ExtendI32U); // [.., index, len:i64]
            out.push(Lir::I64LtS); // [(index >= 0,) index < len]
            if !index_nonneg {
                out.push(Lir::I32And); // [in_bounds]
            }
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(box-int(bytes-get(bytes, index))). `bytes-get` yields the byte as an i32 VALUE
            // (0..=255); zero-extend to i64 and box it into an `Int64` handle for the `Some` payload.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, index:i32] — bytes-get takes a u32
            out.push(Lir::CallImport(OP_BYTES_GET)); // [disc_some, byte:i32] (raw value 0..=255)
            out.push(Lir::I64ExtendI32U); // [disc_some, byte:i64] (a byte is non-negative)
            out.push(Lir::CallImport(OP_BOX_INT)); // [disc_some, Int64-handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: the unit payload is an empty array.
            out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
            // The `None` (nullary) variant's unit payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a
            // runtime `arr-alloc(0)` CALL — the runtime's `arr-alloc(0)` returns exactly `imm_unit()`, so
            // pushing the derived constant is equivalent and drops one import call per `None` (the same
            // optimization the `SumNew` nullary path already uses; this brings the `List.at`/`Map.lookup`/
            // `String.at`/`Bytes.at` None arms to parity).
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            if reclaim_bytes {
                // [Option] — drop the owned-temporary bytes now that both borrows (len + get) are done.
                out.push(Lir::LocalGet(bytes_slot));
                out.push(Lir::CallImport(OP_DROP)); // → [Option] (bytes reclaimed)
            }
            Ok(())
        }
        // `String.at(str, index)` on a RUNTIME string — read the i-th UNICODE SCALAR as a one-scalar
        // String, fallibly. A String is a flat UTF-8 byte leaf, so WALK the byte buffer: a byte is a
        // scalar START iff `(byte & 0xC0) != 0x80` (not a `10xxxxxx` continuation byte). Phase 1 skips
        // `index` scalar starts, leaving `pos` at the target scalar's first byte; phase 2 measures that
        // scalar's byte span (lead byte + its continuation bytes) and `bytes-slice`s it into `Some`. A
        // negative index or one at/beyond the scalar count → `None`. The string handle is BORROWED for the
        // scan (`bytes-len`/`bytes-get`) and CONSUMED by the final `bytes-slice`; the None branch drops it.
        Core::StrAt {
            string,
            index,
            disc_some,
            disc_none,
        } => {
            let str_slot = base;
            let index_slot = base + 1;
            let pos_slot = base + 2;
            let scalar_slot = base + 3;
            let bytelen_slot = base + 4;
            let spanstart_slot = base + 5;
            if spanstart_slot + 1 > *high {
                *high = spanstart_slot + 1;
            }
            scratch_ty.insert(str_slot, ValType::I32);
            for s in [
                index_slot,
                pos_slot,
                scalar_slot,
                bytelen_slot,
                spanstart_slot,
            ] {
                scratch_ty.insert(s, ValType::I64);
            }
            emit(db, string, slots, base + 6, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            emit(db, index, slots, base + 6, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // byte-count (i64), read repeatedly below.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::LocalSet(bytelen_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(pos_slot)); // pos = 0
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(scalar_slot)); // scalar = 0
            // A byte at `pos` (already known `pos < bytelen`) begins a NEW scalar iff it is NOT a `10xxxxxx`
            // continuation byte: `(bytes-get(str, pos) & 0xC0) != 0x80`. Helper closure emitting that test
            // as an i32 bool onto the stack (borrows `str`).
            let push_is_lead = |out: &mut Emit| {
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32WrapI64);
                out.push(Lir::CallImport(OP_BYTES_GET)); // [byte:i32]
                out.push(Lir::ConstI32(0xC0));
                out.push(Lir::I32And);
                out.push(Lir::ConstI32(0x80));
                out.push(Lir::I32Ne); // [(byte & 0xC0) != 0x80]
            };
            // "Advance `pos` past ONE whole scalar": `pos++`, then while `pos < bytelen` and the byte at
            // `pos` is a CONTINUATION byte, `pos++`. Emitted as `block { pos++; loop { br_out if pos>=len;
            // br_out if is_lead(pos); pos++; br loop } }`. Precondition: `pos < bytelen` (a scalar starts
            // here). Used in both phases.
            let emit_skip_one_scalar = |out: &mut Emit, push_is_lead: &dyn Fn(&mut Emit)| {
                // pos++ past the lead byte.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty)); // $cont_done
                out.push(Lir::Loop(BlockType::Empty)); // $cont
                // pos >= bytelen → done.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // → $cont_done
                // byte at pos is a LEAD byte (new scalar) → done.
                push_is_lead(out);
                out.push(Lir::BrIf(1)); // → $cont_done
                // else a continuation byte: pos++, loop.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0)); // → $cont
                out.push(Lir::End); // end $cont
                out.push(Lir::End); // end $cont_done
            };
            // PHASE 1 — skip `index` scalar starts (only if index >= 0; a negative index leaves scalar=0 <
            // index false, so found is computed false below). `block { loop { br_out if scalar>=index;
            // br_out if pos>=bytelen; skip_one_scalar; scalar++; br loop } }`.
            out.push(Lir::Block(BlockType::Empty)); // $skip_done
            out.push(Lir::Loop(BlockType::Empty)); // $skip
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I64GeS);
            out.push(Lir::BrIf(1)); // scalar >= index → $skip_done
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64GeS);
            out.push(Lir::BrIf(1)); // pos >= bytelen → $skip_done (index out of range)
            emit_skip_one_scalar(out, &push_is_lead);
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::ConstI64(1));
            out.push(Lir::I64Add);
            out.push(Lir::LocalSet(scalar_slot));
            out.push(Lir::Br(0)); // → $skip
            out.push(Lir::End); // end $skip
            out.push(Lir::End); // end $skip_done
            // found = (index >= 0) & (scalar == index) & (pos < bytelen). scalar only reaches `index` if it
            // did not hit end first, so `scalar == index && pos < bytelen` is the in-range condition; the
            // explicit `index >= 0` guards a negative index (where scalar=0 stayed below index).
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [index >= 0]
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I64Eq); // [.., scalar == index]
            out.push(Lir::I32And);
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64LtS); // [.., pos < bytelen]
            out.push(Lir::I32And); // [found]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — measure the scalar's byte span and slice it. spanstart = pos; advance pos past this
            // scalar; span_len = pos - spanstart. `bytes-slice(str, spanstart, span_len)` CONSUMES str.
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalSet(spanstart_slot));
            emit_skip_one_scalar(out, &push_is_lead);
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            // `bytes-slice` CONSUMES its buffer, but `str` is a BORROWED operand of `String.at` (the
            // `str_slot` handle is only borrowed — a param/local the caller owns, or an owned temporary
            // this arm's own end reclaims uniformly). So RETAIN it (`dup`, rc++) before the consuming slice,
            // exactly as `List.at` `dup`s the borrowed `vec-get` element before the `Some` consumes it — the
            // slice then owns an INDEPENDENT reference, and `str` is left untouched for its other uses (the
            // recursive `(cnt s …)` that threads the same string). Without this, slicing consumes the sole
            // reference to `str`; the pre-fix code masked it by LEAKING the un-compacted `Some(slice)` (the
            // leak pinned `str` alive but compared by rope offset — wrong), and compacting the slice
            // (below) then `op_drop`s `str`'s node → a use-after-free the recursive scan hit as a trap.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_DUP)); // rc++ str: the slice takes an independent reference
            out.push(Lir::LocalGet(str_slot)); // [disc_some, str] (retained)
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I32WrapI64); // [.., spanstart:i32]
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I64Sub);
            out.push(Lir::I32WrapI64); // [.., span_len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes the dup'd str)
            // COMPACT the fresh slice to an INDEPENDENT flat leaf before wrapping it in `Some`. A
            // `bytes-slice` result is a ROPE node — a `[off, len]` offset INTO the source string — whose
            // PHYSICAL bytes are the source's, not a flat `[byte…]` leaf. A String's content-equality
            // (`Core::ValueEq` → `champ_eq`) and its map/set-key hashing compare PHYSICAL bytes, so a rope
            // slice compares by its offset and never matches a flat twin of identical content (the
            // `String.at` content-equality miscompile: `(= (String.at s i) "a")` silently false; a lexer
            // over a runtime string cannot classify a char). `bytes-compact` here is REFCOUNT-NEUTRAL — it
            // CONSUMES the owned slice we just produced and returns a content-equal owned leaf (flattening
            // releases the source-string reference the slice pinned, exactly the reference `bytes-slice`
            // transferred in), so the ownership accounting is unchanged (owned in, owned out) and no
            // downstream borrow/drop shifts. Doing it at the PRODUCER (not the `=` site) fixes ALL uses of
            // the result — equality, a map/set key, a re-concat — and is cheap: a `String.at` result is a
            // SINGLE Unicode scalar (≤4 bytes), so the flatten copies at most 4 bytes. This is why the
            // `Core::ValueEq` owned-String compaction did NOT catch it: the extracted payload reaches `=` as
            // a BORROW from the `Some` wrapper (owned by it), so the value-eq site cannot own-and-compact it
            // without a double-free — the slice must be flattened while it is still the fresh owned producer
            // result, here.
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // slice rope → independent flat leaf (owned→owned)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `str` was BORROWED (the Some branch `dup`'d it for the slice; this branch takes
            // no reference), so it is NOT dropped here — its owner reclaims it (an enclosing `let`/param),
            // exactly as `List.at`'s None branch leaves its list untouched. `String.at` is now a clean
            // BORROW of its string, like `List.at`/`Bytes.at`.
            out.push(Lir::ConstI32(disc_none as i32));
            // The `None` (nullary) variant's unit payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a
            // runtime `arr-alloc(0)` CALL — the runtime's `arr-alloc(0)` returns exactly `imm_unit()`, so
            // pushing the derived constant is equivalent and drops one import call per `None` (the same
            // optimization the `SumNew` nullary path already uses; this brings the `List.at`/`Map.lookup`/
            // `String.at`/`Bytes.at` None arms to parity).
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // `String.slice string start end` — the fallible half-open SCALAR sub-range `[start, end)`. Walk the
        // flat UTF-8 buffer scalar-by-scalar (identical machinery to `String.at`, generalized to TWO scalar
        // positions): PHASE 1 skips `start` scalars, recording the byte position `spanstart`; PHASE 2 skips
        // to `end`, leaving the byte position `pos`. The slice is the byte span `[spanstart, pos)`. In range
        // (`start >= 0 && start <= end && scalar == end` — reaching `scalar == end` proves exactly `end`
        // scalars existed, so `end <= scalar-len`) → `Some(bytes-slice(str, spanstart, pos - spanstart))`
        // COMPACTED to an independent flat leaf; else `None`. `pos == bytelen` is a VALID end (slicing to the
        // very end), so the found test does NOT require `pos < bytelen` (unlike `String.at`, which reads a
        // scalar AT pos). `str` is BORROWED (Some branch `dup`s it before the consuming `bytes-slice`; None
        // branch takes no reference) — the owner reclaims it, exactly like `String.at`.
        Core::StrSlice {
            string,
            start,
            end,
            disc_some,
            disc_none,
        } => {
            let str_slot = base;
            let start_slot = base + 1;
            let end_slot = base + 2;
            let pos_slot = base + 3;
            let scalar_slot = base + 4;
            let bytelen_slot = base + 5;
            let spanstart_slot = base + 6;
            if spanstart_slot + 1 > *high {
                *high = spanstart_slot + 1;
            }
            scratch_ty.insert(str_slot, ValType::I32);
            for s in [
                start_slot,
                end_slot,
                pos_slot,
                scalar_slot,
                bytelen_slot,
                spanstart_slot,
            ] {
                scratch_ty.insert(s, ValType::I64);
            }
            emit(db, string, slots, base + 7, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            emit(db, start, slots, base + 7, high, scratch_ty, layout, out)?; // [start:i64]
            out.push(Lir::LocalSet(start_slot));
            emit(db, end, slots, base + 7, high, scratch_ty, layout, out)?; // [end:i64]
            out.push(Lir::LocalSet(end_slot));
            // byte-count (i64), read repeatedly below.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::LocalSet(bytelen_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(pos_slot)); // pos = 0
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(scalar_slot)); // scalar = 0
            // A byte at `pos` (`pos < bytelen`) begins a NEW scalar iff it is NOT a `10xxxxxx` continuation
            // byte: `(bytes-get(str, pos) & 0xC0) != 0x80`. (Same helper as `String.at`.)
            let push_is_lead = |out: &mut Emit| {
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32WrapI64);
                out.push(Lir::CallImport(OP_BYTES_GET)); // [byte:i32]
                out.push(Lir::ConstI32(0xC0));
                out.push(Lir::I32And);
                out.push(Lir::ConstI32(0x80));
                out.push(Lir::I32Ne); // [(byte & 0xC0) != 0x80]
            };
            // "Advance `pos` past ONE whole scalar" — `pos++`, then skip continuation bytes. Precondition:
            // `pos < bytelen`. (Same helper as `String.at`.)
            let emit_skip_one_scalar = |out: &mut Emit, push_is_lead: &dyn Fn(&mut Emit)| {
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty)); // $cont_done
                out.push(Lir::Loop(BlockType::Empty)); // $cont
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // pos >= bytelen → $cont_done
                push_is_lead(out);
                out.push(Lir::BrIf(1)); // lead byte → $cont_done
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0)); // → $cont
                out.push(Lir::End); // end $cont
                out.push(Lir::End); // end $cont_done
            };
            // A "skip scalars until `scalar >= <limit_slot>` or `pos >= bytelen`" loop — used for both
            // phases (limit = start, then end). Advances pos/scalar together. Captures `push_is_lead` /
            // `emit_skip_one_scalar` from the enclosing scope (rather than taking them as params) so its
            // signature stays a simple `(&mut Emit, u32)`.
            let emit_skip_until = |out: &mut Emit, limit_slot: u32| {
                out.push(Lir::Block(BlockType::Empty)); // $done
                out.push(Lir::Loop(BlockType::Empty)); // $skip
                out.push(Lir::LocalGet(scalar_slot));
                out.push(Lir::LocalGet(limit_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // scalar >= limit → $done
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // pos >= bytelen → $done (limit beyond scalar-len)
                emit_skip_one_scalar(out, &push_is_lead);
                out.push(Lir::LocalGet(scalar_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(scalar_slot));
                out.push(Lir::Br(0)); // → $skip
                out.push(Lir::End); // end $skip
                out.push(Lir::End); // end $done
            };
            // PHASE 1 — skip `start` scalars, then record the slice's byte start.
            emit_skip_until(out, start_slot);
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalSet(spanstart_slot)); // spanstart = byte pos of the start-th scalar
            // PHASE 2 — skip on to `end`; `pos` is then the byte position of the end-th scalar boundary.
            emit_skip_until(out, end_slot);
            // found = (start >= 0) & (start <= end) & (scalar == end). `scalar == end` proves exactly `end`
            // scalars were consumed (so `end <= scalar-len`); `start <= end` rejects a reversed range (which
            // could otherwise reach `scalar == end` when phase 1 ran out at exactly `end` scalars); `start >=
            // 0` rejects a negative start (which skips no scalars, leaving `scalar` free to reach `end`).
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start >= 0]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::LocalGet(end_slot));
            out.push(Lir::I64LeS); // [.., start <= end]
            out.push(Lir::I32And);
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(end_slot));
            out.push(Lir::I64Eq); // [.., scalar == end]
            out.push(Lir::I32And); // [found]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — `Some(bytes-slice(str, spanstart, pos - spanstart))`. `bytes-slice` CONSUMES its buffer,
            // but `str` is BORROWED, so `dup` (rc++) before the slice takes an independent reference (exactly
            // as `String.at` does). COMPACT the fresh slice rope to a flat leaf so its content-equality /
            // key-hashing compares by physical bytes, not rope offset (the `String.at` content-eq fix,
            // refcount-neutral). `spanstart == pos` (an empty slice, `start == end`) yields `Some ""`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_DUP)); // rc++ str: the slice takes an independent reference
            out.push(Lir::LocalGet(str_slot)); // [disc_some, str] (retained)
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I32WrapI64); // [.., spanstart:i32]
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I64Sub);
            out.push(Lir::I32WrapI64); // [.., span_len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes the dup'd str)
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // slice rope → independent flat leaf (owned→owned)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `str` was BORROWED (this branch took no reference), so it is NOT dropped here; its
            // owner reclaims it (exactly like `String.at`'s None branch).
            out.push(Lir::ConstI32(disc_none as i32));
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // `Bytes.concat(a, b)` — emit both handles, `bytes-concat` (consumes both, returns the new one).
        Core::BytesConcat { lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, base, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(OP_BYTES_CONCAT)); // → [a++b]
            Ok(())
        }
        // `BigInt.of x` on a runtime `Int a` — widen to a BigInt heap leaf (an i32 handle). `x` is an i64
        // SCALAR (no heap ref), so nothing to drop — a fresh owned handle is left on the stack.
        //
        // SIGNEDNESS: `bigint-of-i64`'s operand is a SIGNED i64, so it reads the high bit as a sign. That is
        // correct for a SIGNED source width (Int8..Int64), but a runtime `UInt64` value ≥ 2^63 has its high
        // bit SET as MAGNITUDE, not sign — passing it through `bigint-of-i64` yields the WRONG NEGATIVE
        // BigInt (`BigInt.of` is `∀a.(Int a)->BigInt`, and a big UInt64 is a positive value). So for an
        // UNSIGNED source, build the BigInt from its canonical sign-magnitude BYTES instead: a non-negative
        // sign byte (0) + the 8 little-endian magnitude bytes of the u64, then `bigint-of-bytes` (the runtime
        // analogue of the beyond-i64 CONSTANT path `emit_const_bigint_leaf`). `from_sign_magnitude_bytes`
        // re-normalizes, so trailing-zero magnitude bytes are fine — a fixed 9-byte leaf needs no runtime
        // stripping. (A signed source keeps the one-instruction `bigint-of-i64`; only unsigned pays the
        // ~9-op byte materialization, and only Int64/UInt64's top bit can even differ — a narrower unsigned
        // width's value is always < 2^63 so both paths agree, but keying on signedness is uniform + correct.)
        Core::BigIntOfI64 { value } => {
            if int_ty_of(db, value).ground_signed() {
                emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [x]
                // A NARROW signed source (Int8/16/32) lives in an i32 SLOT, but `bigint-of-i64` takes an i64
                // — sign-extend the i32 to i64 first, or the module fails wasm validation ("expected i64,
                // found i32"). A full-width Int64 is already i64 and this is a no-op. (`emit_box_i32_to_i64_
                // extend` is the same widen every `box-int` payload site uses; it extends by the source sign.)
                emit_box_i32_to_i64_extend(db, value, out); // [x : i64]
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // → [bigint handle : i32]
                return Ok(());
            }
            // UNSIGNED source: materialize `[sign=0][8 LE magnitude bytes]` then `bigint-of-bytes`. Stash the
            // u64 value in a scratch i64 local so each byte-extraction re-reads it (the operand emits once).
            let val_slot = *high;
            *high = val_slot + 1;
            scratch_ty.insert(val_slot, ValType::I64);
            emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [x]
            // A NARROW unsigned source (UInt8/16/32) is in an i32 slot — ZERO-extend to i64 before stashing
            // in the i64 scratch (else the `local.set` type-mismatches, and the high magnitude bytes would be
            // garbage). `emit_box_i32_to_i64_extend` zero-extends an unsigned narrow int; a full-width UInt64
            // is already i64 (no-op). This keeps the byte-extraction loop reading a correct 64-bit magnitude.
            emit_box_i32_to_i64_extend(db, value, out); // [x : i64]
            out.push(Lir::LocalSet(val_slot)); // [] — x stashed
            out.push(Lir::ConstI32(9)); // [9] — 1 sign byte + 8 magnitude bytes
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            out.push(Lir::ConstI32(0)); // [buf, index=0]
            out.push(Lir::ConstI32(0)); // [buf, 0, sign=0 (non-negative)]
            out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            for i in 0..8u32 {
                out.push(Lir::ConstI32((i + 1) as i32)); // [buf, index=i+1]
                out.push(Lir::LocalGet(val_slot)); // [buf, i+1, x]
                out.push(Lir::ConstI64((i * 8) as i64)); // [buf, i+1, x, shift]
                out.push(Lir::I64ShrU); // [buf, i+1, x >>u (8*i)] — LOGICAL shift (magnitude, not sign)
                out.push(Lir::I32WrapI64); // [buf, i+1, low32] — wrap keeps the low byte in the low bits
                out.push(Lir::ConstI32(0xFF)); // [buf, i+1, low32, 0xFF]
                out.push(Lir::I32And); // [buf, i+1, byte_i]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            }
            out.push(Lir::CallImport(OP_BIGINT_OF_BYTES)); // consumes buf → [fresh owned BigInt handle : i32]
            Ok(())
        }
        // `Int64.of b` on a runtime BigInt — checked narrow back to i64 (traps out of range at run time).
        // `bigint-to-i64-checked` BORROWS its operand (`unbox_bigint` reads without consuming) and returns
        // an i64 scalar, so an OWNED-temporary operand must be dropped after the read (a borrowed param/
        // local is left to its owner) — the `value-eq` reclamation discipline for a borrowing op.
        Core::BigIntToI64 { operand } => emit_bigint_borrow_unary(
            db,
            operand,
            OP_BIGINT_TO_I64_CHECKED,
            high,
            slots,
            scratch_ty,
            layout,
            out,
        ),
        // A runtime BigInt `+`/`-`/`*`/`/` — the runtime op BORROWS both operand handles (`unbox_bigint`)
        // and returns a FRESH owned result handle, so each OWNED-temporary operand is dropped after the
        // call while the new result is kept. A borrowed param/local operand is NOT dropped (its owner
        // reclaims it). Same borrow-and-reclaim shape as `value-eq`, but the result is a handle to keep.
        Core::BigIntBinOp { op, lhs, rhs } => {
            let import = match op {
                crate::core::BigIntOp::Add => OP_BIGINT_ADD,
                crate::core::BigIntOp::Sub => OP_BIGINT_SUB,
                crate::core::BigIntOp::Mul => OP_BIGINT_MUL,
                crate::core::BigIntOp::Div => OP_BIGINT_DIV,
                crate::core::BigIntOp::Rem => OP_BIGINT_REM,
            };
            emit_bigint_borrow_binary(db, lhs, rhs, import, high, slots, scratch_ty, layout, out)
        }
        // A runtime BigInt COMPARISON `<`/`>`/`<=`/`>=`/`=` — `bigint-cmp` BORROWS both operands and
        // returns the three-way `-1`/`0`/`1` (`a<b`/`a=b`/`a>b`) as an i64; the operator is then the
        // SIGNED i64 compare of that against `0`: `< → cmp <ₛ 0`, `> → cmp >ₛ 0`, `<= → cmp <=ₛ 0`,
        // `>= → cmp >=ₛ 0`, `= → cmp == 0` (`i64.eqz`). The borrowing binary helper emits the operands,
        // drops each OWNED temporary, and leaves the i64 `cmp` result; then push the compare-with-zero.
        Core::BigIntCmp { op, lhs, rhs } => {
            emit_bigint_borrow_binary(
                db,
                lhs,
                rhs,
                OP_BIGINT_CMP,
                high,
                slots,
                scratch_ty,
                layout,
                out,
            )?; // → [cmp : i64]
            match op {
                Prim::Eq => out.push(Lir::I64Eqz), // cmp == 0
                Prim::Lt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LtS);
                }
                Prim::Gt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GtS);
                }
                Prim::Le => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LeS);
                }
                Prim::Ge => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GeS);
                }
                // `lower_bigint_cmp` only builds `BigIntCmp` for a comparison prim; anything else is a
                // compiler invariant violation.
                _ => {
                    return Err(Reject::decline("BigIntCmp carries a non-comparison prim"));
                }
            }
            Ok(())
        }
        // `Rational.of n d` on runtime ints — widen EACH int operand to a BigInt leaf (`bigint-of-i64`),
        // then `rational-of` (which CONSUMES both BigInt handles, normalizes, and builds the 2-handle
        // rational node; traps on a zero denominator at run time). Both `num`/`den` are i64 scalars (no
        // heap ref), so nothing to drop — the two fresh BigInt handles are consumed by `rational-of`.
        Core::RationalOfInts { num, den } => {
            emit(db, num, slots, base, high, scratch_ty, layout, out)?; // [num : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big : i32]
            let den_base = base.max(*high);
            emit(db, den, slots, den_base, high, scratch_ty, layout, out)?; // [num-big, den : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big, den-big]
            out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle : i32]
            Ok(())
        }
        // `Rational.of-int n` — the whole rational `n/1`: widen `n` and the constant `1` to BigInt, then
        // `rational-of`. `n` is an i64 scalar.
        Core::RationalOfIntWiden { value } => {
            emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [n : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [n-big : i32]
            out.push(Lir::ConstI64(1));
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [n-big, 1-big]
            out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle]
            Ok(())
        }
        // A runtime Rational `+`/`-`/`*`/`/` — the runtime op BORROWS both operand handles and returns a
        // FRESH normalized rational; each OWNED-temporary operand is dropped after the call. Same
        // borrow-and-reclaim shape as the BigInt arithmetic (reuses `emit_bigint_borrow_binary`, which is
        // op-generic — it just threads two handle operands, drops owned temporaries, and leaves the result).
        Core::RationalBinOp { op, lhs, rhs } => {
            let import = match op {
                crate::core::RationalOp::Add => OP_RATIONAL_ADD,
                crate::core::RationalOp::Sub => OP_RATIONAL_SUB,
                crate::core::RationalOp::Mul => OP_RATIONAL_MUL,
                crate::core::RationalOp::Div => OP_RATIONAL_DIV,
            };
            emit_bigint_borrow_binary(db, lhs, rhs, import, high, slots, scratch_ty, layout, out)
        }
        // A runtime Rational COMPARISON — `rational-cmp` (three-way `-1`/`0`/`1`) BORROWS both operands,
        // then the operator's signed i64 compare-with-zero (`=`→`i64.eqz`), exactly like `BigIntCmp`.
        Core::RationalCmp { op, lhs, rhs } => {
            emit_bigint_borrow_binary(
                db,
                lhs,
                rhs,
                OP_RATIONAL_CMP,
                high,
                slots,
                scratch_ty,
                layout,
                out,
            )?; // → [cmp : i64]
            match op {
                Prim::Eq => out.push(Lir::I64Eqz),
                Prim::Lt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LtS);
                }
                Prim::Gt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GtS);
                }
                Prim::Le => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LeS);
                }
                Prim::Ge => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GeS);
                }
                _ => {
                    return Err(Reject::decline("RationalCmp carries a non-comparison prim"));
                }
            }
            Ok(())
        }
        // `Bytes.compact(b)` — emit the handle, `bytes-compact` (consumes it, returns a content-equal one).
        // `Bytes.compact` realizes the memory model's storage-independence guarantee: it derives from a
        // value (a `bytes-slice`/`bytes-concat` rope that may RETAIN a large backing buffer through a small
        // window) another value EQUAL to it whose storage is independent (a fresh flat leaf holding only the
        // window's bytes), so the larger buffer can be released — changing storage use without changing the
        // value.
        //= spec/capabilities/memory-and-resource-model.md#retained-storage-is-what-a-value-s-representation-holds-live
        //# A program MUST be able to derive from a value another value equal to it whose storage is independent of the storage the value was derived from, so that a value retaining a small part of a larger value's storage can release the larger value's storage, changing storage use without changing the value.
        Core::BytesCompact { operand } => {
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [b]
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // → [compacted]
            Ok(())
        }
        // A runtime `String.to-bytes(string)` — the TOTAL UTF-8 encoding `String → Bytes`. A String IS a
        // UTF-8 Bytes leaf (byte-identical), so the encoding is a no-op re-view — but the string may be a
        // `String.concat`/`.slice` ROPE whose node `raw` holds header bytes, not content; the result must be
        // a well-formed Bytes value (a nested rope compares/keys WRONG under the tagless heap walk unless
        // flattened AT CONSTRUCTION — the canonicalize-at-construction invariant). `bytes-compact` does
        // exactly that (flatten the rope to a canonical flat leaf, CONSUMES the handle, transfers it out), so
        // it is the whole op — the exact inverse of `str-from-bytes` on well-formed input. No `sum-new`
        // (total, unlike the fallible decode), no `dup` (the handle is owned out of `bytes-compact`).
        Core::StrToBytes { string } => {
            emit(db, string, slots, base, high, scratch_ty, layout, out)?; // [string]
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // → [flat Bytes leaf] (consumes string)
            Ok(())
        }
        // A runtime `String.from-bytes(bytes)` — the TOTAL UTF-8 decode. Emit the bytes handle,
        // `str-from-bytes` (CONSUMES it; strict UTF-8 validate → the buffer AS a String handle, or NULL when
        // invalid), then build `Some(handle)` / `None`. The returned handle is already OWNED (str-from-bytes
        // transfers the buffer out on success), so it is used DIRECTLY as the `Some` payload — no `dup`. On
        // failure the runtime already dropped the buffer, so the `None` branch has nothing to drop. One
        // scratch slot (the result handle i32) above `base`; the operand recursion floats above it.
        Core::StrFromBytes {
            bytes,
            disc_some,
            disc_none,
        } => {
            let result_slot = base;
            if result_slot + 1 > *high {
                *high = result_slot + 1;
            }
            scratch_ty.insert(result_slot, ValType::I32);
            emit(db, bytes, slots, base + 1, high, scratch_ty, layout, out)?; // [buf]
            out.push(Lir::CallImport(OP_STR_FROM_BYTES)); // [handle-or-NULL] (consumes buf)
            out.push(Lir::LocalSet(result_slot)); // result_slot = handle-or-NULL, stack empty
            // valid = (handle != NULL).
            out.push(Lir::LocalGet(result_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [valid]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(handle). The handle is OWNED (str-from-bytes transferred the buffer out); use it
            // directly as the payload under `disc_some`, no `dup`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(result_slot)); // [disc_some, handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: the unit payload is the inline-unit constant (as `Map.lookup`/`Bytes.at` do). The
            // buffer was consumed+dropped by str-from-bytes on failure, so there is nothing to release here.
            out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // A runtime `Bytes.slice(bytes, start, len)` — the fallible sub-range read. Bounds-check `start >=
        // 0 && len >= 0 && start <= bytes-len && len <= bytes-len - start` (all i64), then in bounds build
        // `Some(bytes-slice(bytes, start, len))` — `bytes-slice` CONSUMES its buffer, but `bytes-len` above
        // only BORROWED it, so the one owned ref is consumed exactly by the slice — else `None`. The slice
        // result is a Bytes HANDLE, the `Some` payload directly (no box, unlike `at`'s byte value). Four
        // scratch slots (bytes i32, start i64, len i64, byte-count i64) above `base`; operand recursions
        // float above them.
        //
        // ⚠ The predicate is OVERFLOW-SAFE. The naive `start + len <= bytes-len` OVERFLOWS: for
        // attacker-chosen `start`/`len` near i64::MAX the i64 sum wraps to a negative value that trivially
        // passes the signed `<=`, wrongly taking the in-range path (a wrong `Some`, or a trap when the
        // i32-wrapped index exceeds the runtime's u32 range) — a soundness hole in a FALLIBLE op that
        // promises `None`, never a trap. Instead, once `start >= 0` holds, `bytes-len - start` (with the
        // `start <= bytes-len` guard, so the difference is in `[0, bytes-len]`) cannot underflow — a small
        // u32-extended non-negative i64 — and `len <= bytes-len - start` is the same range test with no
        // add, so no sum ever overflows. Mirrors the const-fold path's i128 check (`lower_bytes_slice`).
        Core::BytesSlice {
            bytes,
            start,
            len,
            disc_some,
            disc_none,
        } => {
            let bytes_slot = base;
            let start_slot = base + 1;
            let len_slot = base + 2;
            let bytelen_slot = base + 3;
            if bytelen_slot + 1 > *high {
                *high = bytelen_slot + 1;
            }
            scratch_ty.insert(bytes_slot, ValType::I32);
            scratch_ty.insert(start_slot, ValType::I64);
            scratch_ty.insert(len_slot, ValType::I64);
            scratch_ty.insert(bytelen_slot, ValType::I64);
            emit(db, bytes, slots, base + 4, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalSet(bytes_slot));
            emit(db, start, slots, base + 4, high, scratch_ty, layout, out)?; // [start:i64]
            out.push(Lir::LocalSet(start_slot));
            emit(db, len, slots, base + 4, high, scratch_ty, layout, out)?; // [len:i64]
            out.push(Lir::LocalSet(len_slot));
            // Materialize the byte-count ONCE (i64, u32-extended — read three times below).
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [len:i32]
            out.push(Lir::I64ExtendI32U); // [byte-count:i64]
            out.push(Lir::LocalSet(bytelen_slot));
            // in_bounds = (start >= 0) & (len >= 0) & (start <= byte-count) & (len <= byte-count - start),
            // all i64 — OVERFLOW-SAFE (no `start + len`; `byte-count - start` cannot underflow once
            // `start >= 0 & start <= byte-count`).
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start >= 0]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start>=0, len>=0]
            out.push(Lir::I32And); // [start>=0 & len>=0]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64LeS); // [.., start <= byte-count]
            out.push(Lir::I32And); // [start>=0 & len>=0 & start<=byte-count]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::I64Sub); // [.., len, byte-count - start]  (no underflow: start in [0, byte-count])
            out.push(Lir::I64LeS); // [.., len <= byte-count - start]
            out.push(Lir::I32And); // [in_bounds]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(bytes-slice(bytes, start, len)). The operand emit produced ONE owned ref;
            // `bytes-len` above only BORROWED it (rc unchanged), so that one ref is still live and
            // `bytes-slice` CONSUMES it exactly — net zero, no dup, no leak.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(bytes_slot)); // [disc_some, bytes]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, start:i32]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, start, len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes bytes)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `bytes-slice` was NOT called, so the operand's one owned ref is still live —
            // DROP it (the None path does not consume the bytes) to avoid a leak.
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_DROP)); // release the un-consumed bytes reference
            out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
            // The `None` (nullary) variant's unit payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a
            // runtime `arr-alloc(0)` CALL — the runtime's `arr-alloc(0)` returns exactly `imm_unit()`, so
            // pushing the derived constant is equivalent and drops one import call per `None` (the same
            // optimization the `SumNew` nullary path already uses; this brings the `List.at`/`Map.lookup`/
            // `String.at`/`Bytes.at` None arms to parity).
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc_none, unit-payload]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // A runtime PROJECTION `(. t i)` — read element `i` off the operand's array handle and UNBOX it
        // to its scalar: `<operand handle> ; i32.const i ; arr-get ; get-<T>`. The result type (this
        // node's solved type) chooses the unbox op.
        Core::Proj { operand, index } => {
            // RECLAMATION (U13/U14): if the projected `operand` is a fresh OWNED temporary (a peer/host call
            // result, a constructor — `heap_operand_ownership` == Owned) rather than a BORROW of a live
            // binding (a `Param`/`LocalRef`, reclaimed by its owner), the aggregate would otherwise LEAK —
            // `arr-get` borrows it to read the element but nothing releases it. Two element cases:
            //   • SCALAR element (`get_op` Some): `get-int`/`get-bool` COPY the value out, so after the read
            //     the parent can be dropped directly (U13).
            //   • NESTED-COMPOUND element (`get_op` None): the `arr-get` result IS the child handle, a BORROW
            //     into the parent. `dup` the child (rc++) so it survives the parent's drop, THEN drop the
            //     parent — the parent's storage + every OTHER child is reclaimed, and the returned child
            //     stays live under its own retained reference (U14).
            // Stash the aggregate in a scratch i32 slot so it survives the read for the post-read drop.
            // `heap_operand_ownership` declines an operand whose ownership it cannot prove, so an unhandled
            // shape rejects (Owned only on a proven-fresh producer), never leaks wrongly or double-frees.
            let scalar_elem = get_op(db, id)?;
            // A UNIT element: `arr-get` yields the inline-unit sentinel, but a `Unit` projection leaves NO
            // machine value (`valtype_of(Unit) = None`), so the sentinel must be DROPPED. Distinguished
            // from a nested-compound element (both classify as `None`) by this node's solved type — and a
            // Unit is never a live-compound FBIP target, so it skips the child-dup/retain logic below.
            let unit_elem =
                scalar_elem.is_none() && matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let agg_slot = base;
                if agg_slot + 1 > *high {
                    *high = agg_slot + 1;
                }
                scratch_ty.insert(agg_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalTee(agg_slot)); // [handle], agg_slot = the owned aggregate
                out.push(Lir::ConstI32(index as i32)); // [handle, i]
                out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle] (BORROWS the aggregate)
                match scalar_elem {
                    _ if unit_elem => {
                        // Unit element: discard the sentinel `arr-get` yielded, then release the owned
                        // aggregate — the projection leaves NOTHING on the stack.
                        out.push(Lir::Drop); // [] (drop the inline-unit sentinel)
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // [] (aggregate reclaimed)
                    }
                    Some(op) => {
                        out.push(Lir::CallImport(op)); // → [scalar (i64 for an int, i32 for a bool)]
                        if needs_get_int_narrow(db, id) {
                            out.push(Lir::I32WrapI64);
                        }
                        // The scalar is now a COPY on the stack; release the owned aggregate (rc--, cascades).
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // [scalar] (unchanged; aggregate reclaimed)
                    }
                    None => {
                        // A NESTED-COMPOUND element: retain the returned child (rc++) so it outlives the
                        // parent, then drop the parent. `dup` POPS its handle, so re-read the child from a
                        // scratch slot for the dup and leave the original copy on the stack as the result.
                        let child_slot = base + 1;
                        if child_slot + 1 > *high {
                            *high = child_slot + 1;
                        }
                        scratch_ty.insert(child_slot, ValType::I32);
                        out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                        out.push(Lir::LocalGet(child_slot)); // [child, child]
                        out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // drop the parent; the dup'd child survives → [child]
                    }
                }
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [handle]
            out.push(Lir::ConstI32(index as i32)); // [handle, i]
            out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
            // A scalar element unboxes (`get-int`/`get-bool`, then a NARROW int narrows i64→i32); a
            // nested compound: the handle `arr-get` yields IS the nested compound — use it as-is. (A
            // nested-compound projection of a BORROWED aggregate is kept alive by the aggregate's owner not
            // being dropped while the projection escapes — see `binding_escapes`'s nested-compound Proj arm,
            // which treats such a projection as an ESCAPE of the operand so its owner is not reclaimed.)
            if let Some(op) = scalar_elem {
                out.push(Lir::CallImport(op)); // → [scalar (i64 for an int, i32 for a bool)]
                if needs_get_int_narrow(db, id) {
                    out.push(Lir::I32WrapI64);
                }
            } else if unit_elem {
                // A Unit element: drop the inline-unit sentinel `arr-get` yielded — the projection leaves
                // NO machine value. (Never an FBIP-retained child, so it never reaches the dup branch.)
                out.push(Lir::Drop);
            } else if out.dup_sites.contains(&id) {
                // PERCEUS RETAIN of the projected CHILD (`collect_dup_sites`/`mark_binder_dups` marked this
                // consuming nested-compound projection of a still-live binder): the `arr-get` returned the
                // child as a BORROW (no rc++), so the child's rc is 1 (only the parent's array cell). A
                // consuming op (`vec-push`/…) would FBIP-mutate it in place, corrupting a LATER re-projection
                // that reads the same child. `dup` the child (rc++ → 2) so the consumer takes the persistent
                // copy path and the parent's array stays intact for the later read; the consumer's own drop
                // (or the persistent path's drop of its taken ref) reclaims the extra reference. `dup` POPS
                // its arg and returns nothing, so re-materialize the child from a scratch slot: tee it, dup
                // the copy, leave the original on the stack for the consumer.
                let child_slot = base;
                if child_slot + 1 > *high {
                    *high = child_slot + 1;
                }
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
            }
            Ok(())
        }
        // A sum-variant pattern's payload binder — WALK the access `path` from the scrutinee handle
        // (`sum-payload` per `Payload` step, `arr-get i` per `Elem` step), then unbox the leaf by THIS
        // node's solved type. A single `[Payload]` path is the flat `(Some x)` case; `[Payload, Payload]`
        // is the nested `(Some (Some y))` binder.
        Core::SumPayload { scrutinee, path } => {
            // SHARED-PREFIX FAST PATH: if a proper prefix of this path (ending in `Payload`) was
            // materialized into a slot for this arm body (`collect_sum_payload_prefixes` +
            // `emit_sum_cont`), start from that slot's payload handle and walk only the SUFFIX — instead of
            // re-emitting `<scrutinee> …prefix`. The prefix ends in `Payload`, so the slotted value is a
            // payload handle (`cur = Ty::Any`). Take the LONGEST matching slotted prefix. Only prefixes
            // whose shared extensions are borrowing `Elem` reads are ever recorded (see the collector), so
            // the slot holds a borrowed handle this suffix walk only reads.
            let prefix_hit = (0..path.len()).rev().find_map(|k| {
                out.payload_prefix_slots
                    .get(&(scrutinee, path[..k].to_vec()))
                    .map(|&s| (k, s))
            });
            // A step's array read depends on the CURRENT sub-value's kind: a tuple/record/sum-payload is a
            // flat `arr` (`arr-get`), but a `List` is an RRB `vec` (`vec-get`), and a list REST binder
            // slices the tail with `vec-split`. Track the sub-value type as the walk descends.
            let walk_from;
            let mut cur;
            // The absolute path PREFIX walked so far — used to consult `sum_path_types` (the enclosing
            // switch's recorded entered-variant payload types) so a `Payload` step resolves to the ACTUAL
            // entered variant, not variant 0. When starting from a shared-prefix slot, seed the prefix with
            // the skipped steps so the key stays absolute-from-scrutinee.
            let mut walked_prefix: Vec<crate::core::PathStep>;
            if let Some((k, slot)) = prefix_hit {
                out.push(Lir::LocalGet(slot)); // [payload-handle] — the shared prefix, computed once
                walk_from = k;
                walked_prefix = path[..k].to_vec();
                // The slotted prefix ends in `Payload`; recover its recorded type so a following `Elem`
                // picks the right accessor (else a bare payload handle, `Any`).
                cur = out
                    .sum_path_types
                    .get(&path[..k])
                    .cloned()
                    .unwrap_or(Ty::Any);
            } else {
                emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
                walk_from = 0;
                walked_prefix = Vec::new();
                cur = type_of(db, scrutinee);
            }
            for step in &path[walk_from..] {
                walked_prefix.push(*step);
                match step {
                    crate::core::PathStep::Payload => {
                        out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // → [payload-handle]
                        // The payload's TYPE — a variant carrying a `List` needs a following `Elem` to read
                        // it with `vec-get`, not `arr-get`. Resolve via the ENTERED variant recorded by the
                        // enclosing switch (`sum_path_types`), falling back to variant 0 at the root. This
                        // is what makes a nested list-in-payload element binder `(Ast.List (list x .. r)) →
                        // x` — where `List` is a NON-variant-0 variant — read with `vec-get`; without the
                        // recorded type, variant 0's payload (`Int64`) mis-picked `arr-get` on the RRB vec.
                        cur = match cur.strip_nominal() {
                            Ty::Sum { .. } => payload_step_ty_of(
                                db,
                                Some(scrutinee),
                                &cur,
                                &walked_prefix,
                                &out.sum_path_types,
                            ),
                            // A nominal newtype's `Payload` step is a static unwrap to its inner type.
                            inner => inner.clone(),
                        };
                    }
                    crate::core::PathStep::Elem(i) => {
                        // STRIP nominal before the List check: for an ERASED newtype whose `Payload` step is
                        // elided at resolve time (the path is `[Elem(0)]`, no leading `Payload`), `cur` is
                        // the raw `Ty::Nominal` wrapping the list — an unstripped `matches!(cur, List)` missed
                        // it and mis-emitted `arr-get` on a vec handle (reads garbage/0). `(type Box (Bx
                        // (List Int64)))` matched `(Bx (list x .. r)) → x` is exactly this shape.
                        if matches!(cur.strip_nominal(), Ty::List(_)) {
                            out.push(Lir::ConstI32(*i as i32));
                            out.push(Lir::CallImport(OP_VEC_GET)); // list element → vec-get
                            cur = match cur.strip_nominal() {
                                Ty::List(e) => (**e).clone(),
                                _ => Ty::Any,
                            };
                        } else {
                            out.push(Lir::ConstI32(*i as i32));
                            out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
                            cur = Ty::Any;
                        }
                    }
                    crate::core::PathStep::RestFrom(k) => {
                        // Tail sublist from `k`: `vec-drop(list, k)` returns the `[k, len)` tail as ONE
                        // handle (dropping the `[0, k)` prefix internally). ⚠ `vec-drop` CONSUMES its
                        // argument (rc--). But the handle on the stack here is a BORROW of the shared match-
                        // arm scrutinee slot (from `emit(scrutinee)` = a plain `local.get`), and a SIBLING
                        // binder in the SAME arm may still read it — e.g. `((list x .. rest) (f rest (+ acc
                        // x)))`, where `x` is a `vec-get` (BORROW) off the same handle. If `vec-drop` runs
                        // first and drops the arm handle's last reference, the vector is reclaimed and the
                        // sibling `x` read returns garbage (0) — a MISCOMPILE (the head element reads 0).
                        // So RETAIN the handle before consuming: rc++ it (`dup`) so `vec-drop` decrements a
                        // fresh reference and the arm handle's own count is unchanged (every co-binder + the
                        // arm's end-of-scope drop still sees a live handle). `dup` POPS a handle (rc++,
                        // returns nothing), so RE-READ the scrutinee's slot for the extra reference rather
                        // than teeing into a new scratch slot — the scrutinee is a `Core::Param`/`LocalRef`
                        // (the materialized arm handle) with a stable slot, so `emit(scrutinee)` is a pure
                        // `local.get`. This needs NO fresh scratch slot (which could alias a sibling
                        // element binder's i64 slot in a multi-binder pattern `(list a b .. rest)`).
                        // Stack here: [handle] (the copy vec-drop will consume). Push another read, dup it
                        // (rc++, pops it), leaving the original copy for vec-drop.
                        emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle, handle]
                        out.push(Lir::CallImport(OP_DUP)); // pops the 2nd read, rc++ → [handle]
                        out.push(Lir::ConstI32(*k as i32));
                        out.push(Lir::CallImport(OP_VEC_DROP)); // → [tail-handle]
                    }
                }
            }
            // A scalar leaf unboxes; a compound handle is used as-is; a UNIT payload binder drops the
            // inline-unit sentinel the walk landed on (a `Unit` binder holds no machine value).
            let unboxed = get_op(db, id)?;
            // PERCEUS RETAIN of the extracted COMPOUND CHILD (`collect_dup_sites`/`mark_binder_dups` marked
            // this consuming payload extraction of a still-live scrutinee — the `SumPayload` arm there): the
            // `sum-payload`/`arr-get` walk returned the child as a BORROW (no rc++), so its rc is 1 (only the
            // scrutinee's cell refs it). A consuming op (`List.push`/…) would FBIP-mutate it in place,
            // corrupting the still-live scrutinee (matched again / threaded to a self-call). `dup` the child
            // (rc++ → 2) so the consumer takes the persistent copy path and the scrutinee's payload stays
            // intact; the consumer's own drop reclaims the extra reference. Only a COMPOUND leaf (`unboxed`
            // None AND not Unit — a real handle) aliases; a scalar `unboxed` COPIES out and is never a marked
            // site. ⚠ `get_op` returns `None` for BOTH a compound handle AND a `Unit` payload (Unit has no
            // machine value — the walk lands on the `IMM_UNIT` sentinel that `emit_heap_read_tail` DROPS). A
            // Unit has no heap cell to alias, so it must NOT take the dup fast path (which would `dup` +
            // return, leaving the sentinel un-dropped → an extra stack value → INVALID WASM). Route Unit
            // through `emit_heap_read_tail` as usual. `dup` POPS its arg and returns nothing, so tee the child,
            // dup the copy, leave the original for the consumer.
            let unit_leaf = matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            if unboxed.is_none() && !unit_leaf && out.dup_sites.contains(&id) {
                let child_slot = base;
                if child_slot + 1 > *high {
                    *high = child_slot + 1;
                }
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
                return Ok(());
            }
            emit_heap_read_tail(db, id, unboxed, out); // → [scalar | handle | nothing]
            Ok(())
        }
        // `Option.expect` / `Result.expect` on a RUNTIME sum — probe the discriminant; on the PRESENT
        // variant (`disc_present`) read the payload + unbox by the result type, else TRAP (`unreachable`).
        // Materialize the scrutinee ONCE into a fresh i32 slot (a computed scrutinee — a `checked-add` —
        // must not be recomputed for the disc probe and the payload read; a reusable param/local is read
        // from its own slot). Then: `sum-disc == disc_present` selects an `if` whose THEN reads
        // `sum-payload` (+ `get-*` unbox, narrowing an i32 int) and whose ELSE is `unreachable` (the
        // absent-variant trap — textless; core-semantics.md §Requiring The Value Of An Optional Traps On
        // Absence). Both `sum-disc`/`sum-payload` BORROW the handle (no rc change), like a match probe.
        Core::Trap => {
            // `trap` — an unconditional divergence. `unreachable` HALTS and leaves the stack polymorphic,
            // so it validates in ANY result position (the runtime counterpart of `trap`'s `Never` type,
            // exactly as `SumExpect`'s absent branch emits below). The `String` message is already dropped
            // at lowering (the wasm trap carries no text).
            out.push(Lir::Unreachable);
            Ok(())
        }
        Core::SumExpect {
            scrutinee,
            disc_present,
        } => {
            // The sum handle is read TWICE — the disc probe (`sum-disc`) and the present-payload read
            // (`sum-payload`), BOTH BORROWING (no rc change, never consume). HANDLE SLOT REUSE (mirrors
            // `MatchSum`/`List.at`/`MatchList`): a REUSABLE handle — a `Param`/kept `let`-`LocalRef` already
            // resident in a stable slot — is read from its OWN slot directly; no copy. A COMPUTED scrutinee
            // (a call/`if`/fresh construction) is stashed ONCE into a fresh i32 slot reserved ABOVE the
            // running high-water (`*high`), NOT at `base`: when this `SumExpect` is a SUB-EXPRESSION whose
            // SIBLING uses `base` for a different width — `(tuple (AInt (Option.expect …)) (+ i 1))`, where
            // the i64 `(+ i 1)` sibling also starts scratch at `base` — reusing `base` for the i32 handle
            // re-types a slot the sibling `local.set`s at i64, an invalid module. A slot at `*high` is
            // guaranteed never pre-typed. Either way, reading the slot twice evaluates the scrutinee EXACTLY
            // ONCE.
            let handle_slot = match reusable_handle_slot(db, scrutinee, slots) {
                Some(owner) => owner,
                None => {
                    let handle_slot = *high;
                    *high = handle_slot + 1;
                    scratch_ty.insert(handle_slot, ValType::I32);
                    emit(
                        db,
                        scrutinee,
                        slots,
                        handle_slot + 1,
                        high,
                        scratch_ty,
                        layout,
                        out,
                    )?;
                    out.push(Lir::LocalSet(handle_slot));
                    handle_slot
                }
            };
            // The result block type is this node's solved type (the payload type).
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "expect result type has no machine representation",
                        ));
                    }
                },
            };
            // disc(handle) == disc_present ?  (`disc_present == 0` → `i32.eqz`, one instruction — the
            // sum-disc eqz special case; a `Some`/`Ok` present variant is discriminant 0.)
            out.push(Lir::LocalGet(handle_slot));
            out.push(Lir::CallImport(OP_SUM_DISC)); // [disc]
            if disc_present == 0 {
                out.push(Lir::I32Eqz); // [present?]
            } else {
                out.push(Lir::ConstI32(disc_present as i32));
                out.push(Lir::I32Eq); // [present?]
            }
            out.push(Lir::If(block_ty));
            // THEN — the present payload: sum-payload + unbox by result type. A scalar unboxes; a compound
            // is used as-is; a UNIT result drops the inline-unit sentinel so the THEN leaves NOTHING —
            // matching the `BlockType::Empty` a Unit result selects above (else a stray handle would defy
            // the block's declared type).
            out.push(Lir::LocalGet(handle_slot));
            out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [payload-handle]
            let unboxed = get_op(db, id)?;
            // PERCEUS RETAIN of the extracted COMPOUND child (mirrors the `SumPayload` emit): a marked site
            // (`mark_binder_dups`' `SumExpect` arm) is a compound payload consumed while its scrutinee stays
            // live, so `dup` the child (rc++) before it flows into the consuming op — else the shared payload
            // is FBIP-mutated at rc==1 and the still-live scrutinee drifts. Only a COMPOUND leaf (`unboxed`
            // None AND not Unit — a real handle) aliases; a scalar unboxes/copies and is never a site. ⚠
            // `get_op` returns `None` for BOTH a compound handle AND a `Unit` payload; a Unit has no heap cell
            // to alias and its `IMM_UNIT` sentinel must be DROPPED by `emit_heap_read_tail` — taking the dup
            // fast path would leave the sentinel un-dropped → an extra stack value in the block → INVALID
            // WASM (Copilot PR#441). Route Unit through the `else`. `dup` POPS + returns nothing, so tee the
            // child, dup the copy, leave the original for the consumer. A fresh scratch slot at `*high`
            // (never `base`, which a width-different sibling may claim).
            let unit_leaf = matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            if unboxed.is_none() && !unit_leaf && out.dup_sites.contains(&id) {
                let child_slot = *high;
                *high = child_slot + 1;
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
            } else {
                emit_heap_read_tail(db, id, unboxed, out); // [scalar | handle | nothing]
            }
            out.push(Lir::Else);
            // ELSE — absent variant: trap. `unreachable` leaves the stack polymorphic, so the block's
            // declared result type validates without a produced value.
            out.push(Lir::Unreachable);
            out.push(Lir::End);
            Ok(())
        }
        Core::If { cond, then_, else_ } => {
            let result = type_of(db, id);
            // FLOW-SENSITIVE DEAD-BRANCH ELIMINATION: when the active branch refinement DECIDES this `if`'s
            // condition (`(if (> n 0) (if (> n 0) …) …)` — the inner cond is known true; `(if (>= n 5) (if
            // (> n 0) …))` — implied), emit ONLY the taken branch and drop the other. The condition is a
            // comparison of a refined (trap-free) variable against a constant, so evaluating it has no
            // effect to preserve. Fires before the select/if lowering so a fully-decided nested conditional
            // costs nothing (not even a `select` on a constant). The taken branch keeps this `if`'s tail
            // context via `emit` (non-tail arm).
            if let Core::Compare { op, lhs, rhs } = core_of(db, cond)
                && let Some(taken) = crate::lower::refined_comparison_const(db, op, lhs, rhs)
            {
                let branch = if taken { then_ } else { else_ };
                trace!(target: "rcdzc::select", node = id.0, taken, "if condition decided by branch refinement — emit only the taken branch");
                return emit_branch(
                    db, branch, &result, slots, base, high, scratch_ty, layout, out,
                );
            }
            // FLOW-SENSITIVE EQUAL-BRANCH COLLAPSE: when both branches reduce to the SAME constant UNDER
            // their respective branch refinements — `(if (> x 10) (if (> x 5) 7 8) 7)`: the then-branch's
            // inner `(> x 5)` is decided true by the `x > 10` refinement, so it is `7`, matching the else —
            // the whole `if` is that constant, provided the condition is trap-free (it is still evaluated,
            // so a trapping cond must stay). This is the emit-time analogue of `lower`'s `core_equiv(then,
            // else)` fold, which only sees branches equal WITHOUT flow facts; `refined_const_value` applies
            // each branch's refinement to expose the equality `lower` could not. Each branch's refined
            // constant is computed under its own pushed frame (as the real emit below would).
            if crate::lower::is_trap_free(db, cond) {
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let tc = refined_const_value(db, then_);
                db.pop_range_refinements();
                if let Some(tc) = tc {
                    let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                    db.push_range_refinements(else_frame);
                    let ec = refined_const_value(db, else_);
                    db.pop_range_refinements();
                    if ec.as_ref() == Some(&tc) {
                        // Both branches are the same constant — emit it, dropping the (trap-free) branch.
                        trace!(target: "rcdzc::select", node = id.0, "if with equal refined-constant branches → the constant (trap-free cond dropped)");
                        let cid = crate::lower::synth_core(db, tc, result.clone());
                        return emit_branch(
                            db, cid, &result, slots, base, high, scratch_ty, layout, out,
                        );
                    }
                }
            }
            // BRANCHLESS SELECT: when both branches are cheap trap-free scalar computations (a
            // param/local/constant, or a small trap-free op like `(& x 7)` — see `is_select_arm`) and the
            // result is a SCALAR (not unit, not a heap handle), emit wasm's `select` instead of an
            // `if`/`else`/`end` block — no branch. `select` pops `[a, b, cond]` and pushes `a` if `cond` is
            // nonzero else `b`, evaluating BOTH unconditionally; that is sound here precisely because each
            // arm is trap-free, allocation-free, effect-free, and cheap (so little is wasted vs the branch
            // it replaces). A HEAP result is excluded: `select` would evaluate both handles and discard one
            // WITHOUT the Perceus `drop` that the owning branch would run, leaking its cell — the `if`
            // (which evaluates only the taken branch) stays for those. This is the classic `min`/`max`/
            // conditional-value idiom `(if (< a b) a b)` and the masked/bitwise conditional
            // `(if c (& x 7) (| x 8))`.
            // BOOLEAN MATERIALIZATION first: `(if c 1 0)`/`(if c 0 1)` is the condition itself (coerced to
            // the result width), cheaper than the `const;const;select` a leaf select would emit.
            if let Some(r) = try_bool_materialization(
                db, cond, then_, else_, &result, slots, base, high, scratch_ty, layout, out,
            ) {
                return r;
            }
            if !matches!(result, Ty::Unit)
                && (!is_heap_type(&result) || ty_is_enum_disc(db, &result))
                && valtype_of(&result).is_some()
                && is_select_arm(db, then_)
                && is_select_arm(db, else_)
            {
                // An ENUM-DISC result is admitted alongside a scalar (see the tail `Core::If` arm): its
                // runtime rep is an i32 discriminant and each enum-disc arm emits just that constant (no
                // allocation, no drop), so a `select` between two discriminants is sound.
                // Each arm is emitted UNDER its branch-refinement frame, exactly as the structured `if`
                // below — a `select` arm computes the same value the `if` arm would, so a refinement that
                // simplifies the arm (elides a redundant mask `(& x 255)` under `x∈[0,255]`, folds a
                // now-constant subexpression) must still apply. Sound: a trap-free arm carries no guard to
                // wrongly elide, the TAKEN arm's refinement always holds (so its refined value is its true
                // value), and the untaken arm's value is discarded by `select` regardless of whether its
                // refinement held. So refining both arms is strictly better (branchless AND still elided).
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let then_res = emit_branch(
                    db, then_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                then_res?;
                let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                db.push_range_refinements(else_frame);
                let else_res = emit_branch(
                    db, else_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                else_res?;
                emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Select);
                return Ok(());
            }
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
            // The branches start their scratch ABOVE the high-water the COND reached, NOT at `base`. A
            // cond may stash an i32 HEAP HANDLE in a scratch slot (a runtime `value-eq`/`MatchSum` on
            // constructed sums) that stays TYPED for the whole function; a branch reusing that slot at a
            // different width (an i64 arith temp — `(if (= (mk n) (mk 3)) n (find (+ n 1)))`) would force
            // one wasm local to two types and fail validation (`expected i64, found i32`). Advancing to
            // `*high` hands each branch fresh, never-typed slots — the same discipline `MatchSum`'s arms
            // and `emit_call_args` already apply. (A scalar cond leaves `*high == base`, so this is a
            // no-op for the common case and the emitted bytes are unchanged.)
            let branch_base = *high;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            // FLOW-SENSITIVE RANGE REFINEMENT: while emitting a branch, push a refinement frame recording
            // any one-sided bound this branch's condition establishes on a variable (`(< n 2)` → `n ≤ 1`
            // in `then`, `n ≥ 2` in `else`). A guard-elision check inside the branch (`value_range` →
            // `arith_provably_in_range`) then sees the narrowed range and drops a dead overflow guard
            // (`(- n 1)` under `n ≥ 2` cannot underflow). Frame merges the parent's (nested `if`s
            // accumulate). Pushed/popped around EACH branch so the refinement never leaks past it — and
            // popped even on an early `?` return (capture the result, pop, then `?`).
            let base_frame = db.current_refinements();
            // Both branches must produce the `if`'s RESULT machine slot; a bare-literal branch (default
            // Int64) opposite a NARROW branch would otherwise push a mismatched i64 into a narrow-i32
            // block. Ground a bare-`ConstInt` branch to the result's integer width via `emit_operand`,
            // exactly as an operator operand (`@1a4528f`) and a match arm (`@10f7bdb`) are grounded.
            let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
            db.push_range_refinements(then_frame);
            let then_res = emit_branch(
                db,
                then_,
                &result,
                slots,
                branch_base,
                high,
                scratch_ty,
                layout,
                out,
            );
            db.pop_range_refinements();
            then_res?;
            out.push(Lir::Else);
            // The else branch starts its scratch ABOVE the then branch's high-water (see the TAIL `Core::If`
            // arm for the full rationale): the two mutually-exclusive branches may want the same slot index
            // at DIFFERENT widths (a base arm's i32 Option handle vs a recursive arm's i64 temp), and a
            // slot's type is recorded ONCE — sharing `branch_base` sets one local at both types (invalid
            // module). Advancing past `*high` gives the else branch fresh slots; byte-identical when the
            // then branch used no scratch (`*high == branch_base`).
            let else_base = branch_base.max(*high);
            let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
            db.push_range_refinements(else_frame);
            let else_res = emit_branch(
                db, else_, &result, slots, else_base, high, scratch_ty, layout, out,
            );
            db.pop_range_refinements();
            else_res?;
            out.push(Lir::End);
            Ok(())
        }
        // A scalar MATCH → a chain of `if`s. The match's solved type is each arm's block-result type.
        // Each non-wildcard arm probes `scrutinee == literal` (push scrutinee, push the literal, compare)
        // and takes its body on a match, else falls through to the next arm; the wildcard arm is the
        // unconditional tail (`else`). The scrutinee is a scalar, so re-pushing it per probe is a cheap
        // local reload — no naming needed.
        Core::Match { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "match result type has no machine representation",
                        ));
                    }
                },
            };
            let it = int_ty_of(db, scrutinee);
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_match_arms(
                db, scrutinee, &arms, it, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out,
            )
        }
        // A sum MATCH → a chain of `if`s over `sum-disc(scrutinee)`. Each variant arm probes
        // `sum-disc(scrutinee) == disc` and takes its body on a match; a wildcard/binder arm (`disc:
        // None`) is the unconditional `else` tail. The scrutinee is a heap handle (an i32 local reload
        // per probe, cheap). A payload binder in a body reads `sum-payload(scrutinee)` on its own
        // (`Core::SumPayload`), so the arm dispatch needs only the disc.
        Core::MatchSum { scrutinee, root } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "sum match result type has no machine representation",
                        ));
                    }
                },
            };
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            // A REUSABLE scrutinee (a param/local already in a slot) is re-read cheaply per probe. A
            // NON-reusable one (a `List.at`, a call, an `if` — anything computed) must be evaluated ONCE:
            // re-emitting it per probe/payload would recompute the value (rebuilding a list, re-running a
            // call) AND its own scratch would clash with the arm bodies' scratch at the shared `base`,
            // producing an invalid module. Materialize it into a fresh i32 slot (a sum is a heap handle)
            // and record `(scrutinee → slot)` so every re-reference reads the slot (top of `emit`).
            let (arms_slots, arms_base) = if reusable_handle_src(db, scrutinee, slots) {
                (slots.clone(), base)
            } else {
                // Reserve a FRESH slot for the scrutinee (an i32 heap handle) ABOVE the running
                // high-water — NOT `base` itself. When this `MatchSum` is an OPERAND of an enclosing op
                // (`(* (match (Bytes.at b i) …) …)`), `base` is that op's operand-scratch floor, which the
                // enclosing arith already TYPED (i64 for its operand/result slot); reusing `base` for the
                // i32 handle would re-type a slot the enclosing op `local.set`s at i64 — an invalid module
                // (`expected i64, found i32`). A slot at `*high` is guaranteed never pre-typed. Emit the
                // scrutinee's value above THAT (its own transient scratch — a `List.at`/`Bytes.at` types
                // slots for the buffer/index/element), then start the arm scratch above the high-water the
                // scrutinee emit reached (`*high`), so an i32 scrutinee-scratch slot never clashes with an
                // i64 arm temp.
                let slot = *high;
                *high = slot + 1;
                scratch_ty.insert(slot, ValType::I32);
                emit(
                    db,
                    scrutinee,
                    slots,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                let mut m = slots.clone();
                m.insert(scrutinee, slot);
                (m, (*high).max(slot + 1))
            };
            emit_sum_cont(
                db,
                scrutinee,
                &root,
                result_it,
                block_ty,
                &arms_slots,
                arms_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::NonTail,
            )
        }
        // A runtime LIST match → dispatch by LENGTH. Read `vec-len(scrutinee)` once, then a chain of
        // `if (len <cond>) then <arm-body> else …`. Each arm's element/rest binders read the list on their
        // own (`SumPayload` `Elem`/`RestFrom` → `vec-get`/`vec-split`). The scrutinee is materialized ONCE
        // into a fresh i32 slot so every arm-body binder re-reads the SAME handle. Exhaustiveness (checked
        // in `lower`) guarantees the last arm is a catch-all, so the innermost `else` runs unconditionally.
        Core::MatchList { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "list match result type has no machine representation",
                        ));
                    }
                },
            };
            let (arm_slots, len_slot, arm_base) = materialize_list_match_scrutinee(
                db, scrutinee, slots, high, scratch_ty, layout, out,
            )?;
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_list_arms_tailable(
                db,
                &arms,
                len_slot,
                block_ty,
                result_it,
                &arm_slots,
                arm_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::NonTail,
            )
        }
        // A parameter reference — read its local slot. The slot was assigned in `select_function`; a
        // reference to a binder with no slot is either a `Unit` param (elided from the signature — Unit
        // occupies no slot, so reading it pushes nothing, the read analogue of `Core::Unit`) or a
        // compiler bug (a represented param not in the signature), so decline in the latter case rather
        // than emit a wrong `local.get`.
        Core::Param { binder } => match slots.get(&binder) {
            Some(&slot) => {
                emit_binder_ref(id, slot, out);
                Ok(())
            }
            None if matches!(type_of(db, binder).strip_nominal(), Ty::Unit) => Ok(()),
            None => Err(Reject::decline("parameter reference has no local slot")),
        },
        // An A-normal binding sequence: give each binding a PERSISTENT local slot (unlike the reused
        // scratch pool, a binding's value must survive across every `LocalRef` to it), emit its value
        // ONCE into that slot, then emit the body reading the slots. This is where naming pays off:
        // a value used N times is computed once here and read N times as `local.get`. Slots are
        // allocated at the current scratch floor; the floor rises past them so the body's scratch does
        // not clobber a live binding. A `let*` binding's value may reference an earlier binding, so the
        // extended slot map carries the earlier bindings when a later value is emitted.
        Core::Let { bindings, body } => {
            let mut extended = slots.clone();
            let mut floor = base;
            // Track each HEAP-typed binding as `(binder, slot)`, to `drop` after the body UNLESS it
            // escapes (Perceus). A kept binding is always a genuine runtime value — a constant tuple
            // folds and is never kept (H2c) — so every heap binding here is an owned allocation whose
            // reference is released once its scope ends, or transferred out if it escapes. (A scalar
            // binding owns no heap cell → no drop.)
            let mut heap_bindings: Vec<(StructId, u32)> = Vec::new();
            for (binder, value) in &bindings {
                let slot = floor;
                let ty = type_of(db, *binder);
                // The binding's machine value type — read off its solved type (the value's type). A
                // binding whose type has no machine rep (a compound/unresolved value) declines.
                let vt = valtype_of(&ty).ok_or_else(|| {
                    Reject::decline("a let binding's type has no machine representation")
                })?;
                // RESERVE the binding slot BEFORE emitting the initializer: record its type and lift `*high`
                // past it now. The initializer emits at `slot + 1`, but many emit sites float their own
                // scratch off `*high` (a tuple/record element `elem_base = *high`, an `if`-branch base, a
                // call arg) — those all ASSUME `*high >= base`. Without pre-reserving, `*high` still LAGS at
                // the pre-let value (< `slot + 1`), so an initializer whose value is an `if`/compound would
                // hand its inner scratch the binding's OWN slot: `(let ((r (if … (tuple (+ p 5) …) …)))
                // (+ (. r 0) (. r 1)))` teed the i64 `(+ p 5)` into slot `r` (declared i32, the tuple
                // handle) → invalid wasm (`expected i32, found i64`). Reserving keeps every inner scratch
                // strictly above the binding slot. (`scratch_ty` for `slot` is set here; the `LocalSet`
                // below stores into it. A `match`-producer initializer already worked because `MatchSum`
                // pre-advances `*high` for its own handle slot — this brings the general `let` in line.)
                scratch_ty.insert(slot, vt);
                if slot + 1 > *high {
                    *high = slot + 1;
                }
                // Emit the value into scratch ABOVE this persistent slot (its own scratch floats), then
                // store it once. The value sees the earlier bindings via `extended`.
                emit(
                    db,
                    *value,
                    &extended,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                if is_heap_type(&ty) {
                    heap_bindings.push((*binder, slot));
                }
                // DEBUG (D3 locals): a SCALAR binding with a source name lives in this slot for its whole
                // scope — record it so a `DW_TAG_variable` DIE lets a debugger `print` the local. (A heap
                // binding is a handle DWARF can't walk (§3), so only scalars are recorded.) The binder key
                // is the initializer occurrence, so recover the name from its `(name init)` pair.
                if matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
                    && let Some(name) = db.let_binding_name(*binder)
                {
                    out.binding_local(slot, name.to_string(), ty.clone());
                }
                extended.insert(*binder, slot);
                // The body emits ABOVE both this binding slot AND any scratch the INITIALIZER used (its
                // transient slots are recorded in `scratch_ty` at a fixed TYPE; a body reusing one at a
                // different type would re-type a wasm local → invalid module — e.g. a runtime-`(bin …)`
                // scrutinee initializer uses an i64 `val` slot, and the match body reuses it as an i32).
                // `*high` tracks the top slot touched so far. For a scalar/handle initializer with no
                // scratch, `*high == slot+1`, so this is byte-identical to before.
                floor = (slot + 1).max(*high);
            }
            // The body computes its value (left on the stack). A heap binding is RECLAIMED
            // (`local.get <slot>; drop`) only if it is DEAD after the body — used solely in BORROWING
            // positions (a `Core::Proj` operand: `arr-get` borrows). A binding that ESCAPES — its
            // reference flows into the RETURNED value: it IS the result (`(let ((t …)) t)`), or an
            // element of a constructed tuple (`arr-set` CONSUMES it), or a call argument — is NOT
            // dropped: its ownership transfers to the caller (the ownership-transfer-on-return rule of
            // Perceus — `value-heap-runtime.md` §the ordering obligation). Dropping an escaped value
            // would be a use-after-free / double-free; `binding_escapes` is conservative (any non-borrow
            // use → escapes → keep), so we never drop a live value.
            emit(db, body, &extended, floor, high, scratch_ty, layout, out)?;
            for &(binder, slot) in &heap_bindings {
                if !binding_escapes(db, body, binder, false) {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            Ok(())
        }
        // A reference to a kept `let` binding — read its persistent slot, exactly like a parameter. The
        // slot was assigned when the enclosing `Core::Let` was emitted; a `LocalRef` with no slot is a
        // compiler bug (a ref lowered as `LocalRef` without its binding kept), so decline.
        Core::LocalRef { binder } => match slots.get(&binder) {
            Some(&slot) => {
                emit_binder_ref(id, slot, out);
                Ok(())
            }
            None => Err(Reject::decline("let-binding reference has no local slot")),
        },
        // A runtime CALL — push each argument (in the CALLER's frame, so an arg's own scratch floats
        // above `base`), then `call` the callee's absolute wasm-function index (its position in the
        // layout's emission order). The callee is reachable (`layout` added it), so its index exists; a
        // callee not in the emission order is a compiler bug (reachability missed it) → decline.
        Core::Call { callee, args } => {
            emit_call_args(
                db, callee, &args, slots, base, high, scratch_ty, layout, out,
            )?;
            match layout.abs(callee) {
                Some(idx) => {
                    trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit runtime call");
                    out.push(Lir::Call(idx));
                    Ok(())
                }
                None => Err(Reject::decline(
                    "a called function is not in the emission order (reachability gap)",
                )),
            }
        }
        // A runtime comparison: emit both operands, then the machine comparison selected from the
        // operands' width AND signedness (`_s` for a signed type, `_u` for an unsigned one; `eq` is
        // sign-agnostic). The result is an i32 boolean. A comparison never overflows, so both operands
        // share the same scratch base (each is dead once pushed).
        Core::Compare { op, lhs, rhs } => {
            // FLOW-SENSITIVE COMPARISON FOLD: when the enclosing branch has REFINED a variable operand's
            // range so this comparison against a constant is already decided (`(if (> n 0) (if (> n 0) …)
            // …)` — the inner test is known true; or an IMPLIED test `(if (>= n 5) (if (> n 0) …))`), emit
            // the constant bool directly instead of recomputing the compare. Refinements live only during
            // emit, so `lower`'s `fold_comparison_at_type_bound` could not see this; the runtime operand is
            // a refined `Param`/`LocalRef` (trap-free), so discarding it drops no trap. Result is an i32
            // bool (`0`/`1`), exactly what a comparison leaves.
            if let Some(r) = crate::lower::refined_comparison_const(db, op, lhs, rhs) {
                trace!(target: "rcdzc::select", node = id.0, result = r, "comparison folded to a constant by the active branch refinement");
                out.push(Lir::ConstI32(r as i32));
                return Ok(());
            }
            // Both operands must share the comparison's machine slot; ground a bare-literal operand to
            // the shared width so `(> x 50)` over a narrow `x` does not push an i64 beside `x`'s i32.
            let it = operand_int_ty(db, lhs, rhs);
            // EQUALITY-WITH-ZERO → `eqz`. `(= x 0)` (or `(= 0 x)`) is the shape of every recursion base
            // case `(if (= n 0) …)`; `x == 0` is exactly wasm's `i32.eqz`/`i64.eqz` (1 if the operand is
            // zero) — one instruction instead of pushing a `0` constant and an `eq` (three). This is
            // INSTRUCTION SELECTION at the site where the semantics ("compare with zero") are known — not
            // a byte peephole re-recognizing `const 0 ; eq` after the fact. A CONSTANT-vs-constant `= 0`
            // would already have folded in `lower`, so here the non-zero operand is a runtime value.
            if op == Prim::Eq
                && let Some(nonzero) = eq_zero_operand(db, lhs, rhs)
            {
                // DIVISIBILITY-BY-POWER-OF-TWO: `(= (% x 2^k) 0)` — the "is x divisible by 2^k?" test (the
                // even test `(= (% x 2) 0)` is the k=1 case) — is exactly `(x & (2^k − 1)) == 0`, for BOTH
                // signednesses (a number is divisible by 2^k iff its low k bits are zero, regardless of
                // sign). Emitting `x & mask ; eqz` skips the whole `%` — for a SIGNED `%` that is the ~10
                // instruction round-toward-zero bias sequence (`emit_div_rem`), collapsed to 3. Verified
                // `(x % 2^k == 0) ≡ (x & (2^k−1) == 0)` exhaustively over signed inputs and every k. Only
                // fires when the divisor is a compile-time power of two > 1 (`k=1` divisor `2` even test;
                // `%1` folds to 0 in `lower`, never reaching here).
                if let Some((x, mask)) = rem_pow2_mask(db, nonzero) {
                    emit_operand(db, x, it, slots, base, high, scratch_ty, layout, out)?;
                    out.push(if it.ground_width() <= 32 {
                        Lir::ConstI32(mask as i32)
                    } else {
                        Lir::ConstI64(mask)
                    });
                    out.push(if it.ground_width() <= 32 {
                        Lir::I32And
                    } else {
                        Lir::I64And
                    });
                    out.push(if it.ground_width() <= 32 {
                        Lir::I32Eqz
                    } else {
                        Lir::I64Eqz
                    });
                    return Ok(());
                }
                emit_operand(db, nonzero, it, slots, base, high, scratch_ty, layout, out)?;
                out.push(if it.ground_width() <= 32 {
                    Lir::I32Eqz
                } else {
                    Lir::I64Eqz
                });
                return Ok(());
            }
            // Both operands are simultaneously live on the stack for the compare, so — like sibling call
            // args and checked-arith's A/B — the RHS emits ABOVE the LHS's high-water, never reusing an
            // LHS scratch slot at a different width. An LHS that inlines a heap-match (`(= (cbor-major b
            // i) 4)` — `cbor-major` β-reduces to a `Bytes.at` MatchSum materializing an i32 handle) types
            // its slots i32; an RHS (or the LHS of a sibling compare) reusing them as an i64 arith temp
            // would re-type one wasm local to two widths → an invalid module. Floating the RHS above
            // `*high` hands it fresh, never-typed slots.
            emit_operand(db, lhs, it, slots, base, high, scratch_ty, layout, out)?;
            let rhs_base = base.max(*high);
            emit_operand(db, rhs, it, slots, rhs_base, high, scratch_ty, layout, out)?;
            out.push(compare_op(op, it));
            Ok(())
        }
        // RUNTIME FLOAT EQUALITY under the CANONICAL BYTE FORM — `nan == nan` TRUE, `-0.0 != +0.0`, all
        // NaN equal (core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form). NOT IEEE
        // `f64.eq` (which says `nan != nan` and `-0.0 == 0.0` — a miscompile). Each operand is
        // CANONICALIZED to its integer bit pattern with NaN folded to one canonical form
        // (`canon(x) = select(x != x, CANON_NAN_BITS, reinterpret_int(x))`), then the two canonical bit
        // patterns compare with an INTEGER `eq` — so any two NaNs compare equal and the sign bit of a zero
        // is significant. `x != x` is the isnan test (true only for NaN); a bare f64 param can carry a
        // non-canonical NaN across the host boundary, so the canonicalize is load-bearing (it matches
        // `op_box_float`'s runtime canonicalization and the constant fold's `to_f64_bits` basis).
        Core::FloatCompare {
            op,
            lhs,
            rhs,
            width,
        } => {
            if op == Prim::FEq {
                // EQUALITY: canonical-byte bit compare (see above).
                emit_canon_float_bits(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
                emit_canon_float_bits(db, rhs, width, slots, base, high, scratch_ty, layout, out)?;
                out.push(if width == 32 { Lir::I32Eq } else { Lir::I64Eq });
            } else {
                // ORDERING (`< <= > >=`): the RAW IEEE float compare (operator ruling — IEEE partialOrd).
                // `f64.lt`/etc. already give the wanted semantics: a NaN operand → 0 (unordered → false),
                // and `-0.0`/`+0.0` compare EQUAL (`f64.le -0.0 0.0` = 1). NO canonicalization — that's the
                // equality path; ordering DISAGREES with it on NaN + signed zero, by design. Emit each float
                // operand directly (grounded to the op width), then the raw compare op.
                emit_float_operand(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
                emit_float_operand(db, rhs, width, slots, base, high, scratch_ty, layout, out)?;
                out.push(float_ordering_op(op, width));
            }
            Ok(())
        }
        // RUNTIME STRUCTURAL EQUALITY on two COMPOUND heap values — a `value-eq` (`champ_eq`) call. The
        // op BORROWS both operands, so refcounts are balanced by DROPPING each OWNED-temporary operand
        // after the compare (a bare reference — a parameter/binding the owner reclaims — is NOT dropped).
        // Each operand is emitted, `tee`d into a scratch slot (kept on the stack for the call AND
        // remembered for a possible drop), then `value-eq` pops both and pushes the i32 bool; a drop of a
        // remembered owned handle leaves that bool on top. An operand whose ownership cannot be proved
        // (an `if`/`match`/`let` that may return a borrowed sub-value) DECLINES — reject, never a leak or
        // a double-free.
        // Canonicalizing every rope operand before the physical `champ_eq` is what makes a value that
        // SHARES another's storage (a `String.concat`/`Bytes.concat` rope over shared segments) and a value
        // that COPIES it (a flat leaf of the same content) compare EQUAL — indistinguishable by equality and
        // by the canonical byte form, exactly as the memory model requires. (The float leaf's twin of this
        // is `op_box_float`'s canonicalize-on-construct; here it is the rope compaction.)
        //= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
        //# A value that shares another value's storage and a value that copies it MUST be indistinguishable by every operation the executable semantics defines, including equality, length, indexing, and the value's canonical byte form, so that whether storage is shared is never observable.
        Core::ValueEq { lhs, rhs } => {
            // `value-eq` is `champ_eq` — a PHYSICAL-byte compare (the map-key contract). A runtime String OR
            // Bytes operand can be a ROPE (a `String.concat`/`Bytes.concat`/`.slice` lowers to a rope node),
            // whose bytes differ from a flat leaf of IDENTICAL content — so comparing a rope directly would
            // return the WRONG answer. CANONICALIZE EVERY String/Bytes operand with `bytes-compact` (a
            // content-equal flat leaf; a no-op-shaped pass on an already-flat value) before the compare, so a
            // rope and its flat twin compare equal — whether the operand is OWNED (a fresh `String.concat`/
            // `Bytes.concat`/`String.to-bytes` result) or BORROWED (a param / a `Map.lookup`/`SumPayload`-
            // extracted rope value, `(= s "…")` where `s` is a variant/map payload). `bytes-compact` FLATTENS
            // its argument IN PLACE and returns the SAME handle with an UNCHANGED refcount when it was already
            // owned-consuming — but critically, the runtime op is refcount-NEUTRAL (`op_bytes_compact` =
            // `bytes_flatten(buf); buf`): it mutates the node into a leaf (content-preserving, UNOBSERVABLE
            // even on a shared value per the memory model's #Sharing Is Not Observable) and hands the SAME
            // handle back. So compacting a BORROWED operand neither consumes it nor mints a new handle — the
            // borrow stays the owner's and is NOT dropped here; an OWNED operand's handle is likewise threaded
            // through and dropped after the borrowing compare. A non-text operand (a tuple/sum/map compound
            // handle) is NOT compacted — it is passed as-is (a String/Bytes NESTED inside a compound rides the
            // construction-site element compaction; only a DIRECT String/Bytes operand is canonicalized here,
            // which is why `compound_eq_heap_walkable` admits a DIRECT Bytes but `ty_heap_walkable` still
            // declines a nested one).
            let lo = heap_operand_ownership(db, lhs)?;
            let ro = heap_operand_ownership(db, rhs)?;
            // Compact ANY String/Bytes operand — owned OR borrowed. Since `bytes-compact` is refcount-neutral
            // (it flattens in place, returning the same handle), it is safe on a borrow: the flatten is
            // unobservable, and no drop follows a borrowed operand. This closes the rope miscompile for the
            // WHOLE class — previously only an OWNED String was compacted, so a genuine rope reaching `=`
            // through a BORROWED operand (a `Map.lookup`/`SumPayload` payload, or a runtime-rope param)
            // compared by its unflattened header bytes and silently returned the wrong answer.
            let lhs_str = operand_is_string_or_bytes(db, lhs);
            let rhs_str = operand_is_string_or_bytes(db, rhs);
            // Two i32 scratch slots for the operand handles, above the running high-water (they must not
            // clash with an operand emit's own transient scratch — a `Call` arg's i64 guard slot).
            let slot_l = *high;
            let slot_r = *high + 1;
            *high = slot_r + 1;
            scratch_ty.insert(slot_l, ValType::I32);
            scratch_ty.insert(slot_r, ValType::I32);
            let op_base = *high;
            emit(db, lhs, slots, op_base, high, scratch_ty, layout, out)?;
            // `bytes-compact` flattens the operand IN PLACE and returns the SAME handle (refcount-neutral),
            // so it is applied uniformly to an owned OR a borrowed String — the borrow is not consumed and
            // the returned handle carries the operand's original ownership through to the drop decision.
            if lhs_str {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope/flat → canonical flat leaf (in place)
            }
            out.push(Lir::LocalTee(slot_l));
            emit(db, rhs, slots, op_base, high, scratch_ty, layout, out)?;
            if rhs_str {
                out.push(Lir::CallImport(OP_BYTES_COMPACT));
            }
            out.push(Lir::LocalTee(slot_r));
            out.push(Lir::CallImport(OP_VALUE_EQ)); // pops both handles (borrowed) → [bool]
            // Drop each operand handle the compare borrowed but we OWN. Ownership is unchanged by the
            // compaction (`bytes-compact` returns the same handle), so drop iff the operand was OWNED to
            // begin with — an OWNED temporary (a constructor / call / concat result) leaks otherwise; a
            // BORROWED operand (param / kept-local / payload read) is left to its owner (dropping it would
            // be a double-free), whether or not it was compacted in place.
            if lo == HandleOwnership::Owned {
                out.push(Lir::LocalGet(slot_l));
                out.push(Lir::CallImport(OP_DROP));
            }
            if ro == HandleOwnership::Owned {
                out.push(Lir::LocalGet(slot_r));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // A runtime arithmetic op. The numeric model fixes each operation's DEFINED outcome, which the
        // emitted instruction must honor at run time exactly as the constant fold does (`numeric-
        // model.md`; the const path folds in `lower` with the SAME traps → CDZ0304). Every op is
        // WIDTH-GENERIC — the operand type's width `N` and signedness drive it, no hard-coded 64:
        //   - the value lives in a MACHINE slot `M` = i32 (N ≤ 32) or i64 (else), normalized (sign-/
        //     zero-extended) — so a machine op computes the exact result whenever it fits M bits;
        //   - `+`/`-`/`*`/`<<` are CHECKED in two composed steps: an M-OVERFLOW guard (carry/borrow/
        //     round-trip) traps when the true result exceeds the machine slot, leaving `r` EXACT; then a
        //     RANGE-CHECK traps when `r` fits `M` but not the narrower `[min_N, max_N]`. Together they
        //     trap iff the true result leaves the N-bit type — at any N (an Int8 `100+100`, a UInt48
        //     `*` past 2^48, an Int64 `+` past 2^63 all trap by the same recipe);
        //   - `/`/`%` map to `div_s`/`rem_s`/`div_u`/`rem_u`; wasm traps natively on ÷0 and (`div_s` at
        //     N==M) on `MIN/-1`; a NARROW signed `/` whose `min_N / -1` the machine op does not trap is
        //     caught by the same range-check;
        //   - `&`/`|`/`^` are total on the two's-complement value — just the operands then the op.
        // A runtime operand only arises from a boundary parameter, and only the aliased widths
        // (8/16/32/64) have a boundary representation — but the recipe is correct for any N in 1..=64,
        // so nothing here assumes a machine width. A constant of any width folds in `lower` instead.
        // A runtime FLOAT arithmetic op (`+.`/`-.`/`*.`/`/.`) — emit the two operands then the machine
        // `f64`/`f32` op at the result width (read off the solved type). IEEE, NEVER traps, so NO overflow
        // guard (unlike the integer arith below). Both operands share the result's float type (binary-op
        // unification), so they emit at the same width.
        Core::Arith { op, lhs, rhs } if op.is_float_arith() => {
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            trace!(target: "rcdzc::select", node = id.0, ?op, width, "emit runtime float op");
            // Ground each operand to the OP width: a bare float literal defaults to Float64 (an f64 slot),
            // which beside a Float32 operand would push an f64 into an `f32.add` and wasm rejects the
            // module. `emit_float_operand` materializes a literal at the op width (and demotes/promotes a
            // control-flow operand whose slot disagrees), the float analogue of the integer `emit_operand`.
            emit_float_operand(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
            emit_float_operand(db, rhs, width, slots, base, high, scratch_ty, layout, out)?;
            out.push(float_arith_op(op, width));
            Ok(())
        }
        Core::Arith { op, lhs, rhs } => {
            let m = Machine::of(int_ty_of(db, id));
            trace!(target: "rcdzc::select", node = id.0, ?op, width = m.width, signed = m.signed, "emit runtime integer op");
            match op {
                // STRENGTH REDUCTION: `x * 2^k` → `x << k`. A left shift IS exact multiplication by a
                // power of two, with the SAME defined overflow-trap (`numeric-model.md` §Overflow Is
                // Defined for shifts: "a left shift is exact multiplication by a power of two, so an
                // overflowing left shift MUST behave like an overflowing *"), so the rewrite preserves
                // BOTH value and trap at every width/signedness (verified: `* 8` and `<< 3` agree on
                // value and overflow-trap). The shift's overflow check is a cheap round-trip vs mul's
                // division; `k < width` is a compile-time constant so there is no count guard. Only for
                // `Mul` with a constant power-of-two operand ≥ 2 (`*1`/`*0` are handled by `lower`).
                Prim::Mul if mul_pow2_shift(db, lhs, rhs, m).is_some() => {
                    let (val, k) = mul_pow2_shift(db, lhs, rhs, m).unwrap();
                    emit_mul_pow2_as_shift(
                        db,
                        m,
                        val,
                        k,
                        slots,
                        base,
                        high,
                        scratch_ty,
                        layout,
                        out,
                        ResultDest::Stack,
                    )
                }
                Prim::Add | Prim::Sub | Prim::Mul => emit_checked_arith(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                // WRAPPING arithmetic — the RAW machine `add`/`sub`/`mul`, NO overflow guard (wasm's op
                // already wraps modulo the slot). At a NARROW width the result is masked to the width by the
                // ordinary operand/consumer normalization, exactly as a bitwise op's is.
                Prim::WrappingAdd | Prim::WrappingSub | Prim::WrappingMul => {
                    let ot = IntTy::fixed(m.signed, m.width);
                    emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    out.push(match op {
                        Prim::WrappingAdd => m.add(),
                        Prim::WrappingSub => m.sub(),
                        _ => m.mul(),
                    });
                    Ok(())
                }
                Prim::BitAnd | Prim::BitOr | Prim::BitXor => {
                    let ot = IntTy::fixed(m.signed, m.width);
                    // REDUNDANT-MASK ELISION (flow-sensitive): `(& v M)` where the constant `M` covers `v`'s
                    // whole provable range is just `v` — emit the value alone, drop the mask. This is the
                    // emit-time sibling of the `is_full_mask_for` lower fold; it fires where `lower` cannot,
                    // on a value the branch REFINEMENT bounds (`(if (and (>= x 0) (< x 256)) (& x 255) …)` →
                    // `x & 255 == x` under `x ∈ [0,255]`). `&` is total (no trap dropped) and the value
                    // operand is emitted so its own evaluation/traps stay.
                    if op == Prim::BitAnd
                        && let Some(v) = crate::lower::redundant_and_mask_value(db, lhs, rhs)
                    {
                        return emit_operand(db, v, ot, slots, base, high, scratch_ty, layout, out);
                    }
                    // OR-SATURATION ELISION (flow-sensitive): `(| v M)` where the constant `M` covers `v`'s
                    // whole provable range is just `M` — `v | M == M`, so emit the constant alone (`v`'s
                    // bits are all already set in `M`). The emit-time sibling of the `BitOr` OR-saturation
                    // lower fold, firing on a branch-REFINED `v` (`(if (and (>= x 0) (< x 256)) (| x 255) …)`
                    // → `255` under `x ∈ [0,255]`). DISCARDS `v` — `redundant_or_mask_const` already checked
                    // `v` is trap-free — so no defined trap is dropped.
                    if op == Prim::BitOr
                        && let Some(c) = crate::lower::redundant_or_mask_const(db, lhs, rhs)
                    {
                        return emit_operand(db, c, ot, slots, base, high, scratch_ty, layout, out);
                    }
                    emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    out.push(m.bitwise(op));
                    Ok(())
                }
                Prim::Div | Prim::Rem => emit_div_rem(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                Prim::Shl | Prim::Shr => emit_shift(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                // A comparison prim never reaches `Core::Arith` (it lowers to `Core::Compare`); a type
                // constructor never lowers as a runtime op. Decline rather than emit a wrong op.
                _ => Err(Reject::decline(
                    "not a runtime integer arithmetic operation",
                )),
            }
        }
        // A runtime integer CONVERSION. `wrap` TRUNCATES the operand to this node's target width/sign
        // (read off `type_of(id)`): move the operand into the target slot, keep its low N bits, and
        // reinterpret them at the target sign. Total (never traps) — the const path folds identically in
        // `lower`. This is the runtime sibling of `IntValue::wrap_to`.
        // INT→FLOAT conversion (`Float N.of-int`): emit the integer operand (an i64 machine value —
        // grounded to Int64 as the `of-int` source type) then `f{64,32}.convert_i64_s` at the target
        // float width. Handled BEFORE the integer-target `Machine::of` below (its target is a float).
        Core::Convert {
            op: Prim::FloatOfInt,
            operand,
        } => {
            let width = match type_of(db, id) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            emit_operand(
                db,
                operand,
                IntTy::i64(),
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(if width == 32 {
                Lir::F32ConvertI64S
            } else {
                Lir::F64ConvertI64S
            });
            Ok(())
        }
        // FLOAT-WIDTH conversion (`Float N.of`): emit the float operand, then demote/promote by the
        // SOURCE (operand) and TARGET (this node) widths — `f32.demote_f64` (64→32), `f64.promote_f32`
        // (32→64), or NOTHING (same width = identity). Handled before the integer-target arm below.
        Core::Convert {
            op: Prim::FloatOf,
            operand,
        } => {
            let src_w = match type_of(db, operand) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let dst_w = match type_of(db, id) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
            match (src_w, dst_w) {
                (64, 32) => out.push(Lir::F32DemoteF64),
                (32, 64) => out.push(Lir::F64PromoteF32),
                // same width — the conversion is the identity, no opcode.
                _ => {}
            }
            Ok(())
        }
        Core::Convert { op, operand } => {
            let src = Machine::of(int_ty_of(db, operand));
            let dst = Machine::of(int_ty_of(db, id));
            trace!(target: "rcdzc::select", node = id.0, ?op, from_width = src.width, to_width = dst.width, "emit runtime conversion");
            match op {
                Prim::Wrap => emit_wrap(
                    db, src, dst, operand, slots, base, high, scratch_ty, layout, out,
                ),
                _ => Err(Reject::decline("not a runtime conversion")),
            }
        }
        // A runtime boolean NEGATION `!operand` — emit the logical NOT of the operand (a Bool i32). From
        // the `(if c false true)` fold. `emit_negated_bool` folds `(not (CMP a b))` into the complement
        // comparison and otherwise emits `operand ; i32.eqz`.
        Core::Not { operand } => {
            emit_negated_bool(db, operand, slots, base, high, scratch_ty, layout, out)
        }
        // A SHORT-CIRCUITING boolean connective — emitted as an `if` over `lhs` (a Bool i32), so `rhs` is
        // evaluated on ONLY ONE branch (the shield core-semantics.md §Boolean Connectives Short-Circuit
        // requires): `and` → `if lhs then rhs else 0`; `or` → `if lhs then 1 else rhs`. The `if` yields an
        // i32 Bool. (A constant `lhs` folded in `lower`, so here `lhs` is a runtime bool.)
        Core::And { lhs, rhs, is_and } => {
            // BRANCHLESS BOOLEAN: when `rhs` can neither TRAP nor EFFECT, the short-circuit is unnecessary
            // — `and`/`or` become a bitwise `i32.and`/`i32.or`. Booleans are canonical i32 `0`/`1`, so
            // `p & q` IS the boolean AND and `p | q` IS the boolean OR; the only thing short-circuit
            // preserves is NOT evaluating `rhs` when `lhs` decides the result — and a trap-free,
            // effect-free `rhs` has nothing to skip, so evaluating it unconditionally is identical. This
            // covers a bare LEAF and — the common case — a COMPARISON `(and (< a b) (< c d))` or a
            // bitwise/`not`/`wrap` combination of leaves, all total (`is_branchless_bool_rhs`). A `rhs`
            // that could trap (a checked op, `/`), call, allocate, or effect KEEPS the short-circuit `if`
            // so it runs only when reached.
            if is_branchless_bool_rhs(db, rhs) {
                emit(db, lhs, slots, base, high, scratch_ty, layout, out)?;
                emit(db, rhs, slots, base, high, scratch_ty, layout, out)?;
                out.push(if is_and { Lir::I32And } else { Lir::I32Or });
                return Ok(());
            }
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?;
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            if is_and {
                // then: rhs ; else: false (0)
                emit(db, rhs, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Else);
                out.push(Lir::ConstI32(0));
            } else {
                // then: true (1) ; else: rhs
                out.push(Lir::ConstI32(1));
                out.push(Lir::Else);
                emit(db, rhs, slots, base, high, scratch_ty, layout, out)?;
            }
            out.push(Lir::End);
            Ok(())
        }
        // A runtime CLOSURE VALUE — a NO-CAPTURE closure is exactly its funcref-TABLE SLOT, an i32
        // constant. The element section maps slot `code` → the lifted function's wasm index, so pushing
        // the slot is the whole closure value (no heap cell — captures would add an `arr-alloc` here).
        // A runtime CLOSURE VALUE — a heap CELL `arr-alloc(1 + captures)`: slot 0 = `box-int(code)` (the
        // funcref-table slot), slots 1.. = each captured value (boxed if a scalar). Leaves the cell's u32
        // handle on the stack. Uniform shape for capturing AND non-capturing closures, so a fn-typed
        // parameter holds either interchangeably. Built exactly like a tuple (`arr-set` returns the array
        // handle for threading, so no scratch local needed).
        Core::Closure { code, captures } => {
            out.push(Lir::ConstI32(1 + captures.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [cell]
            // Slot 0: box-int(code). `box-int` takes an i64, so the table slot is an `i64.const`.
            out.push(Lir::ConstI32(0)); // index
            out.push(Lir::ConstI64(code as i64)); // the table slot (box-int wants i64)
            out.push(Lir::CallImport(OP_BOX_INT));
            out.push(Lir::CallImport(OP_ARR_SET)); // → [cell]
            // Slots 1..: each captured value, boxed if scalar (a narrow int extends i32→i64 first, like a
            // tuple element), arr-set into place.
            for (k, &cap) in captures.iter().enumerate() {
                out.push(Lir::ConstI32(1 + k as i32)); // index
                emit(db, cap, slots, base, high, scratch_ty, layout, out)?;
                // A scalar capture boxes; a compound is stored as-is; a UNIT capture holds the inline-unit
                // sentinel in its cell slot (the value pushed nothing).
                let boxed = box_op(db, cap)?;
                emit_heap_store_tail(db, cap, boxed, out);
                out.push(Lir::CallImport(OP_ARR_SET)); // → [cell]
            }
            Ok(())
        }
        // A runtime CLOSURE APPLICATION — `call_indirect` through the funcref table. The lifted function
        // is `(env, arg) -> result`: push the arg, then the env cell, then read the table slot from the
        // cell (`arr-get(cell, 0)` + `get-int`) as the indirection index, then `call_indirect`. The cell
        // must be materialized into a local so it is read TWICE (once passed as env, once for the code
        // slot) without recomputation.
        Core::CallClosure { closure, args } => {
            let type_index = match closure_type_index(db, closure, &args, layout) {
                Some(ti) => ti,
                None => {
                    // No lifted lambda has this application's machine signature. Distinguish a PROVABLY
                    // DEAD site from a merely UNSUPPORTED one by the CLOSURE OPERAND'S TYPE, not the
                    // application's arity: a runtime closure value arises ONLY from a lambda lift, so if
                    // NO lift can produce a value of the operand's full-curried type
                    // (`closure_operand_is_dead`), the operand holds no callable value and this
                    // application can NEVER execute — emit `unreachable` (validates by wasm's
                    // stack-polymorphic typing, traps only if somehow reached, which the type system
                    // forbids). This is the "two distinctly-typed boxed closures in ONE sum, only one
                    // ever built" shape: an iterator sum with both a binary `scan` accumulator and an
                    // element→sub-iterator `flat-map` closure — each `next` arm statically applies its
                    // boxed closure, but a program constructing only the `scan` variant never lifts the
                    // `flat-map` machine shape, so its arm's `call_indirect` is dead.
                    //
                    // When a lift DOES inhabit the operand's type but no lift matches the APPLICATION's
                    // arity (a curried multi-param closure lifted as nested unaries, applied at flattened
                    // higher arity — `(f 2 3)` over `(fn a (fn x …))`), the site is LIVE and must NOT be
                    // stubbed with `unreachable`; it declines as an unsupported application shape, exactly
                    // as before. A non-function / non-representable operand type also declines here.
                    let operand_ty = type_of(db, closure);
                    if closure_operand_is_dead(&operand_ty, layout) {
                        out.push(Lir::Unreachable);
                        return Ok(());
                    }
                    return Err(Reject::decline(
                        "a runtime closure application has no matching function type",
                    ));
                }
            };
            // Materialize the closure cell into a scratch local (read twice: env arg + code slot).
            let cell_slot = base.max(*high);
            *high = (*high).max(cell_slot + 1);
            scratch_ty.insert(cell_slot, ValType::I32);
            emit(
                db,
                closure,
                slots,
                cell_slot + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(Lir::LocalSet(cell_slot));
            // The lifted function is `(env, args…) -> result`, so push env (param 0) THEN each arg, in
            // order, before the indirection index. Each arg emits above the cell slot (never reusing it).
            out.push(Lir::LocalGet(cell_slot)); // env (the cell)
            for &arg in &args {
                emit(db, arg, slots, cell_slot + 1, high, scratch_ty, layout, out)?;
            }
            // …then the indirection index: arr-get(cell, 0) → box-int(code); get-int → the table slot as
            // an i64; `call_indirect` needs the index as an i32, so narrow it (`i32.wrap_i64`). The code
            // is a small table slot, so the wrap is exact.
            out.push(Lir::LocalGet(cell_slot));
            out.push(Lir::ConstI32(0));
            out.push(Lir::CallImport(OP_ARR_GET));
            out.push(Lir::CallImport(OP_GET_INT));
            out.push(Lir::I32WrapI64);
            out.push(Lir::CallIndirect(type_index));
            Ok(())
        }
        // A CAPTURED free-variable read inside a lifted closure body — `arr-get(env, 1 + index)` then
        // unbox by the captured value's type (a scalar `get-int`/`get-bool`, then a NARROW int narrows
        // i64→i32; a compound handle is used as-is). The env cell is the lifted function's local slot 0.
        // The node's own `type_of` is the captured value's type (set at lowering), so `get_op`/`is_narrow`
        // read it exactly as a tuple projection does.
        Core::Captured { index, .. } => {
            out.push(Lir::LocalGet(0)); // the env cell (lifted fn's 1st param)
            out.push(Lir::ConstI32(1 + index as i32));
            out.push(Lir::CallImport(OP_ARR_GET));
            // A scalar capture unboxes; a compound is used as-is; a UNIT capture drops the inline-unit
            // sentinel the cell slot held (a `Unit`-typed captured variable has no machine value).
            let unboxed = get_op(db, id)?;
            emit_heap_read_tail(db, id, unboxed, out);
            Ok(())
        }
        // A HOST CALL — a perform delegated to the component boundary. Emit each scalar argument (in
        // order), then `call <host-import-index>` (the imported op's core-func index, its position in the
        // program's host-import set, resolved via `layout.host_index`). A `Unit` argument occupies no
        // boundary slot, so it is skipped (a nullary op `(E.op)` pushes nothing). The op's scalar result
        // is left on the stack by the imported call.
        //
        // This is JUST an ordinary imported-function call: the program pushes its arguments and reads the
        // response the import leaves on the stack — nothing here encodes or observes how the host produces
        // that response (inline, suspend-and-resume, or abort). The emitted instruction is the same under
        // every host resolution strategy.
        //= spec/capabilities/capabilities-and-effects.md#a-host-call-returns-a-response
        //# A host call MUST be an ordinary call to an imported function that returns its response to the program, so that from the program's side reaching the host is a plain function call and how the host produces the response is the host's concern.
        //= spec/capabilities/capabilities-and-effects.md#a-host-call-returns-a-response
        //# The program MUST NOT observe or encode how a host call is resolved, so that whether the host answers inline, suspends the run and resumes it later, or aborts it is invisible to the program and not part of the program's meaning.
        Core::HostCall {
            effect, op, args, ..
        } => {
            // EFFECTS-UNIFICATION (U2): an escaping effect BOUND to a peer contract
            // (`db.effect_bindings`) is a PEER call — resolve it against the extern-import set and emit a
            // `CallExternImport`, exactly as a `Core::ExternCall` did. An unbound effect stays a host call.
            if let Some(iface) = db.effect_bindings.get(&effect).cloned() {
                let index = layout.extern_index(&iface, &op).ok_or_else(|| {
                    // The peer op is not in `extern_order` — which the RESOURCE-ESCAPE emit paths
                    // (`emit_runtime_resource`/`emit_recursive_sum_resource`) do not populate: they carry
                    // the runtime import but not the peer extern envelope. So a peer-bound op reached in a
                    // body whose ENTRYPOINT RESULT escapes as a runtime resource (the entrypoint RETURNS
                    // the compound/Option a peer produced) has no import to call. This is a known gap (a
                    // resource×peer-extern envelope fusion is the fix); until then, the workaround is to
                    // consume the peer's value into a SCALAR the entrypoint returns (e.g. read the field/
                    // element and return it, or `List.len`) rather than returning the raw compound, OR
                    // handle the effect in-program instead of binding it to a peer.
                    Reject::decline(format!(
                        "a peer-bound effect op (`{op}` on `{iface}`) is reached in an entrypoint whose \
                         RESULT escapes as a runtime resource (it returns the compound/collection the peer \
                         produced) — the resource-escape boundary does not yet carry the peer import. \
                         Consume the peer's value into a scalar the entrypoint returns, or handle the \
                         effect in-program instead of binding it to a peer"
                    ))
                })?;
                for &arg in &args {
                    if matches!(crate::infer::type_of(db, arg), Ty::Unit) {
                        continue;
                    }
                    emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
                }
                out.push(Lir::CallExternImport(index));
                return Ok(());
            }
            let index = layout.host_index(&effect, &op).ok_or_else(|| {
                Reject::decline("a host call's operation is not in the host-import set")
            })?;
            for &arg in &args {
                let at = crate::infer::type_of(db, arg);
                match at {
                    // A unit argument carries no boundary value.
                    Ty::Unit => continue,
                    // A STRING argument crosses as `(ptr, len)` — its constant bytes were laid in the core
                    // module's data segment at a known offset (`host_string_offset`), so push that ptr +
                    // the byte length. Only a CONSTANT string is supported (a runtime byte-rope is a later
                    // increment); a non-constant string arg declines.
                    Ty::String => {
                        let s = match core_of(db, arg) {
                            Core::ConstStr(s) => s,
                            _ => {
                                return Err(Reject::decline(
                                    "a host call with a non-constant string argument is not yet emitted",
                                ));
                            }
                        };
                        let offset = layout.host_string_offset(&s).ok_or_else(|| {
                            Reject::decline("a host-arg string was not laid in the data segment")
                        })?;
                        out.push(Lir::ConstI32(offset as i32));
                        out.push(Lir::ConstI32(s.len() as i32));
                    }
                    // A scalar argument emits its value directly.
                    _ => emit(db, arg, slots, base, high, scratch_ty, layout, out)?,
                }
            }
            out.push(Lir::CallHostImport(index));
            Ok(())
        }
        // (The `Core::ExternCall` emit arm was REMOVED in U4 — a peer op is now a peer-bound effect's
        // escaping `Core::HostCall`, which the `Core::HostCall` arm above emits as a `CallExternImport`
        // when the effect is peer-bound.)
        // A SEQUENCING block — emit each statement FOR ITS EFFECT (in order), then the tail as the value.
        // A statement is a host call whose result is `Unit` (it leaves NOTHING on the stack — a
        // `func()`-typed import), so emitting it needs no `drop`; a value-leaving statement is not produced
        // here yet (the `do`-fold only sequences Unit-returning host calls). The tail leaves the block's
        // value on the stack.
        Core::Seq { stmts, tail } => {
            for s in &stmts {
                // A statement must leave nothing on the stack (a Unit host call). Guard it: a non-Unit
                // statement would leave a dangling value (stack imbalance) — decline rather than emit it.
                if !matches!(crate::infer::type_of(db, *s), Ty::Unit) {
                    return Err(Reject::decline(
                        "a sequencing statement that leaves a value is not yet emitted (only a \
                         unit-returning host-call statement)",
                    ));
                }
                emit(db, *s, slots, base, high, scratch_ty, layout, out)?;
            }
            emit(db, tail, slots, base, high, scratch_ty, layout, out)
        }
        // The `?`/try boundary block + break are the `block`/`br` emit (BRICK 3): a `Core::Block` emits a
        // wasm `block` whose result type is `T_B`'s core repr, with each contained `Core::Break` emitting a
        // `br` to that block's label. BRICK 1 lays down the node + its non-emit arms; until BRICK 3 fills
        // the `block`/`br` bytes, emitting one is a clean decline (never wrong code).
        Core::Block { .. } | Core::Break { .. } => Err(Reject::decline(
            "the `?`/try boundary block/break does not emit yet (block/br lowering is the next brick)",
        )),
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}

/// Emit a scalar match as a chain of `if`s. `arms` is `[(probe, body)…]` in order; `it` is the
/// scrutinee's integer type (for the comparison op — a boolean scrutinee is compared as an i32). Each
/// LITERAL arm probes `scrutinee == literal` and takes its body on a match, else recurses on the
/// remaining arms in the `else`; a WILDCARD arm is the unconditional tail (emit its body, stop). The
/// scrutinee is re-emitted per probe (a scalar local reload — cheap and correct). `lower` guaranteed a
/// wildcard tail for a runtime match (exhaustiveness), so the chain always terminates in a body.
#[allow(clippy::too_many_arguments)]
fn emit_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    emit_match_arms_tailable(
        db,
        scrutinee,
        arms,
        it,
        result_it,
        block_ty,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        TailPos::NonTail,
    )
}

/// `emit_match_arms`, but with a [`TailPos`]: when the match is in TAIL position, each ARM BODY is a
/// tail position too — a tail call in an arm becomes `return_call`, or, when the enclosing function is
/// self-recursive (`TailPos::Tail(Some(tl))`), a SELF tail-call in an arm iterates the loop. The
/// scrutinee and the probe comparisons are never tail (they are values the dispatch reads).
#[allow(clippy::too_many_arguments)]
fn emit_match_arms_tailable(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    // RANGE-BASED DEAD-ARM ELIMINATION: an arm with an `Int` literal probe the scrutinee's provable range
    // EXCLUDES can never match — its `scrutinee == C` test is a compile-time `false`. Drop it, provided a
    // LATER arm still covers (dropping it cannot break exhaustiveness: `lower` proved the arms cover the
    // scrutinee's TYPE, and the range only removes values the type already covered, so the survivors still
    // cover every REACHABLE value). The match analogue of the range-vs-constant comparison fold —
    // `(match (& x 7) (100 a) (0 b) (_ c))` drops the dead `100` arm, and a flow-refined scrutinee
    // (`(match n …)` under `(> n 100)`) drops arms below the refinement. Sound to drop a GUARDED dead arm
    // too: a probe never true means the arm (guard and all) never runs. Done HERE — before the branchless
    // 2-arm-select and the probe chain — so BOTH paths see the filtered arms (a dead arm in a 2-arm match
    // must not force a `select` on a probe that is always false). Recurses with the kept arms only when the
    // filter removed something (else infinite recursion / wasted re-run); order preserved.
    //
    // ⚠ The probe's NUMERIC value (`to_i64()`), NOT its bit pattern (`to_i64_bits()`), is what `value_range`
    // reasons about: a wide UNSIGNED probe (`UInt64` `2^63`) has a NEGATIVE bit pattern that would falsely
    // read as "below [0, …]" and drop a LIVE arm — a miscompile. `to_i64()` is `None` for such a value (out
    // of i64), so the arm is conservatively KEPT.
    let arm_is_dead = |db: &mut Db, i: usize, a: &crate::core::MatchArm| -> bool {
        i + 1 < arms.len()
            && matches!(&a.probe, crate::core::Probe::Int(v)
                if v.to_i64().is_some_and(|c| crate::lower::value_excludes(db, scrutinee, c)))
    };
    if arms.len() > 1 && arms.iter().enumerate().any(|(i, a)| arm_is_dead(db, i, a)) {
        let mut kept: Vec<crate::core::MatchArm> = Vec::with_capacity(arms.len());
        for (i, a) in arms.iter().enumerate() {
            if !arm_is_dead(db, i, a) {
                kept.push(a.clone());
            }
        }
        trace!(target: "rcdzc::select", dropped = arms.len() - kept.len(), "match: dropped dead arms the scrutinee's range excludes");
        return emit_match_arms_tailable(
            db, scrutinee, &kept, it, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        );
    }
    // Resolve the scrutinee to a SOURCE pushed once per probe. A match dispatches by testing the
    // scrutinee against each arm's literal in turn — so the scrutinee is read once PER PROBE. If it is a
    // reusable value (a parameter/local, or a constant), re-pushing it each time is free. But a COMPUTED
    // scrutinee (`(match (+ a b) …)`) would be fully RE-EVALUATED per probe — recomputing the add AND
    // its overflow guard N times. So a non-reusable scrutinee is evaluated ONCE into a scratch slot here,
    // and every probe reads that slot. A scalar match's scrutinee is Int or Bool (an i32/i64 slot).
    let scrut_vt = match block_scalar_slot(db, scrutinee) {
        Some(vt) => vt,
        None => {
            return Err(Reject::decline(
                "match scrutinee has no machine representation",
            ));
        }
    };
    let (src, chain_base) = match reusable_scalar_src(db, scrutinee, slots) {
        // A reusable scrutinee is pushed in place at each probe — no scratch, the probe chain keeps the
        // full scratch region from `base`.
        Some(src) => (src, base),
        None => {
            // Evaluate the scrutinee ONCE into scratch slot `base`; the arm bodies and later probes run
            // ABOVE that live slot (it must survive every probe). The scrutinee's own emit uses `base+1`
            // as its floor, and may itself claim MORE scratch — a runtime `value-eq`/`MatchSum` scrutinee
            // (`(match (= (mk n) (mk 3)) …)`) stashes i32 heap handles in slots the high-water records.
            // So the probe chain starts at the high-water the scrutinee emit REACHED (`*high`), NOT a bare
            // `base+1`: reusing a scrutinee-scratch slot the value-eq typed i32 for a branch's i64
            // iteration arithmetic would force one wasm local to two types (invalid module). A scalar
            // scrutinee leaves `*high == base+1`, so `chain_base == base+1` and the bytes are unchanged.
            let slot = base;
            if slot + 1 > *high {
                *high = slot + 1;
            }
            scratch_ty.insert(slot, scrut_vt);
            emit(
                db,
                scrutinee,
                slots,
                base + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(Lir::LocalSet(slot));
            (OperandSrc::Slot(slot), *high)
        }
    };
    // DEBUG (D3 match-binder locals): a bare-binder arm (`(x body)`) binds the WHOLE scrutinee — which,
    // for a scalar match, lives in the single spill slot resolved above. Collect one local per DISTINCT
    // binder name across the arms (all alias that slot) so the backend emits a `DW_TAG_lexical_block`
    // scoping them to this match's PC range. Only a SLOT-backed scrutinee is describable (a constant
    // scrutinee folds; a re-pushed param/local is itself already a nameable var). `scope_start` anchors
    // the block at the first dispatch instruction; each `return` records the scope at the block's end.
    let scope_start = out.here();
    let binder_vars: Vec<LocalVar> = match src {
        OperandSrc::Slot(slot) => {
            let mut seen: Vec<String> = Vec::new();
            let mut vars = Vec::new();
            for arm in arms {
                let ty = type_of(db, arm.body);
                if !matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_)) {
                    continue;
                }
                if let Some(name) = db.match_arm_binder_name(arm.body)
                    && !seen.iter().any(|s| s == name)
                {
                    seen.push(name.to_string());
                    vars.push(LocalVar {
                        slot,
                        name: name.to_string(),
                        ty,
                        is_param: false,
                    });
                }
            }
            vars
        }
        _ => Vec::new(),
    };
    // BRANCHLESS 2-ARM SELECT: a match of exactly TWO UNGUARDED arms — a literal probe then a wildcard
    // (`(match n (0 a) (_ b))`), or a Bool's two literals (`(match p (true a) (false b))`) — is
    // `(if (scrutinee == probe0) body0 body1)`, so when both bodies are cheap trap-free SCALAR arms
    // (`is_select_arm` — a leaf, a small trap-free op like `(& x 7)`, or a shallow nested conditional,
    // exactly as the `if`→`select` conversion) and the result is a scalar it emits wasm's `select`
    // instead of an `if`/`else` block: `body0 ; body1 ; (scrutinee == probe0) ; select`. This is the
    // match analogue of the `if`→`select` rewrite and rests on the same soundness (a `select` evaluates
    // both operands, safe precisely because each arm is trap-/allocation-/effect-free). Excluded for a
    // heap/unit result (a `select` on a handle would drop-leak; unit has no value). TAIL position is fine
    // even though a `select` cannot carry a tail call: an `is_select_arm` body is trap-free, and a call is
    // never trap-free, so no arm is ever a tail call to preserve. A body that is a call / heavier op, a
    // guard, or >2 arms falls through to the probe chain (which does handle tail bodies). `arms[1]` is the
    // wildcard/second-literal cover (`lower` guaranteed exhaustiveness), so `(scrutinee == probe0) ?
    // body0 : body1` is total.
    if arms.len() == 2
        && arms.iter().all(|a| a.guard.is_none())
        && matches!(
            arms[0].probe,
            crate::core::Probe::Int(_) | crate::core::Probe::Bool(_)
        )
        && is_select_arm(db, arms[0].body)
        && is_select_arm(db, arms[1].body)
        && !matches!(block_ty, BlockType::Empty)
    {
        // The body leaves are grounded to the match's result width (as the probe-chain arms are),
        // recovered from `result_it` (an Int result) or the block valtype (a Bool result is an i32).
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => Ty::Bool,
        };
        emit_branch(
            db,
            arms[0].body,
            &res_ty,
            slots,
            chain_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_branch(
            db,
            arms[1].body,
            &res_ty,
            slots,
            chain_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_probe_condition(&arms[0].probe, src, it, out);
        out.push(Lir::Select);
        let end = out.here();
        out.match_scope(scope_start, end, binder_vars);
        return Ok(());
    }
    emit_probe_chain(
        db, scrutinee, src, arms, it, result_it, block_ty, slots, chain_base, high, scratch_ty,
        layout, out, tail,
    )?;
    let end = out.here();
    out.match_scope(scope_start, end, binder_vars);
    Ok(())
}

/// Emit the boolean `scrutinee == probe` for a match's literal probe: push the scrutinee `src`, then the
/// comparison. Uses the same instruction selection the probe chain applies — an `Int` `0` probe is
/// `i64.eqz`/`i32.eqz` (one instruction, cycle-43), a nonzero `Int` is `const ; eq`, and a `Bool` probe
/// against `true` is IDENTITY (a Bool is canonical i32 0/1, so `p == 1` is just `p` — push nothing more),
/// against `false` is `i32.eqz`. Shared by the branchless 2-arm select; the `Wild` probe is not a
/// condition (it's the fallthrough) so it never reaches here.
fn emit_probe_condition(probe: &crate::core::Probe, src: OperandSrc, it: IntTy, out: &mut Emit) {
    src.push(out);
    match probe {
        crate::core::Probe::Int(v) => {
            let m = Machine::of(it);
            if v.to_i64_bits() == 0 {
                out.push(if m.slot32 { Lir::I32Eqz } else { Lir::I64Eqz });
            } else {
                out.push(m.konst(v.to_i64_bits()));
                out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
            }
        }
        // A Bool is canonical i32 0/1: `p == true` IS `p` (nothing more), `p == false` is `i32.eqz`.
        crate::core::Probe::Bool(true) => {}
        crate::core::Probe::Bool(false) => out.push(Lir::I32Eqz),
        // A string-literal probe only ever FOLDS (a constant scrutinee) — a runtime string scrutinee is
        // not a scalar (`is_scalar`), so a `Probe::Str` never reaches the runtime scalar probe emit.
        crate::core::Probe::Str(_) => {
            unreachable!(
                "a string-literal probe folds; it is never emitted as a runtime scalar probe"
            )
        }
        // A char-literal probe only ever FOLDS (a constant scrutinee) — a runtime char is not a scalar
        // (`is_scalar`) and has no machine rep, so a `Probe::Char` never reaches the runtime scalar emit.
        crate::core::Probe::Char(_) => {
            unreachable!(
                "a char-literal probe folds; it is never emitted as a runtime scalar probe"
            )
        }
        // A `ListLen` probe folds against a constant list; a runtime list payload declines earlier, so it
        // never reaches a runtime scalar probe.
        crate::core::Probe::ListLen { .. } => {
            unreachable!("a list-length probe folds; it is never emitted as a runtime scalar probe")
        }
        // A `MapHasKeys` probe folds against a constant map; a runtime map declines earlier, so it never
        // reaches a runtime scalar probe.
        crate::core::Probe::MapHasKeys { .. } => {
            unreachable!("a map-key probe folds; it is never emitted as a runtime scalar probe")
        }
        crate::core::Probe::Wild => {}
    }
}

/// The wasm slot type of a scalar match scrutinee (Int → its width's slot, Bool → i32), or `None` if
/// it has no machine representation.
fn block_scalar_slot(db: &mut Db, scrutinee: StructId) -> Option<ValType> {
    match type_of(db, scrutinee) {
        Ty::Int(it) => Some(m_slot(it)),
        Ty::Bool => Some(ValType::I32),
        _ => None,
    }
}

/// The reusable [`OperandSrc`] for a match scrutinee that need NOT be stashed — a parameter/kept-local
/// (re-`local.get` is free) or a compile-time constant (re-materialized inline). `None` for a computed
/// scrutinee, which the caller evaluates once into a scratch slot. (A constant scrutinee normally folds
/// away in `lower` before reaching a runtime match, but handling it keeps the source uniform.)
/// Whether a HEAP-HANDLE scrutinee (a sum) can be re-read per match probe WITHOUT re-evaluation — a
/// parameter or `let`-binding already living in a slot. Anything computed (a `List.at`, a call, an `if`,
/// a fresh construction) is NOT reusable: re-emitting it would recompute the value and its scratch would
/// clash with the arm bodies', so `emit`'s `MatchSum` materializes it into a dedicated slot first.
fn reusable_handle_src(db: &mut Db, scrutinee: StructId, slots: &HashMap<StructId, u32>) -> bool {
    reusable_handle_slot(db, scrutinee, slots).is_some()
}

/// The local SLOT holding a reusable heap-handle expression, or `None`. A `Param` / kept `let`-`LocalRef`
/// whose binder has a slot IS resident in a stable local for the whole body — a BORROWING read (`vec-len`/
/// `vec-get`/`bytes-len`/…) can read that slot DIRECTLY at each use site instead of copying the handle into
/// a fresh scratch slot first (the heap analogue of the scalar `reusable_scalar_src` / `operand_src` reuse).
/// Sound because the collection reads only borrow (no refcount change, never consume) and the owner keeps
/// the handle live across them (a param is owned by the caller; a kept `let`-binding is dropped at scope
/// end, after the read). A computed handle (`None`) still gets stashed in scratch once, as before.
fn reusable_handle_slot(
    db: &mut Db,
    scrutinee: StructId,
    slots: &HashMap<StructId, u32>,
) -> Option<u32> {
    match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => slots.get(&binder).copied(),
        _ => None,
    }
}

/// Prepare a `MatchList` scrutinee for its arm bodies: bind the list HANDLE to a slot the arms read
/// (`arm_slots[scrutinee]`), compute the `vec-len` ONCE into a `len_slot` (the arms' length dispatch reads
/// it), and return the scratch floor `arm_base` past both. Returns `(arm_slots, len_slot, arm_base)`.
///
/// HANDLE SLOT REUSE (mirrors `MatchSum`'s scrutinee discipline + the `List.at` reuse): a REUSABLE handle —
/// a `Param` / kept `let`-`LocalRef` already resident in a stable slot — is read from its OWN slot; the arm
/// bodies' element reads (`vec-get`, BORROWING) and the rest read (`vec-drop`, which `dup`s the handle
/// before consuming — see the `SumPayload` `RestFrom` emit) keep that owner reference intact, so no copy is
/// needed. `emit(scrutinee)` for such a handle is a plain borrowing `local.get`, so the previous
/// copy-into-scratch was pure waste. A COMPUTED scrutinee (a call result, an `if`, a fresh construction) is
/// evaluated ONCE into a fresh i32 slot as before (re-emitting it would recompute + its scratch would clash
/// with the arm bodies').
#[allow(clippy::too_many_arguments)]
fn materialize_list_match_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    slots: &HashMap<StructId, u32>,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(HashMap<StructId, u32>, u32, u32), Reject> {
    let (arm_slots, handle_slot) = match reusable_handle_slot(db, scrutinee, slots) {
        // Resident handle: the arms read the owner slot directly; `slots` already maps the binder there,
        // so `emit(scrutinee)` (a `Param`/`LocalRef`) resolves to it. No copy, no fresh handle scratch.
        Some(owner) => (slots.clone(), owner),
        None => {
            let handle_slot = *high;
            *high = handle_slot + 1;
            scratch_ty.insert(handle_slot, ValType::I32);
            emit(
                db,
                scrutinee,
                slots,
                handle_slot + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(Lir::LocalSet(handle_slot));
            let mut m = slots.clone();
            m.insert(scrutinee, handle_slot);
            (m, handle_slot)
        }
    };
    // The list length is a derived SCALAR read once into its own slot regardless (the length dispatch reads
    // it per arm; recomputing `vec-len` per arm would be a repeated borrow).
    let len_slot = *high;
    *high = len_slot + 1;
    scratch_ty.insert(len_slot, ValType::I32);
    out.push(Lir::LocalGet(handle_slot));
    out.push(Lir::CallImport(OP_VEC_LEN)); // [len:i32]
    out.push(Lir::LocalSet(len_slot));
    let arm_base = *high;
    Ok((arm_slots, len_slot, arm_base))
}

/// Whether the handle an expression's emit leaves on the stack is a NEW OWNED reference the current
/// frame must reclaim, or a BORROW another owner (a parameter's caller, a `let`'s binding-slot drop)
/// already accounts for. Drives whether the `value-eq` emit `drop`s an operand after the borrowing
/// compare — an OWNED temporary must be dropped (else it leaks), a BORROW must NOT (else double-free).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HandleOwnership {
    /// A fresh allocation the frame owns — a constructor result (`SumNew`/`Tuple`/`Record`/`ListNew`) or
    /// a call's returned value (ownership transfers out of the callee). The `value-eq` emit drops it.
    Owned,
    /// A reference the frame only borrows — a parameter (the caller owns it) or a kept `let`-binding (the
    /// `Core::Let` emit drops it at scope end). The `value-eq` emit must NOT drop it.
    Borrowed,
}

/// Classify a BORROWING op's handle OPERAND ownership, or DECLINE (`Err`) a shape whose ownership this
/// analysis cannot prove — reject-don't-miscompile: a wrong guess would leak or double-free the heap.
/// Used by every op that BORROWS a heap operand and returns a fresh/scalar result (so the emit must drop
/// each OWNED-temporary operand but leave a borrowed reference to its owner): `value-eq` and the runtime
/// BigInt ops (`bigint-add`/…/`bigint-cmp`/`bigint-to-i64-checked`, which `unbox_bigint`-BORROW their
/// operands). A constructor / call is `Owned`; a parameter / kept-local reference is `Borrowed`. An `if`/
/// `match`/`let` is classified by its result sub-expression(s) — but ONLY when every branch agrees (a
/// mixed owned/borrowed result cannot be dropped uniformly, so it declines). Anything else declines.
/// Whether operand `id`'s solved type is a `String` (possibly through a nominal/symbol wrapper) — the
/// type whose runtime rep can be a NON-CANONICAL rope (`String.concat` → `bytes-concat`). Such an operand
/// of `value-eq`/`champ_eq` (a physical-byte compare) is canonicalized with `bytes-compact` before the
/// compare so a rope and its flat twin compare equal. A `Symbol` is a nominal over a String leaf (same
/// rope-capable rep); peel nominals so a `Symbol`/String-newtype operand is compacted too.
/// Whether an operand is a rope-capable text/byte value that must be `bytes-compact`ed before the physical
/// `champ_eq` compare — a `String`/`Symbol` OR a `Bytes`. A runtime `Bytes` can be a `bytes-concat`/`.slice`
/// ROPE (physical bytes ≠ a flat leaf of equal content), exactly like a `String.concat` rope, so it is
/// canonicalized the same way (the runtime `bytes-compact` is refcount-neutral, so it is safe on an owned
/// OR a borrowed operand). Used at BOTH physical-byte compare sites: the DIRECT `Core::ValueEq` operand
/// path AND the Map/Set KEY path (`key_needs_compaction`). With the key path now Bytes-compacting,
/// `ty_heap_walkable` admits a `Bytes` leaf (nested in a compound / a Set/Map key) — the nested/keyed
/// companion of the direct-operand Bytes `=`.
fn operand_is_string_or_bytes(db: &mut Db, id: StructId) -> bool {
    fn peel(ty: &Ty) -> &Ty {
        match ty {
            Ty::Nominal { inner, .. } => peel(inner),
            other => other,
        }
    }
    matches!(peel(&type_of(db, id)), Ty::String | Ty::Symbol | Ty::Bytes)
}

/// Whether a Map/Set KEY operand needs `bytes-compact` before the CHAMP `champ_hash`/`champ_eq` — an
/// OWNED runtime String OR Bytes (a `String.concat`/`Bytes.concat`/`.slice` rope, whose physical bytes
/// differ from a flat twin's of equal content, so it would hash into a different slot and never match its
/// flat twin). This is the KEY-path companion of the `Core::ValueEq` compaction (`731dbf09`): both
/// `value-eq` and the map/set key path use `champ_eq` over physical bytes, so a rope must be canonicalized
/// at BOTH. Bytes is included alongside String/Symbol (`operand_is_string_or_bytes`) because a `Bytes`
/// value has the SAME rope representation and the same physical-byte CHAMP key contract — the reasoning is
/// verbatim the String story. Only a DIRECT, OWNED key is compacted — a BORROWED key (a param / a
/// kept-local reference) is a FLAT leaf in practice and `bytes-compact` would consume it under its owner
/// (mirrors the value-eq ownership gate); a String/Bytes NESTED inside a compound key is the same rarer
/// deferred case value-eq leaves. A compacted owned key is stack- and ownership-NEUTRAL: an owned rope in,
/// an owned flat leaf out, so each site's existing key accounting (consumed by insert, or the dropped
/// borrow-temporary at lookup/remove/contains) is unchanged.
fn key_needs_compaction(db: &mut Db, key: StructId) -> bool {
    operand_is_string_or_bytes(db, key)
        && matches!(heap_operand_ownership(db, key), Ok(HandleOwnership::Owned))
}

/// Whether the key/element handle left on the stack after `emit` (+ optional box, + optional compact) is
/// an OWNED TEMPORARY the frame must `drop` after a BORROWING key op (`map-lookup`/`set-contains`, which
/// read the key without consuming it) — vs a BORROW of a live owner it must NOT drop. Owned iff the key
/// was BOXED (a scalar → a fresh `box-*` leaf), or COMPACTED (an owned rope → a fresh flat leaf), or the
/// key OPERAND itself is a fresh owned handle (a constructor / call / const compound). A BORROWED
/// String/compound key — a parameter, a kept `let`-local, or a `sum-payload`/`arr-get` projection of a
/// still-live value — is used AS-IS (no box, no compact), so dropping it frees a reference its owner still
/// holds: a use-after-free MISCOMPILE. This is exactly the two-live-matched-String-payloads shape (a
/// tree-walker looking up a node's OWN key and its CHILD's key, both `String` sum-payload projections of
/// live nodes) — the second borrowed key was freed under its owner, flipping the comparison and dropping a
/// per-node decision (a silent wrong count). Declines (via `heap_operand_ownership`) a key whose ownership
/// cannot be proved — reject-don't-miscompile, never a double-free. Mirrors the `Core::ValueEq` ownership
/// gate, the sibling String-payload family (`731dbf09`).
fn key_handle_is_owned_temporary(db: &mut Db, key: StructId, key_ty: &Ty) -> Result<bool, Reject> {
    if box_op_for(db, key, key_ty)?.is_some() {
        return Ok(true); // a scalar key → a fresh `box-*` leaf the op borrows, then we drop
    }
    if key_needs_compaction(db, key) {
        return Ok(true); // an owned rope key → a fresh compacted flat leaf we drop
    }
    // An unboxed, uncompacted key is used as-is: drop it only if the operand is a fresh owned handle (a
    // constructor / call / const compound); a borrowed param/local/projection is left to its owner.
    Ok(heap_operand_ownership(db, key)? == HandleOwnership::Owned)
}

/// Whether a compound ELEMENT (tuple/record/list element, sum payload) is a rope-capable byte value — a
/// `String`/`Symbol` (a `String.concat` rope) or a `Bytes` (a `Bytes.concat`/`.slice` rope), peeling
/// nominals. Such a leaf STORED INSIDE a compound must be CANONICALIZED with `bytes-compact` at the
/// construction site, exactly as `op_box_float` canonicalizes a NaN when a float leaf is boxed: the value
/// heap is TAGLESS, so `champ_eq`/`champ_hash`'s structural walk compares a nested leaf by its PHYSICAL
/// raw bytes and cannot know a child is a rope (vs a compound), so a rope leaf nested in a tuple/record/
/// sum/map-key compares UNEQUAL to its flat twin (and a compound map key containing one lands in a
/// different CHAMP slot). Compacting on construction means no compound ever holds a rope, so the walk's
/// physical compare is exact — the nested-leaf twin of the `Core::ValueEq`/key-path top-level compaction.
/// `bytes-compact` is REFCOUNT-NEUTRAL (it flattens the node IN PLACE and returns the same handle, a
/// no-op on an already-flat leaf) and `bytes_flatten` is content-preserving hence safe even on a SHARED
/// node, so it is sound for an element of ANY ownership (an owned `String.concat` result, or a BORROWED
/// String param the caller could have passed as a rope — the case a naive owned-only compile-time fix
/// would miss).
fn elem_needs_rope_compaction(db: &mut Db, id: StructId) -> bool {
    fn peel(ty: &Ty) -> &Ty {
        match ty {
            Ty::Nominal { inner, .. } => peel(inner),
            other => other,
        }
    }
    matches!(peel(&type_of(db, id)), Ty::String | Ty::Symbol | Ty::Bytes)
}

/// Whether `id`'s solved type is BigInt-VALUED — a bare `Ty::BigInt` OR a quantity over a BigInt
/// magnitude (`Ty::Qty { inner: BigInt }`). A `(Qty BigInt u)` erases to its inner BigInt handle, so
/// every place that materializes / classifies a constant BigInt as a heap handle (the `Core::ConstInt`
/// emit choke-point, the const-materialize-ops inserters, the borrow-ownership classifier) must treat a
/// BigInt-inner quantity the same — else a `(Qty.of (BigInt.of k) u)` constant emits as a raw `i64.const`
/// where an i32 handle is expected (invalid wasm). One helper so the peel is consistent across all sites.
fn is_bigint_valued(db: &mut Db, id: StructId) -> bool {
    matches!(type_of(db, id), Ty::BigInt)
        || matches!(type_of(db, id), Ty::Qty { inner, .. } if matches!(*inner, Ty::BigInt))
}

fn heap_operand_ownership(db: &mut Db, id: StructId) -> Result<HandleOwnership, Reject> {
    match core_of(db, id) {
        // Constructors and calls produce a fresh owned reference (ownership transfers out). A map
        // construction/update (`map-empty`+inserts, `map-insert`, `map-remove`) returns a fresh owned
        // map handle exactly like a list/tuple constructor — the `value-eq` emit drops it after the compare.
        Core::SumNew { .. }
        | Core::Tuple { .. }
        | Core::Record { .. }
        | Core::ListNew { .. }
        | Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapRemove { .. }
        // A constant string/bytes materializes a FRESH owned byte-leaf handle (`bytes-alloc`+`bytes-set`,
        // see the `Core::ConstStr`/`BytesOf` emit), so — like a constructor — the `value-eq` emit drops
        // it after the borrowing compare. This is the `(= h "+")` shape: comparing a runtime payload
        // string against a constant-string literal.
        | Core::ConstStr(_)
        | Core::BytesOf { .. }
        // A runtime Bytes/String PRODUCER returns a FRESH owned handle: `bytes-concat`/`bytes-slice`/
        // `bytes-compact` each consume their operand(s) and hand back a new sequence; `str-from-bytes`
        // transfers the validated buffer out as a String; `str-to-bytes` (= `bytes-compact`) flattens the
        // string's byte-rope out as a fresh Bytes leaf. So such a value as a DIRECT `value-eq` operand is
        // owned and the emit drops it after the borrowing compare — the `(= (String.to-bytes s) b"…")` shape
        // the compiler-in-Cadenza codec's byte round-trip compares.
        | Core::BytesConcat { .. }
        | Core::BytesSlice { .. }
        | Core::BytesCompact { .. }
        | Core::StrFromBytes { .. }
        | Core::StrToBytes { .. }
        // A set construction/update/algebra (`set-empty`+inserts, `set-insert`, `set-remove`, union/
        // intersection/difference) returns a fresh owned set handle — the `value-eq` emit drops it.
        | Core::SetOf { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetAlgebra { .. }
        // A BigInt PRODUCER returns a fresh owned handle: `bigint-of-i64` mints a leaf, and each
        // `bigint-add`/`-sub`/`-mul`/`-div` re-boxes a normalized result (the operands are borrowed, the
        // result is new). So a BigInt operand that is itself the result of another BigInt op is owned —
        // the enclosing op drops it after borrowing. (`BigIntToI64` returns an i64 scalar, never a handle,
        // so it is not a heap operand and never reaches here.)
        | Core::BigIntOfI64 { .. }
        | Core::BigIntBinOp { .. }
        // A Rational PRODUCER likewise returns a fresh owned handle: `rational-of` (`RationalOfInts`/
        // `RationalOfIntWiden`) builds a new 2-handle node, and each `rational-add`/…-`div` re-normalizes
        // into a new node. So a Rational operand that is itself a Rational op's result is owned.
        | Core::RationalOfInts { .. }
        | Core::RationalOfIntWiden { .. }
        | Core::RationalBinOp { .. }
        // A HOST/PEER call returning a COMPOUND yields a fresh OWNED handle (a peer-bound effect returns a
        // runtime value the consumer now owns — the shared-runtime handle transport, U5/U11), exactly like
        // a defined-func `Call`. So a peer-returned compound projected/consumed here is an owned temporary
        // the enclosing op reclaims (U13) rather than leaking until run-end.
        | Core::HostCall { .. }
        | Core::Call { .. } => Ok(HandleOwnership::Owned),
        // A CONSTANT typed `BigInt` materializes to a FRESH owned handle at `emit` (the `Core::ConstInt`
        // arm routes a BigInt-typed constant through `bigint-of-i64`), exactly like `ConstStr` above — so
        // as a borrowing-op operand it is Owned and the emit drops it. This is what lets `Int64.of (if c
        // (BigInt.of 1) (BigInt.of 2))` narrow a BigInt-valued `if` whose branches are constant BigInts.
        // A constant BigInt (bare OR a BigInt-inner quantity — `is_bigint_valued` peels the `Qty`)
        // materializes to a FRESH owned handle at emit (the `Core::ConstInt` arm routes it through
        // `bigint-of-i64`), exactly like `ConstStr` — so as a borrowing bigint-op operand it is Owned and
        // the emit drops it. Covers `(+ (Qty.of (BigInt.of v) m) (Qty.of (BigInt.of 100) m))` (runtime +
        // constant BigInt quantity) and `Int64.of (if c (BigInt.of 1) (BigInt.of 2))`.
        Core::ConstInt(_) if is_bigint_valued(db, id) => Ok(HandleOwnership::Owned),
        // A CONSTANT Rational likewise materializes to a FRESH owned handle at `emit` (`bigint-of-i64` ×2
        // + `rational-of`), so as a borrowing-op operand it is Owned.
        Core::ConstRational(_, _) => Ok(HandleOwnership::Owned),
        // A reference to a parameter or a kept `let`-binding — the owner elsewhere reclaims it.
        Core::Param { .. } | Core::LocalRef { .. } => Ok(HandleOwnership::Borrowed),
        // A payload/element READ (`sum-payload`/`arr-get`) BORROWS its operand — the enclosing compound
        // owns the sub-value, so the read yields a borrowed handle the `value-eq` emit must NOT drop
        // (`sum-payload`/`arr-get` read without transferring ownership; see `binding_escapes`). This is
        // the shape a recursive tree-walker compares — `(= h "+")` where `h` is a variant's tuple-payload
        // element bound via `SumPayload`. `SumExpect` (an `Option.expect` payload read) borrows likewise.
        // (A `String.at` `Some` payload is a rope slice, but it is COMPACTED at the producer — the `StrAt`
        // Some-branch flattens the slice before wrapping it — so the extracted payload is a flat leaf that
        // `value-eq` compares correctly without reclassifying this borrow as owned; see `Core::StrAt` emit.)
        Core::SumPayload { .. } | Core::SumExpect { .. } | Core::Proj { .. } => {
            Ok(HandleOwnership::Borrowed)
        }
        // Control flow: the operand's value is produced on one of several paths, so its ownership is the
        // JOIN of the reachable results — OWNED only when EVERY path provably yields a fresh owned
        // temporary (so the single post-compare drop is correct on all paths), else BORROWED. Classifying
        // BORROWED is always leak-safe: the emit then does NOT drop the operand, so a path that actually
        // produced an owned temporary merely LEAKS it (the conservative bias `binding_escapes` states — a
        // false "borrowed" only leaks) rather than risk freeing a borrowed path's still-live value under
        // its owner (a double-free). This mirrors the standalone-function path exactly: a body returning a
        // borrowed match payload leaves the value un-dropped and leaks the scrutinee it borrows from.
        //
        // `if` joins both arms; `let` forwards its body; a `match` (scalar / sum / list) joins its arm
        // bodies (`join_arm_ownership` / `sum_cont_ownership`). A bare-`Leaf`-rooted sum match folds to its
        // body in `lower` and never reaches here as a `MatchSum`.
        Core::If { then_, else_, .. } => Ok(join_arm_ownership(db, [then_, else_])),
        Core::Let { body, .. } => heap_operand_ownership(db, body),
        Core::Match { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchList { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchSum { root, .. } => Ok(sum_cont_ownership(db, &root)),
        // When the operand's ownership (its aliasing status — whether the enclosing op may reclaim it or must
        // leave it to another owner) cannot be established by any arm above, DECLINE rather than emit a
        // component whose dup/drop placement would be a guess: the aliasing discipline could not be proven
        // safe here, so refusing is the sound outcome, not an unchecked emit with unspecified aliasing.
        //= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
        //# The compiler MUST reject a program whose aliasing the memory discipline cannot establish as safe, rather than emit a component with unspecified aliasing behavior.
        _ => Err(Reject::decline(
            "borrowing op operand has an ownership this backend cannot yet prove",
        )),
    }
}

/// The JOIN of several result positions' ownership for a borrowing-op operand (see
/// [`heap_operand_ownership`]): [`HandleOwnership::Owned`] iff EVERY body is provably `Owned`, otherwise
/// [`HandleOwnership::Borrowed`]. A body whose ownership cannot be proven counts as `Borrowed` — the
/// leak-safe join value, so an unhandled arm shape never declines the whole match (it just leaves the
/// operand un-dropped, a leak, never a double-free). Empty (a match with no arms cannot reach a value)
/// is `Borrowed` — the safe default.
fn join_arm_ownership(db: &mut Db, bodies: impl IntoIterator<Item = StructId>) -> HandleOwnership {
    for body in bodies {
        if !matches!(heap_operand_ownership(db, body), Ok(HandleOwnership::Owned)) {
            return HandleOwnership::Borrowed;
        }
    }
    HandleOwnership::Owned
}

/// Ownership of a sum-match CONTINUATION as a borrowing-op operand — the join over every LEAF body the
/// decision tree can reach (mirrors `cont_child_ids`): a `Guarded` arm joins its body with the
/// fall-through `els`, a `LitTest` joins its `then_`/`els`, a `Switch` joins all its arms'
/// continuations. `Owned` iff every reachable leaf is provably `Owned`, else `Borrowed` (leak-safe).
fn sum_cont_ownership(db: &mut Db, cont: &crate::core::SumCont) -> HandleOwnership {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            if matches!(
                heap_operand_ownership(db, *body),
                Ok(HandleOwnership::Owned)
            ) {
                HandleOwnership::Owned
            } else {
                HandleOwnership::Borrowed
            }
        }
        crate::core::SumCont::Guarded { body, els, .. } => {
            match (
                heap_operand_ownership(db, *body),
                sum_cont_ownership(db, els),
            ) {
                (Ok(HandleOwnership::Owned), HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            match (sum_cont_ownership(db, then_), sum_cont_ownership(db, els)) {
                (HandleOwnership::Owned, HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms.iter() {
                if sum_cont_ownership(db, &a.cont) == HandleOwnership::Borrowed {
                    return HandleOwnership::Borrowed;
                }
            }
            HandleOwnership::Owned
        }
    }
}

/// Emit a UNARY runtime BigInt op that BORROWS its handle operand and returns a scalar (`bigint-to-i64-
/// checked`). The op reads the operand without consuming it, so an OWNED-temporary operand must be
/// DROPPED after the call (a borrowed param/local is left to its owner) — the `value-eq` reclamation
/// discipline. `tee` the operand into a scratch slot (kept on the stack for the call AND remembered for a
/// possible drop), call the op (which pops the borrowed handle and pushes the scalar), then drop the
/// remembered handle if it was owned. Declines (via `heap_operand_ownership`) an operand whose ownership
/// cannot be proved — reject, never a leak or double-free.
/// Register the runtime ops the inline materialization of a CONSTANT BigInt operand emits, so
/// `collect_used_ops` imports them: `bigint-of-i64` for an i64-fitting constant, or `bytes-alloc` +
/// `bytes-set` + `bigint-of-bytes` for a beyond-i64 one (the baked-sign-magnitude-bytes path). Mirrors the
/// two branches of `emit_const_bigint_leaf`. A non-constant / non-BigInt operand registers nothing.
fn insert_const_bigint_materialize_ops(
    db: &mut Db,
    operand: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    if let Core::ConstInt(v) = core_of(db, operand)
        && is_bigint_valued(db, operand)
    {
        if v.to_i64().is_some() {
            out.insert(OP_BIGINT_OF_I64);
        } else {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_BIGINT_OF_BYTES);
        }
    }
}

/// The canonical sign-magnitude heap-leaf bytes of a constant integer — `[sign][LE magnitude, trailing
/// zero bytes stripped]`, zero → `[0x00]` — byte-IDENTICAL to `bigint::Big::to_sign_magnitude_bytes` in
/// `cdz-runtime`, so a leaf built from these bytes via `bigint-of-bytes` is the SAME rep `bigint-of-i64` /
/// runtime arithmetic produces (so `bigint-cmp`/`value-eq` compare it correctly). `IntValue.magnitude` is
/// big-endian with no leading zero bytes, so reversing yields little-endian with no trailing zero bytes;
/// zero is the empty magnitude → the single sign byte `[0]` (never negative-zero).
fn const_bigint_sign_magnitude_bytes(v: &crate::ast::IntValue) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + v.magnitude.len());
    // Zero is non-negative on the wire (matches the runtime canonical form + `IntValue::zero`).
    bytes.push((v.negative && !v.magnitude.is_empty()) as u8);
    bytes.extend(v.magnitude.iter().rev().copied()); // big-endian → little-endian
    bytes
}

/// Emit a CONSTANT BigInt as a fresh OWNED heap-leaf handle on the stack. Fits i64 → `bigint-of-i64`;
/// beyond i64 → bake its canonical sign-magnitude bytes as a Bytes leaf (`bytes-alloc` + per-byte
/// `bytes-set`, exactly as a constant string materializes) then re-tag as a BigInt via `bigint-of-bytes`
/// (which consumes the byte leaf). Shared by the in-body value materialization (`Core::ConstInt`-typed-
/// BigInt) and the operand path (`emit_bigint_operand`).
fn emit_const_bigint_leaf(v: &crate::ast::IntValue, out: &mut Emit) {
    match v.to_i64() {
        Some(x) => {
            out.push(Lir::ConstI64(x));
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // → [fresh owned BigInt handle : i32]
        }
        None => {
            let bytes = const_bigint_sign_magnitude_bytes(v);
            out.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            }
            out.push(Lir::CallImport(OP_BIGINT_OF_BYTES)); // consumes buf → [fresh owned BigInt handle : i32]
        }
    }
}

/// Emit ONE BigInt operand as a heap HANDLE on the stack and return its ownership (for a possible post-op
/// drop). A CONSTANT BigInt that fits `i64` has no heap leaf yet, so materialize it: push its `i64` value
/// then `bigint-of-i64` → a FRESH OWNED handle (the borrowing op drops it after). A constant BEYOND `i64`
/// range DECLINES (the arbitrary-magnitude constant leaf is a B4 concern — the sign-magnitude byte
/// builder). Any other operand emits via `emit` and is classified by `heap_operand_ownership`.
#[allow(clippy::too_many_arguments)]
fn emit_bigint_operand(
    db: &mut Db,
    operand: StructId,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<HandleOwnership, Reject> {
    if let Core::ConstInt(v) = core_of(db, operand)
        && is_bigint_valued(db, operand)
    {
        // A constant BigInt operand has no heap leaf of its own — materialize one (fits-i64 via
        // `bigint-of-i64`, beyond-i64 via `bigint-of-bytes` on its baked sign-magnitude bytes). A FRESH
        // OWNED handle either way; the borrowing op drops it after.
        emit_const_bigint_leaf(&v, out);
        return Ok(HandleOwnership::Owned);
    }
    let o = heap_operand_ownership(db, operand)?;
    let op_base = *high;
    emit(db, operand, slots, op_base, high, scratch_ty, layout, out)?; // [h : i32]
    Ok(o)
}

#[allow(clippy::too_many_arguments)]
fn emit_bigint_borrow_unary(
    db: &mut Db,
    operand: StructId,
    import: &'static str,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let slot = *high;
    *high = slot + 1;
    scratch_ty.insert(slot, ValType::I32);
    let o = emit_bigint_operand(db, operand, high, slots, scratch_ty, layout, out)?; // [h : i32]
    out.push(Lir::LocalTee(slot));
    out.push(Lir::CallImport(import)); // pops the borrowed handle → [scalar]
    if o == HandleOwnership::Owned {
        out.push(Lir::LocalGet(slot));
        out.push(Lir::CallImport(OP_DROP));
    }
    Ok(())
}

/// Emit a BINARY runtime BigInt op that BORROWS both handle operands and returns a FRESH owned result
/// handle (`bigint-add`/`-sub`/`-mul`/`-div`, and — the next slice — `bigint-cmp`, which returns a scalar
/// instead; both leave the operands to be reclaimed by this emit). Each OWNED-temporary operand is
/// dropped after the call while the result stays on the stack; a borrowed param/local is left to its
/// owner. Same shape as the `value-eq` emit, but the result (a handle or a scalar) is kept rather than
/// discarded. Two i32 scratch slots hold the operand handles for the possible drops; the operands emit
/// above the running high-water so neither reuses the other's transient scratch at a different width.
#[allow(clippy::too_many_arguments)]
fn emit_bigint_borrow_binary(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    import: &'static str,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let slot_l = *high;
    let slot_r = *high + 1;
    *high = slot_r + 1;
    scratch_ty.insert(slot_l, ValType::I32);
    scratch_ty.insert(slot_r, ValType::I32);
    let lo = emit_bigint_operand(db, lhs, high, slots, scratch_ty, layout, out)?; // [a : i32]
    out.push(Lir::LocalTee(slot_l));
    let ro = emit_bigint_operand(db, rhs, high, slots, scratch_ty, layout, out)?; // [a, b : i32]
    out.push(Lir::LocalTee(slot_r));
    out.push(Lir::CallImport(import)); // pops both borrowed handles → [result]
    if lo == HandleOwnership::Owned {
        out.push(Lir::LocalGet(slot_l));
        out.push(Lir::CallImport(OP_DROP));
    }
    if ro == HandleOwnership::Owned {
        out.push(Lir::LocalGet(slot_r));
        out.push(Lir::CallImport(OP_DROP));
    }
    Ok(())
}

fn reusable_scalar_src(
    db: &mut Db,
    scrutinee: StructId,
    slots: &HashMap<StructId, u32>,
) -> Option<OperandSrc> {
    match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => {
            slots.get(&binder).copied().map(OperandSrc::Slot)
        }
        Core::ConstInt(v) => match type_of(db, scrutinee) {
            Ty::Int(it) if it.ground_width() <= 32 => {
                Some(OperandSrc::ConstI32(v.to_i32_bits(it.ground_width())))
            }
            _ => Some(OperandSrc::ConstI64(v.to_i64_bits())),
        },
        Core::ConstBool(b) => Some(OperandSrc::ConstI32(if b { 1 } else { 0 })),
        _ => None,
    }
}

/// The probe chain over a match's arms, dispatching on a scrutinee already resolved to `src` (pushed
/// once per probe — a local read or an inline constant, never a recomputation). See
/// [`emit_match_arms_tailable`], which resolves `src` and (for a computed scrutinee) evaluates it once.
#[allow(clippy::too_many_arguments)]
fn emit_probe_chain(
    db: &mut Db,
    scrutinee: StructId,
    src: OperandSrc,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    // BR_TABLE DECISION TREE for a DENSE integer match: ≥3 `Int` probes over a small contiguous-ish
    // range dispatch in O(1) via a jump table instead of the linear `if (== k)` cascade below. Only
    // fires for an unguarded integer match with a wildcard default (see `try_emit_scalar_br_table`),
    // and only when the value range is dense enough to not waste table slots. `None` → fall through to
    // the linear chain (a sparse range, guards, too few arms, a non-int probe).
    if let Some(()) = try_emit_scalar_br_table(
        db, src, arms, it, result_it, block_ty, slots, base, high, scratch_ty, layout, out, tail,
    )? {
        return Ok(());
    }
    // BRANCHLESS TERMINAL PAIR: when the chain has narrowed to exactly TWO arms — a literal-probe arm
    // then its unconditional cover (a wildcard, or the redundant last probe of an exhaustive wildcard-less
    // match) — this is `(if (scrutinee == probe0) body0 body1)`, the same shape the standalone 2-arm match
    // selects (see `emit_match_dispatch`). When both are unguarded with cheap trap-free `is_select_arm`
    // bodies and the result is a scalar, emit `body0 ; body1 ; (scrutinee == probe0) ; select` instead of
    // an `if`/`else` block — so the TAIL of an N-arm sparse chain (`(match x (0 a) (5 b) (_ c))` → the
    // inner `(5 b)/(_ c)` pair) is branchless too, not only a standalone 2-arm match. `body1` covers every
    // non-`probe0` value, so the select is total. TAIL position is fine: a trap-free body is never a call,
    // so no arm is a tail call to preserve (matching the standalone case). Falls through to the linear
    // chain for a guarded arm, a heavier/possibly-trapping body, or a non-Int/Bool probe.
    if arms.len() == 2
        && arms.iter().all(|a| a.guard.is_none())
        && matches!(
            arms[0].probe,
            crate::core::Probe::Int(_) | crate::core::Probe::Bool(_)
        )
        && is_select_arm(db, arms[0].body)
        && is_select_arm(db, arms[1].body)
        && !matches!(block_ty, BlockType::Empty)
    {
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => Ty::Bool,
        };
        emit_branch(
            db,
            arms[0].body,
            &res_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_branch(
            db,
            arms[1].body,
            &res_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_probe_condition(&arms[0].probe, src, it, out);
        out.push(Lir::Select);
        return Ok(());
    }
    // An arm body is emitted via `emit_arm_body` (grounds a bare-`ConstInt` body to the match's result
    // width, threads the tail context). The chain dispatches per arm below.
    let Some((arm, rest)) = arms.split_first() else {
        // No arm matched and no wildcard — `lower` forbids this for a runtime match, so it is a
        // compiler bug if reached. Decline rather than emit an undefined fallthrough.
        return Err(Reject::decline(
            "match ran off the end with no wildcard arm",
        ));
    };
    // An UNGUARDED arm whose probe always matches — a wildcard, or the LAST arm of an exhaustive
    // wildcard-less match (its probe redundant since every earlier probe failed) — is the unconditional
    // tail: emit its body at THIS nesting, no `if`. A GUARDED arm is NEVER an unconditional tail (its
    // guard may fail), so it always emits a test; `lower`'s exhaustiveness guarantees a later UNGUARDED
    // cover, so the chain still terminates.
    let probe_redundant = matches!(arm.probe, crate::core::Probe::Wild) || rest.is_empty();
    if arm.guard.is_none() && probe_redundant {
        // A literal-probe arm reached as the unconditional tail (the last arm of an exhaustive
        // wildcard-less match) STILL knows `scrutinee == literal` — refine its body so a `(- n 1)` there
        // sheds its guard. (A `Wild` tail arm binds no constant → the frame is unchanged.)
        let frame =
            refined_frame_for_match_arm(db, scrutinee, &arm.probe, db.current_refinements());
        db.push_range_refinements(frame);
        let r = emit_arm_body(
            db, arm.body, result_it, slots, base, high, scratch_ty, layout, out, tail,
        );
        db.pop_range_refinements();
        return r;
    }
    // The matched body AND the `else` recursion are both INSIDE this `if` block — so a self-loop `br`
    // from either must jump one MORE level out to reach the loop top (depth + 1).
    let inner = deeper_tail(tail);
    // The arm's TEST: `probe` (scrutinee == literal), AND its `guard` when present. A `Wild` probe has no
    // literal test — the guard alone gates it. To preserve short-circuit trap semantics (the guard is
    // evaluated only when the probe matched — a guard MAY contain a trapping op), a literal-probe-plus-
    // guard nests the guard inside the probe's `if`; the two else-arms both fall through to `rest`.
    let has_literal_probe = !matches!(arm.probe, crate::core::Probe::Wild);
    if has_literal_probe {
        // `if (scrutinee == literal) <guard-gated body> else <rest>`.
        src.push(out);
        match &arm.probe {
            crate::core::Probe::Int(v) => {
                let m = Machine::of(it);
                // PROBE-AGAINST-ZERO → `eqz`. A `0` literal arm (the shape of every recursion base case
                // `(match n (0 …) …)`) is `scrutinee == 0` — exactly `i32.eqz`/`i64.eqz` (one
                // instruction), not a pushed `0` constant + `eq` (two). Same instruction-selection the
                // comparison path applies to `(= n 0)`; mirrored here for the match probe. A nonzero
                // literal keeps the `const ; eq`.
                if v.to_i64_bits() == 0 {
                    out.push(if m.slot32 { Lir::I32Eqz } else { Lir::I64Eqz });
                } else {
                    out.push(m.konst(v.to_i64_bits()));
                    out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
                }
            }
            crate::core::Probe::Bool(b) => {
                out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
                out.push(Lir::I32Eq);
            }
            crate::core::Probe::Str(_) => {
                unreachable!(
                    "a string-literal probe folds; a runtime string match declines at is_scalar"
                )
            }
            crate::core::Probe::Char(_) => {
                unreachable!(
                    "a char-literal probe folds; a runtime char match declines at is_scalar"
                )
            }
            crate::core::Probe::ListLen { .. } => {
                unreachable!(
                    "a list-length probe folds; a runtime list match declines at build_lit_test"
                )
            }
            crate::core::Probe::MapHasKeys { .. } => {
                unreachable!(
                    "a map-key probe folds; a runtime map match declines at build_lit_test"
                )
            }
            crate::core::Probe::Wild => unreachable!("has_literal_probe"),
        }
        out.push(Lir::If(block_ty));
        emit_arm_guarded_body(
            db, scrutinee, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty,
            layout, out, inner,
        )?;
        // The probe's ELSE (the fall-through probe chain) starts scratch ABOVE the high-water the THEN
        // (a guarded body) reached, NOT at `base`. A guard in the THEN may stash an i32 HEAP HANDLE (a
        // runtime `value-eq`/`MatchSum`) in a low slot, typing it i32 for the whole function; the ELSE's
        // fall-through i64 iteration arithmetic reusing that slot number would force one wasm local to
        // two types (invalid module). The two `if` branches are mutually exclusive at RUN time but share
        // ONE function-global local declaration, so a slot used at two widths across them is illegal. A
        // scalar guard/body leaves `*high` unchanged, so this is byte-identical for the common case. (The
        // `src` scrutinee slot is below `base`-relative scratch and stays live regardless.)
        let else_base = *high;
        out.push(Lir::Else);
        emit_probe_chain(
            db, scrutinee, src, rest, it, result_it, block_ty, slots, else_base, high, scratch_ty,
            layout, out, inner,
        )?;
        out.push(Lir::End);
        Ok(())
    } else {
        // A `Wild` probe with a guard: the guard alone gates the arm — `if guard body else rest`. There
        // is NO probe `if` here (a wildcard needs no literal test), so pass `tail`, NOT `inner`: the ONLY
        // block the guard's body/fall-through nest inside is the guard's own `if` (which
        // `emit_arm_guarded_body` accounts for with its own `deeper_tail`). Passing the probe-adjusted
        // `inner` here DOUBLE-COUNTED the nesting — a self-tail-call in the fall-through `br`'d one level
        // too far, PAST the loop, producing invalid wasm (`expected i64 but nothing on stack`). `inner`
        // is correct ONLY for the literal-probe path above, where a real probe `if` IS pushed.
        emit_arm_guarded_body(
            db, scrutinee, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty,
            layout, out, tail,
        )
    }
}

/// Try to emit a DENSE integer `match` as a BR_TABLE decision tree (O(1) jump) instead of the linear
/// `if (== k)` cascade. Returns `Ok(Some(()))` when it emitted the table, `Ok(None)` to fall back.
///
/// Eligible when: the match is NOT in tail position (a tail match keeps the linear chain, which threads
/// the self-loop context — a br_table here would bypass the match-based tail-loop and break O(1) stack);
/// the arms are ≥3 UNGUARDED `Int` probes followed by ONE trailing UNGUARDED wildcard default (a scalar
/// int match is always wildcard-terminated — int is unbounded, so exhaustiveness requires it); every
/// literal fits an i64; and the value RANGE is DENSE — `span = max - min + 1` satisfies `span <= 2*count`
/// and `span <= 256` (so the jump table is not mostly default padding). Otherwise fall back to the chain.
///
/// The index is `scrutinee - min` (a 0-based table position). Values outside `[min, max]`, and gaps in
/// the range with no arm, route to the default via `br_table`'s own unsigned out-of-range check — EXCEPT
/// an i64 scrutinee, where the required `i32.wrap_i64` of the shifted index could alias a value
/// `>= min + 2^32` into `[0, span)`; for that case a `br_if` bounds guard (`(idx as u64) >= span →
/// default`) precedes the table. A ≤32-bit scrutinee needs no guard (its slot IS i32; the subtraction is
/// exact mod 2^32 and br_table's bounds check is correct). The block structure mirrors
/// `try_emit_disc_br_table` (one typed `$join`, empty label blocks, each arm `br`s its value to `$join`).
#[allow(clippy::too_many_arguments)]
fn try_emit_scalar_br_table(
    db: &mut Db,
    src: OperandSrc,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<Option<()>, Reject> {
    // A SELF-LOOP tail match must keep the linear chain (it threads the loop context so a self-tail-call
    // in an arm iterates the loop — a br_table's value-join structure can't carry a loop `br` out of an
    // arm). A NON-self-loop match — value position (`NonTail`) OR a plain tail position with no loop
    // (`Tail(None)`, e.g. an exported non-recursive body) — is eligible: its arm bodies are ordinary
    // values `br`'d to the join block, and the join's value is the function's result. Disqualify only
    // `Tail(Some(_))`.
    if matches!(tail, TailPos::Tail(Some(_))) {
        return Ok(None);
    }
    // Split off a trailing unguarded wildcard default; the rest must be unguarded `Int` probes.
    let (default, int_arms): (&crate::core::MatchArm, &[crate::core::MatchArm]) = match arms.last()
    {
        Some(a)
            if matches!(a.probe, crate::core::Probe::Wild)
                && a.guard.is_none()
                && arms.len() >= 4 =>
        {
            (a, &arms[..arms.len() - 1])
        }
        _ => return Ok(None),
    };
    // O(1) SIZE GATE before the O(arms) literal walk below. Eligibility requires a DENSE range
    // (`span <= 256`, checked below) and the literals are DISTINCT (a duplicate falls back), so
    // `count <= span <= 256`: a table can NEVER fire with more than 256 int-arms. Reject those here in
    // O(1) instead of building an O(arms) `lits` vector that the density check would discard. This is
    // what keeps a LARGE sparse/dense match O(arms) overall: `emit_probe_chain` re-attempts this table
    // on every recursive `rest`, so without the gate a 6400-arm match rebuilt a shrinking O(arms) vector
    // at each of ~6400 levels — O(arms²). (A dense SUFFIX of <=256 arms still becomes eligible and emits
    // its table exactly as before — byte-identical; only the always-doomed long-prefix attempts are cut.)
    if int_arms.len() > 256 {
        return Ok(None);
    }
    let mut lits: Vec<i64> = Vec::with_capacity(int_arms.len());
    for a in int_arms {
        match &a.probe {
            crate::core::Probe::Int(v) if a.guard.is_none() => match v.to_i64() {
                Some(x) => lits.push(x),
                None => return Ok(None), // a value that doesn't fit i64 — fall back.
            },
            _ => return Ok(None), // a guard, a bool probe, or a wildcard mid-list — fall back.
        }
    }
    // Density: ≥3 arms, contiguous-enough range, capped table size.
    let min = *lits.iter().min().unwrap();
    let max = *lits.iter().max().unwrap();
    let span: i128 = max as i128 - min as i128 + 1;
    let count = lits.len() as i128;
    if count < 3 || span > 2 * count || span > 256 {
        return Ok(None);
    }
    let span = span as u32;
    // The table: index `i` (a shifted value `min + i`) → the arm whose literal is `min + i`, or the
    // default. `arm_at[i] = Some(arm_index)` maps a covered slot to its position in `int_arms`.
    let mut arm_at: Vec<Option<usize>> = vec![None; span as usize];
    for (ai, &lit) in lits.iter().enumerate() {
        let slot = (lit - min) as usize;
        if arm_at[slot].is_some() {
            return Ok(None); // duplicate literal — fall back (the chain handles it, first-wins).
        }
        arm_at[slot] = Some(ai);
    }
    let m = Machine::of(it);

    // Open the ONE typed join block, the default label block, then one empty block per COVERED arm
    // (arm 0 innermost). The br_table's targets index into these by SHIFTED VALUE, remapped to the
    // covering arm's block depth (a gap slot → the default depth).
    out.push(Lir::Block(block_ty)); // $join (typed)
    out.push(Lir::Block(BlockType::Empty)); // $default
    let n_arms = int_arms.len() as u32;
    for _ in 0..n_arms {
        out.push(Lir::Block(BlockType::Empty)); // $a_{n-1} … $a_0 (innermost = arm 0)
    }
    // Compute the shifted index `scrutinee - min` in the scrutinee's slot width.
    // At the innermost point the enclosing blocks (inner→outer) are: a_0 … a_{n-1}, default, join.
    // `br d`: d in 0..n → $a_d ; d = n → $default ; d = n+1 → $join.
    let default_depth = n_arms; // exits $default
    src.push(out);
    // The shifted index is `scrutinee - min`; when the covered range STARTS AT 0 (the common `(match x
    // (0 …) (1 …) …)` shape) the shift is the identity `x - 0`, so skip the dead `const 0 ; sub` — the
    // scrutinee IS the table index. (`m.sub()` wraps, so `x - 0 == x` exactly at both slot widths.)
    if min != 0 {
        out.push(m.konst(min));
        out.push(m.sub());
    }
    if !m.slot32 {
        // i64 scrutinee: guard against the wrap-aliasing (idx as u64 >= span → default), then narrow.
        let idx_slot = base;
        if idx_slot + 1 > *high {
            *high = idx_slot + 1;
        }
        scratch_ty.insert(idx_slot, ValType::I64);
        out.push(Lir::LocalTee(idx_slot)); // keep idx, leave a copy on the stack
        out.push(Lir::ConstI64(span as i64));
        out.push(Lir::I64GeU);
        out.push(Lir::BrIf(default_depth)); // out of range → default (br_if pops the bool)
        out.push(Lir::LocalGet(idx_slot));
        out.push(Lir::I32WrapI64);
    }
    // Targets: one entry per SHIFTED VALUE `0..span`, each the block depth of the covering arm, or the
    // default depth for a gap. Arm `ai` (position in `int_arms`) sits at block depth `ai` (a_0 innermost).
    let targets: Vec<u32> = (0..span as usize)
        .map(|i| match arm_at[i] {
            Some(ai) => ai as u32,
            None => default_depth,
        })
        .collect();
    out.push(Lir::BrTable(targets, default_depth));

    // Emit each covered arm's body after its label's `end`, innermost (arm 0) first, then `br` its value
    // to $join. After `End`ing $a_0..$a_k, the enclosing blocks (inner→outer) are a_{k+1}…a_{n-1} (that is
    // `n_arms - 1 - k` blocks), then $default (1 block), then $join — so $join sits at DEPTH
    // `(n_arms - 1 - k) + 1 = n_arms - k` (the count of blocks BEFORE it; $join is AT that depth, not one
    // past). This mirrors `try_emit_disc_br_table`'s `(m - 1 - k) + join_from_arm_extra` with the always-
    // present $default block (extra = 1). A bare `n_arms - k + 1` branched ONE BLOCK TOO FAR — past $join
    // to the FUNCTION-result label, so in NON-tail position the arm value escaped the whole function and the
    // consuming code (`+ 100`, a `bytes-concat`, a `let` body) never ran (a silent wrong value; the default
    // arm, which falls through to $join with no `br`, was unaffected — masking the bug in tail position
    // where the function result IS $join).
    for (k, arm) in int_arms.iter().enumerate() {
        out.push(Lir::End); // close $a_k → br_table target `k` lands here
        emit_arm_body(
            db,
            arm.body,
            result_it,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
        out.push(Lir::Br(n_arms - k as u32)); // → $join, carrying the value
    }
    // Close $default; emit the default body (falls through to $join's end — no `br` needed).
    out.push(Lir::End); // close $default
    emit_arm_body(
        db,
        default.body,
        result_it,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        TailPos::NonTail,
    )?;
    out.push(Lir::End); // close $join
    Ok(Some(()))
}

/// A [`TailPos`] one `if` block deeper — a self-loop `br` from inside a fresh `if` targets one level
/// further out. Shared by the probe chain and the guarded-body emit (each opens an `if`).
fn deeper_tail(tail: TailPos) -> TailPos {
    match tail {
        TailPos::Tail(tl) => TailPos::Tail(tl.map(|t| TailLoop {
            depth: t.depth + 1,
            ..t
        })),
        TailPos::NonTail => TailPos::NonTail,
    }
}

/// Emit a match-arm BODY at [`TailPos`] `tp`. Every arm produces the match's RESULT type, so a bare
/// `ConstInt` body is grounded to the result's integer width (`result_it`) — else a default-Int64 literal
/// arm beside a narrow arm pushes a mismatched slot and wasm rejects the block. A tail body goes through
/// `emit_tail` (a `ConstInt` is never a tail call); `tp` carries the self-loop context.
#[allow(clippy::too_many_arguments)]
fn emit_arm_body(
    db: &mut Db,
    body: StructId,
    result_it: Option<IntTy>,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tp: TailPos,
) -> Result<(), Reject> {
    if let (Some(rit), Core::ConstInt(_)) = (result_it, core_of(db, body)) {
        return emit_operand(db, body, rit, slots, base, high, scratch_ty, layout, out);
    }
    match tp {
        TailPos::Tail(tl) => emit_tail(db, body, slots, base, high, scratch_ty, layout, out, tl),
        TailPos::NonTail => emit(db, body, slots, base, high, scratch_ty, layout, out),
    }
}

/// Emit a runtime LIST match's arms as a length-dispatch `if`-chain, each ARM BODY at [`TailPos`] `tail`.
/// The list handle is already materialized (its slot is in `arm_slots` under the scrutinee) and `len_slot`
/// holds `vec-len`. Each non-final arm tests its length condition and, on match, emits its body; the final
/// (or `Any`) arm is the unconditional `else`. In TAIL position each arm body is emitted via `emit_arm_body`
/// (so a tail self-call in an arm becomes a `return_call` / loop iteration) — and since each preceding
/// non-final arm nests the remaining arms one `if` DEEPER, the threaded `TailLoop.depth` bumps +1 per level
/// (via `deeper_tail`) so a self-loop `br` targets the loop top correctly. `result_it` grounds a
/// bare-`ConstInt` arm body to the match's integer result width (as `emit_arm_body` does for scalar arms).
#[allow(clippy::too_many_arguments)]
fn emit_list_arms_tailable(
    db: &mut Db,
    arms: &[crate::core::ListArm],
    len_slot: u32,
    block_ty: BlockType,
    result_it: Option<IntTy>,
    arm_slots: &HashMap<StructId, u32>,
    arm_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    let Some((first, rest)) = arms.split_first() else {
        out.push(Lir::Unreachable);
        return Ok(());
    };
    // An UNGUARDED `Any` (or the final) arm is the unconditional tail. A GUARDED arm — even an `Any`/rest
    // one — may FAIL its guard, so it is NOT unconditional: it still tests its guard and falls through.
    let is_tail_arm = first.guard.is_none()
        && (rest.is_empty() || matches!(first.cond, crate::core::ListArmCond::Any));
    if is_tail_arm {
        // The unconditional final arm — its body is in the SAME tail position as the whole match.
        return emit_arm_body(
            db, first.body, result_it, arm_slots, arm_base, high, scratch_ty, layout, out, tail,
        );
    }
    // BRANCHLESS 2-ARM LIST SELECT: a list match of exactly TWO arms — a LENGTH-test arm then a single
    // unconditional cover (an `Any`/final rest arm) — is `(if (len ⋈ k) body0 body1)`, the list analogue of
    // the scalar/sum 2-arm select. When both are UNGUARDED with cheap trap-free `is_select_arm` bodies and
    // the result is a scalar, emit `body0 ; body1 ; (len ⋈ k) ; select` instead of an `if`/`else` block —
    // so `(match xs ((list) 0) ((list a .. r) 1))` (dispatch on `len == 0`) goes branchless. Only for
    // NON-self-loop position (`select` cannot carry a loop `br`; a trap-free body is never a tail call).
    // A body that reads an ELEMENT/REST binder does so via `SumPayload` — NOT trap-free — so `is_select_arm`
    // declines and the structured `if` survives (no speculative out-of-bounds element read on the wrong
    // arm), exactly as a payload-reading sum arm keeps its `if`.
    // `rest` is a single UNGUARDED arm: it is the last arm of an exhaustive match, so it is the
    // UNCONDITIONAL cover — the fall-through emits its body with NO cond re-test (the `is_tail_arm` rule),
    // so its own `cond` (whether `Any` or a now-redundant length like `LenGe(1)` complementing the first
    // arm's `LenEq(0)`) is irrelevant. Any single unguarded `rest` arm qualifies.
    if !matches!(tail, TailPos::Tail(Some(_)))
        && matches!(block_ty, BlockType::Val(_))
        && first.guard.is_none()
        && !matches!(first.cond, crate::core::ListArmCond::Any)
        && let [cover] = rest
        && cover.guard.is_none()
        && is_select_arm(db, first.body)
        && is_select_arm(db, cover.body)
    {
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => type_of(db, first.body),
        };
        emit_branch(
            db, first.body, &res_ty, arm_slots, arm_base, high, scratch_ty, layout, out,
        )?;
        emit_branch(
            db, cover.body, &res_ty, arm_slots, arm_base, high, scratch_ty, layout, out,
        )?;
        out.push(Lir::LocalGet(len_slot));
        match first.cond {
            crate::core::ListArmCond::LenEq(n) => {
                out.push(Lir::ConstI32(n as i32));
                out.push(Lir::I32Eq);
            }
            crate::core::ListArmCond::LenGe(k) => {
                out.push(Lir::ConstI32(k as i32));
                out.push(Lir::I32GeU);
            }
            crate::core::ListArmCond::Any => unreachable!("guarded by the matches! above"),
        }
        out.push(Lir::Select);
        return Ok(());
    }
    // Open the LENGTH test — except for an `Any` cond (a guarded catch-all/rest), whose length always holds
    // so its only gate is the guard. For a length-carrying cond, `if (len ⋈ k)` wraps the arm.
    let has_len_test = !matches!(first.cond, crate::core::ListArmCond::Any);
    if has_len_test {
        out.push(Lir::LocalGet(len_slot));
        match first.cond {
            crate::core::ListArmCond::LenEq(n) => {
                out.push(Lir::ConstI32(n as i32));
                out.push(Lir::I32Eq);
            }
            crate::core::ListArmCond::LenGe(k) => {
                out.push(Lir::ConstI32(k as i32));
                out.push(Lir::I32GeU);
            }
            crate::core::ListArmCond::Any => unreachable!(),
        }
        out.push(Lir::If(block_ty));
    }
    // Inside the length `if` (or unconditionally, for an `Any` guarded arm): emit the arm's body, gated on
    // its GUARD when present. A guarded arm becomes `if guard then body else <rest>` — a false guard FALLS
    // THROUGH to the remaining arms, exactly as a false length test does; the guard is a boolean the arm's
    // element/rest binders are in scope for (resolve Case 6lg), emitted as an operand before the `if`. The
    // body/rest sit one `if` deeper per opened `if`, so the tail depth bumps accordingly.
    let after_len_tail = if has_len_test {
        deeper_tail(tail)
    } else {
        tail
    };
    match first.guard {
        None => {
            emit_arm_body(
                db,
                first.body,
                result_it,
                arm_slots,
                arm_base,
                high,
                scratch_ty,
                layout,
                out,
                after_len_tail,
            )?;
        }
        Some(g) => {
            // The guard reads the scrutinee handle (in `arm_slots`) via its binders' `SumPayload`; emit it
            // as an i32 boolean at `arm_base`. The body/rest start scratch ABOVE the guard's high-water (a
            // guard stashing a heap handle types a low slot i32; a body reusing that slot at i64 would fail
            // validation — the same discipline the scalar guard emit follows).
            emit(db, g, arm_slots, arm_base, high, scratch_ty, layout, out)?;
            let body_base = *high;
            out.push(Lir::If(block_ty));
            let deeper = deeper_tail(after_len_tail);
            emit_arm_body(
                db, first.body, result_it, arm_slots, body_base, high, scratch_ty, layout, out,
                deeper,
            )?;
            out.push(Lir::Else);
            emit_list_arms_tailable(
                db, rest, len_slot, block_ty, result_it, arm_slots, body_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
        }
    }
    if has_len_test {
        out.push(Lir::Else);
        // The remaining arms are ALSO one `if` deeper — pass the bumped tail.
        emit_list_arms_tailable(
            db,
            rest,
            len_slot,
            block_ty,
            result_it,
            arm_slots,
            arm_base,
            high,
            scratch_ty,
            layout,
            out,
            deeper_tail(tail),
        )?;
        out.push(Lir::End);
    }
    Ok(())
}

/// Emit a GUARDED arm's body gated on its guard (the caller has already established that the arm's
/// PROBE matched — for a literal probe, inside its `if`; for a `Wild` probe, unconditionally). Emits
/// `if guard body else <rest>` — a false guard falls through to the remaining arms, exactly as a
/// non-matching pattern does (`core-semantics.md` §Matching Is Exhaustive Or Rejected). An UNGUARDED arm
/// (reached only via a literal probe whose guard is `None`) emits its body directly. The guard is a
/// boolean value (an i32); it is emitted at `base` (a fresh scratch region, its result consumed by the
/// `if`).
#[allow(clippy::too_many_arguments)]
fn emit_arm_guarded_body(
    db: &mut Db,
    scrutinee: StructId,
    arm: &crate::core::MatchArm,
    src: OperandSrc,
    rest: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    inner: TailPos,
) -> Result<(), Reject> {
    // This arm's PROBE matched to reach here — for a literal `Int` probe over a variable scrutinee, the
    // scrutinee EQUALS that literal, so refine its range to `[c, c]` for the BODY (a `(- n 1)` in the
    // `(5 …)` arm computes `4`, its guard dead). The GUARD is a boolean the arm gates on and is NOT
    // refined (a guard like `(> n 5)` reading the same variable must still be evaluated); only the body,
    // reached once the probe (and guard) held, sees the refinement. `Wild`/`Bool` probe → no refinement.
    let body_frame =
        refined_frame_for_match_arm(db, scrutinee, &arm.probe, db.current_refinements());
    match arm.guard {
        None => {
            db.push_range_refinements(body_frame);
            let r = emit_arm_body(
                db, arm.body, result_it, slots, base, high, scratch_ty, layout, out, inner,
            );
            db.pop_range_refinements();
            r
        }
        Some(g) => {
            // `if guard body else <rest>`. The guard is a plain boolean value (never a tail call), so it
            // is emitted with `emit` at `base`; its result is the `if` condition.
            emit(db, g, slots, base, high, scratch_ty, layout, out)?;
            // The body and fallthrough start scratch ABOVE the high-water the GUARD reached, NOT at
            // `base` — the same discipline as the `Core::If` arms. A guard that stashes an i32 HEAP
            // HANDLE (a runtime `value-eq`/`MatchSum` on constructed sums, `(guard x (= (mk x) (mk 3)))`)
            // types a low slot i32 for the whole function; the fallthrough's loop-iteration i64 arith
            // (`(find (+ n 1))`) reusing that slot number at a different width fails validation. A scalar
            // guard leaves `*high == base`, so this is byte-identical for the common case.
            let body_base = *high;
            out.push(Lir::If(block_ty));
            // Both the body and the fallthrough are one `if` deeper than this arm's nesting.
            let deeper = deeper_tail(inner);
            // The BODY (probe matched AND guard held) sees the `[c,c]` refinement; the fall-through `rest`
            // does NOT (the probe failed there — its own arms refine themselves).
            db.push_range_refinements(body_frame);
            let body_res = emit_arm_body(
                db, arm.body, result_it, slots, body_base, high, scratch_ty, layout, out, deeper,
            );
            db.pop_range_refinements();
            body_res?;
            out.push(Lir::Else);
            emit_probe_chain(
                db, scrutinee, src, rest, it, result_it, block_ty, slots, body_base, high,
                scratch_ty, layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Try to emit a sum-discriminant switch as a BR_TABLE decision tree (O(1) jump) instead of the linear
/// `if (disc == k)` chain. Returns `Ok(Some(()))` when it emitted the table, `Ok(None)` to fall back to
/// the linear chain. Eligible when the arms are a set of ≥3 DISTINCT explicit discriminants (each
/// `disc: Some`), optionally followed by ONE trailing default (`disc: None`); a leading/mid default, or
/// fewer than 3 discs, falls back (the linear chain is fine and simpler there).
///
/// The value-producing structure — for discriminants `d_0..d_{m-1}` (each with a continuation) and a
/// default continuation, all yielding `block_ty`:
/// ```text
///   block $join (block_ty)          ; the ONE typed block; every arm br's its value here
///     block $default                ; empty control-flow labels …
///       block $a_{m-1} … block $a_0 ;   ($a_0 innermost)
///         <disc>                    ; sum-disc(scrutinee walked to `path`) → i32 on the stack
///         br_table [0,1,…,m-1] m    ;   index k → exits $a_k; out-of-range → exits $default
///       end                         ; $a_0 label → cont_0 runs here
///       <cont_0> ; br $join
///     end                           ; $a_1 label
///       <cont_1> ; br $join
///     … end $a_{m-1} <cont_{m-1}> ; br $join
///     end                           ; $default label
///     <default cont>                ; falls through to $join's end (no br needed — it is last)
///   end
/// ```
/// The inner blocks are EMPTY (jump labels only); only `$join` carries the result type, so the stack is
/// empty at each `br_table` target and each `end` is reached only via a `br` that already pushed the
/// value to `$join` — a well-typed structure wasm accepts. The `br_table` index maps arm position → its
/// label depth; a discriminant not in `0..m` (impossible for an exhaustive sum, but the ABI is total)
/// takes the default. NOTE: this handles the ROOT and any nested switch uniformly (the discriminant is
/// read at `path`); a continuation that is itself a nested switch still recurses through `emit_sum_cont`.
#[allow(clippy::too_many_arguments)]
fn try_emit_disc_br_table(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<Option<()>, Reject> {
    // Partition into explicit-disc arms (the table entries) and an optional trailing default.
    let (disc_arms, default): (&[crate::core::SumArm], Option<&crate::core::SumArm>) =
        match arms.last() {
            Some(a) if a.disc.is_none() => (&arms[..arms.len() - 1], Some(a)),
            _ => (arms, None),
        };
    // Every table arm must carry an explicit discriminant (a default anywhere but last → fall back).
    if disc_arms.len() < 3 || disc_arms.iter().any(|a| a.disc.is_none()) {
        return Ok(None);
    }
    // Distinct discriminants, and each in `0..disc_arms.len()` so a table position IS its discriminant
    // (sum discs are contiguous `0..k`; a match lists each variant once). If the discs are not exactly
    // the contiguous set `0..m` in arm order, fall back rather than build a sparse/misindexed table.
    let discs: Vec<u32> = disc_arms.iter().map(|a| a.disc.unwrap()).collect();
    let m = discs.len() as u32;
    let contiguous_in_order = discs.iter().enumerate().all(|(i, &d)| d == i as u32);
    if !contiguous_in_order {
        return Ok(None);
    }
    // EXHAUSTIVE-MATCH DEFAULT ELISION: with NO default arm the match lists every variant, and the discs
    // are exactly the contiguous `0..m` (checked above), so the discriminant is ALWAYS in `[0, m)` — the
    // `br_table`'s own out-of-range default is DEAD. Rather than a `$default` block wrapping a stack-
    // polymorphic `unreachable`, the LAST arm serves as the table default (`br_table 0 … m-2  default=m-1`):
    // one fewer block and no dead `unreachable`. When a real default arm IS present it keeps its own block
    // (the table default routes there for a disc the arms do not cover — though for a sum that cannot occur
    // either, the arm is still emitted since the shape allows a user wildcard).
    let has_default_block = default.is_some();
    // Open the ONE typed join block, then the label blocks: `m` arm labels, plus the `$default` label ONLY
    // when a default arm is present. Innermost = arm 0.
    // Block nesting at the innermost point (outermost→innermost): join, [default], a_{m-1}, …, a_0.
    // From there `br d` exits: d=0 → $a_0, …, d=m-1 → $a_{m-1}, then [d=m → $default,] d=(m+default) → $join.
    out.push(Lir::Block(block_ty)); // $join (typed)
    if has_default_block {
        out.push(Lir::Block(BlockType::Empty)); // $default
    }
    for _ in 0..m {
        out.push(Lir::Block(BlockType::Empty)); // $a_{m-1} … $a_0
    }
    // Push the discriminant at `path` — `sum-disc` for a boxed sum, the raw i32 / unboxed int for an
    // enum-disc value (see `push_discriminant`).
    push_discriminant(
        db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
    )?;
    if has_default_block {
        // Target k (arm index) → depth k (exits $a_k); table default → depth m (exits $default).
        let targets: Vec<u32> = (0..m).collect();
        out.push(Lir::BrTable(targets, m));
    } else {
        // Exhaustive: disc ∈ [0, m). Index k (0..m-1) → $a_k; the table default IS the last arm $a_{m-1}
        // (depth m-1), the disc that necessarily remains — no separate default block.
        let targets: Vec<u32> = (0..m - 1).collect();
        out.push(Lir::BrTable(targets, m - 1));
    }
    // Now emit each arm body after its label's `end`, in innermost→outermost order (arm 0 first). After
    // closing block $a_k, control from `br_table` index k (or, for the last arm without a default block,
    // the table default) lands here; run the continuation and `br` its value to $join.
    // After `end`ing $a_0..$a_k the enclosing arm blocks (inner→outer) are a_{k+1}, …, a_{m-1}, then
    // [$default,] $join. So $join is `(m-1-k)` arm blocks out, plus 1 more if a $default block sits below.
    let join_from_arm_extra = if has_default_block { 1 } else { 0 };
    for (k, arm) in disc_arms.iter().enumerate() {
        out.push(Lir::End); // close $a_k → its br_table target lands here
        // The br_table path is only taken in NON-tail position (`emit_sum_match_arms` skips it when
        // looping — see there), so a continuation here is never a loop iteration.
        // RECORD this arm's entered-variant payload type (like the linear switch) so a nested switch /
        // literal-test in the continuation resolves a `Payload` step to the actual variant, not variant 0.
        let disc = arm
            .disc
            .expect("a table arm carries an explicit discriminant");
        let restore = record_entered_payload_ty(db, scrutinee, path, disc, out);
        emit_sum_cont(
            db,
            scrutinee,
            &arm.cont,
            result_it,
            block_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
        restore_entered_payload_ty(path, restore, out);
        // `br` the value to $join — EXCEPT the last arm of an EXHAUSTIVE match (no $default block), whose
        // `br` depth is 0: its body is the final code inside $join, so control falls THROUGH to $join's
        // `end` anyway. A `br 0` there jumps to exactly the next instruction (the `End` below) — a dead
        // branch. Skip it: the value stays on the stack and the block ends, identical behavior, one fewer
        // instruction. (A $default block, when present, sits between the last arm and $join, so the last
        // arm's depth is ≥1 and the `br` is real — the guard `!has_default_block` covers that.)
        let depth = (m - 1 - k as u32) + join_from_arm_extra;
        if depth != 0 {
            out.push(Lir::Br(depth)); // br to $join, carrying the value
        }
    }
    // Close $default and emit its continuation (falls through to $join's end — no `br` needed). Only when
    // a real default arm exists; an exhaustive match has no $default block (the last arm covered it).
    if let Some(d) = default {
        out.push(Lir::End); // close $default
        emit_sum_cont(
            db,
            scrutinee,
            &d.cont,
            result_it,
            block_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
    }
    out.push(Lir::End); // close $join
    Ok(Some(()))
}

/// Record the payload type of the variant `disc` (entered by a switch arm on the sub-value at `path`) into
/// `out.sum_path_types` at `path + [Payload]`, so a nested switch / literal-test / disc-walk in the arm's
/// continuation resolves a `Payload` step to the ACTUAL entered variant's payload (not variant 0's). Returns
/// the PRIOR value at that key for [`restore_entered_payload_ty`] to put back (scoped save/restore, so the
/// ELSE fall-through and sibling arms are unaffected). A no-op (`None` inserted-nothing marker via a bool)
/// when the sub-value is not a boxed sum with a resolvable payload — mirrors the Rust backend's
/// `sum_path_types` recording. The key is `path + [Payload]`; the returned `Option<Option<Ty>>` is
/// `Some(prior)` when a key was inserted (prior may be `None` = was absent), `None` when nothing was
/// inserted (a nullary/unresolvable variant — nothing to restore).
fn record_entered_payload_ty(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    disc: u32,
    out: &mut Emit,
) -> Option<Option<Ty>> {
    record_entered_payload_ty_into(db, scrutinee, path, disc, &mut out.sum_path_types)
}

/// Undo [`record_entered_payload_ty`]: restore the prior value at `path + [Payload]` (or remove the key if
/// it was absent). A `None` `restore` (nothing was inserted) is a no-op.
fn restore_entered_payload_ty(
    path: &[crate::core::PathStep],
    restore: Option<Option<Ty>>,
    out: &mut Emit,
) {
    restore_entered_payload_ty_into(path, restore, &mut out.sum_path_types);
}

/// The map-level core of [`record_entered_payload_ty`] — records the entered variant's payload type into
/// `recorded` at `path + [Payload]`. Shared by the emit (over `Emit::sum_path_types`) and the ops collector
/// (over its own scratch map) so both resolve a `Payload` step to the same entered-variant type.
fn record_entered_payload_ty_into(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    disc: u32,
    recorded: &mut HashMap<Vec<crate::core::PathStep>, Ty>,
) -> Option<Option<Ty>> {
    let root = type_of(db, scrutinee);
    let sub = ty_at_path_recorded(db, &root, path, recorded);
    let payload = variant_payload_ty_at(db, &sub, disc)?;
    let mut key = path.to_vec();
    key.push(crate::core::PathStep::Payload);
    let prior = recorded.insert(key, payload);
    Some(prior)
}

/// The map-level core of [`restore_entered_payload_ty`].
fn restore_entered_payload_ty_into(
    path: &[crate::core::PathStep],
    restore: Option<Option<Ty>>,
    recorded: &mut HashMap<Vec<crate::core::PathStep>, Ty>,
) {
    let Some(prior) = restore else {
        return;
    };
    let mut key = path.to_vec();
    key.push(crate::core::PathStep::Payload);
    match prior {
        Some(t) => {
            recorded.insert(key, t);
        }
        None => {
            recorded.remove(&key);
        }
    }
}

/// Emit one SWITCH of the decision tree: for each variant arm, `sum-disc(<scrutinee walked to `path`>)
/// == disc`, then `if (block_ty) <continuation> else <rest>`; a default arm (`disc: None`) or the LAST
/// arm (its probe redundant — every earlier disc has been tested and this is the only one left) is the
/// unconditional tail. `path` reaches the sub-value THIS switch dispatches on — empty for the ROOT (the
/// scrutinee itself), a `[Payload…]` path for a NESTED switch. Each arm's CONTINUATION is a leaf body or
/// a deeper switch (`emit_sum_cont`), which is what makes the whole match a decision tree that shares the
/// outer probe. Mirrors `emit_match_arms_tailable` but probes the discriminant. (A dense set of ≥3 discs
/// takes the `try_emit_disc_br_table` fast path before this linear chain.)
#[allow(clippy::too_many_arguments)]
fn emit_sum_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    // BR_TABLE DECISION TREE: a switch that tests ≥3 DISTINCT discriminants dispatches in O(1) via a
    // jump table instead of a linear `if (disc == k)` cascade (the arms below). Sum discriminants are
    // contiguous `0..variant_count`, so the table is dense with no wasted slots. `try_emit_disc_br_table`
    // returns `Some(())` when it emitted the table, `None` to fall through to the linear chain (too few
    // arms, or a shape it does not handle — a leading default, non-distinct discs).
    // SKIPPED ONLY FOR A SELF-LOOP (`Tail(Some(tl))`): the table wraps its arm continuations in nested
    // control-flow BLOCKS (`$join`/`$a_k`), a different block-nesting than the linear `if`-chain; a
    // self-tail-call compiled as a loop `br tl.depth` inside an arm would need a table-specific depth (not
    // the `deeper_tail` +1-per-`if` accounting the linear chain uses). The linear chain loops correctly and
    // covers the common recursive-sum shapes (2-variant Cons/Nil, Succ/Zero, Node/Leaf never hit the
    // ≥3-disc table anyway), so fall back to it when a self-loop is in play. A `NonTail` match (a sum match
    // used as an operand) OR a `Tail(None)` one (a non-self-recursive function body — EVERY body is emitted
    // via `emit_tail`, so this is the common case) keeps the O(1) table: the table's continuations are
    // emitted `NonTail` (a `return_call` `br`s to `$join` fine — it's frame-replacing, not depth-relative;
    // and a self-loop `br` never occurs here since there is no loop), so it is byte-identical to the
    // pre-tail behavior for both.
    if !matches!(tail, TailPos::Tail(Some(_)))
        && let Some(()) = try_emit_disc_br_table(
            db, scrutinee, path, arms, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out,
        )?
    {
        return Ok(());
    }
    match arms.split_first() {
        None => Err(Reject::decline(
            "sum match ran off the end with no covering arm",
        )),
        // A default arm, or the last arm of an exhaustive switch — its probe is redundant, so emit its
        // continuation unconditionally (in the SAME tail position as the whole switch — no `if` opened).
        Some((arm, [])) => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
        Some((arm, _)) if arm.disc.is_none() => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
        Some((arm, rest)) => {
            let disc = arm.disc.expect("non-None handled above");
            // BRANCHLESS 2-ARM SUM SELECT: a switch of exactly TWO arms — this disc-arm then a single
            // unconditional cover (a `disc: None` default, or the last arm of an exhaustive switch) — is
            // `(if (disc == d) then else)`, the sum-discriminant twin of the scalar 2-arm select
            // (`emit_match_dispatch`). When both arms are `Leaf` bodies that are cheap trap-free
            // `is_select_arm`s and the result is a scalar (a value `block_ty`), emit `then ; else ;
            // (disc == d) ; select` instead of an `if`/`else` block — so a 2-variant enum match
            // `(match f (On 1) (Off 0))` goes branchless (`disc eqz ; select`) exactly as the equivalent
            // `if` would. Only for NON-self-loop position (`select` cannot carry a loop `br`; a `Leaf`
            // select-arm is never a tail call anyway) and when both continuations are plain leaves — a
            // guarded / nested-switch / lit-test continuation keeps the structured `if` below.
            if !matches!(tail, TailPos::Tail(Some(_)))
                && matches!(block_ty, BlockType::Val(_))
                && let [cover] = rest
                && let crate::core::SumCont::Leaf(then_body) = &arm.cont
                && let crate::core::SumCont::Leaf(else_body) = &cover.cont
                && is_select_arm(db, *then_body)
                && is_select_arm(db, *else_body)
            {
                let (then_body, else_body) = (*then_body, *else_body);
                let res_ty = match result_it {
                    Some(rit) => Ty::Int(rit),
                    None => type_of(db, then_body),
                };
                emit_branch(
                    db, then_body, &res_ty, slots, base, high, scratch_ty, layout, out,
                )?;
                emit_branch(
                    db, else_body, &res_ty, slots, base, high, scratch_ty, layout, out,
                )?;
                push_discriminant(
                    db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
                )?;
                if disc == 0 {
                    out.push(Lir::I32Eqz);
                } else {
                    out.push(Lir::ConstI32(disc as i32));
                    out.push(Lir::I32Eq);
                }
                out.push(Lir::Select);
                return Ok(());
            }
            // discriminant(<scrutinee walked down `path`>) == disc — `sum-disc` for a boxed sum, the raw
            // i32 / unboxed int for an enum-disc value (see `push_discriminant`).
            push_discriminant(
                db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
            )?;
            // `disc == 0` is `i32.eqz` (one instruction), not `const 0 ; i32.eq` (two) — the sum-disc
            // twin of the scalar/probe eqz special case (cycle 43). A `0` discriminant is the FIRST
            // declared variant (`Some`, `Ok`, …), so this fires on the common first-arm test.
            if disc == 0 {
                out.push(Lir::I32Eqz);
            } else {
                out.push(Lir::ConstI32(disc as i32));
                out.push(Lir::I32Eq);
            }
            out.push(Lir::If(block_ty));
            // The matched arm's continuation and the fall-through switch both sit one `if` deeper — bump
            // the tail depth so a self-loop `br` inside either targets the loop top (mirrors the scalar
            // `emit_probe_chain` / list `emit_list_arms_tailable` disc-nesting).
            let deeper = deeper_tail(tail);
            // RECORD this entered variant's payload type at `path + [Payload]` so a NESTED switch / literal-
            // test / disc-walk in the arm's continuation resolves a `Payload` step to the ACTUAL entered
            // variant's payload — not variant 0. Scoped save/restore fences it to this arm (the ELSE
            // fall-through and sibling arms must not see it). Only for a boxed sum with a real payload.
            let restore = record_entered_payload_ty(db, scrutinee, path, disc, out);
            emit_sum_cont(
                db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out, deeper,
            )?;
            restore_entered_payload_ty(path, restore, out);
            // The fall-through switch (the disc-test's ELSE) starts scratch ABOVE the high-water the
            // matched arm's continuation (the THEN) reached, NOT at `base` — the same discipline as the
            // `Core::If` / guard sites. The THEN's continuation may contain a guard that stashes an i32
            // HEAP HANDLE (`value-eq`/`MatchSum`) in a low slot, typing it i32 for the whole function; the
            // ELSE's fall-through loop-iteration i64 arithmetic reusing that slot fails validation (the two
            // `if` branches share one function-global local declaration). A THEN that touches no heap
            // handle leaves `*high` where it was, so this is byte-identical for the common case.
            let else_base = *high;
            out.push(Lir::Else);
            emit_sum_match_arms(
                db, scrutinee, path, rest, result_it, block_ty, slots, else_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Emit a matched arm's CONTINUATION: a LEAF emits its body (a bare-`ConstInt` body grounded to the
/// match's result width `result_it`, as the scalar-match arms are); a nested SWITCH emits a fresh switch
/// chain on its deeper sub-value (`emit_sum_match_arms`), which is the decision tree recursing to share
/// the outer probe. The nested switch's `if`s reuse the SAME `block_ty` (both branches yield the match's
/// one result type at every depth). `tail` carries the [`TailPos`]: in a TAIL sum match each LEAF/GUARDED
/// body is a tail position (a self-tail-call there iterates the loop / becomes a `return_call`), and the
/// nested dispatch bumps the threaded loop `depth` +1 per enclosing `if` (via `deeper_tail`). `NonTail` is
/// byte-identical to the pre-tail behavior (bodies emit via `emit`).
#[allow(clippy::too_many_arguments)]
fn emit_sum_cont(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            // SHARED SUM-PAYLOAD-PREFIX CSE: if this arm body reads ≥2 elements off the same payload tuple
            // (`(Node (tuple l r))`), compute the shared `sum-payload` prefix ONCE into a slot here and
            // register it so each element's `SumPayload` emit reads the slot + walks only its suffix. The
            // slots are fenced to THIS arm body (removed after), so a sibling arm never reads a prefix that
            // its own scrutinee value did not populate.
            let prefix_keys = materialize_payload_prefixes(
                db, *body, base, high, scratch_ty, slots, layout, out,
            )?;
            // A bare-`ConstInt` leaf grounds to the result width (never a tail call); otherwise the body is
            // emitted at the ambient `tail` position — in a tail match a self-tail-call in the body loops.
            // The arm body emits ABOVE the reserved prefix slots (the base advanced by `materialize_*`).
            let arm_base = (*high).max(base);
            let r = emit_arm_body(
                db, *body, result_it, slots, arm_base, high, scratch_ty, layout, out, tail,
            );
            for key in prefix_keys {
                out.payload_prefix_slots.remove(&key);
            }
            r
        }
        // A GUARDED arm: `if cond then body else <els>`. The guard cond is a boolean (an i32); each of the
        // body and the fall-through `els` produces the match's result type (`block_ty`), grounding a
        // bare-literal body to the result width exactly as an `if` branch does. The `els` continuation
        // recurses — it is the rest of the sub-matrix (a later arm of the same variant, or the default).
        crate::core::SumCont::Guarded { cond, body, els } => {
            emit(db, *cond, slots, base, high, scratch_ty, layout, out)?;
            // The body and fall-through start scratch ABOVE the high-water the GUARD reached, NOT at
            // `base` — the same discipline as the `Core::If` / scalar-match-guard / probe-else sites. A
            // guard that stashes an i32 HEAP HANDLE (a runtime `value-eq`/`MatchSum` — `(guard (N.I x)
            // (= (mk x) (mk 3)))`) types a low slot i32 for the whole function; the fall-through's
            // loop-iteration i64 arithmetic reusing that slot fails validation. A scalar guard leaves
            // `*high == base`, so this is byte-identical for the common case.
            let body_base = *high;
            out.push(Lir::If(block_ty));
            // Both the body and the fall-through `els` sit one `if` deeper — bump the tail depth so a
            // self-loop `br` from either targets the loop top (mirrors `emit_arm_guarded_body`).
            let deeper = deeper_tail(tail);
            emit_arm_body(
                db, *body, result_it, slots, body_base, high, scratch_ty, layout, out, deeper,
            )?;
            out.push(Lir::Else);
            emit_sum_cont(
                db, scrutinee, els, result_it, block_ty, slots, body_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
        // A LITERAL TEST: `if (<sub-value at path> == literal) then <then_> else <els>`. Walk the `path`
        // from the scrutinee handle (`sum-payload`/`arr-get`, exactly as `Core::SumPayload` does), read the
        // leaf scalar (`get-int` → i64 / `get-bool` → i32), compare against the literal, and branch. Both
        // continuations recurse through `emit_sum_cont` and yield the match's result type (`block_ty`). The
        // `then_` typically ends in the arm body; `els` is the same-variant fall-through (the binding arm).
        // The read mirrors `SumPayload`'s walk + unbox; the compare mirrors `emit_probe_chain`'s Int/Bool
        // probe. A narrow-int payload's `get-int` yields the normalized i64, so an i64 compare against the
        // literal's i64 bits is exact (the pattern literal is in range or the arm is ill-typed and rejected
        // earlier).
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            // Push the scrutinee handle and walk to the leaf's boxed handle — tracking the sub-value TYPE as
            // the walk descends (mirrors `Core::SumPayload`), so an ERASED newtype `Payload` is a no-op (the
            // box is elided) and a `List` sub-value's `Elem` reads with `vec-get`, not `arr-get`. Without
            // this, a `(Bx (list …))` newtype's `ListLen` test called `sum-payload` on the raw list (garbage
            // length), and a boxed-list `Elem` used `arr-get` on a vec handle (garbage element).
            emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
            let mut cur = type_of(db, scrutinee);
            // Whether the value now on the stack is a HEAP HANDLE (needs `get-int`/`get-bool` to read the
            // scalar leaf out of the box) or a RAW SCALAR already (read directly). It starts a handle unless
            // the scrutinee is itself an unboxed scalar — an ERASED single-variant newtype over a scalar,
            // `(type W (Wrap Int64))`, whose value IS a bare i64 (no box). Each heap-child accessor below
            // (`sum-payload`/`arr-get`/`vec-get`) produces a child HANDLE (→ true); an erased `Payload`
            // no-op leaves the representation unchanged. WITHOUT this, a literal-payload test on an erased
            // scalar newtype (`(match (W.Wrap n) ((W.Wrap 0) …) ((W.Wrap x) …))`) emitted `get-int` on the
            // raw i64 — an i32-handle unbox over an i64 value → an INVALID component (`func failed to
            // validate: expected i32, found i64`), a decline-don't-miscompile violation. The binding arm
            // reads the same payload raw (bare `local.get`), so this aligns the literal arm with it.
            let mut holds_handle =
                !matches!(cur.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_));
            let mut lit_prefix: Vec<crate::core::PathStep> = Vec::with_capacity(path.len());
            for step in path {
                lit_prefix.push(*step);
                match step {
                    crate::core::PathStep::Payload => {
                        match cur.strip_nominal() {
                            // A boxed sum's payload is unwrapped with `sum-payload`; its type is the ENTERED
                            // variant's payload (from `sum_path_types`, else variant 0) — a following `Elem`
                            // needs it to pick vec-get vs arr-get, and a non-variant-0 list payload matched
                            // by a nested element pattern reads the wrong accessor without it.
                            Ty::Sum { .. } => {
                                out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                                holds_handle = true; // sum-payload yields the child HANDLE
                                cur = payload_step_ty_of(
                                    db,
                                    Some(scrutinee),
                                    &cur,
                                    &lit_prefix,
                                    &out.sum_path_types,
                                );
                            }
                            // An ERASED nominal newtype: the box is gone, so the `Payload` step is a static
                            // unwrap — NO `sum-payload` op, `cur` becomes the inner type. The stack value is
                            // UNCHANGED (still whatever the scrutinee was — a raw scalar for a scalar
                            // newtype), so `holds_handle` is left as-is.
                            inner => cur = inner.clone(),
                        }
                    }
                    crate::core::PathStep::Elem(i) => {
                        out.push(Lir::ConstI32(*i as i32));
                        holds_handle = true; // arr-get/vec-get yield the child HANDLE
                        if matches!(cur.strip_nominal(), Ty::List(_)) {
                            out.push(Lir::CallImport(OP_VEC_GET));
                            cur = match cur.strip_nominal() {
                                Ty::List(e) => (**e).clone(),
                                _ => Ty::Any,
                            };
                        } else {
                            out.push(Lir::CallImport(OP_ARR_GET));
                            cur = match cur.strip_nominal() {
                                Ty::Tuple(elems) => elems.get(*i).cloned().unwrap_or(Ty::Any),
                                _ => Ty::Any,
                            };
                        }
                    }
                    crate::core::PathStep::RestFrom(_) => {} // never on a sum-lit-test path
                }
            }
            // Read the leaf scalar and compare against the literal. A `0` literal (a `(Some 0)`/`(Ok 0)`
            // payload pattern) is `payload == 0` — `i64.eqz` (one instruction), not `const 0 ; eq` (two);
            // the sum-payload twin of the scalar-probe eqz special case.
            match probe {
                crate::core::Probe::Int(v) => {
                    // Read the scalar out of the box (`get-int` → NORMALIZED i64) when the leaf is a heap
                    // handle, and compare at i64. An ERASED scalar newtype instead left the RAW payload on
                    // the stack at its NATIVE machine width — i64 for `Int64`, but i32 for a NARROW newtype
                    // (`(Wrap UInt8)`/`Int8`/`Int16`/`Int32`, whose raw rep is an i32 slot). So the compare
                    // op must match the payload's actual width: `i64.eqz`/`i64.eq` over a boxed-or-i64 leaf,
                    // `i32.eqz`/`i32.eq` over a narrow raw leaf. Reading the raw scalar but comparing it at
                    // the hard-coded i64 emitted `i64.eqz` over an i32 → an INVALID component (the narrow
                    // twin of the Int64 invalid-component this branch first fixed; `holds_handle`=false but
                    // the width was still assumed i64). Boxed path stays i64 (`get-int` normalizes).
                    let slot32 = if holds_handle {
                        out.push(Lir::CallImport(OP_GET_INT)); // [i64] — normalized
                        false
                    } else {
                        // The erased payload's native slot: i32 for a narrow width (`≤ 32`), else i64. `cur`
                        // is the payload type after the path walk (an `Int` for a scalar newtype).
                        match cur.strip_nominal() {
                            Ty::Int(it) => Machine::of(*it).slot32,
                            _ => false, // non-narrow / unknown → i64 (Int64 and the prior behavior)
                        }
                    };
                    if v.to_i64_bits() == 0 {
                        out.push(if slot32 { Lir::I32Eqz } else { Lir::I64Eqz }); // [bool]
                    } else if slot32 {
                        out.push(Lir::ConstI32(v.to_i64_bits() as i32));
                        out.push(Lir::I32Eq); // [bool]
                    } else {
                        out.push(Lir::ConstI64(v.to_i64_bits()));
                        out.push(Lir::I64Eq); // [bool]
                    }
                }
                crate::core::Probe::Bool(b) => {
                    // Same erased-newtype gate: a boxed Bool payload unboxes with `get-bool`, an erased
                    // Bool newtype is already a raw i32 0/1 on the stack.
                    if holds_handle {
                        out.push(Lir::CallImport(OP_GET_BOOL)); // [i32]
                    }
                    out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
                    out.push(Lir::I32Eq); // [bool]
                }
                crate::core::Probe::Str(s) => {
                    // A string-literal payload over a RUNTIME value (`(Ast.Name "+")` matched on a runtime
                    // Ast, a `(k "lit")` map-value pattern): compare the leaf String handle against the
                    // literal by CONTENT — the same `value-eq` (`champ_eq`) physical-byte compare
                    // `Core::ValueEq` uses on two strings. The path walk above left the leaf String HANDLE on
                    // the stack — a BORROWED payload (`sum-payload`/`arr-get`/`vec-get` all borrow).
                    // Canonicalize it with `bytes-compact` (rope→flat, refcount-NEUTRAL: flattens in place,
                    // returns the SAME handle, so the borrow is neither consumed nor a fresh mint) so a rope
                    // payload and its flat twin compare equal — exactly as the `Core::ValueEq` emit does for a
                    // borrowed String operand. Save the compacted leaf handle in a slot, build the literal as a
                    // fresh OWNED `ConstStr` byte-leaf (canonical UTF-8, NFC by the reader — the same build the
                    // `Core::ConstStr` emit lays down, so `value-eq` compares two canonical leaves), `value-eq`
                    // (borrows + pops both → bool), then DROP the owned literal (the borrowed leaf is left to
                    // its owner — no drop, matching the `Core::ValueEq` borrowed-operand rule).
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [leaf'] — canonical flat leaf, same handle
                    let leaf_slot = *high;
                    let lit_slot = *high + 1;
                    *high += 2;
                    scratch_ty.insert(leaf_slot, ValType::I32);
                    scratch_ty.insert(lit_slot, ValType::I32);
                    out.push(Lir::LocalSet(leaf_slot)); // stash the borrowed leaf handle
                    // Build the literal string as a fresh flat UTF-8 byte-leaf (mirrors `Core::ConstStr`).
                    let bytes = s.as_bytes();
                    out.push(Lir::ConstI32(bytes.len() as i32));
                    out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [buf]
                    for (i, &byte) in bytes.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32));
                        out.push(Lir::ConstI32(byte as i32));
                        out.push(Lir::CallImport(OP_BYTES_SET)); // [buf]
                    }
                    out.push(Lir::LocalTee(lit_slot)); // [lit] — keep the owned literal handle for the drop
                    out.push(Lir::LocalGet(leaf_slot)); // [lit, leaf]
                    out.push(Lir::CallImport(OP_VALUE_EQ)); // pops both (borrowed) → [bool]
                    // DROP the owned literal (a fresh leaf we minted); the leaf handle is a borrowed payload,
                    // left to its owner. The bool result stays on the stack for the `if` below.
                    out.push(Lir::LocalGet(lit_slot));
                    out.push(Lir::CallImport(OP_DROP));
                    // `value-eq` left [bool] then we pushed/dropped the literal — the drop consumed its own
                    // arg, so the stack is back to [bool]. Fall through to the shared `if`.
                }
                crate::core::Probe::ListLen { len, at_least } => {
                    // A list-pattern payload over a RUNTIME list: the path walked to the sub-value's LIST
                    // HANDLE (an i32); its `vec-len` is the length to test. A FIXED-arity `(list p0…p_{n-1})`
                    // matches length EXACTLY `n` (`vec-len == n`); a rest `(list p… .. rest)` matches AT
                    // LEAST `n` (`vec-len >= n`, the tail binds the surplus). The leading element binders +
                    // the rest binder read the list on their own via `SumPayload{Elem}/{RestFrom}` (resolve
                    // Case 6l/6r), so this arm only emits the LENGTH gate. On a mismatch, control falls
                    // through to `els` exactly as an Int/Bool literal test does.
                    out.push(Lir::CallImport(OP_VEC_LEN)); // [len:i32]
                    out.push(Lir::ConstI32(*len as i32));
                    out.push(if *at_least { Lir::I32GeU } else { Lir::I32Eq }); // [bool]
                }
                crate::core::Probe::Char(_) => {
                    // A char-literal payload over a RUNTIME value: a `Char` has NO runtime machine rep yet
                    // (its `=` folds only at compile time), so there is no leaf handle to compare — a
                    // CONSTANT char payload folds the `Char` test instead (`build_tree`), never reaching
                    // here. Decline (like the runtime map-payload probe), never a miscompile.
                    return Err(Reject::decline(
                        "a char-literal payload over a runtime char is not yet matched at run time (no runtime char rep)",
                    ));
                }
                crate::core::Probe::MapHasKeys { .. } => {
                    // A map-pattern payload over a RUNTIME map: the key-presence gate would need a runtime
                    // `map-lookup` per key (and the value binders a runtime keyed read), not yet wired — a
                    // CONSTANT map folds the `MapHasKeys` test instead (`build_tree`), never reaching here.
                    // Decline (like the runtime string-payload probe), never a miscompile.
                    return Err(Reject::decline(
                        "a map-pattern payload over a runtime map is not yet matched at run time",
                    ));
                }
                crate::core::Probe::Wild => {
                    return Err(Reject::decline("a wildcard literal test is a compiler bug"));
                }
            }
            out.push(Lir::If(block_ty));
            // Both continuations sit one `if` deeper — bump the tail depth (mirrors the guard/switch sites).
            let deeper = deeper_tail(tail);
            emit_sum_cont(
                db, scrutinee, then_, result_it, block_ty, slots, base, high, scratch_ty, layout,
                out, deeper,
            )?;
            // The `els` continuation starts scratch above the `then_`'s high-water — same discipline as
            // the disc-switch/guard sites: a `then_` that stashes an i32 heap handle must not have its
            // slot reused by `els`'s i64 loop arithmetic (byte-identical when `then_` touches no handle).
            let els_base = *high;
            out.push(Lir::Else);
            emit_sum_cont(
                db, scrutinee, els, result_it, block_ty, slots, els_base, high, scratch_ty, layout,
                out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
        crate::core::SumCont::Switch { path, arms } => emit_sum_match_arms(
            db, scrutinee, path, arms, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
    }
}

/// The MACHINE realization of an integer type of width `N` and a signedness — the width-generic engine
/// every runtime op is emitted through. A value of width `N` lives in the smallest wasm slot that holds
/// it: an i32 for `N ≤ 32`, else an i64 (`slot32`). It sits there NORMALIZED — sign-extended if signed,
/// zero-extended if unsigned — which is exactly what the boundary lift and the constant emit produce, so
/// a machine op reads the true value. `Machine` carries the constants and op selectors keyed by the slot
/// width, plus whether `N` is NARROW (`N < slot bits`, so a machine op can produce a value that fits the
/// slot but not the N-bit type — caught by a range-check) versus FULL (`N == slot bits`, where the
/// machine op's own carry/borrow IS the type's overflow). Nothing here hard-codes 64.
#[derive(Clone, Copy)]
struct Machine {
    /// The language width `N` (1..=64).
    width: u32,
    signed: bool,
    /// Whether the value occupies an i32 slot (`N ≤ 32`) rather than an i64.
    slot32: bool,
}

impl Machine {
    fn of(it: IntTy) -> Machine {
        let width = it.ground_width();
        Machine {
            width,
            signed: it.ground_signed(),
            slot32: width <= 32,
        }
    }

    /// The bits of the machine slot (32 or 64).
    fn slot_bits(self) -> u32 {
        if self.slot32 { 32 } else { 64 }
    }

    /// The wasm value type of this machine's slot — the type a scratch local holding its value is
    /// declared at.
    fn slot(self) -> ValType {
        if self.slot32 {
            ValType::I32
        } else {
            ValType::I64
        }
    }

    /// Whether `N` is NARROWER than its slot — the case a range-check is needed after a machine op
    /// (a `FULL` width, `N == slot_bits`, is enforced entirely by the machine op's carry/borrow).
    fn narrow(self) -> bool {
        self.width < self.slot_bits()
    }

    /// A constant in this machine's slot (an i32 or i64 const of the given signed value).
    fn konst(self, v: i64) -> Lir {
        if self.slot32 {
            Lir::ConstI32(v as i32)
        } else {
            Lir::ConstI64(v)
        }
    }

    fn add(self) -> Lir {
        if self.slot32 {
            Lir::I32Add
        } else {
            Lir::I64Add
        }
    }
    fn sub(self) -> Lir {
        if self.slot32 {
            Lir::I32Sub
        } else {
            Lir::I64Sub
        }
    }
    fn mul(self) -> Lir {
        if self.slot32 {
            Lir::I32Mul
        } else {
            Lir::I64Mul
        }
    }
    fn and(self) -> Lir {
        if self.slot32 {
            Lir::I32And
        } else {
            Lir::I64And
        }
    }
    fn xor(self) -> Lir {
        if self.slot32 {
            Lir::I32Xor
        } else {
            Lir::I64Xor
        }
    }
    fn ne(self) -> Lir {
        if self.slot32 { Lir::I32Ne } else { Lir::I64Ne }
    }
    fn lt_s(self) -> Lir {
        if self.slot32 {
            Lir::I32LtS
        } else {
            Lir::I64LtS
        }
    }
    fn lt_u(self) -> Lir {
        if self.slot32 {
            Lir::I32LtU
        } else {
            Lir::I64LtU
        }
    }
    fn ge_u(self) -> Lir {
        if self.slot32 {
            Lir::I32GeU
        } else {
            Lir::I64GeU
        }
    }
    fn gt_u(self) -> Lir {
        if self.slot32 {
            Lir::I32GtU
        } else {
            Lir::I64GtU
        }
    }
    fn gt_s(self) -> Lir {
        if self.slot32 {
            Lir::I32GtS
        } else {
            Lir::I64GtS
        }
    }
    fn shl(self) -> Lir {
        if self.slot32 {
            Lir::I32Shl
        } else {
            Lir::I64Shl
        }
    }
    fn shr(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32ShrS,
            (true, false) => Lir::I32ShrU,
            (false, true) => Lir::I64ShrS,
            (false, false) => Lir::I64ShrU,
        }
    }
    /// An ARITHMETIC (sign-propagating) shift-right at this slot width, regardless of the type's own
    /// signedness — used by the signed div-by-2^k bias sequence, which needs both shift kinds explicitly.
    fn shr_s_forced(self) -> Lir {
        if self.slot32 {
            Lir::I32ShrS
        } else {
            Lir::I64ShrS
        }
    }
    /// A LOGICAL (zero-filling) shift-right at this slot width, regardless of the type's own signedness.
    fn shr_u_forced(self) -> Lir {
        if self.slot32 {
            Lir::I32ShrU
        } else {
            Lir::I64ShrU
        }
    }
    fn div(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32DivS,
            (true, false) => Lir::I32DivU,
            (false, true) => Lir::I64DivS,
            (false, false) => Lir::I64DivU,
        }
    }
    fn rem(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32RemS,
            (true, false) => Lir::I32RemU,
            (false, true) => Lir::I64RemS,
            (false, false) => Lir::I64RemU,
        }
    }

    /// The bitwise op for `&`/`|`/`^` at this machine width.
    fn bitwise(self, op: Prim) -> Lir {
        match (self.slot32, op) {
            (true, Prim::BitAnd) => Lir::I32And,
            (true, Prim::BitOr) => Lir::I32Or,
            (true, _) => Lir::I32Xor,
            (false, Prim::BitAnd) => Lir::I64And,
            (false, Prim::BitOr) => Lir::I64Or,
            (false, _) => Lir::I64Xor,
        }
    }

    /// This width's inclusive bounds `[min_N, max_N]` as machine-slot values. A signed N holds
    /// `-(2^(N-1)) ..= 2^(N-1)-1`; an unsigned N holds `0 ..= 2^N-1`. At `N == slot_bits` (64 or 32) the
    /// bounds ARE the slot extremes, so the range-check is skipped (see `narrow`); this is only consulted
    /// when narrow, so `N < slot_bits ≤ 64` and every bound fits an i64. Computed via `u64` so the shift
    /// never overflows an `i64` (`2^63` as an intermediate would).
    fn bounds(self) -> (i64, i64) {
        if self.signed {
            let half = 1i64 << (self.width - 1); // width ≤ 63 here, so 2^(width-1) ≤ 2^62 fits i64
            (-half, half - 1)
        } else {
            let max = ((1u64 << self.width) - 1) as i64; // width ≤ 63 here, so 2^width - 1 ≤ 2^63 - 1
            (0, max)
        }
    }
}

/// Emit a CHECKED `+`/`-`/`*` that TRAPS when the true result leaves the N-bit type (the numeric-model
/// default). Two composed guards make it correct at ANY width, over scratch locals `$a=base`,
/// `$b=base+1`, `$r=base+2`:
///
///   <A> set$a ; <B> set$b ; get$a get$b <machine-op> set$r ; <M-overflow guard> ; <range-check> ; get$r
///
/// The machine op (`add`/`sub`/`mul` in the i32 or i64 slot) is bit-identical for signed and unsigned.
/// STEP 1, the M-OVERFLOW guard, traps when the true result does not fit the MACHINE slot — needed only
/// when the machine op can overflow it: `+`/`-` at a FULL width (`N == slot bits`), and `*` whenever a
/// full-width product can exceed the slot. After it, `$r` holds the EXACT result as a slot value. STEP 2,
/// the RANGE-CHECK, traps when `$r` fits the slot but not `[min_N, max_N]` — needed when `N` is NARROW.
/// This is what makes a narrow width (Int8's `100+100=200`, a UInt48 `*` past `2^48`) trap. Together they
/// trap iff the true result leaves the N-bit type. The per-op M-overflow tests, SIGNED (validated against
/// exact arithmetic in the seed compiler, mul over 172k random cases) — add `((r^a)&(r^b))<0`, sub
/// `((a^b)&(a^r))<0`, mul `a≠0 && r/a≠b` (`div_s` traps MIN/-1 itself) — and UNSIGNED (carry/borrow out of
/// the slot) — add `r <ᵤ a`, sub `a <ᵤ b`, mul `a≠0 && r/ᵤa≠b`.
///
/// LIVENESS / minimal locals: both operands recurse at `base+3` (NOT disjoint ranges) — operand A is
/// stored into `$a` before B's code runs, so A's scratch `[base+3..]` is DEAD during B and B safely
/// reuses it. The declared-locals count is therefore `max(A-scratch, B-scratch)+3`, not the sum — the
/// high-water mark in `high` captures exactly that.
/// Emit a binary op's OPERAND at the operation's width. A binary integer op's two operands must share
/// one machine slot (i32 for a ≤32-bit op, i64 otherwise) — wasm rejects a mixed `i32`/`i64` op. A
/// bare integer LITERAL is width-polymorphic (it defaults to Int64 = an i64 slot when typed on its
/// own), so a `(+ x 1)` / `(> x 50)` over a NARROW parameter `x` would otherwise push the literal as
/// an i64 beside `x`'s i32 and produce invalid wasm. Ground a bare-literal operand to the OP's width
/// `it` here (the width unification the per-node `type_of` does not thread back to the operand). A
/// non-literal operand carries its own machine width already and is emitted unchanged.
#[allow(clippy::too_many_arguments)]
fn emit_operand(
    db: &mut Db,
    id: StructId,
    it: IntTy,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    if let Core::ConstInt(v) = core_of(db, id) {
        let width = it.ground_width();
        if !v.fits_width(it.ground_signed(), width) {
            return Err(Reject::coded(
                Code::IntOutOfRange,
                "integer literal does not fit its width",
            ));
        }
        if width <= 32 {
            out.push(Lir::ConstI32(v.to_i32_bits(width)));
        } else {
            out.push(Lir::ConstI64(v.to_i64_bits()));
        }
        return Ok(());
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)?;
    // WIDTH NORMALIZATION for a CONTROL-FLOW / non-literal operand. `emit_operand` grounds a DIRECT
    // literal to the op width above; but an operand that is an `if`/`match`/`let` (or any node) whose
    // BRANCHES are bare deferred-width literals types as its own join — which defaults to Int64 (an i64
    // slot) — while the enclosing op emits at a NARROW width (an i32 slot). That pushes an i64 into an
    // i32 op and wasm rejects the module (`expected i32, found i64`). Reconcile HERE, at the consuming
    // site: when the operand's emitted machine slot is WIDER than the op's, wrap it down (`i32.wrap_i64`).
    // SOUND: a genuine fixed-width Int64-vs-narrow disagreement is a type FAULT (CDZ0203) that aborts
    // before emit — so an i64 operand reaching a narrow op is necessarily a deferred literal defaulted to
    // i64, whose low bits ARE its value; the enclosing op's own range-check then traps a true overflow.
    // (The reverse — a narrow operand into a wider op — is likewise a fault, so it never reaches here; the
    // comparison path handles its own pair via `operand_int_ty`, and a direct literal is grounded above.)
    if matches!(type_of(db, id), Ty::Int(_)) {
        let op_slot = m_slot(it);
        let operand_slot = valtype_of(&type_of(db, id));
        if operand_slot == Some(ValType::I64) && op_slot == ValType::I32 {
            // Before truncating a control-flow operand's i64 value down to the narrow op width, REJECT a
            // constant branch VALUE that does not fit — `(+ (if c 1099511627776 2) 5) : Int8` must be a
            // CDZ0302 (as the bare `(: (if c 1099511627776 2) Int8)` is), NOT a silent `i32.wrap_i64`
            // truncation to `0`. The operand's branches were emitted at the `if`/`match` node's own
            // deferred→i64 width (nothing threads the narrow op width INTO the branches), so a constant
            // branch literal wider than the type slips through until this wrap. Walk the value-position
            // constants and range-check each at `it` — the same check `emit_operand` applies to a DIRECT
            // literal operand. (A runtime branch value is unconstrainable here and keeps the wrap; only a
            // compile-time-constant branch is judged, matching how the bare-if path grounds its literals.)
            reject_oversize_branch_constant(db, id, it)?;
            out.push(Lir::I32WrapI64);
        }
    }
    Ok(())
}

/// When a control-flow operand (`if`/`match`/`let`) is truncated to a NARROW op width, reject a
/// compile-time-constant branch VALUE that does not fit that width (CDZ0302) — so an out-of-range literal
/// buried in a conditional branch is caught rather than silently wrapped. Walks only VALUE positions that
/// carry the operand's result: an `if`'s two branches, a scalar `match`'s arm bodies, a `let`'s body; it
/// recurses through nested control flow. A `ConstInt` value that overflows `it` is the error; any
/// non-constant (a param, a call, an arithmetic node — whose own overflow the enclosing op's range-check
/// governs) is left alone. Conservative: it never rejects a value the language would accept.
fn reject_oversize_branch_constant(db: &mut Db, id: StructId, it: IntTy) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
            if !v.fits_width(it.ground_signed(), it.ground_width()) {
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            Ok(())
        }
        Core::If { then_, else_, .. } => {
            reject_oversize_branch_constant(db, then_, it)?;
            reject_oversize_branch_constant(db, else_, it)
        }
        Core::Match { arms, .. } => {
            for arm in arms {
                reject_oversize_branch_constant(db, arm.body, it)?;
            }
            Ok(())
        }
        Core::Let { body, .. } => reject_oversize_branch_constant(db, body, it),
        // Any other value (param, ref, call, arithmetic, …) is not a bare constant — leave it.
        _ => Ok(()),
    }
}

/// Emit a FLOAT operation's OPERAND at the operation's width `w` (32 or 64). The float analogue of
/// [`emit_operand`]: a bare float LITERAL is width-polymorphic (it defaults to Float64 = an f64 slot
/// when typed on its own), so `(+ x 1.0)` over a `Float32` `x` would otherwise push the literal as an
/// f64 beside `x`'s f32 and produce invalid wasm (`expected f32, found f64`). Materialize a bare-literal
/// operand (or the canonical NaN) DIRECTLY at the op width `w` — the width unification the per-node
/// `type_of` does not thread back to the operand. Any other operand emits normally, then a slot
/// DISAGREEMENT is reconciled by a demote/promote: an f64-slot operand into an f32 op demotes
/// (`f32.demote_f64`), an f32-slot operand into an f64 op promotes (`f64.promote_f32`). SOUND: a genuine
/// fixed-width Float32-vs-Float64 disagreement is a type FAULT (CDZ0301) that aborts before emit, so a
/// mismatched-slot operand reaching here is necessarily a bare deferred literal (its value is exact at
/// either width for the small constants a literal denotes; a demote is the same rounding the op width
/// would apply). This mirrors the integer normalization above and the `Float N.of` conversion arm.
#[allow(clippy::too_many_arguments)]
fn emit_float_operand(
    db: &mut Db,
    id: StructId,
    w: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A bare float literal / canonical NaN materializes at the OP width directly (no f64 detour).
    match core_of(db, id) {
        Core::ConstFloat(d) => {
            if w == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                out.push(Lir::F32ConstBits(bits));
            } else {
                out.push(Lir::F64ConstBits(d.to_f64_bits()));
            }
            return Ok(());
        }
        Core::ConstFloatNan => {
            if w == 32 {
                out.push(Lir::F32ConstBits(f32::NAN.to_bits()));
            } else {
                out.push(Lir::F64ConstBits(f64::NAN.to_bits()));
            }
            return Ok(());
        }
        _ => {}
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)?;
    // Reconcile a control-flow / non-literal operand whose emitted float slot differs from the op width.
    let operand_slot = valtype_of(&type_of(db, id));
    match (operand_slot, w) {
        (Some(ValType::F64), 32) => out.push(Lir::F32DemoteF64),
        (Some(ValType::F32), 64) => out.push(Lir::F64PromoteF32),
        _ => {}
    }
    Ok(())
}

/// Emit a float operand at width `w` and leave its CANONICAL INTEGER BIT PATTERN on the stack — the basis
/// of the canonical-byte float equality (`Core::FloatCompare`). Every NaN (any payload, any sign) folds to
/// ONE canonical bit pattern so `nan == nan` is true, while a zero's sign bit is preserved so `-0.0` and
/// `+0.0` have distinct patterns. Emits `select(x != x /*isnan*/, CANON_NAN_BITS, reinterpret_int(x))`:
/// the operand is `tee`d into a fresh float scratch slot so it can be read twice (once for the `x != x`
/// isnan test, once to reinterpret), then wasm `select` (`t1 t2 c → c ? t1 : t2`) picks the canonical NaN
/// bits when `x` is NaN, else `x`'s own bits. A constant operand is materialized at width first (via
/// `emit_float_operand`'s literal path). Width 32 uses i32/f32 ops + the binary32 canonical NaN
/// `0x7FC00000`; width 64 uses i64/f64 + `0x7FF8000000000000`.
#[allow(clippy::too_many_arguments)]
fn emit_canon_float_bits(
    db: &mut Db,
    id: StructId,
    w: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A CONSTANT float operand has no NaN ambiguity at runtime — fold its canonical bits directly. A
    // constant NaN uses the canonical quiet-NaN bits; a finite constant uses its own bits (sign-preserving).
    match core_of(db, id) {
        Core::ConstFloatNan => {
            if w == 32 {
                out.push(Lir::ConstI32(0x7FC0_0000u32 as i32));
            } else {
                out.push(Lir::ConstI64(0x7FF8_0000_0000_0000u64 as i64));
            }
            return Ok(());
        }
        Core::ConstFloat(d) => {
            if w == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                out.push(Lir::ConstI32(bits as i32));
            } else {
                out.push(Lir::ConstI64(d.to_f64_bits() as i64));
            }
            return Ok(());
        }
        _ => {}
    }
    // Materialize the runtime float at the op width, then tee into a fresh float scratch slot to read twice.
    emit_float_operand(db, id, w, slots, base, high, scratch_ty, layout, out)?;
    let slot = *high;
    *high = slot + 1;
    let (vt, reinterpret, ne, canon_nan) = if w == 32 {
        (
            ValType::F32,
            Lir::I32ReinterpretF32,
            Lir::F32Ne,
            Lir::ConstI32(0x7FC0_0000u32 as i32),
        )
    } else {
        (
            ValType::F64,
            Lir::I64ReinterpretF64,
            Lir::F64Ne,
            Lir::ConstI64(0x7FF8_0000_0000_0000u64 as i64),
        )
    };
    scratch_ty.insert(slot, vt);
    // CONSUME the materialized float into `slot` (set, not tee — leave nothing stray on the stack), then
    // rebuild the three `select` inputs from the slot.
    out.push(Lir::LocalSet(slot));
    // t1 = CANON_NAN_BITS (chosen when x is NaN)
    out.push(canon_nan);
    // t2 = reinterpret_int(x) — x's own bit pattern
    out.push(Lir::LocalGet(slot));
    out.push(reinterpret);
    // c = (x != x) → 1 iff x is NaN
    out.push(Lir::LocalGet(slot));
    out.push(Lir::LocalGet(slot));
    out.push(ne);
    // select: c ? CANON_NAN_BITS : reinterpret(x)
    out.push(Lir::Select);
    Ok(())
}

/// Emit an `if`/`match` branch (or arm) body producing the construct's RESULT type. Both branches must
/// leave the same machine slot on the stack (the block's result type), so a bare-literal branch — a
/// width-polymorphic `ConstInt` that defaults to Int64 — is GROUNDED to the result's integer width
/// (`emit_operand`), exactly as an operator operand is: else a default-Int64 literal branch opposite a
/// NARROW branch pushes a mismatched i64 into a narrow-i32 block and wasm rejects the function. A
/// non-literal branch, or a non-integer result, emits normally.
#[allow(clippy::too_many_arguments)]
fn emit_branch(
    db: &mut Db,
    id: StructId,
    result: &Ty,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    if let (Ty::Int(rit), Core::ConstInt(_)) = (result, core_of(db, id)) {
        return emit_operand(db, id, *rit, slots, base, high, scratch_ty, layout, out);
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)
}

/// The flow-sensitive range REFINEMENT a branch of `if <cond> …` establishes for a variable — the frame
/// [`crate::db::Db::push_range_refinements`] pushes while that branch is emitted. Dispatches on the
/// condition SHAPE, merging every bound the taken branch guarantees into the parent frame (`base`):
///   • a SIGNED var-vs-const comparison (`(< n 2)`, `(>= n 1)`, either operand order) → its one-sided
///     bound (negated in the `else` branch);
///   • `(and a b)` in the THEN branch → BOTH operands hold, so apply both (the range-check idiom
///     `(and (> n 0) (< n 100))` bounds `n` to `[1,99]`); in the else, De Morgan gives a disjunction — no
///     clean single-variable bound, so skip;
///   • `(or a b)` in the ELSE branch → `!(a or b) = !a and !b`, so apply BOTH operands negated; in the
///     then, skip;
///   • `(not a)` → refine `a` with the opposite polarity.
/// Nested `if`s accumulate (each frame merges the parent's). A shape this does not model contributes
/// NOTHING (returns `base`) — conservative, so no guard is ever wrongly elided. UNSIGNED comparisons and
/// `Eq`/`Ne` are skipped (no sound one-sided interval). The refinement is a narrowing the branch
/// GUARANTEES, so a guard the narrowed range proves dead is safe to drop.
fn refined_frame_for_branch(
    db: &mut Db,
    cond: StructId,
    then_branch: bool,
    base: crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)>,
) -> crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)> {
    match core_of(db, cond) {
        Core::Compare { op, lhs, rhs } => {
            refine_from_comparison(db, op, lhs, rhs, then_branch, base)
        }
        // `(and a b)` holds in the THEN branch iff BOTH hold — apply each. `(or a b)` fails in the ELSE
        // branch iff BOTH fail — apply each operand NEGATED (pass `then_branch=false` down). The other
        // polarity (an `and`'s else, an `or`'s then) is a disjunction of the operands' negations/holds,
        // which does not yield a single-variable interval — skip it (returns `base` unchanged).
        Core::And { lhs, rhs, is_and } => {
            let apply_both = (is_and && then_branch) || (!is_and && !then_branch);
            if !apply_both {
                return base;
            }
            // Each operand is itself a condition establishing its own bound in this branch's polarity: an
            // `and`'s THEN wants both operands HELD (then_branch=true), an `or`'s ELSE wants both operands
            // FAILED (then_branch=false). Recurse so a nested `(and …)`/comparison in either operand is
            // handled uniformly.
            let after_lhs = refined_frame_for_branch(db, lhs, is_and, base);
            refined_frame_for_branch(db, rhs, is_and, after_lhs)
        }
        // `(not a)` in this branch's polarity = `a` in the OPPOSITE polarity.
        Core::Not { operand } => refined_frame_for_branch(db, operand, !then_branch, base),
        _ => base,
    }
}

/// The compile-time-constant value a branch reduces to UNDER THE CURRENTLY-ACTIVE refinement frame, if
/// any — a `Core::ConstInt`/`ConstBool` directly, or a nested `Core::If` whose condition the active
/// refinement DECIDES (recurse into the taken branch, having pushed that branch's own refinement frame).
/// Returns the constant `Core`, or `None` when the branch is not a refinement-constant. This is the
/// emit-time analogue of `lower`'s const-fold: `lower` folds a branch that is constant WITHOUT flow facts,
/// but a branch like `(if (> x 5) 7 8)` becomes the constant `7` only under an active `x > 10` refinement
/// that `lower` never saw. Used to collapse an `if` whose two branches reduce to the SAME constant under
/// their respective refinements (`(if (> x 10) (if (> x 5) 7 8) 7)` → `7`). Bounded by the branch depth
/// (each recursion strips one decided `if`); pushes/pops the refinement frame around the recursion so the
/// nested fact is visible and never leaks. Only the ORDERING-decided `if` is chased — a non-decided inner
/// `if`, or any non-constant leaf, returns `None`.
fn refined_const_value(db: &mut Db, branch: StructId) -> Option<Core> {
    match core_of(db, branch) {
        c @ (Core::ConstInt(_) | Core::ConstBool(_)) => Some(c),
        Core::If { cond, then_, else_ } => {
            // The inner `if` reduces to a constant only if the active refinement DECIDES its condition.
            let Core::Compare { op, lhs, rhs } = core_of(db, cond) else {
                return None;
            };
            let taken = crate::lower::refined_comparison_const(db, op, lhs, rhs)?;
            let branch = if taken { then_ } else { else_ };
            // Descend with the taken branch's own refinement pushed (it may decide a further-nested `if`).
            let base_frame = db.current_refinements();
            let frame = refined_frame_for_branch(db, cond, taken, base_frame);
            db.push_range_refinements(frame);
            let r = refined_const_value(db, branch);
            db.pop_range_refinements();
            r
        }
        _ => None,
    }
}

/// Merge into `base` the one-sided bound a single SIGNED var-vs-const comparison `(op lhs rhs)` guarantees
/// in the given branch polarity — the atom [`refined_frame_for_branch`] composes for `and`/`or`/`not`.
/// `(op var C)` / `(op C var)` (flipped) → in the `then` branch `var op C` holds, in the `else` its
/// negation; the resulting `[lo, hi]` bound is intersected with any existing refinement for `var`. A
/// non-comparison, an unsigned variable, or `Eq`/`Ne` contributes nothing (returns `base`).
fn refine_from_comparison(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    then_branch: bool,
    base: crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)>,
) -> crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)> {
    let binder_of = |db: &mut Db, id: StructId| -> Option<StructId> {
        match core_of(db, id) {
            Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
            _ => None,
        }
    };
    let const_of = |db: &mut Db, id: StructId| -> Option<i64> {
        match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        }
    };
    // `(var, op-with-var-on-left, const)`. `(C op var)` flips to `var op.flip C`.
    let (var, cmp, c) = if let (Some(v), Some(k)) = (binder_of(db, lhs), const_of(db, rhs)) {
        (v, op, k)
    } else if let (Some(k), Some(v)) = (const_of(db, lhs), binder_of(db, rhs)) {
        let flipped = match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        };
        (v, flipped, k)
    } else {
        return base;
    };
    // EQUALITY guard: `(if (= x c) THEN ELSE)` — in the THEN branch `x == c`, so pin `x` to the EXACT
    // range `[c, c]` (the `if`-guard analogue of a match arm's exact-value refinement). This lets the body
    // fold a guard/comparison on `x` (`(if (= x 5) (+ x 1) …)` — the `+ 1` under `x == 5` cannot overflow;
    // a range-comparison on `x` decides). The ELSE branch gives only `x != c` — no interval — so skip it.
    // Sound for BOTH signednesses: equality does not depend on the order's wraparound. Intersects with any
    // existing frame bound for `x` (a `[c,c]` is the tightest, so it wins).
    if matches!(cmp, Prim::Eq) {
        if !then_branch {
            return base; // `x != c` — no single interval
        }
        // `c` came from `to_i64()`, so it is already an i64 — no clamp needed.
        let ec = c;
        let mut frame = base;
        // Pin `x` to the exact `[c, c]` — but only when `c` lies WITHIN any prior frame bound for `x`. If it
        // does not, the guard is unsatisfiable (a contradiction the branch never reaches), so leave the
        // prior frame rather than fabricate an inverted `[c,c]` a downstream consumer might misread.
        let (plo, phi) = frame
            .get(&var)
            .copied()
            .unwrap_or((i64::MIN, Some(i64::MAX)));
        if plo <= ec && phi.is_none_or(|h| ec <= h) {
            frame.insert(var, (ec, Some(ec)));
        }
        return frame;
    }
    // SIGNED integer variable only — an unsigned comparison's order wraps differently.
    let signed = matches!(type_of(db, var), Ty::Int(it) if it.ground_signed());
    if !signed {
        return base;
    }
    // The bound the taken branch establishes; the `else` branch negates the op.
    let effective = if then_branch {
        cmp
    } else {
        match cmp {
            Prim::Lt => Prim::Ge,
            Prim::Le => Prim::Gt,
            Prim::Gt => Prim::Le,
            Prim::Ge => Prim::Lt,
            other => other,
        }
    };
    let c = c as i128;
    let clamp = |x: i128| -> Option<i64> {
        if x > i64::MAX as i128 || x < i64::MIN as i128 {
            None
        } else {
            Some(x as i64)
        }
    };
    let (new_lo, new_hi): (Option<i64>, Option<i64>) = match effective {
        Prim::Lt => (None, clamp(c - 1)),
        Prim::Le => (None, clamp(c)),
        Prim::Gt => (clamp(c + 1), None),
        Prim::Ge => (clamp(c), None),
        _ => return base, // Eq/Ne/compare — no interval bound
    };
    let mut frame = base;
    let (mut lo, mut hi) = frame
        .get(&var)
        .copied()
        .unwrap_or((i64::MIN, Some(i64::MAX)));
    if let Some(nl) = new_lo {
        lo = lo.max(nl);
    }
    if let Some(nh) = new_hi {
        hi = Some(match hi {
            Some(h) => h.min(nh),
            None => nh,
        });
    }
    frame.insert(var, (lo, hi));
    frame
}

/// The refinement frame active inside a scalar `match` ARM whose literal `Int` probe matched: the
/// scrutinee EQUALS that literal, so pin its range to the exact `[c, c]`. Only when the scrutinee is a
/// `Param`/`LocalRef` (a binder to key on) and the probe is an `Int` — a computed scrutinee has no
/// binder, a `Bool`/`Wild`/`Str` probe pins no useful integer interval. Merges into `base` (nested
/// matches accumulate). `None` scrutinee-binder or non-`Int` probe → `base` unchanged. Exact-value
/// knowledge is the tightest refinement — a `(- n 1)` in the `(5 …)` arm computes `4`, its guard dead.
fn refined_frame_for_match_arm(
    db: &mut Db,
    scrutinee: StructId,
    probe: &crate::core::Probe,
    base: crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)>,
) -> crate::fxhash::FxHashMap<StructId, (i64, Option<i64>)> {
    let binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => binder,
        _ => return base,
    };
    // SIGNED integer scrutinee only (the range lattice reasons over signed intervals).
    if !matches!(type_of(db, scrutinee), Ty::Int(it) if it.ground_signed()) {
        return base;
    }
    let crate::core::Probe::Int(v) = probe else {
        return base;
    };
    let Some(c) = v.to_i64() else {
        return base;
    };
    let mut frame = base;
    // Intersect with any parent refinement (the exact point is the tightest, so it wins whenever it lies
    // within the parent range — and a match arm that reached here proves the scrutinee IS `c`).
    frame.insert(binder, (c, Some(c)));
    frame
}

/// Whether an `if`'s or 2-arm `match`'s BRANCH is a candidate for the branchless `select`: a SMALL,
/// TRAP-FREE scalar computation — from a one-instruction leaf (a param/kept `let`-local/constant) up
/// through a small trap-free op — OR a shallow NESTED CONDITIONAL whose parts are themselves convertible
/// (so a nested `if`/select folds into a nested `select` — the sign/clamp/3-way idiom
/// `(if (< x 0) -1 (if (> x 0) 1 0))`). A `select` evaluates BOTH arms unconditionally then picks, so an
/// arm is convertible iff every value it computes on the untaken path is SAFE to compute there — no trap,
/// no allocation, no effect — and the whole thing is CHEAP (a bounded subtree, so the wasted untaken work
/// never exceeds the branch it removes). Two shapes qualify (see [`select_arm_convertible`] for the
/// recursion):
///   (a) a TRAP-FREE scalar op (`is_trap_free`: bitwise/compare/not/wrap/proj/count/in-range shift/
///       const-divisor div-rem over trap-free operands, and every leaf — EXCLUDES checked `+`/`-`/`*`, a
///       runtime-count shift, a call, and any heap construct);
///   (b) a nested `Core::If` whose CONDITION is trap-free (safe to evaluate unconditionally) and whose two
///       arms are RECURSIVELY convertible — the inner `if` will itself select-convert when emitted.
/// The total node budget (`<= SELECT_ARM_MAX_SIZE`, or `SELECT_NESTED_MAX_SIZE` for a nested conditional)
/// bounds the unconditional work either way.
fn is_select_arm(db: &mut Db, id: StructId) -> bool {
    if !select_arm_convertible(db, id) {
        return false;
    }
    // A nested-conditional arm gets a larger node budget than a flat op: an inner `if` turns into an inner
    // `select`, which is still all-branchless cheap work, but the shape naturally spans more nodes (an
    // inner `if` + its compare + operands). A flat trap-free op keeps the tight leaf-idiom budget.
    let budget = if matches!(core_of(db, id), Core::If { .. }) {
        SELECT_NESTED_MAX_SIZE
    } else {
        SELECT_ARM_MAX_SIZE
    };
    subtree_size(db, id) <= budget
}

/// The convertibility recursion for [`is_select_arm`] (the size bound is applied by the caller; this only
/// checks the SHAPE). A node is convertible when it is a trap-free scalar op, or a nested `Core::If` with
/// a trap-free condition and two convertible arms. A nested conditional is sound to turn into a nested
/// `select` because: the condition is trap-free (safe to evaluate even on the untaken outer path), and
/// each arm — being convertible — is itself trap-free/allocation-free/effect-free all the way down, so
/// evaluating BOTH inner arms discards no owned cell and runs no side effect.
fn select_arm_convertible(db: &mut Db, id: StructId) -> bool {
    if let Core::If { cond, then_, else_ } = core_of(db, id) {
        return crate::lower::is_trap_free(db, cond)
            && select_arm_convertible(db, then_)
            && select_arm_convertible(db, else_);
    }
    // An ENUM-DISCRIMINANT sum constructor (`(Dir.North)`, a nullary variant of an all-nullary sum) emits
    // as JUST its discriminant constant (`i32.const disc` — see the `SumNew` emit's `node_is_enum_disc`
    // fast path): no `sum-new` box, no allocation, no drop. So it is trap-free, allocation-free, and
    // effect-free — a valid `select` arm. `is_trap_free` conservatively rejects every `SumNew` (heap
    // constructs are possibly-trapping in general), so admit the enum-disc case explicitly here. This lets
    // `(if c (Dir.North) (Dir.South))` — an `if` over two immediate discriminants — go branchless, just
    // like the scalar `(if c 0 1)` it compiles down to.
    if matches!(core_of(db, id), Core::SumNew { .. }) && node_is_enum_disc(db, id) {
        return true;
    }
    crate::lower::is_trap_free(db, id)
}

/// The node-count ceiling for a FLAT (non-nested) [`is_select_arm`]: a branch bigger than this is left as
/// an `if` so a `select` never duplicates a non-trivial computation onto the untaken path. Sized to admit
/// the common one-operator idioms — `(& x mask)`, `(| x bit)`, `(>> x k)`, `(not b)`, `(< a b)` (each an
/// op over two leaves = 3 nodes) — plus a shallow nest (a masked shift `(& (>> x k) m)` = 5), while
/// excluding a deep expression whose unconditional evaluation would cost more than the branch it replaces.
const SELECT_ARM_MAX_SIZE: u32 = 5;

/// The node-count ceiling for a NESTED-CONDITIONAL [`is_select_arm`] (an arm whose top node is a
/// `Core::If`): larger than the flat budget so a ONE-LEVEL nested conditional `(if (< x 0) -1 (if (> x 0)
/// 1 0))` — an inner `if` + a compare over two leaves + two constants = 8 nodes — folds to a nested
/// `select` (the sign/clamp/3-way idiom), while a deeper tree still stays a branch.
const SELECT_NESTED_MAX_SIZE: u32 = 9;

/// Emit the LOGICAL NEGATION of a boolean expression `id` (a Bool i32 → its `0`/`1` complement). When
/// `id` is a `Core::Compare`, the negation folds into the single COMPLEMENT comparison (`(not (< a b))`
/// → `a >=ₛ b`, `(not (= a b))` → `a ≠ b`) — the operands emit exactly as the `Core::Compare` arm does
/// (same width grounding + RHS-above-`*high` discipline), with the inverted op and NO trailing `i32.eqz`.
/// Any other bool emits then `i32.eqz`. Shared by `Core::Not` and the negated arm of the boolean
/// materialization, so a `(not CMP)` reached either directly or through the `(if c 0 1)` bool-int form
/// gets the same one-op complement (no `eqz ; eqz` double negation when the two folds compose).
#[allow(clippy::too_many_arguments)]
fn emit_negated_bool(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    if let Core::Compare { op, lhs, rhs } = core_of(db, id) {
        let it = operand_int_ty(db, lhs, rhs);
        emit_operand(db, lhs, it, slots, base, high, scratch_ty, layout, out)?;
        let rhs_base = base.max(*high);
        emit_operand(db, rhs, it, slots, rhs_base, high, scratch_ty, layout, out)?;
        out.push(compare_op_negated(op, it));
        return Ok(());
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)?;
    out.push(Lir::I32Eqz);
    Ok(())
}

/// BOOLEAN MATERIALIZATION: an `(if c 1 0)` / `(if c 0 1)` whose branches are the integer literals `1`
/// and `0` is just the condition itself, coerced to the result's integer width — no branch and no
/// `select`. A bool `c` already evaluates to exactly `0`/`1` in an i32 slot, so:
///   `(if c 1 0)` → `c`            (identity, then widen to the result slot);
///   `(if c 0 1)` → `!c`           (logical negation via `emit_negated_bool`, likewise `0`/`1`).
/// This attempts the emit and returns `Some(Ok(()))` when it fired, `None` when the shape does not match
/// (the caller falls through to the `select`/`if` lowering). Sound at every width: `c` is unconditionally
/// evaluated exactly as it was as the condition (so any trap in `c` still fires), and the branches carry
/// no traps of their own (bare literals). The result width comes from the node's solved type — a 64-bit
/// result zero-extends the i32 bool (`i64.extend_i32_u`); a ≤32-bit result already holds `0`/`1`.
#[allow(clippy::too_many_arguments)]
fn try_bool_materialization(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
    result: &Ty,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Option<Result<(), Reject>> {
    // The result must be an integer type (a `Bool` result already folded `(if c true false)`→`c` in
    // `lower`; this is the INTEGER-literal analogue that `lower` cannot see without width knowledge).
    let Ty::Int(it) = result else {
        return None;
    };
    let (t, e) = (core_of(db, then_), core_of(db, else_));
    // Read each branch's constant i64 value, if it is one.
    let as_int = |c: &Core| match c {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    let (tv, ev) = (as_int(&t)?, as_int(&e)?);
    // `(if c 1 0)` → c ; `(if c 0 1)` → !c. Any other constant pair is not a bool materialization.
    let negate = match (tv, ev) {
        (1, 0) => false,
        (0, 1) => true,
        _ => return None,
    };
    // Emit the condition (a bool → i32 `0`/`1`). The `0 1` form is the NEGATION, emitted via
    // `emit_negated_bool` so a `(if (not (= n 0)) 1 0)` — which `lower` branch-swaps to `(if (= n 0) 0 1)`
    // — folds the negation into the compare's complement (`n ≠ 0`) instead of stacking a second `i32.eqz`
    // atop the compare-with-zero `eqz` (the `eqz ; eqz` double negation).
    let emitted = if negate {
        emit_negated_bool(db, cond, slots, base, high, scratch_ty, layout, out)
    } else {
        emit(db, cond, slots, base, high, scratch_ty, layout, out)
    };
    if let Err(r) = emitted {
        return Some(Err(r));
    }
    // Widen the i32 `0`/`1` to a 64-bit result slot; a ≤32-bit result already holds it.
    if m_slot(*it) == ValType::I64 {
        out.push(Lir::I64ExtendI32U);
    }
    Some(Ok(()))
}

/// Whether `id` is safe to evaluate UNCONDITIONALLY as the right operand of a BRANCHLESS boolean
/// connective (`(and lhs rhs)` / `(or lhs rhs)` → `i32.and`/`i32.or`, no short-circuit `if`). The
/// short-circuit exists ONLY to skip a `rhs` that could TRAP or has an EFFECT when `lhs` already decides
/// the result; a `rhs` that can neither trap nor effect is identical evaluated always. This is broader
/// than `is_select_arm` (which also bounds COST for the `if`→`select` branch rewrite): a boolean `rhs`
/// is only ever a few instructions, so cost is not the concern — only trap/effect-freedom is. Accepts a
/// leaf, plus the TOTAL boolean-producing forms over recursively-safe operands: a comparison
/// (`i64.lt_s` etc. never trap), a bitwise `&`/`|`/`^` (total), a `not` (`i32.eqz`), and a `wrap`
/// (truncation, total). A checked `+`/`-`/`*`/`/`/`%`, a call, a heap op, or an effecting form is NOT
/// safe — it keeps the short-circuit `if`.
fn is_branchless_bool_rhs(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Param { .. } | Core::LocalRef { .. } | Core::ConstInt(_) | Core::ConstBool(_) => true,
        // A comparison never traps — safe if its operands are (they are always trap-free scalars, but
        // recurse for uniformity: a comparison operand is a leaf/arith, and only a trap-free one qualifies).
        Core::Compare { lhs, rhs, .. } | Core::FloatCompare { lhs, rhs, .. } => {
            is_branchless_bool_rhs(db, lhs) && is_branchless_bool_rhs(db, rhs)
        }
        // Bitwise `&`/`|`/`^` are total; `not` is `i32.eqz`; `wrap` truncates — all trap-free.
        Core::Arith {
            op: Prim::BitAnd | Prim::BitOr | Prim::BitXor,
            lhs,
            rhs,
        } => is_branchless_bool_rhs(db, lhs) && is_branchless_bool_rhs(db, rhs),
        Core::Not { operand }
        | Core::Convert {
            op: Prim::Wrap,
            operand,
        } => is_branchless_bool_rhs(db, operand),
        // A nested `and`/`or` whose OWN rhs is branchless-safe is itself safe (it emits branchlessly too).
        Core::And { lhs, rhs, .. } => {
            is_branchless_bool_rhs(db, lhs) && is_branchless_bool_rhs(db, rhs)
        }
        _ => false,
    }
}

/// How a checked-arith operand is pushed onto the stack at each of its use sites (the machine op AND
/// every guard re-read). An operand read many times need not be copied into a scratch local IF it is
/// cheap and side-effect-free to re-materialize:
///  - `Slot` — the operand already lives in a wasm local (a parameter, a kept `let`-binding, or a
///    scratch slot a non-reusable operand was stored into); push is `local.get`.
///  - `Const` — the operand is a compile-time integer; push is the grounded `i32.const`/`i64.const`
///    directly, so it needs neither a scratch slot nor a `local.set`.
///
/// Deciding the source ONCE (in [`operand_src`]) and pushing it at each site keeps the machine op and
/// the guard in agreement and removes the store+slot for a reusable operand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OperandSrc {
    Slot(u32),
    ConstI32(i32),
    ConstI64(i64),
}

impl OperandSrc {
    /// Push this operand's value onto the stack (`local.get slot`, or the constant push).
    fn push(self, out: &mut Emit) {
        match self {
            OperandSrc::Slot(slot) => out.push(Lir::LocalGet(slot)),
            OperandSrc::ConstI32(v) => out.push(Lir::ConstI32(v)),
            OperandSrc::ConstI64(v) => out.push(Lir::ConstI64(v)),
        }
    }

    /// The compile-time constant this operand carries (as i64), or `None` for a runtime slot. Both
    /// widths widen to i64 for the sign test the constant-operand overflow guard makes (the sign of the
    /// constant is all that guard needs — an i32 constant's sign is preserved by the i64 widening).
    fn const_value(self) -> Option<i64> {
        match self {
            OperandSrc::ConstI32(v) => Some(v as i64),
            OperandSrc::ConstI64(v) => Some(v),
            OperandSrc::Slot(_) => None,
        }
    }
}

/// The reusable operand source for `id` at machine slot type `slot_ty`, or `None` if the operand must
/// be computed and stashed in a scratch slot (a nested computation). A REUSABLE operand is one that is
/// side-effect-free and cheap to re-emit at every use site — so no scratch local and no `local.set`:
///  - a parameter (`Core::Param`) or kept `let`-binding (`Core::LocalRef`) already in a local of the
///    op's machine type (a narrow local feeding a wider op does NOT match — its i32 slot ≠ the i64 op);
///  - a compile-time integer (`Core::ConstInt`) that fits the op width, grounded to the op width `ot`
///    (the same range-check + bit-pattern `emit_operand` applies to an inline literal, so an
///    out-of-range constant still declines — CDZ0302 — rather than silently truncating).
fn operand_src(
    db: &mut Db,
    id: StructId,
    ot: IntTy,
    slots: &HashMap<StructId, u32>,
) -> Result<Option<OperandSrc>, Reject> {
    // A node MATERIALIZED into a slot (CSE / LICM / a match-scrutinee) is read back as a `local.get` of
    // THAT slot — an operand-source in its own right, no copy. Honor the node's own slot BEFORE the
    // core-kind dispatch: without this, a CSE-hoisted `Core::Arith` operand (`(+ (& x 7) (& x 7))`, both
    // uses reading the one CSE slot) fell to the copy path (`emit_operand_into` did `local.get src ;
    // local.set slot2`), spilling the already-slotted value into a fresh scratch slot for nothing. Reading
    // the CSE slot directly drops that copy (and its dead slot). Same slot-machine-type guard as the
    // Param/LocalRef arm — a slot of a different width takes the copy path (where `emit_operand` widens).
    if let Some(&slot) = slots.get(&id) {
        if valtype_of(&type_of(db, id)) == Some(m_slot(ot)) {
            return Ok(Some(OperandSrc::Slot(slot)));
        }
        return Ok(None);
    }
    match core_of(db, id) {
        Core::Param { binder } | Core::LocalRef { binder } => {
            let Some(&slot) = slots.get(&binder) else {
                return Ok(None);
            };
            // The operand must live in a slot of the op's machine type; else reading it would feed a
            // mismatched i32/i64 into the machine op. A same-width operand matches; a narrow operand
            // feeding a wider op does not and takes the copy path (where `emit_operand` widens it).
            if valtype_of(&type_of(db, id)) == Some(m_slot(ot)) {
                Ok(Some(OperandSrc::Slot(slot)))
            } else {
                Ok(None)
            }
        }
        Core::ConstInt(v) => {
            // A constant is re-materializable for free — inline it (grounded to the op width) at each
            // use, so it needs no scratch slot. Same range-check as `emit_operand`: out of range
            // declines, never truncates.
            let width = ot.ground_width();
            if !v.fits_width(ot.ground_signed(), width) {
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            let src = if width <= 32 {
                OperandSrc::ConstI32(v.to_i32_bits(width))
            } else {
                OperandSrc::ConstI64(v.to_i64_bits())
            };
            Ok(Some(src))
        }
        _ => Ok(None),
    }
}

/// The wasm machine slot (i32 for a ≤32-bit width, i64 otherwise) for an integer op of type `ot` —
/// the same choice [`Machine::slot`] makes, computed straight from the `IntTy` so `operand_src` need
/// not build a `Machine`.
fn m_slot(ot: IntTy) -> ValType {
    if ot.ground_width() <= 32 {
        ValType::I32
    } else {
        ValType::I64
    }
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core computation — the basis
/// for common-subexpression elimination. Two nodes are equal iff their core forms are the same operator
/// over recursively-equal operands, bottoming out at the same param/local slot or the same constant.
/// This is used ONLY to decide whether a repeated operand can be computed ONCE and read twice, so it is
/// deliberately CONSERVATIVE: any core kind not enumerated here (a call, a conditional, a heap
/// construct — whose equality would need more than structural matching, or whose sharing is not clearly
/// safe) compares UNEQUAL, so CSE simply does not fire. Every kind that DOES compare equal is a PURE,
/// deterministic scalar computation (arithmetic/comparison/conversion/projection over equal operands,
/// or a leaf) — so computing it once and reusing the value is observably identical to computing it
/// twice, INCLUDING its trap behavior (a trapping subexpression traps at the same first-occurrence
/// point whether shared or not). Effects would break this, but rcdzc has none yet.
fn core_eq(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true; // the SAME occurrence — trivially identical.
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        // A leaf reference: equal iff the SAME binder (same param/local slot → same value).
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
        // A pure binary op: same operator over recursively-equal operands. (Arithmetic and comparison
        // are the operators whose two runtime operands can be the shared subexpression.)
        (
            Core::Arith {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Arith {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        )
        | (
            Core::Compare {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Compare {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        ) => ox == oy && core_eq(db, lx, ly) && core_eq(db, rx, ry),
        // A pure float equality (`Core::FloatCompare`): same operator AND WIDTH over recursively-equal
        // operands. `is_cse_shareable` already admits a `FloatCompare` (it is a total canon-and-compare),
        // so `core_eq` MUST recognize two equal ones or the CSE could never fire for it — the sibling of
        // the `Compare` arm above, plus the `width` (an f32-eq and an f64-eq of the same operands are
        // DIFFERENT machine ops — `i32.eq` over canon f32 bits vs `i64.eq` over canon f64 bits — so a
        // width mismatch is not the same value).
        (
            Core::FloatCompare {
                op: ox,
                lhs: lx,
                rhs: rx,
                width: wx,
            },
            Core::FloatCompare {
                op: oy,
                lhs: ly,
                rhs: ry,
                width: wy,
            },
        ) => ox == oy && wx == wy && core_eq(db, lx, ly) && core_eq(db, rx, ry),
        // A pure conversion: same op over an equal operand.
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_eq(db, px, py),
        // A tuple projection: same index off an equal (runtime) operand.
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_eq(db, px, py),
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) BORROWING read
        // returning a SCALAR — pure, no rc change, deterministic — so two counts of an equal collection
        // yield the same value and share safely (the count analogue of `Proj`/`SumPayload`). This lets CSE
        // compute a repeated `(List.len xs)` — a `vec-len` runtime import — ONCE across `(+ (len xs) (* (len
        // xs) 3))`. Each takes ONE operand handle; equal iff those handles are `core_eq`.
        (Core::ListLen { operand: ox }, Core::ListLen { operand: oy })
        | (Core::BytesLen { operand: ox }, Core::BytesLen { operand: oy }) => core_eq(db, ox, oy),
        (Core::MapSize { map: mx }, Core::MapSize { map: my }) => core_eq(db, mx, my),
        (Core::SetLen { set: sx }, Core::SetLen { set: sy }) => core_eq(db, sx, sy),
        // A sum-variant payload read: equal iff the SAME path off an equal (runtime) scrutinee — the
        // pattern-binder analogue of `Proj`. `sum-payload`/`get-*` BORROW the handle and are pure (no rc
        // change, no effect), so two reads of the same payload yield the same value; sharing them lets the
        // arith-CSE compute `(Some x)`'s `x` ONCE for `(+ x x)` exactly as it already does for a repeated
        // tuple/record field `(+ (. r x) (. r x))`. `path` is a small `Vec<PathStep>` (each `Copy`), so
        // `==` is a cheap element compare.
        (
            Core::SumPayload {
                scrutinee: sx,
                path: px,
            },
            Core::SumPayload {
                scrutinee: sy,
                path: py,
            },
        ) => px == py && core_eq(db, sx, sy),
        // A `List.at`/`Bytes.at` indexed read: equal iff the SAME (list/bytes, index) off equal operands,
        // with the same Option discriminants. `vec-get`/`bytes-get` (behind a bounds check) BORROW the
        // sequence and are deterministic (no rc change, no effect), so two reads of the same element yield
        // the same `Option` value. Shared only as the scrutinee of a scalar-unwrapping `SumExpect` (an
        // `Option`-typed node is filtered from candidacy by the scalar gate); both operands `core_eq`.
        (
            Core::ListAt {
                list: lx,
                index: ix,
                disc_some: sx,
                disc_none: nx,
            },
            Core::ListAt {
                list: ly,
                index: iy,
                disc_some: sy,
                disc_none: ny,
            },
        ) => sx == sy && nx == ny && core_eq(db, lx, ly) && core_eq(db, ix, iy),
        (
            Core::BytesAt {
                bytes: bx,
                index: ix,
                disc_some: sx,
                disc_none: nx,
            },
            Core::BytesAt {
                bytes: by,
                index: iy,
                disc_some: sy,
                disc_none: ny,
            },
        ) => sx == sy && nx == ny && core_eq(db, bx, by) && core_eq(db, ix, iy),
        // An `Option.expect`/`Result.expect` (`SumExpect`) unwrap: equal iff the SAME present-discriminant
        // off an equal scrutinee. Borrowing + deterministic (present → the payload, absent → trap); two
        // identical unwraps yield the same value and trap identically. This is what makes a repeated
        // `(Option.expect (List.at xs i))` — scalar-valued — compute its bounds-check + `vec-get` + unbox
        // ONCE across `(+ (…at xs i) (…at xs i))`, the indexed-read analogue of the `List.len` CSE.
        (
            Core::SumExpect {
                scrutinee: sx,
                disc_present: dx,
            },
            Core::SumExpect {
                scrutinee: sy,
                disc_present: dy,
            },
        ) => dx == dy && core_eq(db, sx, sy),
        // A `Map.lookup`: equal iff the SAME map, key, and Option discriminants. It BORROWS the map and is
        // deterministic; two lookups of the same key yield the same `Option`. Shared only as the scrutinee
        // of a scalar-unwrapping `SumExpect` (an `Option`-typed node is filtered from candidacy by the
        // scalar gate). The `key_ty`/`val_ty` fields are derived from the operands (identical when `core_eq`)
        // and `Ty` is not `PartialEq`, so they are not compared.
        (
            Core::MapLookup {
                map: mx,
                key: kx,
                disc_some: sx,
                disc_none: nx,
                ..
            },
            Core::MapLookup {
                map: my,
                key: ky,
                disc_some: sy,
                disc_none: ny,
                ..
            },
        ) => sx == sy && nx == ny && core_eq(db, mx, my) && core_eq(db, kx, ky),
        // A boolean negation: equal iff the negated operands are. `not` is `i32.eqz` — pure and total.
        (Core::Not { operand: ox }, Core::Not { operand: oy }) => core_eq(db, ox, oy),
        // A conditional `select`/`if`: equal iff the condition AND both branches are recursively equal —
        // then the two `if`s compute the identical value, so the arith-CSE can compute the whole `if` ONCE
        // and read it twice (`(+ (if (< a b) a b) (if (< a b) a b))` = min(a,b) computed once). `core_eq`
        // returns true here ONLY when cond/then/else all match its PURE set (a leaf, arith, compare,
        // convert, proj, payload, not, or a nested pure `if`), so a branch with a call/effect never
        // qualifies — the shared `if` is pure and deterministic, safe to compute once. Both arms are
        // evaluated in neither the original nor the shared form (an `if` runs one branch), so no trap is
        // added or dropped by sharing.
        (
            Core::If {
                cond: cx,
                then_: tx,
                else_: ex,
            },
            Core::If {
                cond: cy,
                then_: ty,
                else_: ey,
            },
        ) => core_eq(db, cx, cy) && core_eq(db, tx, ty) && core_eq(db, ex, ey),
        _ => false,
    }
}

/// Where a checked op leaves its result:
///  - `Stack` — the usual case: the result is left on the operand stack (via `local.get $r`), for the
///    enclosing expression to consume.
///  - `Slot(d)` — the caller wants the result in local `d` and NOT on the stack. Used when this op is
///    an OPERAND of an enclosing checked op: the enclosing op would otherwise `emit_operand(this) ;
///    local.set d` — computing this result into its own `$r` then COPYING it to `d`. Passing `Slot(d)`
///    makes THIS op use `d` as its `$r` directly, so its final `local.set` IS the store and the copy
///    (`local.get $r_inner ; local.tee d`) vanishes, along with the separate `$r_inner` scratch slot.
#[derive(Clone, Copy)]
enum ResultDest {
    Stack,
    Slot(u32),
}

#[allow(clippy::too_many_arguments)]
fn emit_checked_arith(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    emit_checked_arith_to(
        db,
        op,
        m,
        lhs,
        rhs,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        ResultDest::Stack,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_checked_arith_to(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    dest: ResultDest,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // GUARD-ELIDED FAST PATH: when interval arithmetic proves the result stays in the type, NO overflow
    // guard and NO range-check follow — so each operand is used EXACTLY ONCE (only the machine op reads
    // it). There is then no reason to stash a non-reusable operand in a scratch slot for the guards to
    // re-read: emit both operands straight onto the wasm operand stack, run the machine op, and place the
    // result per `dest`. This skips both operand scratch slots AND the `$r` slot for the common
    // masked/refined-arith idiom (`(+ (& x 7) (& y 7))`, a loop counter step under a proving refinement).
    // `emit_operand` grounds a bare-literal operand to the op width `ot` (an out-of-range literal is still
    // rejected CDZ0302), exactly as the guarded path's `operand_src`/`emit_operand_into` do. B's transient
    // scratch (a nested computation) floats above `base` and never aliases A's already-pushed stack value.
    // Uses the SAME `arith_provably_in_range` predicate the guarded path below checks after the op — moved
    // up so the slot machinery is skipped entirely rather than claimed-then-unused.
    let result_ty = IntTy::fixed(m.signed, m.width);
    if crate::lower::arith_provably_in_range(db, op, lhs, rhs, result_ty) {
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        // B emits its own transient scratch above the running high-water — A is already on the stack, so B
        // never needs a slot A used; a fresh floor keeps B's width-disjoint scratch from re-typing a slot.
        let b_base = base.max(*high);
        emit_operand(db, rhs, ot, slots, b_base, high, scratch_ty, layout, out)?;
        out.push(match op {
            Prim::Add => m.add(),
            Prim::Sub => m.sub(),
            Prim::Mul => m.mul(),
            _ => return Err(Reject::decline("not a checked arithmetic op")),
        });
        match dest {
            ResultDest::Stack => {}
            ResultDest::Slot(d) => out.push(Lir::LocalSet(d)),
        }
        return Ok(());
    }
    // Each operand's SOURCE at every use site (the machine op + the guard's re-reads): a reusable
    // operand — a matching local, or a compile-time constant — is pushed directly (`local.get` / an
    // inline `const`) and needs NO scratch slot; only a nested computation is stashed in a fresh
    // scratch slot (source = that slot). `$r` (the result) always needs its own scratch. Scratch slots
    // are claimed from `base`; the operand recursion floats ABOVE whatever scratch is actually used, so
    // an operand that needs no copy also frees the slot it would have occupied.
    let mut next_scratch = base;
    let mut claim = |high: &mut u32| {
        let s = next_scratch;
        next_scratch += 1;
        if s + 1 > *high {
            *high = s + 1;
        }
        s
    };
    // A reusable source is settled now; a non-reusable operand claims a scratch slot to be stored into.
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // COMMON-SUBEXPRESSION ELIMINATION: if B is a non-reusable computation STRUCTURALLY IDENTICAL to A,
    // it produces the same value — so compute it ONCE (as A, into `$a`) and read `$a` for B too, rather
    // than emitting the whole computation (and its guards) a second time. `(+ (* a b) (* a b))` becomes
    // one `*` + one guard, read twice. Safe because `core_eq` only matches PURE deterministic scalar
    // computations (see its doc) — no effects, and a trapping operand traps identically. Only fires when
    // A itself was stashed in a slot (`sa` is a Slot): a reusable A (a bare local/const) is already free
    // to re-push, so B just shares that same source with no CSE needed.
    let sb_src = operand_src(db, rhs, ot, slots)?;
    // `sb_shares_a` records that B is the SAME computation as A and reuses A's slot (CSE) — so B is NOT
    // emitted separately below (it would recompute into A's slot). Distinct from `sb_src.is_some()` (a
    // reusable source that also skips the emit but for a different reason).
    let mut sb_shares_a = false;
    let sb = match sb_src {
        Some(src) => src,
        None if matches!(sa, OperandSrc::Slot(_)) && core_eq(db, lhs, rhs) => {
            trace!(target: "rcdzc::select", lhs = lhs.0, rhs = rhs.0, "CSE: identical operands share one computation");
            sb_shares_a = true;
            sa
        }
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // `$r` (the result slot): the caller-requested destination when this op is an operand of an
    // enclosing op (`Slot(d)`), else a fresh scratch slot. Using `d` directly means this op's final
    // `local.set` IS the store the enclosing op wanted — no copy. `d` is one of the enclosing op's
    // operand slots, claimed BELOW this op's `base`, so this op's own operand scratch (claimed from
    // `base` up) never collides with it.
    let sr = match dest {
        ResultDest::Slot(d) => d,
        ResultDest::Stack => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            s
        }
    };
    // Operands that DO need a copy recurse above the scratch slots claimed so far; A is stored before
    // B runs, so B may reuse A's operand scratch (the liveness the high-water mark captures).
    let operand_base = next_scratch;
    // <A> compute A into $a — only when A is a stashed (non-reusable) operand; a reusable source is
    // pushed in place at each use. `emit_operand_into` writes the result straight into `$a`: a nested
    // checked op targets `$a` as its own result slot (no copy), any other operand is `emit_operand`ed
    // then `local.set $a`. A bare-literal operand is grounded to the OP's width `ot` by `emit_operand`.
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // <B> compute B into $b — only for a stashed operand that is NOT shared with A (CSE). When
    // `sb_shares_a`, B's value already sits in A's slot (computed once), so re-emitting it would both
    // recompute and clobber — skip it.
    //
    // B emits ABOVE A's high-water (`b_base = max(operand_base, *high)`), not at the shared
    // `operand_base`. A's transient scratch is dead once A is stored in `$a`, so REUSING it would be
    // sound by liveness — BUT a slot A typed one way (an inlined heap-match materializes its scrutinee
    // as an i32 handle) and B reuses at another width (an i64 arith guard) re-types one wasm local to
    // two types → an invalid module (`expected i64, found i32`). Floating B above A's high-water hands B
    // fresh, never-typed slots — the same disjoint-slot discipline `emit_loop_iteration`/`emit_call_args`
    // apply to sibling arguments (a slot's TYPE is fixed for the whole function, so width-disjoint temps
    // must not alias even when their lifetimes don't overlap).
    let b_base = operand_base.max(*high);
    if sb_src.is_none()
        && !sb_shares_a
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db, rhs, ot, sb_slot, slots, b_base, high, scratch_ty, layout, out,
        )?;
    }
    // push$a push$b <machine-op> — the result is left on the operand stack.
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Add => m.add(),
        Prim::Sub => m.sub(),
        Prim::Mul => m.mul(),
        _ => return Err(Reject::decline("not a checked arithmetic op")),
    });
    // GUARD ELISION was already checked at the top of this fn (the `arith_provably_in_range` fast path):
    // when the result provably stays in the type, BOTH the machine overflow guard AND the narrow
    // range-check are dead, and — since no guard then re-reads the operands or the result — that path
    // emits the operands inline with NO scratch slots at all and returns before the slot machinery here.
    // So reaching THIS point means a guard follows and reads `$r` — store the machine result there first.
    out.push(Lir::LocalSet(sr));
    // Step 1: the machine-slot overflow guard (only where the machine op can overflow its slot). This is
    // the DEFINED outcome of the trapping default — an overflowing `+`/`-`/`*` traps rather than yielding
    // an undefined value; the guard is emitted (or provably elided) at EVERY reachable overflow, so no
    // integer op with undefined overflow behavior is ever emitted. This is the general partial-operation
    // discipline for arithmetic: an operation with no in-type result for its inputs (an overflowing add,
    // a `MIN/-1` divide) raises a trap of a defined kind here rather than producing an unspecified value —
    // the total-or-trap alternative to the fallible ops that instead return an `Option` (e.g. `List.at`):
    //= spec/capabilities/core-semantics.md#partial-operations-have-a-defined-outcome
    //# An operation that has no result for some inputs MUST, on those inputs, either evaluate to a value the executable semantics defines or raise a trap of a defined kind.
    //= spec/capabilities/core-semantics.md#partial-operations-have-a-defined-outcome
    //# An operation that has no result for some inputs MUST NOT produce an unspecified value.
    //= spec/capabilities/numeric-model.md#overflow-is-defined
    //# An integer operation that overflows its type MUST have a defined, deterministic outcome fixed by the numeric model, whether that outcome is a value or a trap.
    //= spec/capabilities/numeric-model.md#overflow-is-defined
    //# The compiler MUST NOT emit an integer operation whose overflow behavior is undefined.
    //= constitution.md#iii-the-compiler-introduces-no-undeclared-nondeterminism
    //# The compiler MUST emit each numeric operation with a fully specified result so that the operation does not vary between conforming runtimes.
    emit_machine_overflow_guard(op, m, sa, sb, sr, out);
    // Step 2: the narrow-width range-check on the exact result in `$r`. For a narrow signed `± const`
    // the exact result moves in ONE direction from an in-range operand, so only that bound is reachable
    // — drop the dead check. `(+ a C)` C>0 (or `(- a C)` C<0) moves UP → only `r > max`; the reverse
    // moves DOWN → only `r < min`. (`C==0` is elided in `lower`; a two-const op folds there too.) The
    // general/two-runtime case, and `*`, keep BOTH bounds (a product can leave either side).
    // A const `+`/`-` moves the exact result in ONE direction from an in-range operand, so a narrow
    // range-check needs only that bound (cycle 38). This does NOT hold for `*`: a narrow `(* a C)`
    // product can leave EITHER type bound (positive `a` overflows up, negative `a` down), so a const
    // multiplier keeps BOTH range bounds. Restricted to `Add`/`Sub` explicitly (`const_operand_split`
    // also matches `Mul` now — for the mul-guard fast path below — so the op check is load-bearing).
    let reach = match const_operand_split(op, sa, sb) {
        Some((_, c)) if c != 0 && matches!(op, Prim::Add | Prim::Sub) => {
            let moves_up = (matches!(op, Prim::Add) && c > 0) || (matches!(op, Prim::Sub) && c < 0);
            if moves_up {
                ReachableBounds::UpperOnly
            } else {
                ReachableBounds::LowerOnly
            }
        }
        _ => ReachableBounds::Both,
    };
    emit_range_check(m, sr, reach, out);
    // The result. `Stack` leaves it on the operand stack (`local.get $r`) for the enclosing expression;
    // `Slot(d)` means `$r` IS `d` and the caller wants the value only in the slot, so nothing is pushed
    // (the `local.set $r` above already stored it) — this is where the copy-into-the-operand-slot goes
    // away.
    if matches!(dest, ResultDest::Stack) {
        out.push(Lir::LocalGet(sr));
    }
    Ok(())
}

/// Emit operand `id` (at op width `ot`) so its value ends up in local `slot`. When `id` is itself a
/// nested checked `+`/`-`/`*`, it is emitted with `ResultDest::Slot(slot)` so its own result store
/// writes `slot` directly — no `emit_operand` + separate `local.set`, hence no `local.get $r_inner ;
/// local.tee slot` copy and no extra `$r_inner` scratch. Any other operand (a projection, a call, a
/// conversion, a shift/bitwise, a literal, …) is `emit_operand`ed onto the stack then `local.set slot`.
#[allow(clippy::too_many_arguments)]
fn emit_operand_into(
    db: &mut Db,
    id: StructId,
    ot: IntTy,
    slot: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A node MATERIALIZED into a slot (CSE / LICM / a match-scrutinee) reads back as a `local.get`, not a
    // recomputation — honor it BEFORE the nested-arith re-emit below (which would rebuild the checked op,
    // defeating the sharing). Read the slot, store into the destination. (The top-level `emit` has the same
    // fast path, but this operand-into-slot path bypasses `emit` for a nested checked op, so it needs its
    // own check.)
    if let Some(&src) = slots.get(&id) {
        out.push(Lir::LocalGet(src));
        out.push(Lir::LocalSet(slot));
        return Ok(());
    }
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
    {
        // WIDTH from the CONSUMING op when this nested arith has NO width anchor of its own. A nested
        // `+`/`-`/`*` whose operands are all deferred-width (bare literals, or `if`/`match` branches of
        // bare literals) types as `Int(Deferred)` — which `int_ty_of` would ground to the i64 DEFAULT,
        // storing an i64 result into the i32 slot the enclosing narrow op declared → INVALID WASM
        // (`(+ (+ (if c 1 2) (if d 3 4)) 5) : Int8`). It also computed the inner op at the WRONG width, so
        // its overflow range-check checked i64 not the narrow type. Emit it at the consuming width `ot`
        // instead: the inner op then computes AND range-checks at the right width, and a bare-literal
        // branch is grounded (and `fits_width`-checked) to `ot`, so an out-of-range branch literal is
        // REJECTED rather than silently truncated. SOUND: a genuine FIXED inner width differing from `ot`
        // is a CDZ0301 fault that aborts before emit, so a deferred-width inner arith reaching here has no
        // anchor and correctly takes its context's width. A fixed inner width (a real Int64 sub-result) is
        // kept as-is.
        let own = int_ty_of(db, id);
        let m = if own.width_is_fixed() {
            Machine::of(own)
        } else {
            Machine::of(ot)
        };
        // STRENGTH REDUCTION reaches the NESTED-operand path too: a `(* v 2^k)` that is an OPERAND of an
        // enclosing op (`(* (* x 2) 4)`) strength-reduces to `v << k` exactly as a top-level `* 2^k` does
        // (the `Core::Arith` emit arm). Without this, a nested constant-pow2 multiply fell straight to
        // `emit_checked_arith_to`, emitting the full `mul` + `div_s` round-trip guard the top-level path
        // avoids. The shift leaves its result on the stack; store it into the operand slot (mirrors the
        // fallback `emit_operand ; LocalSet` below).
        if matches!(op, Prim::Mul)
            && let Some((val, k)) = mul_pow2_shift(db, lhs, rhs, m)
        {
            // Write the shift result DIRECTLY into the operand slot (its own `$r == slot`) — no separate
            // `emit + local.set slot` copy, mirroring the nested checked `+`/`-`/`*` path below.
            emit_mul_pow2_as_shift(
                db,
                m,
                val,
                k,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                ResultDest::Slot(slot),
            )?;
            return Ok(());
        }
        return emit_checked_arith_to(
            db,
            op,
            m,
            lhs,
            rhs,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            ResultDest::Slot(slot),
        );
    }
    emit_operand(db, id, ot, slots, base, high, scratch_ty, layout, out)?;
    out.push(Lir::LocalSet(slot));
    Ok(())
}

/// For the constant-operand overflow fast path: return `(runtime_operand, C)` when `(op sa sb)` has a
/// compile-time constant operand and the OTHER is a runtime value that the specialized `r </ₛ> a` guard
/// tests against. For `Add` (commutative) EITHER side may be the constant — the other is `a`. For `Sub`
/// (`a - C`) ONLY the RIGHT operand `sb` may be the constant: a constant LEFT (`C - b`) is not the
/// `a ± C` shape the sign reasoning covers (it would need `-b`'s own overflow analysis), so it declines
/// to the general guard. `None` when neither operand (of the eligible side) is a constant.
fn const_operand_split(op: Prim, sa: OperandSrc, sb: OperandSrc) -> Option<(OperandSrc, i64)> {
    match op {
        // `+` and `*` are commutative — EITHER operand may be the constant; the other is the runtime `a`.
        Prim::Add | Prim::Mul => {
            if let Some(c) = sb.const_value() {
                Some((sa, c))
            } else {
                sa.const_value().map(|c| (sb, c))
            }
        }
        Prim::Sub => sb.const_value().map(|c| (sa, c)),
        _ => None,
    }
}

/// The machine-slot overflow guard for `(op a b)` with result in `$r` — traps (`if (empty) unreachable
/// end`) when the true result does not fit the MACHINE slot. For a NARROW `+`/`-` the machine add/sub
/// cannot overflow its slot (operands are far from the slot extremes), so the guard is skipped and the
/// range-check alone bounds the result; `*` always runs the `r/a≠b` guard (a narrow product can still
/// exceed the slot — e.g. two 48-bit values multiply past 2^64). See `emit_checked_arith`.
fn emit_machine_overflow_guard(
    op: Prim,
    m: Machine,
    sa: OperandSrc,
    sb: OperandSrc,
    sr: u32,
    out: &mut Emit,
) {
    // `+`/`-` overflow the slot only at a FULL width; a narrow add/sub stays within the slot.
    let addsub_can_overflow = !m.narrow();
    // CONSTANT-OPERAND FAST PATH (full-width signed `+`/`-`): when one operand is a compile-time
    // constant `C != 0`, the general two-`xor` sign test collapses to a SINGLE signed compare of the
    // result `r` against the RUNTIME operand `a`. A signed add/sub overflows iff the true result leaves
    // the type, and with a known-sign constant that shows up as `r` landing on the wrong side of `a`:
    //   (+ a C): C>0 overflows only upward → wrap makes `r <ₛ a`;  C<0 only downward → `r >ₛ a`.
    //   (- a C): C>0 subtracts, overflows only downward → `r >ₛ a`; C<0 → `r <ₛ a`.
    // (`C==0` never overflows and is already elided by the `lower` identity fold, so it never reaches
    // here; a two-constant op folds entirely in `lower`. `a` is the OTHER, runtime operand.) Reads `$r`
    // first so the preceding `local.set $r` fuses to `local.tee $r` via the peephole. ~5 fewer ops than
    // the general guard, on the hot path (loop counters `(- n 1)`, accumulators `(+ acc 1)`).
    if addsub_can_overflow
        && m.signed
        && matches!(op, Prim::Add | Prim::Sub)
        && let Some((a_src, c)) = const_operand_split(op, sa, sb)
        && c != 0
    {
        // `r < a` traps for: add with C>0, sub with C<0. `r > a` traps for: add with C<0, sub with C>0.
        let trap_when_r_lt_a =
            (matches!(op, Prim::Add) && c > 0) || (matches!(op, Prim::Sub) && c < 0);
        out.push(Lir::LocalGet(sr));
        a_src.push(out);
        out.push(if trap_when_r_lt_a { m.lt_s() } else { m.gt_s() });
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    // NEGATION FAST PATH (full-width signed `(- 0 a)`): the constant is on the LEFT (`0 - a`), which
    // `const_operand_split` does not cover (a left constant is not the `a ± C` sign shape). But negation
    // has exactly ONE overflow: `-a` leaves the type iff `a == MIN` (since `-MIN` is not representable).
    // So the guard is a single equality `a == MIN → trap` — 4 ops (`get a ; const MIN ; eq ; if`) vs the
    // general two-`xor` sub guard's 8, and it tests the OPERAND `a` directly (no dependence on `$r`).
    // Full-width only: a narrow `(- 0 a)` cannot overflow the SLOT (the machine guard is skipped,
    // `addsub_can_overflow` is false), and its type-bound escape (`0 - MIN_N = -MIN_N > MAX_N`) is caught
    // by the range-check, exactly as for any other narrow sub.
    if addsub_can_overflow && m.signed && matches!(op, Prim::Sub) && sa.const_value() == Some(0) {
        let min = if m.slot32 { i32::MIN as i64 } else { i64::MIN };
        sb.push(out); // the operand `a`
        out.push(m.konst(min));
        out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    // IDENTICAL-OPERAND FAST PATH (full-width signed `(+ a a)` — doubling): the general add guard is
    // `((r^a) & (r^b)) < 0`, but with `b == a` (the SAME operand source — CSE fuses `(+ a a)` to one
    // slot) that is `((r^a) & (r^a)) < 0` = `(r^a) < 0`. So one `xor` and one `and` drop: the guard is
    // `get $r ; push a ; xor ; const 0 ; lt_s` (`(r^a)<0`). Sound — `x & x = x` is an identity, verified
    // value-exact vs the general guard at every boundary. Constant operands never reach here (a two-const
    // add folds in `lower`), so equal sources are the same slot/param.
    if addsub_can_overflow && m.signed && matches!(op, Prim::Add) && sa == sb {
        out.push(Lir::LocalGet(sr));
        sa.push(out);
        out.push(m.xor());
        out.push(m.konst(0));
        out.push(m.lt_s());
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    match op {
        Prim::Add if addsub_can_overflow && m.signed => {
            // signed add: `((r^a) & (r^b)) < 0` → trap.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            out.push(Lir::LocalGet(sr));
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Add if addsub_can_overflow => {
            // unsigned add: `r <ᵤ a` → trap (the sum carried out of the slot).
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Sub if addsub_can_overflow && m.signed => {
            // signed sub: `((r^a) & (a^b)) < 0` → trap. Mathematically `((a^b) & (a^r)) < 0`, but `^`
            // and `&` are commutative, so we compute `(r^a)` FIRST — reading `$r` immediately after the
            // `local.set $r` that produced the result, so the peephole fuses that `set ; get` into a
            // `local.tee $r` (one fewer instruction). `(r^a)` ≡ `(a^r)`, `(r^a)&(a^b)` ≡ `(a^b)&(a^r)`.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            sa.push(out);
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Sub if addsub_can_overflow => {
            // unsigned sub: `a <ᵤ b` → trap (an unsigned value cannot go below 0). For a NARROW unsigned
            // width the machine sub CAN go negative in the slot (below 0), which the range-check then
            // catches — but the unsigned-underflow meaning is clearer as this direct test, and it holds
            // at full width where the range-check is a no-op. (A narrow signed/unsigned sub also relies on
            // the range-check for the upper edge, which never trips for sub.)
            sa.push(out);
            sb.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Mul => {
            // NARROW-PRODUCT-FITS-SLOT FAST PATH: when `2 * width <= slot bits`, the machine multiply in
            // the slot CANNOT overflow the slot — the largest magnitude product of two N-bit values needs
            // at most `2N` bits (`|a*b| < 2^(2N) <= 2^(slot bits)`). So the `div_s`/`div_u` round-trip
            // machine-overflow guard is entirely DEAD; the exact product sits in `$r` and the narrow
            // range-check (emitted after this guard) alone bounds it to `[min_N, max_N]`. Covers Int8/UInt8
            // (16 <= 32) and Int16/UInt16 (32 <= 32) in the i32 slot — a hardware DIVISION removed from
            // every such multiply. Int32×Int32 (64 > 32) and full-width still need the div check below.
            // (This is the mul analogue of the narrow `+`/`-` machine-guard skip: a narrow operand pair is
            // too small to overflow the slot; the range-check catches leaving the TYPE.)
            if m.narrow() && m.width * 2 <= m.slot_bits() {
                return;
            }
            // CONSTANT-MULTIPLIER FAST PATH (full-width signed `(* a C)`, `C` a compile-time constant).
            // The general guard runs a `div_s` (the slowest integer op) on EVERY multiply; but for a
            // known `C` the product `a*C` overflows iff `a` leaves the interval of `a`-values whose
            // product fits — a compile-time-constant interval, tested with TWO compares. `MAX/C` and
            // `MIN/C` truncate toward zero (Rust `/`), which is exactly the interval endpoints
            // (brute-verified at every boundary, both signs of C):
            //   C > 0: `aC` grows with `a` → fits iff `MIN/C <= a <= MAX/C`; trap iff `a > MAX/C || a < MIN/C`.
            //   C < 0: `aC` shrinks with `a` → fits iff `MAX/C <= a <= MIN/C`; trap iff `a < MAX/C || a > MIN/C`.
            // Eligible when `|C| >= 2` (`C ∈ {-1,0,1}` excluded: 0/1 fold in `lower`, and `C == -1` is the
            // negation whose `MIN/-1 = 2^63` bound is NOT i64-representable — `i64::MIN / -1` even panics —
            // so `-1` keeps the `div_s` guard) AND `C` is not a POSITIVE power of two (already
            // strength-reduced to a shift; a NEGATIVE power like `-2`/`-4` is not, so it IS eligible here).
            // Full-width only (the machine slot extremes ARE the type bounds); unsigned and narrow keep the
            // `div_s` round-trip below.
            if !m.narrow()
                && m.signed
                && let Some((a_src, c)) = const_operand_split(Prim::Mul, sa, sb)
                && c.unsigned_abs() >= 2
                && !(c > 0 && (c & (c - 1)) == 0)
            {
                let (slot_min, slot_max) = if m.slot32 {
                    (i32::MIN as i64, i32::MAX as i64)
                } else {
                    (i64::MIN, i64::MAX)
                };
                // The interval endpoints (both trunc-toward-zero); `a*C` fits iff `lo <= a <= hi`.
                // C>0: `aC` grows with `a` → [MIN/C, MAX/C]. C<0: `aC` shrinks → [MAX/C, MIN/C].
                let (lo, hi) = if c > 0 {
                    (slot_min / c, slot_max / c)
                } else {
                    (slot_max / c, slot_min / c)
                };
                // SINGLE unsigned range check (the classic `lo <= a <= hi` ⟺ `(a - lo) <=ᵤ (hi - lo)`
                // fold): shift the interval to start at 0 by subtracting `lo`, then ONE unsigned compare
                // decides both sides — `a < lo` wraps `a - lo` around to a huge unsigned value (> hi-lo),
                // `a > hi` overshoots `hi - lo` directly. So `trap ⟺ (a -ʷ lo) >ᵤ (hi - lo)`. This replaces
                // the two signed compares + two trap blocks (each re-reading `a`) with one subtract, one
                // unsigned compare, and ONE trap block. `hi - lo` fits the slot (the interval width is at
                // most the full slot span, and `c == +2`'s full-span case is excluded as a power of two),
                // and `a - lo` is a wrapping slot subtract (the wasm `i*.sub` is modular), so the unsigned
                // reading is exact. Brute-verified value-equal to the two-compare guard at every boundary,
                // both signs of C. (Reads `a` ONCE — no `local.tee`/CSE needed for the second read.)
                a_src.push(out);
                out.push(m.konst(lo));
                out.push(m.sub());
                out.push(m.konst(hi.wrapping_sub(lo)));
                out.push(m.gt_u());
                out.push(Lir::IfIntegerOverflowEnd);
                return;
            }
            // mul: `if a≠0 { if r/a ≠ b { unreachable } }` — guards div against a=0 (a=0 can't overflow);
            // the machine `div_s` traps on MIN/-1 itself (the sole case `r/a` can't detect at full width),
            // `div_u` is total for a≠0. Runs at every width — a narrow product can exceed the slot too.
            sa.push(out);
            out.push(m.konst(0));
            out.push(m.ne());
            out.push(Lir::If(BlockType::Empty)); // if a != 0 {
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.div());
            sb.push(out);
            out.push(m.ne());
            out.push(Lir::IfIntegerOverflowEnd); //   if (r/a) != b { unreachable }
            out.push(Lir::End); // }
        }
        _ => {}
    }
}

/// Which SIGNED narrow-range bounds a result can actually leave — a range-analysis hint that lets the
/// range-check drop a provably-unreachable side. `Both` is the safe default (a general op can land
/// anywhere). `UpperOnly`/`LowerOnly` are asserted only where the caller has PROVEN the result cannot
/// leave the other side (a narrow signed `± const`: the exact result moves in ONE direction from an
/// in-range operand, so it can exceed only that bound). Ignored for an unsigned width (already one test).
#[derive(Clone, Copy, PartialEq)]
enum ReachableBounds {
    Both,
    UpperOnly,
    LowerOnly,
}

/// The narrow-width range-check on an exact result in `$r`: trap unless `min_N <= r <= max_N`. A no-op
/// at a FULL width (`N == slot bits`, where the slot extremes ARE the bounds).
///
/// SIGNED width → two SIGNED guards: `r <ₛ min_N → trap` and `r >ₛ max_N → trap` (the bound and value
/// are signed slot values; the result sits sign-extended, so a value outside `[min_N, max_N]` is caught
/// on one side or the other). `reach` may PROVE only one side is possible (a narrow signed `± const`),
/// dropping the dead check — 4 instructions (`local.get`, `const`, compare, `if unreachable`).
///
/// UNSIGNED width → ONE UNSIGNED guard: `r >ᵤ max_N → trap`, i.e. `r >=ᵤ 2^N`. An unsigned narrow
/// result is `0 <= true value < 2^(slot bits)` and sits zero-extended, so the ONLY way it can leave the
/// type is by exceeding `2^N-1` — a single unsigned upper-bound test covers it. This is correct at EVERY
/// width, including one just below the slot size (a `UInt31` sum of `2^32-2` reads as a NEGATIVE signed
/// slot value, which the old signed `r <ₛ 0` guard caught and a signed `r >ₛ max` would MISS — the
/// unsigned compare catches it directly). (`reach` does not apply to unsigned — already one test.)
fn emit_range_check(m: Machine, sr: u32, reach: ReachableBounds, out: &mut Emit) {
    if !m.narrow() {
        return;
    }
    let (min_n, max_n) = m.bounds();
    if m.signed {
        // r <ₛ min_N → trap. Skipped when the result provably cannot fall below min (UpperOnly).
        if reach != ReachableBounds::UpperOnly {
            out.push(Lir::LocalGet(sr));
            out.push(m.konst(min_n));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        // r >ₛ max_N → trap. Skipped when the result provably cannot exceed max (LowerOnly).
        if reach != ReachableBounds::LowerOnly {
            out.push(Lir::LocalGet(sr));
            out.push(m.konst(max_n));
            out.push(m.gt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
    } else {
        // r >=ᵤ 2^N → trap (the single unsigned upper-bound test; `2^N = max_N + 1`).
        out.push(Lir::LocalGet(sr));
        out.push(m.konst(max_n.wrapping_add(1)));
        out.push(m.ge_u());
        out.push(Lir::IfIntegerOverflowEnd);
    }
}

/// Emit a runtime `/`/`%`. The machine `div`/`rem` traps natively on ÷0 (all widths) and, for a FULL
/// signed width, on `MIN/-1` — exactly two of the defined traps. Two extra guards make it correct at any
/// width: a NARROW signed `/` whose `min_N / -1` overflows the type is NOT trapped by the machine op (the
/// quotient `2^(N-1)` fits the wider slot), so it is caught by a range-check on the result; `%` never
/// overflows (its result is bounded by the divisor), so it needs no range-check. Over scratch locals
/// `$a`,`$b`,`$r` when a range-check is required; otherwise a bare `operands; op` suffices.
#[allow(clippy::too_many_arguments)]
fn emit_div_rem(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // STRENGTH REDUCTION: an UNSIGNED `/`/`%` by a constant POWER OF TWO becomes a shift/mask — far
    // cheaper than the hardware `div_u`/`rem_u`. `(/ a 2^k)` = `a >>ᵤ k`; `(% a 2^k)` = `a & (2^k - 1)`.
    // Only UNSIGNED: a signed `div_s`/`rem_s` rounds toward ZERO, which differs from an arithmetic shift
    // for negatives (`-1 / 2 = 0` but `-1 >>ₛ 1 = -1`), so a signed divide is left as-is. The constant
    // divisor is a nonzero power of two, so the ÷0 trap the machine op carries is provably not needed
    // (and `2^k - 1` for `%` is likewise exact). Applies at every width (the operand is already
    // range-valid; a shift/mask keeps it in range — an unsigned quotient/remainder only shrinks). `k=0`
    // (divisor 1) is excluded: `/1` is identity and `%1` is 0, both folded in `lower` before here.
    if !m.signed
        && let Core::ConstInt(v) = core_of(db, rhs)
        && let Some(d) = v.to_i64()
        && d > 1
        && (d & (d - 1)) == 0
    {
        let k = d.trailing_zeros() as i64;
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        if matches!(op, Prim::Div) {
            out.push(m.konst(k));
            out.push(m.shr()); // unsigned width → `shr_u`
        } else {
            out.push(m.konst(d - 1));
            out.push(m.and());
        }
        return Ok(());
    }
    // STRENGTH REDUCTION: a SIGNED `/`/`%` by a constant POWER OF TWO `2^k` also becomes shifts, but a
    // plain arithmetic shift rounds toward −∞ while `div_s`/`rem_s` truncate toward ZERO — they disagree
    // for negatives. The textbook fix (Hacker's Delight) BIASES the dividend by `2^k − 1` when it is
    // negative, so the truncation matches, and it is BRANCHLESS:
    //
    //   bias = (x >>ₛ (W−1)) >>ᵤ (W−k)      ; W = slot bits. `x >>ₛ (W−1)` = all-ones iff x<0, else 0;
    //                                        ; `>>ᵤ (W−k)` turns that into `2^k − 1` iff x<0, else 0.
    //   q    = (x + bias) >>ₛ k             ; arithmetic shift — now truncates toward zero, = `x / 2^k`.
    //   r    = x − (q << k)                 ; `% 2^k` from the reduced quotient (`= x − q·2^k`).
    //
    // The divisor is a positive power of two, so ÷0 never applies and `MIN/−1` (the only `div_s` overflow)
    // cannot arise — no trap, no range-check even when narrow (`|q| ≤ |x|` stays in the slot; a narrow
    // dividend is already sign-extended, so `>>ₛ (W−1)` reads its true sign). Verified value-exact vs
    // `div_s`/`rem_s` for every `k ∈ 1..=W−2` and all sign/boundary inputs (`k = W−1` would need divisor
    // `2^(W−1)`, unrepresentable as a positive slot constant, so it never reaches here). The value operand
    // is read three times, so it is stashed in a scratch local `$a` once.
    if m.signed
        && let Core::ConstInt(v) = core_of(db, rhs)
        && let Some(d) = v.to_i64()
        && d > 1
        && (d & (d - 1)) == 0
    {
        let k = d.trailing_zeros() as i64;
        // NON-NEGATIVE DIVIDEND fast path: the bias sequence exists ONLY to make an arithmetic shift
        // truncate toward zero for NEGATIVE dividends (`-1 / 2 = 0` but `-1 >>ₛ 1 = -1`). When the dividend
        // is provably `≥ 0` (a mask `(& x 255)`, an unsigned-typed value, or a flow-refined `x` under
        // `(> x 0)`), truncation toward zero equals floor equals a plain shift/mask — the whole bias is
        // DEAD. Emit `x >>ₛ k` (div) / `x & (2^k−1)` (rem), exactly the unsigned case, 1 op instead of 6.
        // Verified: for `x ≥ 0`, `x / 2^k == x >> k` and `x % 2^k == x & (2^k−1)` (toward-zero = floor).
        if crate::lower::value_provably_nonneg(db, lhs) {
            emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
            if matches!(op, Prim::Div) {
                out.push(m.konst(k));
                out.push(m.shr_s_forced()); // x ≥ 0 → arithmetic shift = floor = toward-zero quotient
            } else {
                out.push(m.konst(d - 1));
                out.push(m.and());
            }
            return Ok(());
        }
        let w = m.slot_bits() as i64;
        // The dividend scratch `$a` must be a slot of THIS op's machine width (i64 for Int64, i32 for a
        // narrow int). Reserve it ABOVE the running high-water, NOT at `base`: when this `%`/`/` is emitted
        // as a SUB-EXPRESSION whose enclosing context already typed `base` at a DIFFERENT width — e.g. the
        // bool element `(= (% s 2) 0)` of a compound-`=` tuple, where the synthesized compare-fn allocates
        // `base` as the i32 Bool slot — writing the i64 dividend into `base` re-types one wasm local to two
        // widths → `type mismatch: expected i32, found i64`, an invalid module (the tuple-`=` const-divisor
        // miscompile). A slot at `*high` is guaranteed never pre-typed. Mirrors the `ValueEq`/`SumExpect`
        // "float above `*high`" discipline for exactly this hazard.
        let sa = *high;
        *high = sa + 1;
        scratch_ty.insert(sa, m.slot());
        // `$a = x` (emit the dividend once; later reads are cheap `local.get`s). Its own transient scratch
        // floats above the reserved `sa`.
        emit_operand(db, lhs, ot, slots, *high, high, scratch_ty, layout, out)?;
        out.push(Lir::LocalSet(sa));
        // `q = (x + bias) >>ₛ k`, bias = `(x >>ₛ (W−1)) >>ᵤ (W−k)`.
        let emit_quotient = |out: &mut Emit| {
            out.push(Lir::LocalGet(sa)); // x
            out.push(Lir::LocalGet(sa)); // x  (for the sign replicate)
            out.push(m.konst(w - 1));
            out.push(m.shr_s_forced());
            out.push(m.konst(w - k));
            out.push(m.shr_u_forced());
            out.push(m.add()); // x + bias
            out.push(m.konst(k));
            out.push(m.shr_s_forced()); // >>ₛ k
        };
        if matches!(op, Prim::Div) {
            emit_quotient(out);
        } else {
            // `r = x − (q << k)`.
            out.push(Lir::LocalGet(sa)); // x
            emit_quotient(out);
            out.push(m.konst(k));
            out.push(m.shl()); // q << k
            out.push(m.sub()); // x − q·2^k
        }
        return Ok(());
    }
    // A narrow signed division needs a range-check on the quotient (its `min_N / -1` overflows the type
    // but not the slot). Every other case — `%` (bounded by the divisor), unsigned `/` (magnitude only
    // shrinks), full-width signed `/` (the machine `div_s` traps MIN/-1 itself) — is exact after the
    // native trap, so no scratch is needed. And the range-check is DEAD in two cases, since the sole
    // overflowing quotient is `MIN_N / -1`:
    //   • the DIVISOR provably is NOT `-1` — a constant `≠ -1`, or a range excluding -1 (`(/ x:Int8 3)`,
    //     `(/ x (& y 7))`); or
    //   • the DIVIDEND is provably NON-NEGATIVE — `MIN_N` is negative, so a nonneg dividend can never be
    //     it. For `a ≥ 0` and any `d ≠ 0`, `|a/d| ≤ a ≤ MAX_N`, so the quotient always fits the type
    //     (`(/ (& x 7) d)`, a loop counter, an unsigned-sourced value). (÷0 still native-traps via `div_s`.)
    let needs_range_check = matches!(op, Prim::Div)
        && m.signed
        && m.narrow()
        && crate::lower::divisor_can_be_neg_one(db, rhs)
        && !crate::lower::value_provably_nonneg(db, lhs);
    if !needs_range_check {
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
        out.push(if matches!(op, Prim::Div) {
            m.div()
        } else {
            m.rem()
        });
        return Ok(());
    }
    // Narrow signed `/`: compute into `$r`, then range-check. Reserve `$r` ABOVE `*high` (not at `base`):
    // as a compound-`=` element (or any sub-expression whose enclosing context typed `base` differently)
    // a `base`-anchored slot would re-type one wasm local to two widths → invalid module (see the signed
    // pow2 branch above — the tuple-`=` const-divisor hazard).
    let sr = *high;
    *high = sr + 1;
    scratch_ty.insert(sr, m.slot());
    let operand_base = *high;
    emit_operand(
        db,
        lhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    emit_operand(
        db,
        rhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    out.push(m.div()); // traps on ÷0 natively; the machine op does not overflow at a narrow width
    out.push(Lir::LocalSet(sr));
    // A narrow signed quotient can overflow the type ONLY upward: the sole out-of-type case is
    // `MIN_N / -1 = 2^(N-1) = MAX_N + 1` (above the max). It can never fall below `min`: `|q| = |a|/|b| <=
    // |a| <= 2^(N-1)`, so the most-negative reachable quotient is `-2^(N-1) = MIN_N` itself (in range,
    // e.g. `MIN_N / 1 = MIN_N`). So the `r < min` half of the range-check is provably dead — only the
    // upper bound is reachable (`UpperOnly`), dropping 4 instructions (get/const/lt_s/if).
    emit_range_check(m, sr, ReachableBounds::UpperOnly, out);
    out.push(Lir::LocalGet(sr));
    Ok(())
}

/// Emit a runtime `<<`/`>>` that GUARDS the shift count and (for `<<`) tests overflow, over scratch
/// locals `$a=base` (value), `$b=base+1` (count), `$r=base+2` (result). A wasm shift MASKS the count mod
/// the slot width and never traps, so a naive lowering would leak C-style undefined-shift behavior. The
/// numeric model forbids this: a count outside `[0, N)` has no defined value (trap), and a left shift is
/// exact multiplication by `2^count`, so it traps on overflow like `*`. The sequence:
///
///   <A> set$a ; <B> set$b
///   ; count guard: `b >=ᵤ N` → trap           (a negative count read unsigned is huge, so ≥ N too)
///   ; get$a get$b <machine-shift> set$r
///   ; <<-only: <M-overflow round-trip> ; <range-check>
///   ; get$r
///
/// The count is guarded against the LANGUAGE width `N` (not the slot width). `>>` never overflows, so it
/// has only the count guard; it is arithmetic (`shr_s`) for a signed type, logical (`shr_u`) for an
/// unsigned one. `<<`'s overflow has two parts: the round-trip `(r >>[s/u] b) != a` catches bits shifted
/// out of the SLOT, and the range-check catches a result that fits the slot but not the narrower N-bit
/// type — together the exact `2^count`-overflow at any width.
#[allow(clippy::too_many_arguments)]
/// For a `Mul` node, if EXACTLY ONE operand is a compile-time constant power of two `2^k` with
/// `1 <= k < width`, return `(value_operand, k)` — the runtime factor and the shift amount that replaces
/// the multiply. `None` otherwise (neither operand a power of two, both constant — folded in `lower` —
/// or `k` out of the useful range: `2^0 = 1` is the `*1` identity `lower` already elides, and `k >=
/// width` can't be represented as a valid shift). The power-of-two test is on the constant's fit-in-i64
/// magnitude: `v > 0 && v & (v-1) == 0`, with `k = v.trailing_zeros()`. Commutative — checks both sides.
fn mul_pow2_shift(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    m: Machine,
) -> Option<(StructId, u32)> {
    let pow2_k = |db: &mut Db, id: StructId| -> Option<u32> {
        match core_of(db, id) {
            Core::ConstInt(v) => {
                let n = v.to_i64()?;
                if n > 1 && (n as u64).is_power_of_two() {
                    let k = n.trailing_zeros();
                    // `k` must be a valid shift for this width (a `<< width` would trap as a bad count;
                    // such a multiplier only ever overflows anyway, but keep the shift well-formed).
                    if k < m.width {
                        return Some(k);
                    }
                }
                None
            }
            _ => None,
        }
    };
    // The OTHER operand must be the runtime value (not also a constant — that folds in `lower`).
    if let Some(k) = pow2_k(db, rhs)
        && !matches!(core_of(db, lhs), Core::ConstInt(_))
    {
        return Some((lhs, k));
    }
    if let Some(k) = pow2_k(db, lhs)
        && !matches!(core_of(db, rhs), Core::ConstInt(_))
    {
        return Some((rhs, k));
    }
    None
}

/// Emit `x * 2^k` as `x << k` with a COMPILE-TIME-CONSTANT count `k` — the strength-reduced multiply.
/// Same recipe as `emit_shift`'s `Shl` path (machine shift, then the overflow round-trip `(<r >> k) !=
/// x → trap`, then the narrow range-check) but the count is an inline constant, so there is NO count
/// operand and NO count guard (`k < width` by construction). The value operand `$a` is a reusable source
/// (a local/const pushed at each use) or stashed once in a scratch slot; `$r` holds the shift result.
#[allow(clippy::too_many_arguments)]
fn emit_mul_pow2_as_shift(
    db: &mut Db,
    m: Machine,
    val: StructId,
    k: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    dest: ResultDest,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    let mut next_scratch = base;
    let mut claim = |high: &mut u32| {
        let s = next_scratch;
        next_scratch += 1;
        if s + 1 > *high {
            *high = s + 1;
        }
        s
    };
    // The value operand `$a` is read three times (the shift, the round-trip check's compare); a reusable
    // source (matching local / constant) is pushed at each use, else it is stashed once in scratch.
    let sa_src = operand_src(db, val, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // `$r` (result slot): the caller-requested destination when this shift is an OPERAND of an enclosing
    // op (`Slot(d)`) — so this shift's `local.set` IS the store the enclosing op wanted, no `local.get $r ;
    // local.set d` copy and no extra `$r` scratch — else a fresh scratch slot. `d` is one of the enclosing
    // op's operand slots, claimed BELOW this op's `base`, so this op's own operand scratch never collides
    // with it. The round-trip guard re-reads `$r`, which is fine at either a scratch slot or `d`.
    let sr = match dest {
        ResultDest::Slot(d) => d,
        ResultDest::Stack => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            s
        }
    };
    let operand_base = next_scratch;
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            val,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // `$a << k` → `$r` (count is the inline constant `k`, no guard: `k < width`).
    sa.push(out);
    out.push(m.konst(k as i64));
    out.push(m.shl());
    // GUARD ELISION: when interval analysis proves `val << k` (= `val * 2^k`) stays in the type, both the
    // round-trip overflow check AND the narrow range-check are dead — the machine `shl` already produced
    // the exact result. `(* (& x 15) 2)` = `(& x 15) << 1` ∈ [0,30] fits Int64. With NO guard reading `$r`,
    // the `local.set $r` exists only to place the result: for `Stack` leave it on the stack (emit nothing);
    // for `Slot(d)` the single `local.set d` IS the store — mirrors `emit_checked_arith_to`'s elision.
    if crate::lower::shl_provably_in_range(db, val, k) {
        match dest {
            ResultDest::Stack => {}
            ResultDest::Slot(d) => out.push(Lir::LocalSet(d)),
        }
        return Ok(());
    }
    // A guard follows and re-reads `$r` — store the machine result there first.
    out.push(Lir::LocalSet(sr));
    // Overflow round-trip: `($r >> k)` must recover `$a`, else the shift dropped bits out of the slot.
    // The inverse shift matches signedness so the round-trip is exact (arithmetic for signed).
    out.push(Lir::LocalGet(sr));
    out.push(m.konst(k as i64));
    out.push(m.shr());
    sa.push(out);
    out.push(m.ne());
    out.push(Lir::IfIntegerOverflowEnd);
    // Range-check: a narrow `<<` result may fit the slot but exceed the N-bit type.
    emit_range_check(m, sr, ReachableBounds::Both, out);
    // Leave the result where the caller wants it: on the stack (`Stack`) or already in `$r == d` (`Slot`).
    match dest {
        ResultDest::Stack => out.push(Lir::LocalGet(sr)),
        ResultDest::Slot(_) => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_shift(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // The count read once here to fold a compile-time-constant count (see the count-guard below).
    let const_count = match core_of(db, rhs) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // An OUT-OF-RANGE constant count (`k >= N`, or negative — which reads unsigned as `>= N`) makes the
    // shift ALWAYS trap: emit a bare `unreachable` (one instruction) and nothing else — no operand
    // evaluation, no shift. `unreachable` is stack-polymorphic, so it satisfies the function's result
    // type. (A constant OOR count is a defined runtime trap for the shift's count, not a compile-time
    // reject — so it stays a trap, just emitted directly instead of as a dead comparison + `if`.)
    if let Some(k) = const_count
        && (k < 0 || k >= m.width as i64)
    {
        out.push(Lir::Unreachable);
        return Ok(());
    }
    // The value `$a` and the count `$b` are read several times (count guard, the shift, the round-trip
    // check), so — like `emit_checked_arith` — a reusable operand (a matching local or a constant) is
    // pushed directly at each use (no scratch), and only a nested computation is stashed in a scratch
    // slot. `$r` (the result) always needs its own scratch. Both share the op's machine slot, so a
    // bare-literal value/count is grounded to that width (a mixed i32/i64 shift is invalid wasm).
    let mut next_scratch = base;
    let mut claim = |high: &mut u32| {
        let s = next_scratch;
        next_scratch += 1;
        if s + 1 > *high {
            *high = s + 1;
        }
        s
    };
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    let sb_src = operand_src(db, rhs, ot, slots)?;
    let sb = match sb_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // `$r` (the result scratch) is needed ONLY by `<<`, which reads it back for the overflow round-trip
    // + range-check. `>>` leaves its exact result on the stack, so it claims no `$r` slot (no dead local).
    let sr = if matches!(op, Prim::Shl) {
        let s = claim(high);
        scratch_ty.insert(s, m.slot());
        s
    } else {
        0 // unused for `>>` — the result stays on the stack.
    };
    let operand_base = next_scratch;
    // Stash a non-reusable value/count into its scratch slot (a nested op writes it directly).
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    if sb_src.is_none()
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db,
            rhs,
            ot,
            sb_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // Count guard: `b >=ᵤ N` → trap. A negative count read unsigned is huge (≥ N), so this one test
    // catches both a negative and a too-large count. Bound is the LANGUAGE width N, not the slot width.
    // ELIDED for a VALID constant count (`0 <= k < N`, established above): the guard's condition is a
    // compile-time `false`, so it is dead (mirrors `lower`'s const-`if` fold). Also elided for a RUNTIME
    // count the value-range lattice proves is already in `[0, N-1]` — the common masked-count idiom
    // `(<< x (& k 63))` / `(>> x (& k 7))`, where `(& k M)` with `M < N` bounds the count to `[0, M]`, so
    // the `>=ᵤ N` test can never fire. `value_range_within(rhs, 0, N-1)` confirms both bounds (the lower
    // bound also rules out a negative count reading huge unsigned). Only a count of genuinely unknown range
    // keeps the runtime test. (An OOR constant count already returned a bare `unreachable` at the top.)
    let count_in_range =
        const_count.is_some() || crate::lower::value_range_within(db, rhs, 0, m.width as i64 - 1);
    if !count_in_range {
        sb.push(out);
        out.push(m.konst(m.width as i64));
        out.push(m.ge_u());
        out.push(Lir::IfUnreachableEnd);
    }
    // push$a push$b <machine-shift>. `>>` (`shr`) is EXACT — its result only shrinks in magnitude, so it
    // needs NO overflow round-trip and NO range-check (a right-shift of an in-range value stays in
    // range). So `>>` leaves the result directly on the stack: no `$r` store, no `$r` local — the `set
    // $r ; get $r` round-trip the old code emitted for BOTH shifts was pure dead motion for `>>`. Only
    // `<<` needs `$r`: it is read back for the overflow round-trip check and the narrow range-check.
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Shl => m.shl(),
        Prim::Shr => m.shr(),
        _ => return Err(Reject::decline("not a shift op")),
    });
    if matches!(op, Prim::Shl) {
        // GUARD ELISION: a `<<` whose result interval provably stays in the type needs neither the overflow
        // round-trip nor the range-check. For a CONSTANT count `(<< (& x 15) 2)` = [0,60] the fixed shift
        // amount drives `shl_provably_in_range`. For a RUNTIME count whose RANGE is known — the masked-count
        // idiom `(<< (& x 15) (& k 3))`, value [0,15] × count [0,7] → max 1920 — the dynamic variant bounds
        // the result by the count's max shift (`shl_provably_in_range_dynamic`).
        let elide = const_count.is_some_and(|k| {
            (0..m.width as i64).contains(&k)
                && crate::lower::shl_provably_in_range(db, lhs, k as u32)
        }) || (const_count.is_none()
            && crate::lower::shl_provably_in_range_dynamic(db, lhs, rhs));
        if elide {
            // The machine `shl` result is already on the stack (no round-trip needs `$r`) — nothing to do.
        } else {
            out.push(Lir::LocalSet(sr));
            // Round-trip: shifting `$r` back right by `$b` must recover `$a`; else the shift dropped bits
            // out of the SLOT (overflow). The inverse shift matches signedness so the round-trip is exact.
            out.push(Lir::LocalGet(sr));
            sb.push(out);
            out.push(m.shr());
            sa.push(out);
            out.push(m.ne());
            out.push(Lir::IfIntegerOverflowEnd);
            // Range-check: a narrow `<<` result may fit the slot but exceed the N-bit type.
            emit_range_check(m, sr, ReachableBounds::Both, out);
            out.push(Lir::LocalGet(sr));
        }
    }
    // `>>`: the result is already on the stack — nothing more to do.
    Ok(())
}

/// Emit a runtime `wrap` — TRUNCATE the operand (source machine `src`) to the target `dst`'s width and
/// signedness, keeping the low `dst.width` bits and reinterpreting them at the target sign. NEVER traps
/// (the whole point of `wrap`). Three composed steps, all width-generic:
///
///   1. emit the operand (it lands in the SOURCE slot, normalized to the source width);
///   2. MOVE it to the TARGET slot: `i32.wrap_i64` (i64→i32, drops the high half — which the mask would
///      drop anyway) or `i64.extend_i32_{s,u}` (i32→i64, extended by the SOURCE sign so the source value
///      is preserved before masking); same slot → nothing;
///   3. TRUNCATE to `dst.width` in the target slot when it is narrow (`dst.width < slot bits`): `and` the
///      low-`N`-bits mask, then — if the TARGET is signed — sign-extend from bit `N-1` via
///      `(x << (M-N)) >> (M-N)` (arithmetic shr). An unsigned target stops after the mask (zero-filled).
///
/// A full-width target (`dst.width == slot bits`) needs no truncation after the slot move — the slot IS
/// the width. The result is left normalized in the target slot, exactly as every other value.
#[allow(clippy::too_many_arguments)]
fn emit_wrap(
    db: &mut Db,
    src: Machine,
    dst: Machine,
    operand: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // 1. The operand, in the source slot.
    emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
    // 2. Move into the target slot (drop/extend the machine width). The extend is by the SOURCE sign so
    //    the source value's bits are preserved into the wider slot before the target mask.
    match (src.slot32, dst.slot32) {
        (false, true) => out.push(Lir::I32WrapI64), // i64 source → i32 target
        (true, false) => out.push(if src.signed {
            Lir::I64ExtendI32S
        } else {
            Lir::I64ExtendI32U
        }),
        _ => {} // same slot width — nothing to move
    }
    // 3. Truncate to the target width within the target slot, when narrower than the slot.
    //
    // REDUNDANT-TRUNCATION ELISION: the truncation is a no-op when the SOURCE value is already a valid,
    // identically-represented target value — i.e. the source width fits the target width AND they share
    // signedness. Then every source value lies in `[min_dst, max_dst]` and its normalized slot bits are
    // already the target's, so the mask (unsigned) or sign-extend (signed) changes nothing. This is the
    // `UInt8.wrap(UInt8)` identity and a same-sign widening like `UInt16.wrap(UInt8)`. A NARROWING
    // (`src.width > dst.width`) or a SIGN CHANGE (`Int8.wrap(UInt8)` — a `200` must become `-56` via
    // sign-extend) genuinely reshapes the value, so it keeps the truncation. (Signedness must match: even
    // at equal width, `Int8.wrap(UInt8)` reinterprets the top bit.)
    let truncation_is_identity = src.width <= dst.width && src.signed == dst.signed;
    // RANGE-BASED elision: even when the SOURCE TYPE is wider (or a different sign), the truncation is a
    // no-op if the operand's VALUE provably already lies in the target's `[min_N, max_N]` — then its low
    // N bits already encode it and the high slot bits are the correct sign extension, so masking/
    // sign-extending changes nothing. `UInt8.wrap(& x 255)` (operand ∈ [0,255], Int64-typed) and a wrap of
    // a flow-refined value shed the mask. Consults the same lattice as the guard-elision checks.
    //
    // `bounds()` is only defined for a NARROW width (`1u64 << 64` overflows), so it is consulted STRICTLY
    // behind the `dst.narrow()` guard — a full-width `wrap` (`UInt64.wrap`, `Int64.wrap`) never masks and
    // never queries the range.
    let operand_fits = dst.narrow() && {
        let (min_n, max_n) = dst.bounds();
        crate::lower::value_range_within(db, operand, min_n, max_n)
    };
    if dst.narrow() && !truncation_is_identity && !operand_fits {
        let slot_bits = dst.slot_bits();
        if dst.signed {
            // Sign-extend from bit N-1: `(x << (M-N)) >> (M-N)` with arithmetic (signed) shr. This both
            // masks (the << pushes the high bits out) and sign-fills. `dst.shr()` is arithmetic for a
            // signed dst.
            let shift = (slot_bits - dst.width) as i64;
            out.push(dst.konst(shift));
            out.push(dst.shl());
            out.push(dst.konst(shift));
            out.push(dst.shr());
        } else {
            // Zero-fill: mask to the low N bits.
            let mask = if dst.width >= 64 {
                -1i64 // all ones (unreachable for narrow, but total)
            } else {
                (1i64 << dst.width) - 1
            };
            out.push(dst.konst(mask));
            out.push(dst.and());
        }
    }
    Ok(())
}

/// For an equality comparison `(= a b)`, if EXACTLY ONE operand is a compile-time constant ZERO, return
/// the OTHER (non-zero) operand — the one to push before an `eqz`. `None` if neither operand is a
/// constant zero (a general equality → `eq`), or if BOTH are (a `0 == 0`, which `lower` already folds to
/// `true`, so it should not reach here — return `None` defensively so it takes the ordinary `eq` path
/// rather than a wrong single-operand `eqz`). The zero test is by VALUE (`IntValue::eq_value` against
/// zero), width-agnostic — a zero of any width is the additive identity the `eqz` recognizes.
/// If `id` is `(% x C)` with `C` a compile-time power of two `> 1`, return `(x, C-1)` — the dividend and
/// the low-bit mask, for the divisibility test `(= (% x 2^k) 0)` ⇔ `(= (x & (2^k−1)) 0)`. Sign-agnostic:
/// `x % 2^k == 0` iff `x`'s low `k` bits are all zero, whichever sign, so this fires for both signed and
/// unsigned `%`. `None` for any other operand (a non-power-of-two divisor, a constant dividend that
/// already folded, or a different op). `C == 1` never reaches here (`%1` folds to `0` in `lower`).
fn rem_pow2_mask(db: &mut Db, id: StructId) -> Option<(StructId, i64)> {
    let Core::Arith {
        op: Prim::Rem,
        lhs,
        rhs,
    } = core_of(db, id)
    else {
        return None;
    };
    let Core::ConstInt(v) = core_of(db, rhs) else {
        return None;
    };
    let d = v.to_i64()?;
    (d > 1 && (d & (d - 1)) == 0).then_some((lhs, d - 1))
}

fn eq_zero_operand(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<StructId> {
    let is_zero = |db: &mut Db, id: StructId| matches!(core_of(db, id), Core::ConstInt(v) if v.eq_value(&crate::ast::IntValue::zero()));
    let l0 = is_zero(db, lhs);
    let r0 = is_zero(db, rhs);
    match (l0, r0) {
        (true, false) => Some(rhs),
        (false, true) => Some(lhs),
        _ => None, // neither, or both (folded elsewhere) → ordinary `eq`.
    }
}

/// The flat wasm comparison op for a relational prim over an operand integer type — the width chooses
/// i32 (≤32-bit, or a boolean operand) vs i64, and the SIGNEDNESS chooses `_s` (a signed type orders by
/// two's-complement value) vs `_u` (an unsigned type orders by magnitude). Equality is sign-agnostic
/// (the same bits compare equal either way). A ≤32-bit value is properly sign-/zero-extended in its
/// slot, so the i32 `_s`/`_u` ops compare it correctly.
fn compare_op(op: Prim, it: IntTy) -> Lir {
    let narrow = it.ground_width() <= 32;
    let signed = it.ground_signed();
    match (op, narrow, signed) {
        (Prim::Eq, false, _) => Lir::I64Eq,
        (Prim::Lt, false, true) => Lir::I64LtS,
        (Prim::Gt, false, true) => Lir::I64GtS,
        (Prim::Le, false, true) => Lir::I64LeS,
        (Prim::Ge, false, true) => Lir::I64GeS,
        (Prim::Lt, false, false) => Lir::I64LtU,
        (Prim::Gt, false, false) => Lir::I64GtU,
        (Prim::Le, false, false) => Lir::I64LeU,
        (Prim::Ge, false, false) => Lir::I64GeU,
        (Prim::Eq, true, _) => Lir::I32Eq,
        (Prim::Lt, true, true) => Lir::I32LtS,
        (Prim::Gt, true, true) => Lir::I32GtS,
        (Prim::Le, true, true) => Lir::I32LeS,
        (Prim::Ge, true, true) => Lir::I32GeS,
        (Prim::Lt, true, false) => Lir::I32LtU,
        (Prim::Gt, true, false) => Lir::I32GtU,
        (Prim::Le, true, false) => Lir::I32LeU,
        (Prim::Ge, true, false) => Lir::I32GeU,
        // Not a comparison — `Core::Compare` only ever carries a comparison prim, so unreachable.
        _ => Lir::I64Eq,
    }
}

/// The machine op for the LOGICAL NEGATION of a comparison — used to fold `(not (CMP a b))` into a single
/// inverted comparison instead of `compare ; i32.eqz`. Every comparison over a TOTAL order has an exact
/// complement: `= ↔ ≠`, `< ↔ ≥`, `> ↔ ≤`. Integer order (signed and unsigned) and `Bool` order (a bool
/// is a total 0/1) are total, and these are the only operand types a `Core::Compare` carries (a compound
/// takes `Core::ValueEq`), so the complement holds for every case `compare_op` handles.
fn compare_op_negated(op: Prim, it: IntTy) -> Lir {
    let negated = match op {
        Prim::Eq => {
            return if it.ground_width() <= 32 {
                Lir::I32Ne
            } else {
                Lir::I64Ne
            };
        }
        Prim::Lt => Prim::Ge,
        Prim::Gt => Prim::Le,
        Prim::Le => Prim::Gt,
        Prim::Ge => Prim::Lt,
        // Not a comparison — unreachable, as in `compare_op`.
        _ => return Lir::I64Ne,
    };
    compare_op(negated, it)
}

/// The integer type governing a runtime comparison's operands — read off whichever operand solves to
/// an integer (they unify to one type). A boolean comparison has no integer operand, so it grounds to
/// the ≤32-bit path via the default `i64`… (a bool is compared as an i32 — see `Compare` selection,
/// which reads the operand's own `valtype`). Falls back to signed-64.
fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> IntTy {
    // A boolean operand is an i32; represent that as a signed ≤32-bit width so `compare_op` picks i32.
    let bool_as_i32 = IntTy::fixed(true, 32);
    let lt = type_of(db, lhs);
    let rt = type_of(db, rhs);
    // Both operands share ONE machine width. Prefer whichever carries a CONCRETELY-fixed integer width
    // (a narrow-typed variable `n : UInt8` pins the pair to i32) over a still-DEFERRED bare literal (whose
    // width defaults to i64). POSITION-INDEPENDENT: `(< 1 n)` and `(< n 1)` both pick `n`'s width. Reading
    // `lhs` first unconditionally emitted a deferred LEFT literal at its i64 default beside the i32
    // variable → a mismatched operand pair → INVALID WASM ("expected i64, found i32"). This is the emit-
    // side companion of the `unify_width`/`unify_sign` inference fix (which links an ARITH op's operands
    // through its shared result-width var); a COMPARISON's result is `Bool`, so its operand widths are not
    // carried on a result var and must be reconciled HERE from the operands' own types. A grounded literal
    // is then narrowed by `emit_operand` at the shared width, whichever side it is on.
    let concrete =
        |t: &Ty| matches!(t, Ty::Int(it) if matches!(it.width, crate::ty::Width::Fixed(_)));
    match (&lt, &rt) {
        (Ty::Int(it), _) if concrete(&lt) => *it,
        (_, Ty::Int(it)) if concrete(&rt) => *it,
        (Ty::Int(it), _) => *it,
        (_, Ty::Int(it)) => *it,
        (Ty::Bool, _) | (_, Ty::Bool) => bool_as_i32,
        // An ENUM-DISCRIMINANT operand is a bare discriminant i32 (like a bool), so its comparison is an
        // i32 op — the same signed-≤32 width bool uses. Lets `(= c C.Red)` emit `i32.eq` on the raw
        // discriminants rather than a `value-eq` heap walk (which would misread a discriminant as a
        // tagged handle). Reached only for an enum-disc `=` routed here by `lower`.
        _ if ty_is_enum_disc(db, &lt) || ty_is_enum_disc(db, &rt) => bool_as_i32,
        _ => IntTy::i64(),
    }
}

/// The integer type of the node at `id`, if its solved type is an integer — used to ground a literal's
/// width at selection. Defaults to the signed-64 instance when the node is not an integer (a
/// defensive fallback; a `ConstInt` node always types as an integer).
fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    match type_of(db, id) {
        Ty::Int(it) => it,
        _ => IntTy::i64(),
    }
}

/// The wasm machine op for a runtime FLOAT arithmetic prim at a given width — the f64/f32 `add`/`sub`/
/// `mul`/`div`. `width` is the operands' solved float width (32 → f32, else f64). IEEE, never trapping.
/// The raw IEEE float-ordering machine op for `Prim::FLt/FLe/FGt/FGe` at the given width. IEEE partialOrd:
/// a NaN operand → 0 (false), `-0.0`/`+0.0` compare equal. (Not for `FEq` — equality uses the canonical-
/// byte bit compare, a different relation.)
//= spec/capabilities/numeric-model.md#a-floating-point-relational-operator-follows-the-ieee-partial-order
//# A floating-point relational operator (`<`, `<=`, `>`, `>=`) MUST follow the IEEE-754 partial order over the operand type, so that a relational operator with a not-a-number operand yields false because a not-a-number value is unordered with respect to every value including itself.
//= spec/capabilities/numeric-model.md#a-floating-point-relational-operator-follows-the-ieee-partial-order
//# A negative zero and a positive zero MUST compare as neither less than nor greater than one another under a floating-point relational operator, so that the two zeroes are ordered as equal even though they are distinct under equality.
fn float_ordering_op(op: Prim, width: u32) -> Lir {
    let f32 = width == 32;
    match op {
        Prim::FLt if f32 => Lir::F32Lt,
        Prim::FLt => Lir::F64Lt,
        Prim::FLe if f32 => Lir::F32Le,
        Prim::FLe => Lir::F64Le,
        Prim::FGt if f32 => Lir::F32Gt,
        Prim::FGt => Lir::F64Gt,
        Prim::FGe if f32 => Lir::F32Ge,
        Prim::FGe => Lir::F64Ge,
        // Not a float-ordering prim — `Core::FloatCompare` only carries FEq (handled separately) or these.
        _ => unreachable!("float_ordering_op called with a non-ordering prim"),
    }
}

fn float_arith_op(op: Prim, width: u32) -> Lir {
    let f32 = width == 32;
    match op {
        Prim::FAdd => {
            if f32 {
                Lir::F32Add
            } else {
                Lir::F64Add
            }
        }
        Prim::FSub => {
            if f32 {
                Lir::F32Sub
            } else {
                Lir::F64Sub
            }
        }
        Prim::FMul => {
            if f32 {
                Lir::F32Mul
            } else {
                Lir::F64Mul
            }
        }
        Prim::FDiv => {
            if f32 {
                Lir::F32Div
            } else {
                Lir::F64Div
            }
        }
        // A non-float-arith prim never reaches here (guarded by `op.is_float_arith()` at the call site).
        _ => Lir::F64Add,
    }
}

/// The INTEGER type of each parameter of the def at index `callee` — `Some(it)` for an integer
/// parameter, `None` for a non-integer one. This lets a `Core::Call` GROUND a bare-literal integer
/// argument to its parameter's machine width via `emit_operand`: a narrow parameter (UInt8/Int8/…) is
/// an i32 slot, so a bare-literal argument that would otherwise default to i64 (`(f n 0)` — the `0` for
/// a UInt8 `acc`) must be emitted as i32, else the call pushes an i64 into an i32 param slot and the
/// module fails wasm validation. This is the narrow-normalization discipline (an operator operand / an
/// `if` branch already grounds via `emit_operand`) applied at the recursive/ordinary CALL boundary.
fn callee_param_int_tys(db: &mut Db, callee: usize) -> Vec<Option<IntTy>> {
    let Some(d) = db.defs.get(callee) else {
        return Vec::new();
    };
    let params = d.params.clone();
    params
        .into_iter()
        .map(|p| {
            // The name occurrence a reference binds to — bare `a` or the inner name of `(: a T)`.
            let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => p,
            };
            match type_of(db, binder) {
                Ty::Int(it) => Some(it),
                _ => None,
            }
        })
        .collect()
}

/// Emit a `Core::Call`'s arguments, GROUNDING each bare-literal integer argument to its parameter's
/// machine width (`emit_operand`), so a narrow (i32-slot) parameter never receives a default-i64 literal.
/// A non-integer parameter, or an argument past the known parameters, emits normally. Shared by the
/// tail (`return_call`) and non-tail (`call`) emit paths.
#[allow(clippy::too_many_arguments)]
fn emit_call_args(
    db: &mut Db,
    callee: usize,
    args: &[StructId],
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let param_its = callee_param_int_tys(db, callee);
    // Each arg after the first starts its scratch ABOVE the running high-water (`arg_base = *high`): the
    // args are all simultaneously live on the operand stack before the `call`, so a later arg reusing an
    // earlier arg's scratch slot at a different width (a heap-match handle's i32 slot over an arith
    // guard's i64 slot — `(g (- n 1) (match <heap-Option> …))`) would force one wasm local to two types
    // and fail validation. Advancing to `*high` hands each arg fresh, never-typed slots. Mirrors the same
    // discipline in `emit_loop_iteration` (the self-tail-loop back-edge).
    let mut arg_base = base;
    for (i, &arg) in args.iter().enumerate() {
        match param_its.get(i).copied().flatten() {
            Some(it) => emit_operand(db, arg, it, slots, arg_base, high, scratch_ty, layout, out)?,
            // A BigInt argument to a BigInt parameter (an i32 HANDLE) needs no special-casing here: a
            // CONSTANT-BigInt arg materializes to a handle in the `Core::ConstInt` emit arm (which routes
            // any BigInt-typed constant through `bigint-of-i64`), and a runtime BigInt arg is already a
            // handle. `emit` does the right thing for both — the fix is at that single choke point.
            None => emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?,
        }
        arg_base = *high;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::scalar_program;

    /// Compute the boundary layout for a test program (all test fixtures have an export). `select_*`
    /// needs it to resolve a `Core::Call` callee's function index; these Lir-level tests exercise no
    /// call, so the layout's contents beyond the exported def are irrelevant — but a real one is passed
    /// so the signature is honest.
    fn layout_of(db: &mut Db) -> Layout {
        crate::layout::compute(db).expect("layout")
    }

    #[test]
    fn selects_a_literal_to_i64_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let f = select_body(&mut db, body, &layout).expect("select");
        assert_eq!(f.code, vec![Lir::ConstI64(42)]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn selects_a_runtime_if_with_leaf_branches_to_a_branchless_select() {
        // A RUNTIME condition (a bool param `p`) with two CHEAP TRAP-FREE LEAF branches (constants) —
        // the `if` selects to wasm's BRANCHLESS `select`, not a structured block: push the two branch
        // values then the condition, then `select` (which pops `[then, else, cond]` and pushes `then`
        // if `cond` is nonzero). `local.get 0` is the condition `p`. This replaces the old
        // `if (result i64) … else … end` control block — one instruction, no branch. (A CONSTANT
        // condition folds away in `lower`; a NON-leaf/heap/effecting branch keeps the structured `if`,
        // covered by `keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf`.)
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool)) (if p 1 2)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::ConstI64(1),
                Lir::ConstI64(2),
                Lir::LocalGet(0),
                Lir::Select,
            ]
        );
    }

    #[test]
    fn an_if_between_two_enum_disc_variants_selects_branchlessly() {
        // `(if c (Dir.North) (Dir.South))` — the result type `Dir` is an ENUM-DISC sum (all variants
        // nullary), so its runtime rep is a plain i32 DISCRIMINANT and each variant emits as just its
        // discriminant constant (no `sum-new`, no allocation, no drop). So this is `(if c 0 1)` on the
        // disc, and it selects BRANCHLESSLY — `i32.const 0 ; i32.const 1 ; local.get 0 ; select` — even
        // though the result is nominally a "heap type" (the `is_heap_type` gate is relaxed for enum-disc).
        let ast = crate::testkit::parse(
            "(module m (type Dir (North) (South) (East) (West)) \
               (def (f (: c Bool)) (if c (Dir.North) (Dir.South))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::ConstI32(0), // Dir.North's discriminant
                Lir::ConstI32(1), // Dir.South's discriminant
                Lir::LocalGet(0), // the condition c
                Lir::Select,
            ],
            "an if between two enum-disc variants is a branchless select on the discriminant; got {:?}",
            f.code
        );
    }

    #[test]
    fn a_negated_if_condition_swaps_branches_and_drops_the_eqz() {
        // `(if (not c) a b)` ≡ `(if c b a)`: the negation is absorbed by swapping the branches — no
        // `i32.eqz`. It then selects branchlessly (leaf branches): `b ; a ; c ; select`, where the
        // branch operands are swapped (else-then `a`, then-first `b`) vs the un-negated `(if c a b)`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: c Bool) (: a Int64) (: b Int64)) (if (not c) a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(2), // b (the else branch, now first — swapped)
                Lir::LocalGet(1), // a (the then branch)
                Lir::LocalGet(0), // c (the un-negated condition)
                Lir::Select,
            ],
            "the negation is absorbed by the branch swap — no i32.eqz"
        );
        assert!(
            !f.code.contains(&Lir::I32Eqz),
            "the `not` must be gone (swapped into the branch order), got: {:?}",
            f.code
        );
        // A double negation `(if (not (not c)) a b)` cancels back to the un-swapped order `a ; b ; c`.
        let ast2 = crate::testkit::parse(
            "(module m (def (f (: c Bool) (: a Int64) (: b Int64)) (if (not (not c)) a b)) (def (main) 0) (export main))",
        );
        let mut db2 = Db::load(ast2);
        let layout2 = layout_of(&mut db2);
        let (params2, body2) = function_of(&mut db2, "f");
        let f2 = select_function(&mut db2, body2, &params2, &layout2).expect("select");
        assert_eq!(
            f2.code,
            vec![
                Lir::LocalGet(1),
                Lir::LocalGet(2),
                Lir::LocalGet(0),
                Lir::Select
            ],
            "double negation cancels — branches back in original order, no eqz"
        );
    }

    #[test]
    fn keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf() {
        // A branch that is NOT a cheap trap-free leaf (here `(+ a a)`, a checked add) must keep the
        // structured `if`/`else`/`end`: `select` evaluates BOTH branches unconditionally, so converting
        // a heavier/possibly-trapping branch would waste the work the `if` avoids (and could surface a
        // trap on the untaken side). So the wasm block survives with a real `if`. This pins the
        // eligibility gate `is_select_arm` alongside the positive case above.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool) (: a Int64)) (if p a (+ a a))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
            "a non-leaf branch keeps the structured if, got: {:?}",
            f.code
        );
        assert!(
            !f.code.contains(&Lir::Select),
            "a non-leaf branch must NOT use select, got: {:?}",
            f.code
        );
    }

    #[test]
    fn selects_a_runtime_if_with_small_trap_free_arms_to_a_branchless_select() {
        // A runtime `if` whose arms are NOT leaves but ARE small TRAP-FREE scalar ops — here `(& x 7)`
        // and `(| x 8)`, each a total bitwise op — converts to a branchless `select` (the widened
        // `is_select_arm` gate). Both arms + the condition are pushed, then `select`; no `if`/`else`/`end`
        // block. Sound because a bitwise op can neither trap nor allocate when evaluated on the untaken
        // path. Emitted arms: `(& x 7)` = get x ; const 7 ; and ; `(| x 8)` = get x ; const 8 ; or.
        let ast = crate::testkit::parse(
            "(module m (def (f (: x Int64)) (if (< x 0) (& x 7) (| x 8))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::Select),
            "small trap-free bitwise arms convert to a branchless select, got: {:?}",
            f.code
        );
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
            "the structured if/else block is gone (branchless), got: {:?}",
            f.code
        );
        // The bitwise ops themselves are present (both arms evaluated, then select picks).
        assert!(
            f.code.contains(&Lir::I64And) && f.code.contains(&Lir::I64Or),
            "both trap-free arms are emitted before the select, got: {:?}",
            f.code
        );
    }

    #[test]
    fn keeps_the_structured_if_when_a_trap_free_arm_exceeds_the_size_bound() {
        // A TRAP-FREE arm that is TOO BIG (`> SELECT_ARM_MAX_SIZE` nodes) keeps the structured `if`: a
        // `select` would compute the whole heavy arm on the untaken path, wasting more than the branch it
        // removes. Here the then-arm `(& (| (& (>> x 1) 3) 4) 7)` is a 4-deep bitwise nest (>5 nodes) —
        // trap-free but over the ceiling — so the branch survives. Pins the cost bound, not just the
        // trap-freedom gate.
        let ast = crate::testkit::parse(
            "(module m (def (f (: x Int64)) (if (< x 0) (& (| (& (>> x 1) 3) 4) 7) x)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
            "an over-size trap-free arm keeps the structured if, got: {:?}",
            f.code
        );
        assert!(
            !f.code.contains(&Lir::Select),
            "an over-size trap-free arm must NOT use select, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_nested_conditional_folds_to_nested_branchless_selects() {
        // The sign/clamp/3-way idiom `(if (< x 0) -1 (if (> x 0) 1 0))` — an `if` whose else arm is
        // itself a small conditional over trap-free (compare + constant) parts — folds to fully BRANCHLESS
        // code: no `if`/`else`/`end` block anywhere. The inner `(if (> x 0) 1 0)` is a bool materialization
        // (`x>0` extended) and the outer selects between `-1` and that. Sound: every condition is trap-free
        // (safe to evaluate unconditionally) and every arm is a constant, so evaluating both discards no
        // owned cell and runs no effect. Pins the nested-conditional widening of `is_select_arm`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
            "a nested conditional over trap-free parts is fully branchless, got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::Select),
            "the nested conditional uses select, got: {:?}",
            f.code
        );
        // A genuine 3-way nested select `(if (= x 0) 0 (if (< x 0) -1 1))` nests TWO selects (the inner
        // picks -1/1, the outer picks 0/inner) — still no branch.
        let ast2 = crate::testkit::parse(
            "(module m (def (f (: x Int64)) (if (= x 0) 0 (if (< x 0) -1 1))) (def (main) 0) (export main))",
        );
        let mut db2 = Db::load(ast2);
        let layout2 = layout_of(&mut db2);
        let (params2, body2) = function_of(&mut db2, "f");
        let f2 = select_function(&mut db2, body2, &params2, &layout2).expect("select");
        assert_eq!(
            f2.code.iter().filter(|i| matches!(i, Lir::Select)).count(),
            2,
            "a 3-way nested conditional nests two selects, got: {:?}",
            f2.code
        );
        assert!(
            !f2.code
                .iter()
                .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
            "the 3-way nested conditional is fully branchless, got: {:?}",
            f2.code
        );
    }

    #[test]
    fn a_nested_conditional_with_a_trapping_inner_arm_keeps_the_branch() {
        // A nested conditional whose inner arm is NOT trap-free — here `(* x 1000000000000)`, a checked
        // multiply that overflows for a large `x` — must keep the structured `if` and NOT become a nested
        // `select` (which would evaluate the would-overflow arm unconditionally, surfacing a trap on the
        // untaken path). The branch survives; the mul keeps its overflow guard. Pins the trap-freedom gate
        // on the nested-conditional recursion (`select_arm_convertible` descends into the inner arm).
        let ast = crate::testkit::parse(
            "(module m (def (f (: x Int64)) (if (< x 0) 0 (if (> x 100) (* x 1000000000000) x))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
            "a nested conditional with a trapping inner arm keeps the structured if, got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::I64Mul),
            "the checked multiply survives (guarded), got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_two_arm_match_with_leaf_bodies_selects() {
        // The match analogue of the `if`→`select` rewrite: a 2-arm scalar/bool match with a literal
        // probe + wildcard (or the two Bool literals) and cheap trap-free LEAF bodies emits a branchless
        // `select`, not an `if`/`else`. `(match n (0 a) (_ b))` → `a ; b ; (n eqz) ; select` (the 0-probe
        // uses `eqz`, cycle-43); `(match p (true a) (false b))` → `a ; b ; p ; select` (a Bool IS its own
        // condition — no `p == 1` compare). A NON-leaf body / a guard / >2 arms keeps the probe chain.
        let lir = |src: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        // (match n (0 a) (_ b)) → a ; b ; n ; eqz ; select.
        let zero = lir(
            "(module m (def (f (: n Int64) (: a Int64) (: b Int64)) (match n (0 a) (_ b))) (def (main) 0) (export main))",
        );
        assert_eq!(
            zero,
            vec![
                Lir::LocalGet(1), // a
                Lir::LocalGet(2), // b
                Lir::LocalGet(0), // n
                Lir::I64Eqz,      // n == 0
                Lir::Select,
            ],
            "a 2-arm 0-probe match selects with eqz"
        );
        // (match p (true a) (false b)) → a ; b ; p ; select — no `p == 1` compare (a Bool is the cond).
        let boolm = lir(
            "(module m (def (f (: p Bool) (: a Int64) (: b Int64)) (match p (true a) (false b))) (def (main) 0) (export main))",
        );
        assert_eq!(
            boolm,
            vec![
                Lir::LocalGet(1),
                Lir::LocalGet(2),
                Lir::LocalGet(0),
                Lir::Select,
            ],
            "a Bool 2-arm match selects on the bare condition"
        );
        // A body that is NOT trap-free (`(+ a 1)`, a checked add) keeps the structured if (no select) —
        // `select` would evaluate the untaken arm, possibly surfacing its overflow trap.
        let nonleaf = lir(
            "(module m (def (f (: n Int64) (: a Int64) (: b Int64)) (match n (0 (+ a 1)) (_ b))) (def (main) 0) (export main))",
        );
        assert!(
            !nonleaf.contains(&Lir::Select) && nonleaf.iter().any(|i| matches!(i, Lir::If(_))),
            "a possibly-trapping arm body keeps the if, got: {nonleaf:?}"
        );
    }

    #[test]
    fn a_two_arm_match_with_small_trap_free_op_bodies_selects() {
        // The match analogue of cycle-161/162's widened `if`→`select`: a 2-arm scalar/bool match whose
        // bodies are small TRAP-FREE ops (not bare leaves) — here `(& x 7)` / `(| x 8)` — emits a
        // branchless `select`, not a probe chain. Sound for the same reason as the `if` case: a bitwise op
        // can neither trap nor allocate on the untaken path. This unifies the match dispatch with the `if`
        // dispatch (both use `is_select_arm`).
        let lir = |src: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        // (match n (0 (& x 7)) (_ (| x 8))) → branchless select over the two bitwise arms.
        let ops = lir(
            "(module m (def (f (: n Int64) (: x Int64)) (match n (0 (& x 7)) (_ (| x 8)))) (def (main) 0) (export main))",
        );
        assert!(
            ops.contains(&Lir::Select) && !ops.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
            "small trap-free op arms select branchlessly, got: {ops:?}"
        );
        assert!(
            ops.contains(&Lir::I64And) && ops.contains(&Lir::I64Or),
            "both trap-free arms are emitted before the select, got: {ops:?}"
        );
        // A body binding the SCRUTINEE (`(match n (0 -1) (m (& m 255)))` — `m` binds `n`) selects too: the
        // binder reads the scrutinee's spill slot, which is materialized before the arm bodies emit.
        let bind = lir(
            "(module m (def (f (: n Int64)) (match n (0 -1) (m (& m 255)))) (def (main) 0) (export main))",
        );
        assert!(
            bind.contains(&Lir::Select) && !bind.iter().any(|i| matches!(i, Lir::If(_))),
            "a scrutinee-binding trap-free arm selects, got: {bind:?}"
        );
    }

    #[test]
    fn the_terminal_pair_of_a_sparse_match_chain_selects() {
        // A 3+-arm SPARSE scalar match (not dense enough for a br_table) emits a linear probe chain — but
        // its TERMINAL pair (the last literal-probe arm + the wildcard cover) is a 2-arm select shape, so
        // when both are trap-free `is_select_arm` bodies it emits a branchless `select` there instead of a
        // nested `if`/`else`. `(match x (0 10) (100 20) (_ 30))`: the outer `(0 10)` stays an `if` (its
        // else is the inner match sub-chain), but the `(100 20)/(_ 30)` tail → `20 ; 30 ; (x==100) ;
        // select`. So the chain has exactly ONE structured `if` (the outer 0-probe) and ONE `select`.
        let lir = |src: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let sparse = lir(
            "(module m (def (f (: x Int64)) (match x (0 10) (100 20) (_ 30))) (def (main) 0) (export main))",
        );
        assert_eq!(
            sparse.iter().filter(|i| matches!(i, Lir::Select)).count(),
            1,
            "the terminal pair selects, got: {sparse:?}"
        );
        assert_eq!(
            sparse.iter().filter(|i| matches!(i, Lir::If(_))).count(),
            1,
            "only the outer 0-probe stays a structured if, got: {sparse:?}"
        );
        // A 4-arm sparse chain: only the LAST pair selects; the two leading probes stay `if`s.
        let four = lir(
            "(module m (def (f (: x Int64)) (match x (0 1) (5 2) (9 3) (_ 4))) (def (main) 0) (export main))",
        );
        assert_eq!(
            four.iter().filter(|i| matches!(i, Lir::Select)).count(),
            1,
            "the 4-arm chain's terminal pair selects once, got: {four:?}"
        );
        // A terminal pair with a POSSIBLY-TRAPPING body (`(+ y 1)`, checked add) does NOT select — the
        // chain stays a nested `if` for that pair.
        let trapping = lir(
            "(module m (def (f (: x Int64) (: y Int64)) (match x (0 y) (7 (+ y 1)) (_ y))) (def (main) 0) (export main))",
        );
        assert!(
            trapping.iter().filter(|i| matches!(i, Lir::Select)).count() == 0,
            "a possibly-trapping terminal-pair body keeps the if, got: {trapping:?}"
        );
    }

    // ── runtime lowering: a parameterized function body selects to local reads + machine ops ──────
    //
    // These select a FUNCTION body standalone (as `select_function`, the path an exported function
    // takes) — the parameters are runtime values, so their references become `local.get` and the
    // operation is a runtime machine op, NOT folded. Asserted at the Lir level (no export/run yet).

    /// Locate def `name`'s parameter name-occurrences (seeing through `(: a T)`) and body, plus solve
    /// each param's type — the inputs `select_function` takes for an exported parameterized function.
    fn function_of(db: &mut Db, name: &str) -> (Vec<(StructId, Ty)>, StructId) {
        let d = db.def_by_name(name).expect("def present");
        let sig_params = db.defs[d].params.clone();
        let body = db.defs[d].body.expect("body");
        let mut params = Vec::new();
        for p in sig_params {
            // The name occurrence a reference binds to — bare `a` or the inner name of `(: a T)`.
            let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => p,
            };
            let ty = type_of(db, binder);
            params.push((binder, ty));
        }
        (params, body)
    }

    #[test]
    fn a_parameterized_addition_selects_to_a_checked_sequence() {
        // (def (add (: a Int64) (: b Int64)) (+ a b)) — the body is a RUNTIME add over two params, and
        // the numeric model requires it to TRAP on overflow, so it selects to the CHECKED sequence.
        // Both operands are ALREADY in locals (params, slots 0,1), so they are read DIRECTLY — no copy
        // into `$a`/`$b` scratch (see `operand_src`). Only the result needs scratch: $r = slot 2.
        // get0 get1 add set$r; signed-overflow guard `((r^a)&(r^b))<0 → if unreachable` reading the
        // params' own slots; get$r.
        let ast = crate::testkit::parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "add");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.params, vec![ValType::I64, ValType::I64]);
        assert_eq!(
            f.code,
            vec![
                // r = a + b — operands read straight from the param slots, no set$a/set$b copies.
                Lir::LocalGet(0),
                Lir::LocalGet(1),
                Lir::I64Add,
                // `local.set 2 ; local.get 2` (store $r, then the guard's first read of $r) is fused by
                // the `peephole` pass into a single `local.tee 2`.
                Lir::LocalTee(2),
                // overflow guard: ((r^a) & (r^b)) < 0 → trap, reading a=slot0, b=slot1 directly.
                Lir::LocalGet(0),
                Lir::I64Xor,
                Lir::LocalGet(2),
                Lir::LocalGet(1),
                Lir::I64Xor,
                Lir::I64And,
                Lir::ConstI64(0),
                Lir::I64LtS,
                Lir::IfIntegerOverflowEnd,
                // result
                Lir::LocalGet(2),
            ]
        );
        // One i64 scratch local declared ($r) — the operand copies ($a,$b) are eliminated.
        assert_eq!(f.declared, vec![ValType::I64; 1]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn a_constant_operand_is_inlined_not_stashed_in_scratch() {
        // (def (f (: a Int64)) (+ a 1)) — the RHS is a compile-time constant. `operand_src` returns a
        // `Const` source for it, so it is pushed inline (`i64.const 1`) at the add rather than stored
        // into a `$b` scratch local. Only $r needs scratch. And because a constant `+`/`-` operand at
        // full signed width lets the overflow guard SPECIALIZE, the guard is a single `r <ₛ a` compare
        // (C=1>0 for `+` overflows only upward → wrap makes `r < a`), NOT the general two-`xor` sign
        // test. Sequence: get$a const1 add tee$r ; get$a lt_s ; if unreachable ; get$r — the `set$r ;
        // get$r` pair fuses to `local.tee` via the peephole.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (+ a 1)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                // r = a + 1 — `a` from its param slot, `1` inline (no $b scratch).
                Lir::LocalGet(0),
                Lir::ConstI64(1),
                Lir::I64Add,
                Lir::LocalTee(1), // set $r then the guard's first read of $r, fused.
                // specialized guard: `r <ₛ a` → trap (a constant `+1` overflows only past MAX).
                Lir::LocalGet(0),
                Lir::I64LtS,
                Lir::IfIntegerOverflowEnd,
                Lir::LocalGet(1),
            ]
        );
        // Only $r (slot 1) is declared — the constant operand needs no scratch slot at all.
        assert_eq!(f.declared, vec![ValType::I64; 1]);
    }

    #[test]
    fn a_list_at_on_a_param_reads_the_param_slot_directly_no_handle_copy() {
        // (def (at (: xs (List Int64)) (: i Int64)) (List.at xs i)) — the list is a parameter, already
        // resident in slot 0 for the whole body. `vec-len` (bounds check) and `vec-get` (element read)
        // BORROW it, so both read slot 0 DIRECTLY — no copy into a scratch slot first (the heap analogue
        // of the scalar operand-slot reuse). So the body has NO `LocalSet(0)` (a param slot is never
        // stored to here), and every `vec-len`/`vec-get` is immediately preceded by `LocalGet(0)`.
        let ast = crate::testkit::parse(
            "(module m (def (at (: xs (List Int64)) (: i Int64)) (List.at xs i)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "at");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // The list handle is never copied into a scratch slot: no `local.set` targets the param slot 0,
        // and — since the reuse frees the would-be list scratch slot — no `local.set`/`tee` of the list
        // handle appears at all before the first `vec-len`. Assert both `vec-len` and `vec-get` read the
        // list param slot 0 directly.
        let vec_len_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
            .expect("a bounds-check vec-len");
        assert_eq!(
            f.code[vec_len_pos - 1],
            Lir::LocalGet(0),
            "the bounds-check vec-len reads the list param slot 0 directly; got {:?}",
            &f.code[..=vec_len_pos]
        );
        let vec_get_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_GET))
            .expect("an element vec-get");
        // vec-get takes the wrapped index on top, so the handle is one deeper: `LocalGet(0) ; LocalGet(idx)
        // ; I32WrapI64 ; vec-get`. Confirm slot 0 is pushed for the handle (three before the call).
        assert_eq!(
            f.code[vec_get_pos - 3],
            Lir::LocalGet(0),
            "the element vec-get reads the list param slot 0 directly; got {:?}",
            &f.code[vec_get_pos - 3..=vec_get_pos]
        );
        // No instruction stores the list handle into slot 0 (it is a param — read-only here).
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
            "the list param slot 0 is never written (no handle copy); got {:?}",
            f.code
        );

        // BYTES.AT shares the same reuse (bytes handle read by `bytes-len` + `bytes-get`): a param bytes
        // value in slot 0 is read directly, never copied into scratch.
        let ast = crate::testkit::parse(
            "(module m (def (at (: bs Bytes) (: i Int64)) (Bytes.at bs i)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "at");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        let blen_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_BYTES_LEN))
            .expect("a bounds-check bytes-len");
        assert_eq!(
            f.code[blen_pos - 1],
            Lir::LocalGet(0),
            "the bounds-check bytes-len reads the bytes param slot 0 directly; got {:?}",
            &f.code[..=blen_pos]
        );
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
            "the bytes param slot 0 is never written (no handle copy); got {:?}",
            f.code
        );
    }

    #[test]
    fn a_list_match_on_a_param_reads_the_scrutinee_slot_directly_no_handle_copy() {
        // (def (hd (: xs (List Int64))) (match xs ((list) 0) ((list h .. rest) h))) — the scrutinee is a
        // parameter, resident in slot 0. The match reads its handle for `vec-len` (length dispatch) and the
        // arm bodies' element reads (`vec-get`, BORROW) + rest read (`vec-drop`, `dup`-guarded); all read
        // slot 0 DIRECTLY — the handle is NOT copied into a scratch slot first (the c180 reuse, matching the
        // `MatchSum`/`List.at` discipline). So the FIRST `vec-len` reads `LocalGet(0)`, and slot 0 is never
        // written (a param — the reuse removes the would-be `local.set handle_slot` copy).
        let ast = crate::testkit::parse(
            "(module m (def (hd (: xs (List Int64))) (match xs ((list) 0) ((list h .. rest) h))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "hd");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // The length dispatch's `vec-len` reads the scrutinee param slot 0 directly (no prior copy).
        let vec_len_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
            .expect("a length-dispatch vec-len");
        assert_eq!(
            f.code[vec_len_pos - 1],
            Lir::LocalGet(0),
            "the list match's vec-len reads the scrutinee param slot 0 directly; got {:?}",
            &f.code[..=vec_len_pos]
        );
        // The scrutinee param slot 0 is never written — the handle-copy `local.set` is gone.
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
            "the scrutinee param slot 0 is never copied (no handle stash); got {:?}",
            f.code
        );
    }

    #[test]
    fn an_option_expect_on_a_param_reads_the_scrutinee_slot_directly_no_handle_copy() {
        // (def (unwrap (: o (Option Int64))) (Option.expect o "v")) — the scrutinee is a parameter, resident
        // in slot 0. `SumExpect` reads its handle TWICE — the disc probe (`sum-disc`) and the present-payload
        // read (`sum-payload`), both BORROWING — so both read slot 0 DIRECTLY, no copy into a scratch slot
        // (the c181 reuse, matching the `MatchSum`/`List.at`/`MatchList` discipline). So `sum-disc` reads
        // `LocalGet(0)`, and slot 0 is never written.
        let ast = crate::testkit::parse(
            "(module m (def (unwrap (: o (Option Int64))) (Option.expect o \"v\")) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "unwrap");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // The disc probe's `sum-disc` reads the scrutinee param slot 0 directly (no prior copy).
        let disc_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_DISC))
            .expect("a disc probe sum-disc");
        assert_eq!(
            f.code[disc_pos - 1],
            Lir::LocalGet(0),
            "the expect's sum-disc reads the scrutinee param slot 0 directly; got {:?}",
            &f.code[..=disc_pos]
        );
        // The present-payload `sum-payload` also reads slot 0 directly.
        let payload_pos = f
            .code
            .iter()
            .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
            .expect("a present-payload sum-payload");
        assert_eq!(
            f.code[payload_pos - 1],
            Lir::LocalGet(0),
            "the expect's sum-payload reads the scrutinee param slot 0 directly; got {:?}",
            &f.code[payload_pos - 1..=payload_pos]
        );
        // The scrutinee param slot 0 is never written — the handle-copy `local.set` is gone.
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
            "the scrutinee param slot 0 is never copied (no handle stash); got {:?}",
            f.code
        );
    }

    #[test]
    fn multiply_by_power_of_two_strength_reduces_to_a_shift() {
        // (def (f (: n Int64)) (* n 8)) — `* 2^k` becomes `<< k` (here k=3): push n, `shl 3` into $r,
        // then the overflow round-trip (`($r >> 3) != n → trap`) — no `i64.mul`, no division-based
        // guard, no count guard (k is the inline constant 3, always < width). Sequence:
        // get n ; const 3 ; shl ; tee $r ; get $r ; const 3 ; shr_s ; get n ; ne ; if unreachable ; get $r.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (* n 8)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(0),
                Lir::ConstI64(3),
                Lir::I64Shl,
                Lir::LocalTee(1), // set $r then the round-trip's first read of $r, fused.
                Lir::ConstI64(3),
                Lir::I64ShrS, // arithmetic shift (signed) for the exact round-trip.
                Lir::LocalGet(0),
                Lir::I64Ne,
                Lir::IfIntegerOverflowEnd,
                Lir::LocalGet(1),
            ]
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
            "the multiply is strength-reduced away, no i64.mul"
        );
    }

    #[test]
    fn a_provably_in_range_shift_elides_its_overflow_guard() {
        let select = |src: &str| {
            let mut db = Db::load(crate::testkit::parse(src));
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        // `(* (& x 15) 2)` → `(& x 15) << 1` ∈ [0,30], fits Int64 → NO round-trip guard (`shr ; ne`).
        let mul =
            select("(module m (def (f (: x Int64)) (* (& x 15) 2)) (def (main) 0) (export main))");
        assert!(
            !mul.iter().any(|i| matches!(i, Lir::I64Ne))
                && !mul.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
            "a provably-in-range `* 2^k` drops its shift-overflow guard; got {mul:?}"
        );
        // A user `(<< (& x 15) 2)` ∈ [0,60] likewise.
        let shl =
            select("(module m (def (f (: x Int64)) (<< (& x 15) 2)) (def (main) 0) (export main))");
        assert!(
            !shl.iter().any(|i| matches!(i, Lir::I64Ne)),
            "a provably-in-range `<<` drops its guard; got {shl:?}"
        );
        // SAFETY: a full-range `(<< x 2)` CAN overflow → keeps the round-trip guard.
        let open = select("(module m (def (f (: x Int64)) (<< x 2)) (def (main) 0) (export main))");
        assert!(
            open.iter().any(|i| matches!(i, Lir::I64Ne)),
            "a full-range `<<` keeps its guard; got {open:?}"
        );
        // SAFETY: `(<< (& x 15) 60)` = [0,15]<<60 overflows Int64 → keeps its guard.
        let over = select(
            "(module m (def (f (: x Int64)) (<< (& x 15) 60)) (def (main) 0) (export main))",
        );
        assert!(
            over.iter().any(|i| matches!(i, Lir::I64Ne)),
            "an over-range `<<` keeps its guard; got {over:?}"
        );
    }

    #[test]
    fn multiply_by_a_non_power_of_two_keeps_the_checked_multiply() {
        // (* n 3) — 3 is not a power of two, so the strength reduction to a shift does NOT fire: the
        // checked `i64.mul` stays. Its overflow guard, however, is the CONST-MULTIPLIER bound check
        // (`n` must lie in `[MIN/3, MAX/3]` for `n*3` to fit), NOT the general `div_s` round-trip — a
        // constant multiplier lets a compile-time-constant interval test replace the hardware divide.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (* n 3)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
            "a non-power-of-two multiply keeps i64.mul, got: {:?}",
            f.code
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64Shl)),
            "a non-power-of-two multiply does not become a shift"
        );
        // The const-multiplier overflow guard is a bound check, NOT a `div_s` round-trip.
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
            "a full-width const multiply's guard is a bound check, not div_s, got: {:?}",
            f.code
        );
    }

    #[test]
    fn const_multiply_guard_is_a_single_unsigned_range_check() {
        // (* n 3) — the const-multiplier overflow guard shifts the fitting interval `[MIN/3, MAX/3]` to
        // start at 0 (`n - MIN/3`) and does ONE unsigned compare `> (MAX/3 - MIN/3)`, so BOTH out-of-
        // interval directions are caught by a single test + a single trap block. It reads `n` ONCE and
        // uses NO signed compares (the old two-`gt_s`/`lt_s` + two-trap-block guard is gone). Parity with
        // the two-compare form is gate-verified at every interval boundary, both signs of C.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (* n 3)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // ONE unsigned compare, no signed compares in the guard.
        assert_eq!(
            f.code.iter().filter(|i| matches!(i, Lir::I64GtU)).count(),
            1,
            "the guard is a single unsigned range check, got: {:?}",
            f.code
        );
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::I64GtS | Lir::I64LtS)),
            "the unsigned range check replaces the two signed compares, got: {:?}",
            f.code
        );
        // The interval is shifted by `MIN/3` (the low endpoint) and the bound is its width `MAX/3-MIN/3`.
        let lo = i64::MIN / 3;
        assert!(
            f.code.contains(&Lir::ConstI64(lo)),
            "the guard subtracts the low endpoint MIN/3, got: {:?}",
            f.code
        );
        assert!(
            f.code
                .contains(&Lir::ConstI64((i64::MAX / 3).wrapping_sub(lo))),
            "the guard compares against the interval width MAX/3-MIN/3, got: {:?}",
            f.code
        );
        // Exactly ONE trap block (the two-block guard collapsed to one).
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::IfIntegerOverflowEnd))
                .count(),
            1,
            "the two trap blocks collapse to one, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_non_const_multiply_keeps_the_div_s_guard() {
        // Only a CONSTANT multiplier gets the bound check; a two-runtime-operand `(* a b)` keeps the
        // general `div_s` round-trip guard (`if a≠0 { r/a≠b → trap }`) — there is no compile-time bound
        // to compare against.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (* a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::I64DivS),
            "a runtime-operand multiply keeps the div_s guard, got: {:?}",
            f.code
        );
    }

    #[test]
    fn not_over_a_comparison_folds_to_the_complement_op() {
        // (def (f (: a Int64) (: b Int64)) (not (< a b))) — the negation folds into the complement
        // comparison `a >=ₛ b`: get a ; get b ; i64.ge_s — NO i32.eqz.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (not (< a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64GeS],
            "not(<) is the single complement ge_s, no eqz"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
            "the eqz is folded away into the complement comparison"
        );
    }

    #[test]
    fn not_over_equality_folds_to_ne() {
        // (not (= a b)) → i64.ne (not i64.eq ; i32.eqz).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (not (= a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.code, vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64Ne]);
    }

    #[test]
    fn not_over_an_unsigned_comparison_uses_the_unsigned_complement() {
        // (not (< a b)) over UInt64 → i64.ge_U (the unsigned complement, not the signed ge_s).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a UInt64) (: b UInt64)) (not (< a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::I64GeU) && !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
            "unsigned not(<) is ge_u, no eqz, got: {:?}",
            f.code
        );
    }

    #[test]
    fn if_c_one_zero_materializes_the_bool_without_a_select() {
        // (def (f (: a Int64) (: b Int64)) (if (< a b) 1 0)) — the boolean materialization: the compare
        // already yields 0/1, so the `if` is just that i32 bool widened to the i64 result — NO two consts,
        // NO select, NO branch.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 1 0)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(0),
                Lir::LocalGet(1),
                Lir::I64LtS,
                Lir::I64ExtendI32U, // widen the 0/1 bool to the Int64 result — no select.
            ]
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Select)),
            "the 1/0 branches materialize the condition directly, no select"
        );
    }

    #[test]
    fn if_c_zero_one_materializes_the_negated_bool() {
        // (if (< a b) 0 1) — the reversed literals are the NEGATION of the condition. Since the condition
        // is a comparison, the negation folds into the COMPLEMENT comparison (`a >=ₛ b`) rather than
        // `compare ; i32.eqz` — one instruction fewer, no double negation — then widen.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 0 1)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(0),
                Lir::LocalGet(1),
                Lir::I64GeS, // the complement of `<` — no trailing eqz.
                Lir::I64ExtendI32U,
            ]
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
            "the negated materialization folds into the complement, no eqz"
        );
    }

    #[test]
    fn if_not_compare_one_zero_avoids_the_double_negation() {
        // (if (not (< a b)) 1 0) — `lower` branch-swaps this to `(if (< a b) 0 1)`, then the negated
        // materialization would naively stack `i32.eqz` on the compare. Because the condition is a
        // comparison, the negation folds into the complement `a >=ₛ b` — NO `eqz` at all (the fold that
        // prevents an `eqz ; eqz` when `(not (= n 0))` composes with the bool-int form).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (not (< a b)) 1 0)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(0),
                Lir::LocalGet(1),
                Lir::I64GeS,
                Lir::I64ExtendI32U,
            ]
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
            "no eqz — the negation folded into the complement comparison"
        );
    }

    #[test]
    fn if_not_eq_zero_one_zero_is_a_single_ne() {
        // (if (not (= n 0)) 1 0) — the ubiquitous "n is nonzero as an int" idiom. Was `eqz ; eqz` (the
        // compare-with-zero peephole then the negation). Now the negation folds the compare's complement:
        // `n ≠ 0` = `n ; const 0 ; i64.ne` — one `ne`, no double eqz.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (if (not (= n 0)) 1 0)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::I64Eqz | Lir::I32Eqz)),
            "no eqz double negation, got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::I64Ne),
            "nonzero folds to a single i64.ne, got: {:?}",
            f.code
        );
    }

    #[test]
    fn if_with_non_zero_one_constants_keeps_the_select() {
        // (if (< a b) 5 7) — the branches are not 1/0, so the materialization does NOT fire; the leaf
        // branches still lower to a branchless `select` (5, 7, cond).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 5 7)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Select)),
            "non-0/1 constant branches keep the select, got: {:?}",
            f.code
        );
    }

    #[test]
    fn signed_negation_uses_the_a_equals_min_guard_not_the_two_xor_sub() {
        // (def (f (: a Int64)) (- 0 a)) — negation: the machine `0 - a` plus a guard that traps iff
        // `a == MIN` (the one overflow), NOT the general two-`xor` signed-sub guard.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (- 0 a)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64Xor)),
            "negation's guard is a == MIN, not the two-xor sub guard, got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::ConstI64(i64::MIN)) && f.code.contains(&Lir::I64Eq),
            "the guard compares the operand against MIN, got: {:?}",
            f.code
        );
    }

    #[test]
    fn signed_divide_by_power_of_two_strength_reduces_to_the_bias_shift_sequence() {
        // (def (f (: n Int64)) (/ n 8)) — a SIGNED `/ 2^k` (k=3) becomes the branchless round-toward-zero
        // bias sequence, no `i64.div_s`: stash n in $a, then `(n + ((n >>ₛ 63) >>ᵤ 61)) >>ₛ 3`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (/ n 8)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::LocalGet(0),
                Lir::LocalTee(1), // $a = n, keeping n on the stack as the first quotient read.
                Lir::LocalGet(1),
                Lir::ConstI64(63),
                Lir::I64ShrS, // n >>ₛ 63 — all-ones iff n<0.
                Lir::ConstI64(61),
                Lir::I64ShrU, // >>ᵤ (64−3) — 2^3−1 iff n<0, else 0.
                Lir::I64Add,  // n + bias.
                Lir::ConstI64(3),
                Lir::I64ShrS, // >>ₛ 3.
            ]
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
            "the signed divide is strength-reduced away, no i64.div_s"
        );
    }

    #[test]
    fn signed_remainder_by_power_of_two_strength_reduces_without_rem_s() {
        // (def (f (: n Int64)) (% n 8)) — a SIGNED `% 2^k` reduces to `n − (q << k)` over the same bias
        // quotient, no `i64.rem_s`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (% n 8)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64RemS)),
            "the signed remainder is strength-reduced away, no i64.rem_s, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::I64Sub))
                && f.code.iter().any(|i| matches!(i, Lir::I64Shl)),
            "remainder is n − (q << k), so a sub over a shifted quotient, got: {:?}",
            f.code
        );
    }

    #[test]
    fn divide_by_a_non_power_of_two_keeps_the_machine_divide() {
        // (/ n 3) — 3 is not a power of two, so the strength reduction does NOT fire and the machine
        // `i64.div_s` stays.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (/ n 3)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
            "a non-power-of-two divide keeps i64.div_s, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_right_shift_leaves_its_result_on_the_stack_without_a_dead_store() {
        // (def (f (: a Int64)) (>> a 3)) — a `>>` is EXACT (its result only shrinks), so it needs no
        // overflow round-trip and no range-check. The result stays on the stack: just `get a ; const 3 ;
        // shr_s`, with NO `$r` store and NO declared local. (The old code routed EVERY shift through a
        // `set $r ; get $r` round-trip — dead motion + a dead local for `>>`, since only `<<` reads `$r`
        // back for its overflow check.)
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (>> a 3)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::ConstI64(3), Lir::I64ShrS],
            "a constant-count `>>` is exactly the machine shift — no dead round-trip"
        );
        assert!(
            f.declared.is_empty(),
            "a `>>` claims no result scratch local, got: {:?}",
            f.declared
        );
    }

    #[test]
    fn identical_operands_are_computed_once_via_cse() {
        // (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) — the two operands are the SAME product.
        // CSE computes `(* a b)` ONCE into a slot; the outer add reads that slot for BOTH operands. So
        // the body contains exactly ONE `i64.mul` (not two), and the add's operands are two reads of the
        // shared slot.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            1,
            "the shared product is computed exactly once (CSE)"
        );
        // The add reads the product's slot twice as its two operands (a LocalGet of the same slot). Find
        // the mul's result slot (the LocalSet right after the sole I64Mul... or a LocalTee if the guard
        // fused) and confirm the I64Add is preceded by two reads of it.
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Add).count(),
            1,
            "one add over the shared product"
        );
    }

    #[test]
    fn a_multi_use_inlined_param_arg_is_computed_once_by_straight_line_cse() {
        // β-reduction SHARES a param's argument occurrence at every use. `(def (g s) (+ s (* s 3)))`
        // inlined with `s = (* a b)` leaves the ONE `(* a b)` node referenced twice — but across DIFFERENT
        // ops (`+` and `*`), so the intra-op arith-CSE (which shares only the two operands of ONE op) does
        // NOT catch it, and `(* a b)` emitted TWICE. Straight-line CSE now computes the shared `(* a b)`
        // ONCE into a slot up-front and reads it at both uses → exactly ONE `i64.mul` for the argument
        // (plus the `(* s 3)` = 2 total muls). Pins the count + relies on the corpus/run for value parity.
        let ast = crate::testkit::parse(
            "(module m (def (g (: s Int64)) (+ s (* s 3))) \
               (def (f (: a Int64) (: b Int64)) (g (* a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // Two muls: the SHARED `(* a b)` computed once + the `(* s 3)`. Before straight-line CSE this was
        // THREE (the `(* a b)` argument duplicated at each of its two uses).
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            2,
            "the inlined multi-use argument `(* a b)` is computed once (2 muls: shared arg + `* s 3`), \
             got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_cse_slotted_checked_arith_rep_writes_directly_to_its_slot() {
        // When the CSE representative is a CHECKED arithmetic op (`+`/`-`/`*`), it is emitted with its
        // result DEST = the CSE slot (via `emit_operand_into`'s `ResultDest::Slot`), so its `$r` IS the
        // slot — the store is direct, with NO `local.get $r ; local.set $cse` register-move. Before this,
        // the checked op wrote its own `$r` scratch, then the CSE pass copied `$r → slot` (a wasted temp +
        // move). `(f x (+ x 1))` inlines `f(a,b)=(+ (* a b) (- a b))`; `b = (+ x 1)` is used twice → CSE'd.
        // Assert there is NO `LocalGet(t) ; LocalSet(s)` pair where `t != s` (a pure register-to-register
        // move) among the emitted code — the CSE arith stores straight into its slot.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (- a b))) \
               (def (g (: x Int64)) (f x (+ x 1))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "g");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        let reg_move = f
            .code
            .windows(2)
            .any(|w| matches!((&w[0], &w[1]), (Lir::LocalGet(t), Lir::LocalSet(s)) if t != s));
        assert!(
            !reg_move,
            "the CSE'd `(+ x 1)` writes directly to its slot — no `get t ; set s` register move, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_single_use_inlined_param_arg_is_not_cse_slotted() {
        // A param used ONCE needs no CSE — the argument is inlined at its single site, same as before.
        // `(def (g s) (* s 5))` given `s = (* a b)` → exactly ONE `(* a b)` for the arg, plus the `(* s 5)`
        // = 2 muls (5 is not a power of two, so it stays a real mul, not a strength-reduced shift). No CSE
        // slot is introduced — straight-line CSE only fires at ≥2 references.
        let ast = crate::testkit::parse(
            "(module m (def (g (: s Int64)) (* s 5)) \
               (def (f (: a Int64) (: b Int64)) (g (* a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            2,
            "single-use arg inlines (2 muls: the arg + `* s 5`), got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_repeated_indexed_read_shares_one_vec_get_via_cse() {
        // `(+ (Option.expect (List.at xs 2)) (Option.expect (List.at xs 2)))` — the SAME bounds-checked
        // indexed read (`vec-get` behind a bounds check, then unbox via `expect`) twice. `List.at` BORROWS
        // the list and is deterministic, and the whole `SumExpect(ListAt …)` is a SCALAR read (the element
        // is an `Int64`), so straight-line CSE computes it ONCE — the emitted body contains exactly ONE
        // `vec-get`, not two (the ~20-instruction bounds-check + read + unwrap sequence is shared). This is
        // the indexed-read analogue of the `List.len` CSE (a repeated count already shares its `vec-len`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: xs (List Int64))) \
               (+ (Option.expect (List.at xs 2) \"v\") (Option.expect (List.at xs 2) \"v\"))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_GET))
                .count(),
            1,
            "the repeated indexed read shares one vec-get (CSE), got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_repeated_map_lookup_shares_one_map_lookup_via_cse() {
        // `(+ (Option.expect (Map.lookup m 2)) (Option.expect (Map.lookup m 2)))` — the SAME keyed lookup
        // twice. `Map.lookup` BORROWS the map and is deterministic; the whole `SumExpect(MapLookup …)` is a
        // SCALAR read (the value is an `Int64`), so straight-line CSE computes it ONCE — exactly ONE
        // `map-lookup` (an O(log n) CHAMP walk), not two. The keyed-read analogue of the `List.at` CSE.
        let ast = crate::testkit::parse(
            "(module m (def (f (: m (Map Int64 Int64))) \
               (+ (Option.expect (Map.lookup m 2) \"v\") (Option.expect (Map.lookup m 2) \"v\"))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_MAP_LOOKUP))
                .count(),
            1,
            "the repeated keyed lookup shares one map-lookup (CSE), got: {:?}",
            f.code
        );
    }

    #[test]
    fn str_from_bytes_does_not_over_declare_arr_alloc() {
        // `String.from-bytes` emits `str-from-bytes` (decode → handle-or-NULL) then builds `Some(handle)` /
        // `None` via `sum-new`; the `None` payload is the INLINE-unit constant (`IMM_UNIT`), so no
        // `arr-alloc` is ever called. The used-ops collector must therefore NOT import `arr-alloc` for a
        // body whose only heap op is `str-from-bytes` (an earlier version over-declared it "for None's
        // unit", forcing an unnecessary runtime import — PR #404 Copilot review). The bytes come from a
        // PARAMETER so no construction op contributes other imports.
        let ast = crate::testkit::parse(
            "(module m (def (f (: b Bytes)) (String.from-bytes b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (_params, body) = function_of(&mut db, "f");
        let mut ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        collect_used_ops(&mut db, body, &mut ops);
        assert!(
            ops.contains(OP_STR_FROM_BYTES),
            "str-from-bytes must be imported, got: {ops:?}"
        );
        assert!(
            ops.contains(OP_SUM_NEW),
            "sum-new (Some/None build) must be imported, got: {ops:?}"
        );
        assert!(
            !ops.contains(OP_ARR_ALLOC),
            "arr-alloc must NOT be imported — None uses the inline-unit constant, not an allocation; \
             got: {ops:?}"
        );
    }

    #[test]
    fn fallible_read_ops_do_not_over_declare_arr_alloc() {
        // Every fallible read that returns `(Option T)` — `List.at`, `Map.lookup`, `Bytes.at`,
        // `String.at`, `Bytes.slice` (the family sharing `String.from-bytes`'s shape) — builds its `None`
        // from the inline-unit constant (`IMM_UNIT`), NOT an allocation, so NONE of them calls
        // `arr-alloc` (verified against each emit arm). The used-ops collector must not import `arr-alloc`
        // for a body whose only heap op is one of these reads over PARAMETERS (no construction op
        // contributes other imports). This pins the whole family against the PR #404 over-declaration
        // class (an over-imported op forces an unnecessary component import).
        let cases: &[(&str, &str)] = &[
            (
                "(def (f (: xs (List Int64)) (: i Int64)) (List.at xs i))",
                "List.at",
            ),
            (
                "(def (f (: m (Map Int64 Int64)) (: k Int64)) (Map.lookup m k))",
                "Map.lookup",
            ),
            (
                "(def (f (: b Bytes) (: i Int64)) (Bytes.at b i))",
                "Bytes.at",
            ),
            (
                "(def (f (: s String) (: i Int64)) (String.at s i))",
                "String.at",
            ),
            (
                "(def (f (: b Bytes) (: s Int64) (: l Int64)) (Bytes.slice b s l))",
                "Bytes.slice",
            ),
        ];
        for (def, label) in cases {
            let src = format!("(module m {def} (def (main) 0) (export main))");
            let mut db = Db::load(crate::testkit::parse(&src));
            let (_params, body) = function_of(&mut db, "f");
            let mut ops: std::collections::BTreeSet<&'static str> =
                std::collections::BTreeSet::new();
            collect_used_ops(&mut db, body, &mut ops);
            assert!(
                ops.contains(OP_SUM_NEW),
                "{label}: sum-new (Some/None build) must be imported, got: {ops:?}"
            );
            assert!(
                !ops.contains(OP_ARR_ALLOC),
                "{label}: arr-alloc must NOT be imported — None uses the inline-unit constant, not an \
                 allocation; got: {ops:?}"
            );
        }
    }

    #[test]
    fn str_at_does_not_over_declare_drop() {
        // `String.at` DUPs the string (the slice takes an independent ref and consumes that dup), so the
        // ORIGINAL string is not consumed here — its owner (an enclosing let/param) reclaims it, and the
        // emit calls no `drop` (unlike `Map.lookup`/`Set.contains`, whose boxed KEY is an owned temporary
        // they must drop). The used-ops collector must not import `drop` for a `String.at` body — an
        // over-declaration found auditing the fallible-read family (the same import-minimization class as
        // the arr-alloc over-declares).
        let ast = crate::testkit::parse(
            "(module m (def (f (: s String) (: i Int64)) (String.at s i)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (_params, body) = function_of(&mut db, "f");
        let mut ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        collect_used_ops(&mut db, body, &mut ops);
        assert!(
            ops.contains(OP_DUP) && ops.contains(OP_BYTES_SLICE),
            "String.at must import dup + bytes-slice (the dup-then-consume slice), got: {ops:?}"
        );
        assert!(
            !ops.contains(OP_DROP),
            "drop must NOT be imported — the original string is reclaimed by its owner, not dropped \
             here; got: {ops:?}"
        );
    }

    #[test]
    fn straight_line_cse_value_numbers_distinct_occurrences_across_ops() {
        // VALUE-NUMBERING (not node identity): two DISTINCT `(* a b)` occurrences across DIFFERENT ops —
        // `(+ (* a b) (* (* a b) 3))` — are `core_eq`, so straight-line CSE computes the product ONCE and
        // shares it. Exactly TWO muls remain: the shared `(* a b)` + the `(* … 3)`. Before value-numbering
        // (node-identity only) this was THREE (each hand-written `(* a b)` emitted separately).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (* (* a b) 3))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            2,
            "value-equal `(* a b)` across ops is computed once (2 muls), got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_cse_slotted_operand_is_read_directly_not_recopied() {
        // A CSE-hoisted subexpression used as an ARITHMETIC OPERAND is read straight from its CSE slot —
        // `operand_src` honors the node's own slot, so no spurious copy into a fresh scratch slot. Before
        // this, `(+ (& x 7) (& x 7))` emitted `local.tee <cse> ; local.tee <scratch> ; local.get <scratch>`
        // (the operand path spilled the already-slotted value again); now it is `local.tee <cse> ;
        // local.get <cse> ; add` — identical to the explicit `(let ((y (& x 7))) (+ y y))`. Assert the two
        // lower to the SAME local count and the SAME emitted code.
        let lir = |src: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let cse = lir(
            "(module m (def (f (: x Int64)) (+ (& x 7) (& x 7))) (def (main) 0) (export main))",
        );
        let via_let = lir(
            "(module m (def (f (: x Int64)) (let ((y (& x 7))) (+ y y))) (def (main) 0) (export main))",
        );
        assert_eq!(
            cse, via_let,
            "a CSE'd operand emits identically to an explicit let (no extra copy), got: {cse:?}"
        );
        // Concretely: exactly ONE `i64.and` (computed once) and the shared value read straight from its
        // CSE slot — `[get x ; const 7 ; and ; tee <cse> ; get <cse> ; add ; tee <ret>]`. The redundant
        // `local.tee/set <scratch>` that spilled the already-teed value is GONE (was a distinct middle
        // slot); the only two tees are the shared-value store and the result store.
        assert_eq!(
            cse.iter().filter(|i| **i == Lir::I64And).count(),
            1,
            "the shared `(& x 7)` is computed once, got: {cse:?}"
        );
        assert!(
            !cse.iter().any(|i| matches!(i, Lir::LocalSet(_))),
            "no redundant local.set spilling the already-teed CSE value, got: {cse:?}"
        );
    }

    #[test]
    fn straight_line_cse_does_not_hoist_a_let_local_subexpression() {
        // A shared subexpression over a `let`-LOCAL — `(let ((k (+ a b))) (+ (* k k) (* k k)))` — must NOT
        // be hoisted before the body: the local `k`'s slot is only established when the `let` binding is
        // emitted INSIDE the body, so a hoisted `(* k k)` would read an unbound slot ("let-binding reference
        // has no local slot"). `is_cse_shareable` excludes `Core::LocalRef`, so a computation over a
        // let-local is left in place. This must COMPILE (a regression guard — an early value-numbering
        // version crashed here) and value-check. `(let ((k 7)) (+ (* k k) (* k k)))` = 49+49 = 98.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((k (+ a b))) (+ (* k k) (* k k)))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        // The key assertion is that selection SUCCEEDS (no "no local slot" crash from a bad hoist).
        select_function(&mut db, body, &params, &layout)
            .expect("a let-local subexpression must not be hoisted before its binding");
    }

    #[test]
    fn dominator_cse_hoists_a_condition_dominated_subexpression() {
        // `(if (> (* a b) 0) (* a b) (- 0 (* a b)))` — the `(* a b)` in the CONDITION is always evaluated
        // (it DOMINATES both branches), so all three value-equal `(* a b)` collapse to ONE computed slot
        // read in the cond + both branches. Exactly ONE `i64.mul` (was 3). The dominance requirement is
        // what makes hoisting across the `if` sound: the class runs on entry regardless of the branch taken.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (> (* a b) 0) (* a b) (- 0 (* a b)))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            1,
            "a condition-dominated `(* a b)` is computed once and shared across cond+branches, got: {:?}",
            f.code
        );
    }

    #[test]
    fn dominator_cse_does_not_hoist_a_branch_only_subexpression() {
        // `(if (> c 0) (* a b) (- 0 (* a b)))` — `(* a b)` appears ONLY in the two BRANCHES, never in the
        // (always-evaluated) condition, so it is NOT in the dominating frontier. Hoisting it would SPECULATE
        // the product (and, for a trapping op, its trap) onto the code path that runs before the branch is
        // chosen — unsound. So it must be left in place: exactly TWO `i64.mul` (one per branch), not one.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (if (> c 0) (* a b) (- 0 (* a b)))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            2,
            "a branch-only shared `(* a b)` (no dominating occurrence) is NOT hoisted, got: {:?}",
            f.code
        );
    }

    #[test]
    fn cse_shares_a_repeated_collection_count() {
        // `(List.len xs)` is a TOTAL O(1) BORROWING scalar read (a `vec-len` runtime import — no rc change,
        // deterministic). Two identical counts of the same list param `(+ (List.len xs) (* (List.len xs) 3))`
        // are `core_eq` and dominate (straight-line body), so CSE computes the `vec-len` ONCE and shares it
        // → exactly ONE `vec-len` CallImport (was two). `xs` is a real PARAM (a list handle live up front),
        // so the read is well-formed at the hoist point. Selects `f` directly (its param is an i32 handle).
        let ast = crate::testkit::parse(
            "(module m (def (f (: xs (List Int64))) (+ ((. List len) xs) (* ((. List len) xs) 3))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
                .count(),
            1,
            "a repeated `(List.len xs)` is computed once and shared, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_repeated_sum_payload_read_is_shared_by_cse() {
        // (match o ((Some x) (+ x x)) ((None) 0)) — the binder `x` resolves to a `Core::SumPayload` at
        // EACH occurrence, so `(+ x x)` names two DISTINCT SumPayload nodes. `core_eq` now recognizes
        // them as equal (same scrutinee + path), so the arith-CSE reads the payload ONCE
        // (`sum-payload ; get-int` a single time) into a slot and shares it for both `+` operands —
        // exactly as a repeated tuple/record field `(+ (. r x) (. r x))` already was. The match is kept
        // runtime by making `f` recursive on a fresh `(None)`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: o (Option Int64)) (: acc Int64)) \
               (match o ((Some x) (f (None) (+ acc (+ x x)))) ((None) acc))) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
                .count(),
            1,
            "the payload `x` is read exactly once and shared across `(+ x x)`, got: {:?}",
            f.code
        );
    }

    #[test]
    fn doubling_add_collapses_the_overflow_guard_to_one_xor() {
        // (def (f (: a Int64)) (+ a a)) — both operands are the SAME source, so the signed-add guard
        // `((r^a)&(r^b))<0` with `b==a` collapses to `(r^a)<0`: ONE xor, no `and`, no second `r^b`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (+ a a)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| matches!(i, Lir::I64Xor)).count(),
            1,
            "(+ a a) guard is a single xor (`(r^a)<0`), got: {:?}",
            f.code
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64And)),
            "the `& (r^b)` half is gone — x & x = x, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_provably_in_range_arith_op_elides_its_overflow_guard() {
        let select = |src: &str| {
            let mut db = Db::load(crate::testkit::parse(src));
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        // `(+ (& x 15) (& y 15))`: both operands ∈ [0,15], sum ∈ [0,30], fits Int64 → NO overflow guard
        // (no `((r^a)&(r^b))<0` sign test).
        let add = select(
            "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 15) (& y 15))) (def (main) 0) (export main))",
        );
        assert!(
            !add.iter().any(|i| matches!(i, Lir::I64Xor))
                && !add.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
            "a provably-in-range add drops its guard; got {add:?}"
        );
        // `(* (& x 15) 3)`: [0,15]×3 = [0,45], fits → NO const-multiplier bound check.
        let mul =
            select("(module m (def (f (: x Int64)) (* (& x 15) 3)) (def (main) 0) (export main))");
        assert!(
            !mul.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
            "a provably-in-range mul drops its bound check; got {mul:?}"
        );
        // A full-range add (either operand unbounded) KEEPS its guard.
        let kept = select(
            "(module m (def (f (: x Int64) (: y Int64)) (+ x y)) (def (main) 0) (export main))",
        );
        assert!(
            kept.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
            "a full-range add keeps its overflow guard; got {kept:?}"
        );
        // A NARROW result whose interval EXCEEDS the type keeps its range-check: [0,200]+[0,200]=[0,400]
        // > UInt8 255.
        let narrow_over = select(
            "(module m (def (f (: x UInt8) (: y UInt8)) (+ (& x 200) (& y 200))) (def (main) 0) (export main))",
        );
        assert!(
            narrow_over
                .iter()
                .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
            "an over-range narrow add keeps its range-check; got {narrow_over:?}"
        );
        // CHAINED: the range PROPAGATES through nested arith — the inner `(+ (& x 15) (& y 15))` bounds to
        // [0,30], so the OUTER `(+ … (& z 15))` sees [0,30]+[0,15]=[0,45] and BOTH adds elide their guard
        // (zero xor across the whole body).
        let chained = select(
            "(module m (def (f (: x Int64) (: y Int64) (: z Int64)) \
               (+ (+ (& x 15) (& y 15)) (& z 15))) (def (main) 0) (export main))",
        );
        assert!(
            !chained.iter().any(|i| matches!(i, Lir::I64Xor)),
            "both adds in a chain elide their guard via range propagation; got {chained:?}"
        );
        // A chain where a middle operand is UNBOUNDED (`y`) keeps BOTH guards.
        let chained_open = select(
            "(module m (def (f (: x Int64) (: y Int64) (: z Int64)) \
               (+ (+ (& x 15) y) (& z 15))) (def (main) 0) (export main))",
        );
        assert!(
            chained_open
                .iter()
                .filter(|i| matches!(i, Lir::I64Xor))
                .count()
                >= 2,
            "an unbounded operand in the chain keeps the guards; got {chained_open:?}"
        );
    }

    #[test]
    fn a_guard_elided_arith_operand_leaves_its_result_on_the_stack_no_dead_store() {
        // When a checked arith's overflow guard is PROVABLY elided (result in range) AND the op is used as
        // an operand/argument (dest = Stack), the machine op's result is already on the stack — it must be
        // left there, NOT round-tripped through `local.set $r ; local.get $r` (which the peephole then
        // fuses to a `local.tee $r` INTO A SLOT NEVER READ — a dead store). `(g (- n 1))` under `n >= 2`
        // (from the branch refinement) elides the `(- n 1)` underflow guard, so the arg should be
        // `... i64.sub ; call` with no `local.tee`/`local.set` of a dead slot between the sub and the call.
        let ast = crate::testkit::parse(
            "(module m \
               (def (g (: n Int64)) (if (< n 2) n (+ (g (- n 1)) 1))) \
               (def (f (: x Int64)) (g x)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("g").expect("g");
        let (params, body) = function_of(&mut db, "g");
        let code = select_function_of(&mut db, body, &params, &layout, Some(d))
            .expect("select")
            .code;
        // The `(- n 1)` argument: an `I64Sub` immediately followed by the `Call` (or a `return_call`), with
        // NO `LocalTee`/`LocalSet` in between (the guard-elided result flows straight into the call).
        let sub_ix = code
            .iter()
            .position(|i| matches!(i, Lir::I64Sub))
            .expect("the (- n 1) argument subtracts");
        let next = &code[sub_ix + 1];
        assert!(
            matches!(next, Lir::Call(_) | Lir::ReturnCall(_)),
            "a guard-elided (- n 1) argument flows straight into the call — no dead store between \
             the sub and the call; got next = {next:?} in {code:?}"
        );
    }

    #[test]
    fn a_guard_elided_arith_emits_its_operands_inline_with_no_scratch_slots() {
        // `(+ (& x 7) (& y 7))`: both operands ∈ [0,7], sum ∈ [0,14], fits → the overflow guard AND the
        // narrow range-check are elided. With NO guard to re-read the operands or the result, each operand
        // is used EXACTLY ONCE (only the `i64.add` reads it), so a non-reusable operand need not be stashed
        // in a scratch slot: both masked operands emit straight onto the stack. The whole body declares ZERO
        // locals (before, each masked operand was `local.set` into a slot then reloaded, plus a dead `$r`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 7) (& y 7))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.declared,
            Vec::<ValType>::new(),
            "a guard-elided masked add needs no scratch slots — operands emit inline; got {:?}",
            f.code
        );
        // The exact inline sequence: mask x, mask y, add — no `local.set`/`local.tee` anywhere.
        assert!(
            !f.code
                .iter()
                .any(|i| matches!(i, Lir::LocalSet(_) | Lir::LocalTee(_))),
            "no operand is stashed in a slot when the guard is elided; got {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::I64Add)
                && f.code.iter().filter(|i| matches!(i, Lir::I64And)).count() == 2,
            "the body is `(& x 7)` inline, `(& y 7)` inline, `i64.add`; got {:?}",
            f.code
        );
    }

    #[test]
    fn distinct_add_operands_keep_the_two_xor_guard() {
        // (+ a b) with DISTINCT operands cannot collapse — both `r^a` and `r^b` are needed.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| matches!(i, Lir::I64Xor)).count(),
            2,
            "distinct operands keep both xors, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_self_tail_recursive_function_compiles_to_a_loop() {
        // (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) — the self-call is
        // in tail position (the `if`'s else branch), so `select_function_of` (given f's own def index)
        // compiles it as a LOOP: the body opens with `Lir::Loop`, the self-call updates the param slots
        // (`local.set`s) and `br`s back, and there is NO `ReturnCall`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64) (: acc Int64)) \
               (if (= n 0) acc (f (- n 1) (+ acc 1)))) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            matches!(f.code.first(), Some(Lir::Loop(_))),
            "a self-tail-recursive function body opens with a loop"
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Br(_))),
            "the self-tail-call branches back to the loop top"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "no return_call — the self-call became a loop iteration"
        );
    }

    #[test]
    fn a_pass_through_parameter_elides_its_self_move_at_the_loop_back_edge() {
        // (def (go (: n Int64) (: k Int64) (: acc Int64)) (if (= n 0) acc (go (- n 1) k (+ acc k)))) —
        // `k` is re-passed UNCHANGED to its own slot. The back-edge parallel move is `set acc ; set n`
        // only: the `k` arg is neither pushed (no `local.get k` for it) nor stored (no `local.set k`),
        // since a self-move `k ← k` is a no-op. `k`'s slot is read only by `(+ acc k)`, not moved.
        let ast = crate::testkit::parse(
            "(module m (def (go (: n Int64) (: k Int64) (: acc Int64)) \
               (if (= n 0) acc (go (- n 1) k (+ acc k)))) (export go))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("go").expect("def go");
        let (params, body) = function_of(&mut db, "go");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        // Param slots are 0=n, 1=k, 2=acc. The back-edge stores only n and acc — NOT k. Count the
        // `local.set` into slot 1 (k): there must be none in the whole body (k is never re-stored).
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::LocalSet(1))),
            "the pass-through param k (slot 1) is never re-stored, got: {:?}",
            f.code
        );
        // The other two params ARE stored at the back-edge.
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::LocalSet(0)))
                && f.code.iter().any(|i| matches!(i, Lir::LocalSet(2))),
            "n and acc are still updated each iteration"
        );
    }

    #[test]
    fn a_mutually_tail_recursive_pair_compiles_to_a_shared_loop() {
        // even/odd tail-call each other (same signature) — each compiles to ONE `loop` with a `which`
        // dispatch: the body opens with `Lir::Loop`, a cross-call sets `which` + `br`s back, and there
        // is NO `ReturnCall` (the mutual tail-call became a loop iteration, not a real call).
        let ast = crate::testkit::parse(
            "(module m (def (even (: n Int64)) (if (= n 0) true (odd (- n 1)))) \
               (def (odd (: n Int64)) (if (= n 0) false (even (- n 1)))) (export even))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("even").expect("def even");
        let (params, body) = function_of(&mut db, "even");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        // The loop is not the FIRST instruction (the `which` init precedes it), but it is present near
        // the top, the cross-calls `br`, and no `return_call` survives.
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "a mutually-tail-recursive member compiles to a loop, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Br(_))),
            "the mutual tail-call branches back to the loop top"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "no return_call — the mutual tail-call became a loop iteration, got: {:?}",
            f.code
        );
    }

    #[test]
    fn mutual_recursion_with_different_signatures_stays_return_call() {
        // `f(n)` tail-calls `g(n,k)` and vice-versa — DIFFERENT arities, so they can't share one set of
        // parameter slots. The shared-loop transform must decline (signature guard) and leave the mutual
        // tail-calls as `return_call` (still O(1) stack, just a real tail call, not a loop `br`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (if (= n 0) 1 (g (- n 1) 2))) \
               (def (g (: n Int64) (: k Int64)) (if (= n 0) k (f (- n 1)))) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "heterogeneous-signature mutual recursion is not merged into a loop, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "the cross-call to a different-signature peer stays a return_call"
        );
    }

    #[test]
    fn a_non_recursive_function_is_not_wrapped_in_a_loop() {
        // A plain `(+ a b)` — no self-call, so no loop wrapping even when the def index is supplied.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "a non-recursive function is not wrapped in a loop"
        );
    }

    #[test]
    fn a_dense_scalar_match_emits_a_br_table() {
        // A value-position match over ≥3 dense integer literals (0..4) + a wildcard emits a `br_table`
        // decision tree (and the enclosing `Block`s), not a linear `if (== k)` chain (no `I64Eq` probe).
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) \
               (let ((r (match n (0 100) (1 101) (2 102) (3 103) (4 104) (_ 999)))) r)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
            "a dense scalar match emits a br_table, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Block(_))),
            "the br_table is wrapped in dispatch blocks"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
            "a br_table dispatch has no linear per-arm equality probe, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_br_table_over_a_zero_based_range_skips_the_index_shift() {
        // A dense `br_table` normalizes the scrutinee to a 0-based table index via `scrutinee - min`. When
        // the covered range STARTS AT 0 — the common `(match x (0 …) (1 …) …)` shape — that shift is the
        // identity `x - 0`, so the `const 0 ; sub` is dead and skipped: the scrutinee IS the index. A
        // range NOT starting at 0 keeps the subtract. Assert the min=0 table has NO `I64Sub` while the
        // min=5 table has exactly one (the index shift).
        let lir = |src: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, "f");
            select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let min0 = lir(
            "(module m (def (f (: x Int64)) (match x (0 10) (1 20) (2 30) (3 40) (_ 50))) (def (main) 0) (export main))",
        );
        assert!(
            min0.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
            "the min=0 match still uses a br_table, got: {min0:?}"
        );
        assert!(
            !min0.iter().any(|i| matches!(i, Lir::I64Sub)),
            "a 0-based range skips the `x - 0` index shift, got: {min0:?}"
        );
        // The wrap-aliasing guard (`idx >=u span → default`) is UNAFFECTED — a negative/huge i64 scrutinee
        // still routes to the default, so the out-of-range compare survives.
        assert!(
            min0.iter().any(|i| matches!(i, Lir::I64GeU)),
            "the out-of-range wrap guard is kept, got: {min0:?}"
        );
        let min5 = lir(
            "(module m (def (f (: x Int64)) (match x (5 10) (6 20) (7 30) (_ 40))) (def (main) 0) (export main))",
        );
        assert_eq!(
            min5.iter().filter(|i| matches!(i, Lir::I64Sub)).count(),
            1,
            "a non-zero-based range keeps its `x - min` index shift, got: {min5:?}"
        );
    }

    #[test]
    fn an_exhaustive_sum_match_br_table_elides_the_dead_default_block() {
        // A 3-variant EXHAUSTIVE sum match (Sign: Neg/Zero/Pos, no wildcard) — the disc is provably in
        // [0,3), so the br_table's out-of-range default is dead. The LAST arm serves as the default:
        // `br_table [0, 1] default=2` (2 explicit targets, NOT 3), and there is no separate `$default`
        // block wrapping an `unreachable`. So the table's target list has `m-1 = 2` entries.
        let ast = crate::testkit::parse(
            "(module m (def (f (: s Sign)) \
               (let ((r (match s ((Neg) 10) ((Zero) 20) ((Pos) 30)))) r)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        let table = f
            .code
            .iter()
            .find_map(|i| match i {
                Lir::BrTable(targets, default) => Some((targets.clone(), *default)),
                _ => None,
            })
            .expect("an exhaustive sum match emits a br_table");
        assert_eq!(
            table.0,
            vec![0, 1],
            "3 variants → 2 explicit targets (arms 0,1); the last arm is the default"
        );
        assert_eq!(
            table.1, 2,
            "the table default targets the last arm (disc 2)"
        );
        // No `unreachable` from a dead default (this match has no arithmetic guards, so ANY unreachable
        // would be the elided dead-default one).
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Unreachable)),
            "no dead-default unreachable, got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_two_variant_sum_match_with_leaf_bodies_selects_branchlessly() {
        // A 2-variant sum (enum) match with cheap trap-free LEAF arm bodies is `(if (disc == d) A B)` — the
        // sum-discriminant twin of the scalar 2-arm select. `(match f (On 1) (Off 0))` → `1 ; 0 ;
        // <disc> ; i32.eqz ; select`, NOT an `if`/`else` block. Sound: a `Leaf` body is trap-free
        // (`is_select_arm`); a payload-reading arm (`SumPayload`) is NOT trap-free and keeps the `if` — see
        // `an_option_match_with_a_payload_reading_arm_keeps_its_if`.
        let ast = crate::testkit::parse(
            "(module m (type Flag On Off) \
               (def (rank (: f Flag)) (match f (Flag.On 1) (Flag.Off 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("rank").expect("rank");
        let (params, body) = function_of(&mut db, "rank");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            f.code.contains(&Lir::Select)
                && !f.code.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
            "a 2-variant enum match with leaf bodies selects branchlessly (no if/else): {:?}",
            f.code
        );
    }

    #[test]
    fn an_option_match_with_a_payload_reading_arm_keeps_its_if() {
        // A 2-arm sum match whose arm READS the payload — `(match o ((Some v) (+ v 1)) ((None) 0))` — must
        // NOT become a branchless `select`: `select` evaluates BOTH arms, so it would read the `Some`
        // payload even when the value is `None` (a `SumPayload` on the wrong variant). `is_select_arm`
        // (via `is_trap_free`) excludes a `SumPayload` read, so the `if`/`else` decision-tree survives.
        let ast = crate::testkit::parse(
            "(module m \
               (def (f (: o (Option Int64))) (match o ((Some v) (+ v 1)) ((None) 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.contains(&Lir::Select),
            "a payload-reading Option arm keeps the if (no speculative select): {:?}",
            f.code
        );
    }

    // (corpus companion: `05-compound-types.sexp` "a match arm reading two elements of a boxed payload
    // tuple shares the sum-payload prefix" pins the runtime tree/list fold value with the prefix CSE.)
    #[test]
    fn a_match_arm_reading_two_payload_elements_computes_the_prefix_once() {
        // A `(Pair (tuple a b))` arm binds `a` = SumPayload{p, [Payload, Elem(0)]} and `b` = SumPayload{p,
        // [Payload, Elem(1)]}. Both re-walk the shared `sum-payload(p)` prefix — the per-arm-body prefix
        // CSE computes it ONCE into a slot, so the arm reads BOTH elements off the one `sum-payload` via
        // `arr-get`. Non-recursive so `select_function_of` needs no cross-function emission order. `sum`
        // reads a and b: exactly ONE `sum-payload` (the shared prefix) + TWO `arr-get` (a and b).
        // TWO variants so `Pair`'s payload is genuinely BOXED (a single-variant newtype erases the box,
        // so there is no `sum-payload` prefix to share).
        let ast = crate::testkit::parse(
            "(module m (type P (Pair (Tuple Int64 Int64)) Nil) \
               (def (sum (: p P)) (match p ((P.Pair (tuple a b)) (+ a b)) ((P.Nil) 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("sum").expect("sum");
        let (params, body) = function_of(&mut db, "sum");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
                .count(),
            1,
            "the arm's shared payload prefix is computed ONCE (1 sum-payload, not 2): {:?}",
            f.code
        );
        assert_eq!(
            f.code
                .iter()
                .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_ARR_GET))
                .count(),
            2,
            "the two tuple elements read via arr-get off the shared prefix: {:?}",
            f.code
        );
    }

    #[test]
    fn a_two_arm_list_match_with_leaf_bodies_selects_branchlessly() {
        // A 2-arm list match — a LENGTH-test arm then a single unconditional cover — with cheap trap-free
        // LEAF bodies is `(if (len ⋈ k) A B)`, the list analogue of the scalar/sum 2-arm select.
        // `(match xs ((list) 0) ((list a .. r) 1))` dispatches on `len == 0` → `0 ; 1 ; (len==0) ; select`,
        // not an `if`/`else` block.
        let ast = crate::testkit::parse(
            "(module m (def (f (: xs (List Int64))) (match xs ((list) 0) ((list a .. r) 1))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            f.code.contains(&Lir::Select)
                && !f.code.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
            "a 2-arm list match with leaf bodies selects branchlessly (no if/else): {:?}",
            f.code
        );
    }

    #[test]
    fn a_list_match_reading_an_element_binder_keeps_its_if() {
        // A 2-arm list match whose cons arm READS an element binder — `(match xs ((list) -1) ((list a .. r)
        // a))` — must NOT become a `select`: `select` evaluates BOTH arms, so it would read element 0 even
        // on an EMPTY list (a `SumPayload` out-of-bounds). `is_select_arm` (via `is_trap_free`) excludes a
        // `SumPayload`, so the length `if` survives.
        let ast = crate::testkit::parse(
            "(module m (def (f (: xs (List Int64))) (match xs ((list) -1) ((list a .. r) a))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.contains(&Lir::Select),
            "an element-binder-reading list arm keeps the if (no speculative empty-list read): {:?}",
            f.code
        );
    }

    #[test]
    fn a_sum_match_with_a_wildcard_keeps_its_default_block() {
        // A sum match with FEWER explicit arms than variants + a wildcard (Color: Red/Green/Blue + `_`
        // covering Yellow) DOES need a real default block — the table default routes the uncovered disc
        // there. So the br_table has all 3 explicit targets AND a distinct default depth (= 3), and the
        // default block exists.
        let ast = crate::testkit::parse(
            "(module m (type Color Red Green Blue Yellow) \
               (def (f (: c Color)) \
                 (let ((r (match c ((Red) 1) ((Green) 2) ((Blue) 3) (_ 9)))) r)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        let table = f
            .code
            .iter()
            .find_map(|i| match i {
                Lir::BrTable(targets, default) => Some((targets.clone(), *default)),
                _ => None,
            })
            .expect("br_table");
        assert_eq!(
            table.0,
            vec![0, 1, 2],
            "3 explicit disc arms each get a target; the default is separate"
        );
        assert_eq!(
            table.1, 3,
            "the default routes past the 3 arms to the $default block"
        );
    }

    #[test]
    fn a_sparse_scalar_match_keeps_the_linear_probe_chain() {
        // A sparse range (0 and 100 — span 101 ≫ 2·2) is NOT worth a jump table; it keeps the linear
        // `if (== k)` chain (an `I64Eq` probe, no `br_table`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) \
               (let ((r (match n (0 1) (100 2) (7 3) (_ 0))) ) r)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
            "a sparse scalar match keeps the linear chain (no br_table), got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
            "the linear probe chain compares the scrutinee per arm"
        );
    }

    #[test]
    fn peephole_fuses_set_then_get_of_the_same_local_into_tee() {
        // `local.set N ; local.get N` (store then immediately re-read the SAME local) → `local.tee N`.
        let mut code = vec![
            Lir::I64Add,
            Lir::LocalSet(2),
            Lir::LocalGet(2), // same local as the set → fuses
            Lir::LocalGet(0),
            Lir::I64Xor,
        ];
        peephole(&mut code);
        assert_eq!(
            code,
            vec![Lir::I64Add, Lir::LocalTee(2), Lir::LocalGet(0), Lir::I64Xor]
        );
    }

    #[test]
    fn peephole_leaves_a_set_get_of_different_locals_alone() {
        // A `local.get` of a DIFFERENT local must NOT fuse (it is a genuine read of another value), and
        // a `local.set` not immediately followed by a matching `local.get` is untouched.
        let mut code = vec![
            Lir::LocalSet(3),
            Lir::LocalGet(4), // different local → no fuse
            Lir::LocalSet(5),
            Lir::I64Add, // set not followed by a get → no fuse
        ];
        let before = code.clone();
        peephole(&mut code);
        assert_eq!(code, before);
    }

    #[test]
    fn peephole_does_not_fuse_across_a_block_boundary() {
        // A block marker (`End`) between the set and the get keeps them non-adjacent, so no fuse — a
        // `local.get` opening a different block never merges with a `local.set` closing another.
        let mut code = vec![Lir::LocalSet(2), Lir::End, Lir::LocalGet(2)];
        let before = code.clone();
        peephole(&mut code);
        assert_eq!(code, before);
    }

    #[test]
    fn a_parameterized_comparison_selects_to_a_signed_compare() {
        // (def (lt (: a Int64) (: b Int64)) (< a b)) — a runtime signed comparison, result Bool (i32).
        // A comparison never overflows, so no scratch/guard — just push both and compare.
        let ast = crate::testkit::parse(
            "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "lt");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64LtS]
        );
        assert!(f.declared.is_empty());
        assert_eq!(f.ret, Ty::Bool);
    }

    #[test]
    fn equality_with_zero_selects_to_eqz() {
        // `(= n 0)` on a 64-bit param is `i64.eqz` (one instruction: push n, eqz) — NOT
        // `local.get 0 ; i64.const 0 ; i64.eq` (three). The zero operand is recognized at the compare
        // emit site; the commuted `(= 0 n)` folds the same way, and a NON-zero rhs keeps `i64.eq`.
        let check = |src: &str, name: &str, want: Vec<Lir>| {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = layout_of(&mut db);
            let (params, body) = function_of(&mut db, name);
            let f = select_function(&mut db, body, &params, &layout).expect("select");
            assert_eq!(f.code, want, "{src}");
        };
        check(
            "(module m (def (f (: n Int64)) (= n 0)) (def (main) 0) (export main))",
            "f",
            vec![Lir::LocalGet(0), Lir::I64Eqz],
        );
        // Commuted: `(= 0 n)` → the non-zero operand (n) then eqz.
        check(
            "(module m (def (f (: n Int64)) (= 0 n)) (def (main) 0) (export main))",
            "f",
            vec![Lir::LocalGet(0), Lir::I64Eqz],
        );
        // A ≤32-bit operand uses i32.eqz.
        check(
            "(module m (def (f (: n Int32)) (= n 0)) (def (main) 0) (export main))",
            "f",
            vec![Lir::LocalGet(0), Lir::I32Eqz],
        );
        // A NON-zero literal keeps the general equality (push both, i64.eq) — eqz is zero-only.
        check(
            "(module m (def (f (: n Int64)) (= n 5)) (def (main) 0) (export main))",
            "f",
            vec![Lir::LocalGet(0), Lir::ConstI64(5), Lir::I64Eq],
        );
    }

    #[test]
    fn a_nested_checked_op_shares_scratch_minimally() {
        // (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) — a nested checked op. The outer
        // mul's LHS is the inner add; instead of computing the add into its OWN $r and copying that to
        // the mul's $a, the add is emitted with `ResultDest::Slot($a)` so its result store writes $a
        // directly (no `local.get $r_inner ; local.tee $a` copy, no separate $r_inner slot). Slots:
        // outer mul $a=3 (the inner add writes here), $b=c=slot 2 (a direct param, no scratch), $r=4;
        // the inner add reuses $a=3 as its own $r and its a,b are direct params → no scratch of its own.
        // So only slots 3 and 4 are declared — 2 locals, down from 3 before the dest-threading.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.declared, vec![ValType::I64; 2]);
    }

    #[test]
    fn a_nested_strength_reduced_multiply_writes_the_operand_slot_directly() {
        // (def (f (: a Int64)) (+ (* a 2) 1)) — `(* a 2)` strength-reduces to `a << 1` and is the LHS
        // operand of the enclosing `+`. Instead of computing the shift into its OWN $r and copying that
        // into the add's $a (`local.get $r_inner ; local.tee $a`, plus a dead $r_inner slot), the shift is
        // emitted with `ResultDest::Slot($a)` so its `local.set` IS the store into the add's operand slot —
        // exactly like the nested checked `+`/`-`/`*` path. The add's RHS is the inline constant `1` (no
        // scratch). So the shift's own $r is the add's $a slot, and only that slot + the add's $r are
        // declared: 2 locals, down from 3 before the dest-threading (the eliminated copy freed a local).
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (+ (* a 2) 1)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.declared,
            vec![ValType::I64; 2],
            "the nested shift writes the add's operand slot directly — no extra $r_inner copy slot; got {:?}",
            f.code
        );
        // The shift is present (strength reduction fired) and there is NO `i64.mul`.
        assert!(
            f.code.contains(&Lir::I64Shl) && !f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
            "the `* 2` is a shift, not a mul; got {:?}",
            f.code
        );
        // No `local.get N ; local.tee M` handoff copy between the shift's result and the add's operand —
        // the shift writes the operand slot directly, so its result is consumed in place. (A `local.get`
        // immediately followed by `local.tee` was the copy the dest-threading removes.)
        let copy = f
            .code
            .windows(2)
            .any(|w| matches!(w, [Lir::LocalGet(_), Lir::LocalTee(_)]));
        assert!(
            !copy,
            "no get-then-tee handoff of the shift result into the add operand slot; got {:?}",
            f.code
        );
    }

    // ── value-heap H2d: Perceus — a kept heap binding constructs then DROPS ───────────────────────

    #[test]
    fn a_projection_only_tuple_folds_and_builds_no_heap() {
        // (def (f (: a Int64) (: b Int64)) (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) — `t` is ONLY
        // ever projected (never used as a whole value), so it does NOT need to exist on the heap: each
        // projection folds straight through to its element (the param), and the body is just `(+ a b)`.
        // No `arr-alloc`, no `box`/`arr-set`, no `drop` — a projection-only compound emits ZERO heap ops
        // (`should_keep_binding` does not keep a projection-only compound). The GENUINE heap-alloc →
        // escape → walk → drop (Perceus) path is exercised by the recursive-escape resource tests
        // (`a_recursive_runtime_tuple_escapes_to_the_host` + the `live-objects == 0` balance probe),
        // where the compound is returned WHOLE and must actually be built.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) \
               (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("arr-alloc")),
            "a projection-only tuple must not be built on the heap"
        );
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "nothing is built, so nothing is dropped"
        );
        // It is exactly the checked add of the two params — the same code `(+ a b)` emits directly.
        assert!(
            f.code.contains(&Lir::I64Add) && f.code.contains(&Lir::LocalGet(0)),
            "the body folds to `(+ a b)` over the params"
        );
    }

    #[test]
    fn a_scalar_let_binding_is_not_dropped() {
        // A scalar (`Int64`) `let` binding owns no heap cell, so NO drop is emitted for it — reclamation
        // is only for heap values. `(let ((s (+ a b))) (+ s s))` — `s` is a kept i64, never dropped.
        let ast = crate::testkit::parse(
            "(module m (def (g (: a Int64) (: b Int64)) \
               (let ((s (+ a b))) (+ s s))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "g");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "a scalar binding owns no heap cell and must not be dropped"
        );
    }

    // ── value-heap: Map.lookup / Set.contains BORROW the key — a BORROWED String key is NOT dropped ──

    #[test]
    fn a_borrowed_string_map_lookup_key_is_not_dropped() {
        // A `Map.lookup` whose KEY is a BORROWED String — here a `String` PARAMETER the caller owns —
        // must NOT be dropped after the borrowing lookup: `map-lookup` reads the key without consuming it,
        // and dropping the param's reference would free a value the caller still holds (a use-after-free).
        // This is the ownership face of the two-live-matched-String-payloads MISCOMPILE: a tree-walker
        // looking up a node's OWN key AND its child's key (both live sum-payload String projections) had
        // the second borrowed key freed under its owner, flipping its comparison and dropping a per-node
        // decision (a silent wrong count). No `box`/`bytes-compact` runs for a String key (it is already a
        // handle, and a borrowed String is a flat leaf), so the un-owned key must be left to its owner.
        // BOTH the map and the key are BORROWED params the caller owns — so this body must drop NEITHER
        // (`map-lookup` borrows both). Using a param MAP (not an inline `Map.insert`) isolates the borrowed
        // -key concern from the owned-temporary-map reclaim (a fresh inline map IS an owned temporary the
        // emit now correctly drops — see `an_owned_temporary_map_lookup_map_is_reclaimed`).
        let ast = crate::testkit::parse(
            "(module m (def (pv (: mm (Map String Int64)) (: op String)) \
               (match (Map.lookup mm op) \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "pv");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "a borrowed String lookup key AND a borrowed map param must not be dropped — \
             dropping either frees a value still live in the caller; got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::CallImport("map-lookup")),
            "the lookup must still emit"
        );
    }

    #[test]
    fn an_owned_temporary_map_lookup_map_is_reclaimed() {
        // The COLLECTION-operand reclaim: a `Map.lookup` whose MAP is a fresh OWNED TEMPORARY (built inline,
        // used once) must be dropped after the borrowing lookup, or it leaks. ⚠ the drop must come AFTER the
        // value is dup'd out (the Some arm) — not right after `map-lookup` (that would free the value the
        // val-slot still borrows → UAF). Here the key is a constant (also owned → also dropped), so we get
        // ≥2 drops (key + map). Pins that the owned-temporary map is reclaimed.
        let ast = crate::testkit::parse(
            "(module m (def (f (: d Int64)) \
               (match (Map.lookup (Map.insert (map) \"a\" 1) \"a\") \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        let drops = f
            .code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport("drop")))
            .count();
        assert!(
            drops >= 2,
            "an owned-temporary map (built inline) AND its owned constant key must both be dropped after \
             the borrowing lookup (≥2 drops); got {drops}: {:?}",
            f.code
        );
    }

    #[test]
    fn an_owned_string_map_lookup_key_is_dropped() {
        // The complement: a `Map.lookup` whose KEY is an OWNED temporary — a CONSTANT String literal,
        // which materializes a FRESH owned byte-leaf handle — MUST be dropped after the borrowing lookup,
        // or the leaf leaks. So exactly one `drop` (the owned key) is emitted. This pins that the ownership
        // gate did not over-correct into leaking every key.
        let ast = crate::testkit::parse(
            "(module m (def (f (: d Int64)) \
               (match (Map.lookup (Map.insert (map) \"a\" 1) \"a\") \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::CallImport("drop")),
            "an owned constant-String lookup key must be dropped after the borrowing lookup, or it \
             leaks; got: {:?}",
            f.code
        );
    }

    #[test]
    fn a_borrowed_string_set_contains_element_is_not_dropped() {
        // The `Set.contains` twin of `a_borrowed_string_map_lookup_key_is_not_dropped`: `set-contains`
        // BORROWS its element, so a BORROWED String element (a `String` param the caller owns) must NOT be
        // dropped after the membership probe — dropping it would free the caller's value.
        // BOTH the set and the element are BORROWED params — so drop NEITHER. Using a param SET (not an
        // inline `Set.of`) isolates the borrowed-element concern from the owned-temporary-set reclaim (a
        // fresh inline set IS an owned temporary the emit now correctly drops).
        let ast = crate::testkit::parse(
            "(module m (def (has (: s (Set String)) (: e String)) \
               (Set.contains s e)) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "has");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "a borrowed String set-contains element AND a borrowed set param must not be dropped; \
             got: {:?}",
            f.code
        );
        assert!(
            f.code.contains(&Lir::CallImport("set-contains")),
            "the membership probe must still emit"
        );
    }

    #[test]
    fn set_to_list_drops_its_baked_descriptor_after_the_borrowing_op() {
        // `Set.to-list` bakes a shape descriptor as an owned `Bytes` (`bytes-alloc`/`bytes-set`) and passes
        // it to `set-to-list`, which only BORROWS it (the runtime reads it as an inspector; see
        // `op_set_to_list` — "BORROWS `s` and `desc`"). So the emit MUST `drop` that owned descriptor
        // temporary after the op, or every `Set.to-list` call leaks the descriptor cell. Pin that a `drop`
        // FOLLOWS the op (past the desc `local.get`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: s (Set Int64))) (List.len (Set.to-list s))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        let to_list_at = f
            .code
            .iter()
            .position(|op| *op == Lir::CallImport("set-to-list"))
            .expect("the emit must call set-to-list");
        assert!(
            f.code[to_list_at + 1..].contains(&Lir::CallImport("drop")),
            "the baked descriptor Bytes is BORROWED by set-to-list, so a `drop` must follow the op to \
             reclaim the owned descriptor temporary; got: {:?}",
            f.code
        );
    }

    #[test]
    fn map_to_list_drops_its_baked_descriptor_after_the_borrowing_op() {
        // The map companion: `map-to-list` likewise BORROWS the baked descriptor, so the emit must drop it
        // after the op.
        let ast = crate::testkit::parse(
            "(module m (def (f (: m (Map Int64 Int64))) (List.len (Map.to-list m))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        let to_list_at = f
            .code
            .iter()
            .position(|op| *op == Lir::CallImport("map-to-list"))
            .expect("the emit must call map-to-list");
        assert!(
            f.code[to_list_at + 1..].contains(&Lir::CallImport("drop")),
            "the baked descriptor Bytes is BORROWED by map-to-list, so a `drop` must follow the op; \
             got: {:?}",
            f.code
        );
    }
}
