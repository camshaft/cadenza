/// Unit tests for the pure output-renderer dispatch: (directive, run outcome) → CellOutput. Pins the
/// happy paths (table/chart/formula/value), the graceful fallbacks (a directive that doesn't fit the
/// value renders the value + a note, never an error), and status passthrough (trap/timeout/error).
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { renderOutput, type RunOutcome } from "./renderOutput.ts";
import type { CellDirective } from "./parseDocument.ts";

const value = (text: string): RunOutcome => ({ kind: "value", text });

test("a `table` directive over a List of records → a table render", () => {
  const out = renderOutput({ kind: "table" }, value("(: (list (record (a 1) (b 2))) T)"));
  assert.equal(out.render, "table");
  if (out.render === "table") {
    assert.deepEqual(out.table.columns, ["a", "b"]);
    assert.deepEqual(out.table.rows, [["1", "2"]]);
  }
});

test("a `chart:line` directive over (x,y) tuples → a chart render carrying the chart kind", () => {
  const out = renderOutput({ kind: "chart", chart: "line" }, value("(: (list (tuple 1 10) (tuple 2 20)) T)"));
  assert.equal(out.render, "chart");
  if (out.render === "chart") {
    assert.equal(out.chart, "line");
    assert.equal(out.series[0].points.length, 2);
  }
});

test("chart:area and chart:stacked carry their kind through to the chart render (operator: more chart types)", () => {
  const area = renderOutput({ kind: "chart", chart: "area" }, value("(: (list (tuple 1 10) (tuple 2 20)) T)"));
  assert.equal(area.render, "chart");
  if (area.render === "chart") assert.equal(area.chart, "area");
  // stacked over a multi-y tuple list (two series sharing x) — the kind rides through; ChartView stacks them.
  const stacked = renderOutput({ kind: "chart", chart: "stacked" }, value("(: (list (tuple 1 10 5) (tuple 2 20 8)) T)"));
  assert.equal(stacked.render, "chart");
  if (stacked.render === "chart") {
    assert.equal(stacked.chart, "stacked");
    assert.equal(stacked.series.length, 2); // two y-series to stack
  }
});

test("a `formula` directive classifies the value shape (scalar → plain, rational → fraction)", () => {
  const scalar = renderOutput({ kind: "formula" }, value("(: 42 Int64)"));
  assert.deepEqual(scalar, { render: "formula", formula: { kind: "plain", text: "42" } });
  const rat = renderOutput({ kind: "formula" }, value("(: 5/2 Rational)"));
  assert.deepEqual(rat, { render: "formula", formula: { kind: "fraction", num: "5", den: "2", negative: false } });
});

test("no directive → a plain value render in FRIENDLY form (`(: 7 Int64)` → `7`)", () => {
  const out = renderOutput({ kind: "none" }, value("(: 7 Int64)"));
  assert.deepEqual(out, { render: "value", text: "7" });
});

test("a rational value renders bare (`(: 5/2 Rational)` → `5/2`)", () => {
  const out = renderOutput({ kind: "none" }, value("(: 5/2 Rational)"));
  assert.deepEqual(out, { render: "value", text: "5/2" });
});

test("a `table` directive over a NON-list value → friendly value render + explanatory note (no error)", () => {
  const out = renderOutput({ kind: "table" }, value("(: 42 Int64)"));
  assert.equal(out.render, "value");
  if (out.render === "value") {
    assert.equal(out.text, "42");
    assert.match(out.note!, /not shown as a table/);
  }
});

test("a `chart` directive over a non-chartable value → value render + note", () => {
  const out = renderOutput({ kind: "chart", chart: "bar" }, value("(: 42 Int64)"));
  assert.equal(out.render, "value");
  if (out.render === "value") assert.match(out.note!, /not shown as a chart/);
});

test("a `chart` directive over an EMPTY list → value render + note (no points)", () => {
  const out = renderOutput({ kind: "chart", chart: "scatter" }, value("(: (list) (List Int64))"));
  assert.equal(out.render, "value");
  if (out.render === "value") assert.match(out.note!, /no data points/);
});

test("trap / timeout / error pass through regardless of directive", () => {
  const tableDir: CellDirective = { kind: "table" };
  assert.deepEqual(renderOutput(tableDir, { kind: "trap", message: "boom" }), { render: "trap", message: "boom" });
  assert.deepEqual(renderOutput(tableDir, { kind: "timeout" }), { render: "timeout" });
  assert.deepEqual(renderOutput(tableDir, { kind: "error", message: "declined" }), { render: "error", message: "declined" });
});

test("a widget/hidden cell's own output renders as a plain value", () => {
  assert.equal(renderOutput({ kind: "widget" }, value("(: 1 Int64)")).render, "value");
  assert.equal(renderOutput({ kind: "hidden" }, value("(: 1 Int64)")).render, "value");
});
