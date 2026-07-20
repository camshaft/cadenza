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

test("a DIAMOND dependency re-runs the re-join cell exactly ONCE (reachable via two dirty paths)", () => {
  // k feeds two branches (a, b); a fourth cell d = a + b re-joins them. d is dirtied via BOTH a and b, but
  // must appear in the plan ONCE, not twice — the plan walks cells in document order and pushes each dirty
  // cell a single time (its produced names then dirty downstream). Pinned so a future refactor of the
  // dirty-propagation loop can't accidentally double-push a cell reachable by multiple dirty paths.
  const cells: Cell[] = [
    code("k : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def a = k + 1.0"), // branch 1 (consumes k)
    code("def b = k + 2.0"), // branch 2 (consumes k)
    code("def d = a + b"), // re-join (consumes BOTH a and b — two dirty paths converge)
  ];
  const plan = recomputePlan(cells, [slider("k")], "k", "ml");
  assert.deepEqual(plan, [1, 2, 3]); // d (index 3) present exactly once, after both branches
  // Belt-and-suspenders: no duplicate indices in any plan.
  assert.equal(new Set(plan).size, plan.length, "plan must contain no duplicate cell indices");
});

test("`main` is NOT a cross-cell dependency — a widget change does not cascade through every cell's `main`", () => {
  // Production reality: EVERY notebook code cell defines its own `main` (its private per-cell entry slot,
  // stripped from downstream scope by assembleCell.stripMainDef, P0 #12). If `main` were treated as a
  // producible/consumable name, changing `rate` (used only by cell 1) would dirty cell 1's `main`, which
  // cell 2 "consumes" (it also has `def (main)`), cascading to EVERY downstream cell — defeating the
  // reactive minimization. Only the cell that truly references `rate` (+ genuine downstream data deps)
  // should re-run. This pins that `main` is excluded from the dependency graph (both surfaces).
  const sexprCells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("(def (main) (* rate 100.0))"), // idx1: really uses rate
    code("(def (main) 42.0)"), // idx2: independent constant, own main — must NOT re-run
    code("(def (main) (+ 1.0 2.0))"), // idx3: independent constant, own main — must NOT re-run
  ];
  assert.deepEqual(recomputePlan(sexprCells, [slider("rate")], "rate", "sexpr"), [1]);

  const mlCells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def main() = rate * 100.0"), // idx1: uses rate
    code("def main() = 42.0"), // idx2: independent, own main
  ];
  assert.deepEqual(recomputePlan(mlCells, [slider("rate")], "rate", "ml"), [1]);
});

test("`main` exclusion does not suppress a GENUINE downstream data dependency", () => {
  // The fix excludes `main` as a dependency name, but a real cross-cell data dep (via a NON-main helper)
  // must still propagate: cell 1 produces `base` from `rate`; cell 2's `main` uses `base` → it re-runs.
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("(def (base) (* rate 100.0))"), // idx1: produces base (non-main) from rate
    code("(def (main) base)"), // idx2: uses base → genuinely dirty
    code("(def (main) 7.0)"), // idx3: independent → NOT in plan
  ];
  assert.deepEqual(recomputePlan(cells, [slider("rate")], "rate", "sexpr"), [1, 2]);
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
