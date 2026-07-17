/// Unit tests for sequential cell-scope assembly (design D1). Pins the accumulating-buffer model: a code
/// cell sees prior code cells' top-level defs; prose + widget cells don't contribute; the dependency
/// scan is whole-word + kebab-aware. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleCell, cellDependencies, stripMainDef, topLevelForms } from "./assembleCell.ts";
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

// ── P0 #12: a prior cell's own `main` must NOT collide with this cell's `main` (CDZ0201) ──

test("topLevelForms splits s-expr top-level forms (balanced parens), keeping order", () => {
  assert.deepEqual(topLevelForms("(def (x) 1)\n(def (main) (+ x 1))", "sexpr"), ["(def (x) 1)", "(def (main) (+ x 1))"]);
  // a string containing parens/quotes stays inside its form
  assert.deepEqual(topLevelForms('(def (s) "a (b) c")', "sexpr"), ['(def (s) "a (b) c")']);
});

test("topLevelForms splits ML top-level forms on def/type/effect line starts", () => {
  assert.deepEqual(topLevelForms("def x = 1\ndef main() = x + 1", "ml"), ["def x = 1", "def main() = x + 1"]);
  // a multi-line def body stays with its def (the continuation isn't a form start)
  assert.deepEqual(topLevelForms("def main() =\n  x + 1", "ml"), ["def main() =\n  x + 1"]);
});

test("stripMainDef removes a top-level `main` def, keeps every non-main def (both surfaces)", () => {
  assert.equal(stripMainDef("(def (base) 100.0)\n(def (main) (* base 2.0))", "sexpr"), "(def (base) 100.0)");
  assert.equal(stripMainDef("def base = 100.0\ndef main() = base * 2.0", "ml"), "def base = 100.0");
  // a cell that is ONLY main → empty contribution
  assert.equal(stripMainDef("(def (main) 42)", "sexpr"), "");
  // a helper whose NAME merely contains "main" is untouched (matched by whole def-name)
  assert.equal(stripMainDef("(def (mainline) 7)", "sexpr"), "(def (mainline) 7)");
});

test("assembleCell strips a PRIOR cell's `main` so two main-defining cells don't collide (P0 #12)", () => {
  const cells: Cell[] = [
    code("(def (main) (* 1000.0 (+ 1.0 rate)))"),
    code("(def (main) (list (tuple 1 (+ 1.0 rate))))"),
  ];
  const a = assembleCell(cells, 1, "sexpr");
  // The prior cell's `main` is dropped → the buffer carries nothing (it was only `main`); this cell's own
  // `main` is the entry, so the assembled module has exactly ONE `main`.
  assert.equal(a.buffer, "");
  assert.equal(a.entry, "(def (main) (list (tuple 1 (+ 1.0 rate))))");
  assert.ok(!a.inScope.includes("main"), "`main` is never an in-scope downstream name");
});

test("assembleCell keeps a prior cell's NON-main helper while stripping its main (sequential scope holds)", () => {
  const cells: Cell[] = [
    code("(def (base) 100.0)\n(def (main) base)"), // defines a helper AND a main
    code("(def (main) (* base 2.0))"), // references the prior helper
  ];
  const a = assembleCell(cells, 1, "sexpr");
  assert.equal(a.buffer, "(def (base) 100.0)"); // helper preserved, prior main dropped
  assert.deepEqual(a.inScope, ["base"]); // base in scope, main is not
});

// ── PR #529 hardening: topLevelForms + stripMainDef edge cases (both harden the P0 #12 fix) ──

test("topLevelForms splits INDENTED ML forms (leading whitespace before the keyword) — PR #529", () => {
  // An indented cell must still split into forms (each keyword line starts a form), else stripMainDef
  // misses its main → CDZ0201 recurs. Two indented defs → two separate forms.
  assert.deepEqual(topLevelForms("  def helper = 1\n  def main() = 2", "ml"), ["def helper = 1", "def main() = 2"]);
  assert.deepEqual(topLevelForms("  def a = 1\n  def b = 2", "ml"), ["def a = 1", "def b = 2"]);
});

test("stripMainDef strips an INDENTED prior-cell main (ML) — PR #529", () => {
  // Before the fix, the indented `main` rode along as part of one un-split form and was NOT stripped.
  assert.equal(stripMainDef("  def helper = 1\n  def main() = 2", "ml"), "def helper = 1");
});

test("stripMainDef strips a form whose `main` is NOT the first def (multi-def form) — PR #529", () => {
  // (do (def helper) (def main)) — main is second; the whole form must still be dropped.
  assert.equal(stripMainDef("(do (def (helper) 1) (def (main) 2))", "sexpr"), "");
  // A multi-def form with NO main is kept intact.
  assert.equal(stripMainDef("(do (def (a) 1) (def (b) 2))", "sexpr"), "(do (def (a) 1) (def (b) 2))");
});

test("stripMainDef regressions still hold: `mainline` kept, plain non-main defs kept", () => {
  assert.equal(stripMainDef("(def (mainline) 7)", "sexpr"), "(def (mainline) 7)");
  assert.equal(stripMainDef("(def (base) 1)\n(def (main) 2)", "sexpr"), "(def (base) 1)");
});
