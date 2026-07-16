/// Unit tests for assembleForRun — the seam that folds current widget values + sequential cell scope into
/// the (buffer, entry) replEval consumes (the §5 runtime-input mechanism). Pins the CRITICAL contract
/// (fixed after v-guide-infra found the starter erroring): the cell's def-block goes in the BUFFER, and
/// `entry` is a CALL to the cell's entry point (an EXPRESSION) — never a `def` in the entry slot.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleForRun, widgetBinding, entryName, entryCall } from "./assembleForRun.ts";
import type { Cell, CellDirective } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";

const code = (source: string, directive: CellDirective = { kind: "none" }): Cell => ({ kind: "code", source, directive });
const slider = (name: string, def = 0): Widget => ({ name, type: "Float64", control: "slider", min: 0, max: 100, step: 1, default: def });

test("widgetBinding emits the surface-appropriate def with a grounded Float64 literal", () => {
  assert.equal(widgetBinding(slider("rate"), 10, "ml"), "def rate = 10.0");
  assert.equal(widgetBinding(slider("rate"), 10, "sexpr"), "(def (rate) 10.0)");
});

test("entryName picks `main` when defined, else the first def; entryCall is a surface-appropriate call", () => {
  assert.equal(entryName("def main() = 1", "ml"), "main");
  assert.equal(entryName("def helper() = 1", "ml"), "helper");
  assert.equal(entryName("(def (main) 1)", "sexpr"), "main");
  assert.equal(entryCall("main", "ml"), "main()");
  assert.equal(entryCall("main", "sexpr"), "(main)");
});

test("the ENTRY is a call to main (an expression), the cell's def-block goes in the BUFFER", () => {
  const cells: Cell[] = [
    code("principal : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def main() = principal * 2.0"),
  ];
  const r = assembleForRun(cells, 1, [slider("principal")], { principal: 25 }, "ml");
  // widget binding + the cell's own def both in the buffer; entry is a call (NOT a def).
  assert.equal(r.buffer, "def principal = 25.0\n\ndef main() = principal * 2.0");
  assert.equal(r.entry, "main()");
  assert.ok(!/^def\b/.test(r.entry), "entry must not be a def-block");
});

test("s-expr: the cell def goes in the buffer, entry is `(main)`", () => {
  const cells: Cell[] = [code("(def (main) (* 1000.0 (+ 1.0 rate)))")];
  const r = assembleForRun(cells, 0, [], {}, "sexpr");
  assert.equal(r.buffer, "(def (main) (* 1000.0 (+ 1.0 rate)))");
  assert.equal(r.entry, "(main)");
});

test("an absent widget value falls back to the widget's declared default", () => {
  const cells: Cell[] = [
    code("k : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def main() = k"),
  ];
  const r = assembleForRun(cells, 1, [slider("k", 7)], {}, "ml");
  assert.equal(r.buffer, "def k = 7.0\n\ndef main() = k");
  assert.equal(r.entry, "main()");
});

test("buffer order: widget bindings → prior-cell scope → this cell's own def", () => {
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def base = 100.0"),
    code("def main() = base * rate"),
  ];
  const r = assembleForRun(cells, 2, [slider("rate", 0.5)], { rate: 0.5 }, "ml");
  assert.equal(r.buffer, "def rate = 0.5\n\ndef base = 100.0\n\ndef main() = base * rate");
  assert.equal(r.entry, "main()");
});

test("a cell defining a non-main entry: entry calls that def", () => {
  const cells: Cell[] = [code("def total = 42")];
  const r = assembleForRun(cells, 0, [], {}, "ml");
  assert.equal(r.buffer, "def total = 42");
  assert.equal(r.entry, "total()");
});

test("multiple widgets bind in list order (all in the buffer, before the cell def)", () => {
  const cells: Cell[] = [
    code("a : Float64 = slider(0,1)\nb : Float64 = slider(0,1)", { kind: "widget" }),
    code("def main() = a + b"),
  ];
  const r = assembleForRun(cells, 1, [slider("a", 1), slider("b", 2)], { a: 1, b: 2 }, "ml");
  assert.equal(r.buffer, "def a = 1.0\ndef b = 2.0\n\ndef main() = a + b");
  assert.equal(r.entry, "main()");
});
