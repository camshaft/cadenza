# PR review comment — mirrored from GitHub PR #377 (Copilot inline)

- **PR:** #377 (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/codec.rs:677`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589313820
- **Link:** https://github.com/camshaft/cadenza/pull/377#discussion_r3589313820

## Comment (verbatim)
> This docstring says the helper "Returns the accepted count", but the function actually returns a boolean. The mismatch makes the contract unclear for future refactors.

## Liaison triage
Confirmed against trunk: `fn assert_canonical_fixed_point(bytes: &[u8]) -> bool` — the docstring's
last sentence claims it "Returns the accepted count" but the signature returns `bool`. Genuine
doc/code contract mismatch in `cadenza-syntax` territory (`v-syntax`). Comment-level fix; route as a
note to the syntax vertical.

<!-- RESOLVED 2026-07-15: fixed on trunk in fb141f13 (v-syntax depth-cap slice, merged @06dadcb7). Docstring now states the real bool contract + panic-on-violation. -->
