/// Pin the shipped CAD example models (the /cad example-switcher content): every example has BOTH surfaces
/// (ml + sexpr), each source is non-empty, defines `main`, and returns `lower(...)` — the preloaded-library
/// contract (CadPage auto-injects the `import … from "exact"` and the model returns `lower(<Solid model>)`
/// so the generic `Solid` becomes a host-renderable monomorphic `SolidR`). Slugs are unique + kebab-case,
/// and the default is the first entry. This guards the shipped example content — a model that lost a
/// surface, dropped its `main`/`lower`, or duplicated a slug is an authoring bug the picker would surface.
/// (The actual compile+mesh of each model is verified out-of-band against the real compiler; here we pin the
/// static structure, mirroring notebook/examples.test.ts — no wasm compiler needed.) Run with `npm run
/// test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES, DEFAULT_EXAMPLE, type ExampleModel } from "./examples.ts";

const SURFACES = ["ml", "sexpr"] as const;

test("there is at least one example and the default is the first", () => {
  assert.ok(EXAMPLES.length > 0, "at least one CAD example");
  assert.equal(DEFAULT_EXAMPLE, EXAMPLES[0], "default is the first example");
});

test("slugs are unique and kebab-case", () => {
  const seen = new Set<string>();
  for (const ex of EXAMPLES) {
    assert.match(ex.slug, /^[a-z0-9]+(-[a-z0-9]+)*$/, `slug "${ex.slug}" is kebab-case`);
    assert.ok(!seen.has(ex.slug), `slug "${ex.slug}" is unique`);
    seen.add(ex.slug);
  }
});

for (const ex of EXAMPLES as ExampleModel[]) {
  test(`example "${ex.slug}" is well-formed in both surfaces`, () => {
    assert.ok(ex.title.trim().length > 0, "has a title");
    assert.ok(ex.description.trim().length > 0, "has a description");
    for (const surface of SURFACES) {
      const src = ex.source[surface];
      assert.ok(typeof src === "string" && src.trim().length > 0, `${surface} source is non-empty`);
      // Every model is an entry program: it defines `main`.
      assert.match(src, /\bmain\b/, `${surface} source defines main`);
      // The preloaded-library contract: the model returns `lower(...)` (generic Solid → monomorphic SolidR
      // the host can render) — without it a generic `Solid(Rational)` result declines host-render.
      assert.match(src, /\blower\b/, `${surface} source returns lower(...)`);
      // The example buffer is a CLEAN model — it must NOT carry the `@!default-fraction Rational` pragma:
      // CadPage's injectImport prepends the pragma (+ the exact import) so the reader never writes it (the
      // operator-directed implicit-default-fraction UX). An example that re-declared the pragma would be
      // redundant boilerplate (and re-introduce the red-squiggle the injection removed).
      assert.doesNotMatch(src, /default-fraction/, `${surface} source is pragma-free (injected by CadPage)`);
    }
  });
}
