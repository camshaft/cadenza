/// Unit tests for the /cad single-mode manifest→slider bridge (`manifestSlider.ts`) — the conversion that
/// turns a compiled model's live `@param` manifest into the `ParamSlider` shape the exact-fraction sliders
/// render. Pins: integer vs fractional step derivation from the declared type (v-cad's exact-Rational UX),
/// bound/default string parsing, and the synthesized-range fallback so a control always renders. Run with
/// `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import {
  parseRational,
  isFractionalType,
  sliderFromManifest,
  slidersFromManifest,
} from "./manifestSlider.ts";
import type { ParamManifestEntry } from "../compiler/client.ts";

test("parseRational: integers, fractions, and unparseable", () => {
  assert.deepEqual(parseRational("50"), [50, 1]);
  assert.deepEqual(parseRational("-3"), [-3, 1]);
  assert.deepEqual(parseRational("7/2"), [7, 2]);
  assert.deepEqual(parseRational("-1/4"), [-1, 4]);
  assert.deepEqual(parseRational(" 5 / 2 "), [5, 2]);
  assert.equal(parseRational("1/0"), null); // no div by zero
  assert.equal(parseRational("((. Rational of) 1 4)"), null); // source-expr render → fallback
  assert.equal(parseRational(undefined), null);
});

test("isFractionalType: Rational-family gets fractional steps, Int64 does not", () => {
  assert.equal(isFractionalType("Rational"), true);
  assert.equal(isFractionalType("(Qty Rational meter)"), true);
  assert.equal(isFractionalType("Length"), true);
  assert.equal(isFractionalType("Int64"), false);
  assert.equal(isFractionalType("Int32"), false);
});

test("sliderFromManifest: an Int64 param → integer slider with parsed bounds", () => {
  const e: ParamManifestEntry = { name: "count", typeName: "Int64", rangeLo: "1", rangeHi: "10", default: "3" };
  const s = sliderFromManifest(e);
  assert.equal(s.name, "count");
  assert.equal(s.fractional, false);
  assert.deepEqual(s.min, [1, 1]);
  assert.deepEqual(s.max, [10, 1]);
  assert.deepEqual(s.default, [3, 1]);
});

test("sliderFromManifest: a Rational param → fractional slider (the exact-fraction payoff)", () => {
  const e: ParamManifestEntry = { name: "thickness", typeName: "Rational", rangeLo: "2", rangeHi: "20", default: "7/2" };
  const s = sliderFromManifest(e);
  assert.equal(s.fractional, true);
  assert.deepEqual(s.default, [7, 2]); // 3.5 held exactly
});

test("sliderFromManifest: derives a readable label from a kebab/snake name", () => {
  assert.equal(sliderFromManifest({ name: "bore-radius", typeName: "Int64" }).label, "Bore radius");
  assert.equal(sliderFromManifest({ name: "wall_thickness", typeName: "Rational" }).label, "Wall thickness");
});

test("sliderFromManifest: NO range config → synthesizes a range around the default so a control still renders", () => {
  const s = sliderFromManifest({ name: "w", typeName: "Int64", default: "50" });
  assert.deepEqual(s.default, [50, 1]);
  assert.ok(s.min[0] / s.min[1] <= 50 && s.max[0] / s.max[1] >= 50, "range brackets the default");
});

test("sliderFromManifest: NO default → default falls back to the range midpoint", () => {
  const s = sliderFromManifest({ name: "w", typeName: "Int64", rangeLo: "10", rangeHi: "20" });
  assert.deepEqual(s.default, [15, 1]); // midpoint of 10..20
});

test("sliderFromManifest: NO range AND no default → neutral 0..100, default midpoint", () => {
  const s = sliderFromManifest({ name: "w", typeName: "Int64" });
  assert.deepEqual(s.min, [0, 1]);
  assert.deepEqual(s.max, [100, 1]);
  assert.deepEqual(s.default, [50, 1]);
});

test("slidersFromManifest: preserves declaration order; empty → no sliders", () => {
  const entries: ParamManifestEntry[] = [
    { name: "a", typeName: "Int64", rangeLo: "0", rangeHi: "5", default: "1" },
    { name: "b", typeName: "Rational", rangeLo: "0", rangeHi: "5", default: "1/2" },
  ];
  const sliders = slidersFromManifest(entries);
  assert.deepEqual(sliders.map((s) => s.name), ["a", "b"]);
  assert.deepEqual(slidersFromManifest([]), []);
});
