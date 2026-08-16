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

Reuses the EXISTING `Core::Let` / `Core::LocalRef` mechanism — NO new Core variant:

- `Core::Let { bindings: Rc<[(binder, value)]>, body }` — `binder` is the value's own
  `StructId` (the existing convention: the slot identity a reference resolves to).
- `Core::LocalRef { binder }` — reads the slot; the backend maps it to `local.get` of the
  binding's slot, exactly as today.

The B2 pass adds a binding `(shared_id, shared_id)` for a shared heap-handle node and rewrites
its K parent edges to `Core::LocalRef { binder: shared_id }`. The binding is placed at the
NEAREST DOMINATOR of all K uses (reuse `collect_dominating_frontier`), so every use is in scope
and the value is computed once before any use.

## Detection (Core-IR side, v-core-opt)

- `collect_node_refs` (core_analysis.rs:186) already computes the per-`StructId` parent-edge
  count (a node reached twice counts 2), interior walked once. A node with count ≥ 2 AND a heap
  handle type (`is_heap_type` / `get_op` None, non-Unit) is a binding candidate.
- Placement: `collect_dominating_frontier` (core_analysis.rs:148) gives the dominating point;
  the slot binds there.
- Scope: ONLY heap-handle shared nodes (scalar shares already handled by scalar CSE; a scalar
  needs no dup/drop). Exclude `Core::LocalRef` itself (already a slot read) and Unit.

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

OPEN QUESTIONS for the emit side (v-rb) to resolve against this shape:
- Does the existing `LocalRef` slot dup/drop already handle a heap-handle slot value
  correctly, or does the shared-emitted-once case need a distinct retain count (K reads vs the
  usual 2-use retain)?
- Interaction with the class-B1 dup-site sets already collected: a slot read that is a
  `Core::Proj`/`SumPayload` off the slot binder is already a dup-site candidate — confirm the
  slot binding does not double-count.

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
