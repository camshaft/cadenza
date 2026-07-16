/// Unit tests for assembleForRun — the seam that folds current widget values into a cell's run buffer
/// (the §5 runtime-input mechanism). Pins that widget bindings are prepended in the right surface syntax,
/// prior-cell scope follows, absent values fall back to the widget default. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleForRun, widgetBinding } from "./assembleForRun.ts";
import type { Cell, CellDirective } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";

const code = (source: string, directive: CellDirective = { kind: "none" }): Cell => ({ kind: "code", source, directive });
const slider = (name: string, def = 0): Widget => ({ name, type: "Float64", control: "slider", min: 0, max: 100, step: 1, default: def });

test("widgetBinding emits the surface-appropriate def with a grounded Float64 literal", () => {
  assert.equal(widgetBinding(slider("rate"), 10, "ml"), "def rate = 10.0");
  assert.equal(widgetBinding(slider("rate"), 10, "sexpr"), "(def (rate) 10.0)");
});

test("current widget values are prepended to the buffer; entry is the cell source", () => {
  const cells: Cell[] = [
    code("principal : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def main() = principal * 2.0"),
  ];
  const r = assembleForRun(cells, 1, [slider("principal")], { principal: 25 }, "ml");
  assert.equal(r.buffer, "def principal = 25.0");
  assert.equal(r.entry, "def main() = principal * 2.0");
});

test("an absent widget value falls back to the widget's declared default", () => {
  const cells: Cell[] = [
    code("k : Float64 = slider(0, 100)", { kind: "widget" }),
    code("def main() = k"),
  ];
  const r = assembleForRun(cells, 1, [slider("k", 7)], {}, "ml");
  assert.equal(r.buffer, "def k = 7.0");
});

test("widget bindings come BEFORE prior-cell scope in the buffer", () => {
  const cells: Cell[] = [
    code("rate : Float64 = slider(0, 1)", { kind: "widget" }),
    code("def base = 100.0"),
    code("def main() = base * rate"),
  ];
  const r = assembleForRun(cells, 2, [slider("rate", 0.5)], { rate: 0.5 }, "ml");
  assert.equal(r.buffer, "def rate = 0.5\n\ndef base = 100.0");
  assert.equal(r.entry, "def main() = base * rate");
});

test("no widgets → buffer is just the prior-cell scope", () => {
  const cells: Cell[] = [code("def a = 1"), code("def main() = a")];
  const r = assembleForRun(cells, 1, [], {}, "ml");
  assert.equal(r.buffer, "def a = 1");
});

test("multiple widgets bind in list order", () => {
  const cells: Cell[] = [
    code("a : Float64 = slider(0,1)\nb : Float64 = slider(0,1)", { kind: "widget" }),
    code("def main() = a + b"),
  ];
  const r = assembleForRun(cells, 1, [slider("a", 1), slider("b", 2)], { a: 1, b: 2 }, "ml");
  assert.equal(r.buffer, "def a = 1.0\ndef b = 2.0");
});
