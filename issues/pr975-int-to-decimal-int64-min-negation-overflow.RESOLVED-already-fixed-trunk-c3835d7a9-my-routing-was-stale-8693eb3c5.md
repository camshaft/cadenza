# PR#975 review comment — int-to-decimal negates for Int64.min → overflow breaks "Text never declines" (⚠ v-compiler-ml)

Mirrored from GitHub PR#975 review comment (Copilot), id `3694618529`.
File: `implementation/compiler-ml/src/emit-db.cdz:336` — compiler-ml PORT source → v-compiler-ml. Blame
`265fe4c90` "compiler-ml: add Target.Text backend (Core→String render) — the 2nd backend".

⚠ CORRECTNESS (Int64.min negation-overflow) — flagged for v-compiler-ml.

## Comment (verbatim)

- (id 3694618529, emit-db.cdz:336) "`int-to-decimal` negates the input (`0 - n`) to compute the
  magnitude. This overflows for `Int64.min` (and can either trap or wrap back to the same negative value),
  which breaks the claim that the Text backend 'never declines' and can also produce an incorrect render
  (e.g., just '-'). A common fix is to do digit extraction while keeping the working value negative, so
  `Int64.min` is representable and `% 10` stays in range."

## Liaison verification (confirmed on trunk 8693eb3c5)

`int-to-decimal` (emit-db.cdz:334-336): `if n == 0 then "0" else (if n < 0 then String.concat("-",
int-to-decimal-go(0 - n, "")) else int-to-decimal-go(n, ""))`. For `n = Int64.min` (-9223372036854775808),
`0 - n` = +9223372036854775808 which is NOT representable in i64 (max is ...807) → overflow: traps (checked
arith) or wraps back to Int64.min (which is `< 0`, so `int-to-decimal-go` then does `n % 10` on a negative
and `n / 10` never reaches 0 the expected way → wrong/`-`-only render). Either way it breaks the render-db
doc's explicit claim that the Text backend "NEVER declines (every well-formed Core renders)" (emit-db.cdz
render-core doc). A `Core.CNum(Int64.min, …)` — a representable, well-formed Core literal — would trap or
mis-render. Fix (Copilot's, the standard one): extract digits while keeping the working value NEGATIVE
(negate each `n % 10` digit, recurse on `n / 10` staying ≤ 0) so `Int64.min` never needs its
non-representable positive — the magnitude is built without ever negating the whole value. Correctness on
the "never declines" invariant.

Owner: **v-compiler-ml** (compiler-ml port `emit-db.cdz` Text backend, `265fe4c90`). Rewrite
`int-to-decimal` to extract digits from the negative side so `Int64.min` renders (no `0 - n` overflow);
add an `Int64.min` render witness.
