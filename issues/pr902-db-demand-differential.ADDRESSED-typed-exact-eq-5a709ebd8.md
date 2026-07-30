# PR#902 review comment — demand-vs-walk differential's typed-tag collapses TIntW width/sign + TSum identity (v-compiler-ml)

Mirrored from GitHub PR#902 review comment (Copilot), id `3677998528`.
File: `implementation/compiler-ml/src/db-demand.cdz:309` — compiler-ml PORT test code → v-compiler-ml
(port owner). Blame `0dcf2b491` "compiler-ml(db-demand): item-4 real query-memo — differential: demand
producers ≡ whole-tree walk + PR#900 comment fix" (same MR that addressed the PR#900 comment).

## Comment (verbatim)

- (id 3677998528, db-demand.cdz:309) "The new demand-vs-walk differential only compares a coarse
  `typed-tag`, which currently collapses all `TIntW(_, _)` values to the same tag (and all `TSum(_)` to
  the same tag). This can let the tests pass even if the two producers disagree on grounded integer
  sign/width or on which sum type was inferred, weakening the intended regression pin."

## Liaison verification (confirmed on trunk 8f6f82404)

`typed-tag` (db-demand.cdz:298-303): `TIntW(_, _) => 1 | TBool => 2 | TErr => 3 | TFn(_, _) => 4 | TSum(_)
=> 5`. Both `TIntW`'s (width, sign) and `TSum`'s payload are DISCARDED — every int width/sign maps to `1`,
every sum to `5`. The `agree` differential (:307-309) compares `typed-tag(ground-deferred(demandFact)) ==
typed-tag(walkFact)`. So if the demand producer and the whole-tree walk disagreed on a grounded integer's
SIGN or WIDTH (e.g. `TIntW(64, signed)` vs `TIntW(32, unsigned)`) or on WHICH sum type was inferred, the
tags would still both be `1` (or `5`) and `agree` returns true — the differential passes despite a real
producer divergence. Weakens the regression pin exactly where a memo/walk mismatch is most likely (width
inference, sum selection). Fix (Copilot's, sound): make `typed-tag` (or the `agree` comparison) carry the
`TIntW` width+sign and the `TSum` identity — compare the full grounded Typed structurally, or extend the
tag to encode (width, sign) and the sum's decl/name. Test-coverage; behavior-neutral to the compiler.

Owner: **v-compiler-ml** (compiler-ml port test, their `0dcf2b491` differential). Tighten `typed-tag`/`agree`
to distinguish int width/sign + sum identity.
