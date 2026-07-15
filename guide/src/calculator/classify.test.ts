/// Unit tests for the calculator's line classifier — assignment (`name = expr`) vs bare expression.
/// This mirrors the native cdz-calc crate's `classify`; the two calculators must agree, so the subtle
/// `=` vs `==`/`<=`/`>=`/`!=` distinction is worth pinning. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { classify, isIdentifier } from "./classify.ts";

test("a clean `name = expr` is an assignment", () => {
  assert.deepEqual(classify("x = 2 + 3"), { kind: "assign", name: "x", expr: "2 + 3" });
  assert.deepEqual(classify("  ans  =  ans + 5 "), { kind: "assign", name: "ans", expr: "ans + 5" });
  // kebab + dotted identifiers are valid names
  assert.deepEqual(classify("c-to-f = 100"), { kind: "assign", name: "c-to-f", expr: "100" });
});

test("equality and comparison operators are NOT assignments (they're expressions)", () => {
  assert.deepEqual(classify("a == b"), { kind: "expr", expr: "a == b" });
  assert.deepEqual(classify("x <= 5"), { kind: "expr", expr: "x <= 5" });
  assert.deepEqual(classify("x >= 5"), { kind: "expr", expr: "x >= 5" });
  assert.deepEqual(classify("x != 5"), { kind: "expr", expr: "x != 5" });
  // a chain of equalities must not be misread as an assignment
  assert.deepEqual(classify("a == b == c"), { kind: "expr", expr: "a == b == c" });
});

test("a `=` whose left side isn't a single identifier is an expression", () => {
  assert.deepEqual(classify("2 + 3 = 5"), { kind: "expr", expr: "2 + 3 = 5" }); // multi-token LHS
  assert.deepEqual(classify("f(x) = x"), { kind: "expr", expr: "f(x) = x" }); // not a bare ident
});

test("a bare expression with no `=` is an expression", () => {
  assert.deepEqual(classify("2 + 3"), { kind: "expr", expr: "2 + 3" });
  assert.deepEqual(classify("Map.size(m)"), { kind: "expr", expr: "Map.size(m)" });
});

test("an assignment with an empty RHS falls back to an expression", () => {
  assert.deepEqual(classify("x ="), { kind: "expr", expr: "x =" });
});

test("a leading `==` (equality with an empty LHS) stays an expression, not an assignment", () => {
  // prev/next guards mean the first char being part of `==` is treated as a comparison
  assert.deepEqual(classify("== 5"), { kind: "expr", expr: "== 5" });
});

test("isIdentifier accepts kebab/dotted names, rejects leading digit / operators / spaces", () => {
  assert.equal(isIdentifier("x"), true);
  assert.equal(isIdentifier("c-to-f"), true);
  assert.equal(isIdentifier("String.scalar-len"), true);
  assert.equal(isIdentifier("_priv"), true);
  assert.equal(isIdentifier("2x"), false); // leading digit
  assert.equal(isIdentifier("a b"), false); // space
  assert.equal(isIdentifier("a+b"), false); // operator
  assert.equal(isIdentifier(""), false);
});
