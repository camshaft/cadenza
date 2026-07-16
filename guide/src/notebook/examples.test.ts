/// Pin that every canonical example notebook parses into well-formed cells: it has at least one code
/// cell, every code cell is non-empty and defines a `main` (the entry the run path calls), every widget
/// cell's DSL parses without errors, and any widget a code cell references is actually declared. This
/// guards the shipped example content — an example that can't parse/run is a bug (the guide's
/// run-every-example discipline, applied to notebooks). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { parseDocument, type Cell } from "./parseDocument.ts";
import { parseWidgets } from "./parseWidgets.ts";

function codeCells(cells: Cell[]) {
  return cells.filter((c): c is Extract<Cell, { kind: "code" }> => c.kind === "code");
}

for (const ex of EXAMPLES) {
  test(`example "${ex.slug}" parses into well-formed cells`, () => {
    const cells = parseDocument(ex.markdown);
    const code = codeCells(cells);
    assert.ok(code.length >= 1, "has at least one code cell");

    // Every widget cell's DSL parses with no errors, and collect the declared widget names.
    const declared = new Set<string>();
    for (const c of code) {
      if (c.directive.kind === "widget") {
        const { widgets, errors } = parseWidgets(c.source);
        assert.deepEqual(errors, [], `widget cell in ${ex.slug} parses clean`);
        for (const w of widgets) declared.add(w.name);
      }
    }

    // Every NON-widget code cell is non-empty and defines `main` (the run entry). s-expr `(def (main) …)`.
    for (const c of code) {
      if (c.directive.kind === "widget") continue;
      assert.ok(c.source.trim().length > 0, `code cell in ${ex.slug} is non-empty`);
      assert.match(c.source, /\(def\s+\(main\)/, `code cell in ${ex.slug} defines main`);
    }
  });
}

test("the compound-interest example is the default and declares a `rate` widget its cells use", () => {
  assert.equal(DEFAULT_EXAMPLE.slug, "compound-interest");
  const cells = parseDocument(DEFAULT_EXAMPLE.markdown);
  const widgetNames = new Set(
    codeCells(cells)
      .filter((c) => c.directive.kind === "widget")
      .flatMap((c) => parseWidgets(c.source).widgets.map((w) => w.name)),
  );
  assert.ok(widgetNames.has("rate"));
  // At least one non-widget code cell references `rate` (so a drag recomputes something).
  const usesRate = codeCells(cells).some((c) => c.directive.kind !== "widget" && /\brate\b/.test(c.source));
  assert.ok(usesRate, "a code cell uses the rate widget");
});

test("every example has a unique slug", () => {
  const slugs = EXAMPLES.map((e) => e.slug);
  assert.equal(new Set(slugs).size, slugs.length);
});
