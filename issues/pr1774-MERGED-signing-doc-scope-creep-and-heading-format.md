# PR #1774 review comments — design/agent-harness-signing.md (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1774 (MERGED — the fix for my #1764 residual-stale-sync-doc
finding). Good fix, but it ALSO bundled a new design doc.

## 1. Scope-creep: a "cdz-kernel doc cleanup" PR also introduces a new signing design proposal (Copilot, agent-harness-signing.md:6) — process
> The PR is titled a cdz-kernel doc cleanup for stale sync-API refs, but it also introduces a new design
> proposal document about agent-harness global-store signing. Update the title, or move the design doc to
> its own PR.

The #1764-review fix (stale sync-API refs) is good, but bundling a NEW signing design proposal under that
doc-cleanup title is scope-creep (recurring: #1747 namespace, #1768 BlobStore-async). Retitle/changelog to
disclose the new design doc, or split. LOW-MED/process (already merged → metadata-honesty).

## 2. Heading has unbalanced parens/colon + trailing ")" (Copilot, agent-harness-signing.md:140) — doc/format
> The bolded lead-in has unbalanced parentheses/colon and an extra closing ")", making the heading hard to
> read and possibly mis-rendering.

LOW/format — balance the parens/colon in the bolded heading. Fix-forward.
