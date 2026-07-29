# PR#886 review comment — 15-rows Record.with doc examples still use bare-name field operand (corpus-bugfix)

Mirrored from GitHub PR#886 review comment (Copilot), id `3670530056` (:327, also :654).
File: `spec/semantics/15-rows-and-open-sums.sexp` — corpus doc → corpus-bugfix. Blame `e2f4e2af0`
"corpus: migrate bare-name Record.extend/with field operands to #label (pre-reject)" — the migration
updated the `(input …)` CODE but left the DOC-string examples stale.

## Comment (verbatim)

- (id 3670530056, 15-rows-and-open-sums.sexp:327) "The doc string example still uses a bare identifier
  `x` as the Record.with field-name operand. After this PR's migration to explicit `#\"label\"` operands,
  this example should be updated to match the now-required spelling so the semantics case is
  self-consistent. This issue also appears on line 654 of the same file."

## Liaison verification (both confirmed on trunk 0b49c0c6a)

1. :323 doc: "`(Record.with (record (x 1) (y 2)) x a)` replaces x…" — bare `x`. But the case's own
   `(input …)` at :327 uses `(Record.with (record (x 1) (y 2)) #\"x\" a)` — the `#\"x\"` label. Doc vs
   code mismatch.
2. :648 doc: "`(if b (Record.with r x 99) r)`…" — bare `x`. The `(input …)` at :654 uses `(Record.with r
   #\"x\" 99)`. Same mismatch.

The `e2f4e2af0` migration (to explicit `#\"label\"` field operands, the CDZ0215 pre-reject line —
[[cdz0215-record-extend-with-field-name-must-be-label-not-bare-pun]]) updated the input code but not the
prose. Update both doc examples to `#\"x\"` to match. Doc-only, behavior-neutral (inputs already correct).

Owner: **corpus-bugfix** (`spec/semantics/*.sexp` case docs). Two doc edits (:323 + :648), same fix.
