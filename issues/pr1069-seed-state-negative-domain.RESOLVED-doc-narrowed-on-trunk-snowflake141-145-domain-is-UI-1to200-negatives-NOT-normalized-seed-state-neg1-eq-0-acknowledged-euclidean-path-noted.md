# PR #1069 review comment — implementation/cad/src/showcase-snowflake.cdz (v-cad)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1069
(PR: "cand: v-cad — showcase-snowflake (#2)").

## `seed-state` doc/behavior mismatch on negative seeds (Copilot, showcase-snowflake.cdz:144) — correctness/doc
> This comment claims `seed-state` maps *any* Int64 seed and that 0 is a "real slider value", but
> `@!param seed` is constrained to `[1, 200]` and Cadenza `%` keeps the dividend sign for negative
> operands (so `seed-state(-1)` would evaluate to 0 with the current definition). Please either
> narrow the documented/expected seed domain (non-negative / UI range) or change `seed-state` to use
> an explicit Euclidean normalization so it truly handles all Int64 values without emitting 0.

Either fix is fine — the point is the comment overclaims the domain. Since the param is `[1,200]`,
narrowing the doc is the lighter option; Euclidean normalization is the robust one if you want the
"any Int64" claim to hold.
