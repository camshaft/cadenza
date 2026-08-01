/// Unit tests for the table extractor (Increment 3): shape a rendered s-expr value (List of tuples /
/// records / scalars) into { columns, rows }. Pins the canonical-form parse, positional vs named
/// columns, ragged rows, and the typed fallback when a value isn't tabular. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { extractTable } from "./extractTable.ts";

test("a List of tuples → positional columns, one row per tuple", () => {
  const r = extractTable("(: (list (tuple 1 100) (tuple 2 121) (tuple 3 133)) (List (Tuple Int64 Int64)))");
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["col 0", "col 1"]);
  assert.deepEqual(r.table.rows, [["1", "100"], ["2", "121"], ["3", "133"]]);
});

test("a List of records → named columns (union of fields, first-seen order)", () => {
  const r = extractTable('(: (list (record (year 1) (balance 105)) (record (year 2) (balance 110))) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["year", "balance"]);
  assert.deepEqual(r.table.rows, [["1", "105"], ["2", "110"]]);
});

test("records with differing fields → union columns; missing cells render empty", () => {
  const r = extractTable('(: (list (record (a 1)) (record (a 2) (b 9))) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["a", "b"]);
  assert.deepEqual(r.table.rows, [["1", ""], ["2", "9"]]);
});

test("string cells lose their quotes in the rendered table", () => {
  const r = extractTable('(: (list (record (name "Ada") (n 1)) (record (name "Bob") (n 2))) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.table.rows, [["Ada", "1"], ["Bob", "2"]]);
});

test("ragged tuple rows pad to the widest row with empty cells", () => {
  const r = extractTable("(: (list (tuple 1) (tuple 2 3 4)) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["col 0", "col 1", "col 2"]);
  assert.deepEqual(r.table.rows, [["1", "", ""], ["2", "3", "4"]]);
});

test("a List of scalars → a single `value` column", () => {
  const r = extractTable("(: (list 10 20 30) (List Int64))");
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["value"]);
  assert.deepEqual(r.table.rows, [["10"], ["20"], ["30"]]);
});

test("an empty list → an empty table (no columns, no rows)", () => {
  const r = extractTable("(: (list) (List Int64))");
  assert.ok(r.ok);
  assert.deepEqual(r.table, { columns: [], rows: [] });
});

test("a nested compound cell renders compactly rather than breaking the table", () => {
  const r = extractTable("(: (list (tuple 1 (tuple 2 3))) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.table.rows, [["1", "(tuple 2 3)"]]);
});

test("a quantity cell renders friendly (`5 meter`, not `(quantity 5 meter)`) — shared displayNode", () => {
  const r = extractTable("(: (list (tuple 1 (quantity 5 meter))) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.table.rows, [["1", "5 meter"]]);
});

test("a REAL runtime quantity `(Qty.of 5 (Unit.base #\"meter\"))` in a table renders friendly `5 meter`", () => {
  // The runtime emits the `Qty.of` shape (not `(quantity …)`); a table column of quantities must still
  // read `5 meter`, not the raw `(Qty.of 5 (Unit.base # meter))`.
  const r = extractTable('(: (list (tuple 1 (Qty.of 5 (Unit.base #"meter")))) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.table.rows, [["1", "5 meter"]]);
});

test("a non-list value → typed fallback (not a table), never throws", () => {
  const scalar = extractTable("(: 42 Int64)");
  assert.equal(scalar.ok, false);
  const unparseable = extractTable("(list 1 2"); // unclosed
  assert.equal(unparseable.ok, false);
});

test("mixed row shapes (tuple + record) → typed fallback", () => {
  const r = extractTable("(: (list (tuple 1 2) (record (a 1))) T)");
  assert.equal(r.ok, false);
});

test("a BARE record (not in a list) → a single-row table (fields = columns)", () => {
  const r = extractTable("(: (record (name \"Ada\") (age 36)) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.table.columns, ["name", "age"]);
  assert.deepEqual(r.table.rows, [["Ada", "36"]]);
});

test("a bare non-record scalar is still NOT a table (value fallback)", () => {
  assert.equal(extractTable("(: 42 Int64)").ok, false);
});
