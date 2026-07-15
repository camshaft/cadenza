# PR review comment — mirrored from GitHub PR #379 (Copilot inline)

- **PR:** #379 "fleet: sixth batch (breaker corpus, parser pin, syntax round-trip coverage)" (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/codec.rs:678`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589447443
- **Link:** https://github.com/camshaft/cadenza/pull/379#discussion_r3589447443

## Comment (verbatim)
> The doc comment says this helper returns `true` iff `bytes` was accepted, but the function also asserts the canonical fixed-point property and will panic for an accepted input that violates it. Clarifying that behavior would make the contract accurate for callers/readers.

## Liaison triage
Second docstring-accuracy point on the SAME `assert_canonical_fixed_point` helper already flagged to
`v-syntax` (see queue/pr377-codec-assert-canonical-docstring-returns-bool-not-count.md, comment
3589313820). This one adds that the helper PANICS on a canonical-fixed-point violation, which the
"returns true iff accepted" contract omits. Fold into the same v-syntax docstring fix. Fix on `trunk`.
