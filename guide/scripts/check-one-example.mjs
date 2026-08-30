/// PER-EXAMPLE shred entry point (operator directive 2026-08-30: SHRED check:examples — each guide example
/// becomes its own nix derivation so it runs on a FRESH cdz-wasm compiler instance; the monolithic check
/// reused ONE instance across 409 calls and leaked its linear memory until an OOB, and masked the 13
/// per-example wasm-path bugs behind one red). Given ONE guideShred case dir, this loads the compiler fresh
/// (one case per process = the leak fix, by construction) and checks THAT one example via the SHARED
/// checkProgram from check-examples.mjs (no drift — same compile+run+grade the monolithic gate uses).
///
/// CONTRACT (locked with v-nix's flake matrix): `node --expose-gc check-one-example.mjs <CASE_DIR>`.
///   env CDZ_WASM_PKG   = the staged cdz-wasm pkg dir (cdz_wasm.js + cdz_wasm_bg.wasm); the flake stages it.
///   env CADENZA_STORE  = the value-heap runtime store (holds the runtime by content-hash; the flake injects it).
/// EXIT 0 = every present surface passed; non-0 = a failure (diagnostic to stderr). One case, one verdict —
/// the flake runs one ca-derivation per case (guideWasmShredAgg) so freshness comes from the process boundary.
///
/// SCOPE (inc-1): single-file PROGRAM cases (chapter Runnable/Exercise + playground) — the case dir's
/// program.<surface> is ALREADY wrapped + rendered by guideShred, so we compile it DIRECTLY via checkProgram
/// (which does NOT re-wrap). Multi-file (compile_with_preloaded peers), mode="test" (compile_tests), notebook
/// cells, and the attr-above render invariant are SEPARATE case kinds handled in inc-2 (with manifest coverage).
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";

// Import check-examples.mjs as a LIBRARY: LIB_ONLY skips its whole-guide extraction + check loop, exposing
// only the compiler (loaded at top-level from CDZ_WASM_PKG on import) + the reusable checker functions.
process.env.CHECK_EXAMPLES_LIB_ONLY = "1";
const { checkProgram } = await import("./check-examples.mjs");

const caseDir = process.argv[2];
if (!caseDir) {
  console.error("usage: node --expose-gc check-one-example.mjs <guideShred-case-dir>");
  process.exit(2);
}

const readIf = (name) => (existsSync(join(caseDir, name)) ? readFileSync(join(caseDir, name), "utf8") : null);
let meta;
try {
  meta = JSON.parse(readFileSync(join(caseDir, "meta.json"), "utf8"));
} catch (e) {
  console.error(`check-one-example: ${caseDir}: unreadable meta.json — ${String(e.message || e)}`);
  process.exit(2);
}

// inc-1 handles single-file program cases only. A multi-file case (peers) or a deferred/test case is NOT
// covered here yet — exit 2 (harness gap, distinct from a 0 pass / 1 example-failure) so the flake/inc-2 can
// route it, rather than a silent green.
if (meta.multiFile || meta.deferred || meta.kind === "multi-file" || meta.kind === "test-mode") {
  console.error(`check-one-example: ${caseDir}: kind=${meta.kind} (multiFile/deferred) not handled in inc-1 — inc-2 adds compile_with_preloaded/compile_tests + notebook/attr-above kinds`);
  process.exit(2);
}

const expectKind = (readIf("expect-kind") ?? "value").trim();
const expected = readIf("expected"); // null for an ungraded Runnable

// Check every present surface (guideShred pre-renders both). checkProgram grades the `expected` scalar only
// on the s-expr pass (the ML pass guards the wrap/render round-trip); mirrors the monolithic gate exactly.
const surfaces = Array.isArray(meta.surfaces) && meta.surfaces.length ? meta.surfaces : ["sexpr", "ml"];
for (const surface of surfaces) {
  const program = readIf(`program.${surface}`);
  if (program == null) continue; // surface not emitted for this case
  const ex = {
    snippet: program, // used only for the failure `brief`; the program is passed explicitly
    file: meta.file ?? caseDir,
    kind: meta.kind ?? "Runnable",
    expect: expectKind === "error" ? "error" : undefined,
    expected,
  };
  const fail = await checkProgram(program, surface, ex, surface === "ml" ? "ML" : "s-expr");
  if (fail) {
    console.error("✗ " + fail);
    process.exit(1);
  }
}

console.log(`✓ ${caseDir} [${meta.kind ?? "?"}] (${surfaces.join("+")})`);
process.exit(0);
