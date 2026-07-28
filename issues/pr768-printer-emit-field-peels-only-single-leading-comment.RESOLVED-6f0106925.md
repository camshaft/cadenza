# PR#768 review comment — printer emit_field peels only ONE leading `(comment …)`; multiple/combined wrappers mis-print

Mirrored from GitHub PR review comment (Copilot), id `3627695482`.
PR: https://github.com/camshaft/cadenza/pull/768 (batch-staging; fix belongs on trunk)
Location: `implementation/seed/crates/cadenza-syntax/src/printer.rs:2368` (`emit_field` in the record/map field printer)

## Comment (verbatim)

> `emit_field` only peels a single leading `(comment ...)` wrapper. If a field/entry has multiple
> own-line comments (nested `(comment ...)` nodes), or if a last field has both a leading `comment`
> and a trailing `comment-after` (nesting can be either order), the remaining wrapper gets passed to
> `emit` and the record/map field prints incorrectly. Peel wrappers in a loop so you always hand
> `emit` the innermost `(name value)` pair, while still rendering all leading comment lines and the
> trailing same-line comment.

## Liaison verification (CONFIRMED on trunk/staging)

`emit_field` (printer.rs ~2364-2382, landed `c31243cec` "preserve own-line comments leading record/map
fields") peels the leading comment with a single `if let`, NOT a loop:
```rust
let mut inner = f;
if let Some(a) = p.a.as_form(inner, "comment") && a.len()==2 && p.is_string(a[0]) {
    p.doc.word(format!("//{}", p.doc_line_text(a[0])));
    p.doc.hardbreak();
    inner = a[1];                        // peels exactly ONE
}
let trailing = p.a.as_form(inner, "comment-after") ...;   // then one trailing
let pair = trailing.map_or(inner, |(_, pair)| pair);
```
So:
- Two+ stacked own-line comments on a field — `(comment c1 (comment c2 (name val)))` — peel only `c1`;
  the inner `(comment c2 …)` is handed to `emit` as if it were the `(name value)` pair → mis-print.
- A field wrapped BOTH leading `(comment …)` and trailing `(comment-after …)` where the nesting is
  `(comment c (comment-after t (name val)))` works, but the OTHER nesting order
  `(comment-after t (comment c (name val)))` — trailing outer, leading inner — isn't handled: the
  single leading peel doesn't fire (outer is comment-after), and after the trailing peel the residual
  `(comment c …)` reaches `emit`.

Reachability caveat: the reader may only ever produce a single-comment-per-field shape today, so this
may not be hit by reader-produced ASTs — but like the PR#763 `is_pairs` case, the PRINTER should be
total over decoded/metaprogramming-built ASTs, and this is the same "printer must not assume the
reader's shape invariant" class.

Fix (per Copilot): peel leading `(comment …)` wrappers in a LOOP (rendering each `// line` +
hardbreak), and handle a trailing `(comment-after …)` at either nesting depth, so `emit` always gets
the innermost `(name value)` pair. Add a decoded-AST regression test with a field carrying two leading
comments + a trailing one.

Owner: v-syntax (`cadenza-syntax/src/printer.rs`; the S3-interior comment-preservation series
`c31243cec`/`bf871e6d0`). Routed as a note. (Companion to the PR#763 printer-totality fix `15b4a5072`.)
