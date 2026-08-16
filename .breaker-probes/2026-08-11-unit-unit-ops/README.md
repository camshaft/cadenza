# Unit-degenerate op shapes (2026-08-11)

Angle: Unit in BOTH op positions (arg and result) driven purely for state
side effects, and unit as the handler STATE itself under a recursive walk.
Degenerate-value ABI faces (zero-width crossings).

GREEN x3:
- uu1: (-> Unit Unit) op, two marks then count — 2/42
- uu2: UNIT handler state threading through a recursive doubling walk — 20/0

Ops note: the match-discard chain version of uu1 DECLINED (the depth-2+ fence
from tick 1193, consistent); the let-chain folds.

Store note: hit a poisoned content-address entry mid-tick ("has content
address X, not the required Y — refusing") — rm -rf target/cadenza-store +
rebuild cleared it (the known store-poison trap, first time seeing the
content-address REFUSING face rather than a false red).

Pin candidates: 243 pool.
