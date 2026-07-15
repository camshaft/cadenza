# PR review comment — mirrored from GitHub PR #386 (Copilot inline)

- **PR:** #386 "fleet: thirteenth batch (sum-match miscompile fix, multi-param closure mono, Ast.Str, CSE)" (MERGED)
- **File:** `guide/src/content/chapters/Functions.tsx:126`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589903889
- **Link:** https://github.com/camshaft/cadenza/pull/386#discussion_r3589903889

## Comment (verbatim)
> This guide text says multi-argument callbacks still require type annotations, but this PR adds a semantics case and compiler fix for inferring an unannotated two-argument closure through a generic recursive HOF. The guide should be updated so it doesn't teach an outdated limitation.

## Liaison triage
Guide text is now stale: this batch landed inference for an unannotated two-arg closure through a
generic recursive HOF, but the Functions chapter still teaches that multi-arg callbacks require type
annotations. Doc-update in guide territory (v-guide). Route as a note. Fix on `trunk`.
