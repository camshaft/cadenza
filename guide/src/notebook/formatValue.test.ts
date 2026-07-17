/// Unit tests for the friendly value formatter: a rendered s-expr `(: value type)` → human-readable
/// display text (bare scalar, bare rational, unquoted string, `n unit` quantity), with compound values
/// kept canonical and unparseable input passed through unchanged. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { formatValue } from "./formatValue.ts";

test("a scalar drops the `(: value type)` ascription → bare value", () => {
  assert.equal(formatValue("(: 42 Int64)"), "42");
  assert.equal(formatValue("(: true Bool)"), "true");
  assert.equal(formatValue("(: 3.14 Float64)"), "3.14");
});

test("a rational renders bare", () => {
  assert.equal(formatValue("(: 5/2 Rational)"), "5/2");
  assert.equal(formatValue("(: 127/20 Rational)"), "127/20");
});

test("a whole-valued rational `n/1` collapses to a plain integer (matches formula-cell display)", () => {
  assert.equal(formatValue("(: 4/1 Rational)"), "4");
  assert.equal(formatValue("(: -4/1 Rational)"), "-4");
  assert.equal(formatValue("9/1"), "9"); // bare, un-ascribed
  // a quantity whose value is a whole rational collapses its value part too
  assert.equal(formatValue("(: (quantity 4/1 meter) T)"), "4 meter");
  // a genuine fraction (den != 1) is untouched
  assert.equal(formatValue("(: 4/2 Rational)"), "4/2");
});

test("a String that LOOKS like n/1 is NOT collapsed — only a genuine Rational is (PR #523)", () => {
  // A String value `(: "3/1" String)` must keep its text `3/1` (quotes stripped) — the n/1-collapse must
  // NOT corrupt it to `3`. The collapse only applies to a bare (Rational) atom, not unquoted String text.
  assert.equal(formatValue('(: "3/1" String)'), "3/1");
  assert.equal(formatValue('(: "4/1" String)'), "4/1");
  assert.equal(formatValue('(: "-5/1" String)'), "-5/1");
  // …while the genuine Rational 3/1 still collapses to 3.
  assert.equal(formatValue("(: 3/1 Rational)"), "3");
});

test("a string loses its quotes", () => {
  assert.equal(formatValue('(: "hello" String)'), "hello");
  assert.equal(formatValue('(: "a \\"q\\" b" String)'), 'a "q" b');
});

test("a quantity renders as `<value> <unit>`", () => {
  assert.equal(formatValue("(: (quantity 2192 meter) (Quantity Int64))"), "2192 meter");
  assert.equal(formatValue("(: (quantity 5/2 second) T)"), "5/2 second");
});

test("a bare (un-ascribed) value formats too", () => {
  assert.equal(formatValue("42"), "42");
  assert.equal(formatValue('"hi"'), "hi");
});

test("a compound value (list/tuple/record) keeps a canonical compact render", () => {
  assert.equal(formatValue("(: (list 1 2 3) (List Int64))"), "(list 1 2 3)");
  assert.equal(formatValue("(: (tuple 1 2) T)"), "(tuple 1 2)");
});

test("unparseable input is returned unchanged (never throws)", () => {
  assert.equal(formatValue("(: 1 2"), "(: 1 2"); // unclosed
  assert.equal(formatValue(""), "");
});
