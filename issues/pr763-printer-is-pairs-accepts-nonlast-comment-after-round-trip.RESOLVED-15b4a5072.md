# PR#763 review comment — printer `is_pairs` strips comment-after for EVERY pair; a non-last wrapped pair breaks round-trip

Mirrored from GitHub PR review comment (Copilot), id `3627016281`.
PR: https://github.com/camshaft/cadenza/pull/763 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/cadenza-syntax/src/printer.rs:3326` (`is_pairs`)

## Comment (verbatim)

> `is_pairs` currently strips `(comment-after ...)` wrappers for *every* field/entry. That makes the
> record/map surface guards accept a decoded AST where a non-last pair is wrapped, but printing such a
> shape would put the next `, …` separator after a `//` comment (swallowed into the comment) and break
> round-tripping. Keep the wrapper transparent only for the last element, and reject any non-last
> `(comment-after ...)` so the printer falls back to the generic call form for malformed ASTs.

## Liaison verification (CONFIRMED on trunk — PR#758-class, printer side)

`is_pairs` (printer.rs:3323-3330) does `let inner = self.strip_comment_after(a);` for EVERY arg, then
checks it's a 2-element pair. Its own comment asserts "only the last one is ever wrapped, by the
reader's `at(RBrace)` gate". That reader invariant now holds for reader-PRODUCED ASTs (the list capture
was just gated on `at(RBracket)` in the PR#758 fix `3cb9b6655`, and the record/map capture landed gated
in `87e08ad77`). BUT `is_pairs` is a PRINTER guard that must be correct on ANY well-formed-enough AST —
including a DECODED (`codec::decode`) or METAPROGRAMMING-constructed one where a non-last pair carries a
`(comment-after …)`. For such an AST, `is_pairs` returns true → the record/map surface prints it → the
`, …` separator lands after the `//` on the comment line → swallowed → invalid re-parse (the exact
PR#758 round-trip break, now via the printer instead of the reader).

This is the printer-side analogue of the PR#758 reader fix (v-syntax `3cb9b6655`, which gated the READER
capture) — a defense-in-depth completion: the printer shouldn't TRUST the reader's last-only invariant
when the AST may not have come from the reader.

Fix (per Copilot): in `is_pairs` (and the sibling `is_record_shape` / any other `strip_comment_after`
surface guard), keep the wrapper transparent ONLY for the LAST element; if a NON-last entry is a
`(comment-after …)`, return false so the printer falls back to the generic call form (which round-trips
safely). Small printer-hardening fix; add a regression test with a decoded/hand-built record AST whose
first field is comment-after-wrapped → must NOT take the `{…}` surface.

Owner: v-syntax (`cadenza-syntax/src/printer.rs`; round-trip / never-corrupt domain; landed the
record/map trailing-comment work in `87e08ad77`). Routed as a note.
