/// Unit tests for the reactive recompute planner (Increment 4, the novel dataflow core). Pins that a
/// widget change re-runs exactly the (transitively) dependent code cells in document order, that
/// independent cells are NOT re-run, and that dirtiness propagates downstream through produced defs.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { recomputePlan, initialRunOrder } from "./recomputePlan.ts";
import type { Cell, CellDirective } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";

const code = (source: string, directive: CellDirective = { kind: "none" }): Cell => ({ kind: "code", source, directive });
const prose = (markdown: string): Cell => ({ kind: "prose", markdown });
const slider = (name: string): Widget => ({ name, type: "Float64", control: "slider", min: 0, max: 100, step: 1, default: 0 });

test("a widget change re-runs the cell that references it", () => {
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def main() = rate * 100.0"),
  ];
  assert.deepEqual(recomputePlan(cells, [slider("rate")], "rate", "ml"), [1]);
});

test("a cell NOT referencing the changed widget is not re-run", () => {
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def main() = rate * 100.0"), // depends on rate
    code("def other() = 5"), // independent
  ];
  assert.deepEqual(recomputePlan(cells, [slider("rate")], "rate", "ml"), [1]);
});

test("dirtiness propagates downstream through produced defs (transitive)", () => {
  const cells: Cell[] = [
    code("principal : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def base = principal * 2.0"), // consumes principal → dirty; produces `base`
    code("def total = base + 1.0"), // consumes base (now dirty) → dirty; produces `total`
    code("def main() = total"), // consumes total → dirty
    code("def unrelated = 9"), // independent → NOT in plan
  ];
  assert.deepEqual(recomputePlan(cells, [slider("principal")], "principal", "ml"), [1, 2, 3]);
});

test("prose cells are skipped; plan indices point at the real code cells", () => {
  const cells: Cell[] = [
    code("x : Float64 = slider(0, 10)", { kind: "widget" }),
    prose("## explanation"),
    code("def main() = x + 1.0"), // index 2 in the full list
  ];
  assert.deepEqual(recomputePlan(cells, [slider("x")], "x", "ml"), [2]);
});

test("a change to an unreferenced widget produces an empty plan", () => {
  const cells: Cell[] = [
    code("a : Float64 = slider(0, 1)", { kind: "widget" }),
    code("b : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def main() = a + 1.0"), // uses a only
  ];
  assert.deepEqual(recomputePlan(cells, [slider("a"), slider("b")], "b", "ml"), []);
});

test("kebab-aware: changing `rate` does not re-run a cell that only uses `rate-adjusted`", () => {
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("rate-adjusted : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def main() = rate-adjusted * 2.0"), // references ONLY rate-adjusted
  ];
  assert.deepEqual(recomputePlan(cells, [slider("rate"), slider("rate-adjusted")], "rate", "ml"), []);
  assert.deepEqual(recomputePlan(cells, [slider("rate"), slider("rate-adjusted")], "rate-adjusted", "ml"), [2]);
});

test("a widget feeding two independent branches re-runs both, in document order", () => {
  const cells: Cell[] = [
    code("k : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def a = k + 1.0"), // branch 1
    code("def b = k + 2.0"), // branch 2
  ];
  assert.deepEqual(recomputePlan(cells, [slider("k")], "k", "ml"), [1, 2]);
});

test("initialRunOrder lists every code cell (not widget/prose) in document order", () => {
  const cells: Cell[] = [
    prose("intro"),
    code("w : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def a = 1"),
    prose("mid"),
    code("def main() = a"),
  ];
  assert.deepEqual(initialRunOrder(cells), [2, 4]);
});
