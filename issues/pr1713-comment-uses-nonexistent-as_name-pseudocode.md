# PR #1713 review comment — rcdzc/src/link.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1713 (follow-up to #1700 — the comment reword on my #1683 fix).

## Reworded comment uses pseudo-code `tail.first().as_name()` — no such method (Copilot, link.rs:779) — doc/accuracy
> The comment still uses pseudo-code `tail.first().as_name()`, but there is no `as_name()` on `StructId`
> here (the actual API is `ast.as_name(s)`), which undermines making the comment less drift-prone.
> Rephrase to describe the structural reason (the first tail element can be a LIST for generic-head forms)
> without naming a non-existent call chain.

This is the #1700 comment-reword (which addressed my #1700 note about the hard-coded line-ref). The reword
swapped the line-anchor for pseudo-code `tail.first().as_name()` — but that call chain doesn't exist
(`as_name` is `ast.as_name(s)`, an Arenas method, not on `StructId`). Describe the STRUCTURAL reason
("a generic-head `(type (Name a) …)` has a LIST as its first tail element, so a name-only read misses it")
rather than a non-existent method chain. LOW/doc, fix-forward.
