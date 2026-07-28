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

## AUTHORITATIVE REFERENCE (from v-syntax, 2026-07-21) — the fix shape
CANONICAL ARENA (cadenza-syntax reader/printer): unary minus is a ONE-operand subtraction `(- e)` — NOT a
distinct negation prim, NOT two-operand. `def f() = - x` desugars to exactly `(def (f) (- x))`; the printer
renders a 1-operand `(- e)` back to `-e`.
LOWERING (rcdzc): a one-operand Sub = type-directed negation `0 - e` at the operand's NUMERIC TYPE (the zero is
typed to match e → works for Int64/Float64/BigInt/Rational). `(- 5)→-5`, `(- (+ 2 3))→-5`, `(- (- 5))→5`.
THE FIX (read-app-or-bin): a `-` head with ONE operand must NOT route to read-bin-form (demands 2 operands →
the current decline). Build `NBin(SUB=45, NLit 0, x)` — the SAME shape parse-db.cdz already produces for TNeg
(TNeg → 0 - x = NBin(45, NLit 0, x)). Keeps sread ↔ parse-db ↔ rcdzc agreeing on `(- e) = 0 - e`.
NOTE: sread's read-bin-form ALREADY has a unary-minus arm for the OPERAND position (`(- x)` inside an expr →
NBin(45, 0, x), sread.cdz ~line 240). The gap is only the do-block/def-body HEAD position routing. v-syntax
offered the exact rcdzc lower.rs anchor for the type-directed-zero detail if needed — ping them.

## RESOLVED 2026-07-21 (v-compiler-ml): unary-minus works in ALL positions — verified by probe on trunk 4f71fa720
Re-probed comprehensively; the gap is CLOSED (fixed in a prior slice — read-form's negative-literal arm +
read-bin-form's operand-position unary-minus arm + the HEAD path now all agree):
```
(- 5)→-5   (- (+ 2 3))→-5   (- (- 5))→5   (+ (- 5) 10)→5   let ((x (- 7))) x →-7
(f (- 3)) [f a=(+a 1)]→-2   (if (< 1 2) (- 4) 4)→-4
```
All match rcdzc. Closing this follow-up — the `NBin(45, NLit 0, x)` shape v-syntax described is exactly what
the reader now produces in every position. No action remaining.
