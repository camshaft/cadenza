/// Unit tests for sequential cell-scope assembly (design D1). Pins the accumulating-buffer model: a code
/// cell sees prior code cells' top-level defs; prose + widget cells don't contribute; the dependency
/// scan is whole-word + kebab-aware. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleCell, cellDependencies } from "./assembleCell.ts";
import type { Cell, CellDirective } from "./parseDocument.ts";

const code = (source: string, directive: CellDirective = { kind: "none" }): Cell => ({
  kind: "code",
  source,
  directive,
});
const prose = (markdown: string): Cell => ({ kind: "prose", markdown });

test("the first code cell has an empty buffer and no in-scope names", () => {
  const cells: Cell[] = [code("def main() = 1 + 2")];
  assert.deepEqual(assembleCell(cells, 0, "ml"), {
    buffer: "",
    entry: "def main() = 1 + 2",
    inScope: [],
  });
});

test("a later cell sees prior code cells' defs in its buffer + inScope (sequential scope)", () => {
  const cells: Cell[] = [
    code("def x = 10"),
    code("def y = 20"),
    code("def main() = x + y"),
  ];
  const a = assembleCell(cells, 2, "ml");
  assert.equal(a.buffer, "def x = 10\n\ndef y = 20");
  assert.equal(a.entry, "def main() = x + y");
  assert.deepEqual(a.inScope, ["x", "y"]);
});

test("prose cells between code cells do NOT contribute to scope", () => {
  const cells: Cell[] = [
    code("def x = 10"),
    prose("## some prose\n\nexplaining things"),
    code("def main() = x + 1"),
  ];
  const a = assembleCell(cells, 2, "ml");
  assert.equal(a.buffer, "def x = 10");
  assert.deepEqual(a.inScope, ["x"]);
});

test("a widget cell does NOT contribute via the def-buffer (its bindings are spliced in Inc 4)", () => {
  const cells: Cell[] = [
    code("principal : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def x = 5"),
    code("def main() = x"),
  ];
  const a = assembleCell(cells, 2, "ml");
  // Only the plain code cell `def x` is in the buffer; the widget cell is skipped here.
  assert.equal(a.buffer, "def x = 5");
  assert.deepEqual(a.inScope, ["x"]);
});

test("a hidden code cell DOES contribute (it runs, just shows no source)", () => {
  const cells: Cell[] = [
    code("def secret = 42", { kind: "hidden" }),
    code("def main() = secret"),
  ];
  const a = assembleCell(cells, 1, "ml");
  assert.equal(a.buffer, "def secret = 42");
  assert.deepEqual(a.inScope, ["secret"]);
});

test("empty code cells contribute nothing to the buffer", () => {
  const cells: Cell[] = [code("def x = 1"), code("   "), code("def main() = x")];
  const a = assembleCell(cells, 2, "ml");
  assert.equal(a.buffer, "def x = 1");
  assert.deepEqual(a.inScope, ["x"]);
});

test("s-expr surface reads defs with the s-expr def regex", () => {
  const cells: Cell[] = [code("(def (x) 10)"), code("(def (main) (+ x 1))")];
  const a = assembleCell(cells, 1, "sexpr");
  assert.equal(a.buffer, "(def (x) 10)");
  assert.deepEqual(a.inScope, ["x"]);
});

test("assembleCell throws on a bad index or a prose cell", () => {
  const cells: Cell[] = [prose("hi")];
  assert.throws(() => assembleCell(cells, 5, "ml"), RangeError);
  assert.throws(() => assembleCell(cells, 0, "ml"), TypeError);
});

test("cellDependencies reports only in-scope names the entry actually references (whole-word)", () => {
  const cells: Cell[] = [
    code("def x = 1"),
    code("def rate = 2"),
    code("def main() = x + 0"), // uses x, not rate
  ];
  const a = assembleCell(cells, 2, "ml");
  assert.deepEqual(a.inScope, ["x", "rate"]);
  assert.deepEqual(cellDependencies(a), ["x"]);
});

test("cellDependencies is kebab-aware — `rate` does NOT match inside `rate-adjusted`", () => {
  const cells: Cell[] = [
    code("def rate = 2"),
    code("def rate-adjusted = 3"),
    code("def main() = rate-adjusted + 1"), // references ONLY rate-adjusted, not the bare `rate`
  ];
  const a = assembleCell(cells, 2, "ml");
  assert.deepEqual(a.inScope, ["rate", "rate-adjusted"]);
  // The token `rate` appears as a substring of `rate-adjusted`, but must NOT be reported as used.
  assert.deepEqual(cellDependencies(a), ["rate-adjusted"]);
});

test("cellDependencies over-approximates safely but doesn't invent names not in scope", () => {
  const cells: Cell[] = [code("def x = 1"), code("def main() = y + z")]; // y, z not defined anywhere
  const a = assembleCell(cells, 1, "ml");
  assert.deepEqual(a.inScope, ["x"]);
  assert.deepEqual(cellDependencies(a), []); // x not used; y/z aren't in scope so not reported
});
