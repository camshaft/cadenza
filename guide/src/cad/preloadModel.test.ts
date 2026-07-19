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
  CAD_HELPERS_NAME,
  CAD_UNITS_NAME,
  CAD_LIB_FORMAT,
  CAD_IMPORTED_NAMES,
  CAD_HELPER_NAMES,
  CAD_UNIT_NAMES,
} from "./preloadModel.ts";

// Clean model buffers — NO import, NO pragma (both are auto-injected; this is what the reader edits).
const ML_MODEL = `def main() = lower(Solid.Difference(Solid.Cube(v3(4/1, 4/1, 4/1)), Solid.Sphere(5/2)))`;

const SEXPR_MODEL = `(def (main) (lower ((. Solid Difference) ((. Solid Cube) (v3 (/ 4 1) (/ 4 1) (/ 4 1))) ((. Solid Sphere) (/ 5 2)))))`;

test("ML: injects the exact + helpers + units import lines + the default-fraction pragma + a trailing export", () => {
  const out = injectImport(ML_MODEL, "ml");
  assert.ok(/^import \{ [^}]*\bSolid\b[^}]*\blower\b[^}]* \} from "exact"\n/.test(out), "exact import (of the CAD superset) is the first line");
  assert.ok(/\nimport \{ [^}]*\bbox\b[^}]*\bhole-through\b[^}]* \} from "helpers"\n/.test(out), "helpers import (the ergonomic superset) follows");
  assert.ok(/\nimport \{ [^}]*\binch\b[^}]* \} from "units"\n/.test(out), "units import (the unit ctors) follows");
  // NO snowflake import — the snowflake showcase is self-contained (its builder is inline in the buffer).
  assert.ok(!/from "snowflake"/.test(out), "no snowflake lib import (the showcase is self-contained)");
  assert.ok(out.includes("@!default-fraction Rational"), "the default-fraction pragma is injected");
  assert.ok(out.trimEnd().endsWith("export { main }"), "export is appended");
});

test("s-expr: wraps the inner forms in (do (import exact) (import helpers) (import units) (pragma …) … (export main))", () => {
  const out = injectImport(SEXPR_MODEL, "sexpr");
  assert.ok(/^\(do\n\(import "exact" \([^)]*\bSolid\b[^)]*\blower\b[^)]*\)\)\n/.test(out), "opens with (do (import exact …))");
  assert.ok(/\(import "helpers" \([^)]*\bbox\b[^)]*\bhole-through\b[^)]*\)\)/.test(out), "includes (import helpers …)");
  assert.ok(/\(import "units" \([^)]*\binch\b[^)]*\)\)/.test(out), "includes (import units …)");
  assert.ok(!/\(import "snowflake"/.test(out), "no snowflake lib import (the showcase is self-contained)");
  assert.ok(out.includes("(pragma default-fraction Rational)"), "the default-fraction pragma is injected");
  assert.ok(out.trimEnd().endsWith("(export main))"), "closes with (export main))");
  // The s-expr import spec is a bare name LIST — no commas (commas are an ML-surface artifact).
  assert.ok(!/\(import "(exact|helpers|units)" \([^)]*,/.test(out), "no commas in the s-expr import specs");
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
  assert.equal(CAD_HELPERS_NAME, "helpers");
  assert.equal(CAD_UNITS_NAME, "units");
  assert.equal(CAD_LIB_FORMAT, "ml"); // exact/helpers/units.cdz are authored in ML (.cdz)
  assert.deepEqual([...CAD_IMPORTED_NAMES], ["Solid", "v3", "lower", "Profile", "path-start", "line-to", "cubic-to", "v2"]);
  assert.ok(CAD_HELPER_NAMES.includes("box") && CAD_HELPER_NAMES.includes("hole-through"), "helper superset includes the ergonomic wrappers");
  // The assembly transforms (rotate/mirror) must be in the superset so an assembly-as-code model's
  // `rotate-x`/`mirror-x` resolve against the preloaded helpers (else CDZ0101 unbound). The snowflake
  // showcase also builds from these (box/ball/fuse/move-x/rotate-z/mirror-x) — self-contained, no snowflake lib.
  assert.ok(CAD_HELPER_NAMES.includes("rotate-x") && CAD_HELPER_NAMES.includes("mirror-x"), "helper superset includes the rotation + mirror transforms");
  assert.ok(CAD_HELPER_NAMES.includes("ball") && CAD_HELPER_NAMES.includes("rotate-z"), "helper superset includes the snowflake primitives (ball + rotate-z)");
  assert.ok(CAD_UNIT_NAMES.includes("inch"), "unit superset includes the inch constructor");
});
