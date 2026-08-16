# Spec-dedup congruence attack (2026-08-11) — re-land 3c770881a

Target: the re-landed content-addressed spec dedup (cost-cliff lever; original
a383c5711 reverted because the RUST backend resolves Core::Call BY NAME and a
merged-away spec left a dangling fn — E0425). Attacked the congruence on its
must-not-merge and rust-by-name faces from a detached origin base.

All GREEN x3x3 (esp. the rust targets = the revert cause):
- cg1: two recursive performers IDENTICAL except the EFFECT they perform
  (A vs B) — must not merge across effect identity — 1833/0
- cg2: near-congruent (one coefficient differs, *2 vs *3) — must keep both —
  12600066
- cg3: TRULY congruent twins (may merge) — both call sites exact — 42033
  (cg3 is the rust-by-name regression witness: if fn_ident missed the
  representative redirect, wb's call would dangle.)

Pin candidates: 241 pool.

## Mutual-congruence angle (tick 1221): NOT REACHABLE through the fold
- mc1 (draw-before-recurse mutual pairs): declines — known fence.
- mc2 (TWO congruent mutual pairs, landed foldable idiom each): declines —
  two mutual SCCs in one handle exceed the group fold.
- mc3 (ONE mutual pair called TWICE): declines — even two CALLS of one
  mutual SCC exceed it (the landed mutual pin has exactly one call).
So the dedup's mutual-partner face (the od_eff4 revert shape) can't be reached
from corpus-position programs — it only arises inside compiler-ml's self-
compile where the fold context differs. cg1-cg3 cover the reachable faces.
FENCE for v-effects: mutual-SCC × multi-call is the next fold frontier.
