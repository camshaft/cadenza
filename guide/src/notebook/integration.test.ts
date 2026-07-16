/// Integration tests over the notebook's PURE orchestration pipeline (no worker): a realistic markdown
/// document flows parseDocument → parseWidgets → assembleForRun → recomputePlan, and we pin the
/// end-to-end invariants the live NotebookPage relies on (which cells run, what buffer a cell sees, what
/// a widget drag recomputes). These guard the wiring between the individually-tested modules — a refactor
/// that breaks the seam (e.g. widget bindings stop reaching a cell) fails here even though each unit
/// still passes. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseDocument, type Cell } from "./parseDocument.ts";
import { parseWidgets, type Widget } from "./parseWidgets.ts";
import { assembleForRun } from "./assembleForRun.ts";
import { recomputePlan, initialRunOrder } from "./recomputePlan.ts";

/// Collect every widget declared across a parsed doc's widget cells (mirrors NotebookPage's `widgets`).
function allWidgets(cells: Cell[]): Widget[] {
  return cells.flatMap((c) =>
    c.kind === "code" && c.directive.kind === "widget" ? parseWidgets(c.source).widgets : [],
  );
}

const NOTEBOOK = `# Compound interest

Drag the rate.

~~~cadenza widget
rate : Float64 = slider(0.0, 0.2, step: 0.01, default: 0.05)
~~~

The balance:

~~~cadenza
def main() = 1000.0 * (1.0 + rate)
~~~

An independent aside:

~~~cadenza
def aside() = 42
~~~`;

test("a realistic notebook parses into the expected cell kinds in order", () => {
  const cells = parseDocument(NOTEBOOK);
  assert.deepEqual(
    cells.map((c) => (c.kind === "prose" ? "prose" : c.directive.kind === "widget" ? "widget" : "code")),
    ["prose", "widget", "prose", "code", "prose", "code"],
  );
});

test("the balance cell's run buffer carries the widget binding at its current value", () => {
  const cells = parseDocument(NOTEBOOK);
  const widgets = allWidgets(cells);
  assert.deepEqual(widgets.map((w) => w.name), ["rate"]);
  // The balance cell is index 3 (prose,widget,prose,CODE,...).
  const { buffer, entry } = assembleForRun(cells, 3, widgets, { rate: 0.1 }, "ml");
  assert.equal(buffer, "def rate = 0.1");
  assert.equal(entry, "def main() = 1000.0 * (1.0 + rate)");
});

test("initialRunOrder runs both code cells (not the widget/prose) top-to-bottom", () => {
  const cells = parseDocument(NOTEBOOK);
  assert.deepEqual(initialRunOrder(cells), [3, 5]);
});

test("dragging `rate` recomputes ONLY the dependent balance cell, not the independent aside", () => {
  const cells = parseDocument(NOTEBOOK);
  const widgets = allWidgets(cells);
  // Cell 3 uses `rate`; cell 5 (aside) does not.
  assert.deepEqual(recomputePlan(cells, widgets, "rate", "ml"), [3]);
});

test("a widget whose value is absent falls back to its default in the buffer", () => {
  const cells = parseDocument(NOTEBOOK);
  const widgets = allWidgets(cells);
  const { buffer } = assembleForRun(cells, 3, widgets, {}, "ml"); // no value supplied
  assert.equal(buffer, "def rate = 0.05"); // the slider's default
});

test("a notebook with no widgets: initial run covers all code cells, no recompute plan for a phantom widget", () => {
  const cells = parseDocument("intro\n\n~~~cadenza\ndef main() = 1 + 2\n~~~");
  assert.deepEqual(allWidgets(cells), []);
  assert.deepEqual(initialRunOrder(cells), [1]);
  assert.deepEqual(recomputePlan(cells, [], "nope", "ml"), []);
});

test("multiple widgets in ONE widget cell are all collected + independently drive recompute", () => {
  const doc = [
    "~~~cadenza widget",
    "a : Float64 = slider(0, 1)",
    "b : Float64 = slider(0, 1)",
    "~~~",
    "",
    "~~~cadenza",
    "def main() = a + 0.0", // uses a only
    "~~~",
  ].join("\n");
  const cells = parseDocument(doc);
  const widgets = allWidgets(cells);
  assert.deepEqual(widgets.map((w) => w.name), ["a", "b"]);
  assert.deepEqual(recomputePlan(cells, widgets, "a", "ml"), [1]); // code cell at index 1
  assert.deepEqual(recomputePlan(cells, widgets, "b", "ml"), []); // b unused → nothing recomputes
});
