# PR review comment — mirrored from GitHub PR #397 (Copilot inline)

- **PR:** #397 (MERGED)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs:836`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590717578
- **Link:** https://github.com/camshaft/cadenza/pull/397#discussion_r3590717578

## Comment (verbatim)
> The doc comment says completions are total on non-parsing buffers and still return a partial candidate set, but the implementation returns an empty vec when `parse_surface` fails. Update the comment to match the actual behavior (especially for malformed s-expr buffers where parsing can hard-fail).

## Liaison triage — CONFIRMED against trunk
Confirmed: `completions_at`'s doc says "TOTAL: a buffer that does not parse yields whatever partial set
the queries produce, never a panic", but the body is `let Ok((arenas, spans, _errors)) =
parse_surface(text, is_ml) else { return Vec::new(); }` — on a parse failure it returns an EMPTY vec,
not a partial set. Doc/code mismatch (the "partial set" claim only holds when parse SUCCEEDS with
recoverable errors). Either fix the comment to say completions are empty on a hard parse failure, or
make it actually return a partial set. Comment-level (unless the partial-set behavior was intended).
New `cdz` LSP, no LSP vertical → route to `corpus-bugfix` PM alongside the references_at fix. Fix on
`trunk`. Quote + link in queue file.
