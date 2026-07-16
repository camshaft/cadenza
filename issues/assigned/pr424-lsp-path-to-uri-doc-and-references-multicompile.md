# PR review comments — mirrored from GitHub PR #424 (Copilot inline)

- **PR:** #424 "fleet: forty-eighth batch (…, lsp shadowing fix, …)" (MERGED)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs` (path_to_uri doc @1196, package_references_at @1425)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592036153, 3592036167
- **Links:** https://github.com/camshaft/cadenza/pull/424#discussion_r3592036153 , #discussion_r3592036167

## Comments (verbatim)
> The docstring says [path_to_uri] returns `None` for a non-absolute path, but the implementation explicitly builds a best-effort `file:///...` URI for relative paths. This mismatch can mislead callers about when `None` is possible and what happens for relative paths.
>
> `package_references_at` currently runs multiple full `rcdzc::compile` invocations per references request (at least one each for `Symbols`, `ResolveOf`, and `UsesOf`). That can make "find references" noticeably slower on larger packages. Consider batching these sidecar queries into a single compile (one `sidecar::encode(&[Query::Symbols, Query::ResolveOf{…}, Query::UsesOf{…}])`) and reading all needed artifacts plus the link-map from the same `compiled` result.

## Liaison triage — CONFIRMED against trunk
- path_to_uri doc: says `None` for a non-absolute path but the body builds a best-effort `file:///…` URI
  for a relative path — doc/behavior mismatch (relates to the pr419/pr421 path_to_uri under-encoding
  findings; same helper).
- `package_references_at` perf: runs SEPARATE full `rcdzc::compile` passes for Symbols, ResolveOf, and
  UsesOf per find-references request → 3× compile on larger packages. Batchable into ONE
  `sidecar::encode(&[Query::Symbols, Query::ResolveOf{…}, Query::UsesOf{…}])` compile, reading all
  artifacts + the link-map from the same result. Real perf win on the new cross-file find-references.
Both cdz-tooling / LSP (v-cdz-tooling). Fixes on `trunk`. Quotes + links in queue file.
