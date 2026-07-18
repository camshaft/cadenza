#!/usr/bin/env node
/// /cad PRELOAD conformance: verify the /cad route's preloaded-library compile path works end-to-end,
/// the way the browser does — WITHOUT a browser (node + the staged wasm, like check-worker-stack).
///
/// WHY a separate check: /cad compiles the reader's BARE model buffer against the CAD library (exact.cdz)
/// PRELOADED via `compile_with_preloaded` (operator P5, ruling A) — the buffer holds only the model; the
/// vocab (Solid/v3/lower) is link-merged. check-examples can't cover this (its examples are self-contained,
/// not preloaded), and check-visual (which renders /cad in a browser) is NOT in CI. So the whole preload
/// path — the compiler's link-merge, the auto-injected import, `lower(...)` → renderable SolidR, and the
/// preload-aware LINTER (diagnostics_with_preloaded resolving the preloaded vocab) — was un-gated in CI.
/// A peer editing the compiler's preload linking, the wasm bindings, or the guide's injection could silently
/// break /cad and only a reader would notice. This check is that regression guard.
///
/// HOW: load the staged wasm + the staged CAD lib (guide/src/wasm/cad/exact.cdz), then for each surface:
///   (1) compile_with_preloaded(injectImport(model), …) → assert a component is emitted, 0 error diags;
///   (2) diagnostics_with_preloaded(injectImport(model), …) → assert NO errors (the preloaded vocab
///       resolves — the exact bug the preload-aware linter fixes: Solid/v3/lower were unbound before).
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

let exactSrc, helpersSrc, unitsSrc;
try {
  exactSrc = await readFile(join(guideRoot, "src/wasm/cad/exact.cdz"), "utf8");
  helpersSrc = await readFile(join(guideRoot, "src/wasm/cad/helpers.cdz"), "utf8");
  unitsSrc = await readFile(join(guideRoot, "src/wasm/cad/units.cdz"), "utf8");
} catch {
  console.error(
    "\n✗ cad-preload conformance FAILED — a staged CAD lib (src/wasm/cad/{exact,helpers,units}.cdz) is missing " +
      "(run `cargo xtask guide-wasm` to stage it). /cad cannot preload without them.",
  );
  process.exit(1);
}

// Reuse the REAL injection + preload arrays /cad uses (no private copy — the gate must match what ships).
const { injectImport, CAD_LIB_NAME, CAD_HELPERS_NAME, CAD_UNITS_NAME, CAD_LIB_FORMAT } = await import(
  pathToFileURL(join(guideRoot, "src/cad/preloadModel.ts")).href
);

// The starter models per surface — a CLEAN bare model returning `lower(<Solid>)`: NO pragma, NO import
// (both are auto-injected by `injectImport`). This mirrors what a /cad reader actually edits now, and
// specifically exercises that the INJECTED `@!default-fraction Rational` makes a pragma-less model compile
// (a bare `n/d` grounds to Rational — without the injected pragma, `v3(4/1,…)` would reject CDZ0203).
const MODELS = {
  ml: `def main() = lower(Solid.Difference(Solid.Cube(v3(4/1, 4/1, 4/1)), Solid.Sphere(5/2)))`,
  sexpr: `(def (main) (lower ((. Solid Difference) ((. Solid Cube) (v3 (/ 4 1) (/ 4 1) (/ 4 1))) ((. Solid Sphere) (/ 5 2)))))`,
};

// SINGLE-MODE preloads ALL THREE modules (exact base vocab + helpers ergonomic wrappers + units ctors) for
// every model, and `injectImport` now emits all three import clauses — so the gate must preload all three or
// the injected `import "units"` faults CDZ0201 (unknown package file). Mirrors CadPage's `runModel` arrays.
const names = [CAD_LIB_NAME, CAD_HELPERS_NAME, CAD_UNITS_NAME];
const sources = [exactSrc, helpersSrc, unitsSrc];
const formats = [CAD_LIB_FORMAT, CAD_LIB_FORMAT, CAD_LIB_FORMAT];

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
  // Solid/v3/lower resolves; before the fix these were CDZ0101 unbound + import-not-modeled = 6 squiggles).
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
    console.log(`  ✓ [${surface}] diagnostics_with_preloaded: preloaded vocab (Solid/v3/lower) resolves — 0 lint errors`);
  }
}

// SINGLE-MODE (operator #6820): a model DECLARES its own `@param`s and /cad auto-surfaces a slider per param
// from the compiled manifest. Gate the enabler: the real parametric example's `@param`s must be read by
// `param_manifest` (over the injected buffer) — this is the binding + injection the single-mode UI depends on.
// Uses the actual PARAMETRIC_PLATE example so a drift between the model source and the manifest scan trips here.
const { EXAMPLES } = await import(pathToFileURL(join(guideRoot, "src/cad/examples.ts")).href);
const plate = EXAMPLES.find((e) => e.slug === "parametric-plate");
if (!plate) {
  failures.push("single-mode: the parametric-plate example is missing from EXAMPLES (the manifest gate can't run)");
} else {
  const EXPECTED_PARAMS = ["width", "depth", "thickness", "bore"];
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(plate.source[surface], surface);
    let entries;
    try {
      entries = wasm.param_manifest(program, surface);
    } catch (e) {
      failures.push(`[${surface}] param_manifest THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
      continue;
    }
    const names = (entries ?? []).map((x) => x.name);
    const missing = EXPECTED_PARAMS.filter((n) => !names.includes(n));
    if (missing.length) {
      failures.push(`[${surface}] param_manifest missing @param(s): ${missing.join(", ")} (got ${names.join(", ") || "none"})`);
    } else {
      // Each entry must carry a type_name (the B-invariant) so the slider knows integer vs fractional steps.
      const noType = (entries ?? []).filter((x) => !x.type_name);
      if (noType.length) {
        failures.push(`[${surface}] param_manifest entr(y/ies) missing type_name: ${noType.map((x) => x.name).join(", ")}`);
      } else {
        console.log(`  ✓ [${surface}] param_manifest: the plate's @params (${names.join(", ")}) surface with types — single-mode sliders auto-populate`);
      }
    }
  }
}

// UNITS-PARAMETRIC showcase (v-cad's inch bracket): a model that imports `inch` from the `units` module must
// COMPILE against the preloaded units lib (the point of staging units.cdz + injecting the units import) and
// surface its 4 inch-valued @params. Gates the whole units path: units.cdz staged + preloaded + `inch`
// injected. A missing units preload would fault CDZ0201 `import "units"`; a missing `inch` import → CDZ0101.
const bracket = EXAMPLES.find((e) => e.slug === "units-bracket");
if (!bracket) {
  failures.push("units: the units-bracket example is missing from EXAMPLES (the units gate can't run)");
} else {
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(bracket.source[surface], surface);
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
    } catch (e) {
      failures.push(`[${surface}] units-bracket compile THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
      continue;
    }
    const errs = (cr.diagnostics ?? []).filter((d) => d.error);
    if (!cr.component || errs.length) {
      failures.push(`[${surface}] units-bracket did not compile against preloaded units.cdz${errs.length ? ` — ${errs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : " (no component)"}`);
    } else {
      const params = (wasm.param_manifest(program, surface) ?? []).map((x) => x.name);
      const missing = ["bwidth", "bdepth", "bthickness", "bbore"].filter((n) => !params.includes(n));
      if (missing.length) failures.push(`[${surface}] units-bracket param_manifest missing @param(s): ${missing.join(", ")}`);
      else console.log(`  ✓ [${surface}] units-bracket: compiles against preloaded units.cdz (inch) + surfaces its inch @params (${params.join(", ")})`);
    }
  }
}

// ASSEMBLY-as-code showcase (v-cad's L-bracket): a model using the ROTATION helper (`rotate-x`) + `fuse`/
// `cut`/`move-*` must COMPILE against the preloaded helpers — gating that the rotate/mirror transforms were
// added to CAD_HELPER_NAMES (else the injected `import "helpers"` omits `rotate-x` → CDZ0101 unbound).
const asm = EXAMPLES.find((e) => e.slug === "assembly-l-bracket");
if (!asm) {
  failures.push("assembly: the assembly-l-bracket example is missing from EXAMPLES (the rotate-helper gate can't run)");
} else {
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(asm.source[surface], surface);
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
    } catch (e) {
      failures.push(`[${surface}] assembly-l-bracket compile THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
      continue;
    }
    const errs = (cr.diagnostics ?? []).filter((d) => d.error);
    if (!cr.component || errs.length) {
      failures.push(`[${surface}] assembly-l-bracket did not compile (rotate-x/fuse against preloaded helpers)${errs.length ? ` — ${errs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : " (no component)"}`);
    } else {
      console.log(`  ✓ [${surface}] assembly-l-bracket: compiles against preloaded helpers (rotate-x + fuse + cut) → component`);
    }
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
