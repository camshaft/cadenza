# PR#758 review comment — parser list-element trailing-comment capture unconditional; mid-list `//` may swallow the following comma (round-trip)

Mirrored from GitHub PR review comment (Copilot), id `3626207970`.
PR: https://github.com/camshaft/cadenza/pull/758 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/cadenza-syntax/src/parser.rs:3002`

## Comment (verbatim)

> `take_trailing_comment_here()` is applied unconditionally after every list element, which can wrap a
> non-last element in `(comment-after ...)` when a trailing `//` sits before a comma token (e.g.
> `[1 // note\n, 2]`). The printer's list layout currently emits the comma after the element, so this
> shape would be re-printed as `1 // note, 2` where `, 2` is swallowed into the comment, producing
> invalid syntax. Consider only capturing trailing comments here when the next token is `]` (the last
> element case that the printer handles), until a general comma-after-comment layout is implemented.

## Liaison verification (STRUCTURAL — needs a round-trip repro by owner)

- parser.rs:3001-3002 (in the list-literal element loop, added `155d0adfb` "preserve a same-line
  trailing comment on a list element"):
  ```rust
  let trailing = self.take_trailing_comment_here();
  items.push(self.wrap_comment_after(trailing, elem));
  ```
  This runs after EVERY element, unconditionally — there is NO guard that the next token is `]`
  (unlike what the comment suggests would be safe). `take_trailing_comment_here` (parser.rs:623) just
  drains the leading `trailing` leads at the current pos; it doesn't care whether the next token is a
  comma or `]`.
- So for `[1 // note\n, 2]`, element `1`'s trailing `// note` IS captured into `(comment-after)`. The
  concern is whether the PRINTER then re-emits it as `1 // note, 2` — putting the `, 2` on the comment
  line where it's swallowed → invalid re-parse (a round-trip break).

I could not build a `cdz` bin cheaply to run the round-trip, so this is a STRUCTURAL confirmation of
the setup, not an observed failure. It's squarely in v-syntax's strict round-trip / never-drop domain
and the capture is a 2-commit-old addition (`155d0adfb`), so it's worth a real repro:
`echo '[1 // note\n, 2]'` (as .cdz) → parse → print → re-parse, check it round-trips and doesn't
swallow `, 2`.

Suggested fix (per Copilot): gate the trailing-comment capture on `self.at(Kind::RBracket)` after the
element (i.e. only the LAST element, which the printer handles), until a general comma-after-comment
layout exists. Owner: v-syntax (`cadenza-syntax/src/parser.rs`; round-trip contract). Routed as a note
flagged PLAUSIBLE-ROUNDTRIP-BREAK, verify-with-repro.
