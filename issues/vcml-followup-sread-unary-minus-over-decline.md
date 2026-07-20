# FOLLOW-UP (v-compiler-ml, self): sread s-expr reader over-declines unary minus `(- x)`

Found 2026-07-20 (trunk 7f6f074e3) while probing reader gaps after the negative-literal fix.

## The gap
The s-expr reader (`sread.cdz`, the run-ml / run-emitted / W4-differential path) DECLINES a unary-minus
form `(- x)` that the reference RUNS:

| program        | run-ml (sread) | reference (rcdzc, wrapped) |
|----------------|----------------|-----------------------------|
| `(- 5)`        | declined       | -5                          |
| `(- (+ 2 3))`  | declined       | -5                          |
| `(- (- 5))`    | declined       | 5                           |

`read-app-or-bin` sends a `-` head with ONE operand to `read-bin-form`, which expects two operands → declines.

## Why it's a real gap (not a decline-by-design)
- The reference supports unary minus (verified: `(- 5)`→-5, `(- (+ 2 3))`→-5, `(- (- 5))`→5).
- The corpus uses `(- n ...)` extensively across many files.
- The TOKEN-based parser ALREADY supports it: `parse-db.cdz` desugars `TNeg` → `0 - x` = `NBin(45, NLit 0, x)`
  (line 173-176), gate-pinned by `pd-unary-minus-desugars-to-sub` + eval-db `run([neg(), n(5)])`→-5,
  `run([neg(), neg(), n(5)])`→5. Only the S-EXPR reader lacks the parallel handling.

## The fix (when unblocked)
In `sread.cdz` `read-app-or-bin` (or `read-bin-form`): when the head sym is `-` (op 45) and there is EXACTLY
ONE operand before the `)`, read it as unary negation → `NBin(45, NLit 0, operand)` (mirroring parse-db's
`TNeg` desugar — same Core shape, no new node). Keep the two-operand `(- a b)` = binary subtraction path.
Boundary: `(- )` (no operand) declines; `(- a b)` stays subtraction.

## Why HELD (not fixed this tick)
`sread.cdz` is in my currently-pending MR `7b1bffcf8` (the negative-LITERAL `-N` reader fix). Editing it now
would stack a same-file commit on an unlanded MR (tangles on reject/reorder — the same-file 1-at-a-time
cadence). PICK THIS UP once 7b1bffcf8 lands + I sync clean. Add: 2 reader @tests (sread) + 2 e2e run-src
@tests (sread-eval), verify e2e vs reference (`(- 5)`→-5, `(- (- 5))`→5, `(- a b)` still subtraction).
