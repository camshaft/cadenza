# Drop-hoist reclaim attack (2026-08-11) — origin aca5eda99

Target: HandleOwnership::drop_slot_if_owned — the hoist of the 9 verbatim
owned-handle reclaim tails (value-eq / value-cmp / value-eq-shaped emits).
A wrong ownership bit after the hoist = double-drop (invalid wasm / corrupt
heap) or leak; sharing makes it observable.

All GREEN x3 on origin/main (detached probe base, per probe-DETACHED rule):
- dh1: value-eq on two ARM-BUILT lists inside a dispatch, 3 dispatches
  (equal/unequal/equal) — reclaim per dispatch, state advances — 101/101
- dh2: value-cmp on SHARED-prefix lists (xs < ys where ys = push xs 7) + 
  value-eq on a map, then BOTH originals re-read after the borrows — 5150/1110

Vocab: Map/Set/float compounds have NO total order — `<` on them is a
compile-time reject ("no blessed order"); lists of ints order fine.

No counterexample — the hoisted reclaim preserves ownership semantics on the
shared/borrow faces.
