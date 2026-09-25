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
mod bigint_emit;
use bigint_emit::*;
mod reclaim;
pub use reclaim::core_child_ids;
use reclaim::*;
pub(crate) use reclaim::{
    EscapeTarget, escaped_field_projections, param_borrow_aware_escapes,
    param_consume_sink_whitelisted, param_escapes_body, param_flow_into_cycle,
    record_cell_param_droppable,
};
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
mod grandchild;
mod surplus;
use emit::*;
use grandchild::arm_consumes_binder_grandchild;
use surplus::collect_surplus_skippable_dups;
mod ownership;
mod tailcall;
pub(crate) use ownership::*;
use tailcall::*;

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
    /// 5786 CALLER-SURPLUS dup sites: the RETAIN-ONLY subset of `dup_sites` (just `collect_dup_sites` over the
    /// retain candidates, MINUS the `collect_shell_reclaim_child_dups` set) — occurrences dup'd because the
    /// binding has a genuine LATER live use in the CALLER (multi-use surplus), NOT because they are a consumed
    /// child of a reclaimed shell. The `Core::Call` caller-drop admit ((B), `call_arg_caller_drops`) keys on
    /// THIS set, not the full `dup_sites`: a shell-reclaim child-dup is already balanced by the shell drop, so
    /// caller-dropping it double-frees (the fst-sum "reclaims with the pair shell" trap). Computed ONCE in
    /// `select_function_of` (snapshot after the retain collector, before the shell collector). Empty otherwise.
    caller_surplus_dup_sites: HashSet<StructId>,
    /// SITE-A owned-binder set (`collect_sitea_owned_binders`): the `Core::Let` binder ids whose initializer
    /// is a genuinely OWNED value (`heap_operand_ownership == Owned`). Read ONLY by the `Core::CallClosure`
    /// SITE-A env-cell reclaim: when a closure operand is a `Core::LocalRef` to such a binder AND its
    /// occurrence is a whole-binder dup site (∈ `dup_sites`), the dup made a SURPLUS owned copy in the env
    /// cell that the borrowing apply never consumes, so the SITE-A drop reclaims it (the binder's own scope
    /// drop reclaims the original). A binder bound to a `Param`/view/wrapper is NOT here → SITE-A never drops
    /// a borrowed-from-caller cell (the 09-functions HOF `(h n)` param UAF). Empty for a body with no
    /// owned-initializer `let`-binding, so the fast path is untouched.
    sitea_owned_binders: HashSet<StructId>,
    /// SITE-A INVARIANT-BORROW-CLEAN closure-param set (`closure_env_invariant_borrow_clean_binders`, v-mem
    /// recognizer): the CLOSURE/fn-typed loop-PARAM binders that are INVARIANT (identity-passed every back-edge)
    /// and whole-body BORROW-CLEAN, so the per-application caller-side dup (`mark_binder_dups` CallClosure arm) is
    /// SPURIOUS (the borrowing apply never consumes the env cell). Read by the `Core::CallClosure` SITE-A env-cell
    /// reclaim: when a closure operand is a `Core::Param` for such a binder AND its occurrence is a per-application
    /// dup site (∈ `dup_sites` AT THAT SITE), the SITE-A drop reclaims that surplus per-application dup — while the
    /// loop-exit `looped_owned_param_drops` (which ALREADY reclaims this exact invariant borrow-clean param once)
    /// reclaims the entry-owned ref. The per-SITE dup check is LOAD-BEARING (v-mem-confirmed): for `(+ (f 0) (f 1))`
    /// `mark_binder_dups` makes ONE dup for two uses, so only ONE application site carries the dup'd operand and the
    /// OTHER carries the ENTRY ref — dropping at the entry-ref site would double-free the ref the loop-exit drop
    /// owns (and UAF a borrowed caller). Empty for a body with no invariant borrow-clean closure param, so the fast
    /// path is untouched.
    closure_env_borrow_clean_binders: HashSet<StructId>,
    /// IF-JOIN PER-ARM DROP plan (v-memory-safety co-design, the Core::If analog of the loop-join per-arm
    /// reconciliation). Keyed by a `Core::If` node id → the `(slot, d_is_then)` of each DIVERGENT heap
    /// let-binding live-in to it: a binding that ESCAPES on one arm (W) but is DEAD on the other (D). The
    /// post-body scope-drop (whole-body `binding_escapes_dup_aware`) sees escapes-on-the-W-arm and SUPPRESSES
    /// the drop → the D arm would LEAK. Populated at the `Core::Let` handler (before the body emit, using the
    /// upfront `dup_sites`); consumed at the `Core::If` handler, which emits an rc-aware `op_drop(slot)` on
    /// the D arm ONLY (never the W arm — the UAF bar) before that arm's join value. This is the SOLE reclaim
    /// for the divergent binder (the post-body drop is already off for it → no double-drop). Closes the
    /// map-select / tree / effects-tuple if-join leakers.
    ifjoin_arm_drops: HashMap<StructId, Vec<(u32, bool)>>,
    /// IF-JOIN OWNERSHIP-EQUALIZE dup plan (v-memory-safety co-design; FIX A for the map-select-family
    /// value-If leak). Keyed by a `Core::If` node that is a `let`-binding's VALUE (`let pick = (if C b …)`)
    /// whose OWNERSHIP DIVERGES per arm → the `(slot, dup_is_then)` of each EARLIER heap binder `b` the
    /// result MOVE-ALIASES on one arm (b escapes that arm → pick == b there, a borrow-view at rc1) while
    /// being OWNED-FRESH on the other. The arm-blind post-body ownership gate then classifies `pick`
    /// `!Owned` (from the alias arm) and SUPPRESSES its drop → the fresh arm's shell LEAKS. Equalize:
    /// emit an rc-aware `dup(b)` (`LocalGet slot; OP_DUP`) on the ALIAS arm (never the fresh arm) so `pick`
    /// is UNIFORMLY OWNED on both arms; then `pick`'s post-body drop is FORCED (see `ifjoin_forced_drops`)
    /// and correct on both (alias: pick-drop + b's own drop net one free of the shared cell; fresh: pick's
    /// shell freed, b's distinct shell freed, interior rc-aware). Populated in the `Core::Let` bindings loop
    /// BEFORE the value emit; consumed at the `Core::If` handler AFTER each arm's `emit_branch` (stack-neutral).
    ifjoin_arm_dups: HashMap<StructId, Vec<(u32, bool)>>,
    /// IF-JOIN FORCED post-body drop (pairs with `ifjoin_arm_dups`, FIX A). The `let`-binder ids whose
    /// value-If was ownership-equalized by an arm dup above: after the dup, the binder is genuinely OWNED,
    /// so its post-body scope-drop MUST fire — this set makes the post-body drop loop BYPASS the arm-blind
    /// borrowed-operand skip (`binder==value && !Owned`) for exactly these binders (never the genuine
    /// self-keyed row-op materialize-borrow the gate protects, breaker #45). Populated + consumed within the
    /// same `Core::Let` handler (a binder is here iff `ifjoin_arm_dups` got a plan for its value-If).
    ifjoin_forced_drops: HashSet<StructId>,
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
    /// INC1: the COMPOUND-payload subset of `nontail_match_reclaim_binders` — param binders whose tail-
    /// `MatchSum` shell the emit reclaims for a COMPOUND (fresh-rebuilt `(Node …)`/`#tuple(…)`) payload,
    /// populated in LOCKSTEP with the dup-pass (`collect_shell_reclaim_child_dups`'s `is_nontail_spine_param`
    /// arm) so every consumed shell child is dup'd BEFORE the param-slot deep-drop (dup ⟺ drop, no double-
    /// free). The emit's compound `param_reclaim` disjunct gates on membership here + `nontail_param_compound_
    /// extra_ok` (the interior-view alias-out exclusion). DISTINCT from `nontail_match_reclaim_binders`'s
    /// SCALAR path (`nontail_param_payload_ok`, which copies out — no dup). Empty for a non-INC1 body.
    nontail_compound_reclaim_binders: HashSet<StructId>,
    /// StrAt self-tail-loop scrutinee-alias drop set (#9271-followup): `StrAt` nodes whose borrow+back-edge
    /// `Param`/`LocalRef` scrutinee the lowering dups into a recycled MatchSum `str_slot` and orphans; the StrAt
    /// arm drops `str_slot` when marked (drop⟺dup, UAF-safe via `bytes-compact`). Full argument in the commit.
    strat_selfloop_scrut_drop: HashSet<StructId>,
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
    /// The def index whose body is being emitted — set by `select_function_of` (the caller `self_def`). Read by
    /// the `Core::Call` caller-drop admit (`call_arg_caller_drops` 5786 conjunct C) so it can EXCLUDE a caller
    /// that is itself a member of the callee's `mutual_loop_group` (a self/mutual non-tail self-call reclaims
    /// per-frame → a caller-drop there double-frees; only an EXTERNAL caller may caller-drop). `None` outside a
    /// `select_function_of` emit (the import-companion `body_has_caller_drop` passes the def index explicitly).
    pub self_def: Option<usize>,
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
const OP_MAP_MERGE: &str = "map-merge";
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
    fn go(db: &mut Db, top: StructId, id: StructId, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::MatchSum { scrutinee, root } = core_of(db, id) {
            let scrut_ty = type_of(db, scrutinee);
            let never_diverges = body_diverges(db, id);
            // Call the SAME gate the emit uses (single source of truth → exact import/emit agreement). A
            // StrAt/BytesSlice scrutinee is always a computed producer → stashed I32, so the stand-in
            // `Some((0, I32))` matches the emit's real stashed slot for the gate's purposes. `top` is the
            // TOP fn body (== the emit's `out.fn_body`), so the multi-consume disjunct's consume-count is
            // computed over the SAME body at import and emit → the added `drop` is imported iff emitted.
            if matchsum_view_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                Some((0, ValType::I32)),
                never_diverges,
                &root,
                Some(top),
            ) {
                return true;
            }
        }
        core_child_ids(db, id)
            .into_iter()
            .any(|c| go(db, top, c, seen))
    }
    let mut seen = HashSet::new();
    go(db, id, id, &mut seen)
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

/// Import-side companion of the NON-LOOPED CONDITIONAL PARAM DROP (`select_function_of` half-2): whether the
/// def's body would get at least one `plan_ifjoin_nested` D-arm drop for a DIVERGENT callee-owned heap param.
/// Mirrors the planning EXACTLY (same `code.dup_sites` reconstruction + the `!loops` + `nonlooped_param_callee_owned`
/// gates) so `collect_module_used_ops` imports `drop` iff the emit actually emits one — precise, no over-
/// declaration (the `str_at_does_not_over_declare_drop` discipline). `pub` for the module's op-collection.
pub fn def_emits_ifjoin_param_drop(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> bool {
    let Some(self_d) = self_def else {
        return false;
    };
    // Non-looped only (mirror the planning's `!loops`).
    if !mutual_loop_group(db, self_d).is_empty() {
        return false;
    }
    // Reconstruct `code.dup_sites` EXACTLY as `select_function_of` does (the four collectors that feed
    // `dup_sites`) so the ifjoin escape verdict (`binding_escapes_dup_aware(Some(dup))`) matches the emit.
    let mut dup: HashSet<StructId> = HashSet::new();
    let mut heap_binders: Vec<StructId> = Vec::new();
    collect_retain_candidate_binders(db, body, &mut heap_binders);
    collect_dup_sites(db, body, &heap_binders, &mut dup);
    collect_shell_reclaim_child_dups(db, body, &mut dup);
    collect_sumpayload_escape_dup_sites(db, body, &mut dup);
    collect_row_op_field_dups(db, body, &mut dup);
    let mut plan: HashMap<StructId, Vec<(u32, bool)>> = HashMap::new();
    for (param_index, (binder, ty)) in params.iter().enumerate() {
        // A heap param always has a machine slot (an i32 handle); the slot VALUE is an opaque tag here (the
        // plan's non-emptiness — not the slot — is what we test), so pass a dummy 0.
        if is_heap_type(ty) && nonlooped_param_callee_owned(db, self_d, param_index, layout) {
            let aliases = HashSet::from([*binder]);
            // MIRROR the emit's per-path AXIS B net-borrow admit + GATE-1 EXACTLY (select_function_of half-2),
            // so `drop` is imported iff a net-borrow (or dead) D-arm drop is actually emitted — no missing/over
            // import.
            let net_borrow = !def_nonlooped_callee_reclaims_threaded_param(db, self_d, param_index);
            plan_ifjoin_nested(db, body, &aliases, 0, &dup, net_borrow, &mut plan);
        }
    }
    !plan.is_empty()
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
        let heap_param_binders: Vec<StructId> = slot_binders
            .iter()
            .copied()
            .filter(|&b| is_heap_type(&type_of(db, b)))
            .collect();
        for i in 0..args.len() {
            if i >= param_slots.len() {
                continue;
            }
            let binder = slot_binders[i];
            if !is_heap_type(&type_of(db, binder)) {
                continue;
            }
            if !rebind_produces_fresh(db, args[i])
                && !reclaim::rebind_is_cross_param_move(db, args[i], binder, &heap_param_binders)
            {
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

/// AXIS A (caller-drop complementarity) for the LOOPED invariant-param reclaim — the "caller-reuse guard".
/// Whether the self-recursive def `self_d` (body `self_body`) OWNS its heap param `binder` on EVERY external
/// entry, so the loop-exit `op_drop` reclaims a genuinely-owned handle rather than one a CALLER still holds.
/// Mirrors [`def_nonlooped_reclaims_param`]'s AXIS A, adapted for the looped case. It declines (a) an EXPORT
/// entry (the boundary trampoline owns/drops the param — a loop-exit drop would double-free); (b) a funcref-
/// taken def or one called from a lifted body (an invisible `call_indirect`/eta edge could forward a borrowed
/// arg the direct call-site index cannot see); and (c) any def where some EXTERNAL (non-self) call site
/// passes a non-OWNED arg for this param (`heap_operand_ownership != Owned`). The SELF back-edge is EXCLUDED
/// from (c): an invariant param is identity-threaded as a bare `Param` (Borrowed) around the loop — owned-by-
/// flow (the same handle circulates), not a fresh entry. Empty external sites (only self-calls, or an unseen
/// edge) cannot prove ownership, so decline.
///
/// Sound-conservative: a wrong FALSE only forgoes the reclaim (a LEAK, never a UAF). This is the guard whose
/// ABSENCE let #9010's compare-arm reclaim drop a caller-REUSED param (the CAESAR `find-at` case: `rot-go`
/// passes its own borrowed `c` to `find-at` and reuses it after — a loop-exit drop of `c` inside `find-at`
/// freed the buffer `rot-go` still read, a use-after-free wasm trap). bcp1's `drive` param is `(BigInt.of n)`,
/// a FRESH owned construction at `main`'s call site, so it stays owned and the reclaim remains sound.
fn looped_invariant_param_caller_owned(
    db: &mut Db,
    self_d: usize,
    self_body: StructId,
    binder: StructId,
) -> bool {
    // An export entry's boundary param is owned/dropped by the trampoline (the nonlooped AXIS A rule).
    if db.exports.iter().any(|e| e.def == Some(self_d)) {
        return false;
    }
    // Invisible edges (call_indirect / eta-lifted) could forward a borrowed arg the direct index misses.
    if def_funcref_taken(db, self_body) || callee_called_from_lifted_body(db, self_d) {
        return false;
    }
    // Param position == arg position at every call site (def_params order == Apply arg order — the same
    // correspondence `def_nonlooped_reclaims_param` relies on).
    let dparams = crate::layout::def_params(db, self_d);
    let Some(param_index) = dparams.iter().position(|(b, _)| *b == binder) else {
        return false;
    };
    let sites = crate::infer::callee_call_site_args_with_caller(db, self_d);
    let mut saw_external = false;
    for (caller_body, args) in sites {
        if caller_body == self_body {
            continue; // self back-edge: the invariant param is owned-by-flow, not a fresh external entry.
        }
        saw_external = true;
        match args.get(param_index) {
            Some(&arg) if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned)) => {
            }
            _ => return false, // borrowed / unknown / missing at this site → not all-owned → decline.
        }
    }
    saw_external // no external call site proves ownership → decline (leak-safe).
}

/// Whether `binder` is read via a STRUCTURAL-COMPARE op (`= `/`<`/`String.compare` = `ValueEq`/
/// `ValueEqShaped`/`ValueCmp`/`StrCmp`/`BigIntCmp`/`RationalCmp`) anywhere in `id`. These are exactly the
/// borrow arms #9010 ADDED to `param_only_borrowed_or_backedge_rec`, and the ONLY ones whose loop-exit reclaim
/// the CAESAR UAF exposed. The PRE-EXISTING arms (`List.at`/`Bytes.at`/`Set.contains`/`Set.len`/`Map.size`/
/// `Map.lookup`) shipped SOUND across the corpus without any caller-owns guard (e.g. the `sum-at` List.at
/// walk, whose `main` FORWARDS a fresh-owned list at last use — a legitimate ownership transfer that the
/// conservative `heap_operand_ownership(LocalRef) == Borrowed` default cannot see). So the
/// `looped_invariant_param_caller_owned` guard is applied ONLY to a COMPARED invariant param — precisely the
/// #9010 regression shape — leaving the proven pre-existing reclaims untouched (no over-conservative flip).
/// The mechanism-wide hardening of the OTHER arms (with a transfer-aware liveness test, not this strict Owned
/// proxy) is a co-design follow-up with v-memory-safety (the borrow-arm family's owner). Conservative: a
/// wrong TRUE only widens the caller-owns gate (a leak, never a UAF).
fn param_compared_in_loop_body(db: &mut Db, id: StructId, binder: StructId) -> bool {
    match core_of(db, id) {
        Core::ValueEq { lhs, rhs }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. }
            if occurs_in(db, lhs, binder) || occurs_in(db, rhs, binder) =>
        {
            return true;
        }
        _ => {}
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| param_compared_in_loop_body(db, c, binder))
}

/// #9140 (v-memory-safety co-design): whether the SHARED heap param `slot` of a MUTUAL loop group (a
/// trampolined dispatch over `members`, all sharing one param-slot set) is reclaimable by the group's single
/// dispatch-loop exit drop. cut-1 is INVARIANT-only (identity-threaded): a member that RE-BOXES the shared
/// slot is varying → declined (a varying mutual param needs a cross-member per-back-edge old-value reclaim,
/// the `drop_old_borrowed` analog coordinated across the dispatch — a follow-up). Reclaimable iff EVERY
/// member (a) binds a heap param at `slot`, (b) keeps it INVARIANT (only identity-threaded on its member
/// back-edges — `invalidate_varying_params` over that member's body), (c) uses it BORROW + back-edge-only
/// (`param_only_borrowed_or_backedge` — a NON-tail consume by any member, e.g. a `read-do-next`/`read-do-form`
/// pair consuming `tree`, FAILS this → decline), AND — the load-bearing UAF guard (the mutual analog of the
/// single-member AXIS A `looped_invariant_param_caller_owned`) — every EXTERNAL entry (a non-group caller of
/// any member) passes the slot OWNED, with at least one such owned entry. A member with no external caller
/// imposes no constraint (intra-group back-edges are owned-by-flow); a truly external caller that passes it
/// BORROWED-and-reuses it (the CAESAR class) → decline (leak, never a double-free). Any export-boundary /
/// funcref-taken / lifted-called member conservatively declines (the trampoline / an invisible edge owns it).
/// Since a pure borrow + identity back-edge emits NO dup, each member leaves the slot at the SAME rc it
/// received, so a single rc1 exit drop is balanced (v-mem Q1: no cross-member dup ⟹ no double-free).
fn mutual_group_slot_reclaimable(
    db: &mut Db,
    members: &[usize],
    slot: u32,
    _param_slots: &[u32],
) -> bool {
    // Conservative invisible-edge / export-boundary decline (any member): the trampoline or an eta-lifted /
    // call_indirect edge could own or forward the handle in a way the direct call-site scan misses.
    for &m in members {
        let Some(body_m) = db.defs.get(m).and_then(|d| d.body) else {
            return false;
        };
        if db.exports.iter().any(|e| e.def == Some(m)) {
            return false; // an export entry's boundary param is owned/dropped by the trampoline.
        }
        if def_funcref_taken(db, body_m) || callee_called_from_lifted_body(db, m) {
            return false;
        }
    }
    let member_bodies: std::collections::HashSet<StructId> = members
        .iter()
        .filter_map(|&m| db.defs.get(m).and_then(|d| d.body))
        .collect();
    let mut any_external_owned = false;
    for &m in members {
        let Some(body_m) = db.defs.get(m).and_then(|d| d.body) else {
            return false;
        };
        let params_m = crate::layout::def_params(db, m);
        // Re-derive member `m`'s slot assignment exactly as the emit does (dense `0..n`, Unit elided).
        let mut slots_m: HashMap<StructId, u32> = HashMap::new();
        let mut pslots_m: Vec<u32> = Vec::new();
        for (b, ty) in params_m.iter() {
            if matches!(ty.strip_nominal(), Ty::Unit) {
                continue;
            }
            if valtype_of(ty).is_none() {
                return false;
            }
            let s = pslots_m.len() as u32;
            slots_m.insert(*b, s);
            pslots_m.push(s);
        }
        // The member's own binder + type at the shared slot (must be a heap param in EVERY member).
        let Some((binder_m, ty_m)) = params_m
            .iter()
            .find(|(b, _)| slots_m.get(b) == Some(&slot))
            .cloned()
        else {
            return false;
        };
        if !is_heap_type(&ty_m) {
            return false;
        }
        // (b) INVARIANT (identity-threaded) in member `m` — cut-1 declines a re-boxed (varying) shared slot.
        let mut invariant_m: std::collections::HashSet<StructId> =
            params_m.iter().map(|(b, _)| *b).collect();
        invalidate_varying_params(
            db,
            body_m,
            &pslots_m,
            &slots_m,
            members,
            m,
            &mut invariant_m,
            &params_m,
        );
        if !invariant_m.contains(&binder_m) {
            return false;
        }
        // (c) BORROW + back-edge-only in member `m` (a non-tail consume by any member fails this → decline).
        if !param_only_borrowed_or_backedge(db, body_m, binder_m, members, &pslots_m, &slots_m) {
            return false;
        }
        // Caller-ownership (the CAESAR UAF guard, generalized to the mutual group): every EXTERNAL entry
        // (a non-group caller of `m`) must pass the slot OWNED; a borrowed-and-reused external arg declines.
        let Some(param_index) = params_m.iter().position(|(b, _)| *b == binder_m) else {
            return false;
        };
        let sites = crate::infer::callee_call_site_args_with_caller(db, m);
        for (caller_body, args) in sites {
            if member_bodies.contains(&caller_body) {
                continue; // intra-group back-edge: owned-by-flow, not a fresh external entry.
            }
            match args.get(param_index) {
                Some(&arg)
                    if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned)) =>
                {
                    any_external_owned = true;
                }
                _ => return false, // external borrowed / unknown / missing → decline (leak, not UAF).
            }
        }
    }
    any_external_owned // no external owned entry proves ownership → decline (leak-safe).
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
    // #9140 MUTUAL group (more than one member): the members are trampolined into ONE dispatch loop sharing
    // one param-slot set (the bodies emitted inline under a `which`-discriminant dispatch), so there is a
    // SINGLE loop-exit and an invariant shared slot holds one identity handle throughout → a single exit drop
    // reclaims it iff EVERY member borrow+identity-threads it AND every external entry owns it. Delegated to
    // `mutual_group_slot_reclaimable` (which analyzes every member's body + the group-wide caller-ownership);
    // cut-1 is INVARIANT-only (a re-boxed/varying shared slot is declined = leak, never a double-free). This
    // is called once per emitted member-function; each is a distinct dispatch invocation reclaiming ITS OWN
    // external-entry handle at its own single exit, so no shared handle is double-dropped across members.
    if loop_members.len() > 1 {
        let mut drops = Vec::new();
        for (binder, ty) in params.iter() {
            if !is_heap_type(ty) {
                continue;
            }
            let Some(&slot) = slot_of.get(binder) else {
                continue;
            };
            if mutual_group_slot_reclaimable(db, &loop_members, slot, param_slots) {
                drops.push(slot);
            }
        }
        return drops;
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
        if invariant.contains(binder) {
            // INVARIANT path (UNCHANGED): the slot holds the SAME handle throughout → a single exit drop
            // reclaims it iff it is provably (borrow + tail-back-edge) only.
            if !param_only_borrowed_or_backedge(
                db,
                body,
                *binder,
                &loop_members,
                param_slots,
                slot_of,
            ) {
                // 5786(a) PARALLEL PATH (v-memory-safety placement over v-core-opt's wrapper): a base-
                // CONSUMING invariant param FAILS the borrow-only path (`List.push base` recurses in a result
                // position → `_ => false`), but if the consume is a base-collection reclaim whose result is
                // scalar-REDUCED / discarded (`param_consumed_reused_in_loop_body` — the walk with
                // `allow_base_consume_reduced = true` + the no-heap-child-escape fences) AND this frame OWNS
                // `binder` on entry (`looped_invariant_param_caller_owned`, HARD — the AXIS-A/CAESAR fence),
                // the slot's own ref is live + UNALIASED at loop exit: invariance forces `binder` identity-
                // threaded to its OWN slot, so the consume never rebinds that slot ⟹ `binder` is dup-backed ⟹
                // the persistent-extend PATH-COPIES ⟹ `binder` survives unconsumed. A single exit deep-drop
                // reclaims it (5786: node#1 spine + node#2 child, one cascade) with no UAF. DISJOINT from the
                // borrow-only path above (a base consume fails it), so no double-count. leak-over-UAF: a wrong
                // admit only over-covers a caller-owned invariant reclaim (a leak), never frees a borrowed param.
                if param_consumed_reused_in_loop_body(
                    db,
                    body,
                    *binder,
                    &loop_members,
                    param_slots,
                    slot_of,
                ) && looped_invariant_param_caller_owned(db, self_d, body, *binder)
                {
                    drops.push(slot);
                }
                continue; // borrow-only path failed; the 5786(a) parallel path decided (pushed or not).
            }
            // CALLER-REUSE GUARD (AXIS A) — scoped to the #9010 COMPARE-arm shape. Borrow-only-within-the-body
            // is necessary but NOT sufficient for a COMPARED invariant param: the loop-exit `op_drop` also
            // requires this frame to OWN the param on entry. A caller that passes it BORROWED and REUSES it
            // (the CAESAR `find-at` UAF) must NOT have its handle freed here. Applied ONLY to a compared param
            // (the arms #9010 added) so the proven pre-existing List.at/Bytes.at/Set/Map reclaims — whose
            // callers may legitimately FORWARD a fresh-owned binding at last use (`sum-at`) — are untouched.
            if param_compared_in_loop_body(db, body, *binder)
                && !looped_invariant_param_caller_owned(db, self_d, body, *binder)
            {
                continue; // a caller borrows/reuses this COMPARED invariant param → leave it (leak, not UAF).
            }
        } else {
            // VARYING-rebound path (INC2 (a) (B) slice-1): the slot is re-bound each iteration; the OLD
            // values are reclaimed on the back-edge (drop_old_borrowed), so the epilogue reclaims the FINAL
            // value's shell iff it is borrow/reclaimed-rebox-only (Q2/F1/F2) AND no terminal arm references
            // binder (no escaping child to double-free — v-mem's no-escape trivial-coverage case, no dup).
            if !varying_param_epilogue_droppable(
                db,
                body,
                *binder,
                &loop_members,
                param_slots,
                slot_of,
            ) {
                continue; // not provably safe → leave it (leak, never double-free).
            }
        }
        drops.push(slot);
    }
    drops
}

/// 11:1403 b2 co-fix — MY LANE (v-memory-safety) = this discriminator predicate; v-core-opt wires the SITE-A
/// env-cell drop that consults it. The set of CLOSURE/fn-typed loop-PARAM binders that are INVARIANT (identity-
/// passed on EVERY back-edge) AND provably BORROW-CLEAN whole-body (every non-back-edge use is a borrow — the
/// CallClosure-borrows-callee arm of `param_only_borrowed_or_backedge`). For such a binder the per-application
/// caller-side dup (`mark_binder_dups` CallClosure arm, reclaim.rs) is SPURIOUS: applying the closure only
/// BORROWS its env cell (the lifted body reads captures via `Core::Captured` arr-get and never self-drops —
/// SITE-A's invariant), so the dup'd env temp is DEAD after the borrowed apply. v-core-opt's SITE-A env-cell
/// reclaim (emit.rs `CallClosure`) drops that dead dup PER APPLICATION — balancing the per-iteration dups —
/// while the loop-exit epilogue drop (`looped_owned_param_drops`, which ALREADY reclaims this same invariant
/// borrow-clean param exactly once) reclaims the entry-owned ref. Net: dup/drop balanced → the 11:1403 shape
/// `times f n x = (if (< n 1) x (times f (- n 1) (f x)))` leak (leaks 1: one un-dropped env cell) clears.
/// DEFAULT-DENY (leak-over-UAF): a binder that is not PROVABLY invariant + borrow-clean is ABSENT from the set
/// → its dup stays un-dropped = the EXISTING leak, never an under-retain UAF. Excludes the 09-functions:0411
/// shape (a genuine SECOND CONSUME of the closure operand ⟹ `param_only_borrowed_or_backedge` denies ⟹ absent
/// ⟹ the dup is kept and feeds the real consume — no under-retain). SINGLE self-loop member only (the witnessed
/// combinator shape); a mutual dispatch group is an unwitnessed case → default-deny. Computed from
/// `body`/`params`/`self_def` alone (like `looped_owned_param_drops`), so the emit can thread the set down.
pub fn closure_env_invariant_borrow_clean_binders(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
) -> HashSet<StructId> {
    let mut out: HashSet<StructId> = HashSet::new();
    let Some(self_d) = self_def else {
        return out;
    };
    // Re-derive the param slot assignment EXACTLY as `select_function_of`/`looped_owned_param_drops` do
    // (dense `0..n`, Unit elided) so the invariance/borrow-clean queries see the emit's slots.
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_slots: Vec<u32> = Vec::new();
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return out; // a param with no machine rep → this def won't select.
        }
        let slot = param_slots.len() as u32;
        slot_of.insert(*binder, slot);
        param_slots.push(slot);
    }
    if param_slots.is_empty() {
        return out;
    }
    // SINGLE-member self-loop only (the witnessed `times` combinator). A mutual dispatch group shares one
    // param-slot set across members and is not the witnessed shape → default-deny (leak, never a UAF).
    let loop_members = mutual_loop_group(db, self_d);
    if loop_members.len() != 1 {
        return out;
    }
    // Params identity-passed on EVERY back-edge (invariant): a varying closure param's slot is replaced each
    // iteration, so its per-application dup is NOT the invariant-reclaim shape → default-deny.
    let mut invariant: HashSet<StructId> = params.iter().map(|(b, _)| *b).collect();
    invalidate_varying_params(
        db,
        body,
        &param_slots,
        &slot_of,
        &loop_members,
        self_d,
        &mut invariant,
        params,
    );
    for (binder, ty) in params.iter() {
        // Fn/closure-typed only — SITE-A is the `CallClosure` env-cell reclaim; a non-closure param is not
        // applicable (and would not reach the SITE-A dup at issue).
        if !matches!(ty.strip_nominal(), Ty::Fn(_, _)) {
            continue;
        }
        if !invariant.contains(binder) {
            continue; // varying closure param → default-deny.
        }
        // Whole-body borrow-clean: every non-back-edge occurrence is a borrow (incl. the CallClosure apply,
        // which borrows the env cell). This is the SAME predicate whose truth put this invariant param into
        // `looped_owned_param_drops` (the loop-exit drop that already reclaims the entry-owned ref once), so a
        // per-application SITE-A drop of the SPURIOUS dup is the only missing half.
        // Borrow-clean over the whole body AND not threaded through a NON-TAIL self-recursive call. The latter
        // fence (v-memory-safety, 09-functions:8750 `filt`) excludes a closure param handed to a fresh recursive
        // frame whose result is CONSUMED here (`(Iter.Cons h (filt rest p))`): that frame reclaims the param in
        // its own lifetime, so the SITE-A per-application env-drop would DOUBLE-reclaim it → over-free. A purely
        // tail-recursive loop (771 `times`) threads its closure param only in the tail back-edge → not flagged.
        // CALLER-OWNERSHIP fence (v-core-opt, #9440 21-host-closures UAF fix): the SITE-A per-application
        // env-drop of a closure PARAM is sound ONLY when the closure is GUEST-OWNED on every external entry —
        // exactly the AXIS-A gate the loop-exit `looped_owned_param_drops` already uses. Without it the b‴
        // Param-admit (which relaxes the "never Param" rule) also fires for a BOUNDARY-CONSUMED closure: the
        // `iter g n acc = (if (< n 1) acc (iter g (- n 1) (g acc)))` case passes every other check IDENTICALLY
        // to the fresh-closure `times`/#9443 `times2` (1-member tail-loop, `g` invariant, borrow-clean apply,
        // tail-threaded), but `iter`'s `g` enters via `apply-n`'s EXPORT param = a HOST RESOURCE the guest must
        // not reclaim; the per-application drop freed its env → the next iteration read freed env → wasm
        // `unreachable` (a shipped UAF). The recognizer cannot tell a fresh guest closure from a boundary
        // resource on `iter`'s body alone; `looped_invariant_param_caller_owned` DOES — it declines `iter`
        // (`apply-n` passes a Borrowed export param) while admitting `times`/`times2` (`main` passes a fresh
        // Owned `(mk-adder k)` producer). Leak-over-UAF: a boundary/unknown-owned closure param → decline
        // (a benign leak, value-correct), never a double-free.
        if param_only_borrowed_or_backedge(db, body, *binder, &loop_members, &param_slots, &slot_of)
            && !param_threaded_through_nontail_selfcall(db, body, *binder, &loop_members, true)
            && looped_invariant_param_caller_owned(db, self_d, body, *binder)
        {
            out.insert(*binder);
        }
    }
    out
}

/// Whether a direct call to `callee` CONSUMES (moves out) the arg at `param_index`. FALSE ⟹ the callee only
/// BORROWS it — reads it in place, identity-threads it on its own back-edge, reclaims a caller-transferred ref
/// at its own loop exit. Drives the borrowing-Call view reclaim (#9218-followup): (iii) the
/// `arm_borrows_heap_subvalue_seen` Call arm and (ii) the `returncall_shell_drop` fence treat a borrow-read
/// payload-view arg as borrowed. Uses back-edge-aware [`param_only_borrowed_or_backedge`], NOT
/// [`reclaim::param_escapes_body`] (which counts the self-recursive identity back-edge as an escape → `true`
/// for every recursive reader). INVARIANCE-GATED: a VARYING param (slot replaced on a back-edge) drops the old
/// value = consumed despite borrow-only body reads, so an invariant classification guards it (as in
/// `looped_owned_param_drops`). DEFAULT-DENY to CONSUMING on unresolvable/non-heap/varying/non-borrow —
/// leak-beats-UAF (a wrong "borrows" could double-free). v-memory-safety co-design; v-core-opt owns it.
pub(super) fn def_consumes_param(db: &mut Db, callee: usize, param_index: usize) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return true; // unresolvable callee → assume consuming (safe).
    };
    let params = crate::layout::def_params(db, callee);
    // Re-derive the callee's dense param-slot assignment (Unit elided), matching the emit + the slot maps in
    // `looped_owned_param_drops` so the member-identity-back-edge test compares against the right slots.
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_slots: Vec<u32> = Vec::new();
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return true; // a param with no machine rep → don't reason; assume consuming.
        }
        let slot = param_slots.len() as u32;
        slot_of.insert(*binder, slot);
        param_slots.push(slot);
    }
    let Some((binder, ty)) = params.get(param_index).cloned() else {
        return true; // arity mismatch → assume consuming.
    };
    if !is_heap_type(&ty) {
        return true; // a scalar param can't hold a heap view; keep the conservative default.
    }
    let members = {
        let g = mutual_loop_group(db, callee);
        if g.is_empty() { vec![callee] } else { g }
    };
    // INVARIANCE GATE (v-memory-safety rc-trace of 13-strings:1059): only an INVARIANT (identity-threaded into
    // its own slot on EVERY recursive edge, never replaced) param can be borrow-only-and-caller-reclaimable; a
    // varying (replaced-slot) param is consumed on the replacing edge even though its body reads look borrow-
    // only.
    let mut invariant: std::collections::HashSet<StructId> =
        params.iter().map(|(b, _)| *b).collect();
    invalidate_varying_params(
        db,
        body,
        &param_slots,
        &slot_of,
        &members,
        callee,
        &mut invariant,
        &params,
    );
    if !invariant.contains(&binder) {
        return true; // varying (slot replaced on a back-edge → old value dropped) → consumed.
    }
    // BORROW-only (⇒ NOT consumed) iff every use is a borrow or a member identity back-edge; any non-borrow
    // use / unmodeled node ⟹ false ⟹ report CONSUMES (default-deny, leak-beats-UAF).
    //
    // 5786 flip-to-0 (v-core-opt owns; v-mem co-designed): additionally reclassify a DUP-BACKED base-consume of
    // this INVARIANT param (List.push/prepend/insert/concat base — the `List.push base` reused-invariant idiom)
    // as a BORROW, so the CALLER RETAINS the base and drops it at its last use (main's post-loop read → the
    // consumed-reused-invariant-base leak clears to 0). SAFE only when DUP-BACKED: gate on the callee's own
    // `dup_sites` (the emit-dup set) so the base is a borrow ⟺ the op path-copies (rc>1) not FBIP-reuses (rc1) —
    // a rc1 reuse consumes the base, so a caller-drop would double-free. CONTAINED to THIS consumes-decision via
    // `allow_base_consume_reduced=true` + `Some(dup_sites)` on the `_rec` worker; the shared
    // `param_only_borrowed_or_backedge` entry (gating looped_owned_param_drops / closure reclaim / the 5786(a)
    // exit-drop) is UNTOUCHED — the #9423 global-classifier blast-radius lesson. The invariance gate above
    // already excludes a varying param; the loop's 5786(a) exit-drop keeps DECLINING when the caller reclaims
    // (caller_owned complementarity), and breaker's #9434 caller-BORROWED tripwire stays known-leak (main does
    // not own a borrowed base there → this stays conservative). guarded-all mandatory (a wrong reclassify into
    // some other List.push consumer surfaces cross-chapter).
    let mut cands: Vec<StructId> = Vec::new();
    collect_retain_candidate_binders(db, body, &mut cands);
    let mut dup_sites: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    collect_dup_sites(db, body, &cands, &mut dup_sites);
    !param_only_borrowed_or_backedge_rec(
        db,
        body,
        binder,
        &members,
        &param_slots,
        &slot_of,
        false,
        false,
        true,
        Some(&dup_sites),
    )
}

/// The EMIT side of [`def_nonlooped_reclaims_param`] (blx1): the param SLOTS a NON-looped def reclaims via
/// a fn-exit `op_drop`. Mirrors [`looped_owned_param_drops`]'s slot assignment (dense `0..n`, Unit elided)
/// and gates each heap param on the shared `def_nonlooped_reclaims_param` (SINGLE SOURCE OF TRUTH with the
/// `call_arg_caller_drops` (6b) yield, so caller-drop XOR this self-drop is exactly complementary). Empty
/// unless `self_def` is a non-looped def owning a borrow-only scalar-returning heap param (blx1: `classify`).
fn nonlooped_owned_param_drops(
    db: &mut Db,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> Vec<u32> {
    let Some(self_d) = self_def else {
        return Vec::new();
    };
    let mut drops = Vec::new();
    let mut slot = 0u32;
    for (param_index, (_binder, ty)) in params.iter().enumerate() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue; // Unit is zero-width — occupies no slot (mirrors the slot assignment).
        }
        if valtype_of(ty).is_none() {
            return Vec::new(); // a param with no machine rep → this def won't select; no drops.
        }
        let this_slot = slot;
        slot += 1;
        if is_heap_type(ty) && def_nonlooped_reclaims_param(db, self_d, param_index, layout) {
            drops.push(this_slot);
        }
    }
    drops
}

/// Whether the subtree `node` contains a direct `Core::Call` to `self_def` — i.e. this arm carries the
/// self-recursive call. Used by the 14966 non-tail-self-recursive param last-use reclaim to pick the
/// RECURSIVE arm of a tail `If` (the arm whose over-dup of an invariant borrow-param leaks), so the drop
/// self-yields on the base-case arm (which has no self-call → reclaims via the dead-param path).
fn arm_contains_self_call(db: &mut Db, node: StructId, self_def: usize) -> bool {
    fn go(db: &mut Db, id: StructId, self_def: usize, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::Call { callee, .. } = core_of(db, id)
            && callee == self_def
        {
            return true;
        }
        core_child_ids(db, id)
            .into_iter()
            .any(|c| go(db, c, self_def, seen))
    }
    let mut seen = HashSet::new();
    go(db, node, self_def, &mut seen)
}

/// Whether EVERY direct self-call to `self_def` reachable in `node` passes `binder` VERBATIM (a bare
/// `Core::Param`/`Core::LocalRef` to `binder`) at argument position `param_index` — the TRUE-INVARIANCE
/// gate the F7 non-tail-selfrec borrow-param drop needs. 14966 `mpow(base, e/2, md)` passes `base`/`md`
/// UNCHANGED (invariant) → admit; giter-takedrop `take(rest, n-1)` passes `rest` for its matched param
/// `it` (VARYING — a projection of the param, not the param) → decline. A per-arm last-use drop is sound
/// ONLY for a truly-invariant param: a varying param's recursive frame gets a DIFFERENT value (a child
/// projected out of it), so dropping the incoming ref frees a child the fresh frame still holds → UAF
/// (the #9466 regression that trapped cad-test-iterators giter-takedrop). `def_consumes_param`'s
/// invariance gate is TCO-slot-based and MISSES this — a NON-TAIL self-call is a fresh frame, not a
/// slot-replacing back-edge, so a matched-then-projected param is wrongly seen as invariant-borrow — so the
/// F7 admit must check recursive invariance DIRECTLY here. Returns false (leak-over-UAF DECLINE) on any
/// non-verbatim arg, arity mismatch, or if no self-call is present.
fn nontail_selfcall_passes_param_invariant(
    db: &mut Db,
    node: StructId,
    self_def: usize,
    param_index: usize,
    binder: StructId,
) -> bool {
    fn go(
        db: &mut Db,
        id: StructId,
        self_def: usize,
        param_index: usize,
        binder: StructId,
        seen: &mut HashSet<StructId>,
        saw: &mut bool,
    ) -> bool {
        if !seen.insert(id) {
            return true;
        }
        let self_call_args = match core_of(db, id) {
            Core::Call { callee, args, .. } if callee == self_def => Some(args),
            _ => None,
        };
        if let Some(args) = self_call_args {
            *saw = true;
            let Some(&arg) = args.get(param_index) else {
                return false; // arity mismatch → decline (leak-over-UAF)
            };
            let verbatim = matches!(
                core_of(db, arg),
                Core::Param { binder: b } | Core::LocalRef { binder: b } if b == binder
            );
            if !verbatim {
                return false; // the recursive frame gets a DIFFERENT value → not invariant → decline
            }
        }
        core_child_ids(db, id)
            .into_iter()
            .all(|c| go(db, c, self_def, param_index, binder, seen, saw))
    }
    let mut seen = HashSet::new();
    let mut saw = false;
    let all_ok = go(db, node, self_def, param_index, binder, &mut seen, &mut saw);
    all_ok && saw
}

/// 14966 (06-numeric): the LAST-USE-PER-ARM drop plan for a NON-TAIL self-recursive fn's INVARIANT
/// borrow-used-after heap params — the BORROW-classified sibling of F6(ii). `mpow(base,e,md)` is NON-TAIL
/// self-recursive (`hh = mpow(base, e/2, md)`, result squared); `base`/`md` are invariant heap params the
/// emit DUP's at the recursive self-call arg (a conservative over-dup — the emit does not honor the
/// `def_consumes_param==false` borrow verdict) and BORROW-uses after (BigInt arith borrows), but NEVER
/// drops → leak (rc climbs, the incoming owned ref of each recursive frame is orphaned). The base-case arm
/// reclaims them (the param is unused / borrow-read-then-dead → the existing dead-param/dead-binding drop);
/// the RECURSIVE arm does not.
///
/// Reclaim each frame's incoming ref at base's LAST USE PER ARM by planning an ifjoin arm-drop on the
/// RECURSIVE arm of the fn's tail `If` (the arm carrying the self-call), consumed by the `Core::If` emit's
/// per-arm-drop after the arm's reads / before its `End`. This SELF-YIELDS at the base-case arm (no
/// self-call there → no plan entry → the dead-param drop stands alone → no e=0 double-free), which a
/// frame-exit epilogue drop does NOT (it fires regardless of arm → the base-case double-free, the
/// v-memory-safety cycle-1 probe). Returns `(if_node, slot, is_then)` triples for `code.ifjoin_arm_drops`;
/// ALSO consulted by `collect_module_used_ops` (`def_emits_nontail_selfrec_borrow_param_drop`) so the `drop`
/// import matches the emit.
///
/// GATES (v-core-opt condition (a), ALL; leak-over-UAF strict):
///   - body is directly the fn's tail `Core::If` (a Let/Do-wrapped If is conservatively skipped);
///   - the param is heap AND `def_consumes_param(self, i) == false` (BORROW-classified — the callee never
///     drops it internally, so a per-arm drop is the reclaim, not a double);
///   - guest-owned at every external entry (`looped_invariant_param_caller_owned`) — LOAD-BEARING;
///   - dup-backed (`collect_dup_sites` non-empty — the over-dup this reclaims);
///   - NON-escaping (`binding_escapes_dup_aware == false` — no verbatim ctor-embed/return → no double-free);
///   - DISJOINT from F6(ii)'s `nontail_selfrec_owned_closure_param_drops` (a Fn closure param used in the
///     base-case arm gets F6(ii)'s frame-exit drop as its SOLE reclaim; adding a per-arm drop double-frees —
///     the (G')-twin, caught by the 857 sentinel).
fn plan_nontail_selfrec_borrow_param_arm_drops(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> Vec<(StructId, u32, bool)> {
    let mut out = Vec::new();
    let Some(self_d) = self_def else {
        return out;
    };
    if !body_is_self_recursive(db, body) {
        return out;
    }
    let Core::If { then_, else_, .. } = core_of(db, body) else {
        return out; // only a direct tail `If` (the recursion discriminant); else conservatively skip.
    };
    let then_rec = arm_contains_self_call(db, then_, self_d);
    let else_rec = arm_contains_self_call(db, else_, self_d);
    if !then_rec && !else_rec {
        return out;
    }
    // Re-derive the dense param-slot assignment (Unit elided), matching `select_function_of`.
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut next: u32 = 0;
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return Vec::new();
        }
        slot_of.insert(*binder, next);
        next += 1;
    }
    // YIELD to F6(ii) (disjoint-set, the (G')-twin): a Fn closure param F6(ii) frame-exit-drops must not
    // also get a per-arm drop (double-free — the 857 sentinel).
    let f6ii: HashSet<u32> =
        nontail_selfrec_owned_closure_param_drops(db, body, params, self_def, layout)
            .into_iter()
            .collect();
    for (param_index, (binder, ty)) in params.iter().enumerate() {
        if !is_heap_type(ty) {
            continue;
        }
        let Some(&slot) = slot_of.get(binder) else {
            continue;
        };
        if f6ii.contains(&slot) {
            continue;
        }
        if def_consumes_param(db, self_d, param_index) {
            continue; // callee CONSUMES → it owns the reclaim; not the borrow-used case.
        }
        if !looped_invariant_param_caller_owned(db, self_d, body, *binder) {
            continue; // guest-owned at external entry (load-bearing).
        }
        let mut dup_sites: HashSet<StructId> = HashSet::new();
        collect_dup_sites(db, body, &[*binder], &mut dup_sites);
        if dup_sites.is_empty() {
            continue; // not dup-backed → no over-dup to reclaim.
        }
        if binding_escapes_dup_aware(
            db,
            body,
            EscapeTarget::Binder(*binder),
            false,
            Some(&dup_sites),
            false,
        ) {
            continue; // escapes verbatim → a drop here would double-free.
        }
        // TRUE-INVARIANCE gate (#9466 regression fix — cad-test-iterators giter-takedrop UAF): only drop the
        // incoming ref in an arm whose self-call passes THIS param VERBATIM (invariant, like mpow's `base`).
        // A VARYING param (the recursive frame gets a projected/different value, like `take(rest, n-1)`'s
        // `it`->`rest`) must NOT be dropped here — the fresh frame holds a child of it, so the drop UAFs.
        // `def_consumes_param`'s invariance gate is TCO-slot-based and misses non-tail-recursion variance.
        if then_rec
            && nontail_selfcall_passes_param_invariant(db, then_, self_d, param_index, *binder)
        {
            out.push((body, slot, true));
        }
        if else_rec
            && nontail_selfcall_passes_param_invariant(db, else_, self_d, param_index, *binder)
        {
            out.push((body, slot, false));
        }
    }
    out
}

/// Import-side companion of [`plan_nontail_selfrec_borrow_param_arm_drops`]: whether the def emits at least
/// one such per-arm drop, so `collect_module_used_ops` imports `drop` iff the emit fires (mirrors
/// `def_emits_ifjoin_param_drop`). `pub` for the module's op-collection.
pub fn def_emits_nontail_selfrec_borrow_param_drop(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> bool {
    !plan_nontail_selfrec_borrow_param_arm_drops(db, body, params, self_def, layout).is_empty()
}

/// F6(ii) (09-functions:857): a per-FRAME drop of an OWNED CLOSURE PARAM in a NON-TAIL self-recursive
/// function. `go f d = (if (< d 1) (f 0) (+ (go f (- d 1)) (go f (- d 1))))` is TREE-recursive — both
/// self-calls are OPERANDS of `+` (NON-TAIL) — so `f` falls through all three existing owned-param
/// reclaims:
///   - SITE-A per-application env-drop EXCLUDES it (its non-tail-selfcall fence
///     [`param_threaded_through_nontail_selfcall`] is EXACTLY this shape — 8750 `filt`);
///   - [`looped_owned_param_drops`] fires only for a TCO'd single-exit loop (`go` is not TCO'd → the
///     emit never opens a `loop`, so no loop-exit epilogue);
///   - [`nonlooped_owned_param_drops`] fires only for a NON-recursive callee-owned borrow param.
///
/// Each real recursive frame receives its OWN `f` copy — an immutable arg slot, since non-tail recursion
/// is real wasm calls with distinct frames (no back-edge slot reuse) — and the two sibling consumes
/// `(go f ..)(go f ..)` each `dup` `f` (the consume-spare), but NOTHING drops the frame's own copy, so
/// `rc` climbs 1→7 with ZERO drops and leaks 3 (the env cell + captured `xs`). Reclaim it at the
/// per-frame owned-param epilogue (this fn's result already on the stack; `drop` takes `f` as a call ARG
/// and returns nothing, leaving the result undisturbed — the SAME shape as the looped/nonlooped epilogue
/// drops emitted alongside). The last frame's `f`-drop cascades the env-cell dtor to `xs`.
///
/// GATES (v-core-opt condition #82679, ALL must hold, per-param; leak-over-UAF strict):
///   (4) NON-TAIL self-recursion: [`body_is_self_recursive`] AND `f` threaded into a member self-call in
///       a NON-TAIL position ([`param_threaded_through_nontail_selfcall`] == TRUE) — the SAME predicate
///       SITE-A uses to EXCLUDE, so a closure param gets the SITE-A per-application drop XOR this
///       frame-exit drop, NEVER both (single-source complementarity). That predicate is SAFE-BIASED
///       toward TRUE/non-tail; here that is the OVER-FREE direction, so (G') below re-adds the yield.
///   (1) GUEST-OWNED at every external entry ([`looped_invariant_param_caller_owned`]) — LOAD-BEARING:
///       a BOUNDARY-CONSUMED closure (an export / host-resource param) must NOT get a frame-exit drop,
///       else the non-tail analog of the #9440 21-host-closures UAF. Declines `iter g n acc` (Borrowed
///       export `g`), admits a fresh guest `(mk-adder k)` producer.
///   (2) OWNED per-frame: `f` is dup-backed (∈ [`collect_dup_sites`]) — the sibling consume-spare, so
///       the frame genuinely OWNS the copy it must drop (not a bare borrow).
///   (3) APPLY-BORROW-ONLY + NON-ESCAPING, via the DUP-AWARE (member-aware) escape query
///       [`binding_escapes_dup_aware`]: a dup-backed consuming self-call arg is a RETAIN, not an escape,
///       so the query reports escape ONLY for a VERBATIM move — `f` embedded in a rebuilt ctor or
///       returned (the tr3 hazard). Such an escape would make the frame-exit drop a double-free.
///   (G') DISJOINT from the existing epilogue drop-sets ([`looped_owned_param_drops`] ∪
///       [`nonlooped_owned_param_drops`]) — the gate-(G) analog: never emit a second drop of a slot
///       those already reclaim. A no-op for 857 (neither fires for `f`), but fences an exotic over-free
///       where the safe-biased-toward-non-tail predicate (4) admits a param an existing set also drops.
fn nontail_selfrec_owned_closure_param_drops(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> Vec<u32> {
    let Some(self_d) = self_def else {
        return Vec::new();
    };
    // (4a) whole-body: must be self-recursive at all (else nothing to reclaim per-frame).
    if !body_is_self_recursive(db, body) {
        return Vec::new();
    }
    // The self-recursion member set for the non-tail-selfcall predicate. `mutual_loop_group` returns the
    // TCO'd SCC — EMPTY for a PURE non-tail self-recursive fn like `go` (no tail self-call) — so fall
    // back to the sole self def, the only member whose self-calls we must detect. A narrower member set
    // only makes the predicate return FALSE more often (leak-over-UAF safe).
    let mut members = mutual_loop_group(db, self_d);
    if members.is_empty() {
        members = vec![self_d];
    }
    // The existing epilogue drop-sets this runs ALONGSIDE — gate (G') yields to them (no double-drop).
    let looped: HashSet<u32> = looped_owned_param_drops(db, body, params, self_def)
        .into_iter()
        .collect();
    let nonlooped: HashSet<u32> = nonlooped_owned_param_drops(db, params, self_def, layout)
        .into_iter()
        .collect();

    let mut drops = Vec::new();
    let mut slot = 0u32;
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue; // Unit is zero-width — occupies no slot (mirrors the slot assignment).
        }
        if valtype_of(ty).is_none() {
            return Vec::new(); // a param with no machine rep → this def won't select; no drops.
        }
        let this_slot = slot;
        slot += 1;
        // Fn/closure-typed + heap (a closure cell is a heap value, `core_analysis::is_heap_type`).
        if !matches!(ty.strip_nominal(), Ty::Fn(_, _)) || !is_heap_type(ty) {
            continue;
        }
        // (G') never double-drop a slot an existing epilogue set already reclaims.
        if looped.contains(&this_slot) || nonlooped.contains(&this_slot) {
            continue;
        }
        // (1) guest-owned at every external entry — LOAD-BEARING (else the 21-host boundary UAF, non-tail form).
        if !looped_invariant_param_caller_owned(db, self_d, body, *binder) {
            continue;
        }
        // (4) non-tail self-recursion: `f` threaded into a member self-call in a NON-TAIL position.
        if !param_threaded_through_nontail_selfcall(db, body, *binder, &members, true) {
            continue;
        }
        // (2)+(3) owned-per-frame + apply-borrow-only/non-escaping, via the DUP-AWARE (member-aware) escape
        // query: build `f`'s dup sites, require it dup-backed (owned copy), and require no verbatim escape.
        let mut dup_sites: HashSet<StructId> = HashSet::new();
        collect_dup_sites(db, body, &[*binder], &mut dup_sites);
        if dup_sites.is_empty() {
            continue; // (2) not dup-backed → the frame does not own a spare copy to drop.
        }
        if binding_escapes_dup_aware(
            db,
            body,
            EscapeTarget::Binder(*binder),
            false,
            Some(&dup_sites),
            false,
        ) {
            continue; // (3) escapes verbatim (ctor-embed / return) → a frame-exit drop would double-free.
        }
        drops.push(this_slot);
    }
    drops
}

/// Import-side companion of [`nontail_selfrec_owned_closure_param_drops`]: whether the def with
/// `body`/`params`/`self_def` emits at least one non-tail-self-recursive owned-closure-param frame-exit
/// drop, so `collect_module_used_ops` imports `drop` iff the epilogue actually emits one (precise, not the
/// over-declaration the drop-minimization tests forbid). Mirrors [`def_drops_owned_param`]. `pub` for the
/// module's op-collection.
pub fn def_drops_nontail_selfrec_closure_param(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: Option<usize>,
    layout: &Layout,
) -> bool {
    !nontail_selfrec_owned_closure_param_drops(db, body, params, self_def, layout).is_empty()
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
    // 5786 caller-drop: expose the emitting def index so the `Core::Call` caller-drop admit can exclude a
    // caller inside the callee's own mutual-loop group (conjunct C — external-caller-only).
    code.self_def = self_def;
    // INC1: the non-tail-spine owned-param reclaim SELECTION uses a COMBINATOR-aware boundary guard, NOT the
    // global `is_boundary_owned` (which 05:18721's surplus_skippable_dups + call_arg_caller_drops read as
    // exports||db.lifted). A lifted COMBINATOR (empty captures — hoisted to funcref, called directly,
    // callee-owned) is EXCLUDED by the global flag but IS reclaimable here (BST del-min/Peano); only an
    // EXPORT entry (caller-built cell) or a genuine CAPTURING closure (closure-arg boundary-built) stays
    // excluded. `call_arg_caller_drops` gate(5) excludes looped callees, and every INC1 target is
    // self-recursive → mutually exclusive with the caller-drop by construction (v-runtime rc-confirmed).
    // APPROACH B (v-core-opt single-source-of-truth): admit ALL lifted COMBINATORS (empty captures =
    // callee-owned) into the INC1 non-tail-spine reclaim. The mutual exclusion with the caller-drop is on
    // the CALLER side — `call_arg_caller_drops` YIELDS to a callee that INC1-reclaims the param, querying the
    // SAME selection (`def_inc1_reclaims_param`) so caller-drop XOR INC1-reclaim is exactly complementary
    // per (edge, param). Only EXPORT entries + genuine CAPTURING closures are wholly boundary-excluded.
    // tr3 REFINEMENT (v-mem G6a sign-off): the `body_is_capturing_lifted` exclusion is gated on
    // `!body_is_self_recursive` — a SELF-RECURSIVE def whose body carries non-empty captures ONLY as a
    // nested-let-continuation lambda-lift artifact (e.g. `depth`'s `(let ((a (depth l)) (b (depth r))) …)`)
    // is still a DIRECT-called combinator, NOT caller-drop'd (`call_arg_caller_drops` gate(5) excludes ALL
    // looped callees), so its captures are SPURIOUS for the caller-drop question → it must self-reclaim (a
    // neither-drops LEAK otherwise). The `exports` conjunct stays UNRELAXED: an export-trampoline still owns
    // its rebuilt param (the 21-host-closures:6896 double-free class), so a self-recursive-AND-exported body
    // stays wholly excluded.
    // SCOPE (v-mem corpus-wide guarded-all, 2026-09-03): this relax admits the self-recursive-capturing body
    // into `nontail_match_reclaim_binders` (this set), reclaimed ONLY via the SCALAR payload path
    // (`nontail_param_payload_ok`, which excludes ctor-rebuild arms) — depth/max/balance return scalars. The
    // COMPOUND path (`is_nontail_spine_param` in reclaim.rs) is deliberately NOT relaxed for a capturing body:
    // a capturing self-recursive arm that rebuilds a ctor embedding a param-payload child (subst/rename:
    // `(Term.Abs w body)`) would self-reclaim + free the still-referenced escaped child → UAF (4 traps).
    let inc1_wholly_excluded = layout.exports.iter().any(|e| e.body == body)
        || (body_is_capturing_lifted(db, body) && !body_is_self_recursive(db, body));
    // INC1 SELF-RECURSION gate: only a self-recursive fold's owned-param shell is safe to reclaim here. A
    // non-self-recursive owned-param match (esp. the boundary-REBUILT compound-Result CLOSURE arg whose
    // export trampoline ALSO drops it) would DOUBLE-FREE — MEASURED on 21-host-closures:6896 (guest func-12
    // INC1 drop + guest func-15 trampoline drop → runtime drop-guard trap). See `body_is_self_recursive`.
    let nontail_reclaim: HashSet<StructId> = if inc1_wholly_excluded
        || !body_is_self_recursive(db, body)
    {
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
        // 5786: snapshot the RETAIN-ONLY dup set NOW (only `collect_dup_sites` has run — before the shell/
        // escape/row collectors union into `code.dup_sites`), MINUS the shell-reclaim child-dups → the
        // caller-surplus set the `Core::Call` caller-drop admit (B) keys on. A shell-reclaim child-dup is
        // already balanced by the shell drop, so caller-dropping it double-frees (the fst-sum trap).
        {
            let retain_only = code.dup_sites.clone();
            let mut shell: HashSet<StructId> = HashSet::new();
            collect_shell_reclaim_child_dups(db, body, &mut shell);
            code.caller_surplus_dup_sites = retain_only.difference(&shell).copied().collect();
        }
        // SITE-A owned-binder set: the let-binders whose initializer is a genuinely Owned value, so a dup'd
        // reference to one consumed by a BORROWING closure apply is a surplus owned copy the SITE-A env-cell
        // drop reclaims (a Param/view/wrapper-bound binder is excluded → never drops a borrowed-from-caller cell).
        collect_sitea_owned_binders(db, body, &mut code.sitea_owned_binders);
        // SITE-A invariant-borrow-clean closure-param set: the CLOSURE/fn-typed loop-params whose per-application
        // caller dup is spurious (invariant + borrow-clean), so the CallClosure SITE-A drop reclaims the surplus
        // per-app dup (loop-exit `looped_owned_param_drops` owns the entry ref). Same threading shape as
        // `sitea_owned_binders`; `params`/`self_def` here match `looped_owned_param_drops`'s call above.
        code.closure_env_borrow_clean_binders =
            closure_env_invariant_borrow_clean_binders(db, body, params, self_def);
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
        // SURPLUS GATE (05:18721, bisect #7255/#7321) + OWNED-FOLD extension (03:522/06 family) — full
        // rationale + the coarse liveness-across-vec-split predicate in surplus.rs. `owned_fold` (self-
        // recursive) relaxes conjunct 3; guarded-all is the net.
        let owned_fold = body_is_self_recursive(db, body);
        if is_boundary_owned || owned_fold {
            collect_surplus_skippable_dups(
                db,
                body,
                &code.dup_sites,
                owned_fold,
                &mut code.surplus_skippable_dups,
            );
        }
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
    // INC1: the COMPOUND-payload subset, populated in LOCKSTEP with the dup-pass
    // (`collect_shell_reclaim_child_dups` → `is_nontail_spine_param`), so the emit's compound param-shell
    // drop fires only where the consumed shell children were dup'd (dup ⟺ drop). Empty for a non-INC1 body.
    collect_nontail_compound_reclaim_binders(db, body, &mut code.nontail_compound_reclaim_binders);
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
    // NON-LOOPED CONDITIONAL PARAM DROP (v-memory-safety, half-2 of the growing-heap-recursive-fold-state
    // leak): a callee-owned heap param CONSUMED on some control paths but DEAD (neither escaped nor consumed)
    // on others is never reclaimed on the dead paths — the unconditional fn-exit drop
    // (`nonlooped_owned_param_drops`) requires borrow-only-ALL-paths, and the epilogue has no per-path form.
    // Plan a D-arm drop on each DEAD arm of a DIVERGENT `If` via the SAME primitive the let-binding reclaim
    // uses (`plan_ifjoin_nested` / `ifjoin_arm_dead`), keyed on the `If` node so `emit_tail`'s `Core::If` arm
    // reclaims it after the dead arm's reads. SOUND: `nonlooped_param_callee_owned` proves the frame owns a
    // ref; `plan_ifjoin_nested` plans the drop ONLY on the arm that neither escapes nor consumes the binder
    // (the other arm consumes/returns it) → balanced, no double-free (it is self-gating on divergence — a
    // param consumed/escaped on EVERY path plans nothing). The non-looped fold's discarded FINAL state (base
    // case `(byte-len s)` — borrow-read, dead, undropped) is reclaimed on its dead arm. LOOPED folds already
    // reclaim via the loop epilogue, so this runs `!loops` only. Beneficiary: the effect-handler string-rope
    // state-exit leak (#9074 14b / #9077 14-effects) + plain recursive folds.
    if !loops && let Some(self_d) = self_def {
        let dup = code.dup_sites.clone();
        for (param_index, (binder, _ty)) in params.iter().enumerate() {
            if let Some(&slot) = slot_of.get(binder)
                && nonlooped_param_callee_owned(db, self_d, param_index, layout)
            {
                let aliases = std::collections::HashSet::from([*binder]);
                // PER-PATH AXIS B net-borrow admit (v-core-opt-blessed, #9074/#9077 handler-state class): also
                // reclaim this callee-owned param on a NET-BORROW arm (every consume dup-backed → the incoming
                // ref is surplus/dead-after), not just a fully-DEAD arm. GATE-1 (closes the go two-sibling
                // double-free): decline the net-borrow admit when the callee ALREADY reclaims this param via the
                // conditional threaded-param drop — a second drop on that arm would double-free. This site is the
                // sole net-borrow-enabled caller (the other `plan_ifjoin_nested` callers pass `net_borrow=false`;
                // GATE-1's own predicate uses an empty dup set so it stays inert). Base-MOVE arms and lf1's
                // capture-escape are declined by the dup-aware `ifjoin_arm_dead` escape check + the mandatory
                // go/lf1 census negative controls (leak-over-UAF, REVERT on any red).
                let net_borrow =
                    !def_nonlooped_callee_reclaims_threaded_param(db, self_d, param_index);
                plan_ifjoin_nested(
                    db,
                    body,
                    &aliases,
                    slot,
                    &dup,
                    net_borrow,
                    &mut code.ifjoin_arm_drops,
                );
            }
        }
    }
    // 14966: non-tail self-recursive INVARIANT borrow-used-after param LAST-USE-PER-ARM drop (v-core-opt
    // condition (a)). Plan drops on the RECURSIVE arm(s) of the tail `If` (reuses the ifjoin per-arm-drop
    // emit). See `plan_nontail_selfrec_borrow_param_arm_drops`.
    if !loops {
        for (if_node, slot, is_then) in
            plan_nontail_selfrec_borrow_param_arm_drops(db, body, params, self_def, layout)
        {
            code.ifjoin_arm_drops
                .entry(if_node)
                .or_default()
                .push((slot, is_then));
        }
    }
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
        selfloop_scrut_slot: None,
        list_scrut_divergent: false,
        returncall_shell_drop: None,
    });
    // rp2 / 13-strings:7255 (SumExpect-shape self-loop `String.at` view-mint): the MatchSum-scrutinee StrAt
    // marking (in the `Core::MatchSum` emit arm, select.rs ~3627) CANNOT reach the shape `(= (Option.expect
    // (String.at s i)) X)` — there is NO `Core::MatchSum` node (the char is extracted by `SumExpect` and
    // compared by `ValueEq`). PRE-mark such StrAt view-mints here so the emit per-read `str_slot` drop
    // (`strat_selfloop_scrut_drop`, emit.rs ~2488) fires, balancing the +1/iter container-alias dup that a
    // self-loop `String.at` scan orphans (v-mem rc-trace: node#52 106dup/54drop → leak; with the drop
    // 106/107 → census 0). Gated (leak-over-UAF): `SumExpect(StrAt(Param/LocalRef s))` + the char CONSUMED
    // IN-PLACE (F1 fence — `collect_consuming_payload_sites_expr` EMPTY: a scalar compare descends
    // `consuming=false`, an escape into a ctor/return/Call is a site → an escaping char like the `last`
    // return is NOT marked) + `s` borrow/back-edge-only (`param_only_borrowed_or_backedge`, via the StrAt
    // borrow arm). UAF-safe: the char is `bytes-compact`'d to an independent leaf (emit.rs ~2452), so freeing
    // the container never dangles it. Only runs for a self-tail-loop (`loops`). v-core-opt + v-mem-safety.
    if loops {
        let self_d = self_def.expect("a loop has a self_def");
        mark_sumexpect_strat_selfloop_drops(
            db,
            body,
            self_d,
            &loop_members,
            &param_slots,
            &slot_of,
            &mut code,
        );
    }
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
    // blx1 (v-mem co-design): the NON-looped analog — a callee-owned borrow-only scalar-returning heap
    // param with no other reclaim (a bin-match borrow → `classify`) is dropped here, its sole reclaim.
    // Gated on `def_nonlooped_reclaims_param` (SAME query the `call_arg_caller_drops` (6b) yield uses → no
    // double-free). Same exit-drop shape as the looped case (result undisturbed beneath on the stack).
    for slot in nonlooped_owned_param_drops(db, params, self_def, layout) {
        code.push(Lir::LocalGet(slot));
        code.push(Lir::CallImport(OP_DROP));
    }
    // F6(ii) (09-functions:857): the NON-TAIL self-recursive owned-CLOSURE-param frame-exit drop — the
    // tree-recursion analog of the two loop/non-loop epilogue drops above. A closure param threaded into a
    // NON-TAIL member self-call (`(+ (go f ..) (go f ..))`) is dup'd per sibling consume but never dropped;
    // each real recursive frame owns its `f` copy and must reclaim it here (result already on the stack).
    // Gated (guest-owned + owned-per-frame + apply-borrow-only/non-escaping + disjoint from the two sets
    // above) so it NEVER double-frees — see `nontail_selfrec_owned_closure_param_drops`. Same drop shape.
    for slot in nontail_selfrec_owned_closure_param_drops(db, body, params, self_def, layout) {
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

mod body_analysis;
use body_analysis::*;

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
    /// INC1 SELF-LOOP-TAIL shell reclaim (pt3, v-mem G1-G7 gated): the PARAM SLOT holding an owned
    /// compound-boxed match scrutinee that is DEAD-after-iteration in a tail-loop (its children are extracted
    /// — one carried into a loop-param, siblings consumed by non-tail sub-calls — all already dup'd, so the
    /// old node shell array is dead once its children are read). Unlike [`scrut_shell_reclaim`] (a STASHED
    /// slot), this is the REASSIGNED loop-param slot, so `emit_loop_iteration` must SAVE it to a scratch
    /// BEFORE the back-edge reassign then deep-`op_drop` the scratch AFTER (the `drop_old_borrowed` save-path
    /// shape, NOT the post-reassign `scrut_shell_reclaim` drop which would free the NEW value). No dup added:
    /// every child already has its escape dup (dup ⟺ cascade lockstep, G6), so the deep op_drop cascade nets
    /// each child to its single surviving owner and frees the old shell. Set by the tail `MatchSum` emit ONLY
    /// when the G1-G7 gate holds (the self-loop-tail scrutinee is dead-after-iteration, not returned/aliased,
    /// children all dup-backed). `None` on every ordinary loop. Fixes the inorder self-tail-traversal spine
    /// leak (the ~165 self-loop-tail lever). SOUNDNESS (double-free) rests on the G6 dup-lockstep gate.
    selfloop_scrut_slot: Option<u32>,
    /// INC2 slice-1 (divergent-consume loop-join, v-mem's W/C/D predicate): the `selfloop_scrut_slot` above
    /// names a MatchLIST scrutinee that is DIVERGENT — reused-WHOLE in one tail arm (the W arm, forcing the
    /// RestFrom preservation-`dup` to fire because the scrutinee is NOT sole-consumed) AND advanced-to-TAIL
    /// (`(.. r)`) in another (the C arm). In the C arm the RestFrom emits `dup(scrut); vec-drop`, so the
    /// `vec-drop` nets the DUP (rc2→1) but leaves the ORIGINAL alloc-rc1 shell orphaned when the loop-param
    /// slot is reassigned to the tail → a per-iteration leak (v-runtime P6 rc-trace: Sum shells #8/10/12/14).
    /// When `true`, `emit_loop_iteration`'s `selfloop_scrut_scratch` does NOT short-circuit on
    /// `is_restfrom_consume` for this slot (the usual skip assumes the `vec-drop` fully consumed the shell —
    /// TRUE only for a SOLE-consume non-divergent RestFrom where the dup was skip-gated; FALSE here, the dup
    /// fired), so it saves + `op_drop`s the orphaned shell. GATED (F3-conservative): set ONLY when the
    /// scrutinee is provably divergent (`count_param_consumes > 1`) — else the dup was skipped and the
    /// `vec-drop` already freed the original, so dropping again would DOUBLE-FREE. `false` on every MatchSum
    /// path (the existing pt3 behavior is unchanged).
    list_scrut_divergent: bool,
    /// A scratch/param slot holding an enclosing tail-`MatchSum`'s OWNED scrutinee SHELL that is DEAD on a
    /// CROSS-FUNCTION `return_call` arm (a mutual-recursion tail call to a NON-loop-member peer) and must be
    /// `drop`ed BEFORE the `ReturnCall` — else it leaks (the post-match fall-through reclaim drop, which the
    /// value-returning arms reach, is `br`/`return`ed PAST by the tail call). The self-loop twin of
    /// [`scrut_shell_reclaim`]/[`selfloop_scrut_slot`], but for a CROSS-FN `return_call` (which `member_which`
    /// does NOT loop-iterate) rather than a self-loop back-edge. Set by the tail `MatchSum` emit ONLY when the
    /// shell is reclaimable (`reclaim_shell`) AND the return_call arm's args carry NO payload-of-scrutinee
    /// (`!expr_tail_is_call_consuming_payload` — the UAF fence: the callee must not receive a live handle into
    /// the shell we free; checked at the MatchSum where the scrutinee is in scope, so the `Core::Call` arm can
    /// drop UNCONDITIONALLY when this is `Some`). Fires on a DIFFERENT exit path than the post-match drop
    /// (return vs fall-through), so no double-free. The fallible-parser `pf` t==0 `return_call pe(i+1)`
    /// Some-shell leak (fp residual 1→0). `None` on every ordinary loop/match.
    returncall_shell_drop: Option<u32>,
}

impl TailLoop<'_> {
    /// The discriminant (index in `members`) of a tail-call callee that is a loop member, or `None` if
    /// the callee is not in this loop's group (so the call stays a `return_call`).
    fn member_which(&self, callee: usize) -> Option<usize> {
        self.members.iter().position(|&m| m == callee)
    }
}

/// Drop an enclosing tail-`MatchSum`'s dead owned scrutinee shell BEFORE a cross-fn `return_call`, when the
/// MatchSum emit set [`TailLoop::returncall_shell_drop`] (it is reclaimable AND the return_call args carry no
/// payload-of-scrutinee — the UAF fence, checked at the MatchSum where the scrutinee is in scope, so this
/// drops UNCONDITIONALLY here). The args are already on the stack; `OP_DROP` consumes only the pushed shell
/// handle, leaving them intact. Fires on the return_call exit path (which `return`s past the post-match
/// reclaim drop), never the value-fall-through path → no double-free. Deep + rc-aware, so its cascade nets
/// the dup-backed children exactly as the post-match drop does.
fn emit_returncall_shell_drop(tl: &Option<TailLoop>, out: &mut Emit) {
    if let Some(s) = tl.and_then(|t| t.returncall_shell_drop) {
        out.push(Lir::LocalGet(s));
        out.push(Lir::CallImport(OP_DROP));
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

/// rp2 / 13-strings:7255 — PRE-mark `SumExpect(String.at param)` view-mints in a self-tail-loop body so the
/// emit per-read `str_slot` drop (`strat_selfloop_scrut_drop`, emit.rs ~2488) balances the +1/iter
/// container-alias dup. The `Core::MatchSum` emit arm's own StrAt marking (select.rs ~3627) covers the
/// MATCH-scrutinee shape; this covers the SUMEXPECT shape `(= (Option.expect (String.at s i)) X)`, which has
/// NO `Core::MatchSum` node (the char is extracted by `SumExpect`, compared by `ValueEq`). Marks a
/// `StrAt(Param/LocalRef s)` iff its `SumExpect`'d char is CONSUMED-IN-PLACE (F1 fence:
/// `collect_consuming_payload_sites_expr` EMPTY — a scalar compare descends `consuming=false`; an escape into
/// a ctor/return/`Call` is a site, so a returned/threaded char like `last`'s is NOT marked) AND `s` is
/// borrow/back-edge-only (`param_only_borrowed_or_backedge`, supplied by the `StrAt` borrow arm). Leak-over-
/// UAF: an escaping char (F1 non-empty) or a consumed `s` (pobb false) is left unmarked = a leak, never a
/// double-free; the drop's own UAF-safety is the `bytes-compact` independent-leaf fence (emit.rs ~2452).
fn mark_sumexpect_strat_selfloop_drops(
    db: &mut Db,
    fb: StructId,
    self_d: usize,
    members: &[usize],
    param_slots: &[u32],
    slots: &HashMap<StructId, u32>,
    code: &mut Emit,
) {
    // Collect every `SumExpect(StrAt(Param/LocalRef s))` occurrence in the body (mirrors the select.rs ~3627
    // MatchSum-scrutinee shape check, but found anywhere — the SumExpect sits inside a recursive-call arg,
    // not at a tail node).
    let mut seen: HashSet<StructId> = HashSet::new();
    let mut stack = vec![fb];
    let mut cands: Vec<(StructId, StructId)> = Vec::new(); // (StrAt node, `s` binder)
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Core::SumExpect { scrutinee, .. } = core_of(db, id)
            && let Core::StrAt { string, .. } = core_of(db, scrutinee)
            && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, string)
        {
            cands.push((scrutinee, binder));
        }
        for c in core_child_ids(db, id) {
            stack.push(c);
        }
    }
    for (strat, binder) in cands {
        // F1: the `SumExpect`'d char must be consumed IN PLACE (no escape site) — a scalar compare descends
        // `consuming=false` (not a site); an escape into a ctor/return/`Call` IS a site.
        let mut sites: HashSet<StructId> = HashSet::new();
        collect_consuming_payload_sites_expr(db, fb, strat, true, &mut sites);
        if !sites.is_empty() {
            continue;
        }
        // (5) pobb: `s` is only borrowed / back-edge-threaded across the loop (the `StrAt` borrow arm supplies
        // this for the `SumExpect(StrAt)` reach). (6) CALLER-OWNED (AXIS-A): mark only a CALLEE-OWNED param
        // (a guest-built rope handed off, rp2); a BOUNDARY-OWNED entry-arg (the host owns the cell, the def
        // borrows it) leaves a +1 residual under the per-read drop → decline (leak-over-UAF), same fence as
        // looped_owned_param_drops + the MatchSum-marking arm.
        if param_only_borrowed_or_backedge(db, fb, binder, members, param_slots, slots)
            && looped_invariant_param_caller_owned(db, self_d, fb, binder)
        {
            code.strat_selfloop_scrut_drop.insert(strat);
        }
    }
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
/// Whether the last-emitted instruction is a control-flow TERMINATOR — the branch already left no value on
/// the stack and control has exited (a tail `ReturnCall`/`Return`, an unconditional `Br` back-edge, an
/// `Unreachable`/diverging-if end). Used by the `emit_tail` `Core::If` arm's IF-JOIN per-arm param drop to
/// SKIP a trailing reclaim that would be dead/unreachable after such an arm (leak-not-UAF on that shape).
fn ends_in_terminator(last: Option<&Lir>) -> bool {
    matches!(
        last,
        Some(
            Lir::ReturnCall(_)
                | Lir::Return
                | Lir::Br(_)
                | Lir::Unreachable
                | Lir::IfUnreachableEnd
        )
    )
}

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
                            emit_returncall_shell_drop(&tl, out);
                            out.push(Lir::ReturnCall(idx));
                            return Ok(());
                        }
                    };
                    let callee_vt = valtype_of(&callee_result_ty);
                    if callee_vt == out.fn_ret_vt {
                        trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit TAIL call (return_call)");
                        emit_returncall_shell_drop(&tl, out);
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
            // IF-JOIN PER-ARM DROP (v-memory-safety half-2): reclaim a DIVERGENT callee-owned heap PARAM
            // (consumed on one arm, dead on the other) on its DEAD arm — the TAIL twin of the value-`emit`
            // `Core::If` arm's `ifjoin_arm_drops` consumer. Planned upfront in `select_function_of`
            // (`nonlooped_param_callee_owned` + `plan_ifjoin_nested`), keyed on THIS `If` node. Emitted AFTER
            // each arm's body (not at the arm top): the D arm may READ the binder (`(byte-len s)`) before it
            // is reclaimed. The drop `LocalGet(slot); OP_DROP` pops only the binder, leaving the arm's result
            // beneath — UNLESS the arm ended in a TERMINATOR (a tail `ReturnCall`/`Return`/diverge left no
            // value + control already exited): then a trailing drop is dead/unreachable, so SKIP it (a leak
            // on that rare shape, never a UAF — leak-over-UAF). The common fold DEAD arm is a scalar VALUE
            // (`byte-len`), which leaves its result and no terminator → the drop fires.
            let ifjoin_plan = out.ifjoin_arm_drops.remove(&id).unwrap_or_default();
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
            // IF-JOIN D-THEN drop: reclaim a divergent owned param whose DEAD arm is the THEN arm, AFTER its
            // reads, before the arm exits — unless the arm ended in a terminator (control already left; a
            // trailing drop is unreachable → skip, a leak-not-UAF on that rare tail-call-dead-arm shape).
            if ifjoin_plan.iter().any(|&(_, d_is_then)| d_is_then)
                && !ends_in_terminator(out.code.last())
            {
                for &(slot, d_is_then) in &ifjoin_plan {
                    if d_is_then {
                        out.push(Lir::LocalGet(slot));
                        out.push(Lir::CallImport(OP_DROP));
                    }
                }
            }
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
            // IF-JOIN D-ELSE drop (twin of the D-THEN drop above): reclaim a divergent owned param whose DEAD
            // arm is the ELSE arm, after its reads, before the block closes; skip on a terminator-ended arm.
            if ifjoin_plan.iter().any(|&(_, d_is_then)| !d_is_then)
                && !ends_in_terminator(out.code.last())
            {
                for &(slot, d_is_then) in &ifjoin_plan {
                    if !d_is_then {
                        out.push(Lir::LocalGet(slot));
                        out.push(Lir::CallImport(OP_DROP));
                    }
                }
            }
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
                        false,
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
            // INC2 slice-1 (divergent-consume loop-join, C-arm shell reclaim): when this tail MatchList's
            // scrutinee is a loop-param that is DIVERGENT (reused-whole in one arm + advanced-to-tail in
            // another → the RestFrom preservation-dup fires, orphaning the old shell rc1 per iteration),
            // thread its slot as `selfloop_scrut_slot` + set `list_scrut_divergent` so `emit_loop_iteration`
            // reclaims the orphan (bypassing its `is_restfrom_consume` skip, which only holds for a
            // sole-consume RestFrom). Gated + fenced by `list_selfloop_scrut_divergent_reclaim_ok`.
            let list_selfloop_scrut = if let Some(t) = tl
                && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
                && let Some(&slot) = slots.get(&binder)
                && t.param_slots.contains(&slot)
                && let Some(fb) = out.fn_body
                && list_selfloop_scrut_divergent_reclaim_ok(db, fb, id, scrutinee, binder, &arms)
            {
                Some(slot)
            } else {
                None
            };
            let arm_tp = if let Some(slot) = list_selfloop_scrut {
                TailPos::Tail(tl.map(|t| TailLoop {
                    selfloop_scrut_slot: Some(slot),
                    list_scrut_divergent: true,
                    ..t
                }))
            } else {
                TailPos::Tail(tl)
            };
            emit_list_arms_tailable(
                db, &arms, len_slot, block_ty, result_it, &arm_slots, arm_base, high, scratch_ty,
                layout, out, arm_tp,
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
            // #9271-followup: PRE-mark a self-tail-loop `String.at` scrutinee the lowering would orphan.
            // CALLER-OWNED gate (AXIS-A, rp2/6307): the per-read `str_slot` drop is sound+balanced only when
            // the scanned param is CALLEE-OWNED (a guest-built rope handed off, e.g. rp2's cnt `s`). For a
            // BOUNDARY-OWNED entry-arg (the host built the cell; the def only BORROWS it — 13-strings:6307's
            // `walk s`, reached here now that the `StrAt` borrow arm makes `param_only_borrowed_or_backedge`
            // true) the per-read drop leaves a +1 residual (the boundary ref the caller reclaims), so DECLINE
            // the marking and leave it to its owner — exactly the `looped_invariant_param_caller_owned` fence
            // `looped_owned_param_drops` uses at the loop-EXIT drop. Keeps rp2/600 (owned) marked, reverts
            // 6307 (boundary entry-arg) to its unmarked/declined baseline. Leak-over-UAF: a wrong exclude only
            // leaks.
            if let Core::StrAt { string, .. } = core_of(db, scrutinee)
                && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, string)
                && let Some(t) = tl
                && sum_cont_has_member_tail_call(db, &root, t.members)
                && let Some(fb) = out.fn_body
                && let Some(self_d) = out.self_def
                && param_only_borrowed_or_backedge(db, fb, binder, t.members, t.param_slots, slots)
                && looped_invariant_param_caller_owned(db, self_d, fb, binder)
            {
                out.strat_selfloop_scrut_drop.insert(scrutinee);
            }
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
            let scrut_binder = match core_of(db, scrutinee) {
                Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
                _ => None,
            };
            // INC1: admit a COMPOUND-payload owned-param shell (a fresh-rebuilt `(Node …)`/`#tuple(…)` arm —
            // the BST del-min/insert 29/13/3 leak class) via the SECOND disjunct: membership in
            // `nontail_compound_reclaim_binders` (populated in LOCKSTEP by the dup-pass `is_nontail_spine_param`,
            // so every consumed shell child is dup'd before this single-op_drop param-slot reclaim — op_drop
            // cascades, no bespoke recursive drop) AND `nontail_param_compound_extra_ok` (the interior-view
            // alias-out exclusion — no arm reads a shell child through Map.lookup/List.at/… whose result
            // aliases the shell). The SCALAR path (`nontail_param_payload_ok`, which copies out) is unchanged.
            // BISECT: compound disjunct temporarily OFF — scalar path only (nontail_param_payload_ok). Peano
            // is fixed via this path (its `+1` arm builds no compound). Isolates whether the compound
            // reconstructing-arm path (nontail_param_compound_extra_ok) is the guarded-all culprit.
            // SINGLE-SOURCE (v-core-opt extract-share): the payload-kind decision is the ONE predicate
            // `nontail_param_reclaim_kind` (occurrence-level), shared with the dup-pass + def_inc1_reclaims_param
            // so caller-drop XOR reclaim can never drift. Scalar path only while the compound arm is bisected
            // OFF — behavior-identical to the prior inline `nontail_param_payload_ok`.
            // SINGLE-SOURCE (v-core-opt extract-share): ONE predicate call decides the kind; the emit then
            // gates on the matching binder-set the dup-pass populated in LOCKSTEP — Scalar on the raw selection
            // `nontail_match_reclaim_binders` (:1226), Compound on `nontail_compound_reclaim_binders`
            // (`collect_nontail_compound_reclaim_binders` → same `is_nontail_spine_param`, so its consumed shell
            // children were dup'd → the op_drop cascade nets). INC1 increment-2: Compound now LIVE.
            let param_reclaim = stashed_slot.is_none()
                && match nontail_param_reclaim_kind(
                    db,
                    out.fn_body
                        .expect("fn_body set in select_function_of before emit"),
                    scrutinee,
                    &scrut_ty,
                    never_diverges,
                    &root,
                ) {
                    Some(ReclaimKind::Scalar) => {
                        scrut_binder.is_some_and(|b| out.nontail_match_reclaim_binders.contains(&b))
                    }
                    Some(ReclaimKind::Compound) => scrut_binder
                        .is_some_and(|b| out.nontail_compound_reclaim_binders.contains(&b)),
                    None => false,
                };
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
                out.fn_body,
            );
            let scalar_shell_ok = sum_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            // LOOPING all-scalar owned shell (v-memory-safety, @1996 per-iteration husk): an owned,
            // freshly-stashed, ALL-SCALAR-payload `Some` shell whose match LOOPS (a member tail-call arm —
            // `(match (Bytes.at b i) ((Some v) (self b (+ i 1) (+ acc v))) ((None) …))`) leaks ONE shell per
            // iteration, because the general `sum_shell_reclaim_ok` reclaim was gated `!arms_tail_call` (its
            // post-match drop can't run on the looping path). Reclaim it EXACTLY as the owned-single-VIEW path
            // (`matchsum_view_shell_reclaim_ok`) does: split across BOTH exits — the fall-through arms via the
            // post-match drop (`reclaim_shell` below), the looping arms via a pre-back-edge deep-drop threaded
            // through `scrut_shell_reclaim` into `emit_loop_iteration`. SOUND: `sum_shell_reclaim_ok`'s
            // ALL-SCALAR-payload floor proves no shell-payload HANDLE aliases out / threads into a back-edge
            // (the extracted scalar `v` is COPIED into `acc + v`), so freeing the shell before the `br` is
            // safe — the same alias-safety the non-tail arm relies on.
            let looped_scalar_shell = arms_tail_call && scalar_shell_ok;
            // PROJ-of-fresh-owned-aggregate local reclaim: a `(. <fresh-owned-aggregate> i)` scrutinee's
            // extracted `Some` shell leaks because `Core::Proj` is not globally `Owned`
            // (`matchsum_proj_owned_aggregate_reclaim_ok`). Non-looping only (the borrow-clean path has no
            // back-edge threading); the post-match fall-through drop covers it (02:7314 fresh-tuple proj).
            let proj_reclaim = matchsum_proj_owned_aggregate_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            // EXPECT-of-owned-Some local reclaim: an `(Option.expect <owned-Some> …)` scrutinee's extracted
            // payload shell leaks because `Core::SumExpect` is not globally `Owned`
            // (`matchsum_expect_owned_reclaim_ok`). Non-looping, borrow-clean (05:2117 disc-only nc match).
            let expect_reclaim = matchsum_expect_owned_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            // MATCH-EXTRACTION owned-locally COMPOUND-SUM shell reclaim (02:6042 residual, increment-1): the
            // scrutinee is itself a `Core::MatchSum` (e.g. top's escaping-proj match INLINED into main) whose
            // escaping child #9391 already dup'd to escape OWNED, so the outer match extracts a scalar and the
            // now-dead COMPOUND-payload sum shell is a dead owned temporary. `sum_shell_reclaim_ok`'s
            // all-scalar-payload TYPE floor (the sread-UAF restriction) over-approximates and declines it even
            // though this value's arm is borrow-clean + scalar-result; `matchsum_matchextract_owned_reclaim_ok`
            // is the 4th owned-LOCALLY twin (after proj/expect/view) — it proves owned-local via the inner
            // match's escaping-proj dup + the G4/G5 per-arm alias fences (the PRECISE guard the coarse type
            // floor stood in for). Non-looping (post-match fall-through drop of the stashed shell slot below);
            // the extracted payload is scalar-copied so no child-dup is owed (the deep-drop nets the shell alone).
            let matchextract_reclaim = matchsum_matchextract_owned_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            // MATERIALIZED runtime-TUPLE scrutinee shell reclaim (19-sets:0204, Map.take desugar): a fresh
            // `Core::Tuple` with runtime elements is arr-alloc'd by the decision-tree builder and its shell +
            // moved-in component husks LEAK (no tuple analog of `sum_shell_reclaim` existed). Deep-drop the
            // materialized tuple after the arms — one drop cascades to node#6 (the tuple array) + node#7 (its
            // moved-in Option husk). STRICT borrow-clean floor (zero consuming sites), gated on `Core::Tuple`
            // so it goes INERT under a future in-place-destructure SROA (v-core-opt owns that separately).
            let tuple_reclaim = matchsum_tuple_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            let reclaim_shell = view_reclaim
                || looped_scalar_shell
                || (!arms_tail_call
                    && (scalar_shell_ok
                        || param_reclaim
                        || proj_reclaim
                        || expect_reclaim
                        || matchextract_reclaim
                        || tuple_reclaim));
            // Thread the owned-view shell slot into the arms' loop context so a member tail-call in an arm
            // (`find-at`'s recursive branch) drops the dead shell before its back-edge `br`. Only when the
            // match actually loops (`arms_tail_call`) and the view reclaim holds; else the arms' `tl` is
            // unchanged (the common case never touches this).
            // INC1 pt3 SELF-LOOP-TAIL shell reclaim: when this tail match LOOPS (a member tail-call arm) over
            // a self-recursive body's COMPOUND loop-param scrutinee, thread that param's slot so
            // `emit_loop_iteration` deep-`op_drop`s the dead node shell per iteration (fixes the inorder
            // self-tail-traversal spine leak — the self-loop-tail lever). G1 (the scrutinee IS a loop-param:
            // its binder slot ∈ `tl.param_slots`) + G2/G4/G5/G6a (`selfloop_scrut_shell_reclaim_ok`) gate here;
            // G3 (the arg carried back into that slot is a CHILD-PROJ, node not carried whole) is checked in
            // `emit_loop_iteration` where the per-arm tail-call args live.
            let selfloop_scrut_slot = if arms_tail_call
                && let Some(t) = tl
                && let Some(b) = scrut_binder
                && let Some(&slot) = slots.get(&b)
                && t.param_slots.contains(&slot)
                && let Some(fb) = out.fn_body
                && selfloop_scrut_shell_reclaim_ok(db, fb, scrutinee, &root)
            {
                Some(slot)
            } else {
                None
            };
            // The reclaimable shell's slot (the stashed temp, OR the non-tail-spine PARAM scrutinee's own
            // slot) — resolved ONCE here for BOTH the cross-fn-return_call drop (threaded into the arms) and
            // the post-match fall-through drop below (they fire on DISJOINT exit paths).
            let reclaim_slot: Option<u32> =
                stashed_slot
                    .map(|s| s.0)
                    .or_else(|| match core_of(db, scrutinee) {
                        Core::Param { binder } | Core::LocalRef { binder } => {
                            arms_slots.get(&binder).copied()
                        }
                        _ => None,
                    });
            // CROSS-FN return_call shell drop (fp-residual): when the shell is reclaimable AND no arm's
            // tail-call consumes a payload-of-scrutinee (`!sum_cont_payload_consumed_in_tail_call` — the UAF
            // fence, checked HERE where the scrutinee is in scope), thread its slot so the `Core::Call` arm
            // drops the dead shell before a cross-fn `return_call` (which `return`s PAST the post-match drop
            // that only the value-returning arms reach). Fence-at-MatchSum → unconditional drop at the Call
            // arm. A tail-call CONSUMING the payload (a live handle into the shell) fails the fence → no drop
            // (leak-safe, never a UAF). The looping/member-tail-call path is handled by
            // scrut_shell_reclaim/selfloop_scrut_slot, not this (member_which loop-iterates before the drop).
            // ONLY when the match ACTUALLY has a tail-position Call arm (a `return_call`). Without this, a
            // VALUE-ONLY reclaim_shell match (no tail call) would still set the carrier → for a non-looped fn
            // it constructs the minimal empty-members TailLoop below, flipping arm_tp Tail(None)→Tail(Some(..))
            // and perturbing every self-loop gate keyed on a bare `Tail(Some(_))` (v-wasm-opt #7963 hardened
            // the 5 dispatch code-size gates to is_self_loop_tail; gating at the SOURCE here also spares the
            // reclaim-safety `list_shell_reclaim_slot` gate v-wasm-opt flagged — no pointless carrier reaches
            // it). `sum_cont_tail_callees` non-empty = the match has a tail Call arm (member → loop-iterate,
            // handled by scrut_shell_reclaim/selfloop; non-member → the cross-fn return_call this drop targets).
            let has_tail_call = {
                let mut callees = Vec::new();
                sum_cont_tail_callees(db, &root, &mut callees);
                !callees.is_empty()
            };
            // Fence: block the drop when a tail-call consumes a payload-of-scrutinee (a live handle escapes →
            // deep-dropping the shell would free it → UAF). RELAXED for the borrowing-Call view reclaim
            // (#9218-followup): ALSO drop when the consumed payload flows ONLY to borrow-only recursive readers
            // (`sum_cont_payload_tail_all_borrow_reader`) AND the scrutinee is OWNED — the OWNED gate is the
            // lockstep guarantee that `collect_shell_reclaim_child_dups` emitted the child-dup (shell deep-drop
            // cascades the shell's own ref, the child-dup keeps the callee's ref alive → balanced; without it,
            // drop-without-dup = double-free). A genuine consumer → not all-borrow → fence holds (leak).
            let returncall_shell_drop = if reclaim_shell
                && has_tail_call
                && (!sum_cont_payload_consumed_in_tail_call(db, &root, scrutinee)
                    || (matches!(
                        heap_operand_ownership(db, scrutinee),
                        Ok(HandleOwnership::Owned)
                    ) && sum_cont_payload_tail_all_borrow_reader(db, &root, scrutinee)))
            {
                reclaim_slot
            } else {
                None
            };
            let base_tl: Option<TailLoop> =
                if (view_reclaim || looped_scalar_shell) && arms_tail_call {
                    let shell_slot = stashed_slot
                        .expect("view/looped-scalar shell reclaim implies a stashed I32 slot")
                        .0;
                    tl.map(|t| TailLoop {
                        scrut_shell_reclaim: Some(shell_slot),
                        ..t
                    })
                } else if let Some(slot) = selfloop_scrut_slot {
                    tl.map(|t| TailLoop {
                        selfloop_scrut_slot: Some(slot),
                        ..t
                    })
                } else {
                    tl
                };
            // Overlay the cross-fn return_call shell drop. When there is no enclosing loop context
            // (`base_tl` is None — a NON-looped fn like the fallible-parser `pf` whose only tail call is a
            // cross-fn `return_call` to a peer), construct a MINIMAL TailLoop (empty members ⇒ `member_which`
            // never loop-iterates, so the arm still `return_call`s) purely to carry the drop slot.
            let arm_tl: Option<TailLoop> = match (base_tl, returncall_shell_drop) {
                (Some(t), rc) => Some(TailLoop {
                    returncall_shell_drop: rc,
                    ..t
                }),
                (None, Some(s)) => Some(TailLoop {
                    members: &[],
                    param_slots: &[],
                    which: None,
                    depth: 0,
                    scrut_shell_reclaim: None,
                    selfloop_scrut_slot: None,
                    list_scrut_divergent: false,
                    returncall_shell_drop: Some(s),
                }),
                (None, None) => None,
            };
            let arm_tp = TailPos::Tail(arm_tl);
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
                // double-free / no leak. (Resolved as `reclaim_slot` above — shared with the cross-fn
                // return_call drop, which fires on the DISJOINT return_call exit paths.)
                let Some(slot) = reclaim_slot else {
                    return Err(Reject::decline(
                        "shell reclaim: no stashed slot or param binder slot for the scrutinee",
                    ));
                };
                out.push(Lir::LocalGet(slot)); // [result, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the owned sum shell)
            } else if let Some(slot) = selfloop_scrut_slot
                && !never_diverges
            {
                // SELF-LOOP-TAIL EXIT-ARM reclaim, dup_sites-GATED (v-memory-safety, tree 05:HEIGHT-BALANCE
                // mode2 +10). `selfloop_scrut_slot` reclaims the dead loop-param scrutinee shell PER ITERATION
                // at the back-edge (`emit_loop_iteration`'s save+deep-`op_drop`), covering only the LOOPING
                // arms. A NON-LOOPING exit arm (a base/`false` return that does NOT `br` the loop —
                // `balanced`'s `(> diff 1) → false`) falls through HERE with the dead loop-param STILL in its
                // slot and NEVER dropped (`reclaim_shell` is FALSE because `arms_tail_call`), so the whole
                // owned scrutinee subtree LEAKS on that exit (mode2 hits the root's false arm immediately →
                // the entire tree leaks = +10; a balanced tree never takes it → 0). Deep-`op_drop` the
                // loop-param on this fall-through — the looping arms `br` PAST it (no double-free with the
                // back-edge drop), so it fires only on the value-returning exit arms.
                //
                // GATE (the P0 double-free fence — a bare exit-drop double-frees `run`'s mutual-recursion Seq
                // arm, which MOVE-consumes its scrutinee children `a`/`b`): the deep-drop cascades into every
                // scrutinee child, so it is sound ONLY if every scrutinee-child CONSUMED on an exit path is
                // dup-BACKED (the shell kept its own ref → the cascade nets). A child MOVED out (its sole ref
                // handed to a consumer) is freed by that consumer → the cascade would double-free it.
                // `collect_consuming_payload_sites_cont` enumerates exactly the consuming compound scrutinee-
                // child extraction sites (borrows excluded, nested MatchList/MatchSum descended); a site is
                // dup-backed IFF it is in the FINAL `dup_sites` (which reflects WHICH shell-reclaim dup branch
                // ran: tree's nontail branch dup-backs `l`/`r` — re-extracted from the still-live parent,
                // multi-consumed by inlined `height`+`balanced` → present → FIRE; `run`'s selfloop_scrut branch
                // dups only the back-edge carried child, NOT the non-looping-exit Seq consumes → `a`/`b` ABSENT
                // → SKIP). ANY candidate absent ⟹ a move ⟹ SKIP (leak-safe: residual leak, never a UAF). The
                // whole-`root` candidate set is the conservative cut (a per-exit-cont set would forfeit less).
                let mut exit_move_candidates: HashSet<StructId> = HashSet::new();
                collect_consuming_payload_sites_cont(
                    db,
                    &root,
                    scrutinee,
                    &mut exit_move_candidates,
                );
                // NON-EMPTY guard (the whole-carry double-free fence): the scrutinee must be genuinely
                // DESTRUCTURED — at least one consuming child-projection (`balanced`'s l/r) — so its shell is a
                // dead husk this drop reclaims. An EMPTY set means the scrutinee is carried WHOLE/identity on
                // the back-edge (a `(walk (- n 1) w)` threading `w` unchanged — 20:BigInt-probe) or only
                // borrowed: a whole-carried loop-param is NOT a dead husk (it is threaded live + reclaimed by
                // the existing identity path on exit), so an exit-drop here DOUBLE-FREES it (guarded-all
                // `unreachable`). `all()` over an empty set is vacuously true, so WITHOUT this guard the gate
                // fires on exactly that case. Requiring non-empty forfeits a borrow-only-scrutinee leak (safe,
                // conservative) but blocks the whole-carry double-free.
                let all_dup_backed = !exit_move_candidates.is_empty()
                    && exit_move_candidates
                        .iter()
                        .all(|site| out.dup_sites.contains(site));
                if all_dup_backed {
                    out.push(Lir::LocalGet(slot)); // [result, shell]
                    out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the dead loop-param on the exit arm)
                }
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

/// Count the IDENTITY SELF-FORWARD consumes of param `p` in `id`: a `Core::Call` to `self_callee` (the def
/// being analyzed) whose arg at EXACTLY `param_index` is a bare ref to `p` — i.e. `(self … p …)` threading the
/// param straight back into its own slot on the recursive frame. This is the subset of `count_param_consumes`
/// hits that are SOUND to forgive for the fn-exit reclaim: the recursive call site dups the param (owned
/// transfer) and the inner frame reclaims it at its own epilogue by the same induction, so dup + inner-drop
/// balances. It counts ONLY the arg at `param_index` (a self-call also passing `p` at ANOTHER index is a real
/// escape into that other param and stays counted by `count_param_consumes` — the `total == self_forwards`
/// equality then fails). Recurses all children so a nested self-call is found. Mirrors the shape of
/// [`count_param_consumes`]'s `Core::Call` arm.
fn count_param_self_forward_consumes(
    db: &mut Db,
    id: StructId,
    p: StructId,
    self_callee: usize,
    param_index: usize,
    seen: &mut HashSet<StructId>,
    count: &mut usize,
) {
    if !seen.insert(id) {
        return;
    }
    if let Core::Call { callee, args } = core_of(db, id)
        && callee == self_callee
        && matches!(args.get(param_index), Some(&a) if is_ref_to(db, a, p))
    {
        *count += 1;
    }
    for c in core_child_ids(db, id) {
        count_param_self_forward_consumes(db, c, p, self_callee, param_index, seen, count);
    }
}

/// Whether `arg` contains a WHOLE-binder retain-dup site for `binder` — a bare `LocalRef`/`Param(binder)`
/// node that `mark_binder_dups` marked in `dups` (the `consuming && live_after` whole-binder dup at
/// reclaim.rs's `Core::LocalRef` arm, NOT a nested `Proj`/`SumPayload` child-dup). Used by
/// [`emit_loop_iteration`]'s `drop_old_borrowed` to detect that the back-edge CONSUME of a varying loop
/// param was DUP'd because a sibling back-edge arg co-BORROWS it (the O(n) borrow-thread-accumulator shape):
/// the dup makes the consuming op (`List.push acc i`) take the COPY path (rc>1), so the OLD accumulator cell
/// SURVIVES the consume and — being dead after the co-borrow read — must be reclaimed once per iteration.
/// Gating the extra drop on the dup HAVING FIRED is the double-free guard: without the co-borrow the consume
/// FBIP-reuses the rc1 cell in place (no surviving old value), and dropping it would free the new accumulator.
fn arg_has_whole_binder_dup(
    db: &mut Db,
    arg: StructId,
    binder: StructId,
    dups: &HashSet<StructId>,
) -> bool {
    if matches!(core_of(db, arg), Core::LocalRef { binder: b } | Core::Param { binder: b } if b == binder)
        && dups.contains(&arg)
    {
        return true;
    }
    core_child_ids(db, arg)
        .into_iter()
        .any(|c| arg_has_whole_binder_dup(db, c, binder, dups))
}

/// (B) PATH-1 same-arg intra-borrow independence (v-core-opt-endorsed avenue-A same-arg relax): `arg` consumes
/// `binder` as the whole list operand of a `List.push`/`List.prepend` AND co-borrows it inside the pushed
/// element, where that co-borrow is INDEPENDENT (see `elem_coborrow_independent`). The intra-arg twin of the
/// sibling co-borrow the #9181 narrowing requires: a dup-independent `List.at` read in the element forces a
/// whole-binder retain-dup at the consume, so the old spine survives + is dead-after → safe for avenue-A's
/// post-store drop. The Catalan self-embed `(List.push c (conv c ..))` embeds `c` un-dup'd → declines
/// (leak-over-UAF). Only `List.push`/`List.prepend` with `binder` as the whole list operand.
fn same_arg_intra_borrow_independent(db: &mut Db, arg: StructId, binder: StructId) -> bool {
    let elem = match core_of(db, arg) {
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            if !matches!(core_of(db, list), Core::LocalRef { binder: b } | Core::Param { binder: b } if b == binder)
            {
                return false;
            }
            elem
        }
        _ => return false,
    };
    // An intra-arg co-borrow of `binder` in the element (survivor-forcing read) that is INDEPENDENT.
    occurs_in(db, elem, binder) && elem_coborrow_independent(db, elem, binder)
}

/// (B) PATH-1 explicit independence discriminator (v-core-opt classifier-call option #1). ADMIT iff EVERY
/// occurrence of `binder` in `elem` is EITHER (a) the CONTAINER operand of a dup-independent borrow prim —
/// `List.at`/`Map.lookup`/`Bytes.at`/`String.at` ONLY (they dup/scalar-copy the element, so `binder`'s spine
/// is not embedded) — OR (b) an explicit dup site (elem-scoped). A whole-carry, a non-dup-independent
/// consuming/embedding op (conv-style Catalan self-embed), or the container of `String.slice`/`Bytes.slice`
/// (ALIASING VIEWS — the load-bearing carve-out: a slice-view of `binder` aliases its cells → dropping the old
/// spine = UAF) → DECLINE. Conjunctive; STRICTER than `binding_escapes` (which admits slice). Sound iff no live
/// reference to `binder`'s cells is embedded in the surviving pushed value.
fn elem_coborrow_independent(db: &mut Db, elem: StructId, binder: StructId) -> bool {
    let mut dup_sites: HashSet<StructId> = HashSet::new();
    reclaim::collect_dup_sites(db, elem, &[binder], &mut dup_sites);
    elem_indep_walk(db, elem, binder, &dup_sites, false)
}

/// Returns true iff subtree `id` contains NO non-independent occurrence of `binder`. `a_container` = this
/// position IS the container operand of a dup-independent borrow prim (a bare `binder` here is admitted (a)).
fn elem_indep_walk(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    dups: &HashSet<StructId>,
    a_container: bool,
) -> bool {
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } if b == binder => {
            a_container || dups.contains(&id) // (a) borrow-prim container OR (b) dup-backed
        }
        // set (a): container operand → a_container=true; index/key is a scalar position.
        Core::ListAt { list, index, .. } => {
            elem_indep_walk(db, list, binder, dups, true)
                && elem_indep_walk(db, index, binder, dups, false)
        }
        Core::MapLookup { map, key, .. } => {
            elem_indep_walk(db, map, binder, dups, true)
                && elem_indep_walk(db, key, binder, dups, false)
        }
        Core::BytesAt { bytes, index, .. } => {
            elem_indep_walk(db, bytes, binder, dups, true)
                && elem_indep_walk(db, index, binder, dups, false)
        }
        Core::StrAt { string, index, .. } => {
            elem_indep_walk(db, string, binder, dups, true)
                && elem_indep_walk(db, index, binder, dups, false)
        }
        // Everything else (incl. StrSlice/BytesSlice aliasing views, Call, ctors) is NOT an a-container:
        // a bare `binder` there must be dup-backed (b) else the elem is not independent.
        _ => core_child_ids(db, id)
            .into_iter()
            .all(|c| elem_indep_walk(db, c, binder, dups, false)),
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
    // The HEAP loop-param binders (unwrapping each `(: binder ty)` form) — the set a cross-param move
    // (`rebind_is_cross_param_move`, the multi-accumulator permutation reclaim) can rebind a slot to.
    let mut heap_param_binders: Vec<StructId> = Vec::new();
    for &mp in &member_params {
        let b = db
            .ast
            .as_form(mp, ":")
            .and_then(|t| t.first().copied())
            .unwrap_or(mp);
        if is_heap_type(&type_of(db, b)) {
            heap_param_binders.push(b);
        }
    }
    // Snapshot the whole-body dup set so the per-arg gate can detect a back-edge CONSUME that was DUP'd for a
    // sibling co-borrow (the O(n) borrow-thread accumulator drop below) without holding an `out` borrow across
    // the `db`-mut closure (mirrors the `dup_sites.clone()` at the post-body loop's arm-drop reconstruction).
    let dup_snapshot = out.dup_sites.clone();
    // THREADED-PREV drop (v-core-opt-ruled, inc 03:522 leak-2): the single member's own body, the escape-query
    // root for the dead-after check below (a loop-carried param that is only compare-borrowed then REPLACED).
    let self_body: Option<StructId> = if single_member {
        db.defs.get(tl.members[0]).and_then(|d| d.body)
    } else {
        None
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
            // DUP-AWARE escape fence (v-core-opt-signed-off, extends this from a plain borrow-only check to
            // the surplus-slot-ref condition): the param is DEAD-after-rebind iff no rebind arg CONSUMES its
            // slot ref un-dup'd. `binding_escapes_fresh_dup_aware` is FALSE iff every consuming occurrence of
            // `binder` dup'd a fresh reference — so `binder`'s OWN slot ref is a dead owned surplus AND the new
            // value holds only independent dup'd refs, hence dropping the slot ref frees only the surplus (no
            // UAF). This subsumes the old plain `!binding_escapes` (a whole-operand consume that MOVES the slot
            // ref stays escaping ⇒ declines) AND admits the slice/concat-RETENTION shape (10-bytes:3049: a
            // Bytes param rebound to a fresh `Bytes.concat(Bytes.slice b ..)` of ITSELF — the slices `op_dup`
            // the parent, so every self-use is dup-backed ⇒ dup-aware escape false ⇒ the surplus slot ref is
            // reclaimed per-back-edge; the successor's slice-refs keep the buffer live). Self-gating: the
            // empty-concat fast-path (returns a b-retaining operand WHOLE, un-dup'd) ⇒ escapes ⇒ declines.
            let borrow_not_consumed = !args
                .iter()
                .any(|&a| reclaim::binding_escapes_fresh_dup_aware(db, a, binder, false))
                && (rebind_produces_fresh(db, args[i])
                    || reclaim::rebind_is_cross_param_move(
                        db,
                        args[i],
                        binder,
                        &heap_param_binders,
                    ));
            // O(n) BORROW-THREAD ACCUMULATOR (v-memory-safety avenue-A): the param is CONSUMED by its own
            // rebind arg (args[i], e.g. `(List.push acc i)` → the new acc) AND a sibling back-edge arg
            // co-BORROWS it (`(+ sum (List.len acc))`), which forced a WHOLE-binder retain-dup at the consume
            // (`arg_has_whole_binder_dup` — the dup FIRED). The dup makes the consuming op take the COPY path
            // (rc>1), so the OLD accumulator cell SURVIVES the consume and is dead after the co-borrow read →
            // reclaim it once per iteration (the save+post-store rc-aware `op_drop` below lands AFTER every
            // arg's borrow). Gated on the dup having fired (double-free guard: without the co-borrow the
            // consume FBIP-reuses the rc1 cell → no survivor, and dropping it would free the new acc) AND the
            // param being only-borrowed (not whole-escaped) through the OTHER args (so the surviving old cell
            // has no live reader after this iteration). Distinct from `borrow_not_consumed` (which requires the
            // param be consumed by NO rebind arg); here args[i] IS the consuming rebind.
            // NARROWING (v-memory-safety, post-#9181 re-land): additionally require SOME SIBLING arg
            // j≠i to actually REFERENCE the binder (an `occurs_in` borrow occurrence, e.g. arg2
            // `(+ sum (List.len acc))` at 09-functions:635). This is the co-borrow that makes the surviving
            // OLD cell genuinely dead-after-read. WITHOUT a sibling reference the whole-binder dup came from
            // an INTRA-arg co-borrow embedded inside args[i] itself — e.g. Catalan
            // `(grow c m n) = (grow (List.push c (conv c 0 m 0)) (+ m 1) n)` where `conv` self-convolves the
            // SAME base list `c` inside the pushed element: there the old `c` cell is NOT dead (it is
            // aliased/shared through the pushed value) so the post-store drop double-frees (05-compound
            // Catalan trapped at n=4 under the un-narrowed #9172). c occurs ONLY in args[0] there → no
            // sibling reference → this narrowing declines the drop → the cell stays leaking (SAFE) rather than
            // double-freed. 09-functions:608/0030 keep their sibling co-borrow → stay reclaimed to 0.
            let dup_forced_old_survives =
                arg_has_whole_binder_dup(db, args[i], binder, &dup_snapshot)
                    && (0..args.len())
                        .all(|j| j == i || !binding_escapes(db, args[j], binder, false))
                    // The whole-binder dup fired for a co-borrow that makes the OLD cell dead-after: EITHER a
                    // SIBLING arg co-borrows binder (#9181 original), OR the co-borrow is INTRA-arg (same-arg)
                    // and PROVABLY INDEPENDENT (the #9181-narrowed same-arg case, relaxed for COIN/ROTATE per
                    // v-core-opt PATH-1 — Catalan's self-embed still escapes → still declines).
                    && ((0..args.len()).any(|j| j != i && occurs_in(db, args[j], binder))
                        || same_arg_intra_borrow_independent(db, args[i], binder));
            // THREADED-PREV reassigned-loop-param drop (v-core-opt-ruled, inc 03:522 leak-2; the #9522 AXIS-B
            // owner-drop family adapted). A loop param whose OLD value is DEAD-AFTER (only compare-borrowed
            // this iteration, then REPLACED by the back-edge store) leaks its old value each iteration when the
            // new value is an EXTRACTION rather than a fresh ctor (inc's `prev`, overwritten by the next key
            // `k` = arr-get, dup'd — so borrow_not_consumed declines since rebind_produces_fresh(k)=false). Two
            // load-bearing conditions, BOTH on the OLD value (v-core-opt's ruling):
            //   (1) DEAD-AFTER: binding_escapes_dup_aware(Binder, tail_borrowed=false, Some) == false. The
            //       reassign-store into the param's own slot is NOT an escape (the param is not re-passed as a
            //       tail-call arg — it is REPLACED), so a compare-only param returns false; a threaded /
            //       returned / CAPTURED param (ratwalk's threaded key, lf1's continuation capture) returns
            //       true -> DECLINE (leak-over-UAF). This is the primary UAF safety + the GATE-2 capture guard.
            //   (2) NEW VALUE is a proper OWNED handoff: a fresh producer (rebind_produces_fresh) OR a
            //       dup-backed transfer (the arg node in dup_sites). Then the reassignment leaves the slot
            //       owned, and the dup>=drop lockstep means dropping the old surplus ref frees ONLY it — even
            //       if old and new alias an interned cell, the new dup keeps rc>=1 (no alias check needed).
            // Distinct from borrow_not_consumed (needs a fresh ctor); this admits the dup-backed EXTRACTION.
            // guarded-all + the go/lf1/threaded/non-owned negative controls are the UAF net (double-free area).
            // GATE-1 (v-core-opt #9522, resolved): the concern was a SEPARATE reclaim of this param + this drop
            // = double-drop. The only pre-existing reclaim of a reassigned loop param is the fn-exit epilogue
            // (`looped_owned_param_drops`), which drops the FINAL (never-overwritten) value; this drop reclaims
            // each INTERMEDIATE (overwritten) value on the back-edge — the two PARTITION the values (final vs
            // intermediates), never the same cell, so they are complementary, NOT a double-drop (rc-trace: inc
            // LEAK SUMMARY none, zero double-free). So `def_looped_callee_reclaims_threaded_param` is NOT the
            // right guard here (it is true for inc precisely because of the complementary epilogue drop). The
            // UAF net for any genuinely-conflicting shape is guarded-all corpus-wide (a real double-drop traps).
            let drop_old_threaded_prev = self_body.is_some_and(|sb| {
                !binding_escapes_dup_aware(
                    db,
                    sb,
                    EscapeTarget::Binder(binder),
                    false,
                    Some(&dup_snapshot),
                    false,                ) && (rebind_produces_fresh(db, args[i]) || dup_snapshot.contains(&args[i]))
            });
            borrow_not_consumed || dup_forced_old_survives || drop_old_threaded_prev
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
            // ORDERING-ADMIT excuse (v-core-opt-ruled #4139 relaxation; ksd 0262 spine-over-dup): on this
            // self-tail-loop `is_restfrom_consume[i]` back-edge, PART-2 above orders the vec-drop LAST, so every
            // sibling head-consume is emitted (== runs, no post-emit scheduler) BEFORE the spine vec-drop. So a
            // grandchild head-consume DEAD-AFTER under `dup_sites=Some` (dup-backed / borrow-only) can't dangle
            // when the preservation dup is skipped. Excuse ONLY those; a move / non-dup-backed thread still fires
            // the K1 fence (leak-over-UAF). Single-member only (as `drop_old_borrowed`); scope = the member body.
            let ordering_excuse: Option<(StructId, &HashSet<StructId>)> = if tl.members.len() == 1 {
                db.defs
                    .get(tl.members[0])
                    .and_then(|d| d.body)
                    .map(|b| (b, &dup_snapshot))
            } else {
                None
            };
            let element_heapchild_consumed = args.iter().enumerate().any(|(j, &a)| {
                j != i && arm_consumes_binder_grandchild(db, a, binder, ordering_excuse)
            });
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
    // INC1 SELF-LOOP-TAIL shell reclaim (pt3, save half): SAVE the dead-after-iteration compound scrutinee
    // shell (a REASSIGNED loop-param slot, `tl.selfloop_scrut_slot`) into a fresh scratch BEFORE the back-edge
    // reassign overwrites it — a slot COPY (no rc change; the old shell handle stays owned in scratch). The
    // deep `op_drop` (post-store, below) then frees it; its cascade decrements each child's shell-ref, all of
    // which are already dup-backed (child carried into a loop-param, siblings consumed by dup'd non-tail
    // sub-calls — the G6 dup ⟺ cascade lockstep), so every child nets to its single surviving owner. Unlike
    // `scrut_shell_reclaim` (a stashed slot dropped as-is post-reassign), this MUST save first: the slot is
    // reassigned to the carried child, so a post-reassign drop of the slot would free the NEW value.
    let selfloop_scrut_scratch: Option<u32> = tl.selfloop_scrut_slot.and_then(|slot| {
        // G3 (checked here where the per-arm tail-call args live): the arg carried BACK into the scrutinee's
        // own slot must be a CHILD-PROJECTION of it (a `SumPayload`/`Proj` rooting at that same param slot) —
        // never the node carried WHOLE. An identity re-pass (`is_identity`) or a non-projection would alias the
        // live loop-param to the shell we free → UAF. Sound-conservative: a shape we can't prove is a
        // self-child-proj → skip the reclaim (leak, safe). This keeps the dead-shell invariant (G3): the shell
        // is dead once its children are extracted, and the carried child is one of those extractions.
        let i = tl.param_slots.iter().position(|&s| s == slot)?;
        // G7 (NOT DOUBLE-DROPPED): skip if this slot is ALREADY reclaimed/handled by another loop path —
        // else our op_drop is a SECOND free of the same shell → double-free (the 10 harden traps' §5-fold
        // class). `is_sumpayload_consume` = the §5 sum-spine reclaim (dup rest + drop old shell) already frees
        // it; `drop_old_borrowed` = the borrowed-accumulator drop; `is_restfrom_consume` = the RestFrom
        // vec-drop consumes it; `is_identity` = a whole re-pass (no consumption, and aliases the live slot).
        //
        // INC2 slice-1 EXCEPTION (`list_scrut_divergent`): the `is_restfrom_consume` skip assumes the RestFrom
        // `vec-drop` fully consumed the shell — TRUE only for a SOLE-consume RestFrom whose preservation-`dup`
        // was skip-gated. For a DIVERGENT MatchList scrutinee (also reused-WHOLE elsewhere), the dup FIRED, so
        // the `vec-drop` nets the dup (rc2→1) and the ORIGINAL rc1 shell is orphaned — it MUST be reclaimed
        // here (v-runtime P6: Sum shells #8/10/12/14). So do NOT short-circuit on `is_restfrom_consume` when
        // `list_scrut_divergent` (the `list_selfloop_scrut_divergent_reclaim_ok` gate proved the dup fired via
        // `count_param_consumes > 1`; the save+`op_drop` below is rc-aware, so it frees the orphan without
        // touching the fresh tail-slice or shared interior). The other three skips still apply.
        if is_identity[i]
            || is_sumpayload_consume[i]
            || drop_old_borrowed[i]
            || (is_restfrom_consume[i] && !tl.list_scrut_divergent)
        {
            return None;
        }
        // G3 (dead-after-iteration via child-projection): the arg carried BACK into the scrutinee's own slot
        // must be a CHILD-PROJECTION of it — a `SumPayload`/`Proj` chain rooting at that same loop-param slot,
        // never the node carried WHOLE (an identity/whole re-pass would alias the live loop-param to the shell
        // we free → UAF). The chain can be NESTED and let-bound: inorder's carried `r` is `Proj(p, 2)` where
        // `p` is `SumPayload(t)` and `t` is the loop-param — i.e. `r → Proj → p → SumPayload → t` through two
        // projection levels + `LocalRef` binders. `carried_roots_at_loop_param` follows the full chain. Sound-
        // conservative: a shape we can't prove roots at the loop-param → skip (leak, safe).
        if !carried_roots_at_loop_param(db, args[i], slot, slots, 0) {
            return None;
        }
        let sc = *high;
        *high = (*high).max(sc + 1);
        scratch_ty.insert(sc, ValType::I32);
        out.push(Lir::LocalGet(slot)); // [old-shell]
        out.push(Lir::LocalSet(sc)); // scratch = old-shell (slot copy, no rc change)
        Some(sc)
    });
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
    // INC1 SELF-LOOP-TAIL shell reclaim (pt3, drop half): free the saved dead compound scrutinee shell. LATE
    // by construction — this fires AFTER the arg-eval (which ran the non-tail sibling sub-calls that consume
    // their dup'd children) AND after the back-edge reassign captured the carried child, so no live handle
    // dangles. The deep `op_drop` cascade decrements each shell child's ref; every child survives via its
    // pre-existing escape dup (G6 dup ⟺ cascade lockstep) → the old node shell is freed, one per iteration
    // (the self-tail-traversal spine reclaimed as walked). NO dup added here (would over-retain).
    if let Some(sc) = selfloop_scrut_scratch {
        out.push(Lir::LocalGet(sc)); // [old-shell]
        out.push(Lir::CallImport(OP_DROP)); // free the dead scrutinee shell; cascade nets each dup-backed child → []
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

mod shell_reclaim;
use shell_reclaim::*;

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

/// INC2 slice-1 (v-mem W/C/D predicate — the C-arm shell-reclaim obligation for a DIVERGENT MatchLIST
/// self-loop-tail scrutinee). `list_shell_reclaim_slot` deliberately SKIPS a tail-loop (its owned-temporary
/// post-arms drop never reaches a `br`/`return_call` arm); that skip is correct for the SHELL-temporary path
/// but LEAKS the loop-param scrutinee's per-iteration old shell in the DIVERGENT case: the scrutinee is
/// reused-WHOLE in one tail arm (the W arm) AND advanced-to-TAIL `(.. r)` in another (the C arm). Because the
/// scrutinee is NOT sole-consumed (the whole-reuse is a second consume), v-wasm-opt's RestFrom
/// preservation-`dup` skip-gate does NOT skip — so the C arm emits `dup(scrut); vec-drop`, the `vec-drop`
/// nets the dup (rc2→1) but the ORIGINAL alloc-rc1 shell is orphaned when the loop-param slot is reassigned
/// to the tail → a per-iteration leak (v-runtime P6 rc-trace: Sum shells #8/10/12/14, one/step). This gates
/// threading the scrutinee's slot as `selfloop_scrut_slot` + `list_scrut_divergent` so `emit_loop_iteration`
/// saves + `op_drop`s the orphan (bypassing its `is_restfrom_consume` skip, which only holds for a
/// sole-consume RestFrom whose dup WAS skip-gated).
///
/// SOUND (F3-conservative, v-mem's asymmetric bar): under-admit = LEAK (safe); over-admit = UAF (forbidden).
///  • DIVERGENCE = `count_param_consumes(count_restfrom=true) > 1` — MORE than the single RestFrom consume ⟹
///    a whole-reuse also consumes it ⟹ the dup FIRED ⟹ the orphan rc1 is real. `count == 1` (sole RestFrom)
///    ⟹ the dup was SKIP-GATED ⟹ the `vec-drop` already freed the original ⟹ reclaiming would DOUBLE-FREE →
///    return false. This is the load-bearing gate.
///  • G6a OWNED-BY-FLOW: `body_is_self_recursive` (a tail self-call carries no caller-drop — the same INC1-1
///    frame pt3 uses); a non-self-recursive owned-param stays a LEAK, never a UAF.
///  • F1/G5 ESCAPE: no arm reads a shell CHILD out as a live handle (`arm_borrows_heap_subvalue`) — the same
///    fence `list_shell_reclaim_slot` uses; a whole-reuse carry of the scrutinee itself is NOT a shell-child
///    borrow (it re-passes the shell, kept live in the W arm; only the C arm reclaims the orphan).
///  • RE-MATCH: no inner match re-reads the scrutinee (`list_arms_rematch_scrutinee`).
fn list_selfloop_scrut_divergent_reclaim_ok(
    db: &mut Db,
    top_body: StructId,
    match_id: StructId,
    scrutinee: StructId,
    binder: StructId,
    arms: &[crate::core::ListArm],
) -> bool {
    if !matches!(type_of(db, scrutinee), Ty::List(_)) {
        return false;
    }
    if !body_is_self_recursive(db, top_body) {
        return false;
    }
    // DIVERGENCE (the dup FIRED): the scrutinee has a NON-RestFrom consume — the whole-reuse carry `(h a …)`
    // (a `Core::Call` arg ref, counted below) — so it is NOT sole-RestFrom-consumed, and v-wasm-opt's
    // preservation-dup skip-gate did NOT skip the RestFrom `dup`. `count_restfrom=false` counts consuming
    // uses EXCLUDING the RestFrom itself (which lives in the tail-binder's OWN value node, not the walked body
    // tree), so `count >= 1` ⟺ a whole-reuse consume exists ⟺ the dup fired ⟺ the orphan rc1 is real. `count
    // == 0` (sole RestFrom, dup skip-gated) ⟹ the vec-drop already freed the original ⟹ reclaiming would
    // DOUBLE-FREE → return false. This is the load-bearing F3-conservative gate.
    let mut count = 0usize;
    count_param_consumes(db, match_id, binder, &mut HashSet::new(), &mut count, false);
    if count == 0 {
        return false;
    }
    // F1/G5 shell-child-escape + RE-MATCH fences (shared with `list_shell_reclaim_slot`).
    if arms.iter().any(|a| {
        arm_borrows_heap_subvalue(db, a.body)
            || a.guard.is_some_and(|g| arm_borrows_heap_subvalue(db, g))
    }) {
        return false;
    }
    if list_arms_rematch_scrutinee(db, scrutinee, arms) {
        return false;
    }
    true
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
    // MATERIALIZED-SCRUTINEE stop (trx1, v-mem-safety): a CONTROL-FLOW value reached in BORROWED position —
    // the operand of a projection reading a sub-value OUT of it (`(. (match … ) k)`, or the inlined
    // MatchSum a `try` lowers to standing as an enclosing match's scrutinee) — is a value MATERIALIZED
    // ONCE into the match's scrutinee slot BEFORE the arm runs. Its DEFINITION's internal heap borrows
    // (its own arms' shell-CONSTRUCTION — e.g. the runtime-`?` failure arm re-wrapping the scrutinee's
    // Err payload into a fresh Err) are NOT arm escapes: they build the very shell we are matching, they
    // do not hand a live handle OUT of the enclosing arm. A GENUINE escape — a heap payload projected out
    // of this value and consumed — trips `is_heap_borrow` at the PROJECTION node ABOVE (in consuming
    // position), before this, so stopping here misses no escape (sread's `SumPayload(scrut):Map` fed to
    // `Map.lookup`, the HOL-kernel `term-eq (Comb x y)` payload threaded into a walk — both trip at the
    // projection, borrowed=false). Leak-safe: this only lets `arm_borrows` say false in MORE cases →
    // reclaim, never a double-free. Without it, the enclosing shell-reclaim of a COMPOUND-payload sum
    // (e.g. `Result Int64 String`) built by an inlined `try` was blocked → the fresh Ok/rebuilt-Err husk
    // LEAKED one cell per match (trx1 + the chapter-16 utf8 Option/Result family).
    if borrowed
        && matches!(
            core_of(db, id),
            Core::MatchSum { .. }
                | Core::Match { .. }
                | Core::MatchList { .. }
                | Core::If { .. }
                | Core::Let { .. }
        )
    {
        return false;
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
        // (C) A direct def CALL (v-memory-safety borrowing-Call view reclaim, co-design). An arg the callee
        // only BORROWS (`!def_consumes_param(callee, i)` — a borrow-only reader like Fletcher's `go` reading `s`
        // via `Bytes.at`) is read in place, so a shell-owned payload-VIEW (`SumPayload`) passed there does NOT
        // escape as a live handle → relax to `borrowed`, un-blocking the enclosing MatchSum shell-reclaim (the
        // borrow-Call twin of the CallClosure/Map.lookup/Set.contains relaxations above). A CONSUMED arg stays
        // CONSUMING. LOCKSTEP (consume-path): the view stays a consuming payload site so
        // `collect_shell_reclaim_child_dups` child-dups it, the shell is deep-dropped before the `return_call`
        // (relaxed `returncall_shell_drop`), and the callee consumes+drops its dup at base. A wrong borrow-relax
        // only leaves a reclaim un-blocked (leak), never frees a live handle (leak-over-UAF).
        Core::Call { callee, ref args } => {
            let args: Vec<StructId> = args.to_vec();
            args.iter().enumerate().any(|(i, &a)| {
                let borrowed = !def_consumes_param(db, callee, i);
                arm_borrows_heap_subvalue_seen(db, a, borrowed, seen)
            })
        }
        // Every other node kind (host calls, constructors, `if`/`let`, arithmetic, …) consumes / results — its
        // children carry no borrow relaxation. SAFE-BY-DEFAULT: an unhandled shape can only over-decline.
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| arm_borrows_heap_subvalue_seen(db, c, false, seen)),
    }
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
    // PEEL `Ty::Qty`/`Ty::Nominal` (via `peel_qty_ty`) before classifying the operand's slot: a Qty-erased
    // operand — `(Qty.of x u)` — types as `Ty::Qty { inner: Int }`, whose magnitude erases to the inner
    // int's machine slot (the op width `it` was itself peeled by `int_ty_of`). WITHOUT the peel the whole
    // block was skipped for a quantity operand (`Ty::Qty` is not `Ty::Int(_)`), so a narrow-inner magnitude
    // fed a WIDER Qty op UN-widened — an i32 beside the op's i64 → invalid wasm (v-cdz-smith / v-core-opt
    // `(+ (Qty.of ((fn ..) ..) u) (Qty.of v0:Int8 u))`, "expected i64, found i32").
    if let Ty::Int(operand_it) = peel_qty_ty(type_of(db, id)) {
        let op_slot = m_slot(it);
        let operand_slot = m_slot(operand_it);
        if operand_slot == ValType::I64 && op_slot == ValType::I32 {
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
        } else if operand_slot == ValType::I32 && op_slot == ValType::I64 {
            // NARROW operand into a WIDER op: sign/zero-extend i32 → i64. This is the reverse of the wrap
            // above and the direction `operand_src` already DEFERS here for ("a narrow operand feeding a
            // wider op … takes the copy path, where `emit_operand` widens it") — but the extend was never
            // implemented, so a width-mismatched runtime operand reached the i64 op as a bare i32 → invalid
            // wasm. SOUND for the same reason the wrap is: a genuine narrow-vs-wide *fixed*-Int disagreement
            // is a CDZ0203 fault caught before emit, so a mismatch reaching here is an ERASURE artifact (a
            // Qty/newtype magnitude whose inner width differs from the op's peeled width), whose i32 value
            // is exact and extends losslessly. Signedness is the OPERAND's own (a `UInt8` magnitude
            // zero-extends, an `Int8`/default-`Int` sign-extends), so the i64 value equals the narrow value.
            out.push(if operand_it.ground_signed() {
                Lir::I64ExtendI32S
            } else {
                Lir::I64ExtendI32U
            });
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
    // HEAP-RESULT exclusion (mirror the `Core::If` select gate, emit.rs:3462 `!is_heap_type(&result) ||
    // ty_is_enum_disc`): a `select` EVALUATES BOTH arms, so a heap-RESULT arm's freshly-allocated cell
    // (a `Tuple`/`ListNew`/`Record`/`MapNew`/`SetOf`/non-enum-disc `SumNew`) — or any dup'd heap handle —
    // on the NON-selected path is discarded off the stack with NO Perceus drop → a per-evaluation LEAK
    // (the 05:26545 33-variant `(W.V33 k)`-arm leak: v-runtime rc-trace — `select` builds the un-taken
    // `(W.V33 k)` sum shell then drops the immortal `V32` handle from `select`, orphaning the alloc). Only
    // an ENUM-DISC sum (an i32 discriminant, no alloc — admitted above) or a scalar/float result is
    // select-safe. `is_trap_free` conservatively covers TRAP-freedom but NOT allocation-freedom (a SumNew
    // of trap-free payloads is trap-free yet allocates), so it mis-admits an allocating arm; this heap gate
    // is what the branchless-select dispatch gates (dispatch.rs int/list/sum-disc terminal-pair selects)
    // rely on — unlike the `Core::If` gate, they have no separate `is_heap_type(result)` check. Excluding a
    // heap arm falls the match back to the structured `if` (evaluates only the taken arm) — value-identical,
    // no leak.
    let ty = crate::infer::type_of(db, id);
    if is_heap_type(&ty) && !ty_is_enum_disc(db, &ty) {
        return false;
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
    self_def: Option<usize>,
    caller_surplus_dup_sites: &HashSet<StructId>,
) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    // 5786 EXTERNAL-CALLER caller-drop (v-core-opt corrected admit; v-mem placement). A dup-backed,
    // borrow-classified, invariant heap param passed by an EXTERNAL caller is RETAINED by the caller and
    // dropped at its last use → the consumed-reused-invariant-base leak (#9434) clears. Admitted BEFORE gates
    // (1)/(5): the callee here is typically a plain local LOOP (excluded by (1) not-export/lifted AND (5)
    // looped), yet the caller still owns the surplus dup. SOUND iff ALL of:
    //   (A) `def_consumes_param(callee, i) == false` — the callee BORROWS the param, so it never drops it
    //       internally → the caller-drop is the ONLY drop (no double-free). This is the classifier's
    //       dup-backed-invariant-base → borrow reclassify (c861652be2); it is REACHED here (unlike the
    //       inert def_consumes_param call sites) because the admit queries it at the real caller-drop site.
    //   (B) `arg ∈ caller_surplus_dup_sites` — the RETAIN-ONLY dup set (multi-use surplus) MINUS the shell-
    //       reclaim child-dups. A dup minted for a genuine later caller use is the +1 that leaks without a
    //       drop. Excludes (i) a moved rc1 arg (not dup-backed → double-free) and (ii) a shell-reclaim
    //       child-dup already balanced by the shell drop (the fst-sum "reclaims with the pair shell" trap).
    //   (C) `mutual_loop_group(callee)` NON-EMPTY (callee is a self-recursive loop) AND `self_def ∉` it
    //       (EXTERNAL caller only). A self/mutual caller reclaims per-frame → caller-drop there double-frees
    //       (the BigInt/guide dup-suppress RED). NB `mutual_loop_group` does NOT distinguish a TCO'd tail loop
    //       from a non-tail self-recursive consumer (both are singleton self-loops) — (G) does that.
    //   (G) the callee must NOT drop this param at its LOOP EPILOGUE (`looped_owned_param_drops` ∌ its slot).
    //       THE load-bearing #9434-vs-sum-at split: sum-at's epilogue drops `xs` (slot 0 ∈ [0]) so a caller-
    //       drop is a SECOND drop → double-free; #9434's loop DECLINES base's exit-drop ([]) so the caller-drop
    //       is the only reclaim. Single-source-of-truth complementarity, exactly like gates (6)/(6b).
    //   (D) non-tail is STRUCTURAL: the tail `Core::Call` path passes `caller_drop_slots = None`, so this fn is
    //       only consulted from the non-tail emit.
    //   (E) `heap_operand_ownership(arg) == Borrowed` — the arg is live-after in the CALLER (a non-last-use
    //       let-binding/param), so its dup is a genuine caller-level SURPLUS. A LAST-USE (Owned) arg's dup is a
    //       shell-reclaim child-dup already reclaimed by the match/scope → a caller-drop there double-frees
    //       (the fst-sum "reclaims with the pair shell" trap). (E) is the load-bearing #9434-vs-fst-sum split.
    // Reuses (2) heap-param + (E) Borrowed-operand (both in the inner `if`/guard); DROPS (4) escape — see the NB
    // below for why (A) subsumes it. The original gates (1)-(6) below still apply to the NON-5786 export/lifted
    // caller-drop path this admit precedes.
    // (E) the operand must be BORROWED (NOT `Owned`) — the genuine "the caller keeps using arg AFTER this
    // call" signal (a live-after let-binding/param whose occurrence here is non-last-use). This is the
    // load-bearing distinguisher between #9434 and the fst-sum UAF: #9434's `base` is Borrowed (read again in
    // `List.len base` post-call) so its dup IS a surplus the caller must drop; fst-sum's `a` is a LAST-USE
    // destructured child (Owned transfer) whose dup is a SHELL-RECLAIM child-dup already reclaimed WITH the
    // pair shell — a caller-drop there double-frees. Requiring Borrowed admits the reuse-surplus, excludes the
    // last-use-move (the shell/scope path owns that reclaim). (The earlier Owned requirement was inverted —
    // it made the admit inert; dropping it entirely admitted the last-use shell-reclaim case → the trap.)
    if let Some(sd) = self_def
        && caller_surplus_dup_sites.contains(&arg)
        && matches!(
            heap_operand_ownership(db, arg),
            Ok(HandleOwnership::Borrowed)
        )
        && !def_consumes_param(db, callee, param_index)
        && {
            // (C): the callee must be a self-recursive loop (non-empty `mutual_loop_group`) with the caller
            // EXTERNAL to it — a self/mutual caller reclaims per-frame → caller-drop there double-frees.
            let g = mutual_loop_group(db, callee);
            !g.is_empty() && !g.contains(&sd)
        }
        && {
            // (G) YIELD to the callee's LOOP EPILOGUE (single-source-of-truth complementarity, exactly like
            // gates (6)/(6b)): if the callee ALREADY drops this param at loop exit (`looped_owned_param_drops`),
            // a caller-drop is a SECOND drop → double-free. This is THE #9434-vs-sum-at split: sum-at
            // epilogue-drops `xs` (slot ∈ the set) so the caller must NOT; #9434's loop DECLINES base's
            // exit-drop (empty set) so the caller-drop is the only reclaim. `mutual_loop_group` non-emptiness
            // does NOT distinguish them (both are singleton self-loops) — the epilogue-drop set does.
            let params = match layout.export_plan(callee) {
                Some(e) => e.params.clone(),
                None => crate::layout::def_params(db, callee),
            };
            // Slot = count of non-Unit params before `param_index` (matches `select_function_of`'s assignment).
            let mut slot = 0u32;
            let mut target: Option<u32> = None;
            for (idx, (_b, ty)) in params.iter().enumerate() {
                if matches!(ty.strip_nominal(), Ty::Unit) {
                    continue;
                }
                if idx == param_index {
                    target = Some(slot);
                    break;
                }
                slot += 1;
            }
            let epilogue = looped_owned_param_drops(db, body, &params, Some(callee));
            target.is_some_and(|s| !epilogue.contains(&s))
        }
    {
        let params = match layout.export_plan(callee) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, callee),
        };
        // NB: NOT gated on `!param_escapes_body` — that analysis is DUP-UNAWARE, so it flags a dup-backed
        // `List.push base` as a reuse-escape (false positive). Conjunct (A) `def_consumes_param == false` IS
        // the dup-aware borrow verdict (the classifier c861652be2 reclassifies the dup-backed invariant
        // base-consume → borrow) and already guarantees the callee neither consumes base nor returns it — so
        // base cannot escape via the callee result → the caller-drop is UAF-safe.
        if let Some((_param_binder, param_ty)) = params.get(param_index).cloned()
            && is_heap_type(&param_ty)
        {
            return true;
        }
    }
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
    // (6) INC1 approach B — YIELD to the callee's own non-tail-spine shell-reclaim; `def_inc1_reclaims_param`
    // is the single source of truth (caller-drop XOR reclaim, exactly complementary). node#3 (b): the blunt
    // `!body_is_capturing_lifted` conjunct is DROPPED (subsumed by def_inc1_reclaims_param — see its doc).
    if !layout.exports.iter().any(|e| e.body == body)
        && def_inc1_reclaims_param(db, body, param_binder)
    {
        return false; // (6)
    }
    // (6b) blx1 — non-looped twin of (6): if the callee reclaims THIS param via its fn-exit op_drop, the
    // caller must not also drop it. `def_nonlooped_reclaims_param` doesn't query us → no cycle.
    if def_nonlooped_reclaims_param(db, callee, param_index, layout) {
        return false; // (6b)
    }
    true
}

/// Whether `body`'s funcref is TAKEN as a first-class value — some `Core::Closure { code, .. }` in the
/// program lifts `body` (its `db.lifted[code].body == body`), so `body` is reachable via `call_indirect`
/// whose arg-ownership the DIRECT call-site index cannot see. blx1 caveat (a): conservatively EXCLUDE such
/// a callee from the non-looped self-drop (a `call_indirect` edge could pass a BORROWED arg → a callee
/// self-drop would free a value it does not own → UAF). A non-inlined named def IS hoisted into `db.lifted`
/// (e.g. `classify`), so `codes` is non-empty for it; a `Core::Closure` referencing that code means its
/// funcref escaped as a value. (A named def used first-class is ETA-wrapped — the wrapper is the lifted
/// body and the wrapper→callee edge is a DIRECT `Core::Call` the every-call-site-Owned gate sees, so that
/// path is covered there; this catches the direct-funcref-of-`body` case.) Conservative: any hit → true.
fn def_funcref_taken(db: &mut Db, body: StructId) -> bool {
    let codes: Vec<usize> = db
        .lifted
        .iter()
        .enumerate()
        .filter(|(_, l)| l.body == body)
        .map(|(i, _)| i)
        .collect();
    if codes.is_empty() {
        return false;
    }
    // LAZY MEMO (operator directive 2026-09-18: collapse this quadratic via demand-driven memoization, not
    // an eager precompute pass). "Is any code referenced by a `Core::Closure` anywhere?" is a WHOLE-PROGRAM
    // fact INDEPENDENT of `body`, so computing it once and caching it answers every per-def query in O(1) —
    // instead of re-walking the whole program per def (this fn is called per-def from 4 reclaim sites, so the
    // old walk was O(defs · program-nodes) = O(N²); it was 65% of a real self-host file compile). Built on
    // FIRST demand and cached on `db` — a compile with no funcref-taken query never builds it.
    ensure_referenced_closure_codes(db);
    let referenced = &db
        .referenced_closure_codes
        .as_ref()
        .expect("ensure_referenced_closure_codes populates the cache")
        .1;
    codes.iter().any(|c| referenced.contains(c))
}

/// Lazily build + cache [`Db::referenced_closure_codes`]: every `Core::Closure` `code` reachable from any
/// `db.defs` body. Rebuilds only when `lifted` grew since the cached snapshot (lift is append-only, so the
/// referenced-code set is fixed once lifting settles — by the emit phase it has). ONE whole-program walk,
/// `seen`-deduped across bodies (a DAG-shared subtree is visited once) — the SAME node coverage the old
/// per-call `def_funcref_taken` walk had, just collecting the complete code set instead of early-out per body.
fn ensure_referenced_closure_codes(db: &mut Db) {
    let version = db.lifted.len();
    if matches!(&db.referenced_closure_codes, Some((v, _)) if *v == version) {
        return;
    }
    #[cfg(test)]
    {
        db.referenced_closure_codes_builds += 1;
    }
    fn walk(db: &mut Db, id: StructId, set: &mut HashSet<usize>, seen: &mut HashSet<StructId>) {
        if !seen.insert(id) {
            return;
        }
        if let Core::Closure { code, .. } = core_of(db, id) {
            set.insert(code);
        }
        for c in crate::backend::wasm::select::reclaim::core_child_ids(db, id) {
            walk(db, c, set, seen);
        }
    }
    let bodies: Vec<StructId> = db.defs.iter().filter_map(|d| d.body).collect();
    let mut set = HashSet::new();
    let mut seen = HashSet::new();
    for b in bodies {
        walk(db, b, &mut set, &mut seen);
    }
    db.referenced_closure_codes = Some((version, set));
}

/// blx1 caveat-(a) COMPLETENESS (v-mem rc-co-read gap): whether `callee` is called from ANY LIFTED body
/// (`db.lifted`). The every-call-site-Owned gate builds its index from `db.defs` bodies ONLY
/// (`ensure_call_site_index` scans `db.defs`, NOT `db.lifted`), so a call to `callee` from an ETA-WRAPPER
/// (`λx. callee x`, a lifted body) is INVISIBLE to that gate — and `def_funcref_taken(callee)` misses it too
/// (the `Core::Closure` references the WRAPPER's code, not `callee`'s). If such a wrapper is invoked via
/// `call_indirect` and forwards a BORROWED arg, `callee` self-dropping it is a UAF. Conservatively EXCLUDE
/// any callee reachable by a `Core::Call` inside a lifted body (leak, never a UAF). This closes the gap
/// WITHOUT scanning `db.lifted` into the SHARED `ensure_call_site_index` — that index also feeds the
/// phase-order emit-once runtime-caller COUNT (#8018), and adding lifted-body edges there could de-stabilize
/// that count's 1-hop idempotence for a def called from a non-idempotent eta-wrapper. So the exclusion is
/// kept LOCAL to the non-looped self-drop.
fn callee_called_from_lifted_body(db: &mut Db, callee: usize) -> bool {
    fn walk(db: &mut Db, id: StructId, callee: usize, seen: &mut HashSet<StructId>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::Call { callee: c, .. } = core_of(db, id)
            && c == callee
        {
            return true;
        }
        crate::backend::wasm::select::reclaim::core_child_ids(db, id)
            .into_iter()
            .any(|ch| walk(db, ch, callee, seen))
    }
    // EXCLUDE every lifted body that is ALSO a `db.defs` body: this gate exists only for an INVISIBLE call
    // edge — an eta-wrapper's `call_indirect`/forwarded call that the `db.defs`-ONLY call-site index
    // (`ensure_call_site_index` walks exactly `db.defs` bodies) misses. A lifted body that IS a def body
    // (a SELF- or MUTUALLY-recursive def hoisted to the funcref table as a combinator — its body is the
    // def's own body) has its `Core::Call { callee }` edges recorded in that index, so AXIS A's per-site
    // owned-arg check already accounts for them (the recursive/partner call's arg is verified `Owned` like
    // any other site). Only a SYNTHETIC lifted body (in `db.lifted` but NOT any def's body — a closure /
    // eta-wrapper) carries the invisible edge this gate must still catch. Without this a borrow-only-heap-
    // param SELF-recursive def (`count-ge`, #8955) OR a MUTUALLY-recursive pair (`drain-a`↔`drain-b`, the
    // partner's def body calls back — #8964-adjacent) is lifted, sees its own/partner recursive call here,
    // and is wrongly excluded → its borrowed `(bytes rest)` scrutinee shell LEAKS one frame per recursion.
    // AXIS A + AXIS B (borrow-only `count_param_consumes==0` + scalar return) still gate each def, so a
    // param that is consumed / escapes a child into a heap result stays excluded (no over-drop / UAF).
    let def_bodies: HashSet<StructId> = db.defs.iter().filter_map(|d| d.body).collect();
    let lifted_bodies: Vec<StructId> = db
        .lifted
        .iter()
        .map(|l| l.body)
        .filter(|b| !def_bodies.contains(b))
        .collect();
    let mut seen = HashSet::new();
    lifted_bodies
        .into_iter()
        .any(|b| walk(db, b, callee, &mut seen))
}

/// blx1 (v-mem co-design): whether the NON-looped def `callee` self-reclaims its heap param at
/// `param_index` via a fn-exit `op_drop` (the non-looped analog of [`looped_owned_param_drops`]). SINGLE
/// SOURCE OF TRUTH — both the emit ([`nonlooped_owned_param_drops`]) and the `call_arg_caller_drops` (6b)
/// YIELD query THIS, so caller-drop XOR callee-epilogue-drop is exactly complementary per (edge, param). A
/// non-inlined non-looped borrow-only-heap-param def (blx1: `classify` reads a `Bytes` via a bin-match and
/// returns a scalar) has NO other callee-side reclaim (the looped epilogue is empty for it; a bin-match
/// borrow lowers to `If`/`BinIntRead`, NOT a `MatchSum`, so `nontail_match_reclaim_binders` misses it) →
/// its shell LEAKS. Reclaim it, DOUBLE-FREE-GATED on BOTH axes (the tr3 lesson):
///   AXIS A (caller-drop complementarity — the callee must OWN the param on EVERY entry):
///     - NOT an export entry (the export trampoline owns/drops the boundary param); AND
///     - NOT funcref-TAKEN ([`def_funcref_taken`] — a `call_indirect` edge is invisible to the direct
///       call-site index → conservatively exclude); AND
///     - NOT called from a LIFTED body ([`callee_called_from_lifted_body`] — an eta-wrapper's call edge is
///       invisible to the `db.defs`-only call-site index; caveat-(a) completeness, v-mem rc-co-read); AND
///     - EVERY DIRECT call site passes an OWNED arg for `param_index` (`heap_operand_ownership == Owned`);
///       a BORROWED / unknown / missing arg at ANY site → NOT owned → decline (missed-borrowed = UAF,
///       missed-owned = leak — default-deny toward the leak).
///   AXIS B (payload-escape — no heap sub-value of the param escapes into the result):
///     `count_param_consumes == 0` (borrow-only — never returned/consumed/escaped as the whole value) AND
///     a SCALAR (non-heap) RETURN (a scalar result cannot embed a heap child of the param nor a view
///     aliasing into its shell — the exact ctor-embed axis that bit tr3, here discharged by scalar-return).
///     A heap-RETURNING borrow-only def needs the general escaped-child analysis → left to leak (follow-up).
/// SOUND-CONSERVATIVE: every gate is default-deny (under-admit = LEAK). v-mem's corpus-wide guarded-all is
/// the empirical double-free backstop.
/// Whether the Bytes param `p` is ever the DIRECT source of a raw VIEW/consume op that can carry an ALIAS
/// of its shell into a value — `Bytes.slice` (a view aliasing the parent backing), `Bytes.compact`, or
/// `String.from-bytes` (both transfer/consume the operand out). These are exactly the Bytes ops that
/// `binding_escapes` recurses `tail_borrowed:false` (escaping) on, and that `count_param_consumes` does NOT
/// count. A bin-match `(bytes rest)`/`(bytes body n)` (`BinRestRead`/`BinSizedRead`) does the OPPOSITE —
/// DUP-before-slice, so its slice owns an INDEPENDENT ref (safe even if it reaches the result), and a
/// `bytes-get`/`bytes-len` reads a scalar (no heap child). So a Bytes param that is NEVER a source here (and
/// is `count_param_consumes==0`) has no shell-alias reaching the result → safe to reclaim at fn-exit even
/// with a heap return. Walks all children (`false` on a shared-node re-visit via `seen`).
fn bytes_param_view_escapes(
    db: &mut Db,
    id: StructId,
    p: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    let src: Option<StructId> = match core_of(db, id) {
        Core::BytesSlice { bytes, .. } => Some(bytes),
        Core::BytesCompact { operand } => Some(operand),
        Core::StrFromBytes { bytes, .. } => Some(bytes),
        _ => None,
    };
    if let Some(s) = src
        && is_ref_to(db, s, p)
    {
        return true;
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| bytes_param_view_escapes(db, c, p, seen))
}

/// Whether the param — or a `Core::Let` binding transitively bound to a BARE ref of it (the runtime
/// bin-match scrutinee materialization `Let{(inner, Param(p))}`) — reaches a RESULT (tail) position of
/// `body` as a bare ref, i.e. is RETURNED directly. Such a return ALIASES the param slot the fn-exit
/// reclaim drop frees, so reclaiming would DOUBLE-FREE it: the drain-that-returns-its-own-scrutinee UAF
/// (`match inp (… (drain rest …)) (_ inp)` — the base/stop arm yields the scrutinee itself). This is the
/// gap that `count_param_consumes` (an operand of a CONSUMING op only) and `bytes_param_view_escapes` (a
/// `Bytes.slice`/`compact`/`from-bytes` VIEW only) both miss — a bare return is neither. Tails are recursed
/// precisely through `Let`/`If`/`Match`/`Seq`; any OTHER tail node (a `Call`/ctor/`BinBuild`/`BytesConcat`
/// — a FRESH or COPIED value, never a bare alias of the param slot) is safe; an unhandled tail shape
/// (`MatchSum`/`Block`) is treated as reaching (conservative — a wrong TRUE only forgoes the reclaim = a
/// leak, never a UAF). `aliases` seeds with the param binder; the `Let` arm grows it with each binding
/// whose value is a bare ref to a current alias (so the materialized `inner` scrutinee counts too).
fn param_ref_reaches_result(db: &mut Db, id: StructId, aliases: &HashSet<StructId>) -> bool {
    param_ref_reaches_result_flagged(db, id, aliases, false)
}

/// [`param_ref_reaches_result`] with the MatchSum tail treated PRECISELY (recurse arm bodies) instead of the
/// conservative "reaches" default. Used ONLY by the 15266 `heap_flat_scalar_reclaimable` disjunct (v-core-opt
/// containment ruling — the shared `param_ref_reaches_result` keeps its conservative MatchSum=reaches default
/// the Bytes carve-out depends on; #9423 shared-classifier lesson). SOUND for the flat-scalar param: an arm's
/// PAYLOAD binder is a CHILD of the scrutinee (not a param alias), and the flat-scalar precondition already
/// proved every child scalar, so an arm can only carry the WHOLE param out by a BARE ref — which the
/// `is_ref_to`/alias tracking still catches. For cf/15266 the Some-arm tail is an `if` of `Rational.of-int`
/// builds (no bare xs) and `rest` is Call-bound (not a tracked bare alias) → precise = FALSE (reclaim admits).
fn param_ref_reaches_result_precise(
    db: &mut Db,
    id: StructId,
    aliases: &HashSet<StructId>,
) -> bool {
    param_ref_reaches_result_flagged(db, id, aliases, true)
}

fn param_ref_reaches_result_flagged(
    db: &mut Db,
    id: StructId,
    aliases: &HashSet<StructId>,
    recurse_matchsum: bool,
) -> bool {
    if aliases.iter().any(|&a| is_ref_to(db, id, a)) {
        return true;
    }
    match core_of(db, id) {
        Core::Let { bindings, body } => {
            let mut ext = aliases.clone();
            for (b, v) in bindings.iter() {
                if ext.iter().any(|&a| is_ref_to(db, *v, a)) {
                    ext.insert(*b);
                }
            }
            param_ref_reaches_result_flagged(db, body, &ext, recurse_matchsum)
        }
        Core::If { then_, else_, .. } => {
            param_ref_reaches_result_flagged(db, then_, &aliases.clone(), recurse_matchsum)
                || param_ref_reaches_result_flagged(db, else_, aliases, recurse_matchsum)
        }
        Core::Match { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            bodies
                .into_iter()
                .any(|b| param_ref_reaches_result_flagged(db, b, aliases, recurse_matchsum))
        }
        // A LIST destructure (`match xs with [] => ys | [h,..t] => …`) is a control node whose ARM bodies
        // are tail-result positions — a bare param ref there ESCAPES as the result (`concat-lists`'s
        // `[] => ys` base arm RETURNS the borrowed `ys` param). It was previously UNHANDLED (fell to the
        // `_ => false` "does not reach" default), so the flat-scalar heap-return admit + self-forward relax
        // wrongly admitted `ys` and its fn-exit epilogue DOUBLE-freed the returned value (the P0
        // cad-test-iterators take-drop-partition UAF, @test-suites-only — the corpus net missed it). Recurse
        // the arm bodies like `Core::Match`. The head/rest pattern binders are PROJECTIONS of the scrutinee
        // (`SumPayload`/`RestFrom`), never bare param aliases, so — as with `Core::Match`/`MatchSum` — only a
        // bare param ref in an arm body reaches (aliases unchanged).
        Core::MatchList { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            bodies
                .into_iter()
                .any(|b| param_ref_reaches_result_flagged(db, b, aliases, recurse_matchsum))
        }
        Core::Seq { tail, .. } => {
            param_ref_reaches_result_flagged(db, tail, aliases, recurse_matchsum)
        }
        // PRECISE MatchSum: recurse the decision-tree arm bodies (like `Core::Match` above). An arm payload
        // binder is a CHILD (not a param alias — flat-scalar proved it scalar), so only a bare param ref in an
        // arm body reaches. (Conservative default keeps MatchSum=reaches for the shared query.)
        Core::MatchSum { root, .. } if recurse_matchsum => {
            sum_cont_param_reaches(db, &root, aliases)
        }
        Core::MatchSum { .. } | Core::Block { .. } => true,
        _ => false,
    }
}

/// Walk a `SumCont` decision tree, asking whether the param (via `aliases`) reaches ANY arm's result —
/// [`param_ref_reaches_result_precise`]'s MatchSum recursion (mirrors [`sum_cont_refs_scrutinee`]'s shape).
fn sum_cont_param_reaches(
    db: &mut Db,
    cont: &crate::core::SumCont,
    aliases: &HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => param_ref_reaches_result_precise(db, *body, aliases),
        crate::core::SumCont::Guarded { body, els, .. } => {
            param_ref_reaches_result_precise(db, *body, aliases)
                || sum_cont_param_reaches(db, els, aliases)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_param_reaches(db, then_, aliases) || sum_cont_param_reaches(db, els, aliases)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_param_reaches(db, &a.cont, aliases)),
    }
}

fn def_nonlooped_reclaims_param(
    db: &mut Db,
    callee: usize,
    param_index: usize,
    layout: &Layout,
) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    // AXIS A: not an export entry (the trampoline owns the boundary param).
    if layout.exports.iter().any(|e| e.body == body) {
        return false;
    }
    // NON-looped only — the looped epilogue owns the looping case (this is its complement).
    if !mutual_loop_group(db, callee).is_empty() {
        return false;
    }
    // AXIS A: funcref-taken exclusion (caveat a — a call_indirect edge is invisible to the direct index).
    if def_funcref_taken(db, body) {
        return false;
    }
    // AXIS A (caveat-a completeness, v-mem rc-co-read): also exclude a callee CALLED FROM a lifted body —
    // an eta-wrapper's `callee x` edge is invisible to the db.defs-only call-site index AND misses
    // def_funcref_taken, so a borrowed forward there would be an unseen UAF. Conservative (leak-safe).
    if callee_called_from_lifted_body(db, callee) {
        return false;
    }
    let params = crate::layout::def_params(db, callee);
    let Some((param_binder, param_ty)) = params.get(param_index).cloned() else {
        return false;
    };
    if !is_heap_type(&param_ty) {
        return false;
    }
    // AXIS B: no heap child of the param escapes into the result.
    //   - A SCALAR (non-heap) return trivially embeds no heap child of the param (any param type).
    //   - A HEAP return is admitted ONLY for a BYTES param that (i) is never the source of a raw view/consume
    //     op (`bytes_param_view_escapes`: `Bytes.slice`/`Bytes.compact`/`String.from-bytes`, which carry a
    //     shell-alias into a value) AND (ii) is never RETURNED as a bare ref (`param_ref_reaches_result` —
    //     the drain-that-yields-its-own-scrutinee `(_ inp)`, which aliases the param slot the fn-exit drop
    //     frees ⇒ a DOUBLE-FREE; the gap `count_param_consumes`/`bytes_param_view_escapes` both miss).
    //     A Bytes param's OTHER result-reaching derivatives are all safe — a
    //     bin-match `(bytes rest)`/`(bytes body n)` DUPs before slicing (own ref) and `bytes-get`/`len` are
    //     scalar — so with no view-escape (and `count_param_consumes==0` below excluding a whole-param
    //     embed/consume: `Bytes.concat`/ctor/call), NO shell-alias reaches the result → reclaiming the
    //     borrowed scrutinee at fn-exit is sound though the drain RETURNS a splice-built heap Bytes. A SUM/
    //     LIST/RECORD param stays scalar-return-gated: its `SumPayload`/`SumExpect`/`Proj` borrowed children
    //     read as non-escaping yet alias the parent shell (the tr3 ctor-embed UAF `(Term.Abs w (payload
    //     p))`) — a hazard this Bytes-only relaxation deliberately does not touch. Closes the heap-
    //     accumulator recursive-drain leak (drain-append / escape-str / encode-elems / encode-members — the
    //     JSON codec encoder, v-json-codec I7).
    // Whether the admit was granted through the 15266 FLAT-SCALAR-CONTAINER heap-return carve-out (below) —
    // the ONLY path that RELIES on the AXIS-B self-forward relax. Scalar-returning self-recursive walks
    // (sum-at/go/lf1) skip the heap-return gate and must keep AXIS B STRICT (self-forwards NOT forgiven):
    // they are already reclaimed by the per-path conditional threaded-param drop (sum-at/go) or correctly
    // leak (lf1, whose xs is captured by the enclosing effect continuation which co-reclaims it — a
    // borrowed-at-the-call-site the epilogue must NOT drop). Forgiving their self-forward here re-admitted
    // an UNCONDITIONAL epilogue drop → a double-free (the go two-sibling + lf1 effect-continuation traps the
    // full-coarse UAF net caught). So the self-forward relax is gated to this flag = heap-return only.
    let mut heap_flat_scalar_admitted = false;
    if is_heap_type(&type_of(db, body)) {
        let bytes_reclaimable = matches!(param_ty.strip_nominal(), Ty::Bytes)
            && !bytes_param_view_escapes(db, body, param_binder, &mut HashSet::new())
            && !param_ref_reaches_result(db, body, &HashSet::from([param_binder]));
        // 15266 FLAT-SCALAR-CONTAINER heap-return admit (v-core-opt-spec'd, the CALLEE-side reclaim locus —
        // sum-at already uses this epilogue; cf/15266 was blocked ONLY here). A heap param with NO extractable
        // heap child (`ty_heap_children_all_scalar`) cannot embed a param-child in a heap result — the tr3
        // ctor-embed UAF the Bytes-only gate guards against is impossible BY TYPE STRUCTURE. The only heap
        // value that could reach the result is the WHOLE param, excluded by the MatchSum-precise bare-return
        // check (`param_ref_reaches_result_precise`) here + `count_param_consumes==0` (no whole-embed/consume)
        // below. So `xs : List Int64` whose Rational result is built from `Int64.of` scalars reclaims at
        // fn-exit like the scalar-returning sum-at. Leak-over-UAF: a param with any heap child (List(List)/
        // Sum-with-heap = tr3) → `ty_heap_children_all_scalar` false → still declined.
        let heap_flat_scalar_reclaimable = ty_heap_children_all_scalar(db, &param_ty)
            && !param_ref_reaches_result_precise(db, body, &HashSet::from([param_binder]));
        if !(bytes_reclaimable || heap_flat_scalar_reclaimable) {
            return false;
        }
        heap_flat_scalar_admitted = heap_flat_scalar_reclaimable;
    }
    // AXIS B: borrow-only — the param is never consumed / returned / escaped as the whole value.
    let mut seen = HashSet::new();
    let mut total = 0usize;
    count_param_consumes(db, body, param_binder, &mut seen, &mut total, true);
    // SELF-FORWARD RELAX: an IDENTITY self-recursive forward — a `Call` to THIS callee whose arg at exactly
    // `param_index` is a bare ref to the param binder (`(cf xs …)` → cf's param 0) — is counted as a "consume"
    // by `count_param_consumes` (it sees a bare param ref as a Call arg = ownership transfer). But it is NOT an
    // escape: admitting the reclaim makes the recursive call site DUP the param (owned transfer) and the inner
    // frame reclaim it at ITS OWN fn-exit epilogue — dup + inner-drop is balanced, and the inner frame reclaims
    // on its base arm by the same induction (the def is callee-owned for this param; the EXTERNAL sites, checked
    // by AXIS A below, establish the ground ownership). This mirrors `nonlooped_param_callee_owned_core`'s
    // self-back-edge skip and `looped_invariant_param_caller_owned`'s. It is admitted ONLY when EVERY consume is
    // such an identity self-forward (`total == self_forwards`); any other consume (a whole-param embed/consume,
    // a forward at a DIFFERENT index, or a forward to a DIFFERENT callee) escapes and still declines. Wrong
    // admit ⇒ leak (never a UAF): if the call site does NOT actually dup, the inner reclaim just leaves the
    // outer ref undropped. Closes 15266's cf (self-recursive flat-scalar-container accumulator).
    // The self-forward relax is SCOPED to the heap-return flat-scalar admit (`heap_flat_scalar_admitted`) —
    // see that flag's comment. For every other shape (scalar return, Bytes carve-out) AXIS B stays STRICT
    // (`total == 0`), preserving the pre-change behavior exactly (those cases reclaim via the conditional
    // threaded-param drop or correctly leak; forgiving their self-forward double-freed — go/lf1).
    let allowed_consumes = if heap_flat_scalar_admitted {
        let mut sf_seen = HashSet::new();
        let mut self_forwards = 0usize;
        count_param_self_forward_consumes(
            db,
            body,
            param_binder,
            callee,
            param_index,
            &mut sf_seen,
            &mut self_forwards,
        );
        self_forwards
    } else {
        0
    };
    if total != allowed_consumes {
        return false;
    }
    let self_forwards = allowed_consumes;
    // SELF-FORWARD MUTUAL-EXCLUSION (the go/two-sibling UAF net, 09-functions): when the admit RELIES on
    // forgiving self-forward consumes (`self_forwards > 0`), decline if the PER-PATH CONDITIONAL threaded-param
    // drop (`def_nonlooped_callee_reclaims_threaded_param` — `plan_ifjoin_nested` D-arm drop) ALREADY reclaims
    // this param. That path engages for a MULTI-sibling owned self-recursion (`(+ (go xs …) (go xs …))`: the
    // scalar_group consume-spare + the coupled base-arm drop already balance it), so ADDING this UNCONDITIONAL
    // fn-exit epilogue drop would DOUBLE-free (a wasm-unreachable trap). cf's linear single self-forward does
    // NOT trigger the consume-spare, so the conditional path is absent (false) and the epilogue is the SOLE
    // reclaim. This is semantic mutual exclusion, not a count cutoff — leave the already-reclaimed case to its
    // existing correct drop (declining here just forgoes a redundant drop, never a leak for those cases).
    if self_forwards > 0 && def_nonlooped_callee_reclaims_threaded_param(db, callee, param_index) {
        return false;
    }
    // AXIS A: every DIRECT call site passes an OWNED arg for this param (unknown/borrowed/missing at ANY
    // site → not all-owned → decline). A callee with NO known call site cannot prove ownership → decline.
    // A SELF-recursive site forwarding the same param (the identity self-forward relaxed in AXIS B above) is
    // NOT a fresh external ownership source — it threads the same ref by induction — so it is EXCLUDED here;
    // the EXTERNAL sites (a caller body != this callee's body) establish ground ownership.
    let self_body = body;
    let sites = crate::infer::callee_call_site_args_with_caller(db, callee);
    let external: Vec<&Vec<StructId>> = sites
        .iter()
        .filter_map(|(caller, args)| {
            if *caller == self_body
                && matches!(args.get(param_index), Some(&a) if is_ref_to(db, a, param_binder))
            {
                None
            } else {
                Some(args)
            }
        })
        .collect();
    if external.is_empty() {
        return false;
    }
    for args in &external {
        match args.get(param_index) {
            Some(&arg) if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned)) => {
            }
            _ => return false,
        }
    }
    true
}

/// AXIS A of [`def_nonlooped_reclaims_param`] WITHOUT its borrow-only-all-paths (AXIS B) gate: whether a
/// non-looped def's heap param at `param_index` is CALLEE-OWNED (every DIRECT call site passes an owned arg;
/// not an export/funcref/lifted boundary whose trampoline owns it). Used to gate the PER-PATH CONDITIONAL
/// param drop (a `plan_ifjoin_nested` D-arm drop for a param that DIVERGES — consumed on some arms, dead on
/// others), the COMPLEMENT of the unconditional never-consumed fn-exit epilogue: callee-owned ⟹ the frame
/// owns a ref to reclaim on the dead arm; a borrowed / boundary-owned param must NOT be dropped (double-free).
/// (v-memory-safety half-2 of the growing-heap-recursive-fold-state leak — the non-looped fold's discarded
/// FINAL state, e.g. the effect-handler string-rope fold-fn base case that borrow-reads then discards.)
fn nonlooped_param_callee_owned(
    db: &mut Db,
    callee: usize,
    param_index: usize,
    layout: &Layout,
) -> bool {
    // The layout-free core is the single source of truth (its export exclusion uses `db.exports`, the same
    // authoritative def-level list `looped_invariant_param_caller_owned` relies on). `layout` is retained in
    // the signature only for the existing `select_function_of` / `def_emits_ifjoin_param_drop` call sites.
    let _ = layout;
    nonlooped_param_callee_owned_core(db, callee, param_index)
}

/// LAYOUT-FREE core of [`nonlooped_param_callee_owned`] — usable from the dup pass (which has no `Layout`),
/// e.g. [`def_nonlooped_callee_reclaims_threaded_param`]. Whether a NON-LOOPED def's heap param at
/// `param_index` is CALLEE-OWNED, with the SELF-FORWARD RELAX: a SELF-recursive call site whose arg for
/// `param_index` is exactly the param binder (identity self-forward to the same index) is EXCLUDED from the
/// all-external-sites-owned check — sound by induction (the def is callee-owned for this param, so forwarding
/// the same param threads ownership into the recursive frame, which reclaims on ITS base arm; the EXTERNAL
/// sites establish the ground ownership). Mirrors [`looped_invariant_param_caller_owned`]'s self-back-edge
/// skip, adapted for the non-looped per-path drop. Wrong FALSE ⇒ a leak (the drop is forgone), never a UAF.
fn nonlooped_param_callee_owned_core(db: &mut Db, callee: usize, param_index: usize) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    // AXIS A (mirrors def_nonlooped_reclaims_param): not an export entry (the trampoline owns the boundary
    // param); non-looped only (the looped epilogue owns that case); not funcref-taken / called-from-lifted
    // (a call_indirect / eta-wrapper edge is invisible to the direct-index owned-arg check → unseen UAF).
    if db.exports.iter().any(|e| e.def == Some(callee)) {
        return false;
    }
    if !mutual_loop_group(db, callee).is_empty() {
        return false;
    }
    if def_funcref_taken(db, body) {
        return false;
    }
    if callee_called_from_lifted_body(db, callee) {
        return false;
    }
    let params = crate::layout::def_params(db, callee);
    let Some((param_binder, param_ty)) = params.get(param_index).cloned() else {
        return false;
    };
    if !is_heap_type(&param_ty) {
        return false;
    }
    // Every EXTERNAL (non-self) call site passes an OWNED arg for this param (unknown/borrowed/missing at any
    // external site → not all-owned → decline). A SELF-recursive site that identity-forwards THIS param is
    // skipped (owned-by-induction, see the doc). At least one external owned site must ground the induction.
    let sites = crate::infer::callee_call_site_args_with_caller(db, callee);
    let mut saw_external = false;
    for (caller_body, args) in &sites {
        let self_forward = *caller_body == body
            && matches!(
                args.get(param_index).map(|&a| core_of(db, a)),
                Some(Core::LocalRef { binder: b } | Core::Param { binder: b }) if b == param_binder
            );
        if self_forward {
            continue;
        }
        saw_external = true;
        match args.get(param_index) {
            Some(&arg) if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned)) => {
            }
            _ => return false,
        }
    }
    saw_external
}

/// NON-LOOPED analog of [`def_looped_callee_reclaims_threaded_param`], for the dup pass's `scalar_group`
/// consume-spare (via [`callee_reclaims_threaded_binder_arg`]). Whether a NON-LOOPED callee RECLAIMS its heap
/// param at `param_index` on the path where it is not consumed — i.e. the callee's body would emit a
/// `plan_ifjoin_nested` base-arm D-arm drop for this callee-owned divergent param (the [`select_function_of`]
/// half-2 emit). This UNIFIES the two levers of the self-recursive-sibling-consume leak: the caller's spare
/// (grant `k-1`) is granted IFF the callee actually drops the reused ref on its base arm — so the spare and
/// the base-arm drop are coupled (present together or absent together; absent ⇒ a leak, never a UAF).
///
/// SELF-RECURSION SAFETY: this runs from WITHIN the dup pass (mark_binder_dups → the seq closure), so it must
/// NOT reconstruct `code.dup_sites` via `collect_dup_sites` (that re-enters mark_binder_dups → this predicate
/// → infinite recursion for a self-recursive callee). It gates on an EMPTY-dup `plan_ifjoin_nested` instead —
/// a SOUND UNDER-APPROXIMATION of the real emit: more dups ⇒ `binding_escapes_dup_aware` reports LESS escape
/// ⇒ an arm is MORE likely dead ⇒ MORE likely to plan the drop. So an empty-dup "plans a drop" ⟹ the real
/// dup-aware emit also plans it (the base arm's borrow-read, e.g. `List.len xs`, is dup-independent anyway).
/// A false negative (empty-dup misses a drop the real emit makes) just forgoes the spare → a leak, never UAF.
pub(crate) fn def_nonlooped_callee_reclaims_threaded_param(
    db: &mut Db,
    callee: usize,
    param_index: usize,
) -> bool {
    if !nonlooped_param_callee_owned_core(db, callee, param_index) {
        return false;
    }
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    let params = crate::layout::def_params(db, callee);
    let Some((binder, ty)) = params.get(param_index).cloned() else {
        return false;
    };
    if !is_heap_type(&ty) {
        return false;
    }
    // Empty-dup under-approximation (see the doc — avoids re-entering the dup pass).
    // `net_borrow=false`: this predicate models ONLY the dead-arm drop (it IS GATE-1 for the net-borrow admit,
    // so enabling net-borrow here would recurse into itself; and with an empty dup set every consume reads as an
    // escape so the net-borrow admit is inert regardless).
    let dup: HashSet<StructId> = HashSet::new();
    let aliases = HashSet::from([binder]);
    let mut plan: HashMap<StructId, Vec<(u32, bool)>> = HashMap::new();
    plan_ifjoin_nested(db, body, &aliases, 0, &dup, false, &mut plan);
    !plan.is_empty()
}

/// CATALAN 2nd-root (v-memory-safety, framing A-gated-B): whether a THREADED-CALLEE co-operand `(callee … c …)`
/// RECLAIMS-OR-BORROWS its heap param at `param_index` WITHIN the call — i.e. the callee does NOT escape/keep
/// that param, so its reference is released by the call (a borrow is never taken; an invariant owned-borrow is
/// dropped at the fn-exit epilogue via [`looped_owned_param_drops`], BEFORE the call returns). Used by the
/// deferred-consume-op operand seq (mark_binder_dups, v-inference) to grant the `k-1` (spare_last) dup
/// accounting to such a co-operand instead of the `k != i` count: since the callee's ref is freed by the call,
/// a SIBLING consume of the same binder does NOT need its own retained ref held simultaneously.
///
/// This is the LOOPED analog of [`def_nonlooped_reclaims_param`] (blx1) restricted to the caller's dup
/// decision. The load-bearing distinction vs the szf-9 threaded-and-consumed witnesses (which KEEP `k != i`):
/// those callees ESCAPE the param (hold it simultaneously-live in the result / a ctor), so
/// `param_only_borrowed_or_backedge` is FALSE for them and they are correctly EXCLUDED here.
///
/// Gate — every conjunct conservative toward NOT granting `k-1` (wrong ⇒ an EXTRA dup = a leak, never a UAF):
///   • LOOPED single-member self-recursion (the looped analog; a mutual group shares slots — deferred).
///   • not an export entry / not funcref-taken / not called-from-lifted (the [`def_nonlooped_reclaims_param`]
///     UAF caveats — a `call_indirect`/eta edge could pass a param this direct-index analysis cannot see).
///   • heap param, INVARIANT across every back-edge (identity-threaded), and `param_only_borrowed_or_backedge`
///     (read + identity-back-edge only → NOT escaped → reclaimed/borrowed within the call). Exactly the
///     invariant-path condition [`looped_owned_param_drops`] uses to prove the callee reclaims the param.
///
/// WIRING (v-inference, mark_binder_dups deferred-consume seq): for a co-operand that is a
/// `Core::Call { callee, args }` where the binder is `args[j]`, this predicate at `param_index = j` being
/// `true` makes that co-operand SPARE-ELIGIBLE (grant `k-1` — do not add it to a sibling bare-binder
/// consume's `other`/`live_after`), THEN further gated PATH-SENSITIVELY at the wiring site (the la-fold spare
/// is only sound when the binder has no OTHER in-path live use — see the `holds_no_handle` gate). VERIFIED
/// (v-memory-safety instrument): `true` for gP2/CATALAN's `rlen`/`conv` (threaded read-only heap param,
/// `drops=[that slot]`); `false` for the varying `grow` accumulator (`drops=[]`).
pub(crate) fn def_looped_callee_reclaims_threaded_param(
    db: &mut Db,
    callee: usize,
    param_index: usize,
) -> bool {
    let Some(body) = db.defs.get(callee).and_then(|d| d.body) else {
        return false;
    };
    let params = crate::layout::def_params(db, callee);
    // The SLOT this param takes (dense `0..n`, Unit elided) — matching `looped_owned_param_drops`.
    let Some((param_binder, param_ty)) = params.get(param_index).cloned() else {
        return false;
    };
    if matches!(param_ty.strip_nominal(), Ty::Unit) || !is_heap_type(&param_ty) {
        return false;
    }
    let mut slot: Option<u32> = None;
    let mut next: u32 = 0;
    for (binder, ty) in params.iter() {
        if matches!(ty.strip_nominal(), Ty::Unit) {
            continue;
        }
        if valtype_of(ty).is_none() {
            return false; // a param with no machine rep → this def won't select.
        }
        if *binder == param_binder {
            slot = Some(next);
        }
        next += 1;
    }
    let Some(slot) = slot else {
        return false;
    };
    // SINGLE SOURCE OF TRUTH: the callee RECLAIMS this param at its fn-exit epilogue iff its slot is in
    // `looped_owned_param_drops` — the EXACT set the emit drops (invariant + `param_only_borrowed_or_backedge`,
    // or the varying-epilogue case). If the callee reclaims the param, its reference is released BEFORE the
    // call returns, so a sibling consume of the same binder at the call site needs no simultaneously-held
    // retained ref → `k-1` is safe. (The `def_nonlooped_reclaims_param` funcref-taken/lifted/export exclusions
    // are NOT needed here: they gate whether the callee SHOULD self-drop for indirect edges; this predicate
    // only READS the drop set the callee ALREADY emits at THIS direct call, so it is sound by construction —
    // wrong ⇒ an extra dup = a leak, never a UAF, since a non-reclaiming param is simply absent from the set.)
    looped_owned_param_drops(db, body, &params, Some(callee)).contains(&slot)
}

/// Whether `body` contains a `Core::Call` whose arg triggers a caller-drop ([`call_arg_caller_drops`]) — the
/// import-side companion of the `Core::Call` emit, so `collect_module_used_ops` imports `drop` iff the emit
/// actually emits a caller-drop (precise import/emit agreement, like `def_drops_owned_param`). Cycle-guarded.
pub fn body_has_caller_drop(
    db: &mut Db,
    body: StructId,
    layout: &Layout,
    self_def: Option<usize>,
) -> bool {
    // Reconstruct THIS body's caller-surplus dup set EXACTLY as `select_function_of` does (retain-only
    // `collect_dup_sites` MINUS the shell-reclaim child-dups), so the 5786 caller-drop admit's (B) check
    // matches the emit → the `drop` import agrees with what the emit actually emits (no under-import).
    let mut heap_binders: Vec<StructId> = Vec::new();
    collect_retain_candidate_binders(db, body, &mut heap_binders);
    let mut retain_only: HashSet<StructId> = HashSet::new();
    collect_dup_sites(db, body, &heap_binders, &mut retain_only);
    let mut shell: HashSet<StructId> = HashSet::new();
    collect_shell_reclaim_child_dups(db, body, &mut shell);
    let caller_surplus_dup_sites: HashSet<StructId> =
        retain_only.difference(&shell).copied().collect();
    fn walk(
        db: &mut Db,
        id: StructId,
        layout: &Layout,
        self_def: Option<usize>,
        caller_surplus_dup_sites: &HashSet<StructId>,
        seen: &mut HashSet<StructId>,
    ) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::Call { callee, args } = core_of(db, id) {
            for (i, &a) in args.iter().enumerate() {
                if call_arg_caller_drops(
                    db,
                    callee,
                    a,
                    i,
                    layout,
                    self_def,
                    caller_surplus_dup_sites,
                ) {
                    return true;
                }
            }
        }
        crate::backend::wasm::select::reclaim::core_child_ids(db, id)
            .into_iter()
            .any(|c| walk(db, c, layout, self_def, caller_surplus_dup_sites, seen))
    }
    walk(
        db,
        body,
        layout,
        self_def,
        &caller_surplus_dup_sites,
        &mut HashSet::new(),
    )
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
        // 5786 admit reads the emitting def (`out.self_def`) + this body's caller-surplus dup set; both are
        // set by `select_function_of` before the emit. Snapshot before the per-arg `out` mutation below.
        let sd = out.self_def;
        let ds = out.caller_surplus_dup_sites.clone();
        (0..args.len())
            .map(|i| call_arg_caller_drops(db, callee, args[i], i, layout, sd, &ds))
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
