# PR review comment — mirrored from GitHub PR #421 (Copilot inline)

- **PR:** #421 "fleet: forty-fifth batch (…, lsp find-references, …)" (MERGED)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs:1366` (`package_references_at` shadowing guard)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591882602
- **Link:** https://github.com/camshaft/cadenza/pull/421#discussion_r3591882602

## Comment (verbatim)
> The package-level shadowing guard is too permissive: `name_is_package_top_level = top_node.is_some() || resolves_to.is_some()` will be true for a reference to a *local* binder (because `ResolveOf` returns the local binder's defining occurrence). That allows `UsesOf{name}` to run and can leak references for an unrelated top-level symbol with the same spelling (the exact failure the single-buffer shadowing guard prevents).

## Liaison triage — CONFIRMED against trunk — partially reopens pr397 in the PACKAGE path
Confirmed in `package_references_at`: `let name_is_package_top_level = top_node.is_some() ||
resolves_to.is_some();` then `if !(is_entry_top_decl || name_is_package_top_level) { return Vec::new(); }`.
The `resolves_to.is_some()` disjunct is TRUE for a purely-local binder (ResolveOf resolves a local to its
own defining occurrence) — and the code's OWN comment admits "a purely-local binder still resolves, but
then `UsesOf{name}` returning the top-level's uses is the very leak — so also require the name to BE a
top-level symbol". But the code does NOT enforce that: OR-ing in `resolves_to.is_some()` re-admits the
shadowing-local case, so a cursor on a local binder that shadows a top-level of the same spelling passes
the guard and `UsesOf{name}` leaks the unrelated top-level's cross-file refs. This is the package-flavor
counterpart of the pr397 references_at shadowing bug (the single-buffer guard landed correctly; the
PACKAGE guard is incomplete). FIX: gate on the name genuinely being a package top-level symbol (e.g.
`is_entry_top_decl || top_node.is_some() || <resolves into an IMPORTED top-level def range>`), NOT any
resolve. cdz-tooling / LSP territory (v-cdz-tooling). Fix on `trunk`. Quote + link in queue file.
