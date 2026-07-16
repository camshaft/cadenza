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
