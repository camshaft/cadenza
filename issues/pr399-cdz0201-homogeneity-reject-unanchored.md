# PR review comments — mirrored from GitHub PR #399 (Copilot inline)

- **PR:** #399 "fleet: twenty-fifth batch (collection-homogeneity CDZ0201 recovery, Binary matching chapter, RRB pin, LSP/syntax)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/infer.rs` (list-op reject @7329, Resolved::List check @9155)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590842024, 3590842040
- **Links:** https://github.com/camshaft/cadenza/pull/399#discussion_r3590842024 , #discussion_r3590842040

## Comments (verbatim)
> The list-op homogeneity reject is emitted via `Reject::coded(...)` without an origin. `collect` will stamp it on the overall application node, but for these cases the actionable locus is the mismatching argument (pushed/updated element or the second list in concat). Anchoring the reject improves diagnostic precision and matches the file's general "anchor the specific offending element" pattern.
>
> In the `Resolved::List` homogeneity check, the new CDZ0201 reject is unanchored. `collect` will stamp it on the list literal node, which makes the diagnostic highlight the whole list rather than the specific mismatching element. Anchor the reject to the offending element (`e`) so the message points at the minimal culprit (consistent with other diagnostics in this file).

## Liaison triage
The new CDZ0201 homogeneity rejects (list literal + list-ops append/replace/concat) are emitted via
`Reject::coded(...)` with NO origin, so `collect` stamps them on the whole application/list node instead
of the specific mismatching element/argument. This is a diagnostic-precision point — exactly the
"anchor the actionable locus, not the enclosing node" work the diagnostics vertient owns (v-diagnostics:
"say how to FIX it" / find a repair at one position not its twin). Anchor the reject to the offending
element `e` / the mismatching arg. Fix on `trunk`. Quotes + links in queue file.
