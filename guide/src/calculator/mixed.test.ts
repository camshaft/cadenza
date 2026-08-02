/// Unit tests for the calculator's mixed-number render (`toMixed`) — an improper bare rational shown as
/// the explicit-plus mixed form `3 + 11/12` (operator's Q-b ruling). Pins the split arithmetic, the
/// symmetric negative case, and the pass-through of everything that ISN'T a bare improper rational.
/// The round-trip property (the rendered form re-parses to the same rational) is pinned e2e against the
/// real compiler in check-calculator.mjs; this file pins the pure string transform. Run with
/// `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { toMixed } from "./mixed.ts";

test("toMixed: an improper positive rational splits into explicit-plus mixed form", () => {
  assert.equal(toMixed("47/12"), "3 + 11/12");
  assert.equal(toMixed("7/2"), "3 + 1/2");
  assert.equal(toMixed("5/4"), "1 + 1/4");
});

test("toMixed: the negative case is symmetric (sign on both parts), still valid + pasteable", () => {
  // -47/12 → -3 + -11/12, which re-evaluates to -3 + (-11/12) = -47/12. A single signed trunc/rem
  // formula, no special-casing.
  assert.equal(toMixed("-47/12"), "-3 + -11/12");
  assert.equal(toMixed("-7/2"), "-3 + -1/2");
});

test("toMixed: a proper fraction is already simplest — unchanged", () => {
  assert.equal(toMixed("1/3"), "1/3");
  assert.equal(toMixed("-5/8"), "-5/8");
  assert.equal(toMixed("11/12"), "11/12");
});

test("toMixed: a whole rational (n/1) is left as-is (not a fraction to split)", () => {
  // The display surface already collapses whole rationals elsewhere, but the guard makes this total.
  assert.equal(toMixed("5/1"), "5/1");
  assert.equal(toMixed("-3/1"), "-3/1");
});

test("toMixed: non-bare-rational displays pass through untouched", () => {
  assert.equal(toMixed("42"), "42"); // bare integer, no slash
  assert.equal(toMixed("47/12 meter"), "47/12 meter"); // a quantity (space + unit) — never split
  assert.equal(toMixed("(tuple 1 2)"), "(tuple 1 2)"); // a compound
  assert.equal(toMixed("1500 meter/second"), "1500 meter/second"); // rate-unit, not a bare rational
  assert.equal(toMixed(""), ""); // empty
});

test("toMixed: arbitrary-precision numerators keep every digit (BigInt, not i64)", () => {
  // 100000000000000000001 / 3 = 33333333333333333333 + 2/3 — a numerator well past 2^63.
  assert.equal(toMixed("100000000000000000001/3"), "33333333333333333333 + 2/3");
});

test("toMixed: a zero denominator passes through — total, never divides by zero", () => {
  // The compiler never emits `n/0`, but toMixed is exported/called on arbitrary strings, so it must not
  // throw on one. BARE_RATIONAL matches `1/0`; without the guard, `n / d` would be a BigInt div-by-zero.
  assert.equal(toMixed("1/0"), "1/0");
  assert.equal(toMixed("-5/0"), "-5/0");
});
