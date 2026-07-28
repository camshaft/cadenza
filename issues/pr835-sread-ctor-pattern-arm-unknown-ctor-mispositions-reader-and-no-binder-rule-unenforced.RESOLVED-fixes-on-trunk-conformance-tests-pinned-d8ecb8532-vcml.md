# PR#835 review comment — sread read-ctor-pattern-arm: unknown-ctor branch mis-positions the reader + "no binder → decline" not enforced

Mirrored from GitHub PR review comment (Copilot), id `3636871107`.
PR: https://github.com/camshaft/cadenza/pull/835 (merged; fix belongs on trunk)
Location: `implementation/compiler-ml/src/sread.cdz:657` (`read-ctor-pattern-arm`)

## Comment (verbatim)

> In read-ctor-pattern-arm, the Option.None(unknown ctor) branch returns after skip-to-close/close-paren
> without consuming the arm body or the rest of the match arms. This leaves the reader index positioned
> at the start of the arm body, which will mis-parse the remainder of the (match …) form. Also, the
> current code doesn't actually enforce the "ctor with NO binder → decline" rule (scan-atom can return
> "" at `)`), so a missing binder silently creates an empty-name binder.

## Liaison verification (CONFIRMED on trunk)

`read-ctor-pattern-arm` (sread.cdz ~645), landed with the M2-a ctor-pattern work:
- Unknown-ctor arm (`Option.None`): `(match unsupported(tree) with | (id, t0) => (id, close-paren(s,
  skip-to-close(s, c1, 1)), t0))`. `skip-to-close` from `c1` (just after the ctor name) closes the
  PATTERN `(Ctor …)` — but it does NOT then consume the ARM BODY or `close-paren` the arm's outer `)`,
  nor recurse via `read-match-arms` for the remaining arms. The returned index sits at the arm body,
  so the rest of the `(match …)` mis-parses. (Contrast the `Option.Some` path, which correctly reads
  the body, closes the arm, and recurses.)
- No-binder rule: the fn's own doc says "A ctor with NO binder … → decline", but the `Option.Some`
  path calls `scan-atom(s, skip-space(s, c1), "")` for the binder — at a `)` (no binder) scan-atom
  returns `""`, and the code proceeds to `add-node(NVar(name-id("")))` → a silent empty-name binder
  instead of a decline.

Both are reader-correctness issues (the mis-positioning is the more serious — a malformed/unknown-ctor
arm corrupts the parse of the whole match). Fix direction: on the unknown-ctor decline, skip to the end
of the WHOLE arm (past body + arm close) so the reader is positioned to continue (or propagate a clean
decline up); and explicitly check the binder atom is non-empty (`""` → decline), matching the doc.

Owner: v-compiler-ml (`implementation/compiler-ml/*` port source — sread reader). Routed as a note
flagged CORRECTNESS. Recommend a reader @test: a `(match … ((UnknownCtor x) body) (other …))` and a
`(match … ((Ctor) body) …)` no-binder shape both parse/decline cleanly without corrupting later arms.
