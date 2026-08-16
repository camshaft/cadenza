# DESIGN: sharing-aware emit — bind a shared heap-handle Core node once into a `Core::Let` slot

Status: DRAFT (2026-08-16). Owner split: v-core-opt owns the Core-IR side (detect a shared
node, bind it once into a `Core::Let` slot, re-point references to `Core::LocalRef`);
v-rust-backend owns the emit-side dup/drop refcount discipline for the shared-emitted-once
handle; v-agent-harness co-reviews the dup/drop seam. This doc is the IR-shape contract the
emit side designs against.

## Problem

`reduce_handle` produces a correct compact shared Core DAG: a subexpression reached from K
parent edges is ONE `StructId` with K in-edges. But the emit-analysis walks descend
`core_child_ids` as a TREE, re-walking a shared node once per in-edge — `O(K^depth)` on a wide
share. The class-A op-walk (`collect_used_ops_into`) and class-B1 node-intrinsic dup-site
walks (`collect_shell_reclaim_child_dups` / `collect_row_op_field_dups`) took sound node-id
visited-sets and are LANDED. The residual is class-B2: `mark_binder_dups` (dup/retain
placement), whose per-node decision depends on the PATH tuple `(consuming, live_after,
in_proj_operand)`, so a node-id visited-set is unsound (same node under different `live_after`
→ different dup decision). Measured: on the heaviest self-host body `mark_binder_dups`
(`collect_dup_sites`) is ~27% of a 271M-core-visit re-descent; on a dup-dominated body it is
~94%. cmb1/pom5 still hang on exactly this.

## Fix: make the sharing EXPLICIT in the IR

A memo keyed on the full 5-tuple `(node, binder, consuming, live_after, in_proj_operand)` would
be sound but has a large key space → poor hit-rate → wrong tool. Instead, bind each shared
HEAP-HANDLE node once into a `Core::Let` slot and re-point its K references to
`Core::LocalRef`. Then `mark_binder_dups` sees the shared node ONCE structurally (as the
binding's value), and each of the K uses is a `LocalRef` whose `live_after` is computed
against the SLOT — not re-derived by re-descending the shared subtree per path. This is a
refcount-correct CSE hoist for heap handles (the existing scalar CSE already hoists pure scalar
shares via `collect_dominating_frontier` + `is_cse_shareable`, but deliberately excludes heap
handles because their sharing needs a dup/drop contract — which is exactly what this adds).

## IR shape (the substrate the emit side reads)

Reuses the EXISTING `Core::Let` / `Core::LocalRef` mechanism — NO new Core variant, and
mirrors `opt.rs cse_body` (opt.rs:344-345) EXACTLY:

- `Core::Let { bindings: Rc<[(binder, value)]>, body }` — the slot is keyed by `binder`, a
  FRESH synth node id DISTINCT from `value` (`binder = synth_core(LocalRef{value}, ty)`); the
  binding is `(fresh_binder, shared_id)` with `fresh_binder != shared_id`.
- `Core::LocalRef { binder }` — reads the slot; the backend maps it to `local.get` of the
  binding's slot, exactly as today.

The B2 pass adds a binding `(fresh_binder, shared_id)` for a shared heap-handle node and
rewrites its K parent edges to `Core::LocalRef { binder: fresh_binder }`. The binding is placed
at the NEAREST DOMINATOR of all K uses (reuse `collect_dominating_frontier`), so every use is
in scope and the value is computed once before any use.

⚠️ DO NOT use a SELF-KEYED binding `(shared_id, shared_id)`. Two reasons: (1) a self-keyed
binder resolves to itself → infinite cycle (the `cse_body` comment at opt.rs:339-343 documents
this); (2) COLLISION with the class-B1 `collect_row_op_field_dups` — that collector matches
EXACTLY a self-keyed `Let [(bk,bv)]` with `bk == bv` + a `Core::Record` body + `Proj{operand:
bk}` heap fields (select.rs:1241-1249). A self-keyed slot Let over a Record-shaped shared node
would be marked by row_op AND dup'd by the slot's own discipline → DOUBLE-dup → refcount LEAK
(v-agent-harness co-review, 2026-08-16). The DISTINCT fresh binder makes row_op's `bk == bv`
guard FALSE for the slot Let → no collision, structurally. This is the "do-not-double-count"
check the co-verify (§ below) asserts.

## Detection + timing (Core-IR side, v-core-opt)

This is the OPTION-A follow-up the scalar CSE MVP explicitly deferred (opt.rs:396-397) — NOT a
tweak to `cse_body`'s pre-lowering hook. Two grounding facts (verified in opt.rs:236-246):
- `cse_body` runs at the PassManager hook BEFORE lazy lowering, and its eligibility gate is
  `if body_is_pure_scalar(db, body)` — which EXCLUDES the heap/compound bodies B2 targets, and
  running pre-lowering MEMO-POISONS context-needing nodes. So B2 must run on the POST-LOWERING
  column (a `force-lower-all` before the pass). `collect_node_refs` → `licm_children` →
  `core_of` already walks that lowered/reduced DAG, so detection sees the real sharing AND the
  within-arm frontier.
- Detection: `collect_node_refs` (core_analysis.rs:186) gives per-`StructId` parent-edge count
  (reached twice counts 2), interior walked once. Candidate = count ≥ 2 AND heap-handle type
  (`is_heap_type` / `get_op` None, non-Unit). Exclude `Core::LocalRef` (already a slot read) and
  Unit. Bind with the distinct fresh binder (see IR shape above).
- Placement: `collect_dominating_frontier` (core_analysis.rs:148) + an UNCONDITIONAL-REACH check
  is the byte-neutrality gate — bind at the nearest common dominator (LCA) of all K uses; only
  hoist past a branch when the node is unconditionally reached on every path reaching any use
  (else the hoist speculates a conditionally-needed value = moved work/trap = not byte-neutral).
  The scalar CSE's Guard (D1) (opt.rs:383-400) is the body-ROOT-frontier instance of this; B2
  extends it to ENCLOSING-scope frontiers (an arm's own root) so cmb1/pom5's WITHIN-ARM shares
  bind — that extension is the core of the B2 Core-IR work. A branch-local slot needs no new
  emit drop logic (the Core::Let closing drop scopes to the Let, so an in-arm LCA scopes the
  drop to that branch automatically).
- Scope: ONLY heap-handle shared nodes (scalar shares already handled by the scalar CSE — this
  is precisely lifting its Guard (A) heap exclusion at opt.rs:377, made safe by the emit dup/drop).

## Refcount invariant the emit side can rely on (v-rust-backend designs the dup/drop)

The contract the Core-IR side GUARANTEES, so the emit side's dup/drop is well-defined:

1. A shared heap-handle node bound into a slot is COMPUTED EXACTLY ONCE (at the binding), so
   its handle enters the slot at refcount 1.
2. It is read by exactly K `Core::LocalRef`s (K = the parent-edge count that triggered the
   binding). The emit side must ensure the handle survives all K reads and is freed exactly
   once — i.e. `dup` per surviving reference and `drop` when the slot's last live use passes,
   NO early-free, NO leak. (This is the standard Perceus slot discipline; the NEW part is that
   the slot value is a shared heap handle, so the K reads each may consume or borrow.)
3. `mark_binder_dups` now runs over the slot binder like any other `Core::Let` binder — the
   per-use `consuming`/`live_after` is computed against the slot's `LocalRef` occurrences, and
   the existing dup-site logic applies UNCHANGED (the win is purely that the shared subtree is
   no longer re-descended per path; the dup decision logic is identical).

RESOLVED (v-rb + v-agent-harness co-review, 2026-08-16):
- Q1 (heap-handle slot value; K reads vs the usual 2-use retain): NO distinct retain count.
  The standard per-OCCURRENCE `mark_binder_dups` logic over the K `LocalRef` reads is correct —
  each CONSUMING read dups EXCEPT the last-consuming one (which takes the slot's owned ref);
  borrowing reads do not dup. Emit routes on the binder TYPE (`is_heap_type_for_retain`), so a
  heap slot binding auto-gets the retain, a scalar one gets nothing — no new emit machinery.
  Q1 NAIL (v-agent-harness): the slot's escape-gated DROP must fire at the POST-DOMINATOR of
  ALL K uses INCLUDING borrows (not the textual last use) — the dual of the dominating-frontier
  entry placement. The existing escape-gated Core::Let drop already does this (its
  `binding_escapes` gate sees every one of the K `LocalRef` occurrences, since they are all
  reads of the slot binder), the same mechanism that fixes the "consumed-early-read-late" case
  documented at select.rs:1284-1291.
- Q2 (no double-count vs class-B1): RESOLVED by the DISTINCT fresh binder (see ⚠ above). A slot
  read that is a `Core::Proj`/`SumPayload` off the binder is a `LocalRef`-rooted occurrence
  marked by `mark_binder_dups`, never a B1 node-intrinsic site; and the distinct binder keeps
  `collect_row_op_field_dups`'s `bk == bv` guard false so the slot Let is never a row-op site.
  OWNERSHIP CAVEAT: if the bound node is itself a B1-marked MatchSum scrutinee, the rewrite must
  keep `heap_operand_ownership == Owned` so B1 fires identically (a computed-once owned handle
  in a slot IS owned) — EXACTLY ONE of {B1, mark_binder_dups} marks each read, never both.

## Verification plan (v-core-opt)

- Byte-neutrality on the CORPUS (`gate --opt-sweep` 0-divergence): binding a shared node into a
  slot must not change any OBSERVABLE output — it changes only WHERE the value is computed
  (once vs K times), which is observably identical for a pure/heap value.
- Node-visit collapse: instrumented per-body `core_of`-call delta (the harness used for the
  291M measurement) — expect the `mark_binder_dups` share to drop toward linear on cmb1/pom5.
- Hang witnesses: cmb1 + pom5 flip hang → compile (breaker re-sweeps; the joint pass-pin
  battery is staged).
- Self-host: `cdz test implementation/compiler-ml` must stay green (the 291M bodies must still
  emit identically, just faster).
