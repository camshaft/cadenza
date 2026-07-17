/// Unit tests for the chart data extractor (Increment 3): shape a rendered s-expr value into numeric
/// series for the hand-rolled SVG chart renderer. Pins the (x,y) tuple case, bare-number index series,
/// categorical (labelled) x, multi-series rows, rationals, and the typed non-chartable fallback.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { extractChart, categoryLabels } from "./extractChart.ts";

test("a List of (x, y) tuples → one unnamed series of points", () => {
  const r = extractChart("(: (list (tuple 1 10) (tuple 2 20) (tuple 3 15)) T)");
  assert.ok(r.ok);
  assert.equal(r.series.length, 1);
  assert.equal(r.series[0].name, "");
  assert.deepEqual(r.series[0].points, [
    { x: 1, y: 10 },
    { x: 2, y: 20 },
    { x: 3, y: 15 },
  ]);
});

test("a List of bare numbers → a series of (index, y)", () => {
  const r = extractChart("(: (list 5 8 3) (List Int64))");
  assert.ok(r.ok);
  assert.deepEqual(r.series[0].points, [
    { x: 0, y: 5 },
    { x: 1, y: 8 },
    { x: 2, y: 3 },
  ]);
});

test("a non-numeric x becomes a category label; its x is the row index", () => {
  const r = extractChart('(: (list (tuple "jan" 10) (tuple "feb" 20)) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.series[0].points, [
    { x: 0, y: 10, label: "jan" },
    { x: 1, y: 20, label: "feb" },
  ]);
});

test("multi-y tuples → multiple named series sharing the x", () => {
  const r = extractChart("(: (list (tuple 1 10 100) (tuple 2 20 200)) T)");
  assert.ok(r.ok);
  assert.equal(r.series.length, 2);
  assert.deepEqual(r.series[0], { name: "y0", points: [{ x: 1, y: 10 }, { x: 2, y: 20 }] });
  assert.deepEqual(r.series[1], { name: "y1", points: [{ x: 1, y: 100 }, { x: 2, y: 200 }] });
});

test("rational coordinates evaluate to numbers (n/d)", () => {
  const r = extractChart("(: (list (tuple 1/2 3/4)) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.series[0].points, [{ x: 0.5, y: 0.75 }]);
});

test("an empty list → no series (empty chart, not an error)", () => {
  const r = extractChart("(: (list) (List Int64))");
  assert.ok(r.ok);
  assert.deepEqual(r.series, []);
});

test("a non-list / unparseable value → typed fallback, never throws", () => {
  assert.equal(extractChart("(: 42 Int64)").ok, false);
  assert.equal(extractChart("(list 1 2").ok, false); // unclosed
});

test("mixed rows (a number and a tuple) → typed fallback", () => {
  const r = extractChart("(: (list 1 (tuple 2 3)) T)");
  assert.equal(r.ok, false);
});

test("a tuple with only an x (no y) → typed fallback", () => {
  const r = extractChart("(: (list (tuple 1) (tuple 2)) T)");
  assert.equal(r.ok, false);
});

test("ragged multi-y tuples use the shared (minimum) y-column count", () => {
  // First row has 2 ys, second has 1 → only 1 shared series.
  const r = extractChart("(: (list (tuple 1 10 100) (tuple 2 20)) T)");
  assert.ok(r.ok);
  assert.equal(r.series.length, 1);
  assert.deepEqual(r.series[0].points, [{ x: 1, y: 10 }, { x: 2, y: 20 }]);
});

test("a List of records → x = first field, y-series = subsequent numeric fields (named)", () => {
  const r = extractChart("(: (list (record (year 1) (bal 10)) (record (year 2) (bal 20))) T)");
  assert.ok(r.ok);
  assert.equal(r.series.length, 1);
  assert.equal(r.series[0].name, ""); // single y-series is unnamed
  assert.deepEqual(r.series[0].points, [{ x: 1, y: 10 }, { x: 2, y: 20 }]);
});

test("a record list with two numeric y fields → two named series sharing x", () => {
  const r = extractChart("(: (list (record (x 1) (a 10) (b 100)) (record (x 2) (a 20) (b 200))) T)");
  assert.ok(r.ok);
  assert.deepEqual(r.series.map((s) => s.name), ["a", "b"]);
  assert.deepEqual(r.series[0].points, [{ x: 1, y: 10 }, { x: 2, y: 20 }]);
  assert.deepEqual(r.series[1].points, [{ x: 1, y: 100 }, { x: 2, y: 200 }]);
});

test("a record list with a non-numeric first field → category label, x = row index", () => {
  const r = extractChart('(: (list (record (month "jan") (n 10)) (record (month "feb") (n 20))) T)');
  assert.ok(r.ok);
  assert.deepEqual(r.series[0].points, [
    { x: 0, y: 10, label: "jan" },
    { x: 1, y: 20, label: "feb" },
  ]);
});

test("a record with only one field (no y) → typed fallback", () => {
  const r = extractChart("(: (list (record (x 1)) (record (x 2))) T)");
  assert.equal(r.ok, false);
});

// ── categoryLabels: the x-axis category names for a labelled (categorical) chart ──

test("categoryLabels returns null for a purely numeric chart (no labels → keep numeric ticks)", () => {
  const r = extractChart("(: (list (tuple 1 10) (tuple 2 20)) T)");
  assert.ok(r.ok);
  assert.equal(categoryLabels(r.series), null);
});

test("categoryLabels lists the category names in x-slot order for a labelled chart", () => {
  const r = extractChart('(: (list (tuple "jan" 10) (tuple "feb" 20) (tuple "mar" 15)) T)');
  assert.ok(r.ok);
  assert.deepEqual(categoryLabels(r.series), ["jan", "feb", "mar"]);
});

test("categoryLabels aligns labels across multiple series sharing the same categorical x", () => {
  const r = extractChart('(: (list (tuple "q1" 10 100) (tuple "q2" 20 200)) T)');
  assert.ok(r.ok);
  assert.equal(r.series.length, 2);
  assert.deepEqual(categoryLabels(r.series), ["q1", "q2"]);
});

test("categoryLabels works for a record-list categorical chart", () => {
  const r = extractChart('(: (list (record (month "jan") (n 10)) (record (month "feb") (n 20))) T)');
  assert.ok(r.ok);
  assert.deepEqual(categoryLabels(r.series), ["jan", "feb"]);
});

test("categoryLabels fills a label-less x-slot with its bare index (defensive)", () => {
  // Hand-built series with a gap at x=1: slot 1 has no label → falls back to "1".
  const series = [{ name: "", points: [{ x: 0, y: 5, label: "a" }, { x: 2, y: 7, label: "c" }] }];
  assert.deepEqual(categoryLabels(series), ["a", "1", "c"]);
});
