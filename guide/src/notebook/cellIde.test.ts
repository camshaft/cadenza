/// Unit tests for the per-cell IDE `prepare` seam (notebook IDE #13). Pins that a cell's linter input is
/// the cell's own live text with the prior-cell scope + widget bindings prepended, and that
/// `wrapPrefixBytes` is the exact UTF-8 byte length of that prefix — so `cadenzaLint` maps a diagnostic in
/// the cell back onto the cell editor (and drops one in the prefix). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { prepareCell } from "./cellIde.ts";
import type { Cell, CellDirective } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";
import type { WidgetValues } from "./assembleForRun.ts";

const code = (source: string, directive: CellDirective = { kind: "none" }): Cell => ({
  kind: "code",
  source,
  directive,
});
const NO_WIDGETS: Widget[] = [];
const NO_VALUES: WidgetValues = Object.create(null);

// The compiled text is `<prefix><cellText><exportSuffix>`. The export suffix (`\nexport { main }` /
// `\n(export main)`) roots the module so `compile` doesn't decline it as "nothing is public" (operator
// UX #1) — a SUFFIX, so it never shifts `wrapPrefixBytes`. These helpers isolate the cell-body slice.
const ML_EXPORT = "\nexport { main }";
const SEXPR_EXPORT = "\n(export main)";

test("the first code cell: prepare is identity + export suffix (no prior scope, no widgets)", () => {
  const cells: Cell[] = [code("def main() = 1 + 2")];
  const r = prepareCell(cells, 0, NO_WIDGETS, NO_VALUES, "ml", "def main() = 1 + 2");
  assert.equal(r.compiled, "def main() = 1 + 2" + ML_EXPORT);
  assert.equal(r.wrapPrefixBytes, 0);
});

test("a later cell: prior cells' defs are prepended and counted in wrapPrefixBytes", () => {
  const cells: Cell[] = [code("def x = 10"), code("def y = 20"), code("def main() = x + y")];
  const cellText = "def main() = x + y";
  const r = prepareCell(cells, 2, NO_WIDGETS, NO_VALUES, "ml", cellText);
  // prefix = the two prior cells joined + a trailing blank-line separator, then the cell text + export.
  assert.equal(r.compiled, "def x = 10\n\ndef y = 20\n\ndef main() = x + y" + ML_EXPORT);
  const prefix = "def x = 10\n\ndef y = 20\n\n";
  assert.equal(r.wrapPrefixBytes, Buffer.byteLength(prefix, "utf8"));
  // The cell text sits exactly at the prefix boundary (so a diagnostic at cell offset 0 maps to editor 0);
  // the export suffix follows it (cadenzaLint clamps suffix diagnostics to the cell-content end).
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText + ML_EXPORT);
});

test("widget bindings are prepended before prior cells (in scope for the linter)", () => {
  const widgets: Widget[] = [
    { name: "rate", type: "Float64", control: "slider", min: 0, max: 1, step: 0.1, default: 0.5 },
  ];
  const values: WidgetValues = Object.assign(Object.create(null), { rate: 0.2 });
  const cells: Cell[] = [code("def main() = rate * 100")];
  const cellText = "def main() = rate * 100";
  const r = prepareCell(cells, 0, widgets, values, "ml", cellText);
  // The widget binding uses the LIVE value (0.2), ML surface `def rate = <lit>`.
  assert.ok(r.compiled.startsWith("def rate = "), `expected widget binding first, got: ${r.compiled.slice(0, 40)}`);
  assert.ok(r.compiled.endsWith(ML_EXPORT), "export suffix roots the module");
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText + ML_EXPORT);
});

test("a widget with no live value falls back to its default", () => {
  const widgets: Widget[] = [
    { name: "n", type: "Int64", control: "number", min: 0, max: 10, step: 1, default: 3 },
  ];
  const cells: Cell[] = [code("def main() = n")];
  const r = prepareCell(cells, 0, widgets, NO_VALUES, "ml", "def main() = n");
  // default 3 is used (no value in the empty map).
  assert.ok(r.compiled.includes("def n = 3"), `expected default binding, got: ${r.compiled.slice(0, 40)}`);
});

test("wrapPrefixBytes counts UTF-8 bytes, not UTF-16 code units", () => {
  // A prior cell with a multi-byte char (a string literal) — the prefix byte length must be the UTF-8
  // length, since the compiler reports UTF-8 offsets that cadenzaLint subtracts this from.
  const cells: Cell[] = [code('def label = "café"'), code("def main() = 1")];
  const r = prepareCell(cells, 1, NO_WIDGETS, NO_VALUES, "ml", "def main() = 1");
  const prefix = 'def label = "café"\n\n';
  assert.equal(r.wrapPrefixBytes, Buffer.byteLength(prefix, "utf8"));
  // "café" is 5 UTF-16 units but 5 chars / 6 UTF-8 bytes for the é — so byte length > char length.
  assert.ok(Buffer.byteLength(prefix, "utf8") > prefix.length, "é should make byte length exceed char length");
});

// ─── s-expr surface ──────────────────────────────────────────────────────────────────────────────
// The shipped notebook runs in the s-expr surface (NotebookPage's `NOTEBOOK_SURFACE = "sexpr"`), so the
// cell IDE's `prepare` is exercised in s-expr in production — pin that path explicitly (the ML tests
// above alone leave the REAL user surface un-gated).

test("s-expr: the first code cell — prepare is identity + export suffix (no prior scope, no widgets)", () => {
  const cells: Cell[] = [code("(def (main) (+ 1 2))")];
  const r = prepareCell(cells, 0, NO_WIDGETS, NO_VALUES, "sexpr", "(def (main) (+ 1 2))");
  assert.equal(r.compiled, "(def (main) (+ 1 2))" + SEXPR_EXPORT);
  assert.equal(r.wrapPrefixBytes, 0);
});

test("s-expr: a later cell — prior cells' defs are prepended and counted in wrapPrefixBytes", () => {
  const cells: Cell[] = [code("(def (x) 10)"), code("(def (y) 20)"), code("(def (main) (+ x y))")];
  const cellText = "(def (main) (+ x y))";
  const r = prepareCell(cells, 2, NO_WIDGETS, NO_VALUES, "sexpr", cellText);
  assert.equal(r.compiled, "(def (x) 10)\n\n(def (y) 20)\n\n(def (main) (+ x y))" + SEXPR_EXPORT);
  const prefix = "(def (x) 10)\n\n(def (y) 20)\n\n";
  assert.equal(r.wrapPrefixBytes, Buffer.byteLength(prefix, "utf8"));
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText + SEXPR_EXPORT);
});

test("s-expr: widget bindings are prepended before prior cells using the live value", () => {
  const widgets: Widget[] = [
    { name: "rate", type: "Float64", control: "slider", min: 0, max: 1, step: 0.1, default: 0.5 },
  ];
  const values: WidgetValues = Object.assign(Object.create(null), { rate: 0.2 });
  const cells: Cell[] = [code("(def (main) (* rate 100.0))")];
  const cellText = "(def (main) (* rate 100.0))";
  const r = prepareCell(cells, 0, widgets, values, "sexpr", cellText);
  // s-expr binding is `(def (rate) <lit>)` using the LIVE value (0.2).
  assert.ok(r.compiled.startsWith("(def (rate) "), `expected widget binding first, got: ${r.compiled.slice(0, 40)}`);
  assert.ok(r.compiled.endsWith(SEXPR_EXPORT), "export suffix roots the module");
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText + SEXPR_EXPORT);
});

test("s-expr: a prior cell's own `main` is stripped from the linter prefix (P0 #12, no CDZ0201)", () => {
  // The prior cell defines ITS OWN `main` (a private per-cell output slot). It must NOT enter this cell's
  // linter prefix, else the compiled buffer has two `main`s and the cell mis-lints with CDZ0201 (>1 main).
  // The prior cell also defines a `base` helper, which SHOULD flow forward.
  const cells: Cell[] = [code("(def (base) 100)\n(def (main) base)"), code("(def (main) (* base 2))")];
  const cellText = "(def (main) (* base 2))";
  const r = prepareCell(cells, 1, NO_WIDGETS, NO_VALUES, "sexpr", cellText);
  // Exactly one `main` DEF in the compiled buffer (this cell's own — the `(export main)` suffix is a
  // reference, not a def), and the prior `base` helper is in scope.
  assert.equal((r.compiled.match(/\(def \(main\)/g) ?? []).length, 1, "prior cell's `main` stripped");
  assert.ok(r.compiled.includes("(def (base) 100)"), "prior non-`main` helper flows into scope");
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText + SEXPR_EXPORT);
});

test("s-expr: a widget with no live value falls back to its default", () => {
  const widgets: Widget[] = [
    { name: "n", type: "Int64", control: "number", min: 0, max: 10, step: 1, default: 3 },
  ];
  const cells: Cell[] = [code("(def (main) n)")];
  const r = prepareCell(cells, 0, widgets, NO_VALUES, "sexpr", "(def (main) n)");
  assert.ok(r.compiled.includes("(def (n) 3)"), `expected default binding, got: ${r.compiled.slice(0, 40)}`);
});

// ─── operator UX #1: the linted module must be rooted so `main` isn't flagged unused ───────────────
// The IDE lints via `compile` (not `repl_eval`, which roots the entry implicitly). A raw cell def-block
// has no `export`, so `compile` declines it "nothing is public" — the operator saw every cell's `main`
// flagged unused. `prepareCell` appends an `export` SUFFIX so the module has a public entry; these pin
// the suffix names `main` when present (both surfaces) and the cell's own def otherwise.

test("prepareCell exports `main` when the cell defines it (roots the module — operator UX #1)", () => {
  const sexpr = prepareCell([code("(def (main) (+ 1 2))")], 0, NO_WIDGETS, NO_VALUES, "sexpr", "(def (main) (+ 1 2))");
  assert.ok(sexpr.compiled.endsWith("\n(export main)"), `s-expr export suffix, got: ${JSON.stringify(sexpr.compiled)}`);
  const ml = prepareCell([code("def main() = 1 + 2")], 0, NO_WIDGETS, NO_VALUES, "ml", "def main() = 1 + 2");
  assert.ok(ml.compiled.endsWith("\nexport { main }"), `ml export suffix, got: ${JSON.stringify(ml.compiled)}`);
});

test("prepareCell exports the cell's own def when there is no `main` (no dangling export)", () => {
  const r = prepareCell([code("(def (helper) 42)")], 0, NO_WIDGETS, NO_VALUES, "sexpr", "(def (helper) 42)");
  assert.ok(r.compiled.endsWith("\n(export helper)"), `should export the first def, got: ${JSON.stringify(r.compiled)}`);
  // A cell that declares nothing exportable (a bare expression) gets NO export suffix (would be dangling).
  const bare = prepareCell([code("(+ 1 2)")], 0, NO_WIDGETS, NO_VALUES, "sexpr", "(+ 1 2)");
  assert.ok(!/\(export/.test(bare.compiled), `a bare expression cell gets no export, got: ${JSON.stringify(bare.compiled)}`);
});

test("prepareCell exports a def CONSUMED by a LATER cell so it isn't false-flagged unused (CDZ0306)", () => {
  // The loan-example shape: cell 0 defines `base` + `main`, and a LATER cell (2) plots `base`. Linting cell 0
  // in ISOLATION would flag `base` as an unused definition — but in the notebook's sequential scope it IS used
  // downstream. prepareCell exports the downstream-consumed def so the per-cell linter counts it used.
  const cells: Cell[] = [
    code("(def (base) 100)\n(def (main) base)"), // cell 0: base used downstream + main
    code("(def (main) 1)"), // cell 1: unrelated
    code("(def (main) (* base 2))"), // cell 2: consumes cell 0's `base`
  ];
  const r = prepareCell(cells, 0, NO_WIDGETS, NO_VALUES, "sexpr", "(def (base) 100)\n(def (main) base)");
  // The export must include `base` (downstream-used) alongside `main`.
  assert.match(r.compiled, /\(export [^)]*\bbase\b[^)]*\)/, `base should be exported (downstream-used), got: ${r.compiled}`);
  assert.match(r.compiled, /\(export [^)]*\bmain\b[^)]*\)/, `main should still be exported, got: ${r.compiled}`);
  // A def NOT used downstream (and not main) is NOT force-exported — only main + downstream-used names appear
  // in the export form. (This cell is the LAST, so `helper` has no downstream consumer.)
  const noDownstream = prepareCell([code("(def (helper) 9)\n(def (main) helper)")], 0, NO_WIDGETS, NO_VALUES, "sexpr", "(def (helper) 9)\n(def (main) helper)");
  const exportForm = /\(export ([^)]*)\)/.exec(noDownstream.compiled)?.[1] ?? "";
  assert.ok(!exportForm.split(/\s+/).includes("helper"), `a non-downstream local def isn't in the export list, got export: (${exportForm})`);
});
