# PR review comments — mirrored from GitHub PR #412 (Copilot inline)

- **PR:** #412 (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/prelude.rs:321` + `:323`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591415967, 3591415984
- **Links:** https://github.com/camshaft/cadenza/pull/412#discussion_r3591415967 , #discussion_r3591415984

## Comments (verbatim)
> This duvet citation anchor appears to be incorrect: the spec heading is "An Open Sum's Payload May Be Schema-Typed", whose slug is `#an-open-sums-payload-may-be-schema-typed` (not `an-open-sum-s-…`).
> Second `//=` duvet citation uses the same incorrect anchor slug.

## Liaison triage — CONFIRMED against trunk
Confirmed: prelude.rs:320 + :322 both cite
`//= spec/capabilities/type-system.md#an-open-sum-s-payload-may-be-schema-typed`, but the heading in
type-system.md:210 is "### An Open Sum's Payload May Be Schema-Typed", which GitHub/duvet slugs to
`#an-open-sums-payload-may-be-schema-typed` (the apostrophe-s collapses to `s`, NOT `-s-`). So both
`//=` anchors are wrong and duvet can't resolve them → a broken/stranded citation (the duvet-coverage
gate should catch this). Duvet-citation territory (v-duvet-coverage). FIX: correct both anchor slugs to
`#an-open-sums-payload-may-be-schema-typed`. Fix on `trunk`. Quotes + links in queue file.
