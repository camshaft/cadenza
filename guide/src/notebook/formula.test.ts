/// Unit tests for hand-rolled formula classification (concierge-approved option A). Pins the value-shape
/// keying: rational → stacked fraction, quantity → value+unit, other scalar → plain, compound → the
/// surfaced "needs richer rendering" gap (NOT a faked formula). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { classifyFormula } from "./formula.ts";

test("a rational classifies as a stacked fraction (num/den, sign)", () => {
  assert.deepEqual(classifyFormula("(: 5/2 Rational)"), { kind: "fraction", num: "5", den: "2", negative: false });
  assert.deepEqual(classifyFormula("(: -3/4 Rational)"), { kind: "fraction", num: "3", den: "4", negative: true });
});

test("a whole-valued rational `n/1` collapses to a plain integer (not a fraction over 1)", () => {
  assert.deepEqual(classifyFormula("(: 4/1 Rational)"), { kind: "plain", text: "4" });
  assert.deepEqual(classifyFormula("(: -4/1 Rational)"), { kind: "plain", text: "-4" });
  // a bare (un-ascribed) whole rational collapses too
  assert.deepEqual(classifyFormula("9/1"), { kind: "plain", text: "9" });
  // a genuine fraction is still a stacked fraction (denominator != 1)
  assert.deepEqual(classifyFormula("(: 4/2 Rational)"), { kind: "fraction", num: "4", den: "2", negative: false });
});

test("a plain scalar classifies as plain friendly text", () => {
  assert.deepEqual(classifyFormula("(: 42 Int64)"), { kind: "plain", text: "42" });
  assert.deepEqual(classifyFormula("(: 3.14 Float64)"), { kind: "plain", text: "3.14" });
  assert.deepEqual(classifyFormula("(: true Bool)"), { kind: "plain", text: "true" });
  assert.deepEqual(classifyFormula('(: "hi" String)'), { kind: "plain", text: "hi" });
});

test("a quantity classifies as value + unit", () => {
  assert.deepEqual(classifyFormula("(: (quantity 2192 meter) T)"), { kind: "quantity", value: "2192", unit: "meter" });
});

test("a compound value surfaces the gap (unrenderable) — NOT a faked formula", () => {
  const f = classifyFormula("(: (list 1 2 3) (List Int64))");
  assert.equal(f.kind, "unrenderable");
  if (f.kind === "unrenderable") {
    assert.equal(f.text, "(list 1 2 3)");
    assert.match(f.reason, /richer math rendering/);
  }
});

test("unparseable input shows plain (never throws)", () => {
  assert.deepEqual(classifyFormula("(: 1 2"), { kind: "plain", text: "(: 1 2" });
});

test("a bare rational (un-ascribed) still classifies as a fraction", () => {
  assert.deepEqual(classifyFormula("7/8"), { kind: "fraction", num: "7", den: "8", negative: false });
});
