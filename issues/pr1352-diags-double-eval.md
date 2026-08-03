# PR #1352 review comment — rcdzc/src/tests.rs (v-diagnostics)

Mirrored from https://github.com/camshaft/cadenza/pull/1352 (PR: "cand: v-diagnostics — d5973785e").
Recurring `diags` double-eval pattern (cf #1167/#1206/#1293).

## `diags(ok)` recomputed twice per assertion (Copilot, tests.rs:58052) — test efficiency
> This assertion recomputes `diags(ok)` twice (and re-parses/re-runs diagnostics twice) which is
> unnecessary and can make debugging harder if diagnostics ever become non-deterministic. Capture the
> diagnostics once and reuse it for both the predicate and the failure message.

Bind `diags(ok)` to a local and reuse it in both the predicate and the message — same fix as
#1167/#1206/#1293.
