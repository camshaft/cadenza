# PR review comment — mirrored from GitHub PR #405 (Copilot inline)

- **PR:** #405 "fleet: thirtieth batch (tagged-template reader, slack-bridge, peer-linking, broad features)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/cadenza-syntax/src/printer.rs:1851` (`print_tagged_template`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591071543
- **Link:** https://github.com/camshaft/cadenza/pull/405#discussion_r3591071543

## Comment (verbatim)
> `print_tagged_template` always uses `emit_name(tag)` when sugaring to `tag"…"`, but `emit_name` may backtick-quote names that aren't bare-safe. That would print something like `` `weird name`"…" `` which won't re-lex as a `TaggedTemplate` token, breaking round-tripping for valid `(tagged-template ...)` nodes whose tag isn't a bare identifier. Gate the sugar on `name_is_bare_safe(tag)` so only tags that can lex as an ident are re-sugared; otherwise fall back to generic call printing.

## Liaison triage — CONFIRMED against trunk — ROUND-TRIP CORRECTNESS
Confirmed: `print_tagged_template` emits `format!("{}\"{}\"", emit_name(tag), ...)`, and `emit_name`
(printer.rs:2668) backtick-quotes any name that isn't `name_is_bare_safe`. So a tagged-template whose
tag is not a bare identifier prints as `` `weird name`"…" ``, which does NOT re-lex as a
`TaggedTemplate` token → the printed form round-trips to something else (garbage-render). The
`is_tagged_template_shape` guard checks the node shape but NOT the tag's bare-safety. Per the project's
"garbage render = not canonical → fix the source" rule, a form that round-trips to garbage is a real
bug. FIX (as the reviewer says): gate the `tag"…"` sugar on `name_is_bare_safe(tag)`; otherwise fall
back to generic `(tagged-template …)` call printing (structure visible, never garbage). Syntax/printer
territory (v-syntax). Fix on `trunk`. Quote + link in queue file.
