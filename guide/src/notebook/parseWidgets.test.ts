/// Unit tests for the widget declaration DSL (design D2) + the current-value→Cadenza-binding splice (the
/// novel reactive-input mechanism, §5). Pins the `name : Type = control(...)` surface, per-control config,
/// error collection, and — critically — that a Float64 widget value grounds with a decimal point (else it
/// would compile as Int64). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseWidgets, bindingFor, literalFor, type Widget } from "./parseWidgets.ts";

test("a slider parses min/max + named step/default", () => {
  const { widgets, errors } = parseWidgets("principal : Float64 = slider(1000, 100000, step: 1000, default: 10000)");
  assert.deepEqual(errors, []);
  assert.deepEqual(widgets[0], {
    name: "principal", type: "Float64", control: "slider",
    min: 1000, max: 100000, step: 1000, default: 10000,
  });
});

test("a slider defaults step (Int64→1, Float64→range/100) and default (→min) when omitted", () => {
  const int = parseWidgets("years : Int64 = slider(1, 30)").widgets[0] as Extract<Widget, { control: "slider" }>;
  assert.equal(int.step, 1);
  assert.equal(int.default, 1); // → min
  const flt = parseWidgets("rate : Float64 = slider(0, 100)").widgets[0] as Extract<Widget, { control: "slider" }>;
  assert.equal(flt.step, 1); // (100-0)/100
});

test("a slider with an inverted or zero-width range is rejected (needs max > min)", () => {
  const inv = parseWidgets("x : Float64 = slider(100, 0)");
  assert.equal(inv.widgets.length, 0);
  assert.match(inv.errors[0].message, /max .* greater than min/);
  const zero = parseWidgets("x : Float64 = slider(5, 5)");
  assert.equal(zero.widgets.length, 0);
  assert.match(zero.errors[0].message, /greater than min/);
});

test("a slider's step is forced positive — a declared step <= 0 falls back to the default", () => {
  const neg = parseWidgets("x : Float64 = slider(0, 10, step: -1)").widgets[0] as Extract<Widget, { control: "slider" }>;
  assert.equal(neg.step, 0.1); // (10-0)/100, not the negative
  const zero = parseWidgets("y : Int64 = slider(0, 10, step: 0)").widgets[0] as Extract<Widget, { control: "slider" }>;
  assert.equal(zero.step, 1); // Int64 default
  const ok = parseWidgets("z : Float64 = slider(0, 10, step: 0.5)").widgets[0] as Extract<Widget, { control: "slider" }>;
  assert.equal(ok.step, 0.5); // a valid positive step is kept
});

test("text / checkbox / dropdown parse their type-appropriate defaults", () => {
  assert.deepEqual(parseWidgets('label : String = text(default: "balance")').widgets[0], {
    name: "label", type: "String", control: "text", default: "balance",
  });
  assert.deepEqual(parseWidgets("on : Bool = checkbox(default: true)").widgets[0], {
    name: "on", type: "Bool", control: "checkbox", default: true,
  });
  assert.deepEqual(parseWidgets('mode : String = dropdown("annual", "monthly", default: "monthly")').widgets[0], {
    name: "mode", type: "String", control: "dropdown", options: ["annual", "monthly"], default: "monthly",
  });
});

test("dropdown default falls back to the first option when the declared default isn't an option", () => {
  const w = parseWidgets('m : String = dropdown("a", "b", default: "z")').widgets[0] as Extract<Widget, { control: "dropdown" }>;
  assert.equal(w.default, "a");
});

test("radio parses like dropdown — same String single-choice shape, control: radio", () => {
  assert.deepEqual(parseWidgets('pick : String = radio("x", "y", default: "y")').widgets[0], {
    name: "pick", type: "String", control: "radio", options: ["x", "y"], default: "y",
  });
  // default falls back to the first option when not one of the options
  const w = parseWidgets('r : String = radio("a", "b", default: "z")').widgets[0] as Extract<Widget, { control: "radio" }>;
  assert.equal(w.default, "a");
  // a non-String type is rejected with a radio-specific message
  const err = parseWidgets("bad : Int64 = radio(\"a\")").errors[0];
  assert.match(err.message, /radio\(\.\.\.\) produces a String/);
});

test("multiple widgets in one cell parse in order; comments + blanks are skipped", () => {
  const src = [
    "-- inputs",
    "x : Int64 = slider(0, 10)",
    "",
    "# another",
    "y : Float64 = slider(0, 1)",
  ].join("\n");
  const { widgets, errors } = parseWidgets(src);
  assert.deepEqual(errors, []);
  assert.deepEqual(widgets.map((w) => w.name), ["x", "y"]);
});

test("errors are collected per-line, not thrown; valid lines still parse", () => {
  const src = [
    "good : Int64 = slider(0, 10)",
    "bad line with no equals",
    "wrongtype : Frobnicate = slider(0, 1)",
    "text-needs-string : Int64 = text(default: \"x\")",
    "also-good : Bool = checkbox(default: false)",
  ].join("\n");
  const { widgets, errors } = parseWidgets(src);
  assert.deepEqual(widgets.map((w) => w.name), ["good", "also-good"]);
  assert.equal(errors.length, 3);
  assert.equal(errors[0].line, 2);
  assert.match(errors[1].message, /unknown type/);
  assert.match(errors[2].message, /String/);
});

test("bindingFor emits a `def name = <literal>` line the assembler can splice", () => {
  const slider = parseWidgets("p : Float64 = slider(0, 100, default: 42)").widgets[0];
  assert.equal(bindingFor(slider, 42), "def p = 42.0");
});

test("Float64 literals ALWAYS carry a decimal point (else they'd ground to Int64)", () => {
  assert.equal(literalFor("Float64", 10), "10.0");
  assert.equal(literalFor("Float64", 10.5), "10.5");
  assert.equal(literalFor("Float64", 0), "0.0");
});

test("Int64 literals are bare integers; a fractional current value truncates", () => {
  assert.equal(literalFor("Int64", 7), "7");
  assert.equal(literalFor("Int64", 7.9), "7");
});

test("literalFor never emits invalid Cadenza for non-finite / huge numbers (self-audit)", () => {
  // NaN / ±Infinity would emit `def x = NaN`/`Infinity` — not valid Cadenza. Clamp to a safe default.
  assert.equal(literalFor("Float64", NaN), "0.0");
  assert.equal(literalFor("Float64", Infinity), "0.0");
  assert.equal(literalFor("Float64", -Infinity), "0.0");
  assert.equal(literalFor("Int64", NaN), "0");
  assert.equal(literalFor("Int64", Infinity), "0");
  // A large Int64 must render in FULL, not exponential (`1e+21` isn't a Cadenza literal).
  assert.equal(literalFor("Int64", 1e21), "1000000000000000000000");
  assert.match(literalFor("Int64", 1e21), /^\d+$/);
});

test("Bool literals are true/false; String literals are quoted + escaped", () => {
  assert.equal(literalFor("Bool", true), "true");
  assert.equal(literalFor("Bool", false), "false");
  assert.equal(literalFor("String", "hi"), '"hi"');
  assert.equal(literalFor("String", 'a "quote" and a \\ slash'), '"a \\"quote\\" and a \\\\ slash"');
});

test("a dropdown option containing a comma splits correctly (quote-aware arg split)", () => {
  const w = parseWidgets('m : String = dropdown("a, b", "c")').widgets[0] as Extract<Widget, { control: "dropdown" }>;
  assert.deepEqual(w.options, ["a, b", "c"]);
});

// ── PR #474 hardening: reject invalid Cadenza widget names; escape-aware arg lexing ──

test("invalid Cadenza widget names are rejected (PR #474): `.`, doubled `-`, trailing `-`", () => {
  // A widget name flows into `def <name>` via bindingFor, so it must be a valid Cadenza binding ident.
  for (const bad of ["a.b", "a--b", "rate-", "-rate", "x.y.z"]) {
    const { widgets, errors } = parseWidgets(`${bad} : Int64 = slider(0, 10)`);
    assert.equal(widgets.length, 0, `expected ${bad} rejected`);
    assert.equal(errors.length, 1);
    assert.match(errors[0].message, /not a valid widget name/);
  }
});

test("valid kebab widget names are still accepted", () => {
  for (const ok of ["rate", "rate-adjusted", "x", "_priv", "a-b-c", "p2"]) {
    const { widgets, errors } = parseWidgets(`${ok} : Int64 = slider(0, 10)`);
    assert.deepEqual(errors, [], `expected ${ok} accepted`);
    assert.equal(widgets[0].name, ok);
  }
});

test("a widget named `main` is rejected (reserved per-cell entry slot → would collide as CDZ0201)", () => {
  // `main` is a valid IDENT, so the generic name check passes it — but it's every cell's reserved entry
  // point. A widget `main` would splice `def (main) = <value>` into the buffer alongside the cell's own
  // `def (main)` → two `main`s in one module → CDZ0201 at run time. parseWidgets must reject it up front
  // with an actionable message (mirrors the run-path P0 #12 discipline: `main` is not a shareable name).
  const { widgets, errors } = parseWidgets("main : Float64 = slider(0, 1)");
  assert.equal(widgets.length, 0, "widget named `main` is not produced");
  assert.equal(errors.length, 1);
  assert.match(errors[0].message, /`main` is reserved/);
  // A NON-main name that merely contains "main" is fine (matched exactly, not as a substring).
  assert.deepEqual(parseWidgets("main-rate : Float64 = slider(0, 1)").errors, []);
  assert.deepEqual(parseWidgets("mainline : Float64 = slider(0, 1)").errors, []);
});

test("escaped quotes inside a dropdown option split + unescape correctly (PR #474)", () => {
  const w = parseWidgets('m : String = dropdown("a \\"q\\" opt", "b")').widgets[0] as Extract<Widget, { control: "dropdown" }>;
  // Two options, not four; the first has its inner quotes unescaped.
  assert.deepEqual(w.options, ['a "q" opt', "b"]);
});

test("a text default containing escaped quotes + a backslash round-trips (PR #474)", () => {
  const w = parseWidgets('t : String = text(default: "a \\"q\\" and \\\\ slash")').widgets[0] as Extract<Widget, { control: "text" }>;
  assert.equal(w.default, 'a "q" and \\ slash');
});

test("a DUPLICATE widget name is an error (would emit two top-level bindings → CDZ0201); first is kept", () => {
  // Each widget splices `def <name> = …` into the run buffer, so two same-named widgets compile to a duplicate
  // definition — flag it in the IDE (like min>max) instead of a mysterious runtime compile failure.
  const { widgets, errors } = parseWidgets("a : Int64 = slider(0, 1)\na : Int64 = slider(0, 2)");
  assert.equal(widgets.length, 1, "only the FIRST `a` is kept");
  assert.equal(widgets[0].max, 1, "the kept widget is the first declaration");
  assert.equal(errors.length, 1);
  assert.match(errors[0].message, /duplicate widget name `a`/);
  assert.equal(errors[0].line, 2, "the error points at the SECOND (duplicate) line");
});

test("distinct widget names across lines all parse (the duplicate check doesn't false-positive)", () => {
  const { widgets, errors } = parseWidgets("a : Int64 = slider(0, 1)\nb : Int64 = slider(0, 2)");
  assert.deepEqual(errors, []);
  assert.deepEqual(widgets.map((w) => w.name), ["a", "b"]);
});
