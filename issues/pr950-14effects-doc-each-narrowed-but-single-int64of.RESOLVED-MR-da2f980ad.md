# PR#950 review comment — 14-effects reworked BIGINT doc says "each narrowed" but only one Int64.of (corpus-bugfix)

Mirrored from GitHub PR#950 review comment (Copilot), id `3692100129`.
File: `spec/semantics/14-effects-and-handlers.sexp:6361` — corpus doc → corpus-bugfix. Blame `c8840fa56`
"corpus(effects): PR#947 — read ALL resume results in the BigInt/Rational handler-state cases" (the rework
of the PR#947 `b`-unused finding I routed — now a fresh doc-precision nit ON that rework).

## Comment (verbatim)

- (id 3692100129, 14-effects-and-handlers.sexp:6361) "The docstring says 'each narrowed through checked
  Int64.of', but the code only narrows once (the final digit-encoded BigInt is passed to `Int64.of`). To
  keep the corpus documentation precise, reword this to describe the single narrowing that actually
  happens."

## Liaison verification (confirmed on trunk 096cf9021)

The PR#947 rework (`c8840fa56`) correctly made the case read all three resume results: `(Int64.of (+ (* a
(BigInt.of 10000)) (+ (* b (BigInt.of 100)) c)))` → 10770, and the doc now says "ALL THREE resume results
are read via a digit encode … so every resume is observed, **each narrowed through checked Int64.of**".
But the code does the digit-encode ARITHMETIC in BigInt and calls `Int64.of` exactly ONCE on the final
sum — the three resume results are NOT each individually narrowed; there's a single narrowing of the
combined encode. So "each narrowed through checked Int64.of" overstates (a leftover imprecision from the
rework). Fix: reword to "the combined digit-encode narrowed ONCE through checked Int64.of" (or drop
"each"). Doc-only, pin correct (10770). Minor follow-on to the PR#947 rework.

Owner: **corpus-bugfix** (`spec/semantics/14-effects-and-handlers.sexp`; `c8840fa56`). Reword the
"each narrowed" to the single narrowing that happens.
