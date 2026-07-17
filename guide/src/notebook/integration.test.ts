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

test("the balance cell's run buffer carries the widget binding + the cell def; entry is a main() call", () => {
  const cells = parseDocument(NOTEBOOK);
  const widgets = allWidgets(cells);
  assert.deepEqual(widgets.map((w) => w.name), ["rate"]);
  // The balance cell is index 3 (prose,widget,prose,CODE,...).
  const { buffer, entry } = assembleForRun(cells, 3, widgets, { rate: 0.1 }, "ml");
  // Widget binding AND the cell's own def-block are both in the buffer; entry is a CALL (an expression).
  assert.equal(buffer, "def rate = 0.1\n\ndef main() = 1000.0 * (1.0 + rate)");
  assert.equal(entry, "main()");
  assert.ok(!/^def\b/.test(entry), "entry must be an expression, never a def-block (replEval contract)");
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
  assert.match(buffer, /^def rate = 0.05\b/); // widget binding uses the slider's default
});

test("a notebook with no widgets: initial run covers all code cells, no recompute plan for a phantom widget", () => {
  const cells = parseDocument("intro\n\n~~~cadenza\ndef main() = 1 + 2\n~~~");
  assert.deepEqual(allWidgets(cells), []);
  assert.deepEqual(initialRunOrder(cells), [1]);
  assert.deepEqual(recomputePlan(cells, [], "nope", "ml"), []);
});

test("dragging `rate` recomputes ONLY the dependent cell even when EVERY code cell defines `main`", () => {
  // The shipped notebook's reality: every code cell defines its own `main` (its private entry slot). The
  // NOTEBOOK fixture above uses `def aside()` for its independent cell, so it wouldn't catch a planner that
  // treats `main` as a cross-cell dependency (the recomputePlan `main`-cascade bug). This end-to-end case
  // makes BOTH code cells define `main` — the balance cell uses `rate`, the aside is an independent
  // constant — and pins that only the balance cell recomputes. A regression where `main` leaks into the
  // dependency graph would recompute the aside too, and this fails.
  const doc = [
    "~~~cadenza widget",
    "rate : Float64 = slider(0.0, 0.2, step: 0.01, default: 0.05)",
    "~~~",
    "",
    "~~~cadenza",
    "def main() = 1000.0 * (1.0 + rate)", // code cell idx 1 — uses rate
    "~~~",
    "",
    "~~~cadenza",
    "def main() = 42.0", // code cell idx 2 — independent constant, ALSO defines main
    "~~~",
  ].join("\n");
  const cells = parseDocument(doc);
  const widgets = allWidgets(cells);
  assert.deepEqual(widgets.map((w) => w.name), ["rate"]);
  // Cells: [0]=widget, [1]=balance code, [2]=aside code (no interleaving prose here).
  assert.deepEqual(initialRunOrder(cells), [1, 2]);
  // Only the balance cell (idx 1) truly depends on rate; the aside (idx 2) must NOT recompute despite
  // also defining `main`. A planner treating `main` as a cross-cell dep would recompute idx 2 too.
  assert.deepEqual(recomputePlan(cells, widgets, "rate", "ml"), [1]);
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
