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
/// SCOPE:
///   inc-1 — single-file PROGRAM cases (chapter Runnable/Exercise + playground): the case dir's
///     program.<surface> is ALREADY wrapped + rendered by guideShred, so we compile it DIRECTLY via
///     checkProgram (which does NOT re-wrap), in every emitted surface.
///   inc-2 — MULTI-FILE `(files …)` runnables: the shred emits the entry as program.<from> + each preloaded
///     peer as module-<name>.<surface> (single surface, no toggle — the files are complete modules). We
///     reconstruct the ExplorerFile set + run it through the SAME checkExample→checkMultiFile path
///     (lowerToCompile + compile_with_preloaded + run + grade) the monolithic gate uses, so zero drift.
///   Still DEFERRED (exit 2): mode="test" runnables (the shred emits meta.deferred + no program — they run
///     via the @test-export driver, a later shred kind) and notebook cells (not yet a shred kind).
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";

// Import check-examples.mjs as a LIBRARY: LIB_ONLY skips its whole-guide extraction + check loop, exposing
// only the compiler (loaded at top-level from CDZ_WASM_PKG on import) + the reusable checker functions.
process.env.CHECK_EXAMPLES_LIB_ONLY = "1";
const { checkProgram, checkExample } = await import("./check-examples.mjs");

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

// A DEFERRED case (mode="test" runnable) carries NO program — the shred defers it to the @test-export driver
// (a later shred kind), so there is nothing to compile+run here. Exit 2 (harness gap, distinct from a 0 pass
// / 1 example-failure) so the flake routes it rather than silently greening on an unchecked case.
if (meta.deferred || meta.kind === "test-mode") {
  console.error(`check-one-example: ${caseDir}: kind=${meta.kind} deferred (${meta.reason ?? "no program"}) — needs the @test-export driver (a later shred kind)`);
  process.exit(2);
}

const expectKind = (readIf("expect-kind") ?? "value").trim();
const expected = readIf("expected"); // null for an ungraded Runnable

// MULTI-FILE case (inc-2): reconstruct the ExplorerFile set from the shred artifacts (program.<from> = the
// entry, module-<name>.<surface> = each preloaded peer) and check it via the SHARED checkExample, which
// dispatches ex.files → checkMultiFile (lowerToCompile + compile_with_preloaded + run + grade) — the exact
// path the monolithic gate + the app's MultiFileRunnable use, so no drift. Single surface, no ml toggle.
if (meta.multiFile || meta.kind === "multi-file") {
  const from = (Array.isArray(meta.surfaces) && meta.surfaces[0]) || "sexpr";
  const peers = Array.isArray(meta.peers) ? meta.peers : [];
  const entrySource = readIf(`program.${from}`);
  if (entrySource == null) {
    console.error(`check-one-example: ${caseDir}: multi-file case missing program.${from}`);
    process.exit(2);
  }
  // The shred drops the entry file's authored name (keeping only entryName="main"). lowerToCompile uses the
  // entry name ONLY for its exactly-one-entry / unique-name checks, never for compilation (the entry is the
  // `text`), so synthesize a non-empty name that is unique against the peers.
  const peerNames = new Set(peers.map((p) => p.name));
  let entryName = meta.entryName || "main";
  while (peerNames.has(entryName)) entryName += "_";
  const files = [{ name: entryName, source: entrySource, surface: from, entry: true }];
  for (const p of peers) {
    const src = readIf(`module-${p.name}.${p.surface}`);
    if (src == null) {
      console.error(`check-one-example: ${caseDir}: multi-file peer module-${p.name}.${p.surface} missing`);
      process.exit(2);
    }
    files.push({ name: p.name, source: src, surface: p.surface, entry: false });
  }
  const ex = {
    file: meta.file ?? caseDir,
    files,
    expect: expectKind === "error" ? "error" : undefined,
    expected,
  };
  const fail = await checkExample(ex);
  if (fail) {
    console.error("✗ " + fail);
    process.exit(1);
  }
  console.log(`✓ ${caseDir} [multi-file] (${from}; ${peers.length} peer(s))`);
  process.exit(0);
}

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
