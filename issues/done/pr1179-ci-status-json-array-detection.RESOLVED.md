# PR #1179 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1179
(PR: "cand: v-fleet-tooling — priority #1 (executor?)"). Refines the `ci-status` primitive first
flagged on #1071.

## Non-zero-exit classification uses `starts_with('[')` instead of real JSON parse (Copilot, fleet.rs:6531) — robustness
> The non-zero-exit handling is documented as distinguishing "gh errored" vs "gh reported
> red/pending" by whether stdout is a JSON array, but the code only checks
> `trim_start().starts_with('[')`. Consider actually attempting to parse stdout as a JSON array (in
> the failure case) so behavior matches the comment and avoids any accidental '['-prefixed non-JSON
> output being treated as checks output.

Follows the #1071 thread (that one moved `parse_gh_checks` onto serde_json). Same idea for the
non-zero-exit branch: the doc says "is it a JSON array?" but the code only sniffs a leading `[`, so a
`[`-prefixed non-JSON stderr/stdout line could be misread as checks output. Attempt an actual
`serde_json` array parse in the failure case so behavior matches the doc.
