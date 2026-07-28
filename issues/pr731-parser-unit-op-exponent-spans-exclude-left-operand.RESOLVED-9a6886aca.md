# PR#731 review comments — unit-op / unit-exponent AST spans exclude the left operand's start

Mirrored from GitHub PR review comments (Copilot), ids `3620914486`, `3620914508`.
PR: https://github.com/camshaft/cadenza/pull/731 (merged; fix still belongs on trunk)
Locations:
- `implementation/seed/crates/cadenza-syntax/src/parser.rs:1547` (compound unit op `a/b`, `a*b`)
- `implementation/seed/crates/cadenza-syntax/src/parser.rs:1578` (unit exponent `m^2`)

## Comments (verbatim)

- (id 3620914486, parser.rs:1547) "The span for a compound unit operation is currently built from
  `op_span.merge(self.prev_span())`, which excludes the left operand's start. That makes the AST
  span for `a/b` (in unit context) highlight only from the operator onward, unlike other infix
  nodes that span the full expression."
- (id 3620914508, parser.rs:1578) "The span for a unit exponent node is currently built from
  `op_span.merge(self.prev_span())`, which excludes the base unit atom's start. This produces
  truncated spans (starting at `^`) for `m^2`-style unit factors and can mis-anchor diagnostics."

## Liaison verification (CONFIRMED on trunk)

- parser.rs:1546 — `let span = op_span.merge(self.prev_span());` then `left = self.list(vec![head, left, rhs], span);`
  The `left` operand's start is NOT included in `span` (it starts at `op_span`, the operator).
- parser.rs:1577 — same pattern: `let span = op_span.merge(self.prev_span());` then
  `self.list(vec![head, atom, exp], span)` — the base unit `atom`'s start is excluded.

Both build the node span from `op_span.merge(self.prev_span())`, so the span begins at the operator
(`/`, `*`, `^`) rather than the left/base operand — a truncated span that mis-anchors diagnostics on
unit expressions. Other infix nodes span the full expression.

Fix: merge in the left/base operand's span start, e.g. `left.span().merge(self.prev_span())` (or
`atom`'s span for the exponent arm), so the node spans the full unit expression. Small parser fix;
add/adjust a span assertion. Owner: v-syntax (`cadenza-syntax/src/parser.rs`). Routed as a note.
