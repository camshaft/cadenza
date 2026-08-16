# Dedup width-pair face (2026-08-11) — re-land 3c770881a, continued

Angle: the congruence hashes body structure; two walkers congruent except the
ACCUMULATOR WIDTH (Int64 vs Float64, distinct wasm value types) must not merge
— a width-blind hash would emit one body and corrupt the other call's ABI.

GREEN x3:
- gi3: walki (Int64 acc) + walkf (Float64 acc, same shape via of-int) under
  one handler; wasm/rust/rust-async all exact — 3308/8

Vocab: NO Float64->Int64 conversion exists in the module surface (no floor-int/
to-int; only of/of-int/nan/max) — compare floats directly or thread as float.
Generic unannotated params infer from FIRST USE and pin the def (gi1's acc
became Int64; a second float call is CDZ0301, not a fresh instantiation —
def-level generics need explicit type params, unlike open rows).

Pin candidate: joins 241 pool (cg1-cg3 + gi3).
