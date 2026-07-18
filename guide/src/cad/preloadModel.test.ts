/// Unit tests for /cad's preloaded-model injection (`preloadModel.ts`) — the scaffolding that wraps a bare
/// model buffer with the `import … from "exact"` + the `@!default-fraction Rational` pragma + `export main`
/// so it compiles against the preloaded CAD library. The reader's buffer is CLEAN (no import, no pragma —
/// operator UX). The load-bearing invariant is CONTIGUITY: the reader's verbatim text must appear as a
/// single substring of the injected output, or the linter's `wrapPrefixOf` span-mapping (a linear prefix
/// subtraction) misplaces every squiggle. These tests pin that + the injected import/pragma/export.

import test from "node:test";
import assert from "node:assert/strict";
import {
  injectImport,
  CAD_LIB_NAME,
  CAD_LIB_FORMAT,
  CAD_IMPORTED_NAMES,
} from "./preloadModel.ts";

// Clean model buffers — NO import, NO pragma (both are auto-injected; this is what the reader edits).
const ML_MODEL = `def main() = lower(Solid.Difference(Solid.Cube(v3r(4/1, 4/1, 4/1)), Solid.Sphere(5/2)))`;

const SEXPR_MODEL = `(def (main) (lower ((. Solid Difference) ((. Solid Cube) (v3r (/ 4 1) (/ 4 1) (/ 4 1))) ((. Solid Sphere) (/ 5 2)))))`;

test("ML: injects the import PREFIX line + the default-fraction pragma + a trailing export", () => {
  const out = injectImport(ML_MODEL, "ml");
  assert.ok(out.startsWith(`import { Solid, v3r, lower } from "exact"\n`), "import is the first line");
  assert.ok(out.includes("@!default-fraction Rational"), "the default-fraction pragma is injected");
  assert.ok(out.trimEnd().endsWith("export { main }"), "export is appended");
});

test("s-expr: wraps the inner forms in (do (import …) (pragma …) … (export main))", () => {
  const out = injectImport(SEXPR_MODEL, "sexpr");
  assert.ok(out.startsWith(`(do\n(import "exact" (Solid v3r lower))\n`), "opens with (do (import …)");
  assert.ok(out.includes("(pragma default-fraction Rational)"), "the default-fraction pragma is injected");
  assert.ok(out.trimEnd().endsWith("(export main))"), "closes with (export main))");
  // The s-expr import spec is a bare name LIST — no commas (commas are an ML-surface artifact).
  assert.ok(!/\(import "exact" \([^)]*,/.test(out), "no commas in the s-expr import spec");
});

// CONTIGUITY — the invariant the linter's span-mapping depends on. The reader's trimmed buffer must be a
// single substring of the injected output (verbatim, unsplit), for BOTH surfaces.
test("ML: the reader's verbatim text is contiguous in the injected output", () => {
  const out = injectImport(ML_MODEL, "ml");
  assert.ok(out.includes(ML_MODEL.trim()), "editor text embedded contiguously (wrapPrefixOf can locate it)");
});

test("s-expr: the reader's verbatim text is contiguous in the injected output", () => {
  const out = injectImport(SEXPR_MODEL, "sexpr");
  assert.ok(out.includes(SEXPR_MODEL.trim()), "editor text embedded contiguously (wrapPrefixOf can locate it)");
});

test("trims surrounding whitespace before embedding (stable prefix length)", () => {
  const out = injectImport(`\n\n  ${ML_MODEL}  \n`, "ml");
  assert.ok(out.includes(ML_MODEL.trim()), "leading/trailing whitespace trimmed");
  assert.ok(!out.includes("\n\n\n"), "no stray blank runs from the raw padding");
});

test("the preloaded-library constants match what CadPage passes the compiler", () => {
  assert.equal(CAD_LIB_NAME, "exact");
  assert.equal(CAD_LIB_FORMAT, "ml"); // exact.cdz is authored in ML (.cdz)
  assert.deepEqual([...CAD_IMPORTED_NAMES], ["Solid", "v3r", "lower"]);
});
