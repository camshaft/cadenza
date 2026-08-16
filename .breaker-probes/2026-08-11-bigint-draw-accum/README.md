# BigInt x effects boundary faces (2026-08-11)

Angle: bg3/bg4 pin BigInt states with small values; the Int64-BOUNDARY faces
(limb carry born from accumulated draws; state crossing MAX mid-thread) were
uncovered.

GREEN x3:
- bi2: recursive draws (i64, near-MAX seed) accumulate into a BigInt PAST the
  Int64 boundary — the limb carry happens in the accumulator — 1/2
- bi3: BigInt handler STATE crosses Int64 MAX mid-thread; the > verdict flips
  exactly at the crossing dispatch (0,0,1 -> 100)

Vocab: integer literals are checked against Int64 range even in BigInt.of
position — build big constants by BigInt ARITHMETIC ((* (BigInt.of big) 
(BigInt.of 2)) etc.), or annotate UInt64.

Pin candidates: 244 pool.
