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

test("the first code cell: prepare is identity (no prior scope, no widgets)", () => {
  const cells: Cell[] = [code("def main() = 1 + 2")];
  const r = prepareCell(cells, 0, NO_WIDGETS, NO_VALUES, "ml", "def main() = 1 + 2");
  assert.equal(r.compiled, "def main() = 1 + 2");
  assert.equal(r.wrapPrefixBytes, 0);
});

test("a later cell: prior cells' defs are prepended and counted in wrapPrefixBytes", () => {
  const cells: Cell[] = [code("def x = 10"), code("def y = 20"), code("def main() = x + y")];
  const cellText = "def main() = x + y";
  const r = prepareCell(cells, 2, NO_WIDGETS, NO_VALUES, "ml", cellText);
  // prefix = the two prior cells joined + a trailing blank-line separator, then the cell text.
  assert.equal(r.compiled, "def x = 10\n\ndef y = 20\n\ndef main() = x + y");
  const prefix = "def x = 10\n\ndef y = 20\n\n";
  assert.equal(r.wrapPrefixBytes, Buffer.byteLength(prefix, "utf8"));
  // The cell text sits exactly at the prefix boundary (so a diagnostic at cell offset 0 maps to editor 0).
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText);
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
  assert.ok(r.compiled.endsWith(cellText));
  assert.equal(r.compiled.slice(r.wrapPrefixBytes), cellText);
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
