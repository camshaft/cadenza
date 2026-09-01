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
use crate::backend::common::diverge::{body_diverges, refined_frame_for_branch};
use crate::backend::wasm::lir::{BlockType, Lir, ValType, valtype_of};
use crate::core::Core;
// Backend-agnostic Core-IR analysis primitives, moved to the shared `core_analysis` module so the
// backend-independent Core optimization passes can reuse the exact same soundness-critical logic
// (frontier + heap-type classification) without duplication. Re-imported here so the Lir-level
// LICM/CSE realization's call sites are unchanged.
use crate::core_analysis::{
    collect_dominating_frontier, collect_node_refs, core_eq, core_hash_key, is_heap_type,
    licm_children, subtree_size,
};
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Ty};
use std::collections::{HashMap, HashSet};
use tracing::trace;

mod lift;
use lift::*;
mod reclaim;
pub use reclaim::core_child_ids;
use reclaim::*;
pub(crate) use reclaim::{EscapeTarget, param_escapes_body};
mod used_ops;
use used_ops::*;
mod arith;
use arith::*;
mod marshal;
use marshal::*;
mod dispatch;
use dispatch::*;
mod boxget;
use boxget::*;
pub use boxget::{LocalVar, SelectedFunc};
mod emit;
use emit::*;

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
    /// The key carries the FULL prefix STEPS, NOT just its length: a TUPLE-OF-TWO-SUMS match
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
    /// 05:18721 SURPLUS keep-alive sites: the SUBSET of `dup_sites` occurrences (`Core::LocalRef`/`Core::Param`)
    /// whose retain `dup` is PROVABLY REDUNDANT and may be skipped — the narrowed replacement for the too-broad
    /// `body_is_boundary_owned`-alone trial gate that caused 159 corpus UAFs. An occurrence of binder `b` is
    /// surplus iff: the body is BOUNDARY-OWNED (caller owns the scrutinee, no arm-end drop in the borrowing
    /// callee) AND `b` is a MatchList SCRUTINEE that is rest-mint-CONSUMED (`matchlist_scrutinee_consumed` — an
    /// arm rest-mints `(.. r)` over `b`, whose `vec-drop` consume has its OWN separate balancer, the emit.rs
    /// RestFrom preservation dup) AND `b` has NO consume OTHER than that RestFrom (`count_param_consumes` with
    /// `count_restfrom=false` == 0). The third conjunct is decisive: it keeps the dup LOAD-BEARING (unskipped)
    /// whenever `b` is also push/insert/escape/self-call-consumed and balanced ONLY by this dup — the 159 UAF
    /// class. Populated in `select_function_of`; read ONLY by `emit_binder_ref`. Empty unless boundary-owned +
    /// a rest-mint match with a borrow-only scrutinee.
    surplus_skippable_dups: HashSet<StructId>,
    /// hcz CAPTURE-ESCAPE retain sites (`collect_captured_escape_dup_sites`): the `Core::Captured` OCCURRENCE
    /// ids of a COMPOUND (heap) closure capture that ESCAPES the closure body via its sole read — a `dup` is
    /// emitted after the env-cell `arr-get` so the returned value owns an INDEPENDENT ref and the monolithic
    /// env-cell drop (unconditional for an Owned closure operand, select.rs — cascades to the capture) frees
    /// only the cell's copy → each ref frees exactly once (no double-release; hcz1/hcz2). GUARD (v-memory-
    /// safety-signed): marked ONLY for a capture read EXACTLY ONCE (that sole read is the escaping consume, so
    /// no borrow occurrence is over-dup'd). A MULTI-read escaping compound capture is left UNMARKED — a TRACKED
    /// RESIDUAL double-free (NOT leak-safe: the env-cell drop is unconditional and captures have no per-
    /// occurrence dup marking yet), surfaced by v-memory-safety's corpus-wide 0-trap sweep; its fix is the
    /// per-occurrence capture-escape marking (the `mark_binder_dups` analog for captures) or a decline of that
    /// shape. A DEDICATED set (disjoint from `dup_sites`) so the single-source-of-truth double-mark discipline
    /// holds. Empty for a non-closure body (no `Core::Captured`) → the fast path is untouched.
    captured_escape_dup_sites: HashSet<StructId>,
    /// (2) rope/slice-view SumExpect reclaim — the `Core::SumExpect` NODE ids whose extracted COMPOUND view
    /// payload is SCALAR-READ (consumed by exactly ONE `Bytes.at`) and does NOT escape, so the extraction is
    /// reclaimable: `compound_dupd` (the SumExpect emit) dup's the view at extract + drops the Some-shell, and
    /// `reclaim_bytes` (the sole consuming `Bytes.at` emit) drops the now-owned view after its len+get borrows.
    /// A DEDICATED set (NOT `dup_sites`) so reclaim_bytes fires the view-drop ONLY for THIS reason — never
    /// conflated with a `mark_binder_dups` (shared-read) or `collect_shell_reclaim_child_dups` (B1) mark on the
    /// same node id (the b2 double-mark class → double-free). Both `compound_dupd` and `reclaim_bytes` gate on
    /// THIS set (single source of truth for the dup-at-extract + view-drop lockstep), never re-derived.
    /// v-mem-safety co-verified (co-own of the rope/slice-view lever). Empty unless the scalar-extracted-view
    /// shape is present; the multi-`Bytes.at` (multi-borrow) case is NOT marked (would double-drop) → leaks.
    /// The VIEW-reclaim set (net -1: WE drop the view): the single consumer is a SCALAR-returning read WITH a
    /// view-drop hook (`Bytes.at`→reclaim_bytes / `String.scalar-len`→StrScalarLen reclaim). `compound_dupd`
    /// dups the view + drops the shell; the consumer's reclaim drops the now-owned view.
    sumexpect_view_reclaim: HashSet<StructId>,
    /// (2) SA2 — the SHELL-reclaim set (net 0: the CONSUMER owns the view, we free only the orphaned shell).
    /// `Core::SumExpect` node ids whose Owned SINGLE-heap-payload Some (a `String.at` / `Bytes.slice` char/
    /// bytes view producer) is consumed ONWARD by a single non-scalar-read consumer (a `Call`/op that takes
    /// the view). `compound_dupd` dups the view (+1) — which EXACTLY compensates the shell-drop's cascade (-1,
    /// a single Some→payload ref) → NET-0 on the view → its existing (correct) consumption is undisturbed, and
    /// only the orphaned shell is freed. NO view-drop on our side (the consumer owns it), so — unlike the
    /// VIEW set — reclaim_bytes/StrScalarLen do NOT fire for these. v-mem-safety's transparency reframe: net-0
    /// is sound for ANY consumer (consume OR borrow), so no call-convention predicate is needed. SINGLE-HEAP-
    /// PAYLOAD FENCE (load-bearing, v-mem-safety): only a Some with EXACTLY ONE heap payload — a multi-payload
    /// variant cascades >1 and dup==1 under-compensates → double-free; scoped to `String.at`/`Bytes.slice`
    /// (inherently single-view) for this increment (Option A). DISJOINT from `sumexpect_view_reclaim` by
    /// consumer-kind (scalar-read→VIEW, consumed-onward→SHELL, count>1/escape→neither) — the b2 exactly-one-of.
    sumexpect_shell_reclaim: HashSet<StructId>,
    /// Site A (self-loop-tail reclaim): wasm-local SLOTS of loop-carried params that, THIS loop iteration,
    /// are reassigned (`local.set`) with NO end-of-scope drop AND whose last emitted use is a consuming
    /// `vec-drop` tail-slice (PART 2 ordered it last). Their PRESERVATION dups (`emit_binder_ref` retain,
    /// the `RestFrom` step's dup) are SKIPPED: a borrow reads the live slot directly and the final
    /// `vec-drop` consumes+FBIP-reuses the sole ref (rc1→0). Populated by `emit_loop_iteration` around its
    /// arg emit, restored after — so the general emit stays default-dup (straight-line matches keep the
    /// preservation dup for their arm end-of-scope drop).
    loop_reassign_no_dup: HashSet<u32>,
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
    ///
    /// Keyed by `(root scrutinee id, path)` — NOT the path alone. The path is RELATIVE to each match's own
    /// scrutinee, so two matches that are BOTH live (an outer sum match whose arm body nests an inner sum
    /// match on a DIFFERENT scrutinee) share the same relative `[Payload]` path; keying by path alone let
    /// the inner match's entered-payload type OVERWRITE the outer's while the outer arm body still emitted,
    /// so an outer payload-binder walk resolved the WRONG variant → wrong heap accessor → a garbage handle →
    /// a runtime `trap_oob` (the large-`lower-ok` miscompile: outer `Node` `[Payload]` shadowed by an inner
    /// `Core`-result match). Scoping by the scrutinee id fences each match's records to its own scrutinee,
    /// exactly as the sibling `payload_prefix_slots` map is keyed `(scrutinee, path)`.
    sum_path_types: HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
    /// The ENCLOSING function's result valtype (`valtype_of(&ret)`), set ONCE at the function-body emit
    /// entry. Read in `emit_tail`'s `Core::Call` arm: a `return_call` returns the callee's result valtype
    /// DIRECTLY as this function's result, which is only valid when they MATCH. A recursive callee's
    /// full-width `i64` result tail-called from a function whose result is a NARROWER ascribed int (`UInt32`
    /// → `i32`) would emit `return_call` and ELIDE the `i32.wrap_i64` the ascription requires — invalid
    /// wasm (fuzzer 38551). When the callee's valtype differs from this, the tail call falls back to a
    /// non-tail `Call` + the width conversion + `Return`. `None` for a Unit/diverging (0-result) function.
    fn_ret_vt: Option<ValType>,
    /// NON-TAIL SPINE RECLAIM (v-mem-safety-signed-off): the PARAM binders proven OWNED + DEAD-AFTER a
    /// tail-position `MatchSum` — a heap param consumed ONLY by the match (`count_param_consumes == 0`, so
    /// the match holds its LAST owned ref) and NOT epilogue-dropped (`looped_owned_param_drops`). For such a
    /// scrutinee the tail-`MatchSum` shell-reclaim drops the param's SLOT (there is no stashed temp), freeing
    /// each recursive frame's un-reclaimed spine shell (e.g. `sum-nat`'s `Nat.S` cells — 1/cell leak → 0).
    /// A NARROW proven-owned exception to the global `heap_operand_ownership(Param) == Borrowed` default
    /// (select.rs:17542) — it does NOT change that load-bearing default. Computed ONCE in
    /// `select_function_of` (it has params/self_def/body); empty otherwise. Reuses the EXISTING
    /// `count_param_consumes` + `looped_owned_param_drops` machinery (no re-derived predicate).
    nontail_match_reclaim_binders: HashSet<StructId>,
    /// 05:18721 PART 1 (RestFrom preservation-dup skip-gate, read by the `emit.rs` `Core::SumPayload`
    /// `RestFrom` arm): whether the function body being emitted is BOUNDARY-OWNED (an export-entry or a
    /// lifted lambda) — i.e. the scrutinee is borrowed and the CALLER emits the single shell-drop_after (the
    /// caller-drop). Set by `select_function_of` (which computes `is_boundary_owned`) before the emit. In such
    /// a body a per-arm RestFrom `(.. r)` preservation dup is never balanced → leak, so it is a candidate for
    /// the skip-gate (together with the rest-borrow-only + no-sibling-after-vec-drop conjuncts).
    pub body_is_boundary_owned: bool,
    /// The function-body ROOT of the emit in progress — set by `select_function_of`. Lets the emit run a
    /// body-scoped escape query (`reclaim::restfrom_result_escapes`) for the RestFrom skip-gate's
    /// rest-borrow-only conjunct. `None` outside a `select_function_of` emit.
    pub fn_body: Option<StructId>,
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
/// `bytes-scalar-at(buf, scalar-index) -> u32` (#5516) — the `scalar-index`-th Unicode SCALAR codepoint of
/// the String's UTF-8 buffer, or `u32::MAX` (0xFFFFFFFF) for out-of-range / ill-formed. Borrows `buf` (does
/// not consume). The runtime does the UTF-8 walk, so `Core::StrScalarAt` emits a single call (unlike `StrAt`,
/// which walks the buffer in wasm).
const OP_BYTES_SCALAR_AT: &str = "bytes-scalar-at";
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
const OP_RATIONAL_NUM: &str = "rational-num";
const OP_RATIONAL_DEN: &str = "rational-den";
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
/// `str-nfc-normalize(h) -> handle` — canonicalize a runtime String to NFC (FINDING #23). Emitted ONLY at
/// String-typed construction sites (a `String.concat` result, a String Map/Set key, a symbol-intern) where
/// the value's identity requires its NFC-normalized form (collections-and-text.md L33-34/L53-54). CONSUMES
/// `h` (returns the same handle when already NFC — the ASCII/pre-composed common case, no alloc — else a
/// fresh normalized leaf with the original dropped). A raw Bytes / `str-from-bytes` decode NEVER calls it
/// (the decode-exemption, L90-94). Runtime op at WIT index 89.
const OP_STR_NFC_NORMALIZE: &str = "str-nfc-normalize";
/// `hash-blake3(bytes) -> handle` — the blake3 content hash (heap op 91). BORROWS the Bytes handle (an
/// inspector), returns a FRESH OWNED 32-byte Bytes leaf. Backs `Core::Blake3Of` (P3b runtime lowering).
const OP_HASH_BLAKE3: &str = "hash-blake3";
/// The value-heap `ast-print` op (heap index 92, appended after `hash-blake3`): renders a heap `Ast` handle
/// to canonical s-expr text (a fresh `String` leaf), guided by a baked disc descriptor. Backs `Core::AstPrint`.
const OP_AST_PRINT: &str = "ast-print";
/// The value-heap `ast-encode` op (heap index 93, appended after `ast-print`): serializes a heap `Ast` handle
/// to its canonical `cdzast` binary form (a fresh OWNED `Bytes` leaf) via the shared `cadenza-ast` codec,
/// guided by a baked 9-disc descriptor. Byte-identical to the compile-time `codec::encode` fold. Backs
/// `Core::AstEncode`.
const OP_AST_ENCODE: &str = "ast-encode";
/// `ast-decode(bytes-handle, discs) -> handle` — parse canonical `cdzast` `Bytes` back to a heap `Ast`
/// value guided by the SAME baked 9-disc descriptor as `ast-encode`, returning a fresh Ast handle or `0`
/// (`NULL_HANDLE`) on a parse failure. TOTAL (never traps). Backs `Core::AstDecode` (the emit wraps the
/// handle-or-0 as `(Result Ast e)`).
const OP_AST_DECODE: &str = "ast-decode";
/// `vec-concat(a, b) -> handle` — concatenate two lists into one.
const OP_VEC_CONCAT: &str = "vec-concat";
/// `vec-prepend(v, elem) -> handle` — a new list = `elem` then `v`'s elements (consumes both). The
/// dedicated front-growth twin of `vec-push`, backing `Core::ListPrepend` (replaces `concat(singleton, v)`).
const OP_VEC_PREPEND: &str = "vec-prepend";
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
/// `value-cmp(a, b, desc) -> s32` — the blessed THREE-WAY order over two compound heap values, guided by
/// the shape descriptor `desc` (baked as a Bytes constant, the same descriptor `value-encode` reads).
/// Returns -1/0/1 (Less/Equal/Greater) or 2 (non-orderable sentinel — never emitted-for, the compiler
/// declines ordering on a float/bytes/set/map leaf). BORROWS both operands (like `value-eq`), so an
/// owned-temporary operand is `drop`ped after the compare. The runtime `<`/`<=`/`>`/`>=` on two runtime
/// compounds the compiler could not fold.
const OP_VALUE_CMP: &str = "value-cmp";
/// `value-eq-shaped(a, b, desc) -> bool` — descriptor-guided STRUCTURAL equality over two compound heap
/// values, baked with the same Bytes descriptor `value-cmp`/`value-encode` use. The element-wise companion
/// of `value-eq`: exact for a LIST(-containing) compound (an RRB spine is element- but not shape-canonical)
/// and for a FLOAT/BYTES leaf a list carries (canonical byte form — nan==nan, -0.0≠+0.0 — which `value-cmp`
/// declines, a float having equality but no total order). BORROWS both operands (like `value-eq`/`value-cmp`),
/// so an owned-temporary operand is `drop`ped after the compare. The runtime `=` on a List<Float>/list-with-
/// float-leaf compound the compiler could not fold (and `value-eq`'s physical byte-walk would misread).
const OP_VALUE_EQ_SHAPED: &str = "value-eq-shaped";
/// `value-canonicalize(a, desc) -> handle` — the blessed CANONICAL form of a heap value of the type `desc`
/// describes, baked as a Bytes constant exactly as `value-cmp`/`value-encode` bake it. Emitted at a Map/Set
/// KEY site for a list-typed (or list-containing) key: a List is an RRB vector that is element-canonical but
/// NOT shape-canonical, so a concat-built and a push-built equal-element list key would hash into different
/// CHAMP slots (a false-miss violating `collections-and-text.md` §162 — a key's identity is construction-
/// independent). Rebuilds every list to its unique strict shape so the tagless byte-walk is exact. BORROWS
/// `a` + `desc`, returns a FRESH owned handle the emit drops after a borrowing key op (like a compacted rope
/// key). A malformed descriptor declines to an identity dup (total). See `value_canonicalize_shaped`.
const OP_VALUE_CANONICALIZE: &str = "value-canonicalize";
/// `value-encode(v, desc) -> handle` — render a runtime value `v` to its canonical binary-AST document,
/// guided by the shape descriptor `desc` (baked as a Bytes constant, the SAME descriptor `value-cmp`/
/// `value-encode` read). BORROWS `v` + `desc` (an inspector), returns a FRESH owned `Bytes` doc handle.
/// Backs `Core::ValueEncode` (R2): the in-fold `Value.encode` — unlike the resource-escape path it does
/// NOT copy the doc into the export retarea, it RETURNS the doc handle as the `Bytes` value. The owned-
/// temporary `desc` (and an owned-temporary `v`) are `drop`ped after the borrowing call.
const OP_VALUE_ENCODE: &str = "value-encode";
/// `value-decode(bytes, desc) -> handle` — the inverse: parse the binary-AST document `bytes` back into a
/// FRESH owned heap value of the type `desc` describes, or the NULL handle (`0`) on a shape/format mismatch
/// (never traps — mirrors `value-encode`'s malformed-desc decline). BORROWS `bytes` + `desc`. Backs
/// `Core::ValueDecode` (R2): the emit wraps the success handle into `Some` / the NULL signal into `None`
/// via `disc_some`/`disc_none` (the `∀a. Bytes → Option a` partial). See `op_value_decode`.
const OP_VALUE_DECODE: &str = "value-decode";
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

/// The emit-walk INSTRUCTION BUDGET (finding-24 sibling, the K^N DAG-serialized-as-tree explosion). The
/// Core IR is a compact DAG (a shared threaded-state subtree reached by many branch successors), but the
/// emit walk re-descends each shared `StructId` reference and re-emits its subtree, so a node reached many
/// times on the walk emits that many instruction (`Lir`) copies. The explosion driver is NOT the branch
/// count: it is the number of DISPATCHES ROUTING THROUGH the branching arm (each such dispatch re-expands
/// the arm body), multiplied by the per-branch state-rebuild width and any compound recomputed per branch.
/// A handler that both resumes AND advances a compound threaded state, re-expanded per dispatch, grows
/// super-linearly — and past a point the emitted wasm function body exceeds the engine's function-size
/// limit ("Code for function is too large" — an INVALID module the guest cannot load, though not a
/// miscompile). (Witness: the two-branch `pwm1` with 7-of-9 dispatches through its arm explodes, while the
/// three-branch `lap1` with only 3-of-9 through the arm stays valid — so branch count K is not the driver.)
///
/// This bound DECLINES cleanly (reject-not-miscompile) once the emitted-`Lir` count crosses it: the guard
/// at the top of `emit` trips mid-walk, so a run-away body declines rather than serializing the
/// multi-megabyte code section the loader rejects. The value separates the measured VALID high-water from
/// the INVALID explosion (`Lir` counts, measured on v-effects' 3-way-partition probe ladder). The largest
/// VALID emitted body is the `cbk1` circuit-breaker corpus case at ~416K `Lir` (the `sw4`/`sw5` window
/// cases ~364K, the `isolate-K3` probe ~301K = 593KB wasm, LARGE but loads + runs); the INVALID cases are
/// `dst1` at ~1.48M `Lir` (2.88MB wasm -> "Code for function is too large") and `dstC` at ~74M.
///
/// RE-CALIBRATED to the CRANELIFT ceiling (breaker dbc1, 2026-08-15). The two engine ceilings DIFFER: the
/// `wasm-tools` VALIDATOR accepts a much larger body than CRANELIFT (wasmtime's compiler, which `cdz run`
/// uses) will compile — cranelift's per-function limit is LOWER. `dbc1` (a hold-debouncer, 7 dispatches
/// through a 3-branch arm recomputing the hold compound) emits ~852K `Lir` = 1.67MB wasm: the validator
/// PASSES it (well under its cap) but cranelift REJECTS it "Code for function is too large" — a run-time
/// `invalid component` trap, NOT the clean decline this backstop must give. The old 1M was tuned to the
/// VALIDATOR ceiling, so it under-fenced cranelift (the mirror of the `cbk1` rust-budget regression, where a
/// budget tuned to the wrong ceiling was too LOW; here it was too HIGH). 600_000 sits in the wide EMPIRICAL
/// GAP between the largest VALID body (`cbk1` at ~416K `Lir` / ~815KB wasm, which cranelift COMPILES + runs)
/// and the smallest cranelift-REJECTED body (`dbc1` at ~852K): ~1.44x headroom over the valid high-water,
/// and it catches `dbc1`/`dst1`/`dstC`. CAVEAT: cranelift's true ceiling is on MACHINE code, not wasm bytes,
/// so no static wasm/`Lir` budget predicts it EXACTLY (a denser-branching shape could reject at fewer `Lir`);
/// this is the calibrated INTERIM backstop that turns the known escapes into clean declines. The durable
/// LINEAR fix — sharing-aware emit (emit a 2+-reached node once into a `Core::Let` slot) — collapses the
/// super-linear body so it never approaches either ceiling; routed to v-core-opt.
const EMIT_INSTRUCTION_BUDGET: usize = 600_000;
/// The emit-walk SCRATCH-LOCALS BUDGET — the SECOND axis of the finding-24-sibling explosion. The K^N
/// DAG-as-tree serialization blows up in TWO independent ways (the "two kinds" split from the original
/// finding-24 arc): (1) BODY SIZE — the emitted `Lir` count, bounded by `EMIT_INSTRUCTION_BUDGET`; and (2)
/// SCRATCH LOCALS — the running high-water `*high` of scratch slots a guarded op claims. A body can blow the
/// LOCALS cap while staying UNDER the instruction bound: `rps1` (the in-branch-compound-recompute face)
/// emits ~2.5MB but wasmparser rejects it "too many locals exceeds maximum" (wasm's ~50000 per-function
/// locals cap) — its per-branch recompute mints fresh scratch slots faster than instructions, so it slips
/// the instruction budget yet overruns the locals cap → an INVALID module. So the emit-walk needs a locals
/// budget too: decline when `*high` crosses this (well below the ~50000 engine cap, with headroom over any
/// valid function — scratch slots are REUSED across siblings so a legitimate body's high-water stays low).
/// Same reject-not-miscompile decline; the durable fix is (b) sharing-aware emit (a shared subtree binds
/// ONCE, so it claims its slots once instead of per-reference).
const EMIT_LOCALS_BUDGET: u32 = 40_000;
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

/// [`is_heap_type`], but CONSERVATIVE for the Perceus RETAIN/dup CANDIDATE decision: a type that still
/// contains a FREE VARIABLE (`Ty::Var` — an unsolved payload/binder type) also counts as heap here.
///
/// WHY (a UAF fix, found via v-patterns' slice-5 diagnostic): the retain-candidate collection reads
/// `type_of(binder)`, but `infer::type_of` DELIBERATELY does not memoize a free-var type (it recomputes so
/// the later A2 connected-solve can win). So a payload/binder whose type is FIRST DEMANDED while still a
/// `Ty::Var` — e.g. `(match acc ((Box.Full m) …))` where `acc` is momentarily `(Box ?0)` — reads as a `Var`,
/// which plain `is_heap_type` classifies NON-heap → the binder is NOT marked a retain candidate → NO `dup`
/// is emitted → a sum-payload BORROW consumed while a sibling re-extracts it is freed under the live alias
/// → USE-AFTER-FREE. The verdict was DEMAND-ORDER-sensitive (any pass that reorders the `type_of` demand
/// across the solve boundary — a peer's emit-time Db mutation did — could flip it into the UAF).
///
/// Treating a free-var type as a retain CANDIDATE removes that fragility STRUCTURALLY: a `Ty::Var` can only
/// become MORE concrete (heap or scalar) once solved, so marking it a candidate is LEAK-SAFE, never a UAF —
/// and it is only a CANDIDATE mark: the actual `dup`/`drop` EMISSION is independently gated on the CONCRETE
/// (by-emit-time ground) element/binder type (the `scalar_elem`/`get_op` arms, the `emit`-side drop gate),
/// so a `Var` that solves to a SCALAR never emits a heap `dup`/`drop` (rc-op on a scalar would be invalid) —
/// it just was a spurious candidate that emits nothing. Import-collection (`collect_used_ops_into`) uses this
/// too so the `dup`/`drop` ops are declared when a candidate does turn out heap (a declared-unused import is
/// harmless if it turns out scalar).
/// Emit the shared "copy `len` host bytes into a fresh value-heap `Bytes`" loop that a host-result lift uses
/// to reconstruct a `list<u8>` the host wrote into the guest's linear memory: `bytes-alloc(len) -> handle`,
/// then `for i in 0..len { handle = bytes-set(handle, i, mem8[ptr + i]) }`, leaving the Bytes handle in the
/// `handle` local. `len`/`ptr`/`handle`/`i` are caller-owned i32 scratch locals. Shared by the kv.get option
/// lift (`option<list<u8>>` Some-arm) and the kv.prefix-scan lift (each pair's key + value) — the LIR is
/// byte-identical, differing only in which scratch slots the caller allocates. Emits the SAME instruction
/// sequence both sites inlined before, so the emitted wasm is byte-for-byte unchanged (a pure dedup).
fn emit_host_bytes_to_value_heap(out: &mut Vec<Lir>, len: u32, ptr: u32, handle: u32, i: u32) {
    out.push(Lir::LocalGet(len));
    out.push(Lir::CallImport(OP_BYTES_ALLOC));
    out.push(Lir::LocalSet(handle));
    out.push(Lir::ConstI32(0));
    out.push(Lir::LocalSet(i));
    out.push(Lir::Block(BlockType::Empty));
    out.push(Lir::Loop(BlockType::Empty));
    out.push(Lir::LocalGet(i));
    out.push(Lir::LocalGet(len));
    out.push(Lir::I32GeU);
    out.push(Lir::BrIf(1));
    out.push(Lir::LocalGet(handle));
    out.push(Lir::LocalGet(i));
    out.push(Lir::LocalGet(ptr));
    out.push(Lir::LocalGet(i));
    out.push(Lir::I32Add);
    out.push(Lir::I32Load8U { offset: 0 });
    out.push(Lir::CallImport(OP_BYTES_SET));
    out.push(Lir::LocalSet(handle));
    out.push(Lir::LocalGet(i));
    out.push(Lir::ConstI32(1));
    out.push(Lir::I32Add);
    out.push(Lir::LocalSet(i));
    out.push(Lir::Br(0));
    out.push(Lir::End); // loop
    out.push(Lir::End); // block
}

/// An inert STUB function with the given parameter types and result type `ret` — its body is a single
/// zero of the result's machine type. Used for an UNREACHED lambda-lifted closure (a dead lift the
/// emitted code folds away and never calls): the stub keeps the function-index + type section consistent
/// with the funcref table's slot numbering without carrying the dead lambda's (possibly ill-formed) body.
/// It is never invoked (its table entry is omitted), so returning a zero is safe. `params` is the
/// `(binder, type)` list the real selection would use; only the value types matter here.
pub fn stub_function(params: &[(StructId, Ty)], ret: &Ty) -> SelectedFunc {
    let param_vts: Vec<ValType> = params.iter().filter_map(|(_, t)| valtype_of(t)).collect();
    // The stub body pushes ONE value of the result's machine slot to satisfy the functype — EXCEPT a
    // `Unit` result, which is a ZERO-RESULT functype (the serializer emits `0x60 <params> <>`): its body
    // must be EMPTY, pushing nothing, or the module is invalid ("values remaining on stack at end of
    // block"). A non-Unit result with no machine rep should not reach a lifted lambda (its result type was
    // checked at lift time); it defaults to an i32 zero — harmless in a never-called stub.
    let code = if matches!(ret, Ty::Unit) {
        Vec::new()
    } else {
        let zero = match valtype_of(ret) {
            Some(ValType::I64) => Lir::ConstI64(0),
            Some(ValType::F64) => Lir::F64ConstBits(0),
            _ => Lir::ConstI32(0),
        };
        vec![zero]
    };
    SelectedFunc {
        params: param_vts,
        ret: ret.clone(),
        code,
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
    // Also the wrapper-scrutinee shell-reclaim's consumed-child dups (must match the emit's set so the
    // `dup` import is present iff the emit dups a consumed shell child) — see `collect_shell_reclaim_child_dups`.
    collect_shell_reclaim_child_dups(db, id, &mut sites);
    // SumPayload-ESCAPE dups: mirror `select_function_of` so `OP_DUP` is imported iff the emit dups an
    // escaping boundary-owned-param payload (the snowflake lower UAF fix). Same set → import ⟺ emit.
    collect_sumpayload_escape_dup_sites(db, id, &mut sites);
    // Also the runtime row-op field-copy dups (breaker #45) — same set as the emit's `collect_row_op_field_dups`
    // so the `dup` import is present iff the emit dups a borrowed heap field before the operand's drop.
    collect_row_op_field_dups(db, id, &mut sites);
    // hcz capture-escape dups: mirror `select_function_of`'s `collect_captured_escape_dup_sites` so `OP_DUP`
    // is imported iff the emit `dup`s an escaping single-read compound capture. Same set as the emit → the
    // import is present exactly when a dup is emitted (empty for a body with no such capture).
    collect_captured_escape_dup_sites(db, id, &mut sites);
    if !sites.is_empty() {
        out.insert(OP_DUP);
    }
    // (2) rope/slice-view SumExpect reclaim: mirror `select_function_of`'s `collect_sumexpect_view_reclaim`
    // so the imports match the emit — a marked view means the SumExpect emit `dup`s it (+ `drop`s the Some
    // shell) and the sole `Bytes.at`'s `reclaim_bytes` `drop`s it, so import BOTH `dup` and `drop` iff any
    // view is marked. Exact (empty when the scalar-extracted-view shape is absent → no over-declare).
    let mut view_reclaim: HashSet<StructId> = HashSet::new();
    let mut shell_reclaim: HashSet<StructId> = HashSet::new();
    collect_sumexpect_view_reclaim(db, id, &mut view_reclaim, &mut shell_reclaim);
    // VIEW set: compound_dupd `dup`s + shell-`drop`s AND the consumer reclaim `drop`s the view → both ops.
    // SHELL set: compound_dupd `dup`s + shell-`drop`s only → both ops too (dup for the net-0 compensation,
    // drop for the shell). Either non-empty ⟹ import dup+drop (exact — empty when neither shape is present).
    if !view_reclaim.is_empty() || !shell_reclaim.is_empty() {
        out.insert(OP_DUP);
        out.insert(OP_DROP);
    }
    // MatchSum OWNED-VIEW shell reclaim (the `matchsum_view_shell_reclaim_ok` emit at the tail + non-tail
    // MatchSum sites): a `String.at`/`Bytes.slice` scrutinee whose whole-match payload-safety holds gets its
    // Some shell `drop`ed (post-match fall-through and/or before a loop back-edge). Import `drop` iff the
    // body has such a match — the precise import/emit companion (mirrors the SumExpect view block above; NO
    // dup, this reclaim only drops the shell). Purely Core-structural (no slots), so decidable here.
    if body_reclaims_view_shell(db, id) {
        out.insert(OP_DROP);
    }
    collect_used_ops_into(db, id, out);
    // NOTE: the owned-heap-param DROP epilogue (`select_body`, looped functions) also needs `drop` imported,
    // but only when it ACTUALLY fires (looping + a dead-at-exit invariant heap param) — which needs the
    // def's `self_def`/params, not available here. That precise import is added by `collect_module_used_ops`
    // (which has the def index) via `looped_owned_param_drops`, NOT here — importing `drop` for every
    // heap-param body would over-declare it (violating the drop-import-minimization discipline the
    // `str_at_does_not_over_declare_drop` test pins).
}

/// Whether `id`'s body contains a `MatchSum` the emit will VIEW-shell-reclaim (`matchsum_view_shell_reclaim_ok`
/// at the tail/non-tail MatchSum sites) — an owned-single-view (`String.at`/`Bytes.slice`) scrutinee whose
/// whole-match payload-safety holds. The import-side companion of that emit: `collect_used_ops` imports `drop`
/// iff this is true (precise, no over-declaration — a payload-CONSUMING arm fails `sum_shell_reclaim_payload_ok`
/// and is excluded). Purely Core-structural: an owned-single-view scrutinee is a `StrAt`/`BytesSlice` NODE (a
/// computed producer, always stashed into an I32 slot at emit — never a reusable handle), so the stashed-I32
/// gate holds by construction and needs no slot context. `never_diverges` mirrors the emit's `body_diverges`.
fn body_reclaims_view_shell(db: &mut Db, id: StructId) -> bool {
    fn go(db: &mut Db, id: StructId, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::MatchSum { scrutinee, root } = core_of(db, id) {
            let scrut_ty = type_of(db, scrutinee);
            let never_diverges = body_diverges(db, id);
            // Call the SAME gate the emit uses (single source of truth → exact import/emit agreement). A
            // StrAt/BytesSlice scrutinee is always a computed producer → stashed I32, so the stand-in
            // `Some((0, I32))` matches the emit's real stashed slot for the gate's purposes.
            if matchsum_view_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                Some((0, ValType::I32)),
                never_diverges,
                &root,
            ) {
                return true;
            }
        }
        core_child_ids(db, id).into_iter().any(|c| go(db, c, seen))
    }
    let mut seen = HashSet::new();
    go(db, id, &mut seen)
}

/// The parameter SLOTS the owned-heap-param drop epilogue (`select_body`) will reclaim at the loop exit for
/// the def whose body is `body`, params `params`, self index `self_def`. EMPTY for a non-looping def, a def
/// with no heap param, or one whose heap params all escape / vary across a back-edge. Shared by `select_body`
/// (to EMIT the drops) and `collect_module_used_ops` (to IMPORT `drop` iff non-empty) so the emit and the
/// import agree exactly — the precise companion of the dup-site import/emit agreement.
/// Whether the def with `body`/`params`/`self_def` will emit at least one owned-heap-param drop at its loop
/// exit — the import-side companion of [`looped_owned_param_drops`], so `collect_module_used_ops` imports
/// `drop` iff the epilogue actually emits one (precise, not the over-declaration the drop-minimization tests
/// forbid). `pub` for the module's op-collection.
pub fn def_drops_owned_param(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
) -> bool {
    !looped_owned_param_drops(db, body, params, self_def).is_empty()
}

/// Import-side companion of [`emit_loop_iteration`]'s §5 SUM-SPINE reclaim: whether this def's body has a
/// member tail-call whose arg is a self-consuming `Payload` extraction of a loop-param it is stored back
/// into (the `depth-tail` spine-walk). When it does, the emit adds a `dup` (retain the carried payload) +
/// a `drop` (free the old shell) per iteration, so `collect_module_used_ops` must import BOTH — precise
/// import/emit agreement (mirrors [`def_drops_owned_param`]). Re-derives the loop context + param slots
/// exactly as [`looped_owned_param_drops`].
pub fn def_sum_spine_reclaims(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
) -> bool {
    let Some(self_d) = self_def else {
        return false;
    };
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_slots: Vec<u32> = Vec::new();
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return false;
        }
        let slot = param_slots.len() as u32;
        slot_of.insert(*binder, slot);
        param_slots.push(slot);
    }
    if param_slots.is_empty() {
        return false;
    }
    let members = mutual_loop_group(db, self_d);
    if members.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    sum_spine_reclaim_in_body(db, body, &members, &param_slots, &slot_of, &mut seen)
}

/// Walk `id` for a member `Call` (a tail-loop back-edge) carrying a self-consuming `Payload` arg — the
/// same predicate [`emit_loop_iteration`]'s `is_sumpayload_consume` applies. Used ONLY for the dup/drop
/// import decision; the emit re-checks per-call. `seen` breaks DAG re-walk.
fn sum_spine_reclaim_in_body(
    db: &mut Db,
    id: StructId,
    members: &[usize],
    param_slots: &[u32],
    slot_of: &HashMap<StructId, u32>,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if let Core::Call { callee, args } = core_of(db, id)
        && members.contains(&callee)
    {
        for (i, &arg) in args.iter().enumerate() {
            if i >= param_slots.len() {
                continue;
            }
            let is_self_payload = matches!(core_of(db, arg), Core::SumPayload { scrutinee, ref path }
                if matches!(path.last(), Some(crate::core::PathStep::Payload))
                    && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder }
                        if slot_of.get(&binder) == Some(&param_slots[i])));
            if is_self_payload
                && let Core::SumPayload { scrutinee, .. } = core_of(db, arg)
                && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
            {
                let mut cseen = HashSet::new();
                let mut total = 0usize;
                for &a in args.iter() {
                    count_param_consumes(db, a, binder, &mut cseen, &mut total, true);
                }
                if total == 0 {
                    return true;
                }
            }
        }
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| sum_spine_reclaim_in_body(db, c, members, param_slots, slot_of, seen))
}

/// Whether def `self_def`'s body will emit the BORROWED-ACCUMULATOR reclaim drop (`drop_old_borrowed` in
/// [`emit_loop_iteration`]) for some loop-carried param — the import-side companion of that emit, so
/// [`collect_module_used_ops`] declares `drop` iff the emit actually reclaims a rebound accumulator (precise
/// import/emit agreement: an under-declaration would leave the emit's `CallImport(OP_DROP)` pointing at an
/// UNRESOLVED op index = an invalid module, the `str_at_does_not_over_declare_drop`-class bug in reverse).
/// Mirrors the drop_old_borrowed gate: a SINGLE-MEMBER self-loop with a member tail-call whose arg `i` (stored
/// into heap param slot `i`) PROVABLY produces a FRESH cell ([`reclaim::rebind_produces_fresh`]) and does NOT
/// consume the old accumulator (the escape guard `!binding_escapes` over EVERY arg). The three emit exclusions
/// (is_identity / RestFrom-consume / SumPayload-consume) are AUTOMATICALLY false when `rebind_produces_fresh`
/// holds — a fresh product ctor / numeric op is never a bare `Param` nor a `SumPayload` — so they need no
/// separate mirror. Arg↔slot alignment follows [`def_sum_spine_reclaims`]'s convention (arg `i` ↔ the i-th
/// non-Unit param slot).
pub fn def_rebinds_fresh_accumulator(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
) -> bool {
    let Some(self_d) = self_def else {
        return false;
    };
    let mut param_slots: Vec<u32> = Vec::new();
    let mut slot_binders: Vec<StructId> = Vec::new();
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return false;
        }
        param_slots.push(param_slots.len() as u32);
        slot_binders.push(*binder);
    }
    if param_slots.is_empty() {
        return false;
    }
    // drop_old_borrowed is SINGLE-MEMBER only (a mutual loop's cross-arm classification is deferred to the
    // leak-not-double-free side, so no drop fires there → nothing to declare).
    let members = mutual_loop_group(db, self_d);
    if members.len() != 1 {
        return false;
    }
    let mut seen = HashSet::new();
    rebinds_fresh_accumulator_in_body(db, body, &members, &param_slots, &slot_binders, &mut seen)
}

/// Walk `id` for a member `Call` (a tail-loop back-edge) whose arg `i` triggers the borrowed-accumulator drop
/// — the same gate [`emit_loop_iteration`]'s `drop_old_borrowed` applies (see [`def_rebinds_fresh_accumulator`]).
/// `seen` breaks DAG re-walk.
fn rebinds_fresh_accumulator_in_body(
    db: &mut Db,
    id: StructId,
    members: &[usize],
    param_slots: &[u32],
    slot_binders: &[StructId],
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if let Core::Call { callee, args } = core_of(db, id)
        && members.contains(&callee)
    {
        for i in 0..args.len() {
            if i >= param_slots.len() {
                continue;
            }
            let binder = slot_binders[i];
            if !is_heap_type(&type_of(db, binder)) {
                continue;
            }
            if !rebind_produces_fresh(db, args[i]) {
                continue;
            }
            if !args.iter().any(|&a| binding_escapes(db, a, binder, false)) {
                return true;
            }
        }
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| rebinds_fresh_accumulator_in_body(db, c, members, param_slots, slot_binders, seen))
}

fn looped_owned_param_drops(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
) -> Vec<u32> {
    let Some(self_d) = self_def else {
        return Vec::new();
    };
    // Re-derive the param slot assignment exactly as `select_function_of` does: represented params take
    // dense slots `0..n` in order, a Unit param (zero-width) is ELIDED (occupies no slot). This must match
    // the emit's `slot_of`/`param_slots` so the drop targets the right local.
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_slots: Vec<u32> = Vec::new();
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return Vec::new(); // a param with no machine rep → this def won't select; no drops.
        }
        let slot = param_slots.len() as u32;
        slot_of.insert(*binder, slot);
        param_slots.push(slot);
    }
    let slot_of = &slot_of;
    let param_slots = &param_slots[..];
    let loop_members: Vec<usize> = if param_slots.is_empty() {
        Vec::new()
    } else {
        mutual_loop_group(db, self_d)
    };
    if loop_members.is_empty() {
        return Vec::new(); // not a looping function → the non-tail `emit` path handles dead-binding drops.
    }
    // NARROW: only PLAIN SELF-recursion (a single-member loop group). A MUTUAL group shares one set of
    // parameter slots across members whose bodies are emitted inline under a dispatch — a heap param is
    // carried BETWEEN members, and a partner member may CONSUME it (a non-tail call), so `tree` in a
    // `read-do-next`/`read-do-form` mutual pair is NOT owned at the group's exit even though each member
    // passes it identity on its own back-edge. Jointly analyzing every member's body is the general pass's
    // job; the narrow gate declines the whole mutual case (leak, never a double-free). The witnessed leak
    // shape (`walk`) is single-member self-recursion, so it is unaffected.
    if loop_members.len() != 1 {
        return Vec::new();
    }
    // Params identity-passed on EVERY back-edge (invariant) — a varying heap param is left to leak (a single
    // exit drop would miss the per-iteration re-boxed values).
    let mut invariant: std::collections::HashSet<StructId> =
        params.iter().map(|(b, _)| *b).collect();
    invalidate_varying_params(
        db,
        body,
        param_slots,
        slot_of,
        &loop_members,
        self_d,
        &mut invariant,
        params,
    );
    let mut drops = Vec::new();
    for (binder, ty) in params.iter() {
        if !is_heap_type(ty) {
            continue;
        }
        let Some(&slot) = slot_of.get(binder) else {
            continue;
        };
        if !invariant.contains(binder) {
            continue; // varying across a back-edge — leave it (a single exit drop would be wrong).
        }
        if !param_only_borrowed_or_backedge(db, body, *binder, &loop_members, param_slots, slot_of)
        {
            continue; // not provably (borrow + tail-back-edge) only → conservatively leave it (default-deny).
        }
        drops.push(slot);
    }
    drops
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

/// Coalesce a selected function's non-interfering DECLARED local slots in place (see
/// [`crate::backend::wasm::coalesce`]): reuses dead slots so the declared-local count and every
/// `local.{get,set,tee}` index shrink to smaller LEB encodings. Applied to EVERY selected body (the
/// win is universal but is largest on the effects-lowering local-slot BLOWUP — `glb1` emits ~18.7k
/// mostly single-use continuation temps of which ~7 are ever simultaneously live). It rewrites
/// `f.code`'s local ops, `f.declared`, AND the DWARF slot references in `f.locals`/`f.scopes` through
/// one remap, so the emitted code and its debug info stay consistent.
///
/// Two guards keep it correct:
/// - **Loops:** the flat-span interference model under-approximates liveness across a `loop` back-edge
///   (a value read early in a loop body is live for the whole loop), so we SKIP any function that
///   contains a `loop`. Sound — skipping only forgoes the optimization. (A loop-aware span extension
///   is a later slice.)
/// - **Debug-named locals:** a `let`-binding / match-binder that a DWARF DIE points at is PINNED — it
///   keeps a distinct slot, so a debugger never reads another variable's value within its scope.
fn coalesce_func(f: &mut SelectedFunc, emit_debug: bool) {
    // Coalescing is sound across ALL control flow — the interference graph is built from precise
    // backward liveness iterated to a fixpoint over the structured CFG (loop back-edges included), so
    // a loop-carried declared local is correctly kept live across its back-edge (see the `coalesce`
    // module doc). No loop-skip guard is needed.
    let nparams = f.params.len() as u32;
    // DECLARED slots a DWARF DIE references (let-binding locals + match-binder scopes) are PINNED so
    // they keep distinct, correctly located slots — but ONLY when this emit actually produces DWARF
    // (`emit_debug`, from the target). A plain `wasm` emit has no DWARF consumer, so pinning would only
    // block coalescing (the effects-lowering blowup pins thousands of continuation temps otherwise);
    // there we leave `pinned` empty and coalesce every non-interfering slot. Param debug locals are
    // slots < nparams — already fixed, so they never need a pin.
    let mut pinned: HashSet<u32> = HashSet::new();
    if emit_debug {
        for lv in &f.locals {
            if lv.slot >= nparams {
                pinned.insert(lv.slot);
            }
        }
        for sc in &f.scopes {
            for v in &sc.vars {
                if v.slot >= nparams {
                    pinned.insert(v.slot);
                }
            }
        }
    }
    // Reuse dead-param slots only when NOT emitting DWARF (a param's scalar DIE must not share a slot
    // with a re-homed local) — same gate as pinning. The plain-`wasm` target (shipped + gap-sweep)
    // gets the extra param-heavy coalescing; a debug build keeps params fixed.
    let (remap, new_declared) = crate::backend::wasm::coalesce::coalesce_locals(
        &f.params,
        &f.declared,
        &f.code,
        &pinned,
        !emit_debug,
    );
    for op in &mut f.code {
        match op {
            Lir::LocalGet(s) | Lir::LocalSet(s) | Lir::LocalTee(s) => *s = remap[*s as usize],
            _ => {}
        }
    }
    f.declared = new_declared;
    for lv in &mut f.locals {
        lv.slot = remap[lv.slot as usize];
    }
    for sc in &mut f.scopes {
        for v in &mut sc.vars {
            v.slot = remap[v.slot as usize];
        }
    }
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
    // The function's result valtype — read in `emit_tail`'s tail-`Call` arm to detect a `return_call`
    // whose callee result valtype differs (a narrowing/widening ascription the tail call cannot carry).
    code.fn_ret_vt = valtype_of(&ret);
    // Perceus RETAIN placement (soundness): find every occurrence that CONSUMES a heap binding (a param or
    // a nested `let`) while that binding has a LATER live use, and record it so the emit `dup`s it. Without
    // this a value consumed by `List.push`/`Map.insert`/… in one operand and read again in a later operand
    // (or shared across two recursive-call operands) is mutated in place by the consuming op — a silent
    // wrong value. Computed ONCE here over all heap binders; the set is empty for the common single-use
    // body, so the FBIP fast path is unchanged. (See `collect_dup_sites`.)
    // NON-TAIL SPINE RECLAIM precompute (v-mem-safety-signed-off) — computed FIRST because BOTH the dup-pass
    // (`collect_shell_reclaim_child_dups`, which must dup the consumed spine payload for a reclaimed param
    // shell) AND the tail-MatchSum emit (which drops the param slot) gate on this set: the heap params proven
    // OWNED + DEAD-AFTER a tail-position MatchSum — consumed ONLY by the match (count_param_consumes == 0, so
    // the match holds the LAST owned ref) and NOT epilogue-dropped (looped_owned_param_drops). REUSES
    // count_param_consumes + looped_owned_param_drops (no re-derived predicate, per v-mem-safety). A NARROW
    // proven-owned exception to the heap_operand_ownership(Param)==Borrowed default — that default is intact
    // everywhere else. The per-match payload-safety (consume-only) + !cont_rematches gates are checked at the
    // reclaim/dup sites; dup ⊇ drop (the dup-pass fires for any such match, the drop only at a tail match →
    // every drop has its dup = no double-free; an extra dup at a rare non-tail match is a leak, never a UAF).
    // CALLEE-OWNED gate (v-mem-safety's exclusive-transfer-reachability spec, cheap-marker form): the
    // non-tail spine reclaim DROPS a param shell, so it is sound ONLY when the param is CALLEE-OWNED (the
    // callee reclaims), NEVER caller/boundary-owned (the caller built + drop_afters it → the callee BORROWS,
    // heap_operand_ownership==Borrowed is CORRECT). count_param_consumes==0 proves DEAD-AFTER but NOT
    // ownership — a boundary-built param is dead-after yet caller-owned → reclaiming DOUBLE-FREES (40 corpus
    // traps). v-mem-safety: the boundary conventions are a CLOSED set of TWO — (1) EXPORT-ENTRY params
    // (try_bare_entry_param_component builds + drop_afters the cell) and (2) CLOSURE-ARG params (a lifted
    // lambda's params, built + drop_after'd at the direct-call boundary). Both markers are CHEAP: an export
    // body is in layout.exports; a closure body is in db.lifted. Excluding BOTH is EXHAUSTIVE for the
    // double-free (trap) class (a closed set, not whack-a-mole) and a clean partition (a top-level def is
    // never a lifted lambda). Internal callee-owned recursive defs (sum-nat) are neither → reclaimed.
    let is_boundary_owned =
        layout.exports.iter().any(|e| e.body == body) || db.lifted.iter().any(|l| l.body == body);
    // 05:18721 PART 1: expose the boundary-owned flag + body root to the emit so the RestFrom preservation-dup
    // skip-gate (emit.rs `Core::SumPayload` RestFrom arm) can read them (v-wasm-opt owns that gate).
    code.body_is_boundary_owned = is_boundary_owned;
    code.fn_body = Some(body);
    let nontail_reclaim: HashSet<StructId> = if is_boundary_owned {
        HashSet::new()
    } else {
        let epilogue_dropped: HashSet<u32> = looped_owned_param_drops(db, body, params, self_def)
            .into_iter()
            .collect();
        let mut set = HashSet::new();
        let mut slot = 0u32;
        for (binder, ty) in params.iter() {
            if matches!(ty.strip_nominal(), Ty::Unit) {
                continue;
            }
            let this_slot = slot;
            slot += 1;
            if !is_heap_type(ty) {
                continue;
            }
            if epilogue_dropped.contains(&this_slot) {
                continue; // already reclaimed at the fn-exit epilogue — a match drop would double-free.
            }
            // SOLE-CONSUME (gate b): count_param_consumes counts CONSUMING uses (RestFrom / consume-ops /
            // escapes / direct-ref call args) but NOT a match's Payload extraction or a scrutinee borrow. == 0
            // ⟹ the param is never consumed elsewhere, so the match reading it holds the LAST owned ref → its
            // shell is dead after the (tail) match. A post-match CONSUME (return/escape/consuming-call of the
            // original ref) makes this > 0 → excluded (the param-reused-after control).
            let mut seen = HashSet::new();
            let mut total = 0usize;
            count_param_consumes(db, body, *binder, &mut seen, &mut total, true);
            if total == 0 {
                set.insert(*binder);
            }
        }
        set
    };
    {
        let mut heap_binders: Vec<StructId> = Vec::new();
        collect_retain_candidate_binders(db, body, &mut heap_binders);
        collect_dup_sites(db, body, &heap_binders, &mut code.dup_sites);
        // The wrapper-scrutinee shell-reclaim's consumed-child dups: for each MatchSum over an owned
        // compound boxed-sum whose shell the emit will deep-drop, `dup` each consuming scrutinee-child
        // extraction so the drop does not double-free a moved-out child. Computed here (upfront) so the
        // emit's child-dup + the `dup` import agree. Also fires (self-contained, relaxed) for a NON-TAIL
        // SPINE param scrutinee — dups the consumed spine payload so the param-slot shell-drop nets correctly.
        collect_shell_reclaim_child_dups(db, body, &mut code.dup_sites);
        // SumPayload-ESCAPE dups (boundary-owned twin of collect_captured_escape_dup_sites): in a LIFTED body
        // a payload of a boundary-owned param that ESCAPES via a result ctor is dup'd so the caller's boundary
        // drop_after of the arg does not free the still-referenced escaped payload (snowflake lower UAF). Into
        // dup_sites so the SumPayload emit's existing child-dup fires; import agrees via the same call below.
        collect_sumpayload_escape_dup_sites(db, body, &mut code.dup_sites);
        // The runtime row-op field-copy dups (breaker #45): a heap-handle field projected off a
        // materialize-`Let` row-op operand must be `dup`'d before the operand's drop, else the built record
        // holds a dangling field (a borrow outliving the operand's owned-node drop). Computed here (upfront)
        // so the emit's child-`dup` + the `dup` import agree. Empty for scalar-only / fresh-record row ops.
        collect_row_op_field_dups(db, body, &mut code.dup_sites);
        // hcz capture-escape dups: a compound capture read once + escaping needs a dup at its `Core::Captured`
        // read so the returned ref is independent of the monolithic env-cell drop (else double-free). A
        // DEDICATED set (not `dup_sites`) — the emit's `Core::Captured` arm gates on it. Empty for a body with
        // no escaping single-read compound capture (every non-closure body, and borrow-only captures).
        collect_captured_escape_dup_sites(db, body, &mut code.captured_escape_dup_sites);
        // 05:18721 SURPLUS GATE — DISABLED (regression fix, bisect #7255): the surplus analysis is UNSOUND
        // for a recursive rest-pattern list-equality (`match xs { [x, .. xr] => … a-eq(x, y) … lst-eq(xr, yr) }`,
        // e.g. choreography `a-list-eq`/`a-eq`, NOT in the `--guarded-all` corpus). There the head `x` is a
        // `vec-get` BORROW aliasing the scrutinee's storage and is used (in `a-eq(x, y)`) AFTER the RestFrom
        // `vec-drop` consumes the scrutinee; so the scrutinee keep-alive is LOAD-BEARING for that borrowed
        // co-element — but conjunct 2 (`count_param_consumes(b, count_restfrom=false) == 0`) misses it because
        // the element read is a BORROW, not a consume. Skipping the keep-alive frees the scrutinee under the
        // still-live element borrow → the later read hits a freed cell → a wrong sum-disc → a `wasm unreachable`
        // trap (`cdz test choreography` gen-is-deterministic; bisect isolated it to THIS gate: gate-ON traps,
        // gate-OFF passes). Leaving `surplus_skippable_dups` EMPTY restores the pre-gate emit_binder_ref
        // (identical to the caller-drop-ALONE state v-memory-safety proved corpus-clean at `--guarded-all`
        // 0-UAF); 05:18721 reverts to its known-leak baseline (its pin was never flipped → no corpus
        // regression). RE-ENABLE only with a sound narrowing that ALSO excludes a scrutinee whose borrowed
        // `vec-get` co-element outlives the RestFrom rest-mint — co-design with v-memory-safety (surplus
        // soundness) + verify on the choreography suite, not just the corpus.
    }
    // (2) rope/slice-view: partition the SumExpect-extracted single-view Somes (String.at/Bytes.slice) into
    // the VIEW set (scalar-read-dead single consumer → we dup+shell-drop+view-drop, net -1) and the SHELL set
    // (consumed-onward single consumer → dup+shell-drop only, net-0, consumer owns the view). Dedicated sets
    // (disjoint from dup_sites AND each other by consumer-kind) that compound_dupd (both) + reclaim_bytes/
    // StrScalarLen (VIEW only) consult, in lockstep.
    collect_sumexpect_view_reclaim(
        db,
        body,
        &mut code.sumexpect_view_reclaim,
        &mut code.sumexpect_shell_reclaim,
    );
    code.nontail_match_reclaim_binders = nontail_reclaim;
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
        scrut_shell_reclaim: None,
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
    // OWNED-HEAP-PARAM DROP EPILOGUE (recursion-param unwind leak). Under callee-owns-args this frame OWNS
    // each heap param; the non-tail `emit` path reclaims a dead `let` binding after the body, but a LOOPED
    // body has no such site — so an owned heap param carried across iterations and consumed only at the base
    // case (a borrowed-heap-sum recursion param `walk(n, w)` whose base `match w` only BORROWS it) is never
    // dropped and LEAKS one cell. Reclaim it HERE, after the body/loop leaves the result on the stack: the
    // runtime `drop` takes the handle as a call ARG (pushed immediately before) and returns nothing, so
    // `local.get slot; call drop` reclaims the param WITHOUT disturbing the result beneath it (exactly the
    // `Core::Let` drop shape). Gated conservatively so it NEVER double-frees:
    //   (a) LOOPED functions only — the non-loop path already emits the dead-binding drops via `emit`.
    //   (b) A HEAP param whose slot ref is DEAD at exit per `param_escapes_non_backedge` (the loop-aware
    //       escape: escapes only into IDENTITY member-tail-call args = the back-edge, not the result / a
    //       constructor / a non-member call / a re-boxed member arg). A param that flows out is not dropped.
    //   (c) INVARIANT across every back-edge (identity-passed on all member calls) — so its slot holds the
    //       SAME handle throughout and a single exit drop is correct. A VARYING heap param (re-boxed each
    //       iteration) would need a per-back-edge drop of the OLD value; that is out of scope, so such a
    //       param is conservatively LEFT (not dropped) — a leak, never a double-free.
    // (b)+(c) together are sound: identity-carried (c) means the exit slot value is the original owned param
    // handle, and dead-at-exit (b) means nothing else reclaims or transfers it — so this is its sole owner.
    for slot in looped_owned_param_drops(db, body, params, self_def) {
        code.push(Lir::LocalGet(slot));
        code.push(Lir::CallImport(OP_DROP));
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
    let mut f = SelectedFunc {
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
    };
    // Reuse non-interfering declared local slots (shrinks the local-decl count + `local.*` index widths;
    // largest win on the effects local-slot blowup). Rewrites body + declared + debug slot refs in place.
    // Pins debug-named slots only when this emit produces DWARF (`db.emit_debug`).
    coalesce_func(&mut f, db.emit_debug);
    Ok(f)
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
        // MULTI-VALUE-UPGRADE back-edge: `(let ((t (member-call …))) (tuple (. t 0) …))` iterates the loop
        // via the bound self-call (see `multivalue_repackage_tail_call` + the `emit_tail` `Core::Let` arm),
        // so its args drive the SAME per-param varying analysis a plain `Core::Call` back-edge does. Without
        // this the varying counter (e.g. `n-1`) is misread as invariant and LICM wrongly hoists the loop
        // condition out — an infinite loop. Handle it BEFORE the generic `Core::Let` body recursion below.
        Core::Let { .. }
            if multivalue_repackage_tail_call(db, id)
                .map(|c| matches!(core_of(db, c), Core::Call { callee, .. } if members.contains(&callee)))
                .unwrap_or(false) =>
        {
            let call = multivalue_repackage_tail_call(db, id).unwrap();
            if let Core::Call { args, .. } = core_of(db, call) {
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
        }
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

/// NARROW, provable-safety (default-DENY) gate for the owned-heap-param loop-exit drop: whether EVERY
/// occurrence of heap PARAMETER `binder` in `id` is either (1) a BORROW (a direct `Param` operand of a
/// match-dispatch / projection / len / sum-payload read — read without consuming) or (2) the loop
/// back-edge (an identity arg to a MEMBER tail-call, which `emit_loop_iteration` turns into an identity
/// slot move). Returns `false` (⇒ do NOT drop, conservatively leak) at the FIRST occurrence that is
/// anything else, and for ANY node kind this walk does not explicitly whitelist.
///
/// DEFAULT-DENY is the point. An earlier "absence-of-escape" gate (delegate uncovered nodes to
/// `binding_escapes`) OVER-FIRED across 3 self-host rounds: "not proven to escape" defaulted to droppable
/// over nodes it didn't model (a non-tail-consumed `tree`, a `MatchSum` sum-payload arm, a threaded-mutated
/// `store`), and dropping a param whose ownership was transferred out double-freed → wasm `unreachable`.
/// A whitelist that can only UNDER-drop (a missed borrow ⇒ a leak, never a double-free) is sound by
/// construction for a UAF-critical reclaim. This is deliberately NARROW: it fires for the witnessed
/// self-recursive-loop shape (a heap param used SOLELY as a base-case-match borrow + the tail-identity
/// back-edge) and declines everything else. The general owned-heap-param pass (the default-deny whitelist
/// extended to model every consuming node precisely) is a documented follow-up, not landed here.
fn param_only_borrowed_or_backedge(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
) -> bool {
    param_only_borrowed_or_backedge_rec(db, id, binder, members, param_slots, slots, false)
}

/// The worker, with a `borrowed` flag: `true` iff THIS occurrence is reached through a BORROW position (a
/// projection / len / sum-payload read / match-dispatch scrutinee), where a direct `Param(binder)` is a
/// pure read (OK); `false` in a CONSUME/result position, where a direct `Param(binder)` is an ownership
/// transfer OUT (deny). Mirrors `binding_escapes`'s `tail_borrowed` threading.
fn param_only_borrowed_or_backedge_rec(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    borrowed: bool,
) -> bool {
    // Fast path: a subtree that does not reference `binder` at all is trivially fine (nothing to consume).
    if !occurs_in(db, id, binder) {
        return true;
    }
    let recur = |db: &mut Db, c: StructId, borrowed: bool| {
        param_only_borrowed_or_backedge_rec(db, c, binder, members, param_slots, slots, borrowed)
    };
    match core_of(db, id) {
        // A direct reference to the param: OK iff this occurrence is in a BORROW position (read, not
        // consumed). In a consume/result position it transfers ownership out → deny.
        Core::Param { binder: b } => b != binder || borrowed,
        // A member TAIL-call back-edge: every arg that is `binder`'s identity pass-through re-establishes
        // the slot (fine); every OTHER arg must not reference `binder` (a re-boxed `(Mk w)` / non-identity
        // `w` CONSUMES → deny).
        Core::Call { callee, args } if members.contains(&callee) => {
            args.iter().enumerate().all(|(i, &arg)| {
                let is_identity = i < param_slots.len()
                    && matches!(core_of(db, arg), Core::Param { binder: b }
                        if b == binder && slots.get(&binder) == Some(&param_slots[i]));
                is_identity || !occurs_in(db, arg, binder)
            })
        }
        // BORROW ops: their heap operand is read without consuming → recurse it with `borrowed = true` (a
        // direct `Param` operand is then a pure borrow; a nested `SumPayload{Param}` / borrow chain threads
        // the flag). Other fields (a `Proj` index, a slice bound) are scalars that don't hold `binder`.
        Core::Proj { operand, .. }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand }
        | Core::BigIntToI64 { operand }
        | Core::CharToInt { operand }
        | Core::IntToCharChecked { operand, .. }
        | Core::RationalNum { operand }
        | Core::RationalDen { operand } => recur(db, operand, true),
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            recur(db, scrutinee, true)
        }
        // Match dispatch BORROWS its scrutinee (sum-disc / list-len read) → scrutinee recursed borrowed;
        // each arm body is a RESULT position → recursed unborrowed.
        Core::Match { arms, scrutinee } => {
            recur(db, scrutinee, true) && arms.iter().all(|a| recur(db, a.body, false))
        }
        Core::MatchList { arms, scrutinee } => {
            recur(db, scrutinee, true) && arms.iter().all(|a| recur(db, a.body, false))
        }
        Core::MatchSum { scrutinee, root } => {
            recur(db, scrutinee, true)
                && cont_only_borrowed_or_backedge(db, &root, binder, members, param_slots, slots)
        }
        // Control flow / binding: recurse each sub-position in RESULT (unborrowed) position — the fast path
        // already cleared sub-positions that don't reference `binder`. (A `let` initializer that borrows the
        // param into a scalar binding is rare + not whitelisted here; conservative = leak, never double-free.)
        Core::If { cond, then_, else_ } => {
            recur(db, cond, false) && recur(db, then_, false) && recur(db, else_, false)
        }
        Core::Let { bindings, body } => {
            bindings.iter().all(|&(_, v)| recur(db, v, false)) && recur(db, body, false)
        }
        // Every OTHER node kind that references `binder` (a non-member Call, a constructor, a mutating op, a
        // Closure, a Seq, arith/compare that consumes, …) is not whitelisted → deny. NARROW by design.
        _ => false,
    }
}

/// `param_only_borrowed_or_backedge` over a sum-match continuation (the `SumCont` tree): the leaf/guarded/
/// switch bodies are result positions checked the same way; the `Payload`/`Elem` path steps are borrows
/// carrying no binding, so only the continuations matter (mirrors `cont_binding_escapes`).
fn cont_only_borrowed_or_backedge(
    db: &mut Db,
    cont: &crate::core::SumCont,
    binder: StructId,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            param_only_borrowed_or_backedge(db, *body, binder, members, param_slots, slots)
        }
        crate::core::SumCont::Guarded { body, els, .. } => {
            param_only_borrowed_or_backedge(db, *body, binder, members, param_slots, slots)
                && cont_only_borrowed_or_backedge(db, els, binder, members, param_slots, slots)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_only_borrowed_or_backedge(db, then_, binder, members, param_slots, slots)
                && cont_only_borrowed_or_backedge(db, els, binder, members, param_slots, slots)
        }
        crate::core::SumCont::Switch { arms, .. } => arms.iter().all(|a| {
            cont_only_borrowed_or_backedge(db, &a.cont, binder, members, param_slots, slots)
        }),
    }
}

/// Whether `binder` occurs anywhere in the subtree at `id` (a fresh-cache wrapper over `binder_occurs`).
fn occurs_in(db: &mut Db, id: StructId, binder: StructId) -> bool {
    let mut cache: HashMap<StructId, bool> = HashMap::new();
    binder_occurs(db, id, binder, &mut cache)
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
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
            licm_invariant(db, lhs, inv_params) && licm_invariant(db, rhs, inv_params)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            licm_invariant(db, operand, inv_params)
        }
        // A collection COUNT / a projection / a sum-payload read is a pure borrowing read — invariant iff
        // the container is. (Its trap-freedom is decided separately by `is_trap_free`.)
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
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
            payloads.push((scrutinee, path.to_vec()));
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
        // materialized), else from the scrutinee. An `Elem` step is USUALLY a tuple/record `arr-get`, but a
        // LEADING `Elem` off a `List` scrutinee (a sum-with-tuple-payload matched as a LIST ELEMENT — prefix
        // `[Elem(0), Payload]`, whose two tuple binders share it) is a `vec-get` into the RRB vec, NOT a flat
        // `arr-get`. So TRACK the sub-value type down the walk exactly as the main `SumPayload` emit does and
        // pick the accessor per step; a bare unconditional `arr-get` mis-read the vec handle (→ garbage → an
        // `unreachable` trap, the list-element/tuple-payload miscompile). A `RestFrom` never appears in a
        // prefix (it is a sole step, never followed).
        let start = (0..prefix.len()).rev().find_map(|k| {
            out.payload_prefix_slots
                .get(&(scrutinee, prefix[..k].to_vec()))
                .map(|&s| (k, s))
        });
        // The absolute path walked so far (from the scrutinee root) and the CURRENT sub-value type — seeded
        // either from a shorter slotted prefix's recorded type (a `Payload`-ending prefix, else `Any`) or
        // from the scrutinee's own type when starting fresh.
        let mut walked_prefix: Vec<crate::core::PathStep>;
        let mut cur;
        let from = if let Some((k, s)) = start {
            out.push(Lir::LocalGet(s)); // [handle] — the shorter shared prefix
            walked_prefix = prefix[..k].to_vec();
            cur = out
                .sum_path_types
                .get(&(scrutinee, walked_prefix.clone()))
                .cloned()
                .unwrap_or(Ty::Any);
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
            walked_prefix = Vec::new();
            cur = type_of(db, scrutinee);
            0
        };
        for step in &prefix[from..] {
            walked_prefix.push(*step);
            match step {
                crate::core::PathStep::Payload => {
                    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                    cur = match cur.strip_nominal() {
                        Ty::Sum { .. } => payload_step_ty_of(
                            db,
                            scrutinee,
                            Some(scrutinee),
                            &cur,
                            &walked_prefix,
                            &out.sum_path_types,
                        ),
                        inner => inner.clone(),
                    };
                }
                crate::core::PathStep::Elem(i) => {
                    out.push(Lir::ConstI32(*i as i32));
                    // A list element reads the RRB vec (`vec-get`); a tuple/record cell reads the flat array
                    // (`arr-get`). Mirror the main `SumPayload` emit's per-step type-directed choice.
                    if matches!(cur.strip_nominal(), Ty::List(_)) {
                        out.push(Lir::CallImport(OP_VEC_GET));
                        cur = match cur.strip_nominal() {
                            Ty::List(e) => (**e).clone(),
                            _ => Ty::Any,
                        };
                    } else {
                        out.push(Lir::CallImport(OP_ARR_GET));
                        cur = Ty::Any;
                    }
                }
                crate::core::PathStep::RestFrom(_) => {
                    return Err(Reject::decline(
                        "a payload prefix cannot contain a RestFrom step",
                    ));
                }
                crate::core::PathStep::TupleRestFrom(_) => {
                    return Err(Reject::decline(
                        "a payload prefix cannot contain a TupleRestFrom step",
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
    // Partition into value-equivalence classes by `core_eq`. A distinct node joins the first class it is
    // `core_eq` to. To avoid an all-pairs O(cands²) `core_eq` scan (each `core_eq` a subtree-cloning
    // walk — the emit path's dominant cost on a WIDE arithmetic body where the "few CSE candidates"
    // assumption fails: N distinct scalar subterms → a singleton-heavy partition → N²/2 `core_eq` calls),
    // BUCKET candidates by a cheap shallow `core_hash_key` FIRST. `core_eq(a,b) ⇒ equal key`, so equal
    // candidates always land in the same bucket; `core_eq` then runs only WITHIN a bucket (near-always a
    // singleton or a genuine equal group), so unequal candidates never pairwise-compare. Behaviour-
    // identical to the old scan (the exact `core_eq` still decides membership within a bucket); the only
    // change is which pairs it is asked about. Distinct hashes never merge, so class identity is stable.
    let mut classes: Vec<Vec<StructId>> = Vec::new();
    let mut by_key: crate::fxhash::FxHashMap<u64, Vec<usize>> = crate::fxhash::FxHashMap::default();
    // Per-partition memo for the full-depth structural hash — each core node hashed once, so keying all
    // candidates is O(total core nodes), not O(candidates · depth).
    let mut hash_memo: crate::fxhash::FxHashMap<StructId, u64> =
        crate::fxhash::FxHashMap::default();
    for id in cands {
        let key = core_hash_key(db, id, &mut hash_memo);
        let bucket = by_key.entry(key).or_default();
        let mut placed = false;
        for &ci in bucket.iter() {
            // Count each within-bucket `core_eq` — the partition's comparison work. With hash-bucketing
            // this is O(#candidates) (each candidate compares only against same-hash predecessors, near
            // always none); the old all-pairs scan made it O(#candidates²). This is the noise-free
            // regression signal (`a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time`). A
            // per-`Db` counter (not a process-global atomic) so the parallel test harness's other
            // concurrent compiles can't pollute the reading — see `Db::cse_partition_core_eq_calls`.
            #[cfg(test)]
            {
                db.cse_partition_core_eq_calls += 1;
            }
            if core_eq(db, classes[ci][0], id) {
                classes[ci].push(id);
                placed = true;
                break;
            }
        }
        if !placed {
            bucket.push(classes.len());
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
        | Core::StrCmp { lhs, rhs, .. }
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
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            is_cse_shareable(db, operand)
        }
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

/// Recognize the MULTI-VALUE-UPGRADE tail shape and, if present, return the underlying self-call node.
///
/// When a recursive PERFORMER's out-state is OBSERVED after the recursion, the effect lowering upgrades
/// it to return `(value, out-state…)` and rewrites its tail self-call into
/// `(let ((temp (self-call …))) (tuple (. temp 0) (. temp 1) …))` — the call moves into the `let` BINDING
/// INIT and the `let` BODY re-packages `temp`'s slots into the return tuple (effects.rs `drain_and_wrap`).
/// That body is an IDENTITY repackage: `temp` already IS a `(value, out-state…)` tuple, and the body
/// rebuilds exactly `(. temp 0) … (. temp k)` in order — so the whole `let` is semantically just
/// `return self-call(…)`, a genuine tail call the upgrade obscured. Without recognizing it, the wasm loop
/// transform misses the edge and emits a real recursive call (one frame per iteration → stack exhaustion
/// at depth ~5-8k; the Rust backend survives only because rustc/LLVM TCO's the emitted identity tail).
///
/// Returns `Some(call_node)` when `id` is exactly that shape: a single-binding `let` whose init is a
/// `Core::Call`, whose body is a `Core::Tuple` of `arity` elements, and whose i-th element is
/// `Core::Proj { operand → the let binder, index: i }` for every `i` (a full, in-order identity
/// repackage). Any deviation (extra bindings, a non-projection element, a permuted/partial projection, a
/// projection of a different operand, a mismatched arity) returns `None` — so only the exact
/// return-packaging shape is treated as a tail call; genuine post-call computation is never mistaken for
/// one.
fn multivalue_repackage_tail_call(db: &mut Db, id: StructId) -> Option<StructId> {
    let Core::Let { bindings, body } = core_of(db, id) else {
        return None;
    };
    if bindings.len() != 1 {
        return None;
    }
    let (temp_binder, init) = bindings[0];
    // The init must be a call (the candidate self-call — membership is checked by the caller).
    if !matches!(core_of(db, init), Core::Call { .. }) {
        return None;
    }
    let Core::Tuple { elems } = core_of(db, body) else {
        return None;
    };
    // Each tuple element must be `(. temp i)` in order — a full identity repackage of the bound temp.
    for (i, elem) in elems.iter().enumerate() {
        let Core::Proj { operand, index } = core_of(db, *elem) else {
            return None;
        };
        if index != i {
            return None;
        }
        // `operand` must resolve to a reference to THIS let's binder (not some other tuple).
        match core_of(db, operand) {
            Core::LocalRef { binder } if binder == temp_binder => {}
            _ => return None,
        }
    }
    Some(init)
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
            // MULTI-VALUE-UPGRADE tail: `(let ((t (member-call …))) (tuple (. t 0) …))` is an identity
            // repackage of a self-call — a genuine tail edge the effect upgrade obscured (see
            // `multivalue_repackage_tail_call`). Treat it as a member tail-call so an observed-out-state
            // performer still loops (else it recurses per iteration and exhausts the wasm stack).
            if let Some(call) = multivalue_repackage_tail_call(db, id)
                && let Core::Call { callee, .. } = core_of(db, call)
                && members.contains(&callee)
            {
                return true;
            }
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
            // MULTI-VALUE-UPGRADE tail (see `multivalue_repackage_tail_call` + `body_has_member_tail_call`):
            // the identity-repackage `let` IS a tail call to the bound callee — collect it as a tail edge
            // so `mutual_loop_group` includes the self-recursion in the loop SCC.
            if let Some(call) = multivalue_repackage_tail_call(db, id)
                && let Core::Call { callee, .. } = core_of(db, call)
                && !out.contains(&callee)
            {
                out.push(callee);
                return;
            }
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
    /// A scratch slot holding an enclosing match's OWNED scrutinee SHELL that is DEAD on this loop's
    /// back-edge and must be `drop`ed BEFORE the `br` to the loop top — else it leaks one heap cell per
    /// iteration. Set by the tail `MatchSum` emit ONLY for an owned-single-view (`String.at`/`Bytes.slice`)
    /// scrutinee whose whole-match payload-safety holds (`matchsum_view_shell_reclaim_ok`): the payload is
    /// borrowed/dead on every arm (never consumed into the tail-call args), so freeing the shell (which
    /// cascades into the dead payload) before the back-edge is sound. The post-match fall-through drop
    /// handles the value-returning arms; this handles the looping arms the post-match drop `br`s past — the
    /// codec `find-at`/`fromcol` String.at scan leak. `None` on every ordinary loop (the common case).
    scrut_shell_reclaim: Option<u32>,
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
                db, callee, &args, slots, base, high, scratch_ty, layout, out, None,
            )?;
            // OPTION C: a CROSS-EDGE callee in TAIL position — it's an imported peer func, and there is no
            // `return_call` to an import (`ReturnCall` targets a local func index only). Emit the extern call
            // as an ORDINARY `CallExternImport` and let its result fall through as the function's result (the
            // wasm fn returns the top-of-stack value; no explicit Return op is emitted for a non-tail call
            // either). This forgoes the tail-call frame reuse for a cross-edge tail call — correct, just not
            // TCO'd; a cross-edge in tail position (a @test whose result IS a shared-closure call) is rare and
            // the closure's own recursion is TCO'd inside the provider. Empty map → never fires (non-consumer
            // byte-identical). Mirrors the peer-bound HostCall extern path (also a plain CallExternImport).
            if let Some(&pos) = layout.cross_edge_import.get(&callee) {
                trace!(target: "rcdzc::select", callee, pos, args = args.len(), "emit cross-edge extern call (tail, non-TCO)");
                out.push(Lir::CallExternImport(pos));
                return Ok(());
            }
            match layout.abs(callee) {
                Some(idx) => {
                    // A `return_call` returns the CALLEE's result valtype DIRECTLY as this function's
                    // result — valid ONLY when they match. If this function's result valtype differs (a
                    // narrowing/widening ascription over the tail-called result — e.g. `(: (rec …) UInt32)`
                    // gives this fn an i32 result while the recursive callee returns i64), a `return_call`
                    // ELIDES the width conversion the ascription requires → invalid wasm (fuzzer 38551).
                    // Fall back to a non-tail `Call` + the stack width conversion; the converted value is
                    // left on the stack and falls through to the function's IMPLICIT return (wasm returns
                    // the stack top — no explicit Return op, same as the `return_call`/extern-tail paths).
                    //
                    // The mismatch is the CALLEE's ACTUAL emitted result valtype vs THIS function's result
                    // valtype — NOT `type_of(id)`, which the call-site ascription already narrowed to the
                    // fn's result (so it would falsely == `fn_ret_vt` and miss the bug). The callee is a def
                    // index; its emitted result type is its body's solved type (all params are bound, so the
                    // body type IS the result). A callee with no body (import) can't be inspected — keep the
                    // tail call unchanged.
                    let callee_body = db.defs.get(callee).and_then(|d| d.body);
                    let callee_result_ty = match callee_body {
                        Some(b) => type_of(db, b),
                        None => {
                            out.push(Lir::ReturnCall(idx));
                            return Ok(());
                        }
                    };
                    let callee_vt = valtype_of(&callee_result_ty);
                    if callee_vt == out.fn_ret_vt {
                        trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit TAIL call (return_call)");
                        out.push(Lir::ReturnCall(idx));
                        return Ok(());
                    }
                    trace!(target: "rcdzc::select", callee, idx, "tail call callee result valtype differs from fn result — non-tail Call + convert");
                    out.push(Lir::Call(idx));
                    match (callee_vt, out.fn_ret_vt) {
                        (Some(ValType::I64), Some(ValType::I32)) => out.push(Lir::I32WrapI64),
                        (Some(ValType::I32), Some(ValType::I64)) => {
                            // Widen using the callee int's signedness (the value on the stack is the
                            // callee's narrow-int result).
                            let signed = matches!(
                                callee_result_ty.strip_nominal(),
                                Ty::Int(it) if it.ground_signed()
                            );
                            out.push(if signed {
                                Lir::I64ExtendI32S
                            } else {
                                Lir::I64ExtendI32U
                            });
                        }
                        (Some(ValType::F64), Some(ValType::F32)) => out.push(Lir::F32DemoteF64),
                        (Some(ValType::F32), Some(ValType::F64)) => out.push(Lir::F64PromoteF32),
                        _ => {
                            return Err(Reject::decline(
                                "tail call callee result valtype differs from the enclosing function's result in an unhandled way",
                            ));
                        }
                    }
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
            // IF-CHAIN → INTEGER MATCH (tail position — see the non-tail `Core::If` arm for the shape and
            // soundness): route a nested `(if (= X k) …)` dispatch on one reusable integer scrutinee
            // through the match backend so a dense range gets a `br_table`. Threads the tail loop context
            // (`tl`), so a self-tail-call in an arm body still iterates the loop / becomes a `return_call`
            // exactly as it did in the `if`-chain — the match backend's `Tail(tl)` path owns that (and the
            // `br_table` path is skipped for a self-loop `Tail(Some(_))`, keeping the linear chain that
            // loops correctly; a plain `Tail(None)`/exported body still gets the table).
            if let Some((scrut, arms)) = if_chain_as_int_match(db, cond, then_, else_) {
                let it = int_ty_of(db, scrut);
                let result_it = match &result {
                    Ty::Int(rit) => Some(*rit),
                    _ => None,
                };
                let block_ty = match &result {
                    Ty::Unit => BlockType::Empty,
                    other => match valtype_of(other) {
                        Some(vt) => BlockType::Val(vt),
                        None => {
                            return Err(Reject::decline(
                                "if-chain match result type has no machine representation",
                            ));
                        }
                    },
                };
                return emit_match_arms_tailable(
                    db,
                    scrut,
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
                );
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
            // A `Never` result (BOTH branches diverge) has no valtype but yields no value on any path —
            // both arms end in `unreachable`. Emit an EMPTY (0-result) block, then a trailing
            // `unreachable` AFTER it so the stack-polymorphic `unreachable` satisfies whatever slot the
            // enclosing (possibly value) position expects — a nested `(if b 1 (if c (trap) (trap)))` sends
            // the inner diverging `if` here as the outer's tail else-arm, which wants an i64. Trailing
            // `unreachable` is dead (both arms trapped) but keeps the module valid. Mirrors `Core::Trap`.
            // A genuinely unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
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
                } else if let Core::ConstFloat(d) = core_of(db, b)
                    && let Ty::Float(rft) = peel_qty_ty(result.clone())
                    && rft.ground_width() == 32
                {
                    // A bare `ConstFloat` branch takes the `if`'s RESULT float width, not its own default
                    // `Float64`: `(: (if c 1.5 0.25) Float32)` has the annotation on the `if`, so each branch
                    // literal solves to `Float64` and `Core::ConstFloat`'s emit (which reads the node's own
                    // solved width) pushes an `f64.const` while the block type is `f32` → an INVALID module
                    // (`expected f32, found f64`). Unlike a narrow INT (masked into the shared i32 slot),
                    // `f32`/`f64` are DISTINCT machine types — a hard validation error, not a silent mask.
                    // Ground it to the result's f32 here (the float twin of the `ConstInt` grounding above).
                    // PEEL `Ty::Nominal`/`Ty::Qty` first (via `peel_qty_ty`): `valtype_of` reads THROUGH those
                    // wrappers to the inner `f32`, so a wrapped-Float32 result (`(type F (Mk Float32))`,
                    // `(Qty Float32 u)`) gives an `f32` block — but a bare `Ty::Float` match would miss it and
                    // fall through to the default `f64.const`, the same invalid-module asymmetry the sibling
                    // int-grounding (`int_ty_of`/`emit_operand`) already avoids by stripping. (Latent today —
                    // `Qty.of` erases before the branch is a bare `ConstFloat` — but symmetric + hazard-free.)
                    out.push(Lir::F32ConstBits(
                        (f64::from_bits(d.to_f64_bits()) as f32).to_bits(),
                    ));
                    Ok(())
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
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A `let` in tail position: its body is tail — BUT only if no heap binding must be `drop`ped
        // AFTER the body. A drop is code that runs on the way out, so a `return_call` (which does not
        // return here) would skip it; when a drop is pending, fall back to the non-tail `emit` (the
        // body's call pushes an ordinary frame that returns, then the drops run). A body with no
        // pending drop (every heap binding escapes, or there are none) keeps the tail position.
        Core::Let { bindings, body } => {
            // MULTI-VALUE-UPGRADE tail: `(let ((t (member-call …))) (tuple (. t 0) …))` is an identity
            // repackage of a self-call (see `multivalue_repackage_tail_call`). When it targets a member of
            // THIS loop group, iterate in place — evaluate the call's args, move them into the param slots,
            // and `br` to the loop top — exactly as the plain `Core::Call` tail arm does. The tuple body is
            // pure return-packaging the loop reconstructs at exit (the base-case leaf), so it is not emitted
            // here. Without this the upgraded performer emits a real recursive call and exhausts the stack.
            if let Some(tl) = tl
                && let Some(call) = multivalue_repackage_tail_call(db, id)
                && let Core::Call { callee, args } = core_of(db, call)
                && let Some(which) = tl.member_which(callee)
                && args.len() == tl.param_slots.len()
            {
                emit_loop_iteration(
                    db, which, &args, tl, slots, base, high, scratch_ty, layout, out,
                )?;
                return Ok(());
            }
            // DUP-AWARE (see the non-tail `Core::Let` drop): a binding whose only consuming occurrences are
            // Perceus retains (`dup_sites`) still needs a scope-end drop of its surviving slot reference, so
            // it must fall back to the non-tail `emit` (which emits the drop epilogue), not the drop-free
            // tail fast path. Consult `dup_sites` so such a binding is detected here too.
            let dup_sites = out.dup_sites.clone();
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder))
                    && !binding_escapes_dup_aware(
                        db,
                        body,
                        EscapeTarget::Binder(*binder),
                        false,
                        Some(&dup_sites),
                    )
            });
            if any_drop {
                return emit(db, id, slots, base, high, scratch_ty, layout, out);
            }
            // Re-emit the bindings exactly as `emit` does, then the body in TAIL position. (No drop
            // epilogue is needed — the `any_drop` check above guaranteed none.)
            let mut extended = slots.clone();
            let mut floor = base;
            for (binder, value) in bindings.iter() {
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
                *high = (*high).max(slot + 1);
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
                // ALSO map the VALUE node → this slot, for a SCALAR binding. A closure that captures a
                // let-bound value records the capture as the VALUE node itself (`collect_captures` keys the
                // capture by the binding's value occurrence, NOT a `LocalRef` to the binder), so the closure
                // build-site's `emit(cap)` would RE-LOWER the value — a SECOND host call for a `(let ((v
                // (io.get))) …)` init captured by ≥2 escaping closures (adv-62: the host op fired once per
                // capturing closure → the extra call had no recorded response and TRAPPED, a soundness bug;
                // the rust backend fixed the same double-emit at expr.rs's `Core::Let`/`Core::Closure` arms).
                // The node→slot fast path at the top of `emit` reads this: a capture of `*value` now emits
                // `local.get slot` (the value computed ONCE at the `let`) instead of re-running the init.
                // SCALAR ONLY: a scalar slot holds the value directly (a `local.get` is a faithful re-read),
                // and a scalar host-result is the confirmed miscompile domain. A HEAP binding is EXCLUDED —
                // its slot holds a refcounted handle whose Perceus dup/drop accounting is per-OCCURRENCE
                // (`dup_sites`/`binding_escapes_dup_aware`), so aliasing the value node to the slot could
                // skew that bookkeeping; a heap value captured by a closure declines today (CDZ0201) anyway,
                // so it is not a live miscompile. Insert only when the slot is not already a node-key (a
                // materialized scrutinee), so this never shadows an existing fast-path entry.
                if !is_heap_type(&ty)
                    && matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
                {
                    extended.entry(*value).or_insert(slot);
                }
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
            // A `Never` match (all arms diverge): empty block + trailing `unreachable` (see the tail
            // `Core::If` arm). A genuinely unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
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
            )?;
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A LIST match in tail position: dispatch by length, each ARM BODY in tail position (a self-tail
        // call in a `(list …)` arm becomes a `return_call` / loop iteration). Mirrors the scalar `Match`
        // arm — materialize the handle + `vec-len` once, then `emit_list_arms_tailable` with `Tail(tl)`.
        // Without this, `MatchList` fell through to non-tail `emit`, so a tail list fold never looped
        // (`(sa xs acc) = (match xs ((list) acc) ((list x .. rest) (sa rest (+ acc x))))` stack-recursed).
        Core::MatchList { scrutinee, arms } => {
            // A `Never` list match (all arms diverge): empty block + trailing `unreachable`. A genuinely
            // unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
                    None => {
                        return Err(Reject::decline(
                            "list match result type has no machine representation",
                        ));
                    }
                },
            };
            let (arm_slots, len_slot, arm_base, owned_stash) = materialize_list_match_scrutinee(
                db, scrutinee, slots, high, scratch_ty, layout, out,
            )?;
            let reclaim = list_shell_reclaim_slot(
                db,
                scrutinee,
                &arms,
                owned_stash,
                TailPos::Tail(tl),
                never_diverges,
            );
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
            )?;
            // Reclaim the owned-temporary list shell after the arms (value-returning tail; the arms left
            // the result on the stack) — the list twin of the `MatchSum` owned-shell drop above.
            if let Some(slot) = reclaim {
                out.push(Lir::LocalGet(slot)); // [result, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [result]
            }
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A SUM match in tail position: dispatch on the discriminant decision tree, each LEAF/GUARDED body
        // in tail position (a self-tail-call in a `(Succ m) → (count m …)` arm becomes a `return_call` /
        // loop iteration). Mirrors the non-tail `MatchSum` emit — materialize the scrutinee handle once (a
        // reusable param/local is re-read cheaply per probe; a computed scrutinee is stashed in a fresh
        // i32 slot so it is evaluated ONCE) — then `emit_sum_cont_tailable` with `Tail(tl)`. Without this,
        // `MatchSum` fell through to non-tail `emit`, so a tail-recursive sum consumer never looped (`(count
        // n acc) = (match n ((Zero) acc) ((Succ m) (count m (+ acc 1))))` stack-recursed).
        Core::MatchSum { scrutinee, root } => {
            // A `Never` sum match (all decision-tree leaves diverge): empty block + trailing
            // `unreachable`. A genuinely unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
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
            let (arms_slots, arms_base, stashed_slot) = if reusable_handle_src(db, scrutinee, slots)
            {
                (slots.clone(), base, None)
            } else {
                let slot = *high;
                *high = slot + 1;
                // The spill slot's width is the SCRUTINEE'S machine type, NOT always i32: a real boxed sum is
                // an i32 handle, but an ERASED single-variant newtype over a SCALAR (`(type W (Wrap Int64))`)
                // is a bare i64 (no box) — spilling that i64 into a hardcoded-i32 slot re-types one wasm local
                // to two widths → `type mismatch: expected i32, found i64`, an invalid module (a literal-
                // payload arm `(match (mk d) ((Wrap 5) …))` over a runtime-built erased newtype). Default to
                // i32 for a rep-less type (a handle-shaped scrutinee).
                let scrut_vt = valtype_of(&type_of(db, scrutinee)).unwrap_or(ValType::I32);
                scratch_ty.insert(slot, scrut_vt);
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
                (m, (*high).max(slot + 1), Some((slot, scrut_vt)))
            };
            // SHELL RECLAIM (the tail twin of the non-tail `MatchSum` reclaim): deep-`drop` the owned
            // freshly-stashed boxed-sum shell after the arms (it is a dead temporary). Consuming child
            // extractions were `dup`'d upfront (`collect_shell_reclaim_child_dups`), so the deep drop nets
            // correctly. WARNING: TAIL-SPECIFIC GUARD: skip the reclaim when ANY arm is a MEMBER TAIL-CALL
            // (`sum_cont_has_member_tail_call`) — that arm `br`s to the loop top and NEVER reaches the post-
            // match drop, so a drop here would (a) not run on the looping path (leak, harmless) but worse
            // (b) the shell slot is a fresh scratch above the params that the next iteration's scrutinee emit
            // reuses, so dropping it is moot AND a return_call tail-call likewise leaves via the call. Only a
            // VALUE-returning tail match (every arm produces a value that falls through to the block end —
            // e.g. `(match (f n+1) ((Mk a _) (Mk a a)))` whose arm is a constructor, not a self-call) has a
            // reclaim point. Also skip a diverging match (no value/return point).
            let scrut_ty = type_of(db, scrutinee);
            let arms_tail_call =
                matches!(tl, Some(t) if sum_cont_has_member_tail_call(db, &root, t.members));
            // WARNING: SAFETY RESTRICTION (sread UAF fix, 2026-07-19): reclaim ONLY an ALL-SCALAR-payload shell —
            // same sound floor as the non-tail arm. The inc2 compound broadening was unsound for a child
            // BORROWED OUT via an aliasing op (`Map.lookup`/`List.at` return a handle aliasing into the shell
            // child; the deep drop then frees a still-read value → the sread OOB/unreachable UAF). A scalar
            // payload copies out (no alias), so the drop is safe; a compound shell is left un-dropped (leak,
            // value-correct) pending a sound compound-reclaim increment.
            // NON-TAIL SPINE param path (v-mem-safety-signed-off): a PARAM scrutinee proven OWNED + DEAD-AFTER
            // (in `nontail_match_reclaim_binders`: heap + count_param_consumes==0 + not-epilogue-dropped) has NO
            // stashed temp, so `sum_shell_reclaim_ok` (which requires a stashed Owned slot) declines it. Reclaim
            // it via its PARAM SLOT when the payload/rematch gates hold. SOUND because: this is TAIL position
            // (the match is the fn's last use → no after-use of the shell); the arm's existing consume-dup of
            // the payload runs BEFORE this post-arm drop (gate 6 ordering); the payload-safety gate excludes a
            // borrowed-out payload (gate 3) and a re-match (gate 8); count_param_consumes==0 means the match
            // holds the LAST owned ref (a post-match consume → count>0 → not in the set → not reclaimed here).
            let param_reclaim = stashed_slot.is_none()
                && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder }
                    if out.nontail_match_reclaim_binders.contains(&binder))
                && nontail_param_payload_ok(db, scrutinee, &scrut_ty, never_diverges, &root);
            // OWNED-SINGLE-VIEW (String.at / Bytes.slice) local reclaim: its Some shell leaks because the
            // scrutinee is not globally `Owned` (`matchsum_view_shell_reclaim_ok`). UNLIKE the general
            // owned/param reclaim it fires EVEN WHEN `arms_tail_call` — a tail-recursive String.at scan
            // (codec `find-at`) is the very shape that leaks per iteration. The reclaim then splits across
            // BOTH exit kinds: (a) the post-match fall-through drop below handles the VALUE-returning arms
            // (`find-at`'s found `i` / `None` `-1`); (b) the LOOPING arms `br` past that drop, so the shell
            // slot is threaded into the arms' `TailLoop` and dropped before each back-edge `br`
            // (`emit_loop_iteration`). Payload-safety (`sum_shell_reclaim_payload_ok`, whole-match) proves
            // the view is borrowed/dead on every arm, so freeing the shell on any exit is sound;
            // `fromcol`'s view-into-`Call` arm fails it → no reclaim (defined leak, never a double-free).
            let view_reclaim = matchsum_view_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            let reclaim_shell = view_reclaim
                || (!arms_tail_call
                    && (sum_shell_reclaim_ok(
                        db,
                        scrutinee,
                        &scrut_ty,
                        stashed_slot,
                        never_diverges,
                        &root,
                    ) || param_reclaim));
            // Thread the owned-view shell slot into the arms' loop context so a member tail-call in an arm
            // (`find-at`'s recursive branch) drops the dead shell before its back-edge `br`. Only when the
            // match actually loops (`arms_tail_call`) and the view reclaim holds; else the arms' `tl` is
            // unchanged (the common case never touches this).
            let arm_tp = if view_reclaim && arms_tail_call {
                let shell_slot = stashed_slot
                    .expect("matchsum_view_shell_reclaim_ok implies a stashed I32 slot")
                    .0;
                TailPos::Tail(tl.map(|t| TailLoop {
                    scrut_shell_reclaim: Some(shell_slot),
                    ..t
                }))
            } else {
                TailPos::Tail(tl)
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
                arm_tp,
            )?;
            if reclaim_shell {
                // The stashed temp's slot, OR (non-tail-spine param path) the PARAM scrutinee's own slot.
                // `arms_slots`/`slots` are keyed by the param's BINDER (select_function_of inserts
                // `binder -> slot`), so resolve the scrutinee's binder, not its occurrence id. op_drop is
                // DEEP + rc-aware: the shell frees, cascading into the payload m which the arm already dup'd
                // (collect_shell_reclaim_child_dups non-tail-spine path) → m lands at its owned rc, no
                // double-free / no leak.
                let slot = stashed_slot
                    .map(|s| s.0)
                    .or_else(|| match core_of(db, scrutinee) {
                        Core::Param { binder } | Core::LocalRef { binder } => {
                            arms_slots.get(&binder).copied()
                        }
                        _ => None,
                    });
                let Some(slot) = slot else {
                    return Err(Reject::decline(
                        "shell reclaim: no stashed slot or param binder slot for the scrutinee",
                    ));
                };
                out.push(Lir::LocalGet(slot)); // [result, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the owned sum shell)
            }
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // Everything else in tail position is an ordinary value (no tail call inside it) — emit normally,
        // then COERCE its valtype to the function's result valtype if they differ (S141). `emit` leaves the
        // value at `valtype_of(type_of(id))` — its OWN natural width — but the function's IMPLICIT return
        // (wasm returns the stack top) needs `fn_ret_vt`. A tail value whose type is NARROWER/WIDER than the
        // fn result (a `UInt8` arith arm — i32 — tail-returned from an `Int64`-result fn, or a `(: … UInt32)`
        // ascription narrowing an i64 value) otherwise leaves the wrong width on the stack → invalid wasm or
        // a wrong result (fuzzer S141). The tail-`Call` arm above already coerces its callee result; this is
        // the same coercion for a non-call tail value. Unlike the Call case (whose `type_of` is masked by the
        // call-site ascription to the fn result), `type_of(id)` HERE is the value's own type, so the compare
        // is exact. Only the four scalar width pairs are coercible; a matching width (the common case) or a
        // non-scalar handle is emitted UNCHANGED (byte-identical), so this changes only genuine mismatches.
        _ => {
            emit(db, id, slots, base, high, scratch_ty, layout, out)?;
            let value_ty = type_of(db, id);
            match (valtype_of(&value_ty), out.fn_ret_vt) {
                (Some(ValType::I64), Some(ValType::I32)) => out.push(Lir::I32WrapI64),
                (Some(ValType::I32), Some(ValType::I64)) => {
                    // Widen using the value int's signedness (the narrow int on the stack).
                    let signed = matches!(
                        value_ty.strip_nominal(),
                        Ty::Int(it) if it.ground_signed()
                    );
                    out.push(if signed {
                        Lir::I64ExtendI32S
                    } else {
                        Lir::I64ExtendI32U
                    });
                }
                (Some(ValType::F64), Some(ValType::F32)) => out.push(Lir::F32DemoteF64),
                (Some(ValType::F32), Some(ValType::F64)) => out.push(Lir::F64PromoteF32),
                // Matching widths (byte-identical to before) or a non-scalar/Unit/Never result: emit as-is.
                _ => {}
            }
            Ok(())
        }
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
/// Whether `x` is a DIRECT `Param`/`LocalRef` occurrence of binder `p`.
fn is_ref_to(db: &mut Db, x: StructId, p: StructId) -> bool {
    matches!(core_of(db, x), Core::Param { binder } | Core::LocalRef { binder } if binder == p)
}

/// Site A sound guard: COUNT the CONSUMING uses of loop-param `p` in `expr` (all nesting). A consume =
/// (a) a runtime CONSUME-BUT-PRODUCE-FRESH op whose consumed operand is DIRECTLY `p` (`List.concat`/push/
/// update, `Bytes.concat`, `Map.insert`/remove, `Set.insert`/remove/algebra — the class `binding_escapes`
/// wrongly calls a borrow because the result is fresh), (b) a `RestFrom` tail-slice (`vec-drop`) of `p`,
/// or (c) an ESCAPE — `p` handed to a `Call`/`CallClosure`/constructor (ownership transfers out). A BORROW
/// (`vec-len`/`vec-get`/`Proj`/length/compare) adds 0. The preservation dup for `p`'s reordered-last
/// `RestFrom` is skippable ONLY when this total is 1 (that single `RestFrom` is `p`'s sole consume; every
/// other use is a pure borrow that reads the live slot before the consume). Recurses ALL children so a
/// NESTED consume (`INVERSION`'s `count-after` Call, or a nested `RestFrom`/`Map.insert` of `p` in a
/// sibling arg) is counted — the gap that made `binding_escapes`-alone unsound.
#[allow(clippy::collapsible_if, clippy::collapsible_match)]
fn count_param_consumes(
    db: &mut Db,
    id: StructId,
    p: StructId,
    seen: &mut HashSet<StructId>,
    count: &mut usize,
    count_restfrom: bool,
) {
    if !seen.insert(id) {
        return;
    }
    match core_of(db, id) {
        Core::ListConcat { lhs, rhs } | Core::BytesConcat { lhs, rhs } => {
            if is_ref_to(db, lhs, p) {
                *count += 1;
            }
            if is_ref_to(db, rhs, p) {
                *count += 1;
            }
        }
        Core::SetAlgebra { lhs, rhs, .. } => {
            if is_ref_to(db, lhs, p) {
                *count += 1;
            }
            if is_ref_to(db, rhs, p) {
                *count += 1;
            }
        }
        Core::ListPush { list, elem }
        | Core::ListPrepend { list, elem }
        | Core::SetInsert {
            set: list, elem, ..
        }
        | Core::SetRemove {
            set: list, elem, ..
        } => {
            if is_ref_to(db, list, p) {
                *count += 1;
            }
            if is_ref_to(db, elem, p) {
                *count += 1;
            }
        }
        Core::ListUpdate { list, elem, .. } => {
            if is_ref_to(db, list, p) {
                *count += 1;
            }
            if is_ref_to(db, elem, p) {
                *count += 1;
            }
        }
        Core::MapInsert { map, val, .. } => {
            if is_ref_to(db, map, p) {
                *count += 1;
            }
            if is_ref_to(db, val, p) {
                *count += 1;
            }
        }
        Core::MapRemove { map, .. } => {
            if is_ref_to(db, map, p) {
                *count += 1;
            }
        }
        Core::SumNew { ref payloads, .. } => {
            for &e in payloads.iter() {
                if is_ref_to(db, e, p) {
                    *count += 1;
                }
            }
        }
        Core::Tuple { ref elems }
        | Core::ListNew { ref elems }
        | Core::SetOf { ref elems, .. }
        | Core::BytesOf { ref elems } => {
            for &e in elems.iter() {
                if is_ref_to(db, e, p) {
                    *count += 1;
                }
            }
        }
        Core::Record { ref fields } => {
            for &e in fields.values() {
                if is_ref_to(db, e, p) {
                    *count += 1;
                }
            }
        }
        Core::Call { ref args, .. } => {
            for &a in args.iter() {
                if is_ref_to(db, a, p) {
                    *count += 1;
                }
            }
        }
        Core::CallClosure { closure, ref args } => {
            if is_ref_to(db, closure, p) {
                *count += 1;
            }
            for &a in args.iter() {
                if is_ref_to(db, a, p) {
                    *count += 1;
                }
            }
        }
        Core::SumPayload {
            scrutinee,
            ref path,
        } if count_restfrom && matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))) => {
            if is_ref_to(db, scrutinee, p) {
                *count += 1;
            }
        }
        _ => {}
    }
    for c in core_child_ids(db, id) {
        count_param_consumes(db, c, p, seen, count, count_restfrom);
    }
}

/// Populate `out` with the SURPLUS-skippable `dup_sites` occurrences (see [`Emit::surplus_skippable_dups`]):
/// the retain dups that are PROVABLY redundant in a boundary-owned body and may be skipped, the NARROW
/// replacement for the too-broad `body_is_boundary_owned`-alone gate (which stripped load-bearing retains =
/// 159 corpus UAFs). A `dup_sites` occurrence of binder `b` is surplus iff BOTH: (1) `b` is a MatchList
/// SCRUTINEE with a `(.. r)` REST-PATTERN arm (`ListArmCond::LenGe`/`Any`) — the RestFrom family, present
/// whether the rest binder is USED or DEAD; AND (2) `b` has NO consume OTHER than a RestFrom
/// (`count_param_consumes` with `count_restfrom=false` == 0). Rationale: in a BOUNDARY-OWNED body the caller
/// holds a live reference to `b` for the whole body, so a pure-BORROW read needs no retain; the keep-alive
/// `dup` exists ONLY to balance a later CONSUME, and for a rest-pattern match `b`'s only consume (if any) is
/// the `(.. r)` RestFrom, whose `vec-drop` already has its OWN balancer (the emit's RestFrom preservation dup)
/// — so the retain is redundant. Covers BOTH the DEAD rest (05:18721 `f` — 0 consumes) and a sole used
/// RestFrom. Conjunct 1 EXCLUDES non-list-rest borrows (a shared inner map, an RRB list as a map value, a
/// Bytes rope read twice — their keep-alive is load-bearing for value-heap sharing `count_param_consumes` does
/// not model); conjunct 2 EXCLUDES a rest scrutinee ALSO consumed by push/insert/escape/self-call (retain is
/// the SOLE balancer) — together the UAF classes the broad gate hit. Caller gates on `is_boundary_owned`.
/// `dup_sites` occurrences are `LocalRef`/`Param` nodes, so an occurrence's binder is read via `core_of`.
// DISABLED pending a sound narrowing (see the call site in `select_function_of`, bisect #7255): the
// surplus analysis miscompiles a recursive rest-pattern list-equality (a borrowed `vec-get` co-element
// outliving the RestFrom rest-mint). Kept in-tree (not deleted) so the sound re-enable is a one-line
// call restore + the borrowed-co-element exclusion, co-designed with v-memory-safety.
#[allow(dead_code)]
fn collect_surplus_skippable_dups(
    db: &mut Db,
    body: StructId,
    dup_sites: &HashSet<StructId>,
    out: &mut HashSet<StructId>,
) {
    use crate::core::ListArmCond;
    // (1) Binders that are a MatchList scrutinee with a `(.. r)` REST-PATTERN arm (LenGe/Any) — the RestFrom
    // family, present whether the rest binder is USED or DEAD. This EXCLUDES non-list-rest borrows (a shared
    // inner map, an RRB list as a map value, a Bytes rope read twice) whose keep-alive is load-bearing for
    // value-heap sharing `count_param_consumes` does not model.
    fn gather_rest_scrutinees(
        db: &mut Db,
        id: StructId,
        out: &mut HashSet<StructId>,
        seen: &mut HashSet<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::MatchList { scrutinee, arms } = core_of(db, id)
            && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
            && arms
                .iter()
                .any(|a| matches!(a.cond, ListArmCond::LenGe(_) | ListArmCond::Any))
        {
            out.insert(binder);
        }
        for c in core_child_ids(db, id) {
            gather_rest_scrutinees(db, c, out, seen);
        }
    }
    let mut rest_scrutinees: HashSet<StructId> = HashSet::new();
    let mut seen = HashSet::new();
    gather_rest_scrutinees(db, body, &mut rest_scrutinees, &mut seen);
    if rest_scrutinees.is_empty() {
        return;
    }
    // (2) Keep only those with NO consume OTHER than a RestFrom (count_restfrom = false == 0) — excludes a rest
    // scrutinee ALSO consumed by push/insert/escape/self-call (its retain is the SOLE balancer for that consume).
    let mut surplus_binders: HashSet<StructId> = HashSet::new();
    for &b in rest_scrutinees.iter() {
        let mut cseen = HashSet::new();
        let mut nonrest = 0usize;
        count_param_consumes(db, body, b, &mut cseen, &mut nonrest, false);
        if nonrest == 0 {
            surplus_binders.insert(b);
        }
    }
    if surplus_binders.is_empty() {
        return;
    }
    for &id in dup_sites.iter() {
        if let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, id)
            && surplus_binders.contains(&binder)
        {
            out.insert(id);
        }
    }
}

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
    // EXPERIMENT PART 2: eval args whose VALUE is a RestFrom tail-slice (SumPayload path ends RestFrom,
    // a runtime-CONSUMING vec-drop materialization) LAST — detected structurally (NOT via binding_escapes,
    // which calls the fresh-tail a borrow).
    let is_restfrom_consume: Vec<bool> = args
        .iter()
        .map(|&arg| {
            matches!(core_of(db, arg), Core::SumPayload { ref path, .. }
                if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))))
        })
        .collect();
    // SELF-LOOP-TAIL SUM-SPINE reclaim (§5, v-runtime co-designed): an arg that is a `Payload` extraction of
    // the loop-param it is stored BACK INTO — `depth-tail`'s `(S rest) => depth-tail rest …`, `rest` =
    // `SumPayload{scrutinee: Param v, path:[Payload]}` carried into v's own slot — consumes v's spine node
    // per iteration. `sum-payload` is a BORROW (no rc++, no reclaim, unlike `vec-drop`/RestFrom which
    // reclaims implicitly), so without help the old S shell is overwritten un-dropped → the whole spine
    // LEAKS (the tail shell-reclaim is SKIPPED for a member-tail-call arm, comment at the reclaim gate). FIX:
    // `dup(rest)` after eval, then `drop(old v)` BEFORE the store — op_drop ALWAYS cascades, so the dup keeps
    // `rest` alive as the cascade decrements v's payload ref back to its owned rc; v's cell is freed, `rest`
    // carried at the correct rc → one node reclaimed per iteration, no leak / no double-free. GATED like the
    // RestFrom case: the Payload must be v's SOLE consuming use (`count_param_consumes == 1`) so the drop
    // reclaims the sole ref, and the scrutinee binder's slot must BE the target param slot (a self-spine walk).
    let is_sumpayload_consume: Vec<bool> = args
        .iter()
        .enumerate()
        .map(|(i, &arg)| {
            i < tl.param_slots.len()
                && matches!(core_of(db, arg), Core::SumPayload { scrutinee, ref path }
                    if matches!(path.last(), Some(crate::core::PathStep::Payload))
                        && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder }
                            if slots.get(&binder) == Some(&tl.param_slots[i])))
                && {
                    // SOLE-consume gate: the Payload extraction is v's only consuming use across all args.
                    if let Core::SumPayload { scrutinee, .. } = core_of(db, arg)
                        && let Core::Param { binder } | Core::LocalRef { binder } =
                            core_of(db, scrutinee)
                    {
                        let mut seen = HashSet::new();
                        let mut total = 0usize;
                        for &a in args.iter() {
                            count_param_consumes(db, a, binder, &mut seen, &mut total, true);
                        }
                        // `count_param_consumes` counts RestFrom / consume-ops / escapes but NOT a `Payload`
                        // extraction, so `total` here is the count of OTHER consuming uses of v. `== 0` ⟹ v is
                        // used only as the match scrutinee (borrow) + this carried Payload → v is DEAD after
                        // the extraction and the drop reclaims its sole remaining (shell) ref. `> 0` (v also
                        // pushed/inserted/escaped/RestFrom'd elsewhere) ⟹ KEEP — dropping it would double-free
                        // the ref that other consume needs.
                        total == 0
                    } else {
                        false
                    }
                }
        })
        .collect();
    // BORROWED-ACCUMULATOR RECLAIM (v-core-opt; v-effects K1-reviewed ORTHOGONAL): a loop-carried param
    // updated by a BORROWING op — `rational-add`/`bigint-*` READ the old accumulator and return a FRESH
    // value — is DEAD after its rebind, but the general store below just LocalSet-overwrites the slot, so
    // the old (distinct-cell) accumulator LEAKS every iteration (harmonic/codec/absorption numeric folds:
    // +1 value/iter; the systemic corpus-06 +N). Drop the old value on rebind. GATED to a PURE BORROW so a
    // CONSUMED / FBIP-reused param (`List.push acc x` — the op reuses `acc` in place, old==new cell) is
    // NEVER double-freed:
    //   (c) NOT identity / RestFrom / SumPayload-consume — excludes the K1/spine class the #5090/#5142 loop
    //       dup-skip fence governs (all RestFrom-consumed), so this never fires on a fenced param; and
    //   (d) the param is NOT consumed by ANY rebind arg (`!binding_escapes(arg, binder)` for every arg) —
    //       a borrow-only param whose ref is NOT carried into the next iteration (a `List.push acc` consumes
    //       `acc` into the arg → escapes → excluded). Since old-acc and new-acc are INDEPENDENT cells for a
    //       borrowing op, the drop needs NO dup (unlike the §5 sum-spine reclaim, where old is new's parent).
    // SINGLE-MEMBER only: a mutual loop's cross-arm param classification is deferred (v-effects' edge — a
    // param borrowed here but RestFrom/grandchild-consumed in a sibling arm could double-free).
    let single_member = tl.members.len() == 1;
    let member_params: Vec<StructId> = if single_member {
        db.defs[tl.members[0]].params.clone()
    } else {
        Vec::new()
    };
    let drop_old_borrowed: Vec<bool> = (0..args.len())
        .map(|i| {
            if !single_member
                || is_identity[i]
                || is_restfrom_consume[i]
                || is_sumpayload_consume[i]
                || i >= tl.param_slots.len()
                || i >= member_params.len()
            {
                return false;
            }
            let binder = db
                .ast
                .as_form(member_params[i], ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(member_params[i]);
            if !is_heap_type(&type_of(db, binder)) {
                return false;
            }
            // Dead iff NOT consumed by any rebind arg (only borrowed) AND the accumulator's NEW value
            // PROVABLY produces a FRESH cell (a numeric rational-add/bigint OR a fresh product-compound ctor
            // that only borrows the old accumulator — never aliases/descends it). These two conjuncts are
            // co-dependent: the `binding_escapes` "borrowed-not-consumed" check ALONE over-approximated
            // "dead" (a compound accumulator whose CHILD is carried forward — a match-payload binder `l` = a
            // child of the old `s`, or a ctor EMBEDDING a heap child — would pass it), but a ctor embedding a
            // heap child of the accumulator makes that child ESCAPE through the ctor element (a nested-compound
            // `Proj`, `get_op` None, consuming position) → `binding_escapes` = true → this whole gate is false,
            // so the old shell is NEVER dropped when it would cascade-free a carried cell (breaker's CAD fold
            // `bb (Diff l _t) => bb l`; the 7 CAD double-frees; the `v2max`/CSG-`fuse` share hazards). The
            // escape guard + fresh-cell gate together are the SOUND sufficient condition — see
            // `rebind_produces_fresh`. Extended from numeric-only to fresh product ctors to close the RECURSIVE
            // tuple/record/list-STATE handler per-perform leak (v-effects wasm-dump-confirmed on rectuple_tail).
            !args.iter().any(|&a| binding_escapes(db, a, binder, false))
                && rebind_produces_fresh(db, args[i])
        })
        .collect();
    let mut eval_order: Vec<usize> = (0..args.len())
        .filter(|&i| !is_identity[i] && !is_restfrom_consume[i])
        .collect();
    eval_order.extend((0..args.len()).filter(|&i| !is_identity[i] && is_restfrom_consume[i]));
    // PART 1 (Site A): mark loop-carried params consumed by a RestFrom tail-slice arg (PART 2 ordered
    // these LAST, so the vec-drop is the param's last emitted use — no read after) as no-preservation-dup
    // for THIS iteration's arg emit. SAFETY: only a param consumed by EXACTLY ONE arg (no double-consume)
    // AND that is a walked loop-param slot (reassigned, so no end-of-scope drop needs the preserved
    // handle). The gated dup sites (`emit_binder_ref`, the `RestFrom` step) then skip the preservation
    // dup → borrows read the live slot, the final `vec-drop` consumes+reuses the sole ref (rc1→0). Restore
    // the set after so a nested/outer emit is unaffected.
    let saved_no_dup = std::mem::take(&mut out.loop_reassign_no_dup);
    // SOUND guard (v-runtime co-designed): for each loop-param consumed by a RestFrom arg, skip its
    // preservation dups ONLY IF that RestFrom is its SOLE consuming use across ALL args (all nesting) —
    // every other use a pure borrow. `count_param_consumes` unions the consume-but-fresh op class +
    // RestFrom + escapes; == 1 ⟹ the single RestFrom is the only consume ⟹ vec-drop consumes the sole
    // ref (rc1→0, FBIP-reuse). > 1 (a NESTED consume: INVERSION's count-after Call, a sibling
    // RestFrom/Map.insert of p) ⟹ KEEP the dup (else an rc-flap / census-hidden UAF).
    for i in 0..args.len() {
        if is_restfrom_consume[i]
            && let Core::SumPayload { scrutinee, .. } = core_of(db, args[i])
            && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
            && let Some(&sl) = slots.get(&binder)
            && tl.param_slots.contains(&sl)
        {
            let mut seen = HashSet::new();
            let mut total = 0usize;
            for &a in args.iter() {
                count_param_consumes(db, a, binder, &mut seen, &mut total, true);
            }
            // FLAGSHIP-UAF fence (breaker K1 / #4139 loop-specific over-optimization; v-rb-diagnosed SITE A),
            // NARROWED (v-mem #5090-over-retain report): `count_param_consumes` counts consumes of the loop-
            // param `binder` itself, blind to a heap GRANDCHILD `(. e val)` — a field of a head ELEMENT `e`
            // destructured by a `(list e .. rest)` match — CONSUMED (`String.concat acc (. e val)`) in a
            // NON-RestFrom sibling arg while `e` STAYS in the list. Skipping `binder`'s preservation dup for
            // FBIP-reuse frees the old list (rc1->0 via the RestFrom) — and with it the still-owned element `e`
            // and its live grandchild — → use-after-free. Keep the dup so the old list survives to that consume.
            //
            // The DISTINGUISHER (empirically confirmed, ksd1 vs FLATTEN): the over-free needs the consumed heap
            // handle to be a GRANDCHILD — a projection `(. e val)` = `Proj{operand: e}` where `e` is itself an
            // element-extraction of `binder` (`e` borrowed, only its field consumed, so `e` stays owned by the
            // freed spine). A DIRECT element consumed — `(List.concat acc h)` where `h = SumPayload{scrutinee:
            // binder, path:[Elem]}` (FLATTEN, 05-compound:11721) — is MOVED OUT: consuming `h` transfers its
            // ownership, so freeing the spine is safe and NO dup is needed. The prior gate used the depth-blind
            // `arm_borrows_heap_subvalue` (true for BOTH), over-retaining FLATTEN-class clean accumulators
            // (+10 spurious leak, v-mem measured). `arm_consumes_binder_grandchild` fires ONLY for the
            // grandchild shape (operand of the consumed projection is a PROPER projection-chain of `binder`,
            // not `binder`/the direct element itself), so ksd1/ksd2's K1 UAF fence holds while FLATTEN reclaims.
            let element_heapchild_consumed = args
                .iter()
                .enumerate()
                .any(|(j, &a)| j != i && arm_consumes_binder_grandchild(db, a, binder));
            if total == 1 && !element_heapchild_consumed {
                out.loop_reassign_no_dup.insert(sl);
            }
        }
    }
    // §5 sum-spine: BEFORE eval, SAVE each self-consuming loop-param's OLD shell into a fresh scratch slot,
    // so it can be dropped AFTER the stores (off-stack) without interleaving with the parallel-move arg
    // stack. The save is a slot COPY (no rc change); the old shell stays owned in the scratch until its drop.
    let mut spine_old_scratch: Vec<u32> = Vec::new();
    // `i` still indexes `tl.param_slots`; iterate `is_sumpayload_consume` directly for the per-arg flag.
    for (i, &consume) in is_sumpayload_consume.iter().enumerate() {
        if consume {
            let sc = *high;
            *high = (*high).max(sc + 1);
            scratch_ty.insert(sc, ValType::I32);
            out.push(Lir::LocalGet(tl.param_slots[i])); // [old-v]
            out.push(Lir::LocalSet(sc)); // scratch = old-v (slot copy)
            spine_old_scratch.push(sc);
        }
    }
    // BORROWED-ACCUMULATOR reclaim (save half): SAVE each borrowed-rebound loop-param's OLD value into a
    // fresh scratch (a slot COPY, no rc change — the old handle stays owned in scratch), so it can be dropped
    // AFTER the stores without interleaving with the parallel-move arg stack. Mirrors the §5 save; the drop
    // (post-store, below) needs NO dup since old-acc and new-acc are independent cells.
    let mut borrowed_old_scratch: Vec<u32> = Vec::new();
    for (i, &drop_old) in drop_old_borrowed.iter().enumerate() {
        if drop_old {
            let sc = *high;
            *high = (*high).max(sc + 1);
            scratch_ty.insert(sc, ValType::I32);
            out.push(Lir::LocalGet(tl.param_slots[i])); // [old-v]
            out.push(Lir::LocalSet(sc)); // scratch = old-v (slot copy)
            borrowed_old_scratch.push(sc);
        }
    }
    // Args start ABOVE the saved-shell scratch so their emit never reuses those persistent slots (and never
    // below the body scratch floor `base`).
    let mut arg_base = base.max(*high);
    for &i in &eval_order {
        let arg = args[i];
        if let Core::ConstInt(_) = core_of(db, arg)
            && let Ty::Int(ait) = type_of(db, arg)
        {
            emit_operand(db, arg, ait, slots, arg_base, high, scratch_ty, layout, out)?;
        } else {
            emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?;
        }
        arg_base = *high;
    }
    for &i in eval_order.iter().rev() {
        out.push(Lir::LocalSet(tl.param_slots[i]));
    }
    // §5 sum-spine reclaim (post-store): the carried Payload `rest` is now IN its param slot. For each
    // self-consuming sum-spine param, RETAIN rest (`local.get slot; dup` — `dup` pops a handle + rc++, no
    // stack result, so this is stack-neutral and bumps rest's rc), then DROP the saved OLD shell
    // (`local.get scratch; drop`). op_drop ALWAYS cascades, so the old shell's free decrements its child ref
    // = rest, which the dup pre-bumped → rest lands at its owned rc. Net per iteration: the old S cell is
    // freed and rest is carried owned — the 10000-deep spine is reclaimed AS WALKED, no leak / no UAF.
    let mut spine_idx = 0usize;
    // `i` still indexes `tl.param_slots`; iterate `is_sumpayload_consume` directly for the per-arg flag.
    for (i, &consume) in is_sumpayload_consume.iter().enumerate() {
        if consume {
            let sc = spine_old_scratch[spine_idx];
            spine_idx += 1;
            out.push(Lir::LocalGet(tl.param_slots[i])); // [rest]
            out.push(Lir::CallImport(OP_DUP)); // rc++ (pops rest; no result) → []
            out.push(Lir::LocalGet(sc)); // [old-v]
            out.push(Lir::CallImport(OP_DROP)); // free old-v; cascade decrements rest → owned → []
        }
    }
    // BORROWED-ACCUMULATOR reclaim (drop half): the new value is now in the param slot; free each saved OLD
    // borrowed accumulator. NO dup — old-acc and new-acc are independent cells (a borrowing op allocated the
    // new value), so this drop does not cascade into the carried value. Net per iteration: the old
    // accumulator is freed, the leak is gone (the systemic corpus-06 +N fix).
    for &sc in &borrowed_old_scratch {
        out.push(Lir::LocalGet(sc)); // [old-v]
        out.push(Lir::CallImport(OP_DROP)); // free the dead old accumulator → []
    }
    out.loop_reassign_no_dup = saved_no_dup;
    // OWNED-VIEW SHELL back-edge reclaim: an enclosing owned-single-view (String.at/Bytes.slice) MatchSum
    // set `scrut_shell_reclaim` to its Some-shell scratch slot (dead on this back-edge — the whole-match
    // payload-safety proved the view is borrowed/dead on every arm, so it is NOT among the args just
    // evaluated/stored). Free it now, before the `br`, else it leaks one cell per loop iteration (the codec
    // find-at/fromcol scan). op_drop is DEEP + rc-aware; the dead payload cascades to 0. AFTER the arg
    // stores (the args never reference the view — that IS the borrow-clean gate), so no arg handle dangles.
    if let Some(sc) = tl.scrut_shell_reclaim {
        out.push(Lir::LocalGet(sc)); // [shell]
        out.push(Lir::CallImport(OP_DROP)); // free the dead owned-view shell → []
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
        // Each member's body gets a FRESH scratch floor past the running high-water mark, NOT the shared
        // `base`. Members sit in mutually-EXCLUSIVE dispatch branches (`which == idx`), but a wasm local is
        // FUNCTION-GLOBAL and has ONE type — so if member A stashes an i64 arith temp in scratch slot `base`
        // and member B stashes an i32 heap handle in that same slot, the one local is declared at two widths
        // and the module fails validation (`type mismatch: expected i64, found i32` at the `local.tee`).
        // Advancing to `*high` hands each member never-typed slots, exactly the discipline the CSE/LICM arm
        // (`body_base = body_base.max(high)`) and `emit_call_args`/`emit_loop_iteration` already apply for
        // simultaneously-typed sibling scratch. (The prior code passed `base` unchanged to every member, so
        // a 6-member SCC of mixed-width readers — `(i32,i64,i32)` args — emitted invalid wasm.)
        let member_base = (*high).max(base);
        if idx + 1 == members.len() {
            // Last member — the unconditional tail (no probe; reached by elimination).
            return emit_tail(
                db,
                body,
                slots,
                member_base,
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
            member_base,
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
/// The type of element/field `i` of a tuple/record container — used to track the sub-value type `cur` down
/// a `SumPayload` `Elem` walk so a SUBSEQUENT `Elem` into a nested `List` field picks `vec-get` (not the
/// default `arr-get` on the RRB vec, which reads garbage → an `unreachable` trap). A record's `Elem` slot is
/// its SORTED-field index (the `BTreeMap` iterates sorted), matching how the value is laid out. `Ty::Any`
/// for a non-tuple/record container or an out-of-range index (the walk then falls back to `arr-get`).
fn elem_field_ty(cur: &crate::ty::Ty, i: usize) -> crate::ty::Ty {
    match cur.strip_nominal() {
        crate::ty::Ty::Tuple(elems) => elems.get(i).cloned().unwrap_or(crate::ty::Ty::Any),
        crate::ty::Ty::Record(fields) => fields
            .values()
            .nth(i)
            .cloned()
            .unwrap_or(crate::ty::Ty::Any),
        _ => crate::ty::Ty::Any,
    }
}

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
pub(crate) fn variant_payload_ty_at(db: &mut Db, sum: &Ty, disc: u32) -> Option<Ty> {
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

/// The shared soundness floor for deep-dropping an owned boxed-sum SHELL after a `MatchSum` (extracted
/// from the tail-position and non-tail `MatchSum` reclaim gates so both compute the IDENTICAL predicate).
/// Reclaim iff the match does not diverge, the scrutinee is a REAL boxed sum (not an erased enum-disc),
/// EVERY payload is scalar (the sread-UAF sound floor — a scalar copies out, holding no handle that could
/// alias the shell), the scrutinee was freshly stashed into an i32 slot, and it is a PROVEN-owned
/// temporary. The tail gate additionally ANDs its own `!arms_tail_call` (a member-tail-call arm `br`s to
/// the loop top and never reaches the post-match drop) — that stays at the call site because it needs the
/// `MatchSum` decision tree, and it is NOT the same as a `TailPos::Tail(Some(_))` self-loop test (a
/// `Tail(Some)` match whose arm is a constructor still has a valid reclaim point).
/// (FIND3, v-mem-safety fence — the SCRUTINEE-ESCAPE analog of scalar-extracted-not-escaped) Whether the
/// matched `scrutinee` is DEAD AFTER DESTRUCTURE — the WHOLE scrutinee value (and, when it is a Param/
/// LocalRef, its binder) does NOT appear in ANY arm body EXCEPT as the destructured value (a `SumPayload`/
/// `Proj`/`SumExpect` read OFF it). A DIRECT re-reference — `(f st)`, `(tuple st …)`, `return st` — makes it
/// LIVE AFTER the match, so the shell-deep-drop would free a still-live value → UAF. STRUCTURAL: any
/// non-destructuring reference counts.
///
/// NOTE: RESUME-ESCAPE (v-effects #048389): a handler-arm `(resume -1 st)` re-reference is INVISIBLE here —
/// by select.rs the resume is REDUCED/threaded, so `st` is not a syntactic arm-body ref. TWO guards cover
/// it: (1) the handler THREADED-STATE scrutinee is classified BORROWED, so the caller's `Owned` gate excludes
/// it (rrb1); (2) a `CallClosure`/`HostCall` in the arm (an opaque consumer that could capture the scrutinee
/// invisibly) is conservatively treated as not-dead. A genuinely OWNED-COMPUTED scrutinee (a fresh recursive
/// `Call` result — the FIND3 target) is not a threaded-state and cannot resume-escape. (Tighter pre-reduction
/// signal = v-effects' #4966 collect_tail_resume_values, deferred unless the conservative floor over-excludes.)
fn scrutinee_dead_after_destructure(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
) -> bool {
    let binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    !sum_cont_refs_scrutinee(db, root, scrutinee, binder)
}

fn sum_cont_refs_scrutinee(
    db: &mut Db,
    cont: &crate::core::SumCont,
    scrutinee: StructId,
    binder: Option<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_refs_scrutinee(db, *body, scrutinee, binder),
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_refs_scrutinee(db, *cond, scrutinee, binder)
                || expr_refs_scrutinee(db, *body, scrutinee, binder)
                || sum_cont_refs_scrutinee(db, els, scrutinee, binder)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_refs_scrutinee(db, then_, scrutinee, binder)
                || sum_cont_refs_scrutinee(db, els, scrutinee, binder)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_refs_scrutinee(db, &a.cont, scrutinee, binder)),
    }
}

/// Whether the subtree at `id` references the scrutinee (its node id, or its binder via [`is_ref_to`])
/// OUTSIDE a destructuring read. A `SumPayload`/`Proj`/`SumExpect` reading OFF the scrutinee is the match's
/// own extraction (allowed) — its scrutinee/operand slot is SKIPPED; every OTHER position that references
/// the scrutinee is an escape. A `CallClosure`/`HostCall` is conservatively an escape (opaque capture).
fn expr_refs_scrutinee(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    binder: Option<StructId>,
) -> bool {
    let mut seen = HashSet::new();
    expr_refs_scrutinee_seen(db, id, scrutinee, binder, &mut seen)
}

fn expr_refs_scrutinee_seen(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    binder: Option<StructId>,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    // CONSERVATIVE OPAQUE-CAPTURE BACKSTOP: a `CallClosure`/`HostCall` env is invisible to this walk and
    // could capture the scrutinee (a reduced resume-continuation is one) → treat as not-dead-after.
    if matches!(
        core_of(db, id),
        Core::CallClosure { .. } | Core::HostCall { .. }
    ) {
        return true;
    }
    let is_scrut_ref =
        |db: &mut Db, x: StructId| x == scrutinee || binder.is_some_and(|b| is_ref_to(db, x, b));
    match core_of(db, id) {
        // A destructuring read OFF the scrutinee is the match's own extraction — SKIP its scrutinee/operand
        // slot (allowed), but still descend the OTHER children (an index/path could reference the scrutinee).
        Core::SumPayload { scrutinee: s, .. }
        | Core::SumExpect { scrutinee: s, .. }
        | Core::Proj { operand: s, .. }
            if is_scrut_ref(db, s) =>
        {
            core_child_ids(db, id)
                .into_iter()
                .filter(|&c| c != s)
                .any(|c| expr_refs_scrutinee_seen(db, c, scrutinee, binder, seen))
        }
        _ => {
            if is_scrut_ref(db, id) {
                return true;
            }
            core_child_ids(db, id)
                .into_iter()
                .any(|c| expr_refs_scrutinee_seen(db, c, scrutinee, binder, seen))
        }
    }
}

/// Whether ANY arm body (or guard) in a `MatchSum` decision tree materializes a heap sub-value of the
/// scrutinee OUT as a live handle — the sum analogue of the per-arm `arm_borrows_heap_subvalue` check
/// `list_shell_reclaim_slot` runs over a `MatchList`'s flat arms. Walks the `SumCont`: a `Leaf`/`Guarded`
/// body (and a `Guarded` guard) is an arm body; `Guarded.els`/`LitTest.then_`/`LitTest.els`/`Switch` arms
/// are continuations. If NONE borrows a heap sub-value out (escape-clean), the payload is destructured to
/// scalars — no live handle survives — and the shell deep-drop is safe. Because
/// `collect_consuming_payload_sites` marks a compound-child dup on exactly the SAME consuming-position
/// condition, escape-clean here implies the shell-reclaim dup pass collected NOTHING for this shell, so
/// the deep-drop needs no dup pairing and reclaims the shell + its borrowed-only children by cascade.
fn sum_cont_arm_borrows_heap_subvalue(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => arm_borrows_heap_subvalue(db, *body),
        crate::core::SumCont::Guarded { cond, body, els } => {
            arm_borrows_heap_subvalue(db, *cond)
                || arm_borrows_heap_subvalue(db, *body)
                || sum_cont_arm_borrows_heap_subvalue(db, els)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_arm_borrows_heap_subvalue(db, then_)
                || sum_cont_arm_borrows_heap_subvalue(db, els)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_arm_borrows_heap_subvalue(db, &a.cont)),
    }
}

/// Conservative reuse-clean for the compound-shell reclaim: whether ANY arm body CONSTRUCTS a compound
/// value. A constructor is the only node that can FBIP-reuse a payload cell in place — a
/// `(tuple (. t 0) (. t 1))` reuses the projected payload's cell, which the shell deep-drop would then
/// double-free even though escape analysis (which sees only borrowing projections) reports it safe (the
/// FBIP-rebuild-of-projection gap `arm_borrows_heap_subvalue` cannot see). Declining any arm that builds a
/// compound over-approximates (a rebuild-to-fresh arm leaks rather than reclaims — value-correct, never a
/// double-free), which honors the discipline "no reclaim widening without the FBIP-aware check"; the
/// destructure-to-scalar acceptance set constructs nothing and is unaffected.
fn sum_cont_arm_constructs_compound(db: &mut Db, cont: &crate::core::SumCont) -> bool {
    let mut seen = HashSet::new();
    sum_cont_arm_constructs_compound_seen(db, cont, &mut seen)
}

fn sum_cont_arm_constructs_compound_seen(
    db: &mut Db,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_constructs_compound_seen(db, *body, seen),
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_constructs_compound_seen(db, *cond, seen)
                || expr_constructs_compound_seen(db, *body, seen)
                || sum_cont_arm_constructs_compound_seen(db, els, seen)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_arm_constructs_compound_seen(db, then_, seen)
                || sum_cont_arm_constructs_compound_seen(db, els, seen)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_arm_constructs_compound_seen(db, &a.cont, seen)),
    }
}

/// Whether the expression subtree `id` contains a compound CONSTRUCTOR node (see
/// [`sum_cont_arm_constructs_compound`]). Node-id `seen` set guards the shared-`StructId` DAG re-walk.
fn expr_constructs_compound_seen(db: &mut Db, id: StructId, seen: &mut HashSet<StructId>) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if matches!(
        core_of(db, id),
        Core::Tuple { .. }
            | Core::SumNew { .. }
            | Core::ListNew { .. }
            | Core::MapNew { .. }
            | Core::SetOf { .. }
            | Core::Record { .. }
            | Core::BytesOf { .. }
            | Core::BinBuild { .. }
            | Core::BinBitsBuild { .. }
    ) {
        return true;
    }
    // A borrowing READ (Proj/SumPayload/SumExpect/*Len) reads its AGGREGATE operand in place without
    // transferring it — do NOT descend into that operand. It is the scrutinee/aggregate, and a constructor
    // in the aggregate's OWN definition (e.g. an inline `(if (Some (list …)) None)` scrutinee) is not an
    // arm reuse of the shell payload; descending it false-flagged every inline-constructed-scrutinee match
    // (d4/dm1/… declined despite pure-scalar arm bodies). A genuine FBIP-rebuild `(tuple (. t 0) (. t 1))`
    // is still caught: the `Tuple` is a top-level arm node, flagged above before its `Proj` operands here.
    match core_of(db, id) {
        Core::Proj { .. }
        | Core::SumPayload { .. }
        | Core::SumExpect { .. }
        | Core::ListLen { .. }
        | Core::BytesLen { .. }
        | Core::StrScalarLen { .. } => false,
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| expr_constructs_compound_seen(db, c, seen)),
    }
}

/// Whether `id` is a FALLIBLE-READ extraction op (`List.at`/`Bytes.at`/`Map.lookup`/`Bytes.slice`/
/// `String.at`/`String.slice`) — each returns a runtime `Option` and `dup`-RETAINS the extracted element
/// into the `Some` (see `heap_operand_ownership`), holding the payload at rc >= 2 through the arm. inc2b
/// keys on this: the rc >= 2 makes an in-place FBIP reuse of the payload PATH-COPY (never alias), and the
/// Stage-B consume reclaim balances the extra retained ref against the shell deep-drop.
fn scrutinee_is_fallible_extraction(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::ListAt { .. }
            | Core::BytesAt { .. }
            | Core::StrAt { .. }
            | Core::StrSlice { .. }
            | Core::BytesSlice { .. }
            | Core::MapLookup { .. }
    )
}

/// The allowlist of PURE PERSISTENT BUILDER ops for the inc2b Stage-B extraction-consume reclaim: each
/// takes EXACTLY ONE owned reference to its input structure(s) and returns one fresh result that
/// references the input once (structural-sharing `dup`s the shared nodes once). So when an extraction
/// `Some`'s payload is CONSUMED by one of these, a single `dup`-on-escape (the site
/// `collect_shell_reclaim_child_dups` marks) balances the shell deep-drop 1:1. An OPAQUE consumer
/// (`Call`/`CallClosure`/`HostCall` — which includes a REDUCED `resume`-thread, `resume` being invisible
/// in Core, and any multi-use/capturing consumer) is NOT provably single-reference at select.rs, so a
/// payload consumed there DECLINES (leak beats UAF). Co-verified with v-runtime (runtime domain).
fn is_allowlisted_builder(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::ListPush { .. }
            | Core::ListPrepend { .. }
            | Core::ListConcat { .. }
            | Core::ListUpdate { .. }
            | Core::BytesConcat { .. }
            | Core::MapInsert { .. }
            | Core::SetInsert { .. }
            | Core::SetAlgebra { .. }
    )
}

/// Collect the DIRECT child node-ids of every allowlisted-builder node reachable in the arm continuation.
/// A consuming scrutinee-payload site that is one of these children is consumed DIRECTLY by a builder
/// (`mlr2`: `inner` is the `lhs` of `List.concat`); a site NOT in this set is consumed by an opaque node
/// (a `Call`/`CallClosure`, or reached only through one) and must decline.
fn collect_allowlisted_builder_children_cont(
    db: &mut Db,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
    out: &mut HashSet<StructId>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            collect_allowlisted_builder_children_expr(db, *body, seen, out)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_allowlisted_builder_children_expr(db, *cond, seen, out);
            collect_allowlisted_builder_children_expr(db, *body, seen, out);
            collect_allowlisted_builder_children_cont(db, els, seen, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_allowlisted_builder_children_cont(db, then_, seen, out);
            collect_allowlisted_builder_children_cont(db, els, seen, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms {
                collect_allowlisted_builder_children_cont(db, &a.cont, seen, out);
            }
        }
    }
}

fn collect_allowlisted_builder_children_expr(
    db: &mut Db,
    id: StructId,
    seen: &mut HashSet<StructId>,
    out: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    let is_builder = is_allowlisted_builder(db, id);
    for c in core_child_ids(db, id) {
        if is_builder {
            out.insert(c);
        }
        collect_allowlisted_builder_children_expr(db, c, seen, out);
    }
}

/// inc2b Stage B — reclaim an extraction-`Some` (`List.at`/`Map.lookup`/`Bytes.slice`, which `dup`-retains
/// its payload) whose payload ESCAPES only by being CONSUMED by a pure persistent builder from the
/// allowlist ([`is_allowlisted_builder`]). Fires iff the scrutinee is a fallible extraction AND there is
/// at least one CONSUMING scrutinee-payload site AND every such site (the set
/// `collect_shell_reclaim_child_dups` will `dup`) is a DIRECT child of an allowlisted builder — so each
/// `dup` is balanced 1:1 by the builder's single-owned-ref move plus the shell deep-drop. A borrow-only
/// arm (no consuming site) is left to Stage A's escape-clean branch; a payload consumed by an opaque
/// `Call`/`CallClosure` (including a reduced `resume`-thread) is not a builder child, so the subset check
/// fails and the shell declines (leak beats UAF). Reuse-clean is not required: the extraction holds the
/// payload at rc >= 2, so any in-place FBIP reuse path-copies (v-runtime's Stage-A argument) and the
/// allowlisted builder likewise path-copies its rc >= 2 input. Any imprecision in either set only
/// OVER-DECLINES (a site missing from the builder set fails the subset check) — never over-reclaims.
fn sum_cont_extraction_consume_allowlisted(
    db: &mut Db,
    root: &crate::core::SumCont,
    scrutinee: StructId,
) -> bool {
    if !scrutinee_is_fallible_extraction(db, scrutinee) {
        return false;
    }
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    if consuming.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    let mut builder_children = HashSet::new();
    collect_allowlisted_builder_children_cont(db, root, &mut seen, &mut builder_children);
    consuming.iter().all(|s| builder_children.contains(s))
}

/// NON-TAIL SPINE param-path payload gate (v-mem-safety predicate 1, consume-only): the tail-MatchSum
/// shell-reclaim of a proven-owned-dead-after PARAM scrutinee is safe iff its heap payload is CONSUMED (a
/// call/op arg — the dup-pass dups it, so the shell-drop cascade nets correctly) and NEVER appears in an
/// arm's RESULT/tail position (returned as-is → could alias the shell ref and be freed by the drop → UAF).
/// Distinct from the STASHED path's `sum_shell_reclaim_ok` (which keeps the strict `arm_borrows` block for a
/// consumed compound): here the dup-pass (`collect_shell_reclaim_child_dups`'s non-tail-spine arm) guarantees
/// the consumed payload is dup'd, so a CONSUMING occurrence is safe; only a RESULT-position occurrence is
/// blocked. Composes with `!sum_cont_arm_constructs_compound` (FBIP-reuse hazard) + `!cont_rematches` (Class-B).
fn nontail_param_payload_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    !never_diverges
        && is_heap_type(scrut_ty)
        && !ty_is_enum_disc(db, scrut_ty)
        && !sum_cont_arm_constructs_compound(db, root)
        && !cont_rematches_scrutinee(db, scrutinee, root)
        && !sum_cont_payload_in_result(db, root, scrutinee)
}

/// Whether ANY arm's RESULT/tail expression IS a heap payload of `scrut` (the UNSAFE non-tail-spine case:
/// the payload is RETURNED, so the shell-drop would free the returned value). Follows result-position tails
/// (`If`/`Let`/`Seq`/`Block`/`Break`) but does NOT descend into CALL/OP ARGS (a payload consumed there is
/// dup-protected + safe). A result-position payload-of-scrut (`SumPayload`/`Proj` rooting at `scrut`, heap)
/// → true.
fn sum_cont_payload_in_result(db: &mut Db, cont: &crate::core::SumCont, scrut: StructId) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => payload_in_result_position(db, *body, scrut),
        crate::core::SumCont::Guarded { body, els, .. } => {
            payload_in_result_position(db, *body, scrut)
                || sum_cont_payload_in_result(db, els, scrut)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_payload_in_result(db, then_, scrut)
                || sum_cont_payload_in_result(db, els, scrut)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_payload_in_result(db, &a.cont, scrut)),
    }
}

fn payload_in_result_position(db: &mut Db, id: StructId, scrut: StructId) -> bool {
    if payload_proj_chain_roots_at_node(db, id, scrut) {
        // A payload projection of the scrutinee IN RESULT position — heap (a scalar leaf copies out, safe).
        return is_heap_type(&type_of(db, id)) && !matches!(get_op(db, id), Ok(Some(_)));
    }
    match core_of(db, id) {
        Core::If { then_, else_, .. } => {
            payload_in_result_position(db, then_, scrut)
                || payload_in_result_position(db, else_, scrut)
        }
        Core::Let { body, .. } => payload_in_result_position(db, body, scrut),
        Core::Seq { tail, .. } => payload_in_result_position(db, tail, scrut),
        Core::Block { body, .. } => payload_in_result_position(db, body, scrut),
        Core::Break { value } => payload_in_result_position(db, value, scrut),
        // A call/constructor/arith as the result CONSUMES the payload inside its args — not a bare result
        // escape (constructor-as-result is separately blocked by sum_cont_arm_constructs_compound).
        _ => false,
    }
}

fn sum_shell_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    // The STASHED-owned path: the payload-safety + rematch gates PLUS a freshly-stashed I32 slot holding an
    // OWNED scrutinee (a computed/materialized temporary). A reused PARAM/local slot fails the Owned gate
    // (heap_operand_ownership(Param)==Borrowed) — that case is the non-tail-spine param path, gated separately
    // via `sum_shell_reclaim_payload_ok` + the proven-owned-dead-after `nontail_match_reclaim_binders` set.
    sum_shell_reclaim_payload_ok(db, scrutinee, scrut_ty, never_diverges, root)
        && matches!(stashed_slot, Some((_, ValType::I32)))
        && matches!(
            heap_operand_ownership(db, scrutinee),
            Ok(HandleOwnership::Owned)
        )
}

/// The owned-single-view-producer twin of [`sum_shell_reclaim_ok`] for `MatchSum` (the `SumExpect`
/// `sumexpect_shell_reclaim` analogue): a `String.at`/`Bytes.slice` scrutinee returns a fresh `Some(one
/// view)` — owned + single-heap-payload BY CONSTRUCTION ([`is_owned_single_view_producer`]) — but is
/// deliberately NOT globally `Owned` (`heap_operand_ownership` — the Stage-B `String.concat` note at
/// select.rs's StrAt comment), so `sum_shell_reclaim_ok`'s `Owned` gate MISSES it and its `Some` shell
/// LEAKS one cell per match (the corpus-06 codec `find-at`/`fromcol` per-`String.at`-iteration leak). Treat
/// it as owned LOCALLY here (the local>global discipline the SumExpect reclaim already uses).
///
/// CRITICAL — the STRICT BORROW-CLEAN floor, NOT the full [`sum_shell_reclaim_payload_ok`]. This view path
/// emits NO compensating child-`dup` (`collect_shell_reclaim_child_dups` keys on a globally-`Owned` scrutinee,
/// which a StrAt is NOT), so it is sound ONLY when the view is purely BORROWED — never consumed/escaped and
/// never FBIP-rebuilt. `sum_shell_reclaim_payload_ok`'s consume-into-builder (disjunct 4,
/// `sum_cont_extraction_consume_allowlisted`) + extraction-borrowed (disjunct 5) branches ASSUME that
/// child-dup, so admitting them here DOUBLE-FREES a consumed view (`rev-go`'s `(String.concat acc c)` — c
/// consumed by the allowlisted `String.concat` AND freed again by the shell-drop cascade → an rc-underflow
/// trap; caught by the corpus enum). So gate on the borrow-clean disjunct-(3) conditions DIRECTLY:
/// `!arm_borrows_heap_subvalue` (the view is only READ — `value-eq`/`Bytes.at`/probe, per the relax set — never
/// materialized out in a consuming position) AND `!arm_constructs_compound` (no rebuild reusing the view cell),
/// plus the shared non-diverging / heap / non-enum / not-re-matched (Class-B) safety gates. `find-at`,
/// `balanced-paren`, the multibyte-rope `(match (String.at …) ((Some c) (if (= c …) …)))` cases qualify (value-eq
/// borrow); `rev-go`/`fromcol` (consume the view) are correctly EXCLUDED (leak beats a double-free). Stage-B /
/// value-eq StrAt consumers are UNCHANGED — a `MatchSum`-emit-LOCAL override, no global reclassification.
fn matchsum_view_shell_reclaim_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    stashed_slot: Option<(u32, ValType)>,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    if !is_owned_single_view_producer(db, scrutinee)
        || !matches!(stashed_slot, Some((_, ValType::I32)))
        || never_diverges
        || !is_heap_type(scrut_ty)
        || ty_is_enum_disc(db, scrut_ty)
        // Class-B: a scrutinee re-matched by a nested MatchSum is reclaimed by the inner drop already.
        || cont_rematches_scrutinee(db, scrutinee, root)
    {
        return false;
    }
    // The PRECISE soundness condition for this NO-CHILD-DUP path: the view must have ZERO CONSUMING sites —
    // it is only BORROWED (a `value-eq`/probe read), never moved into a builder/Call NOR returned as the
    // arm result. A consuming site would transfer ownership of the view to that consumer, so the shell-drop
    // cascade freeing the same view = a DOUBLE-FREE (the owned-scrutinee path compensates with a child-`dup`
    // that this local view path does NOT emit — `rev-go`'s `(String.concat acc c)` consume, `fromcol`'s
    // `find-at … c` consume, and an escape-as-result all register a consuming site → excluded, leak beats
    // UAF). `find-at`/`balanced-paren`/the multibyte-rope cases consume nothing (value-eq only) → empty set →
    // reclaimed. Uses the SAME consume/borrow classifier as the owned-scrutinee dup-site collection.
    let mut consuming = HashSet::new();
    collect_consuming_payload_sites_cont(db, root, scrutinee, &mut consuming);
    consuming.is_empty()
}

/// The scrutinee-shell-reclaim gates that are INDEPENDENT of how the scrutinee's handle is held (stashed
/// temp vs proven-owned param slot): heap + non-enum + non-diverging + payload-safety + not-re-matched.
/// [`sum_shell_reclaim_ok`] ANDs the stashed-Owned requirement on top; the non-tail-spine param path ANDs
/// the proven-owned-dead-after param membership on top. Splitting these lets BOTH reclaim a shell soundly
/// while the payload/rematch soundness stays in ONE place.
fn sum_shell_reclaim_payload_ok(
    db: &mut Db,
    scrutinee: StructId,
    scrut_ty: &Ty,
    never_diverges: bool,
    root: &crate::core::SumCont,
) -> bool {
    !never_diverges
        && is_heap_type(scrut_ty)
        && !ty_is_enum_disc(db, scrut_ty)
        // The all-scalar floor is always safe (a scalar payload copies out). A COMPOUND payload is
        // reclaimable when the arm is escape-clean (no heap sub-value read out as a live handle) AND
        // reuse-clean (no arm constructs a compound that could FBIP-reuse a payload cell).
        // inc2b (Stage A): an escape-clean + reuse-clean compound Some reclaims even when its scrutinee is
        // a fallible EXTRACTION op (List.at/Map.lookup/Bytes.slice). The extraction dup-retains the payload
        // into the Some, so the payload is at rc>=2 through the arm — any in-place FBIP reuse of its cell is
        // structurally suppressed (node_rc: rc>1 path-copies, never mutates in place), and the deep-drop's
        // cascade merely decrements the extra retained ref (the source keeps its own ref). Verified on the
        // debug runtime: the extraction family reclaims value-correct with zero traps (mts1 6->3 no-trap,
        // p.rc>=2 held through the rebuild since the shell drop fires AFTER the arm body). The earlier
        // scrutinee_is_fallible_extraction decline was over-conservative. Stage B (the third OR branch,
        // sum_cont_extraction_consume_allowlisted) additionally reclaims an escape-clean=FALSE extraction
        // Some whose payload is CONSUMED by a pure allowlisted builder (List.concat/push/insert/…): the
        // builder is a single-owned-ref move, so the dup-on-escape balances the deep-drop 1:1. A consume by
        // an opaque Call/CallClosure (incl. a reduced resume-thread — resume is invisible in Core) is NOT a
        // builder child, so it declines (leak beats UAF).
        && (sum_has_only_scalar_payloads(db, scrut_ty)
            // (FIND3, v-mem-safety-confirmed) ALL-SCALAR PRODUCT scrutinee (a Tuple/Record of all-scalar
            // fields): every field is EXTRACTED via get-int/get-bool (COPIED, not a cell alias), so the arm
            // CANNOT FBIP-reuse the old product's cells → the shell-deep-drop is safe EVEN WHEN the arm builds
            // a compound (the `!arm_constructs_compound` FBIP suppression is spurious here). The Tuple/product
            // analog of the all-scalar-payload floor + #4939. Fixes the arg-scaling group (fib fast-doubling,
            // Catalan/Pascal/look-and-say/pairwise-swap — recursive builds' intermediate scalar-tuples).
            // GATED (both load-bearing, v-mem-safety): OWNED scrutinee (a fresh recursive-`Call` result — NEVER
            // a BORROWED handler threaded-state, which can resume-escape invisibly; rrb1) AND DEAD-AFTER-
            // DESTRUCTURE (the whole scrutinee not re-referenced/escaping in any arm — rrb1's `(resume -1 st)`).
            || (ty_is_all_scalar_product(db, scrut_ty)
                // The scrutinee is a fresh recursive-`Call` result (an Owned COMPUTED value consumed by this
                // match) — NEVER a handler THREADED-STATE (an If/materialize/Param that a `resume` re-reads
                // invisibly, rrb1). A `Core::Call` result is inlined once as the match scrutinee and cannot be
                // resume-threaded, so it IS dead after destructure — the sound proxy for v-effects' "exclude
                // any resuming arm" that is decidable at select.rs (the resume-escape being pre-reduction).
                // Keeps the arg-scaling group (fib/Catalan/Pascal recursive-tuple results); excludes rrb1.
                && matches!(core_of(db, scrutinee), Core::Call { .. })
                && scrutinee_dead_after_destructure(db, scrutinee, root))
            || (!sum_cont_arm_borrows_heap_subvalue(db, root)
                && !sum_cont_arm_constructs_compound(db, root))
            || sum_cont_extraction_consume_allowlisted(db, root, scrutinee)
            // (5) EXTRACTION-BORROWED-PROBE (CHAMP-key, v-mem-safety-approved as a new disjoint disjunct — do
            // NOT broaden branch (3)'s !arm_constructs_compound, which is the general FBIP fence for non-
            // extraction scrutinees where rc>=2 is not guaranteed). Reclaim a compound Some from a fallible
            // EXTRACTION (List.at/Bytes.at/Str.at/Str.slice/Bytes.slice/Map.lookup) whose arm is BORROW-CLEAN,
            // EVEN WHEN the arm constructs a compound. Two hazards, two conjuncts, both load-bearing:
            //   (i) FBIP-REUSE — scrutinee_is_fallible_extraction: the extraction dup-retains the payload into
            //       the Some, holding it at rc>=2 through the arm (the inc2b property, mts1 6->3 no-trap), so
            //       any in-place FBIP reuse of its cell path-copies (node_rc: rc>1 never mutates) → the arm's
            //       compound build cannot alias/consume the payload cell.
            //   (ii) ESCAPE — !sum_cont_arm_borrows_heap_subvalue: no heap sub-value is read out as a live
            //       handle outliving the shell-deep-drop (the borrow-relax above classifies a Set.contains/
            //       Map.lookup/value-eq key/probe as a borrow, NOT an escape). A view CONSUMED/STORED into a
            //       collection (Set.of/insert, Map.insert — MIXED/STORED negative controls) is NOT in the
            //       borrow allowlist → arm_borrows stays TRUE → this disjunct does NOT fire → those stay a
            //       defined leak, never a double-free. The outer `&& !cont_rematches_scrutinee` (Class-B) still
            //       applies. This is the borrowed-probe sub-case of the Stage-A extraction-rebuild inc2b already
            //       verified — strictly SAFER (the payload is only READ, never flows into the compound).
            || (scrutinee_is_fallible_extraction(db, scrutinee)
                && !sum_cont_arm_borrows_heap_subvalue(db, root)))
        // Class-B UAF (cb3-5): a scrutinee RE-MATCHED by a NESTED `MatchSum` in an arm (`match s { Circle =>
        // match s { … } … }`, `s` an owned/inlined sum) is reclaimed by the INNER match's shell-drop already;
        // this ENCLOSING reclaim would deep-drop the SAME handle a 2nd time → double-free. Suppress the
        // enclosing reclaim when the scrutinee recurs as a nested-match scrutinee — the innermost reclaim
        // fires once, rc balances. Leak-safe if the nested match is only in SOME arms (a non-re-matching arm
        // then leaves the shell un-reclaimed = a value-correct leak, never a UAF).
        && !cont_rematches_scrutinee(db, scrutinee, root)
}

/// Whether the outer MatchSum's owned `scrutinee` is RE-MATCHED — appears as the scrutinee of a NESTED
/// `MatchSum` within `root`'s arm bodies (Class-B UAF cb3-5). Keyed on the scrutinee NODE (a CSE-shared
/// re-match) and, when the scrutinee is a `Param`/`LocalRef`, its BINDER (a distinct-node same-binder
/// re-match). Used by [`sum_shell_reclaim_ok`] to SUPPRESS the enclosing shell-reclaim so the innermost
/// match's reclaim is the sole drop of the shared owned scrutinee (no double-free).
fn cont_rematches_scrutinee(db: &mut Db, scrutinee: StructId, root: &crate::core::SumCont) -> bool {
    let tgt_binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    let mut seen = HashSet::new();
    cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, root, &mut seen)
}

fn cont_rematches_scrutinee_cont(
    db: &mut Db,
    scrutinee: StructId,
    tgt_binder: Option<StructId>,
    cont: &crate::core::SumCont,
    seen: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            expr_rematches_scrutinee(db, scrutinee, tgt_binder, *body, seen)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            expr_rematches_scrutinee(db, scrutinee, tgt_binder, *cond, seen)
                || expr_rematches_scrutinee(db, scrutinee, tgt_binder, *body, seen)
                || cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, els, seen)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, then_, seen)
                || cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, els, seen)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| cont_rematches_scrutinee_cont(db, scrutinee, tgt_binder, &a.cont, seen)),
    }
}

fn expr_rematches_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    tgt_binder: Option<StructId>,
    id: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    // A nested match — Sum, List, OR scalar — RE-MATCHING the same scrutinee (by node id, or by binder when
    // the scrutinee is a `Param`/`LocalRef`). Covers Class-B for both `MatchSum` (cb3-5) and `MatchList`
    // (breaker's runtime-list re-match UAF): the inner match reclaims/re-reads the shared owned scrutinee, so
    // the enclosing reclaim must be suppressed.
    let nested_scrut = match core_of(db, id) {
        Core::MatchSum { scrutinee: s2, .. }
        | Core::MatchList { scrutinee: s2, .. }
        | Core::Match { scrutinee: s2, .. } => Some(s2),
        _ => None,
    };
    if let Some(s2) = nested_scrut {
        if s2 == scrutinee {
            return true;
        }
        if let Some(b) = tgt_binder
            && matches!(core_of(db, s2), Core::Param { binder } | Core::LocalRef { binder } if binder == b)
        {
            return true;
        }
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| expr_rematches_scrutinee(db, scrutinee, tgt_binder, c, seen))
}

/// Whether the outer `MatchList`'s `scrutinee` is RE-MATCHED (appears as the scrutinee of a NESTED match)
/// within any arm body/guard — the list analogue of [`cont_rematches_scrutinee`]. Used by
/// [`list_shell_reclaim_slot`] to SUPPRESS the enclosing list shell-reclaim so the shared owned list is not
/// deep-dropped while a nested `match xs` still reads it (breaker's Class-B runtime-list re-match UAF).
fn list_arms_rematch_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
) -> bool {
    let tgt_binder = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        _ => None,
    };
    let mut seen = HashSet::new();
    arms.iter().any(|a| {
        expr_rematches_scrutinee(db, scrutinee, tgt_binder, a.body, &mut seen)
            || a.guard
                .is_some_and(|g| expr_rematches_scrutinee(db, scrutinee, tgt_binder, g, &mut seen))
    })
}

/// Whether EVERY variant of the sum type `sum` carries either NO payload (nullary) or a SCALAR payload
/// (Int/Bool/Float — copied off, never a heap handle). Used to gate the `MatchSum` owned-shell reclaim: it
/// is only sound to drop the scrutinee shell after the match when NO arm can BORROW a heap payload handle
/// out of the shell (a borrowed compound/list/string payload is threaded into the arm body — often a
/// recursive walk — and OUTLIVES the match block, so freeing the shell would free it mid-use → a UAF, the
/// HOL-kernel `term-eq (Comb x y)` regression v-patterns caught). If every payload is a scalar or absent,
/// no handle aliases the shell and the drop is safe. Conservative: an `(Option Int64)` / all-scalar-enum
/// qualifies (the reported List.at/Map.lookup leak), a compound-payload sum does NOT (left un-dropped — a
/// residual leak there, never a double-free). This mirrors the SumExpect gate's scalar-payload arm applied
/// to EVERY variant. Returns false for a non-sum or an unresolvable payload (reject-don't-miscompile).
/// (FIND3) Whether `ty` is a PRODUCT (Tuple/Record) whose fields are ALL SCALAR (Int/Bool/Float). Such a
/// product is destructured field-by-field into COPIED immediates (get-int/get-bool) — no field is a heap
/// handle that could alias into an arm's rebuilt compound (the FBIP-reuse hazard `sum_cont_arm_constructs_
/// compound` guards). So its shell is safely deep-droppable after a scalar-extracting match EVEN WHEN the arm
/// builds a compound. The product analog of [`sum_has_only_scalar_payloads`] (which bails on non-`Sum` types,
/// so a bare Tuple/Record scrutinee never matched it — fib fast-doubling's `(Tuple Int Int)`). Conservative:
/// ANY heap field → false (a heap field could alias the arm's compound = v-mem-safety's heap-extracted
/// must-hold). ONE level (a nested-product field is heap → false). Caller ANDs Owned + dead-after-destructure.
fn ty_is_all_scalar_product(db: &mut Db, ty: &Ty) -> bool {
    fn is_scalar(t: &Ty) -> bool {
        matches!(t.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
    }
    match ty.strip_nominal() {
        Ty::Tuple(elems) => !elems.is_empty() && elems.iter().all(is_scalar),
        Ty::Record(fields) => !fields.is_empty() && fields.values().all(is_scalar),
        _ => {
            let _ = db;
            false
        }
    }
}

fn sum_has_only_scalar_payloads(db: &mut Db, sum: &Ty) -> bool {
    let stripped = sum.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return false;
    };
    let Some(n) = db.type_decl_by_occ(*decl).map(|td| td.variants.len()) else {
        return false;
    };
    (0..n as u32).all(|disc| match variant_payload_ty_at(db, &stripped, disc) {
        // No payload (nullary variant) — nothing to borrow.
        None => true,
        // A scalar payload is copied off (get-int/get-bool), never a handle aliasing the shell.
        Some(ty) => matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_)),
    })
}

/// The type reached by a `Payload` step whose FULL path (from the root, INCLUDING this `Payload`) is
/// `prefix`, given the current sub-value type `cur`. Prefer the RECORDED entered-variant payload type in
/// `recorded` (keyed by the absolute path — written as an enclosing switch descended into a specific
/// variant); this is authoritative because it carries WHICH variant was entered, which the flat path alone
/// cannot. Fall back to variant 0 (`sum_single_payload_ty`) only when nothing is recorded (the root switch,
/// whose `cur` IS the scrutinee's type). A NOMINAL newtype `Payload` is a static unwrap to its inner type.
fn payload_step_ty(
    db: &mut Db,
    scrutinee: StructId,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Ty {
    payload_step_ty_of(db, scrutinee, None, cur, prefix, recorded)
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
    root_scrutinee: StructId,
    scrutinee: Option<StructId>,
    cur: &Ty,
    prefix: &[crate::core::PathStep],
    recorded: &HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Ty {
    if let Some(t) = recorded.get(&(root_scrutinee, prefix.to_vec())) {
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
            | (crate::core::PathStep::Elem(i), Core::ListNew { elems }) => *elems.get(*i)?,
            // A multi-payload variant's payloads: after the `Payload` no-op above, `cur` is the `SumNew`
            // and `Elem(i)` selects the i-th payload — the constant twin of `sum-payload` + `arr-get i`.
            (
                crate::core::PathStep::Elem(i),
                Core::SumNew {
                    payloads: elems, ..
                },
            ) => *elems.get(*i)?,
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
    scrutinee: StructId,
    root: &Ty,
    path: &[crate::core::PathStep],
    recorded: &HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Ty {
    let mut cur = root.clone();
    let mut prefix: Vec<crate::core::PathStep> = Vec::with_capacity(path.len());
    for step in path {
        prefix.push(*step);
        cur = match step {
            crate::core::PathStep::Payload => {
                payload_step_ty(db, scrutinee, &cur, &prefix, recorded)
            }
            crate::core::PathStep::Elem(i) => match cur.strip_nominal() {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(e) => e.clone(),
                    None => return Ty::Any,
                },
                // A record erases to a tuple in sorted-field order — field-slot `i` is
                // `fields.values().nth(i)` (same index space as `Core::Record`/`Core::Proj`).
                Ty::Record(fields) => match fields.values().nth(*i) {
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
            crate::core::PathStep::TupleRestFrom(k) => match cur.strip_nominal() {
                Ty::Tuple(elems) => Ty::Tuple(elems.get(*k..).unwrap_or(&[]).to_vec().into()),
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
    let sub = ty_at_path_recorded(db, scrutinee, &root, path, &out.sum_path_types);
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
    // Whether any REAL heap read (`sum-payload`/`arr-get`/`vec-get`) has been emitted so far — i.e. the
    // value now on the stack came out of a heap slot (boxed) rather than being the scrutinee's own top-level
    // value. A `Payload` step through an ERASED single-variant newtype (`Ty::Nominal`) emits NOTHING (the
    // box is erased — the value IS the payload, a compile-time reinterpretation), so it does NOT flip this.
    // Used below to decide the enum-disc unbox: a boxed enum-disc needs `get-int`, a top-level one is already
    // the raw i32. Without the nominal no-op, a match through an erased outer newtype (`(Outer.Wrap (Inner…))`,
    // `Outer` a closed single-variant sum) emitted a spurious `sum-payload` and read the discriminant one
    // level too deep — a silent wrong-variant dispatch (wasm-only differential).
    let mut read_from_heap = false;
    for step in path {
        prefix.push(*step);
        match step {
            // A `Payload` step through an ERASED single-variant newtype (`Ty::Nominal`) is a runtime no-op:
            // the newtype box is erased (`infer::newtype_underlying`), so the value already IS the payload —
            // emit nothing, just peel one nominal layer off the type cursor (the `Core::SumPayload` binder
            // path is erased at construction the same way, `lower.rs erase_nominal_steps`). A `Payload` over a
            // REAL boxed sum reads `sum-payload`.
            crate::core::PathStep::Payload if matches!(cur, Ty::Nominal { .. }) => {
                cur = match &cur {
                    Ty::Nominal { inner, .. } => (**inner).clone(),
                    _ => unreachable!("guarded by the matches! above"),
                };
            }
            crate::core::PathStep::Payload => {
                out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                read_from_heap = true;
                cur = payload_step_ty_of(
                    db,
                    scrutinee,
                    Some(scrutinee),
                    &cur,
                    &prefix,
                    &out.sum_path_types,
                );
            }
            crate::core::PathStep::Elem(i) => {
                out.push(Lir::ConstI32(*i as i32));
                read_from_heap = true;
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
                        // A record erases to a tuple in sorted-field order, so field-slot `i` is
                        // `fields.values().nth(i)` — same index space as `Core::Record`/`Core::Proj`. Tracking
                        // it (not falling to `Ty::Any`) grounds a narrow int/float record field's width.
                        Ty::Record(fields) => fields.values().nth(*i).cloned().unwrap_or(Ty::Any),
                        _ => Ty::Any,
                    };
                }
            }
            crate::core::PathStep::RestFrom(_) => {} // never on a sum-disc path
            crate::core::PathStep::TupleRestFrom(_) => {} // never on a sum-disc path
        }
    }
    if sub_is_enum {
        // The sub-value is an enum-disc value. At the TOP level it is already the raw discriminant i32.
        // At a NESTED position (an actual Payload/Elem heap read happened) it was boxed as an int, so
        // `get-int` recovers the i64 cell and `i32.wrap_i64` narrows it to the discriminant i32. An erased
        // newtype wrapper contributes NO heap read (`read_from_heap` stays false), so an enum-disc reached
        // only through erased nominal Payloads is still top-level (raw i32) — NOT a `!path.is_empty()` test,
        // which would wrongly `get-int` a raw enum-disc behind an erased `(Outer.Wrap Color)` wrapper.
        if read_from_heap {
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
    // Site A: skip the preservation retain for a loop param reassigned-without-drop this iteration (its
    // borrow reads the live slot; the final vec-drop consumes the sole ref). Else default retain.
    // 05:18721 GATE (narrowed): skip the per-occurrence retain dup ONLY when it is PROVABLY SURPLUS — the
    // occurrence is in `surplus_skippable_dups` (a boundary-owned rest-mint-consumed MatchList scrutinee with
    // no other consume, whose RestFrom vec-drop already has its own balancer, the emit.rs:3098 preservation
    // dup). This REPLACES the earlier `!body_is_boundary_owned`-ALONE trial gate, which stripped LOAD-BEARING
    // retains in every boundary-owned body → 159 corpus UAFs; the set-membership skips exactly the redundant
    // dups and keeps the load-bearing ones (see `Emit::surplus_skippable_dups`).
    if out.dup_sites.contains(&id)
        && !out.loop_reassign_no_dup.contains(&slot)
        && !out.surplus_skippable_dups.contains(&id)
    {
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
/// Emit a runtime `ast-print`/`ast-encode` (op 92/93): push the `Ast` operand, bake the compile-time `discs`
/// descriptor into a FRESH `Bytes` buffer on top, then call `op` — which BORROWS both the Ast handle and the
/// discs buffer (the runtime reads them via `op_bytes_get`, dropping neither) and returns a fresh String/Bytes.
/// RECLAMATION: the always-fresh `discs` buffer, AND an OWNED-temporary Ast operand (a constructed
/// `(Ast.Int …)` / call result — `(= (Ast.encode a) (Ast.encode b))` leaked both per side), are otherwise
/// never dropped → leak. Stash the operand (iff owned) and the discs buffer, run the borrowing op, then drop
/// them. A BORROWED operand (param / kept-local) is left to its owner (dropping it would be a double-free).
/// The emitted RESULT bytes are byte-identical to the un-reclaimed form — only dead temporaries are freed.
fn emit_ast_op_with_discs(
    db: &mut Db,
    operand: StructId,
    discs: &[u8],
    op: &'static str,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let reclaim_operand = matches!(
        heap_operand_ownership(db, operand),
        Ok(HandleOwnership::Owned)
    );
    let ast_slot = base;
    let discs_slot = base + 1;
    *high = (*high).max(discs_slot + 1);
    scratch_ty.insert(ast_slot, ValType::I32);
    scratch_ty.insert(discs_slot, ValType::I32);
    emit(
        db,
        operand,
        slots,
        discs_slot + 1,
        high,
        scratch_ty,
        layout,
        out,
    )?; // [ast]
    if reclaim_operand {
        out.push(Lir::LocalTee(ast_slot)); // [ast], ast_slot = the owned Ast operand
    }
    out.push(Lir::ConstI32(discs.len() as i32));
    out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [ast, discs-buf]
    for (j, &byte) in discs.iter().enumerate() {
        out.push(Lir::ConstI32(j as i32));
        out.push(Lir::ConstI32(byte as i32));
        out.push(Lir::CallImport(OP_BYTES_SET)); // [ast, discs-buf]
    }
    out.push(Lir::LocalTee(discs_slot)); // [ast, discs-buf], discs_slot = the fresh discs buffer
    out.push(Lir::CallImport(op)); // → [string|bytes] (borrows ast + discs)
    out.push(Lir::LocalGet(discs_slot)); // [result, discs-buf]
    out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the always-fresh discs buffer)
    if reclaim_operand {
        out.push(Lir::LocalGet(ast_slot)); // [result, ast]
        out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the owned Ast operand)
    }
    Ok(())
}

/// §2d STATIC BYTES/STRINGS (`DESIGN-static-data.md`): if `id` is a fully-constant flat-byte-payload value
/// present in the build-once table (`layout.static_bytes`), emit a BARE `global.get` of its module global
/// and return `true`. The value was built ONCE at instantiation (the `CORE_SEC_START` init) and marked
/// IMMORTAL (`mark-immortal`), so a plain read is all a use needs: `op_dup`/`op_drop` are NO-OPs on an
/// immortal node, so the consumer treating the handle as owned and dropping it is harmless (never frees the
/// shared static → no UAF), and `node_rc == IMMORTAL` makes FBIP path-copy so the static is never mutated
/// in place. No dup, no drop, no per-eval `bytes-alloc`+`bytes-set`.
///
/// Covers a constant `Bytes` (a `Core::BytesOf` of constants OR a baked `Core::ConstBytes`, via
/// `constant_bytes_value`) AND a constant `String` (a `Core::ConstStr`, via `constant_string_value`) — a
/// Cadenza `String` value IS the identical flat UTF-8 byte-leaf a `Bytes` is (`str-new`'s rep), built by
/// the same `bytes-alloc`+`bytes-set`, so both hoist through this one path. The table is interned BY
/// CONTENT, so a `String` and a `Bytes` with equal bytes share the ONE immortal global (sound: both are
/// i32 handles to the same leaf rep). Returns `false` (build inline) for a runtime literal or a program
/// with no static table, keeping every non-hoisted program byte-identical.
fn try_emit_static_bytes(db: &mut Db, id: StructId, layout: &Layout, out: &mut Emit) -> bool {
    if let Some(payload) = crate::lower::constant_bytes_value(db, id)
        .or_else(|| crate::lower::constant_string_value(db, id))
        && let Some(pos) = layout.static_bytes.iter().position(|b| *b == payload)
    {
        out.push(Lir::GlobalGet(pos as u32)); // [handle] — the once-built immortal static, owned-by-value
        return true;
    }
    false
}

/// §2d STATIC COMPOUNDS (`DESIGN-static-data.md` increment 6): if `id` is a markable constant
/// `Tuple`/`Record`/small-`List` in the build-once table, emit a bare `global.get` of its module global and
/// return `true` (the routing is keyed by node id, so it is type-agnostic — a list uses the same table).
/// Compound globals are laid AFTER the static-bytes globals, so compound `pos`'s global index is
/// `static_bytes.len() + pos`. The tree was built ONCE (immortal, per-node marked) by the `start` init, so a
/// use just reads the handle (`op_dup`/`op_drop` no-op on the immortal root; FBIP path-copies). `false`
/// (build the compound inline per-eval, as before) for a non-tabled or runtime compound.
fn try_emit_static_compound(db: &mut Db, id: StructId, layout: &Layout, out: &mut Emit) -> bool {
    let _ = db;
    if let Some(pos) = layout.static_compounds.iter().position(|&c| c == id) {
        out.push(Lir::GlobalGet((layout.static_bytes.len() + pos) as u32)); // [handle] — immortal compound
        return true;
    }
    false
}

/// §2d increment 6: emit the IMMORTAL build of a markable constant compound `id` into the `start` init,
/// leaving its handle on the stack. Builds every node inline and marks it IMMORTAL per node (`mark-immortal`
/// is shallow, so the WHOLE tree must be marked to be census-excluded + drop-safe): `arr-alloc(n)` then, per
/// element, build its handle + `arr-set`, then `mark-immortal` the root array. Mirrors the runtime
/// `Core::Tuple`/`Core::Record` emit (a record IS a tuple at run time) but recurses for a nested compound
/// and marks each node. Self-contained — references no other global — so ordering across the init is
/// irrelevant. Called on a `Tuple`/`Record` (arr root) OR a small constant `List` (arr + `vec-of-arr`, both
/// nodes marked — see the `ListNew` arm) collected by `collect_static_compounds`.
fn emit_immortal_static(
    db: &mut Db,
    id: StructId,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::Tuple { elems } => {
            let elem_tys = match type_of(db, id).strip_nominal() {
                Ty::Tuple(ts) => Some(ts.clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit_immortal_elem(db, elem, elem_tys.as_ref().and_then(|ts| ts.get(i)), layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport("mark-immortal")); // [arr] — the tuple root, immortal
            Ok(())
        }
        Core::Record { fields } => {
            let field_tys = match type_of(db, id).strip_nominal() {
                Ty::Record(m) => Some((*m).clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(fields.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr] (a record IS a tuple at run time)
            for (i, (name, &value)) in fields.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                let declared = field_tys.as_ref().and_then(|m| m.get(name));
                emit_immortal_elem(db, value, declared, layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport("mark-immortal")); // [arr] — the record root, immortal
            Ok(())
        }
        // A NULLARY variant of a MIXED sum (`(Z)`/`(Nil)`) — a real heap node (`sum-new(disc, IMM_UNIT)`)
        // built ONCE, immortal (`is_markable_constant_sum_nullary`; the rsl1 leak-1 fix). SHALLOW
        // `mark-immortal` suffices: the sum root wraps the inline-unit sentinel `IMM_UNIT` (rc-free, no heap
        // child), so there is nothing deeper to mark — unlike the list/map/set roots that hold heap children.
        Core::SumNew { disc, payloads } if payloads.is_empty() => {
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            out.push(Lir::ConstI32(super::runtime_abi::IMM_UNIT as i32)); // [disc, unit]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [sum-handle]
            out.push(Lir::CallImport("mark-immortal")); // [sum-handle] — the nullary sum root, immortal
            Ok(())
        }
        // A PAYLOADED variant of a MIXED sum with ALL-CONSTANT payloads (`(Some 5)`, `(Cons 1 (list …))`) —
        // built ONCE immortal, mirroring the runtime `Core::SumNew` payload marshaling (`select.rs` emit) for
        // constants: 1 payload → the boxed handle IS the sum's payload; n → a tuple `arr` of boxed payloads.
        // Then `mark-immortal-DEEP` (op 96) — unlike the nullary SHALLOW mark, the payload(s) are HEAP CHILDREN
        // (the boxed scalar / built compound / arr), so a deep mark is needed to census-exclude the whole tree
        // (exactly like the const-list/map/set roots). `emit_immortal_elem` builds + shallow-marks each payload
        // (idempotent under the final deep mark). Collected by `is_markable_constant_sum_payloaded`.
        Core::SumNew { disc, payloads } => {
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            match payloads.len() {
                1 => {
                    // The single payload's boxed handle is passed to `sum-new` directly (no wrapping `arr`).
                    emit_immortal_elem(db, payloads[0], None, layout, out)?; // [disc, payload-handle]
                }
                n => {
                    // Multiple payloads: box each into a positional tuple `arr` (the runtime multi-payload shape).
                    out.push(Lir::ConstI32(n as i32)); // [disc, n]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, arr]
                    for (i, &p) in payloads.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32)); // [disc, arr, i]
                        emit_immortal_elem(db, p, None, layout, out)?; // [disc, arr, i, handle]
                        out.push(Lir::CallImport(OP_ARR_SET)); // [disc, arr]
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_NEW)); // [sum-handle]
            out.push(Lir::CallImport("mark-immortal-deep")); // deep — payload(s) are heap children
            Ok(())
        }
        // A constant list of ANY size (non-empty, not all-`Bool`) — built like a tuple (a flat `arr` of boxed
        // elements) then `vec-of-arr`. The build is UNIFORM across sizes: `arr-alloc(n)` + per-element build +
        // `arr-set`, then `vec-of-arr`. What differs is the node topology `vec-of-arr` produces — ≤32 reuses the
        // `arr` as the sole leaf under an 8-byte header; `>32` DRAINS the elements into ≤32-element trie leaves
        // and builds a radix trie (INTERNAL nodes minted inside the op, no compile-time handle). So the root is
        // marked with `mark-immortal-DEEP` (op 96), which transitively marks the whole structure — header + arr
        // leaf (≤32) OR spine + all trie leaves (>32) + every element handle — in ONE call, reaching the trie
        // internals a per-node shallow mark could not. Do NOT shallow-mark the `arr` before `vec-of-arr`: for
        // `>32` the arr shell is drained + dropped (a marked-immortal shell would be orphaned = a leak), and the
        // deep-mark on the result covers the reused-arr leaf for ≤32 anyway. Elements are shallow-marked as built
        // (`emit_immortal_elem`) — redundant with the final deep-mark (idempotent) but harmless. The all-`Bool`
        // PACK path (mints a fresh bit-leaf + drops the arr WITH the marked element boxes → orphaned leak) and the
        // empty-list `vec-empty` singleton are excluded upstream (`is_markable_constant_list`).
        Core::ListNew { elems } => {
            let elem_ty = match type_of(db, id).strip_nominal() {
                Ty::List(t) => Some((**t).clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit_immortal_elem(db, elem, elem_ty.as_ref(), layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport(OP_VEC_OF_ARR)); // [arr] → [list] (arr reused (≤32) or drained into a trie (>32))
            out.push(Lir::CallImport("mark-immortal-deep")); // [list] — transitively immortal (header/arr or spine/leaves + elems)
            Ok(())
        }
        // A constant MAP — built EXACTLY like the runtime `Core::MapNew` arm (map-empty + per-entry box key/value
        // by their types, rope-compact / list-key-canonicalize the key for CHAMP slot exactness, map-insert),
        // then ONE `mark-immortal-deep` on the final root. `map-insert` CONSUMES map+key+value (moves them into the
        // CHAMP, no copy), so there is no orphan-leak hazard — the deep-mark on the final root transitively marks
        // the whole CHAMP (HAMT spine + data-entry key/value handles + nested payloads). The keys/values build via
        // a FRESH minimal emit context (like `emit_immortal_elem`): empty slots, base 0, its own high-water +
        // scratch-type map — and since `collect_static_compounds` does NOT descend into a collected map root, no
        // key/value node is itself in `static_compounds`, so `emit` builds each inline (never routes to global.get).
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            out.push(Lir::CallImport(OP_MAP_EMPTY)); // [map]
            for &(k, v) in entries.iter() {
                let key_base = high; // start this entry's scratch above the running high-water (base 0 → high)
                emit(db, k, &slots, key_base, &mut high, &mut scratch_ty, layout, out)?; // [map, key]
                let key_boxed = box_op_for(db, k, &key_ty)?;
                emit_heap_store_tail(db, k, key_boxed, out); // [map, key-handle]
                if key_needs_compaction(db, k) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
                }
                if key_needs_canonicalize(db, k) {
                    emit_key_canonicalize(db, k, &key_ty, &mut high, &mut scratch_ty, out)?; // [map, canon-key]
                }
                let val_base = high;
                emit(db, v, &slots, val_base, &mut high, &mut scratch_ty, layout, out)?; // [map, key, val]
                let val_boxed = box_op_for(db, v, &val_ty)?;
                emit_heap_store_tail(db, v, val_boxed, out); // [map, key, val-handle]
                out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map'] (consumes map, key, val)
            }
            out.push(Lir::CallImport("mark-immortal-deep")); // [map] — transitively immortal (CHAMP spine + k/v)
            Ok(())
        }
        // A constant SET — the set analogue of the Map arm (CHAMP-minus-value-column): `set-empty` + per-element
        // box-by-type + rope-compact / list-element-canonicalize + `set-insert` (CONSUMES set+element, moves in,
        // no copy), then ONE `mark-immortal-deep` on the final root (marks the whole HAMT + element handles).
        Core::SetOf { elems, elem_ty } => {
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            out.push(Lir::CallImport(OP_SET_EMPTY)); // [set]
            for &e in elems.iter() {
                let elem_base = high;
                emit(db, e, &slots, elem_base, &mut high, &mut scratch_ty, layout, out)?; // [set, elem]
                let elem_boxed = box_op_for(db, e, &elem_ty)?;
                emit_heap_store_tail(db, e, elem_boxed, out); // [set, elem-handle]
                if key_needs_compaction(db, e) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
                }
                if key_needs_canonicalize(db, e) {
                    emit_key_canonicalize(db, e, &elem_ty, &mut high, &mut scratch_ty, out)?; // [set, canon-elem]
                }
                out.push(Lir::CallImport(OP_SET_INSERT)); // → [set'] (consumes set, elem)
            }
            out.push(Lir::CallImport("mark-immortal-deep")); // [set] — transitively immortal (CHAMP spine + elems)
            Ok(())
        }
        _ => Err(Reject::decline(
            "emit_immortal_static reached a non-markable node (only markable Tuple/Record/List/Map/Set are collected)"
                .to_string(),
        )),
    }
}

/// One element of an immortal static compound (see [`emit_immortal_static`]), leaving its handle on the
/// stack: a nested markable `Tuple`/`Record` recurses (its whole subtree is built + marked immortal); a
/// constant `Bytes`/`String` builds its OWN inline immortal leaf (self-contained — not the shared static-
/// bytes global, so init ordering is irrelevant + a tiny duplication is harmless); a constant scalar emits
/// its value, boxes it by the declared element type, and marks the freshly-boxed node immortal (a `Unit`
/// stores the inline `IMM_UNIT` sentinel — no heap node, no mark).
fn emit_immortal_elem(
    db: &mut Db,
    elem: StructId,
    declared: Option<&Ty>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    match core_of(db, elem) {
        // A nested constant compound (Tuple/Record) OR a nested constant mixed-sum (`(Some 5)`/`(Cons …)`/
        // `(Nil)`) OR a nested constant LIST (`(list (list 1) (list 2))`, a list element of a tuple/record/
        // sum-payload): recurse to `emit_immortal_static`, which builds the child + marks it (the parent's
        // final `mark-immortal[-deep]` re-marks idempotently). The `SumNew`/`ListNew` cases are what make
        // nested-collection immortals work — a sum/list element of a list/tuple/record, or a recursive-sum
        // spine, builds once. Without the `ListNew` arm a nested list falls to the `_` scalar path below,
        // whose `box_op` returns `None` for a list handle → the list is left UNMARKED = a census leak.
        Core::Tuple { .. }
        | Core::Record { .. }
        | Core::SumNew { .. }
        | Core::ListNew { .. }
        | Core::MapNew { .. }
        | Core::SetOf { .. } => emit_immortal_static(db, elem, layout, out),
        _ => {
            if let Some(payload) = crate::lower::constant_bytes_value(db, elem)
                .or_else(|| crate::lower::constant_string_value(db, elem))
            {
                out.push(Lir::ConstI32(payload.len() as i32));
                out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [buf]
                for (bi, &b) in payload.iter().enumerate() {
                    out.push(Lir::ConstI32(bi as i32)); // [buf, i]
                    out.push(Lir::ConstI32(b as i32)); // [buf, i, byte]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // [buf]
                }
                out.push(Lir::CallImport("mark-immortal")); // [leaf] — immortal
                return Ok(());
            }
            // A constant scalar (Int/Bool/Unit): emit the value (no scratch — a constant needs none), box it,
            // and mark the box. A fresh empty emit context is safe because a `Core::ConstInt`/`ConstBool`/
            // `Unit` pushes only an inline constant.
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            emit(db, elem, &slots, 0, &mut high, &mut scratch_ty, layout, out)?; // [.., value]
            let boxed = match declared {
                Some(d) => box_op_for(db, elem, d)?,
                None => box_op(db, elem)?,
            };
            emit_heap_store_tail(db, elem, boxed, out); // [.., handle] (box, or the unit sentinel)
            if boxed.is_some() {
                out.push(Lir::CallImport("mark-immortal")); // mark the freshly-boxed scalar node
            }
            Ok(())
        }
    }
}

/// Build the `start`-init `Lir` for all static compounds (`DESIGN-static-data.md` §2d, increment 6): for each
/// entry in `layout.static_compounds`, emit its immortal tree ([`emit_immortal_static`]) and `global.set` it
/// to `static_bytes.len() + k` (compound globals follow the byte globals). Called by the backend (which has
/// `Db` — the tree walk needs `core_of`/`type_of`/box selection) and stored in the `Layout`, so
/// `core_module_impl` (which has no `Db`) can APPEND it to the static-bytes init in the START function.
/// Empty `Vec` when there are no static compounds (no additions → byte-identical).
pub fn build_static_compound_init(
    db: &mut Db,
    compounds: &[StructId],
    byte_base: usize,
    layout: &Layout,
) -> Result<Vec<Lir>, Reject> {
    let mut out = Emit::new();
    for (k, &root) in compounds.iter().enumerate() {
        emit_immortal_static(db, root, layout, &mut out)?; // [handle]
        out.push(Lir::GlobalSet((byte_base + k) as u32)); // store the once-built immortal handle → []
    }
    Ok(std::mem::take(&mut *out))
}

/// The EXACT runtime-op set the static-compound init (`build_static_compound_init` → `emit_immortal_static`)
/// will emit, derived by a DRY-RUN into a throwaway `Emit` + scanning its `CallImport`s. This makes the
/// module import set PRECISE (only the ops each compound's SHAPE actually builds) instead of the prior
/// unconditional over-approximation (which force-imported the full arr/box/bytes/vec/map/set/canonicalize
/// batch whenever ANY static compound existed — leaving e.g. map/set/vec/bytes imports DEAD in a program
/// whose only constants are sums/tuples). No mirror-divergence: this runs the SAME emit path
/// (`emit_immortal_static`), so the collected op set is exactly what the real init emits. A compound that
/// DECLINES in the dry-run is not built by the real init either (`build_static_compound_init` propagates the
/// same `Reject`, so no module is emitted), so ignoring the dry `Err` never under-collects an op that the
/// real init actually emits. The dry `Emit` is discarded; `emit_immortal_static` only reads/memoizes `db`.
pub fn collect_static_compound_ops(
    db: &mut Db,
    compounds: &[StructId],
    layout: &Layout,
) -> std::collections::BTreeSet<&'static str> {
    let mut ops = std::collections::BTreeSet::new();
    for &root in compounds {
        let mut probe = Emit::new();
        if emit_immortal_static(db, root, layout, &mut probe).is_ok() {
            for instr in probe.code.iter() {
                if let Lir::CallImport(op) = instr {
                    ops.insert(*op);
                }
            }
        }
    }
    ops
}

/// Recognize a nested `(if (= X k0) b0 (if (= X k1) b1 … default))` chain — an integer-equality
/// dispatch a user wrote as chained `if`s rather than a `match` — and lift it to the SAME
/// `(scrutinee, arms)` shape a `Core::Match` carries, so it inherits the match backend's dense-range
/// `br_table` (and 2-arm `select`) lowering instead of emitting an O(n) `if (== k)` cascade. Rust gets
/// this jump-table for free from LLVM; wasm does not, so this is a wasm-specific missed opt.
///
/// The scrutinee `X` must be a REUSABLE scalar (a `Param`/`LocalRef` binder or a constant — the same
/// values `reusable_scalar_src` accepts) and the SAME binder in every arm's test (`(= X k)` or the
/// flipped `(= k X)`), each `k` a distinct compile-time `ConstInt` fitting `i64`; the innermost non-`if`
/// (or non-matching) else is the DEFAULT arm (a synthesized trailing `Wild`). Returns `None` (fall
/// through to the ordinary `if` lowering) unless the chain has ≥3 distinct-const arms — below that the
/// existing branchless-`select`/`if` lowering is already at least as good, and the match path would only
/// add overhead. The synthesized arms REUSE the original body `StructId`s (no AST synthesis), so the
/// lowering is byte-for-byte the value the `if`-chain would have produced.
///
/// Soundness: an `if`-chain tests the arms IN ORDER and takes the first whose `== k` holds; the
/// distinct-`k` requirement means at most one arm matches any value, so order is irrelevant and the
/// synthesized first-wins match is equivalent. The default covers every other value (the chain's final
/// else). A guarded/non-equality/mixed-binder link ends the chain (becomes the default), never a wrong
/// arm. Only INTEGER scrutinees qualify (`br_table`/match dispatch is integer) — a Bool `X` has ≤2
/// values so it never reaches the ≥3 threshold.
fn if_chain_as_int_match(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
) -> Option<(StructId, Vec<crate::core::MatchArm>)> {
    // The scalar binder + constant of a single `(= X k)` / `(= k X)` equality test, or `None` if `id`
    // is not an unguarded integer-equality of a reusable scalar binder against a constant.
    fn eq_binder_const(db: &mut Db, id: StructId) -> Option<(StructId, StructId, i64)> {
        let Core::Compare {
            op: Prim::Eq,
            lhs,
            rhs,
        } = core_of(db, id)
        else {
            return None;
        };
        // One operand a reusable scalar binder (Param/LocalRef), the other a ConstInt in i64 range.
        let binder_node = |db: &mut Db, n: StructId| -> Option<StructId> {
            match core_of(db, n) {
                Core::Param { .. } | Core::LocalRef { .. } => Some(n),
                _ => None,
            }
        };
        let const_i64 = |db: &mut Db, n: StructId| -> Option<i64> {
            match core_of(db, n) {
                Core::ConstInt(v) => v.to_i64(),
                _ => None,
            }
        };
        // The binder KEY (its slot binder StructId) identifies which variable is switched on — the
        // stable identity used to require every link tests the SAME variable.
        let key_of = |db: &mut Db, b: StructId| -> Option<StructId> {
            match core_of(db, b) {
                Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
                _ => None,
            }
        };
        if let (Some(b), Some(k)) = (binder_node(db, lhs), const_i64(db, rhs)) {
            return Some((b, key_of(db, b)?, k));
        }
        if let (Some(k), Some(b)) = (const_i64(db, lhs), binder_node(db, rhs)) {
            return Some((b, key_of(db, b)?, k));
        }
        None
    }

    // The head must be an equality test; record its scrutinee node + binder key.
    let (scrut, key, k0) = eq_binder_const(db, cond)?;
    // Only INTEGER scrutinees dispatch via match/br_table.
    if !matches!(type_of(db, scrut).strip_nominal(), Ty::Int(_)) {
        return None;
    }
    let mut arms: Vec<crate::core::MatchArm> = Vec::new();
    let mut seen: Vec<i64> = Vec::new();
    arms.push(crate::core::MatchArm {
        probe: crate::core::Probe::Int(crate::ast::IntValue::from_i64(k0)),
        guard: None,
        body: then_,
    });
    seen.push(k0);
    // Walk the else-chain: each link must be an `(if (= X k) body else')` on the SAME binder key with a
    // fresh constant. The first link that is NOT such a test becomes the default (wildcard) arm.
    let mut cur_else = else_;
    while let Core::If {
        cond: c2,
        then_: t2,
        else_: e2,
    } = core_of(db, cur_else)
    {
        // The link must be `(= X k)` on the SAME binder with a fresh constant; a different variable, a
        // duplicate const, or a non-equality cond ends the chain (this whole `if` becomes the default).
        match eq_binder_const(db, c2) {
            Some((_, k2, kv)) if k2 == key && !seen.contains(&kv) => {
                arms.push(crate::core::MatchArm {
                    probe: crate::core::Probe::Int(crate::ast::IntValue::from_i64(kv)),
                    guard: None,
                    body: t2,
                });
                seen.push(kv);
                cur_else = e2;
            }
            _ => break,
        }
    }
    // Need ≥3 const arms for the match lowering to be worth it (a 2-arm chain already selects/ifs well).
    if arms.len() < 3 {
        return None;
    }
    // The remaining `cur_else` is the DEFAULT arm (covers every other value) — a synthesized wildcard.
    arms.push(crate::core::MatchArm {
        probe: crate::core::Probe::Wild,
        guard: None,
        body: cur_else,
    });
    Some((scrut, arms))
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
    // WARNING: The probe's NUMERIC value (`to_i64()`), NOT its bit pattern (`to_i64_bits()`), is what `value_range`
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
            // Evaluate the scrutinee ONCE into a scratch slot; the arm bodies and later probes run ABOVE
            // that live slot (it must survive every probe). The scrutinee's own emit uses `slot+1` as its
            // floor, and may itself claim MORE scratch — a runtime `value-eq`/`MatchSum` scrutinee
            // (`(match (= (mk n) (mk 3)) …)`) stashes i32 heap handles in slots the high-water records.
            // So the probe chain starts at the high-water the scrutinee emit REACHED (`*high`), NOT a bare
            // `slot+1`: reusing a scrutinee-scratch slot the value-eq typed i32 for a branch's i64
            // iteration arithmetic would force one wasm local to two types (invalid module).
            //
            // The spill slot is `base` UNLESS `base` was already RECORDED (in a sibling operand's emit) at
            // a DIFFERENT width than this scrutinee: when the match is an OPERAND nested in an op/arg list
            // (`Bytes.concat(…, b1(match op with …))`), an earlier sibling arg (a `sum-payload`/`arr-get`
            // i32 handle) may have typed `base` as i32, while a scalar match scrutinee is i64 — those temps
            // have DISJOINT liveness (the payload handle is dead by the match) but a wasm local carries ONE
            // declared type, so writing the i64 scrutinee into the i32-typed `base` yields `type mismatch:
            // expected i32, found i64` (an invalid module — the emit-db `wasm-op` idiomatic-`match` bug).
            // Mirror the `MatchSum` scrutinee-spill: a slot at `*high` is guaranteed never pre-typed, so
            // spill THERE when `base` already carries a conflicting width. A scalar scrutinee whose `base`
            // is untyped or already matches keeps `slot == base` (byte-identical to before).
            let slot = match scratch_ty.get(&base) {
                Some(&existing) if existing != scrut_vt => {
                    let s = *high;
                    *high = s + 1;
                    s
                }
                _ => base,
            };
            *high = (*high).max(slot + 1);
            scratch_ty.insert(slot, scrut_vt);
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
        // recovered from `result_it` (an Int result) or the block valtype. A FLOAT result (`block_ty` is
        // `f32`/`f64`) must ground a bare-`ConstFloat` arm to THAT width via `emit_branch` — otherwise a
        // bare float literal arm defaults to `Float64` and emits `f64.const` under an `f32`-typed select →
        // an INVALID module (the all-literal-arm Float32 match: `(: (match n (0 1.5) (_ 0.25)) Float32)`,
        // which routes here as a 2-arm select of two trap-free literal arms). `result_it` is Int-only, so
        // without the float case `res_ty` fell to `Bool` and `emit_branch` never grounded the ConstFloat.
        // Read the float width off the (already-solved) `block_ty`; a non-float non-int result is `Bool`
        // (its ConstBool leaf is always i32, no width to reconcile).
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => match block_ty {
                BlockType::Val(ValType::F32) => Ty::Float(crate::ty::FloatTy::fixed(32)),
                BlockType::Val(ValType::F64) => Ty::Float(crate::ty::FloatTy::fixed(64)),
                _ => Ty::Bool,
            },
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
        crate::core::Probe::Str(_) | crate::core::Probe::Bytes(_) => {
            unreachable!(
                "a string/byte-literal probe folds or desugars to a value-eq if-chain; it is never \
                 emitted as a runtime scalar probe"
            )
        }
        // A runtime char-literal probe (Char-rep 3/N): the scrutinee is the char's i32 code-point slot, so
        // test it against THIS literal's code point with `i32.eq` — the same `const ; eq` the nonzero-Int
        // path uses. `it` is `int_ty_of(Char)` = signed-32, so `m.slot32` is true → `i32.eq`. (A constant
        // char scrutinee still folds in `lower`; this is the runtime path a `Char` scrutinee reaches now
        // that `is_scalar` includes `Ty::Char` — 2/N.) `#\u+0000` (code point 0) compares by `const 0 ; eq`
        // like any other value; no `eqz` special-case needed.
        crate::core::Probe::Char(c) => {
            let m = Machine::of(it);
            out.push(m.konst(*c as u32 as i64));
            out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
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
        // A CHAR scrutinee is an i32 code-point slot (`valtype_of(Ty::Char) = I32`, Char-rep 1/N), so a
        // runtime char-literal `match` dispatches on it as an i32 scalar (Char-rep 3/N). `is_scalar` (2/N)
        // routes the char scrutinee here; the per-probe test (`emit_probe_condition`) compares it to each
        // char literal's code point with `i32.eq`.
        Ty::Char => Some(ValType::I32),
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
///
/// Returns `(arm_slots, len_slot, arm_base, owned_stash)` — the arm-body slot map (scrutinee handle bound),
/// the `vec-len` slot, the scratch floor past both, and the fresh owned-temporary handle slot for the
/// post-arms shell reclaim (`None` for a resident param/binding — see `list_shell_reclaim_slot`).
type ListMatchScrutinee = (HashMap<StructId, u32>, u32, u32, Option<u32>);

#[allow(clippy::too_many_arguments)]
fn materialize_list_match_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    slots: &HashMap<StructId, u32>,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<ListMatchScrutinee, Reject> {
    // `owned_stash` = the fresh slot holding a COMPUTED (non-resident) scrutinee handle — the shell-reclaim
    // (below, at the emit sites) drops it after the arms when it is an owned temporary, mirroring the
    // `MatchSum` owned-shell reclaim. `None` for a resident `Param`/`LocalRef` (its owner drops it).
    let (arm_slots, handle_slot, owned_stash) = match reusable_handle_slot(db, scrutinee, slots) {
        // Resident handle: the arms read the owner slot directly; `slots` already maps the binder there,
        // so `emit(scrutinee)` (a `Param`/`LocalRef`) resolves to it. No copy, no fresh handle scratch.
        Some(owner) => (slots.clone(), owner, None),
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
            (m, handle_slot, Some(handle_slot))
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
    Ok((arm_slots, len_slot, arm_base, owned_stash))
}

/// Whether a `MatchList`'s owned-temporary scrutinee shell can be reclaimed (dropped) after its arms — the
/// list twin of the `MatchSum` owned-shell reclaim. Sound ONLY when: a fresh owned temporary was stashed
/// (`owned_stash`), the scrutinee is an OWNED handle (not a borrowed param/binding its owner drops), we are
/// NOT in a self-loop tail position (a `return_call`/`br` arm never reaches the post-match drop — leak,
/// harmless — and the stash slot is reused next iteration; conservative skip mirrors `MatchSum`'s
/// `arms_tail_call` guard), and NO arm BORROWS a heap sub-value OUT of the shell that could alias into it.
///
/// The reclaim is a single DEEP `drop` emitted AFTER the selected arm's body has fully run, so every borrow
/// USED DURING arm evaluation is already dead — the only unsafe reference is one that a heap sub-value read
/// materialized and that OUTLIVES the drop (returned as the result, or an inner-match scrutinee handle). A
/// SCALAR element (a `vec-get`+`get-int` COPIES the scalar out) never materializes such a handle; a heap
/// element DESTRUCTURED all the way down to scalars (`(list (tuple a _) …)` → nested `arr-get` bottoming in
/// `get-int`) likewise materializes no live heap handle, so the shell + its (transitively unreferenced) heap
/// sub-structure deep-drops with no live alias. But an arm that reads a heap element/field handle AS A VALUE
/// (`(list r1 r2)` binding whole records, or returning an element) BORROWS into the shell — dropping it would
/// free a still-referenced value (the sread-UAF floor, same restriction `sum_has_only_scalar_payloads`
/// enforces for sums). `arm_borrows_heap_subvalue` detects exactly those borrowing reads; a `RestFrom` tail
/// (`(list _ .. r)`) is EXCLUDED — its `dup`+`vec-drop` yields a FRESH owned sublist that does not alias the
/// shell's ownership, so a heap-element rest binder does not block the shell reclaim.
fn list_shell_reclaim_slot(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
    owned_stash: Option<u32>,
    tail: TailPos,
    never_diverges: bool,
) -> Option<u32> {
    let slot = owned_stash?;
    if never_diverges || matches!(tail, TailPos::Tail(Some(_))) {
        return None;
    }
    if !matches!(type_of(db, scrutinee), Ty::List(_)) {
        return None;
    }
    // A borrowing heap sub-value read in ANY arm aliases into the shell → a deep drop would UAF. (A scalar
    // element, or a heap element fully destructured to scalars, reads no live handle.)
    if arms.iter().any(|a| {
        arm_borrows_heap_subvalue(db, a.body)
            || a.guard.is_some_and(|g| arm_borrows_heap_subvalue(db, g))
    }) {
        return None;
    }
    // Class-B UAF (breaker's nested runtime-list re-match): a scrutinee RE-MATCHED by a nested `match xs`
    // in an arm is read by that inner match, so deep-dropping the shell here (the enclosing reclaim) frees a
    // handle the inner match still needs → double-free. Suppress — the innermost match's reclaim is the sole
    // drop. Mirrors `sum_shell_reclaim_ok`'s `cont_rematches_scrutinee` guard for the MatchSum case (cb3-5).
    if list_arms_rematch_scrutinee(db, scrutinee, arms) {
        return None;
    }
    matches!(
        heap_operand_ownership(db, scrutinee),
        Ok(HandleOwnership::Owned)
    )
    .then_some(slot)
}

/// Whether some arm reads a heap sub-value OUT of a compound AS A LIVE HANDLE that could OUTLIVE the
/// post-match shell deep-drop — a borrowing projection (`arr-get`/`vec-get`/`sum-payload`) whose result is a
/// heap handle, appearing in a CONSUME/RESULT position (returned, a call/constructor argument) rather than a
/// pure borrow. Such a handle may alias into the owned scrutinee shell, so dropping the shell would free a
/// still-referenced value (see [`list_shell_reclaim_slot`]).
///
/// POSITION-AWARE, mirroring [`binding_escapes`]'s borrow threading: a heap projection is HARMLESS when it is
/// only BORROWED — the immediate SCRUTINEE of an enclosing `Match`/`MatchSum`/`MatchList` (the dispatch reads
/// its disc/payload without transferring ownership; after the match it is dead), or the operand of another
/// borrowing projection reading DEEPER into it. That is exactly the `msr6` shape `(list r1 r2) → (match r1 …
/// (match r2 … (+ a b)))`: the record elements `r1`/`r2` are inner-match scrutinees (borrows, `vec-get` never
/// bumps their rc), fully consumed by the dispatch before the arm's scalar result, so the outer shell deep-
/// drop reclaims them (rc 1, shell-owned) with no live alias. A heap projection ANYWHERE ELSE — returned as
/// the result, threaded into a call/constructor, an inner match's heap payload flowing out — ESCAPES and
/// blocks the reclaim (a leak, never a double-free).
///
/// SAFE-BY-DEFAULT: only two positions relax to `borrowed = true` (a match SCRUTINEE, a projection OPERAND —
/// both genuine reads); every other node recurses its children as CONSUMING (`borrowed = false`), so an
/// unhandled shape can only over-decline (leak), never wrongly permit a reclaim (UAF). A `RestFrom` tail is
/// never a shell borrow (`vec-drop` mints a FRESH owned sublist). A read bottoming in a SCALAR
/// (`get-int`/`get-bool`) holds no handle. `seen` is keyed by `(id, borrowed)` so a shared node reached in
/// BOTH positions is still checked in its consuming one (never a missed escape).
fn arm_borrows_heap_subvalue(db: &mut Db, id: StructId) -> bool {
    let mut seen = HashSet::new();
    arm_borrows_heap_subvalue_seen(db, id, false, &mut seen)
}

fn arm_borrows_heap_subvalue_seen(
    db: &mut Db,
    id: StructId,
    borrowed: bool,
    seen: &mut HashSet<(StructId, bool)>,
) -> bool {
    if !seen.insert((id, borrowed)) {
        return false;
    }
    // Does THIS node materialize a heap sub-value handle out of a compound?
    let is_heap_borrow = match core_of(db, id) {
        Core::Proj { .. } | Core::SumExpect { .. } => is_heap_type(&type_of(db, id)),
        Core::SumPayload { ref path, .. } => {
            !matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
                && is_heap_type(&type_of(db, id))
        }
        _ => false,
    };
    // In a CONSUME/RESULT position such a handle escapes and blocks the reclaim; in a BORROW position it is
    // only read (an enclosing match/projection consumes it in place) and is fine — but keep descending, since
    // a deeper sub-read may still escape.
    if is_heap_borrow && !borrowed {
        return true;
    }
    match core_of(db, id) {
        // A match BORROWS its scrutinee (reads disc / length / payload) and does not transfer it out; its
        // arm bodies are RESULT positions (consuming). So the scrutinee relaxes to `borrowed = true`, every
        // other child (arm bodies, guards, the sum decision tree) stays consuming.
        Core::Match { scrutinee, .. }
        | Core::MatchSum { scrutinee, .. }
        | Core::MatchList { scrutinee, .. } => {
            if arm_borrows_heap_subvalue_seen(db, scrutinee, true, seen) {
                return true;
            }
            core_child_ids(db, id)
                .into_iter()
                .any(|c| c != scrutinee && arm_borrows_heap_subvalue_seen(db, c, false, seen))
        }
        // A borrowing projection / read reads DEEPER into its operand: the operand is itself borrowed, so a
        // (nested) heap projection there is still just a read. (`SumExpect`/`Proj`/`SumPayload` operand, and
        // the scalar-returning borrow ops.)
        Core::Proj { operand, .. }
        | Core::SumExpect {
            scrutinee: operand, ..
        }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => arm_borrows_heap_subvalue_seen(db, operand, true, seen),
        Core::SumPayload { scrutinee, .. } => {
            arm_borrows_heap_subvalue_seen(db, scrutinee, true, seen)
        }
        // `Bytes.at bytes index` is a SCALAR-EXTRACTING borrow: its result is ALWAYS a raw Int64 byte
        // (`box-int(bytes-get(...))` — NO borrowed-handle `dup`, core.rs:463), so the `bytes` operand is only
        // READ (a slice-VIEW handle read here does NOT escape as a live handle) → relax it to `borrowed`,
        // exactly like `BytesLen`. This un-blocks the enclosing MatchSum shell-reclaim for the
        // slice-view-then-scalar-`Bytes.at` shape (10-bytes:209/:325/:349 known-leak-2, v-mem-safety's
        // rope/slice-view lever, MATCH-shape half). The `index` is a scalar operand read CONSUMING (safe
        // default — a heap escape in the index subtree is still caught). NOT `StrAt`/`StrSlice`/`BytesSlice`/
        // `ListAt`/`MapLookup`: each can RETURN a heap handle (a String span / a view / a heap element)
        // aliasing the operand, which CAN escape — those stay consuming (blocking the reclaim = leak, not UAF).
        Core::BytesAt { bytes, index, .. } => {
            arm_borrows_heap_subvalue_seen(db, bytes, true, seen)
                || arm_borrows_heap_subvalue_seen(db, index, false, seen)
        }
        // `Bytes.compact` (adv-66) is a SAME-HANDLE IN-PLACE canonicalization (op_bytes_compact flattens the
        // rope in place and returns the SAME handle, refcount-neutral — v-mem-safety runtime-verified) — an
        // IDENTITY transform on the handle for the ESCAPE question: it PASSES THROUGH its operand's borrow
        // status. Recurse the operand with the CURRENT `borrowed` flag. A compact whose RESULT is a borrowed
        // key-op operand (a CHAMP probe) reads the operand as a borrow (does NOT escape → un-blocks the shell-
        // reclaim); a compact whose result is CONSUMED still reads it consuming (safe default — leak, not UAF).
        Core::BytesCompact { operand } => {
            arm_borrows_heap_subvalue_seen(db, operand, borrowed, seen)
        }
        // The BORROWING key-ops read a heap sub-value ONLY as a key/probe/compare operand and retain nothing:
        // `Map.lookup`/`Set.contains` BORROW both operands (dropping only the boxed key/elem after),
        // `Map.remove`/`Set.remove` BORROW the key/elem (and CONSUME the collection), and the structural
        // `value-eq`/`value-cmp`/`value-eq-shaped` compares BORROW both operands (core.rs). A slice-view read
        // out of the matched scrutinee and used ONLY as such a borrowed key/probe does NOT escape the arm →
        // relax the key/elem/compare operands to `borrowed`, un-blocking the enclosing MatchSum shell-reclaim
        // for the slice-view-as-CHAMP-key BORROWED-PROBE shape (13-strings:1408, v-mem-safety co-design, the
        // ESCAPE conjunct (ii) of the extraction-probe reclaim disjunct). The remove ops' CONSUMED collection
        // operand stays consuming (safe default; a slice-view is never the collection). NOT `Map.insert`/
        // `Set.insert`/`Set.of`: those CONSUME/STORE the key into the collection (owned-transfer) → the view
        // genuinely escapes → stays consuming (the STORED-KEY + MIXED negative-control fence — no double-free).
        Core::MapLookup { map, key, .. } => {
            arm_borrows_heap_subvalue_seen(db, key, true, seen)
                || arm_borrows_heap_subvalue_seen(db, map, false, seen)
        }
        Core::SetContains { set, elem, .. } => {
            arm_borrows_heap_subvalue_seen(db, elem, true, seen)
                || arm_borrows_heap_subvalue_seen(db, set, false, seen)
        }
        Core::MapRemove { map, key, .. } => {
            arm_borrows_heap_subvalue_seen(db, key, true, seen)
                || arm_borrows_heap_subvalue_seen(db, map, false, seen)
        }
        Core::SetRemove { set, elem, .. } => {
            arm_borrows_heap_subvalue_seen(db, elem, true, seen)
                || arm_borrows_heap_subvalue_seen(db, set, false, seen)
        }
        Core::ValueEq { lhs, rhs }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. } => {
            arm_borrows_heap_subvalue_seen(db, lhs, true, seen)
                || arm_borrows_heap_subvalue_seen(db, rhs, true, seen)
        }
        // Applying a closure BORROWS it: `call_indirect` reads the closure's env cell (the lifted body reads
        // captures via `arr-get`) and does NOT consume/free it — the env-cell reclaim is a SEPARATE post-apply
        // drop (SITE-A, emit.rs) that fires only for an owned operand. So a heap CLOSURE handle read out of the
        // matched scrutinee purely to be APPLIED does NOT escape the arm as a live handle → relax the CALLEE to
        // `borrowed`, un-blocking the enclosing MatchSum shell-reclaim for the borrowed-extracted-closure-then-
        // apply shape (#6049, e.g. 09:827 `(match (List.at fs 0) ((Some f) (f 10)) …)`, 09:848 the Map twin):
        // the owned Some-shell then deep-drops after the apply, its cascade reclaiming the closure cell + boxed
        // captures the borrow left live. The ARGS are CONSUMED by the call (a heap arg genuinely escapes into
        // the callee) → stay consuming. SAFE: a closure that ALSO escapes/re-stores is read at a SEPARATE
        // consuming `SumPayload` node (a store/tuple/return position) reached with `borrowed=false` → still
        // flagged → the reclaim stays blocked there (a residual leak, never a double-free). Mirrors the
        // Map.lookup/Set.contains borrowed-operand relaxation above.
        Core::CallClosure { closure, ref args } => {
            arm_borrows_heap_subvalue_seen(db, closure, true, seen)
                || args
                    .iter()
                    .any(|&a| arm_borrows_heap_subvalue_seen(db, a, false, seen))
        }
        // Every other node kind (calls, constructors, `if`/`let`, arithmetic, …) consumes / results — its
        // children carry no borrow relaxation. SAFE-BY-DEFAULT: an unhandled shape can only over-decline.
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| arm_borrows_heap_subvalue_seen(db, c, false, seen)),
    }
}

/// Whether `id` CONSUMES a heap GRANDCHILD of the loop-param `binder` — a projection `(. e field)`
/// (`Proj`/`SumExpect`, or a non-`RestFrom` `SumPayload`, heap-typed) in a CONSUME position whose OPERAND is
/// a PROPER projection-chain of `binder` (an element `e` extracted from the loop-param list, `e != binder`,
/// so consuming only `e`'s field leaves `e` itself OWNED BY the list). This is the K1/#4139 loop-skip
/// over-free precondition: FBIP-reusing `binder` (a `RestFrom`) frees the old spine — and with it the still-
/// owned element `e` and its live grandchild — → use-after-free, so the preservation dup must NOT be skipped.
///
/// The DISTINGUISHER vs [`arm_borrows_heap_subvalue`] (which was depth-blind and over-retained clean
/// accumulators, v-mem #5090 report): a DIRECT element consumed — `(List.concat acc h)` where `h =
/// SumPayload{scrutinee: binder, path:[Elem]}`, operand IS `binder` — is MOVED OUT (ownership transfers), so
/// freeing the spine is safe and no dup is needed (FLATTEN, 05-compound). Only a consumed GRANDCHILD (operand
/// a PROPER chain of `binder`, not `binder` itself) fires. Confirmed empirically: FLATTEN's rhs is a direct
/// `SumPayload{scrutinee: binder}` (excluded → reclaims); ksd1's is `Proj{operand: SumPayload{scrutinee:
/// binder}}` (a grandchild → fires → K1 UAF fence holds).
///
/// UAF-SAFE-BY-CONSTRUCTION (biased toward FIRING = keep the dup): only a match SCRUTINEE / borrowing-
/// projection OPERAND — genuine reads — relax to `borrowed` (skipped, since a borrowed grandchild does not
/// over-free); EVERY other position recurses CONSUMING, so a genuine grandchild-consume is never MISSED (a
/// miss = the UAF). An over-fire (a grandchild in a key/probe borrow this simplified walk does not relax, vs
/// the full [`arm_borrows_heap_subvalue`]) only KEEPS a dup → a leak, never a double-free.
fn arm_consumes_binder_grandchild(db: &mut Db, id: StructId, binder: StructId) -> bool {
    let mut seen = HashSet::new();
    arm_consumes_binder_grandchild_seen(db, id, binder, false, &mut seen)
}

fn arm_consumes_binder_grandchild_seen(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    borrowed: bool,
    seen: &mut HashSet<(StructId, bool)>,
) -> bool {
    if !seen.insert((id, borrowed)) {
        return false;
    }
    // A CONSUMED heap grandchild of `binder`: a projection whose OPERAND is a PROPER projection-chain of
    // `binder` (operand roots at `binder` but is NOT `binder` itself — the intermediate element stays owned).
    if !borrowed {
        // GRANDCHILD = the projection's OPERAND itself roots at `binder` through a projection (an element of
        // `binder`), NOT a DIRECT `Param`/`LocalRef` to `binder`. A direct-element projection
        // `SumPayload{scrutinee: <ref to binder>}` (FLATTEN's `h`) has its operand a bare binder reference —
        // that element is MOVED OUT when consumed, so it is NOT a grandchild. `is_direct_binder_ref` peels the
        // binder-ID-vs-reference-node distinction (the `Param{binder}` node id differs from the binder id).
        let is_direct_binder_ref = |db: &mut Db, n: StructId| matches!(core_of(db, n), Core::Param { binder: b } | Core::LocalRef { binder: b } if b == binder);
        let is_grandchild_consume = match core_of(db, id) {
            Core::Proj { operand, .. }
            | Core::SumExpect {
                scrutinee: operand, ..
            } => {
                is_heap_type(&type_of(db, id))
                    && !is_direct_binder_ref(db, operand)
                    && payload_or_proj_chain_roots_at_binder(db, operand, binder)
            }
            Core::SumPayload {
                scrutinee,
                ref path,
            } => {
                !matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
                    && is_heap_type(&type_of(db, id))
                    && !is_direct_binder_ref(db, scrutinee)
                    && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            }
            _ => false,
        };
        if is_grandchild_consume {
            return true;
        }
    }
    // Position walk (mirrors [`arm_borrows_heap_subvalue_seen`]'s genuine-borrow relaxations; every other
    // position stays CONSUMING so a grandchild-consume is never missed).
    match core_of(db, id) {
        Core::Match { scrutinee, .. }
        | Core::MatchSum { scrutinee, .. }
        | Core::MatchList { scrutinee, .. } => {
            arm_consumes_binder_grandchild_seen(db, scrutinee, binder, true, seen)
                || core_child_ids(db, id).into_iter().any(|c| {
                    c != scrutinee
                        && arm_consumes_binder_grandchild_seen(db, c, binder, false, seen)
                })
        }
        Core::Proj { operand, .. }
        | Core::SumExpect {
            scrutinee: operand, ..
        }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => {
            arm_consumes_binder_grandchild_seen(db, operand, binder, true, seen)
        }
        Core::SumPayload { scrutinee, .. } => {
            arm_consumes_binder_grandchild_seen(db, scrutinee, binder, true, seen)
        }
        // The SAME borrow relaxations as [`arm_borrows_heap_subvalue_seen`] — REQUIRED so a grandchild read
        // ONLY as a borrowed key/probe/compare/scalar-extract is not mistaken for a consume (omitting them
        // over-fired Map.to-list's `Bytes.at k 0` borrowed key → a spurious dup/leak, 19-sets:1878). `Bytes.at`
        // scalar-extracts (bytes borrowed); `Bytes.compact` passes the borrow status through; the key-ops
        // borrow their key/probe/compare operands (and consume the collection).
        Core::BytesAt { bytes, index, .. } => {
            arm_consumes_binder_grandchild_seen(db, bytes, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, index, binder, false, seen)
        }
        Core::BytesCompact { operand } => {
            arm_consumes_binder_grandchild_seen(db, operand, binder, borrowed, seen)
        }
        Core::MapLookup { map, key, .. } => {
            arm_consumes_binder_grandchild_seen(db, key, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, map, binder, false, seen)
        }
        Core::SetContains { set, elem, .. } => {
            arm_consumes_binder_grandchild_seen(db, elem, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, set, binder, false, seen)
        }
        Core::MapRemove { map, key, .. } => {
            arm_consumes_binder_grandchild_seen(db, key, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, map, binder, false, seen)
        }
        Core::SetRemove { set, elem, .. } => {
            arm_consumes_binder_grandchild_seen(db, elem, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, set, binder, false, seen)
        }
        Core::ValueEq { lhs, rhs }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. } => {
            arm_consumes_binder_grandchild_seen(db, lhs, binder, true, seen)
                || arm_consumes_binder_grandchild_seen(db, rhs, binder, true, seen)
        }
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| arm_consumes_binder_grandchild_seen(db, c, binder, false, seen)),
    }
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

impl HandleOwnership {
    /// Reclaim the handle held in scratch `slot` IFF this operand was `Owned` — the shared tail of every
    /// borrowing op (`value-eq`/`value-cmp`/`value-eq-shaped`, the runtime BigInt ops): an OWNED temporary
    /// operand must be dropped after the borrowing call (else it leaks), a `Borrowed` reference must be left
    /// to its owner (dropping it would be a double-free). Emits `local.get slot ; drop` for `Owned`, nothing
    /// for `Borrowed`.
    fn drop_slot_if_owned(self, slot: u32, out: &mut Emit) {
        if self == HandleOwnership::Owned {
            out.push(Lir::LocalGet(slot));
            out.push(Lir::CallImport(OP_DROP));
        }
    }
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
/// verbatim the String story. Compacts a String/Bytes key of ANY ownership: `bytes-compact` is
/// REFCOUNT-NEUTRAL (it flattens the node IN PLACE and returns the SAME handle — a no-op on an already-flat
/// leaf), so it is safe on an OWNED or a BORROWED key alike (verbatim the `elem_needs_rope_compaction`
/// reasoning at the compound-construction sites). This FIXES a wasm WRONG-VALUE: a BORROWED runtime rope/
/// slice key (a `sum-payload`/`Option.expect` binder / a kept `let`-local of a `Bytes.slice` result) was
/// previously NOT compacted (the old `Owned`-only gate, on a false "compact would consume it" belief), so
/// its raw slice-VIEW node (`[off, len]`) reached `champ_hash` and hashed differently from an equal-content
/// flat Bytes — the lookup missed while value-eq said equal (equal-means-same-key violated). Because
/// compact is in-place-same-handle, a borrowed key STAYS the owner's handle after compaction, so
/// `key_handle_is_owned_temporary` (the drop gate) still decides drop-vs-keep on the operand's ACTUAL
/// ownership — a borrowed key is not dropped (no double-free), an owned key is dropped as before.
fn key_needs_compaction(db: &mut Db, key: StructId) -> bool {
    operand_is_string_or_bytes(db, key)
}

/// Whether a type CONTAINS a `List` anywhere (the type is a List, or a Tuple/Record/Sum/Nominal/Qty/Frame
/// whose component does). A List is an RRB vector — element-canonical but NOT shape-canonical (a concat-
/// built and a push-built equal-element list have different internal trees), so the tagless CHAMP byte-walk
/// would place two EQUAL list values in different slots. When such a value is a Map/Set KEY, the key must be
/// `value-canonicalize`d at the key site so the walk becomes exact. A Set/Map is NOT itself a list (it is
/// canonical at its own level), but a list buried inside a Set-of-lists / Map-with-list-values is the
/// documented residual `value_canonicalize_shaped` leaves — so we do NOT descend into Set/Map here (matching
/// the runtime canonicalize, which dups a Set/Map as-is). `seen` breaks a recursive Sum.
fn ty_contains_list(db: &mut Db, ty: &Ty, seen: &mut Vec<StructId>) -> bool {
    match ty {
        Ty::List(_) => true,
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().any(|e| ty_contains_list(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().any(|v| ty_contains_list(db, v, seen))
        }
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return false;
            }
            seen.push(*decl);
            let Some(variant_count) = db.type_decl_by_occ(*decl).map(|t| t.variants.len()) else {
                seen.pop();
                return false;
            };
            let mut found = false;
            for disc in 0..variant_count {
                let ctor = db
                    .type_decl_by_occ(*decl)
                    .and_then(|t| t.variants.get(disc))
                    .and_then(|v| v.ctor);
                if let Some(ctor) = ctor
                    && let Some(payload_ty) =
                        crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                    && ty_contains_list(db, &payload_ty, seen)
                {
                    found = true;
                    break;
                }
            }
            seen.pop();
            found
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        _ => false, // scalars, String, Bytes, Char, Set, Map (canonical at own level), Fn, etc.
    }
}

/// Whether a Map/Set KEY must be `value-canonicalize`d before it reaches the tagless CHAMP key path: its
/// type is a `List` or CONTAINS one. This is the `value-canonicalize` analogue of `key_needs_compaction`
/// (which handles the String/Bytes-rope axis). Unlike a rope compaction, this reshapes the RRB spine, so it
/// needs the key's shape descriptor baked at the key site (see the emit).
fn key_needs_canonicalize(db: &mut Db, key: StructId) -> bool {
    let ty = type_of(db, key);
    ty_contains_list(db, &ty, &mut Vec::new())
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
    if key_needs_canonicalize(db, key) {
        return Ok(true); // a list-typed/-containing key → a FRESH owned canonical value we drop
    }
    // A String/Bytes key may be `bytes-compact`ed (`key_needs_compaction`), but compact is REFCOUNT-NEUTRAL
    // and IN-PLACE (same handle), so it does NOT change ownership: a compacted OWNED rope is still owned
    // (dropped here), a compacted BORROWED rope is still the owner's handle (NOT dropped — dropping it would
    // free a reference the owner still holds, a use-after-free). So the drop decision — whether compacted or
    // not — is the operand's ACTUAL ownership: drop iff a fresh owned handle (a constructor / call / const
    // compound / owned rope); a borrowed param/local/projection is left to its owner.
    Ok(heap_operand_ownership(db, key)? == HandleOwnership::Owned)
}

/// Emit the KEY CANONICALIZATION at a Map/Set key site: the boxed key handle is ON TOP OF THE STACK; replace
/// it with its `value-canonicalize`d form so a list-typed/-containing key hashes into the correct CHAMP slot
/// (the RRB-shape-canonicality fix). Bakes the key's shape descriptor into a fresh scratch Bytes slot (the
/// same const-Bytes build `Core::ValueCmp` uses), calls `value-canonicalize(key, desc)` (→ a FRESH owned
/// canonical key; the input is borrowed and its own reclamation is unchanged — value-canonicalize borrows),
/// then drops the descriptor temporary. Declines if the key type has no bakeable descriptor (reject-don't-
/// miscompile — the pre-fix byte-walk still applies, only the false-miss persists rather than a bad emit).
/// Called ONLY when `key_needs_canonicalize` (a list-containing key). Two fresh i32 scratch slots (key stash
/// + descriptor); `high` is advanced past them.
fn emit_key_canonicalize(
    db: &mut Db,
    key: StructId,
    key_ty: &Ty,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    // Bake the descriptor from the passed `key_ty`; but that field can be an undetermined `Var` (a list
    // element/key of an EMPTY collection — `Set.of (list)` / `Map.empty` — carries a `Var` `elem_ty`/`key_ty`
    // field, so `value_cmp_shape_descriptor` finds no shape). `key_needs_canonicalize` already decided YES
    // from the NODE's resolved `type_of` (`List Int64`), so the field/node type disagree exactly like the
    // `box_op_for`-vs-`box_op_ty` node-aware family. Prefer the field type, but fall back to the node's
    // resolved type before declining — so an empty-collection list key canonicalizes like a pinned one,
    // instead of a false-miss-vs-decline asymmetry between the Set-of-empty and Map-of-empty paths.
    let desc = match crate::lower::value_cmp_shape_descriptor(db, key_ty) {
        Some(d) => d,
        None => {
            let resolved = type_of(db, key);
            match crate::lower::value_cmp_shape_descriptor(db, &resolved) {
                Some(d) => d,
                None => {
                    // Neither the field nor the node's resolved key type bakes a shape — the key's element
                    // type is genuinely UNDETERMINED at this (already-monomorphized) emit site, e.g.
                    // `(Set.of (list (list)))` whose inner empty-list element nothing constrains. This is a
                    // determinacy fault, not a feature gap: reject it CODED (CDZ0203 "annotate the type",
                    // the escape-result determinacy reject's twin) rather than the former CODELESS decline
                    // that let a not-fully-determined program slip through to a shape-less lower bail. A
                    // DETERMINED key (`(Set.of (list (list 1)))`, or `(: (list) (List Int64))`) bakes a
                    // descriptor and never reaches here; a GENERIC `Set a`/`Map a _` def is monomorphized
                    // before emit, so its key is concrete by the time this runs — no false reject
                    // (seq-286 / fuzzer #5, v-compiler-primitives + v-deferral-declines routed).
                    return Err(Reject::coded(
                        crate::diag::Code::TypeMismatch,
                        format!(
                            "a Set/Map key's type `{}` is not fully determined — annotate it \
                             (e.g. `(: (list) (List Int64))`) so its keys have a canonical form for \
                             comparison",
                            resolved.render_name(&db.name_ctx())
                        ),
                    ));
                }
            }
        }
    };
    // `value-canonicalize` BORROWS its input and returns a FRESH owned canonical value — so the RAW key on
    // the stack must be reclaimed HERE iff it was an OWNED TEMPORARY (a fresh `List.concat`/build result),
    // else it leaks; a BORROWED key (a param / kept-local / a live projection) is left to its owner (dropping
    // it would free a live reference — a UAF). Mirrors the box/compact ownership discipline.
    let raw_owned = matches!(heap_operand_ownership(db, key), Ok(HandleOwnership::Owned));
    let key_slot = *high;
    let desc_slot = *high + 1;
    *high = desc_slot + 1;
    scratch_ty.insert(key_slot, ValType::I32);
    scratch_ty.insert(desc_slot, ValType::I32);
    // Stash the raw key handle (top of stack), clearing the stack for the descriptor bake.
    out.push(Lir::LocalSet(key_slot)); // [] (raw key stashed)
    // Bake the descriptor Bytes into desc_slot (build then store — the ValueCmp bake pattern).
    out.push(Lir::ConstI32(desc.len() as i32));
    out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
    for (j, &byte) in desc.iter().enumerate() {
        out.push(Lir::ConstI32(j as i32));
        out.push(Lir::ConstI32(byte as i32));
        out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf]
    }
    out.push(Lir::LocalSet(desc_slot)); // [] (descriptor stored)
    // canonicalize(raw, desc) → fresh owned canonical key on the stack (replacing the raw key).
    out.push(Lir::LocalGet(key_slot)); // [raw]
    out.push(Lir::LocalGet(desc_slot)); // [raw, desc]
    out.push(Lir::CallImport(OP_VALUE_CANONICALIZE)); // [canon-key] (borrows raw + desc)
    // Drop the borrowed-only descriptor Bytes temporary (value-canonicalize borrowed it).
    out.push(Lir::LocalGet(desc_slot));
    out.push(Lir::CallImport(OP_DROP));
    // Reclaim the raw key if it was an owned temporary (canonicalize only borrowed it).
    if raw_owned {
        out.push(Lir::LocalGet(key_slot));
        out.push(Lir::CallImport(OP_DROP));
    }
    Ok(())
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
    // PEEL nominal + quantity via `peel_qty_ty` (strip_nominal → peel Qty → strip_nominal), NOT a bare
    // `Ty::BigInt`/`Ty::Qty{BigInt}` match: a single-variant single-payload NEWTYPE over BigInt — e.g.
    // `(type W (Mk BigInt))` — erases to `Ty::Nominal { inner: BigInt }`, so a constant `(Mk 1)` used as a
    // runtime value (a call arg) is a `Core::ConstInt` typed `Nominal{BigInt}`. The old bare match missed the
    // nominal wrapper → the ConstInt emit fell to the raw `i64.const` path → an i32 handle was expected at the
    // call → INVALID module (FACE-B of the nonzero-BigInt-recursive miscompile). `peel_qty_ty` reaches the
    // inner BigInt through the newtype (and a nominal-over-Qty-BigInt), the exact strip→peel→strip the
    // Float32 nominal-over-Qty fix (PR#743) established for its twin.
    matches!(peel_qty_ty(type_of(db, id)), Ty::BigInt)
}

/// The byte offset in the shared host `mem` where a RUNTIME string host-arg's bytes are marshaled (the
/// `_mem` runtime-arg path). The const-string host args occupy `[0, max(offset+len))` (the data segment);
/// the runtime scratch buffer starts past that — the max const-string end rounded up to a 256-byte
/// boundary, plus a 1 KiB gap (headroom so a future data-segment growth does not overlap). The shared mem
/// is 1 page (64 KiB); a runtime string longer than `65536 - scratch_base` would overrun — the caller
/// bounds this by the single-arg + fixed-buffer contract (a huge runtime string is a later increment; the
/// value-heap rope's length is not statically known here, so an over-long string is a runtime concern the
/// host boundary already sizes its read to `len`). Deterministic (no allocation state) — same base every call.
fn host_arg_scratch_base(layout: &Layout) -> u32 {
    let const_end = layout
        .host_strings
        .iter()
        .map(|(s, off)| off + s.len() as u32)
        .max()
        .unwrap_or(0);
    const_end.div_ceil(256) * 256 + 1024
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
        // A list PRODUCER returns a FRESH owned list handle exactly like a `ListNew` constructor:
        // `vec-push` (`ListPush`) CONSUMES its input list + element and returns a NEW owned list (the old
        // version untouched — persistence); `vec-concat` (`ListConcat`) consumes both operands and hands
        // back a new vector; `vec-update` (`ListUpdate`) returns the updated list as a new handle. So such a
        // list as a BORROWING-op operand (`List.len`/`List.at`/`= `) is an owned temporary the emit reclaims
        // after the borrow — the LIST analog of the `BytesConcat`/`BytesSlice` producers below. WITHOUT
        // this, `List.len (List.push (build …) x)` / `List.len (List.concat a b)` / `List.at (List.update l
        // i x) j` fell to the `_ => decline` default, so the reclaim gate never fired and the fresh list
        // LEAKED one vector per call (value correct — a leak, not a miscompile). Classifying `ListPush`
        // Owned reclaims only the fresh PUSH RESULT that a borrowing op leaves un-consumed; the ACCUMULATOR
        // pattern `(build i n (List.push acc i))` returns the push as its TAIL (never a borrowing-op
        // operand), so its ownership is never consulted here and the FBIP single-consume fast path is
        // untouched. A shared/borrowed `acc` (rc>1) is already `dup`'d before the push by the Perceus retain,
        // so `vec-push` produces a genuinely fresh result — dropping it can never double-free the live `acc`.
        | Core::ListPush { .. }
        | Core::ListPrepend { .. }
        | Core::ListConcat { .. }
        | Core::ListUpdate { .. }
        // A fallible READ that returns an `(Option T)` builds a FRESH owned `sum-new` Some shell (or a
        // nullary None) around the extracted/copied payload: `List.at`/`Bytes.at` (`vec-get`/`bytes-get`
        // into `Some`), `Map.lookup` (the looked-up value dup'd into `Some`). So the Option RESULT is an
        // owned temporary — when it is a `Option.expect`/match SCRUTINEE, the `SumExpect`/`MatchSum` emit
        // reclaims the shell after the payload read (else the shell LEAKS one cell per call — the
        // live-objects-gate find). These BORROW their collection (that reclaim is handled at the op's own
        // emit); here we classify the fallible RESULT they hand out, which is a fresh owned sum.
        | Core::ListAt { .. }
        | Core::BytesAt { .. }
        // NOTE: `String.at` (`Core::StrAt`) also returns a FRESH owned `Some(char-view)`, but it is
        // DELIBERATELY NOT classified Owned here — a GLOBAL reclassification perturbs the MatchSum Stage-B
        // extraction-consume reclaim (a String.at-view consumed by an allowlisted `String.concat` nets +1 =
        // a latent Stage-B imbalance, v-mem-safety's separate follow-up). Instead the (2) SumExpect view/shell
        // reclaim treats a `StrAt` scrutinee as owned LOCALLY (in the SumExpect `reclaim_shell` emit, gated by
        // the `sumexpect_view_reclaim`/`sumexpect_shell_reclaim` membership) — same local>global discipline as
        // the reclaim_bytes-local (2) check, so the MatchSum/value-eq/Stage-B consumers see StrAt UNCHANGED.
        | Core::MapLookup { .. }
        | Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapRemove { .. }
        // A constant string/bytes materializes a FRESH owned byte-leaf handle (`bytes-alloc`+`bytes-set`,
        // see the `Core::ConstStr`/`ConstBytes`/`BytesOf` emit), so — like a constructor — the `value-eq`
        // emit drops it after the borrowing compare. This is the `(= h "+")` shape: comparing a runtime
        // payload string against a constant-string literal.
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
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
        // `str-slice` (`String.slice`) returns a FRESH owned `(Option String)` — a fallible read that copies
        // the slice range into a new `Some(str)` (or `None`), exactly like `BytesAt`/`BytesSlice` above. So a
        // `String.slice` result used as a MatchSum SCRUTINEE is an OWNED computed boxed-sum, and its shell +
        // consumed payload must be reclaimed by the shell-reclaim child-dup path. WITHOUT this arm it fell to
        // the `_ => decline` ("ownership this backend cannot yet prove") default, so `sum_shell_reclaim_ok` /
        // `collect_shell_reclaim_child_dups` bailed and a payload CONSUMED MORE THAN ONCE off the slice (the
        // adv54b class: `tail` = `String.slice`'s payload fed to two `String.to-bytes` under one
        // `Bytes.concat`) got NO child-`dup` → the shared payload's ref was released twice → DOUBLE-FREE.
        | Core::StrSlice { .. }
        | Core::StrFromBytes { .. }
        | Core::StrToBytes { .. }
        // `str-nfc-normalize` (FINDING #23) hands out a FRESH owned String handle: it CONSUMES its operand and
        // returns either that same handle (already NFC) or a fresh normalized leaf with the original dropped —
        // either way an owned reference transfers out, exactly like `StrToBytes`/`BytesConcat`. So as a
        // borrowing-op operand it is Owned and the emit reclaims it after the borrow.
        | Core::NfcNormalize { .. }
        // `Blake3.of` on a runtime Bytes (`hash-blake3`, op 91) BORROWS its operand and returns a FRESH
        // owned 32-byte Bytes leaf. So a `Blake3.of` result used as a `value-eq` operand — the P4
        // dispatch shape `(= (Blake3.of msg-contract) id)` / `(= (Blake3.of x) (Blake3.of y))` — is an
        // owned temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this
        // backend cannot yet prove"), blocking a runtime-digest equality (a completeness gap, not a
        // miscompile) even though a `Bytes.of` of the same content compares fine.
        | Core::Blake3Of { .. }
        // `Ast.encode` (op 93) BORROWS its Ast operand and returns a FRESH owned Bytes document; `Ast.print`
        // (op 92) likewise returns a FRESH owned String. So such a result used as a BORROWING-op operand — a
        // bare `(= (Ast.encode a) (Ast.encode b))`, or the PROBE of `Set.contains`/`Map.lookup` keyed by
        // arena Bytes (`(Set.contains s (Ast.encode …))`) — is an owned temporary the borrow must reclaim.
        // WITHOUT this both fell to `_ => decline` ("ownership this backend cannot yet prove"): CHAMP
        // construction dedups arena Bytes fine (the constructor path never consults this), but the query-
        // entry / bare-compare lowering DECLINED (breaker xf3/xf4 + beq bare-`=`). The Ast twin of `Blake3Of`
        // / `ValueEncode` above.
        | Core::AstEncode { .. }
        | Core::AstPrint { .. }
        // A runtime `(bin …)` construction builds a FRESH owned Bytes on the rope heap (`bytes-alloc` +
        // per-segment range-check-and-write, exactly like `BytesOf`), so as a `value-eq` operand it is
        // Owned and the emit drops it after the borrowing compare. WITHOUT this a runtime `(bin …)` result
        // — spec'd a Bytes value — fell to the `_ => decline` below, so `(= (bin (u8 v)) b"…")` DECLINED
        // "an ownership this backend cannot yet prove" even though a `Bytes.of` of the same content compares
        // fine (a completeness gap, not a miscompile). `BinBitsBuild` (a `(bits v k)` run) is the same.
        | Core::BinBuild { .. }
        | Core::BinBitsBuild { .. }
        // A dependent-size / final-rest bin-match PAYLOAD read returns a FRESH owned Bytes: `BinSizedRead`
        // and `BinRestRead` both emit `dup(scrutinee); bytes-slice(copy, off, len)`, and `bytes-slice`
        // CONSUMES the dup'd copy + hands back a NEW owned slice (exactly like `BytesSlice`). So the slice
        // as a borrowing-op operand (`Bytes.len payload` — the payload binder lowers to an INLINE
        // `BinSizedRead`, so `BytesLen`'s operand IS this node) is an owned temporary the borrow must
        // reclaim. WITHOUT this the slice fell to `_ => decline`, `BytesLen`'s reclaim gate never fired,
        // and a fresh slice LEAKED one Bytes cell per dependent-size match (value-correct — a leak, not a
        // miscompile; witnessed by `dependent_size_bin_match_payload_read_leaves_no_live_objects`). The
        // Bytes analog of the `ListConcat`/`ListPush` producer gap fixed earlier.
        | Core::BinSizedRead { .. }
        | Core::BinRestRead { .. }
        // `Value.encode` returns a FRESH owned `Bytes` document handle (the runtime `value-encode` op mints
        // a new doc, borrowing its value operand), and `Value.decode` returns a FRESH owned value handle
        // (`sum-new(Some, decoded)` over the fresh `value-decode` result, or the nullary `None`). So a
        // `Value.encode`/`Value.decode` RESULT used as a borrowing-op operand — the round-trip `(let ((bs
        // (Value.encode v))) (Value.decode bs) …)` where `bs` feeds `value-decode` (a borrow) — is an owned
        // temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this backend
        // cannot yet prove"), blocking the whole R2 round-trip at emit (v-inference's decode-grounding got it
        // past typing, then this fired). The Bytes/value analog of the collection-producer arms below.
        | Core::ValueEncode { .. }
        | Core::ValueDecode { .. }
        // `Char.from-int` (`IntToCharChecked`) wraps the boxed codepoint into a FRESH owned `(Option Char)`
        // sum (`disc_some`/`disc_none`), exactly like `ValueDecode`'s `sum-new(Some, …)`/`None`. So its result
        // used as a MatchSum SCRUTINEE — `(match (Char.from-int n) ((Some c) …) ((None _) …))` — is an OWNED
        // computed boxed-sum whose Option SHELL the shell-reclaim must drop. WITHOUT this it fell to the
        // `_ => decline` default, so `sum_shell_reclaim_ok` bailed and the Option shell LEAKED one cell per
        // match (arm- AND payload-independent — the shell, not the boxed Char; v-mem's chfi triage). The Char
        // twin of the `StrSlice`(Option String) / `ValueDecode`(Option value) fresh-owned-sum producer arms.
        | Core::IntToCharChecked { .. }
        // A set construction/update/algebra (`set-empty`+inserts, `set-insert`, `set-remove`, union/
        // intersection/difference) returns a fresh owned set handle — the `value-eq` emit drops it.
        | Core::SetOf { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetAlgebra { .. }
        // A collection ENUMERATION returns a FRESH owned `List` handle: `map-to-list` materializes the map's
        // entries as a new `List (Tuple k v)` (canonical key order), `set-to-list` the set's elements as a new
        // `List`. So the enumeration RESULT as a borrowing-op operand (`List.len (Map.to-list m)`) is an owned
        // temporary the borrow must reclaim. WITHOUT this they fell to `_ => decline`, the borrowing consumer's
        // reclaim gate (e.g. `ListLen`) never fired, and the fresh result list (+ its boxed entries) LEAKED
        // per call — value-correct, a leak not a miscompile; the enumeration analog of the `ListConcat`/
        // `ListUpdate` producer gap. (This governs the to-list RESULT; a leak of an owned-temporary SOURCE
        // handed TO to-list is a separate face — the to-list op borrows its source — tracked by
        // `map_or_set_to_list_over_an_owned_temporary_source_leaks_it_known_gap`.)
        | Core::MapToList { .. }
        | Core::SetToList { .. }
        // A BigInt PRODUCER returns a fresh owned handle: `bigint-of-i64` mints a leaf, and each
        // `bigint-add`/`-sub`/`-mul`/`-div` re-boxes a normalized result (the operands are borrowed, the
        // result is new). So a BigInt operand that is itself the result of another BigInt op is owned —
        // the enclosing op drops it after borrowing. (`BigIntToI64` returns an i64 scalar, never a handle,
        // so it is not a heap operand and never reaches here.)
        | Core::BigIntOfI64 { .. }
        | Core::BigIntBinOp { .. }
        // `rational-num`/`rational-den` return a FRESH owned BigInt handle (they unbox the Rational's
        // component + re-box it, borrowing the Rational), so a `Rational.numerator`/`denominator` result
        // used as a borrowing-op operand (e.g. `Int64.of (Rational.numerator r)`) is Owned — the enclosing
        // op drops it after borrowing.
        | Core::RationalNum { .. }
        | Core::RationalDen { .. }
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
        // A `Core::Closure` builds a FRESH owned cell (`arr-alloc` of the code index + the dup'd captures),
        // ownership transferring out exactly like a `SumNew`/`Tuple`/`Record` constructor — so as the operand
        // of a BORROWING op it is an owned temporary the enclosing op reclaims after the borrow. This is the
        // SITE-A part (a): the eta-closure `((if c (T.Mk 0) (T.Mk 10)) 5)` distributes the projection into the
        // `if`, so the `CallClosure` operand is an `If`-of-partial-ctors whose arms are each a fresh owned
        // closure; the `Core::If` join (`join_arm_ownership`) can now flip that operand to Owned, which is the
        // precondition v-effects' CallClosure emit (part b) gates its `cell_slot` drop on. Classifying a
        // Closure Owned is UNCONDITIONALLY correct (a freshly-built cell is genuinely owned); the SOUNDNESS
        // that keeps a SHARED/curried/re-applied/forced-thunk cell from being wrongly dropped lives ENTIRELY
        // in v-effects' emit gate (full-arity apply + non-escaping result), NOT here — this arm only states
        // "the cell is owned", never "the cell may be dropped". No reclaim consumer today takes a Fn-typed
        // operand except that CallClosure emit, so until part (b) lands this arm is observably inert (a Fn
        // cannot be a List.len/value-eq/map-key/sum-scrutinee operand — those are type errors), which is what
        // lets part (a) land independently.
        Core::Closure { .. } => Ok(HandleOwnership::Owned),
        // A reference to a parameter or a kept `let`-binding — the owner elsewhere reclaims it.
        Core::Param { .. } | Core::LocalRef { .. } => Ok(HandleOwnership::Borrowed),
        // A list REST-BINDER read (`(list _ .. r)` → a `SumPayload` whose path's SOLE/final step is a
        // `RestFrom`) is the EXCEPTION: its emit is `dup(scrutinee); vec-drop(k)`, and `vec-drop` CONSUMES
        // the dup'd copy and hands back a NEW owned tail vector (the scrutinee's own count nets to zero —
        // see the `RestFrom` emit). So the extracted rest is a FRESH OWNED temporary, exactly like a
        // `ListConcat`/`ListUpdate` producer — NOT a borrow. A borrowing consumer (`List.len r`) must
        // therefore reclaim it after the borrow (else the fresh tail LEAKS one vector + its shared leaf per
        // read — the lm5 live-objects find), and as an arm RESULT it transfers out (Owned). Each reference
        // re-emits its own `dup`+`vec-drop` (an inline `SumPayload`, one per occurrence), so classifying it
        // Owned drops exactly one fresh vector per emit — never a double-free. Any OTHER path (a plain
        // `Payload`/`Elem` read) still BORROWS, as below.
        Core::SumPayload { path, .. }
            if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))) =>
        {
            Ok(HandleOwnership::Owned)
        }
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
        _ => Err(Reject::unsupported(
            "borrowing op operand has an ownership this backend cannot prove",
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
    o.drop_slot_if_owned(slot, out);
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
    lo.drop_slot_if_owned(slot_l, out);
    ro.drop_slot_if_owned(slot_r, out);
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
        Core::ConstFloatInf => {
            if w == 32 {
                out.push(Lir::F32ConstBits(f32::INFINITY.to_bits()));
            } else {
                out.push(Lir::F64ConstBits(f64::INFINITY.to_bits()));
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
        // A constant +∞: its exact IEEE bits (`0x7F80_0000` / `0x7FF0…`), no canonicalization (infinity
        // has one bit form), byte-identical to the rust backend's `f{32,64}::INFINITY`.
        Core::ConstFloatInf => {
            if w == 32 {
                out.push(Lir::ConstI32(0x7F80_0000u32 as i32));
            } else {
                out.push(Lir::ConstI64(0x7FF0_0000_0000_0000u64 as i64));
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
    // A bare `ConstFloat` branch must take the `if`'s RESULT float width, not its own default `Float64` —
    // the float twin of the `ConstInt`-to-result-width grounding above (see the tail-position `Core::If`
    // arm's `emit_tail_branch` for the full rationale). Only `Float32` differs from the literal's default.
    // PEEL `Ty::Nominal`/`Ty::Qty` (via `peel_qty_ty`) before the `Float32` check: `valtype_of` reads
    // through those wrappers to the inner `f32` (so a wrapped-Float32 result gives an `f32` block), but a
    // bare `Ty::Float` match would miss a wrapped result and fall to the default `f64.const` — the same
    // invalid-module asymmetry the sibling int grounding already avoids. Latent today (Qty.of erases first),
    // but keeps this symmetric with the int side and closes the hazard.
    if let Core::ConstFloat(d) = core_of(db, id)
        && let Ty::Float(rft) = peel_qty_ty(result.clone())
        && rft.ground_width() == 32
    {
        out.push(Lir::F32ConstBits(
            (f64::from_bits(d.to_f64_bits()) as f32).to_bits(),
        ));
        return Ok(());
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)
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
    base: crate::fxhash::FxHashMap<StructId, crate::db::ValueFact>,
) -> crate::fxhash::FxHashMap<StructId, crate::db::ValueFact> {
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
    frame.insert(binder, crate::db::ValueFact::from_int_range(c, Some(c)));
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
        Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. } => {
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
/// Whether the caller must `drop` the OWNED-TEMPORARY arg `arg` (the callee's param at `param_index`) AFTER a
/// NON-TAIL call to `callee`. CALLER-owns-args holds ONLY for a BOUNDARY-OWNED callee (export-entry or lifted)
/// whose params are drop_after'd at the call boundary. Gate — all conjuncts conservative toward NOT dropping
/// (wrong ⇒ leak, never double-free):
///   1. boundary-owned callee;  2. heap param;  3. Owned arg;  4. callee BORROWS the param (`!param_escapes`);
///   5. NON-LOOPED callee (`mutual_loop_group` empty) — a LOOPED callee handles its own params (a fold
///      CONSUMES them; an invariant borrow is epilogue-dropped; a varying borrow → at-worst leak), so a
///      caller-drop there double-frees (the 5000-sum/brd1 consuming-fold class). The consuming folds are
///      exactly the looped ones, so `!looped` subsumes the spine-consume exclusion.
fn call_arg_caller_drops(
    db: &mut Db,
    callee: usize,
    arg: StructId,
    param_index: usize,
    layout: &Layout,
) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    if !(layout.exports.iter().any(|e| e.body == body) || db.lifted.iter().any(|l| l.body == body))
    {
        return false; // (1)
    }
    if !mutual_loop_group(db, callee).is_empty() {
        return false; // (5) looped callee handles its own params
    }
    let params = match layout.export_plan(callee) {
        Some(e) => e.params.clone(),
        None => crate::layout::def_params(db, callee),
    };
    let Some((param_binder, param_ty)) = params.get(param_index).cloned() else {
        return false;
    };
    if !is_heap_type(&param_ty) {
        return false; // (2)
    }
    if !matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned)) {
        return false; // (3)
    }
    if param_escapes_body(db, body, param_binder) {
        return false; // (4)
    }
    true
}

/// Whether `body` contains a `Core::Call` whose arg triggers a caller-drop ([`call_arg_caller_drops`]) — the
/// import-side companion of the `Core::Call` emit, so `collect_module_used_ops` imports `drop` iff the emit
/// actually emits a caller-drop (precise import/emit agreement, like `def_drops_owned_param`). Cycle-guarded.
pub fn body_has_caller_drop(db: &mut Db, body: StructId, layout: &Layout) -> bool {
    fn walk(db: &mut Db, id: StructId, layout: &Layout, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::Call { callee, args } = core_of(db, id) {
            for (i, &a) in args.iter().enumerate() {
                if call_arg_caller_drops(db, callee, a, i, layout) {
                    return true;
                }
            }
        }
        crate::backend::wasm::select::reclaim::core_child_ids(db, id)
            .into_iter()
            .any(|c| walk(db, c, layout, seen))
    }
    walk(db, body, layout, &mut HashSet::new())
}

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
    caller_drop_slots: Option<&mut Vec<u32>>,
) -> Result<(), Reject> {
    let drops: Vec<bool> = if caller_drop_slots.is_some() {
        (0..args.len())
            .map(|i| call_arg_caller_drops(db, callee, args[i], i, layout))
            .collect()
    } else {
        Vec::new()
    };
    let mut recorded: Vec<u32> = Vec::new();
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
        if drops.get(i).copied().unwrap_or(false) {
            let slot = *high;
            *high += 1;
            scratch_ty.insert(slot, ValType::I32);
            out.push(Lir::LocalTee(slot));
            recorded.push(slot);
        }
        arg_base = *high;
    }
    if let Some(sink) = caller_drop_slots {
        *sink = recorded;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
