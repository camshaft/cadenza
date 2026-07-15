# PR review comment — mirrored from GitHub PR #377 (Copilot inline)

- **PR:** #377 "fleet: fourth batch (bin width-typed segments, hardened codec::decode)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/infer.rs:8786`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589313869
- **Link:** https://github.com/camshaft/cadenza/pull/377#discussion_r3589313869

## Comment (verbatim)
> The CDZ0203 diagnostic suggests using "<Type>.wrap"/"<Type>.of" based on `want.render_name()`. For non-preinstalled widths (e.g. `(UInt 4)`), the rendered name is "UInt4" but that identifier may not exist in scope, so the example can be syntactically invalid and misleading. Prefer wording that points to the required width module's `wrap`/`of` without assuming a globally-available name.

## Liaison triage
Diagnostic-actionability bug: CDZ0203's suggested fix names an identifier (`UInt4.wrap`) that may not
be in scope for non-preinstalled widths, so the "how to fix" text can be syntactically invalid. This
is squarely the diagnostics vertical's territory (`v-diagnostics` — "say how to FIX it" without
suggesting an unspellable name). Route as a code/correctness point to `corpus-bugfix` PM (diagnostics
worktree can pick it up); fix belongs on `trunk`.
