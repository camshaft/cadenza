# No prefix unary-minus operator: `-x` fails to parse, forcing the `0 - x` workaround

**Reported by operator, 2026-07-15, via concierge.** The operator noticed agents repeatedly writing
`0 - 1` (and `0 - x`) to get negative values and correctly suspected they're working around a
parser/operator gap. Confirmed.

## Diagnosis: it's NOT about negative literals — those work

- `-1`, `-1.5`, `(-1, -2)`, `let x = -1 in x`, `3 * -2` all **check clean**. `-1` converts to
  `(def (main) -1)`. So a negative *literal* parses fine.
- The gap is **prefix unary minus applied to an EXPRESSION** (a name, a call, a parenthesized expr):

  | input (ML) | result |
  |---|---|
  | `let x = 5 in -x` | ❌ `error: expected an expression` (at the `x` after `-`) |
  | `let x = 5 in -(x + 1)` | ❌ `error: expected an expression` |
  | `fn (x) => -x` | ❌ `error: expected an expression` |
  | `let x = 5 in 0 - x` | ✅ (the workaround) |

- s-expr surface confirms `-` is strictly **binary**: `(- 1)` → `CDZ0201: - takes exactly 2 operands`;
  `(- (+ 2 3))` same. There is no unary-negate prim exposed. The `check` heuristic suggests `Neg`,
  but `(Neg 5)` is read as a **nullary sum-variant constructor** (`a nullary variant takes the unit
  value, but a payload of type Int64 was applied`) — a RED-HERRING hint, not a real negation op.

So negating any non-literal value requires spelling it `0 - x`. That's the workaround the operator
is seeing agents reach for.

## Why this matters
`0 - x` is a papering-over that: (1) reads badly, (2) may interact with unit/overflow/exactness
edge cases differently from a real negation, and (3) doesn't exist for the `fn (x) => -x` /
prefix-position case at all (that just fails to parse). Agents adopting it is a signal the surface
is missing an expected operator.

## Likely area / fix
- ML surface: add **prefix unary minus** to the expression grammar so `-<expr>` parses (with correct
  precedence — tighter than binary `+`/`-`, and NOT ambiguous with binary minus / with negative
  literals which already lex as a signed literal). Decide the disambiguation: `a - b` (binary) vs
  `a (-b)` — follow the spec's operator table.
- Core/s-expr: expose a real unary-negate (either a dedicated prim, or lower `-<expr>` to
  `(- 0 <expr>)` / an `Int.neg`-style op) — and make the `check` heuristic stop suggesting the
  `Neg` sum-variant constructor for a subtraction/negation context (that hint is wrong).
- Confirm float negation (`-x` where `x : Float64`) and unit-quantity negation behave.

## Acceptance
- `let x = 5 in -x` checks clean and evaluates to `-5`; `-(x + 1)`, `fn (x) => -x` parse & run.
- Existing negative-literal cases (`-1`, `3 * -2`) still pass; `a - b` binary subtraction unchanged.
- Fix or remove the bogus `did you mean Neg` heuristic for negation contexts.
- Add corpus regression cases: prefix-negate of a name, of a paren expr, in a lambda body, float,
  and a quantity.

<!-- RESOLVED 2026-07-15 (trunk@ab8304572): prefix unary minus -e = arity-1 (- e), type-directed 0-e across int/float/rational/bigint/qty. Migrated to 06-numeric-model.sexp (11 cases). -->
