/// Guard for the TSX→sexpr bootstrap converter (scripts/tsx-to-sexp.mjs) — the one-time migration tool that
/// will be re-run to migrate the carve-out chapters + HomePage (fork1b). Pins the attr-value-abutting-tag-
/// close case that corrupted ControlFlow (operator seq-260): an inline mark whose attribute value ends in
/// `>` (e.g. `<TryChange … replace=">">`) must NOT truncate the attrs at that inner `>` — the naive
/// `[^>]*` tag-open regex did, dropping `replace` and leaking `">…` into the prose. Run with `test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseInline } from "../../scripts/tsx-to-sexp.mjs";

test("an inline mark attr value ending in '>' is parsed whole, not truncated at the inner '>'", () => {
  // the ControlFlow shape: replace=">" immediately abuts the tag-close '>'
  const out = parseInline(`Change <TryChange example="200" find="<" replace=">">the operator</TryChange> here`);
  assert.deepEqual(out, [
    `"Change "`,
    `(try-change (example "200") (find "<") (replace ">") "the operator")`,
    `" here"`,
  ]);
});

test("a normal inline mark (no '>' in attrs) still parses correctly", () => {
  assert.deepEqual(parseInline(`See <Ch to="basics">Basics</Ch>.`), [
    `"See "`,
    `(link (slug "basics") "Basics")`,
    `"."`,
  ]);
});
