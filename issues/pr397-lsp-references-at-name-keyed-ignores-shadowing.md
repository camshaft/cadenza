# PR review comment — mirrored from GitHub PR #397 (Copilot inline)

- **PR:** #397 "fleet: twenty-third batch (quantity×scalar miscompile fix, LSP completion, cdz uses test)" (MERGED)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs:807`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590717557
- **Link:** https://github.com/camshaft/cadenza/pull/397#discussion_r3590717557

## Comment (verbatim)
> `references_at` calls `UsesOf { name }`, but `UsesOf` only indexes references to *top-level* defs/sum types with that name. If the cursor is on a local binder that shadows a top-level symbol of the same spelling, this will incorrectly return references to the unrelated top-level symbol. Guard `UsesOf` by first resolving the cursor via `ResolveOf` and ensuring it matches the `Symbols` entry for that top-level name; otherwise return an empty reference list (until a node-id-keyed uses query exists).

## Liaison triage — CONFIRMED against trunk
Confirmed in lsp.rs `references_at`: it takes the name atom under the cursor (`arenas.as_name(node)`)
and calls `UsesOf { name }` keyed PURELY by the name string — no resolution check. So with the cursor
on a LOCAL binder (a `let`/param/match binder) that shadows a top-level def/sum-type of the same
spelling, find-references returns the unrelated TOP-LEVEL symbol's uses. Real correctness bug in
textDocument/references. Reviewer's fix is sound: resolve the cursor via `ResolveOf` first and only run
`UsesOf` when it matches the `Symbols` entry for that top-level name; else return empty (until a
node-id-keyed uses query exists). New `cdz` LSP, no LSP vertical → route to `corpus-bugfix` PM. Fix on
`trunk`. Quote + link in queue file.
