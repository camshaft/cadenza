#!/usr/bin/env node
/// /cad PRELOAD conformance: verify the /cad route's preloaded-library compile path works end-to-end,
/// the way the browser does — WITHOUT a browser (node + the staged wasm, like check-worker-stack).
///
/// WHY a separate check: /cad compiles the reader's BARE model buffer against the CAD library (exact.cdz)
/// PRELOADED via `compile_with_preloaded` (operator P5, ruling A) — the buffer holds only the model; the
/// vocab (Solid/v3r/lower) is link-merged. check-examples can't cover this (its examples are self-contained,
/// not preloaded), and check-visual (which renders /cad in a browser) is NOT in CI. So the whole preload
/// path — the compiler's link-merge, the auto-injected import, `lower(...)` → renderable SolidR, and the
/// preload-aware LINTER (diagnostics_with_preloaded resolving the preloaded vocab) — was un-gated in CI.
/// A peer editing the compiler's preload linking, the wasm bindings, or the guide's injection could silently
/// break /cad and only a reader would notice. This check is that regression guard.
///
/// HOW: load the staged wasm + the staged CAD lib (guide/src/wasm/cad/exact.cdz), then for each surface:
///   (1) compile_with_preloaded(injectImport(model), …) → assert a component is emitted, 0 error diags;
///   (2) diagnostics_with_preloaded(injectImport(model), …) → assert NO errors (the preloaded vocab
///       resolves — the exact bug the preload-aware linter fixes: Solid/v3r/lower were unbound before).
/// It reuses the REAL `injectImport` from src/cad/preloadModel.ts so the gate exercises exactly what /cad
/// ships (a private copy would drift — the assert-prelude kebab bug taught that lesson).
///
/// Run: `npm run check:cad-preload` (needs the staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.

import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// The staged compiler wasm + the staged CAD library source (exact.cdz), both produced by stage-wasm.mjs.
const pkgDir = join(guideRoot, "src/wasm/pkg");
const wasm = await import(pathToFileURL(join(pkgDir, "cdz_wasm.js")).href);
await wasm.default(await readFile(join(pkgDir, "cdz_wasm_bg.wasm")));

let exactSrc;
try {
  exactSrc = await readFile(join(guideRoot, "src/wasm/cad/exact.cdz"), "utf8");
} catch {
  console.error(
    "\n✗ cad-preload conformance FAILED — the staged CAD lib src/wasm/cad/exact.cdz is missing " +
      "(run `cargo xtask guide-wasm` to stage it). /cad cannot preload without it.",
  );
  process.exit(1);
}

// Reuse the REAL injection + preload arrays /cad uses (no private copy — the gate must match what ships).
const { injectImport, CAD_LIB_NAME, CAD_LIB_FORMAT } = await import(
  pathToFileURL(join(guideRoot, "src/cad/preloadModel.ts")).href
);

// The starter models per surface — a bare model returning `lower(<Solid>)`, carrying the pragma (else a
// bare n/d is Int64 and the model rejects). Mirrors CadPage's STARTER shape; kept here so a starter change
// that breaks the preload path is caught even if CadPage's own copy is edited.
const MODELS = {
  ml: `@!default-fraction Rational
def main() = lower(Solid.Difference(Solid.Cube(v3r(4/1, 4/1, 4/1)), Solid.Sphere(5/2)))`,
  sexpr: `(pragma default-fraction Rational)
(def (main) (lower ((. Solid Difference) ((. Solid Cube) (v3r (/ 4 1) (/ 4 1) (/ 4 1))) ((. Solid Sphere) (/ 5 2)))))`,
};

const names = [CAD_LIB_NAME];
const sources = [exactSrc];
const formats = [CAD_LIB_FORMAT];

const failures = [];

for (const surface of ["ml", "sexpr"]) {
  const program = injectImport(MODELS[surface], surface);

  // (1) compile_with_preloaded → a component must be emitted with no error diagnostics.
  let cr;
  try {
    cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
  } catch (e) {
    failures.push(`[${surface}] compile_with_preloaded THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
    continue;
  }
  const compileErrs = (cr.diagnostics ?? []).filter((d) => d.error);
  if (!cr.component) {
    failures.push(
      `[${surface}] compile_with_preloaded emitted NO component` +
        (compileErrs.length ? ` — ${compileErrs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : ""),
    );
  } else if (compileErrs.length) {
    failures.push(
      `[${surface}] compile_with_preloaded reported ${compileErrs.length} error(s): ` +
        compileErrs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; "),
    );
  } else {
    console.log(`  ✓ [${surface}] compile_with_preloaded: bare model links against preloaded exact.cdz → component (${cr.component.length}b)`);
  }

  // (2) diagnostics_with_preloaded → the preload-aware LINTER must report NO errors (the preloaded vocab
  // Solid/v3r/lower resolves; before the fix these were CDZ0101 unbound + import-not-modeled = 6 squiggles).
  let diags;
  try {
    diags = wasm.diagnostics_with_preloaded(program, surface, names, sources, formats);
  } catch (e) {
    failures.push(`[${surface}] diagnostics_with_preloaded THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
    continue;
  }
  const lintErrs = (diags ?? []).filter((d) => d.error);
  if (lintErrs.length) {
    failures.push(
      `[${surface}] diagnostics_with_preloaded reported ${lintErrs.length} error(s) (the preloaded vocab should resolve): ` +
        lintErrs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; "),
    );
  } else {
    console.log(`  ✓ [${surface}] diagnostics_with_preloaded: preloaded vocab (Solid/v3r/lower) resolves — 0 lint errors`);
  }
}

if (failures.length) {
  console.error(
    "\n✗ cad-preload conformance FAILED — the /cad preloaded-library path regressed (compiler preload " +
      "linking, wasm bindings, or the injected import):\n" +
      failures.map((f) => "  ✗ " + f).join("\n"),
  );
  process.exit(1);
}

console.log(
  "\n✓ cad-preload conformance: a bare /cad model compiles + lints clean against the preloaded exact.cdz " +
    "in both surfaces (compile_with_preloaded + diagnostics_with_preloaded) — the P5 preload path stays working.",
);
