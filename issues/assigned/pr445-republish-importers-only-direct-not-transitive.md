# PR review comment — mirrored from GitHub PR #445 (Copilot inline)

- **PR:** #445 "fleet: sixty-fifth batch (…, lsp reverse-dep invalidation, …)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs:585` (`republish_importers_of`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592792961
- **Link:** https://github.com/camshaft/cadenza/pull/445#discussion_r3592792961

## Comment (verbatim)
> `republish_importers_of` only checks an importer's *direct* `(import ...)` declarations against `changed`, but `compute_diagnostics` uses `closure::load` which follows *transitive* imports. If A imports B and B imports C, editing/opening/closing C should also re-publish A; currently it won't, so diagnostics can remain stale for multi-hop dependencies.

## Liaison triage — CONFIRMED against trunk
Confirmed in lsp.rs `republish_importers_of`: it filters importers by whether any of their DIRECT
`closure::declared_import_paths` resolves to `changed` (line 578: `declared_import_paths(&arenas).iter().
any(|name| resolve_import_file(dir, name) == changed)`). But `compute_diagnostics` types a doc against
the TRANSITIVE closure via `closure::load`. So for A→B→C, editing C re-publishes B (direct importer) but
NOT A (only a transitive importer) → A's in-editor diagnostics go stale on a multi-hop dependency edit.
FIX: walk the transitive importer graph (or re-publish any open doc whose loaded closure CONTAINS
`changed`), not just direct importers. cdz-tooling / LSP (v-cdz-tooling). Fix on `trunk`. Quote + link
in queue file.
