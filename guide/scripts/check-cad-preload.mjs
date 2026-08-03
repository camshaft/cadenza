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
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

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

// SINGLE-MODE preloads the general CAD vocab (exact + helpers + units) for every model, and `injectImport`
// emits the matching import clauses. There is NO snowflake/prng lib preload anymore — the snowflake showcase
// is SELF-CONTAINED (its builder + LCG are inline in the buffer, operator directive), so it needs only the
// same exact+helpers vocab every model does. Mirrors CadPage's `runModel` arrays.
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

// PARAMETRIC ASSEMBLY showcase (v-cad's parametric L-bracket): assembly (rotate-x/fuse) + parametric (@param
// sliders) together. Must compile against preloaded helpers AND surface its 5 @params (single-mode drives
// them). Gates the combined path — the rotate helpers + the @param manifest scan over an assembly model.
const pbracket = EXAMPLES.find((e) => e.slug === "assembly-parametric-bracket");
if (!pbracket) {
  failures.push("assembly: the assembly-parametric-bracket example is missing from EXAMPLES");
} else {
  const EXPECTED = ["pa-len", "pa-wid", "pa-thick", "pa-rise", "pa-bolt"];
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(pbracket.source[surface], surface);
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
    } catch (e) {
      failures.push(`[${surface}] assembly-parametric-bracket compile THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
      continue;
    }
    const errs = (cr.diagnostics ?? []).filter((d) => d.error);
    if (!cr.component || errs.length) {
      failures.push(`[${surface}] assembly-parametric-bracket did not compile${errs.length ? ` — ${errs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : " (no component)"}`);
    } else {
      const params = (wasm.param_manifest(program, surface) ?? []).map((x) => x.name);
      const missing = EXPECTED.filter((n) => !params.includes(n));
      if (missing.length) failures.push(`[${surface}] assembly-parametric-bracket param_manifest missing @param(s): ${missing.join(", ")} (got ${params.join(", ") || "none"})`);
      else console.log(`  ✓ [${surface}] assembly-parametric-bracket: compiles (rotate-x + fuse) + surfaces its 5 @params (${params.join(", ")})`);
    }
  }
}

// PARAMETRIC SNOWFLAKE showcase (v-cad's flagship): SELF-CONTAINED — the whole builder (LCG + recursive
// branch + six-fold) is INLINE in the buffer (operator directive: build from visible primitives, no opaque
// lib). So it compiles against the SAME exact+helpers vocab every model uses — NO snowflake/prng preload.
// Must compile + surface its 3 @!param sliders (seed/arm-length/depth). Its geometry (a fold from Solid.Empty)
// is mesh-checked in the visible-geometry stage below (that's where the empty-Solid-annihilation class bites).
const snow = EXAMPLES.find((e) => e.slug === "parametric-snowflake");
if (!snow) {
  failures.push("snowflake: the parametric-snowflake example is missing from EXAMPLES");
} else {
  const EXPECTED = ["seed", "arm-length", "depth"];
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(snow.source[surface], surface);
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
    } catch (e) {
      failures.push(`[${surface}] parametric-snowflake compile THREW: ${String(e && e.message ? e.message : e).slice(0, 120)}`);
      continue;
    }
    const errs = (cr.diagnostics ?? []).filter((d) => d.error);
    if (!cr.component || errs.length) {
      failures.push(`[${surface}] parametric-snowflake did not compile (self-contained, exact+helpers only)${errs.length ? ` — ${errs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : " (no component)"}`);
    } else {
      const params = (wasm.param_manifest(program, surface) ?? []).map((x) => x.name);
      const missing = EXPECTED.filter((n) => !params.includes(n));
      if (missing.length) failures.push(`[${surface}] parametric-snowflake param_manifest missing @!param(s): ${missing.join(", ")} (got ${params.join(", ") || "none"})`);
      else console.log(`  ✓ [${surface}] parametric-snowflake: compiles self-contained (exact+helpers, inline builder) + surfaces its 3 @!params (${params.join(", ")})`);
    }
  }
}

// ── VISIBLE-GEOMETRY gate (headless, CI-runnable) ────────────────────────────────────────────────────
// The operator loaded a /cad showcase and saw a BLANK viewport while every CI gate passed — because NO CI
// gate ran the model + meshed it: check-examples uses self-contained programs (not the CAD preload); the
// checks above only COMPILE + read the manifest; and check-visual (which meshes in a real browser) is NOT
// in CI. So a model that compiles + lowers but meshes to ZERO triangles (the empty-Solid-annihilation class:
// a fold from `Solid.Empty` where `Union(Empty, x)` renders to nothing) shipped invisibly. This stage closes
// that blind spot IN CI: run each showcase through the real browser pipeline (compile_with_preloaded → run
// via jco → render_value → v-cad's `meshFromSolid`) and assert NON-ZERO geometry. Headless (jco, like
// check-worker-stack), so it runs in the `guide-examples` CI job — complementing check-visual's eyes-on
// browser assertion (local-only). A blank showcase now fails HERE, not in the operator's viewport.
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");
const { meshFromSolid } = await import(pathToFileURL(join(guideRoot, "src/cad/index.ts")).href);
const HEAP_IMPORT = "cadenza:runtime/heap";
const runtimePath = join(guideRoot, "src/wasm/runtime.wasm");
// FINDING#23: the runtime imports cadenza:nfc/normalize (separate NFC component) — supply the JS shim so it
// instantiates. NFC of well-formed UTF-8 is String.prototype.normalize('NFC') over the list<u8> boundary.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes) => new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};
const camel = (s) => s.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());

async function loadComp(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), "cadmesh-"));
  for (const [f, b] of Object.entries(files)) {
    const p = join(dir, f);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, b);
  }
  const mod = await import(pathToFileURL(join(dir, `${name}.js`)).href);
  return { instantiate: mod.instantiate, getCore: async (p) => WebAssembly.compile(readFileSync(join(dir, p))) };
}

// The value-heap runtime, instantiated once (a showcase that builds a runtime collection imports it).
let heapImport = null;
try {
  const rt = await loadComp(readFileSync(runtimePath), "heap");
  const rtRoot = await rt.instantiate(rt.getCore, { [NFC_IMPORT]: nfcHostImport });
  heapImport = rtRoot[HEAP_IMPORT] ?? rtRoot["heap"];
} catch (e) {
  failures.push(`visible-geometry: could not instantiate the value-heap runtime — ${String(e && e.message ? e.message : e).slice(0, 100)}`);
}

// Showcases to mesh-check. ALL are meshed now, INCLUDING parametric-snowflake (a large ~25k-vert mesh). Its
// resource-handle teardown trips the KNOWN jco heap-drop function[27] OOB (`memory access out of bounds` at
// heap.js drop) — a jco/browser-path tooling bug (native drop is clean, per v-memory-safety + v-runtime), NOT
// our emit/runtime. That OOB is fired ONLY by the handle's Symbol.dispose(); the mesh-stage below now disposes
// each handle inside a guarded try/catch (consuming it deterministically before GC finalization throws it
// UNGUARDED at exit), so the snowflake meshes here cleanly + this gate exits 0. So the snowflake is fully
// gated headless again — it's the sharpest guard (a fold from Solid.Empty that regresses to 0 tris if the
// empty-Solid mesh fix ever reverts). (Empty set = mesh every showcase; add a slug here only with a reason.)
const MESH_SKIP = new Set();
if (heapImport) {
  for (const ex of EXAMPLES) {
    if (MESH_SKIP.has(ex.slug)) {
      console.log(`  · [mesh] ${ex.slug}: SKIPPED (documented reason in MESH_SKIP)`);
      continue;
    }
    const program = injectImport(ex.source.sexpr, "sexpr");
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, "sexpr", names, sources, formats);
    } catch (e) {
      failures.push(`[mesh] ${ex.slug}: compile THREW ${String(e && e.message ? e.message : e).slice(0, 80)}`);
      continue;
    }
    if (!cr.component) {
      failures.push(`[mesh] ${ex.slug}: no component to run`);
      continue;
    }
    try {
      const prog = await loadComp(new Uint8Array(cr.component), "prog");
      // Supply each @param host-response from its manifest default (a parametric showcase reads Param.<name>()).
      const manifest = wasm.param_manifest(program, "sexpr") ?? [];
      const param = {};
      for (const m of manifest) {
        const d = Math.trunc(Number(m.default ?? 1));
        param[camel(m.name)] = () => BigInt(d);
        param[camel(`${m.name}-num`)] = () => BigInt(d);
        param[camel(`${m.name}-den`)] = () => 1n;
      }
      const root = await prog.instantiate(prog.getCore, { [HEAP_IMPORT]: heapImport, param });
      const iface = root["cadenza:run/run"] ?? root["run"];
      if (!iface || typeof iface.make !== "function") {
        failures.push(`[mesh] ${ex.slug}: no run interface (make/encode) on the component`);
        continue;
      }
      // make() + encode() both succeed cleanly even for a large heap value; render + mesh from the bytes.
      const handle = iface.make();
      const rendered = wasm.render_value(iface.encode(handle));
      // GUARDED DISPOSE: jco's generated [resource-drop] glue OOBs (`RuntimeError: memory access out of
      // bounds` at heap.js drop → wasm-function[27]) when tearing down a LARGE heap value's resource handle
      // — a jco/browser-path tooling bug (v-memory-safety + v-runtime verified the SAME component's native
      // drop is clean; only jco's host glue trips). make()/encode() are fine — ONLY the handle's teardown
      // throws, whether by an explicit Symbol.dispose() OR by GC FINALIZATION mid-run if the handle goes
      // unreferenced. So dispose it explicitly INSIDE a try/catch: consumes the known OOB deterministically
      // here (once), instead of an unguarded finalization throw that fails this CI job after the ✓ line.
      // (NB: the rcdzc emit fix 2d3bb98ce for the compiler-ml function[27] freeze did NOT resolve THIS jco
      // heap-drop OOB — verified it still trips without the guard. Remove once jco's resource-drop is fixed.)
      try { handle?.[Symbol.dispose]?.(); } catch { /* known jco resource-drop-glue OOB on a large heap value — consumed */ }
      const mesh = await meshFromSolid(rendered);
      if (!mesh.ok) {
        failures.push(`[mesh] ${ex.slug}: meshFromSolid errored — ${String(mesh.error).slice(0, 80)}`);
      } else if (mesh.positions.length === 0) {
        failures.push(`[mesh] ${ex.slug}: meshed to ZERO triangles — BLANK viewport (geometry annihilated; the empty-Solid-in-boolean class)`);
      } else {
        console.log(`  ✓ [mesh] ${ex.slug}: runs + meshes to visible geometry (${mesh.positions.length / 3} verts, ${mesh.indices.length / 3} tris)`);
      }
    } catch (e) {
      failures.push(`[mesh] ${ex.slug}: run/mesh THREW ${String(e && e.message ? e.message : e).slice(0, 80)}`);
    }
  }
}

if (failures.length) {
  console.error(
    "\n✗ cad-preload conformance FAILED — the /cad preloaded-library path regressed (compiler preload " +
      "linking, wasm bindings, the injected import, or a showcase that meshes BLANK):\n" +
      failures.map((f) => "  ✗ " + f).join("\n"),
  );
  process.exit(1);
}

console.log(
  "\n✓ cad-preload conformance: a bare /cad model compiles + lints clean against the preloaded exact.cdz " +
    "in both surfaces (compile_with_preloaded + diagnostics_with_preloaded), AND every showcase runs + meshes " +
    "to NON-ZERO visible geometry (headless run→mesh) — the P5 preload path + the visible-render class stay working.",
);
