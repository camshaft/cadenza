# PR #2178 review — reducer_b3.cdz (v-harness-bootstrap) — OPEN — test-coverage [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2178 (B2/B3 coverage-pin FORMATTING — #2167 review nits, re-cut
over the landed pins; the fix-forward for MY #2167). Copilot 1 inline — a coverage gap.

## the B3 http-effect pins cover kind/target/correlation but never assert `payload == None()`; since `http-effect()` sets `payload = None()`, a silent change to `Some(_)` wouldn't be caught (B2 explicitly pins payload) (Copilot, reducer_b3.cdz:130) — test-coverage [VERIFIED, LOW]
> The B3 http-effect pinning tests cover kind/target/correlation but never assert that `payload` remains
> `None()`. Since `http-effect()` currently sets `payload = None()`, a silent change to `Some(_)` would
> not be caught here (and B2 explicitly pins payload). Consider adding a dedicated payload pin test to
> fully lock down the effect-request fields.

VERIFIED in the #2178 diff: B2 has a dedicated `b2_effect_payload_is_none` pin (diff:25-31 — `match
r.payload with | None() => unit | Some(_) => trap("B2's effect payload must be None")`). The B3 http-effect
pins (b3_http_effect_kind_is_http / _target_and_correlation_pinned) assert kind/target/correlation but
have NO analogous payload assertion. So B3's `http-effect()` sets `payload = None()` today, but a silent
regression to `Some(_)` would pass B3's pins (they don't look at payload) while B2 would catch the same
change for its effect — an asymmetric coverage gap. LOW/test-coverage (no current bug; the pin set just
doesn't fully lock the effect-request fields for B3). Fix per Copilot: add a `b3_http_effect_payload_is_none`
pin mirroring B2's, so all effect-request fields (kind/target/correlation/payload) are locked for the
flagship B3 reducer. v-harness-bootstrap owns the reducer fixtures. PR OPEN → foldable (this PR is already
re-cutting the B2/B3 pins for my #2167 formatting, so it's the natural place to add the payload pin).
Reminder: fixture edits must pass the ML round-trip, not just gate. (On the fix-forward for my #2167 — a
coverage completeness nit on the same pins.)
