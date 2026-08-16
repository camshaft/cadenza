# crs ladder — carousel bitmask boarding (2026-08-16, tick 1620)

Attack: BITWISE ops inside the branch structure — the gate test
`(& (>> mask gate) 1)`, the fill `(| mask (<< 1 gate))`, the clear
`(^ mask (<< 1 g))` — where `gate = (% (- 4 pos) 4)` is itself a shared
compound appearing 4x in the board arm. First bitmask + rotation composition.

## Envelope
- crs1 (6 dispatches, 3 ops): scratch-locals clean decline — consistent with
  the two-shared-compound fence (gate compound + rotation advance).
- crs2 (4 dispatches): PASSES ×3 all backends. Differential: n=10 starts at
  pos 2 so gondola 2 boards first and the unload(2) HITS (102); n=0 starts at
  pos 0 so gondola 0 boards first, gondola 2 never fills, unload(2) MISSES
  (902). Riders/pos pack differs (10 vs 22).

crs2 hand model: n=10 [21,11,102,10] → 21011102010;
n=0 [1,31,902,22] → 1031902022.

Pass ×3 wasm + rust + rust-async on trunk f00670782. crs1 held for (b).

Note: 7-row base-1000 packing overflowed Int64 in the first draft — packed
the tail at base-100 then dropped to 4 rows anyway. Bounds-check stands.
