/// Pin the parametric-showcase manifest (the /cad parametric-slider content): every model has both surfaces
/// (ml + sexpr), each source is non-empty, defines `main`, returns `lower(...)`, and imports both `exact` and
/// `helpers` (a parametric model uses the ergonomic helpers); each declares a `@param` per `params[]` entry
/// (the TS manifest and the .cdz annotations agree — the single-source-of-truth invariant v-guide-infra's UI
/// relies on); slugs are unique kebab-case; param bounds are well-formed (min ≤ default ≤ max, den ≠ 0). This
/// guards the manifest /cad reads to build sliders — a model that lost a surface, dropped a param, or whose
/// manifest disagreed with its source would be an authoring bug the /cad slider UI would surface. (The actual
/// compile+host-response run is exercised by check-cad-preload / check:visual; here we pin the static data,
/// mirroring examples.test.ts — no wasm compiler needed.) Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { PARAMETRIC_MODELS, DEFAULT_PARAMETRIC, type ParametricModel } from "./parametric.ts";

const SURFACES = ["ml", "sexpr"] as const;

test("there is at least one parametric model and the default is the first", () => {
  assert.ok(PARAMETRIC_MODELS.length > 0, "at least one parametric model");
  assert.equal(DEFAULT_PARAMETRIC, PARAMETRIC_MODELS[0], "default is the first parametric model");
});

test("parametric slugs are unique and kebab-case", () => {
  const seen = new Set<string>();
  for (const m of PARAMETRIC_MODELS) {
    assert.match(m.slug, /^[a-z0-9]+(-[a-z0-9]+)*$/, `slug "${m.slug}" is kebab-case`);
    assert.ok(!seen.has(m.slug), `slug "${m.slug}" is unique`);
    seen.add(m.slug);
  }
});

for (const m of PARAMETRIC_MODELS as ParametricModel[]) {
  test(`parametric "${m.slug}" is well-formed + manifest agrees with the source`, () => {
    assert.ok(m.title.trim().length > 0, "has a title");
    assert.ok(m.description.trim().length > 0, "has a description");
    assert.ok(m.params.length > 0, "declares at least one @param slider");

    for (const surface of SURFACES) {
      const src = m.source[surface];
      assert.ok(typeof src === "string" && src.trim().length > 0, `${surface} source is non-empty`);
      assert.match(src, /\bmain\b/, `${surface} source defines main`);
      assert.match(src, /\blower\b/, `${surface} source returns lower(...)`);
      // A parametric model uses the ergonomic helpers → imports BOTH exact and helpers.
      assert.match(src, /exact/, `${surface} source imports exact`);
      assert.match(src, /helpers/, `${surface} source imports helpers`);
      // The single-source-of-truth invariant: every param in the manifest appears as a @param in the source
      // (so /cad's manifest-driven sliders match the model the compiler sees).
      for (const p of m.params) {
        assert.ok(
          new RegExp(`\\b${p.name}\\b`).test(src),
          `${surface} source declares the "${p.name}" @param the manifest lists`,
        );
      }
    }

    // Param bounds are well-formed Rationals: den ≠ 0, and min ≤ default ≤ max (compared as num/den).
    const asNum = ([n, d]: [number, number]) => n / d;
    for (const p of m.params) {
      for (const [n, d] of [p.min, p.max, p.default]) {
        assert.ok(Number.isInteger(n) && Number.isInteger(d), `${p.name} bound is an integer num/den pair`);
        assert.notEqual(d, 0, `${p.name} bound denominator is non-zero`);
      }
      const lo = asNum(p.min), hi = asNum(p.max), def = asNum(p.default);
      assert.ok(lo <= hi, `${p.name}: min ≤ max`);
      assert.ok(lo <= def && def <= hi, `${p.name}: min ≤ default ≤ max`);
      assert.equal(typeof p.fractional, "boolean", `${p.name}: fractional is a boolean`);
    }
  });
}
