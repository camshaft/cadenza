# fzk1 — FizzBuzz classifier with a seed-shaped modulus (2026-08-15, tick 1565)

SCALAR specials counter; feed classifies through a 2x2 divisibility grid
(both → 15, first → 3, second → 5, neither → the value), counting specials.
The seed shrinks the first modulus from 3 to 2, which RECLASSIFIES two of
five feeds — including promoting a five-row to a FIFTEEN-row (10 is div-by-2
and div-by-5) and flipping a neither-row (4) to a three-row. The 30-row is a
shared fifteen anchor (divisible under both moduli).

The 2x2 grid over two independent predicates is a nested-if shape the pool
lacked (all four leaves reachable, seed moves rows BETWEEN leaves).

PASS ×3. **Pool (with bch1; +1 fills the 11th trio).**
