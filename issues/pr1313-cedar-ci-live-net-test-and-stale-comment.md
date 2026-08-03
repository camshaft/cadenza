# PR #1313 review comments — .github/workflows/checks.yml (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1313 (PR: "cand: v-agent-harness-host — ab6dd0198").
This is the follow-up to my #1295 cluster — it ADDS the cedar-guest CI build job I flagged as missing.
Both comments verified against the diff.

## 1. `live-net` feature only runs clippy, not test, despite the comment (amazon-q, checks.yml:319) — CI
> Comment states "Lint AND (where a runner has egress) test it" but only clippy runs for the
> `live-net` feature. Add `cargo test --features live-net` to match the documented intent and the
> pattern used in the `cdz-kernel` job above (lines 276-277), which tests both clippy and actual
> tests for the `live-exec` feature.

Verified (diff line 49 is clippy-only). Either add `cargo test --features live-net` to match the
"Lint AND … test it" comment + the sibling `live-exec` pattern, or soften the comment to "lint only
(egress-gated tests run elsewhere)". (Only add the test if a CI runner actually has egress — otherwise
the comment is the thing to fix.)

## 2. "Guard against a STALE fixture" comment overclaims — step only rebuilds+validates from source (Copilot, checks.yml:303) — doc/CI
> The inline comment claims this step guards against a *stale* committed fixture, but the script only
> rebuilds from source and validates the newly produced component. Without comparing against the
> committed fixture (or its extracted interface), this check won't detect that the committed artifact
> is out of date; it only detects that the source still produces a valid component.

Verified (diff lines 30-40 explicitly do "NOT a byte-diff" + rebuild-and-validate). The step catches
"source no longer builds a valid component", NOT staleness of a committed artifact — reword the comment
to match (or add an interface comparison if staleness-detection is actually wanted). NB: this ties back
to #1295 point 3 — there's no committed cedar component artifact, so "stale committed fixture" doesn't
even apply yet; the wording should reflect that.
