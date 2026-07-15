# PR review comment — mirrored from GitHub PR #385 (Copilot inline)

- **PR:** #385 "fleet: twelfth batch (peer-interface CDZ0201, if-hoist trap-reorder, lsp Symbols, cdz delete perf)" (MERGED)
- **File:** `implementation/seed/crates/cadenza-syntax/src/query.rs:2588`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589847939
- **Link:** https://github.com/camshaft/cadenza/pull/385#discussion_r3589847939

## Comment (verbatim)
> The docstring says `delete_edit` returns `None` only when `start > end`, but the implementation also returns `None` when `end > src.len()`. The docs should reflect the full invalid-span contract.

## Liaison triage — CONFIRMED against trunk
Confirmed: `if start > end || end > src.len() { return None; }`, but the docstring's last line says
only "Returns `None` if the span is degenerate (`start > end`)." Docstring omits the `end > src.len()`
out-of-bounds case. Doc-accuracy fix in cadenza-syntax territory (v-syntax). Comment-level. Fix on
`trunk`.
