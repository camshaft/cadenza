# PR review comments — mirrored from GitHub PR #408 (Copilot inline)

- **PR:** #408 (MERGED)
- **Files:** `implementation/compiler-ml/src/quote-build.cdz:17`, `spec/semantics/05-compound-types.sexp:8671`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591225622, 3591225642
- **Links:** https://github.com/camshaft/cadenza/pull/408#discussion_r3591225622 , #discussion_r3591225642

## Comments (verbatim)
> This module now matches on `Ast.Str`/`Ast.Bool`/`Ast.Float`, but the header comment still claims the built-in `Ast` has exactly three variants (`Int`/`Name`/`List`). Updating the header avoids misleading readers about what `quote` handles.
>
> This new `Map.to-list` case doc says the list length equals `Map.size`, but the prelude surface name is `Map.len` (with `Map.size` being the pre-cutover name). Updating the doc keeps the corpus text consistent with the current prelude surface.

## Liaison triage
Two low-severity doc/comment staleness items:
- `quote-build.cdz:17` header says `Ast` has three variants (Int/Name/List) but the module now handles
  `Ast.Str`/`Ast.Bool`/`Ast.Float` too — stale header.
- `05-compound-types.sexp:8671` `Map.to-list` case doc references `Map.size`, the PRE-cutover name (now
  `Map.len` after the collection-op naming cutover, cf. pr403). Corpus-text consistency.
Both are corpus/compiler-ml doc fixes → route to `corpus-bugfix` PM (low priority). Fix on `trunk`.
Quotes + links in queue file.
