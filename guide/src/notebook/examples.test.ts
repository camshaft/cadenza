/// Pin that every canonical example notebook parses into well-formed cells: it has at least one code
/// cell, every code cell is non-empty and defines a `main` (the entry the run path calls), every widget
/// cell's DSL parses without errors, and every DECLARED widget is referenced by at least one non-widget
/// code cell (no dead control — a widget that recomputes nothing is an authoring bug; this is the
/// reactive-splice contract, generalized to all examples). This guards the shipped example content — an
/// example that can't parse/run, or ships an inert widget, is a bug (the guide's run-every-example
/// discipline, applied to notebooks). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { parseDocument, type Cell } from "./parseDocument.ts";
import { parseWidgets, type Widget } from "./parseWidgets.ts";
import { assembleForRun } from "./assembleForRun.ts";

function codeCells(cells: Cell[]) {
  return cells.filter((c): c is Extract<Cell, { kind: "code" }> => c.kind === "code");
}

/// Whole-word, kebab-aware reference test (mirrors recomputePlan.references): a widget name must appear
/// as a standalone token, so `rate` doesn't match inside `rate-adjusted`.
function references(source: string, name: string): boolean {
  const escaped = name.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&");
  return new RegExp(`(^|[^A-Za-z0-9_.\\-])${escaped}([^A-Za-z0-9_.\\-]|$)`).test(source);
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
    const nonWidget = code.filter((c) => c.directive.kind !== "widget");
    for (const c of nonWidget) {
      assert.ok(c.source.trim().length > 0, `code cell in ${ex.slug} is non-empty`);
      assert.match(c.source, /\(def\s+\(main\)/, `code cell in ${ex.slug} defines main`);
    }

    // Every declared widget must be referenced by some non-widget code cell — else it's a dead control
    // that recomputes nothing (the reactive-splice contract the module docstring promises).
    for (const name of declared) {
      assert.ok(
        nonWidget.some((c) => references(c.source, name)),
        `widget \`${name}\` in ${ex.slug} is referenced by a code cell (not a dead control)`,
      );
    }
  });
}

// Operator directive (ratified): the notebook uses NO Float64 anywhere — it's rational-by-default (bare
// integer literals ground to exact Rational via the `default-fraction Rational` pragma the run/lint paths
// prepend), so exact arithmetic holds (the formula example's `num / den` = 3/4, not integer division's 0).
// This pins the END STATE the operator asked for: no example may reintroduce a Float via a `Float64`/`Float32`
// type annotation or a float LITERAL (`1.0`, `0.5`) in a code or widget cell — a future edit that sneaks one
// back in (regressing to inexact float math) fails here, fast, with no wasm needed. Scans the raw cell source
// (the check-examples result-type gate only pinned `Rational.of` cells, which the no-floats rework removed —
// this is the missing static guard on the operator's actual goal). Prose is exempt (it may DISCUSS floats).
const FLOAT_TYPE_RE = /\bFloat(?:64|32)?\b/; // a Float type annotation (Float / Float64 / Float32)
const FLOAT_LITERAL_RE = /(?:^|[^.\w])\d+\.\d+/; // a decimal literal like 1.0 or 0.5 (not a member access `a.0`)
for (const ex of EXAMPLES) {
  test(`example "${ex.slug}" is Float-free — no Float64 annotation or float literal (operator: rational-by-default)`, () => {
    const cells = parseDocument(ex.markdown);
    for (const c of codeCells(cells)) {
      assert.ok(
        !FLOAT_TYPE_RE.test(c.source),
        `${ex.slug}: a cell uses a Float type — the notebook is rational-by-default (no floats). Cell: ${c.source.slice(0, 80)}`,
      );
      assert.ok(
        !FLOAT_LITERAL_RE.test(c.source),
        `${ex.slug}: a cell has a float literal — bare integers ground to Rational; drop the decimal. Cell: ${c.source.slice(0, 80)}`,
      );
    }
  });
}

test("the compound-interest example is the default and declares a `rate` widget", () => {
  assert.equal(DEFAULT_EXAMPLE.slug, "compound-interest");
  const cells = parseDocument(DEFAULT_EXAMPLE.markdown);
  const widgetNames = new Set(
    codeCells(cells)
      .filter((c) => c.directive.kind === "widget")
      .flatMap((c) => parseWidgets(c.source).widgets.map((w) => w.name)),
  );
  assert.ok(widgetNames.has("rate"));
  // (That `rate` is actually referenced by a code cell — the reactive-splice contract — is now covered
  // by the per-example "declared widget is referenced" assertion in the loop above.)
});

test("every example has a unique slug", () => {
  const slugs = EXAMPLES.map((e) => e.slug);
  assert.equal(new Set(slugs).size, slugs.length);
});

// Every non-widget code cell of every example must ASSEMBLE to a single-`main` module — the end-to-end
// guard on the P0 #12 cell-collision ("notebook busted": >1 `main` in one namespace → CDZ0201). Each cell
// defines its own `main`, and a multi-cell example carries prior cells' defs in its buffer; the per-cell
// `main`-strip (assembleCell.stripMainDef) must leave EXACTLY one `main` so the cell compiles. This pins
// that the SHIPPED examples stay CDZ0201-safe (a new example, or a regression in the strip, fails here).
for (const ex of EXAMPLES) {
  test(`example "${ex.slug}" — every code cell assembles to exactly one \`main\` (CDZ0201-safe, P0 #12)`, () => {
    const cells = parseDocument(ex.markdown);
    const widgets: Widget[] = codeCells(cells)
      .filter((c) => c.directive.kind === "widget")
      .flatMap((c) => parseWidgets(c.source).widgets);
    cells.forEach((c, i) => {
      if (c.kind !== "code" || c.directive.kind === "widget") return;
      const { buffer, entry } = assembleForRun(cells, i, widgets, {}, "sexpr");
      const mainDefs = (buffer.match(/\(def\s+\(main\)/g) ?? []).length;
      assert.equal(mainDefs, 1, `cell ${i} in ${ex.slug} must assemble with exactly one \`main\` (got ${mainDefs})`);
      assert.equal(entry, "(main)", `cell ${i} in ${ex.slug} runs via a (main) call`);
    });
  });
}
