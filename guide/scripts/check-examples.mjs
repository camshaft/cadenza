#!/usr/bin/env node
/// Verify every runnable example in the guide actually compiles — and that every graded exercise's
/// solution runs to its stated `expected` value. This enforces the guide's "only show what runs"
/// discipline in CI, so a chapter can never drift ahead of (or behind) the compiler.
///
/// What it checks, over every `<Runnable>` / `<Exercise>` in `src/content/chapters/*.tsx` (+ Welcome /
/// HomePage examples):
///   - a `source=` (Runnable) or `solution=` (Exercise) snippet is WRAPPED exactly as the app wraps it
///     (`wrapModule`, imported from the guide source so it can never drift), compiled via the real
///     browser compiler (`cdz-wasm`), and:
///       · `expect="error"` examples MUST decline (no component) or trap;
///       · every other example MUST produce a component (compiles clean) AND RUN to a value without
///         throwing/trapping/stack-overflowing — compiling is NOT enough (the operator hit an intro
///         example that compiled yet crashed in the browser; "every example is a test" means it must
///         actually run). Running once on the s-expr surface is enough; the ML pass guards the
///         wrap/strip round-trip.
///   - a graded exercise (has `expected="…"`) additionally asserts the rendered scalar equals `expected`.
/// `starter=` snippets are NOT checked — they contain the `?` hole and are meant not to compile.
///
/// Run: `npm run check:examples` (needs the staged wasm pkg — `npm run wasm` first, or `cargo xtask
/// guide-wasm`). Node ≥ 20.19 for jco.

import { readFileSync, readdirSync, mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// Example EXTRACTION (cookTemplate / extractFilesProp / extractExamples) is shared with
// scripts/shred-examples.mjs via ./example-extract.mjs, so the inline gate and the per-example
// nix-cached shred can NEVER drift in how they parse `<Runnable>`/`<Exercise>` out of a chapter.
import { cookTemplate, extractFilesProp, extractExamples, blockedBy } from "./example-extract.mjs";

// ---- the blocklist: examples that DON'T run yet, classified + routed (operator policy 2026-07-15) ----
// An entry marks a KNOWN failure the guide can't fix on its own (a filed compiler bug, or a content bug
// owned by v-guide). Such an example is reported "known-blocked" (loud + tracked) rather than
// hard-failing the gate — otherwise the gate stays red on something the guide can't fix, and no example
// ships broken. RE-RUN LOOP: each run re-checks every blocked example; when one starts PASSING the
// harness says so, so the entry is removed and the example ships. See example-blocklist.json for shape.
const blocklist = JSON.parse(readFileSync(join(here, "example-blocklist.json"), "utf8")).blocked ?? [];
// blockedBy(ex, blocklist) is imported from ./example-extract.mjs (shared with the sharded nix matrix).

// LIB_ONLY: when this module is IMPORTED (by check-one-example.mjs, the per-example shred entry point)
// rather than run as the monolithic script, skip the extraction + the whole-guide check loop — expose only
// the compiler + the reusable checker functions (checkProgram/checkExample/…). The compiler still loads at
// top-level below (the importer needs it), from CDZ_WASM_PKG if set (the shred stages the pkg per-derivation).
const LIB_ONLY = !!process.env.CHECK_EXAMPLES_LIB_ONLY;

// ---- the compiler (browser wasm) + runner (jco), loaded once ----
const pkgDir = process.env.CDZ_WASM_PKG ?? join(guideRoot, "src/wasm/pkg");
const { default: init, compile, compile_with_preloaded, compile_tests, param_test_signatures, render_value, render_syntax, export_types, repl_eval } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");
// Mirror the app run path's scalar formatting (a whole-number Float gets its `.0` back from the static
// result type) so the harness validates the SAME rendered text the browser shows.
const { formatScalarByType, resultTypeOf } = await import(join(guideRoot, "src/runner/scalarFormat.ts"));
// The shared assert prelude (assert/assert-eq/assert-ne via trap) prepended to a mode="test" example — the
// SAME prepend <Runnable mode="test"> does, so the harness gates exactly what ships. Type-only imports erase.
const { assertPreludeFor } = await import(join(guideRoot, "src/components/assertPrelude.ts"));
// The multi-file <Runnable files={…}> lowering — imported from the guide source (like wrapModule/scalarFormat
// above) so the harness lowers a multi-file example to compile_with_preloaded args EXACTLY as the app does,
// by construction (a fix to lowerToCompile can never drift the gate). See the multi-file extractor below.
const { lowerToCompile } = await import(join(guideRoot, "src/explorer/fileModel.ts"));

// ---- wrapModule / stripModule: the ONE real implementation, imported from the guide source ----
// Previously this harness carried a hand-copy of these — which silently DRIFTED from the app (a bug-(C)
// fix to `wrapModule` would have left the harness testing the OLD wrapping). Import the real module so
// the harness wraps snippets EXACTLY as the app does, by construction. `wrapModule.ts` is React-free
// (its only import is a type), so node loads it directly VIA TYPE-STRIPPING — which needs Node ≥ 22.6
// (on by default) or ≥ 20.19 with --experimental-strip-types. On an older Node the import fails with a
// cryptic "Unknown file extension .ts" loader error; catch it and say exactly what's wrong + how to fix.
let wrapModule, stripModule, gatherTestForms, ungatherTestForms;
try {
  ({ wrapModule, stripModule, gatherTestForms, ungatherTestForms } = await import(join(guideRoot, "src/components/wrapModule.ts")));
} catch (e) {
  const msg = String(e && e.message ? e.message : e);
  if (/Unknown file extension|strip.?types|\.ts/i.test(msg)) {
    console.error(
      `\ncheck-examples: cannot load src/components/wrapModule.ts — this Node (${process.version}) doesn't\n` +
        `strip TypeScript types. Use Node ≥ 22.6 (type-stripping on by default), or run with\n` +
        `\`node --experimental-strip-types scripts/check-examples.mjs\` on Node ≥ 20.19.\n` +
        `(underlying error: ${msg})`,
    );
    process.exit(1);
  }
  throw e;
}
/// The ML the reader sees after toggling: wrap the s-expr snippet, render to ML, strip the scaffolding.
function renderToMl(snippet) {
  return stripModule(render_syntax(wrapModule(snippet, "sexpr"), "sexpr", "ml"), "ml");
}

/// Render a `mode="test"` snippet (bare @test/def forms, no export/main) between surfaces. `render_syntax`
/// takes a SINGLE top-level form, but a test snippet is often MULTIPLE top-level forms (several @tests, or a
/// helper `def` + a `@test`). S-expr has no bare multi-form top level, so we gather them under a `(do …)`
/// before rendering, then strip the `(do …)` back off (ML's native top level IS multi-form, so an ML source
/// renders directly). Mirrors how the app must render a toggled test panel. Returns the snippet in `to`.
function renderTestSnippet(snippet, from, to) {
  if (from === to) return snippet.trim();
  // Gather bare multi-form into one top-level form, render, ungather — via the SHARED helpers the app's
  // `renderSnippet` also uses, so the gate and the app can never render a test panel differently again.
  const rendered = render_syntax(gatherTestForms(snippet, from), from, to);
  return ungatherTestForms(rendered, to);
}

// ---- transpile a component to disk and load its `instantiate` (mirrors runWorker.ts loadComponent) ----
async function loadComponent(componentBytes, name) {
  const { files } = await transpileBytes(new Uint8Array(componentBytes), {
    name,
    instantiation: "async",
    wasiShim: false,
    minify: false,
  });
  const dir = mkdtempSync(join(tmpdir(), "cdz-check-"));
  // jco emits nested files (e.g. `interfaces/cadenza-run-run.d.ts`) whenever the result is a COMPOUND
  // value that escapes via a resource interface — so create each file's parent dir before writing, or
  // the write ENOENTs and a tuple/map/record-returning solution looks like a run failure.
  for (const [f, b] of Object.entries(files)) {
    const p = join(dir, f);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, b);
  }
  const mod = await import(join(dir, `${name}.js`));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  return { instantiate: mod.instantiate, getCore };
}

// The value-heap runtime, instantiated once and reused. A COMPOUND-returning program imports
// `cadenza:runtime/heap`; without it, instantiation throws "Cannot destructure property 'boxInt'/…".
// The browser runner (runWorker.ts) wires this exact interface — the harness MUST too, or every
// list/map/record/tuple-returning example looks like a run failure when it actually works in-app.
const HEAP_IMPORT = "cadenza:runtime/heap";
// The value-heap runtime component. Default = the staged src/wasm/runtime.wasm (stage-wasm resolves it from
// the store by the compiler's required_runtime_hash). The per-example shred injects it via CDZ_RUNTIME_WASM
// (the flake stages the exact runtime for each ca-derivation, alongside CDZ_WASM_PKG).
const runtimePath = process.env.CDZ_RUNTIME_WASM ?? join(guideRoot, "src/wasm/runtime.wasm");

// FINDING#23: the value-heap runtime now IMPORTS `cadenza:nfc/normalize` — a separate component that
// NFC-normalizes a String's UTF-8 bytes (the heavy Unicode tables live there, not in the runtime). In
// the native/CI path cdz-run composes the real cdz-nfc component into the runtime's linker; here the jco
// harness supplies the import directly as a JS shim. `nfc: list<u8> -> list<u8>` crosses as
// (Uint8Array) => Uint8Array; NFC of well-formed UTF-8 is exactly JS's String.prototype.normalize('NFC')
// (round-tripped through UTF-8). Without this, every runtime-instantiating example throws
// "Cannot destructure property 'nfc' of imports['cadenza:nfc/normalize']". The browser runner
// (runWorker.ts) wires the same shim.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes) => {
    const s = new TextDecoder("utf-8").decode(bytes);
    return new TextEncoder().encode(s.normalize("NFC"));
  },
};

// A FRESH value-heap runtime instance PER program-run — do NOT share ONE heap across every example.
// WHY (root-caused 2026-08-28, the fleet-wide "36 playground examples trap 'memory access out of bounds'"
// gate-local blocker): a single memoized heap ACCUMULATES guest allocations across all ~410 examples in
// this one long-lived process. Those allocations live INSIDE the runtime's wasm linear memory (managed by
// the guest's Perceus reclaim, invisible to JS GC), so the shared heap grows monotonically; once it can no
// longer grow, every subsequent run traps `memory access out of bounds` — reproducibly under jco/Node in
// CI, but NOT under native cdz-run (fresh process per program) nor in the browser (runWorker.ts disposes a
// worker per run). Re-instantiating per run gives each example a clean heap, exactly like the browser and
// native paths. We cache only the (expensive) transpiled MODULE; the instance is cheap + fresh each call.
// The freed instances' V8 wasm-memory reservations are reclaimed by the `globalThis.gc()` swept at the top
// of the example loop below (the harness runs with `--expose-gc`; see package.json check:examples) — without
// that sweep the many fresh reservations would balloon the process's VIRTUAL address space.
let __runtimeModule = null;
async function getHeap() {
  if (!__runtimeModule) __runtimeModule = await loadComponent(readFileSync(runtimePath), "heap");
  // The runtime imports the NFC normalization component — supply the JS shim so it instantiates.
  const root = await __runtimeModule.instantiate(__runtimeModule.getCore, { [NFC_IMPORT]: nfcHostImport });
  return root[HEAP_IMPORT] ?? root["heap"];
}

// ---- run a compiled component through jco, return its rendered value text ----
// `program`/`surface` (optional) let the SCALAR path recover a whole-number Float's `.0` from the static
// export type — the same fix the app run path applies (see runner/scalarFormat.ts). Omitting them (the
// expect="error" probe) just skips that formatting.
async function runComponent(componentBytes, program, surface) {
  const prog = await loadComponent(componentBytes, "prog");
  const heap = await getHeap();
  // The example PROGRAM also links the value-heap runtime, so it too imports cadenza:nfc/normalize and
  // needs the NFC shim (not just the shared runtime in getHeap) — supply it at every program-instantiate.
  const imports = heap
    ? { [HEAP_IMPORT]: heap, [NFC_IMPORT]: nfcHostImport }
    : { [NFC_IMPORT]: nfcHostImport };
  const root = await prog.instantiate(prog.getCore, imports);
  // Compound result: the resource-escape path (make/encode). Scalar: the sole exported function.
  const iface = root["cadenza:run/run"] ?? root["run"];
  if (iface && typeof iface.make === "function") {
    return render_value(iface.encode(iface.make())); // canonical value text, e.g. "(: (tuple 1 2) …)"
  }
  const fn = Object.values(root).find((v) => typeof v === "function");
  if (!fn) return null;
  const value = String(fn());
  // Only an integer-looking render could need the Float `.0`; skip the export-types query otherwise
  // (mirrors the app run path's gate).
  if (program == null || !/^-?\d+$/.test(value.trim())) return value;
  return formatScalarByType(value, resultTypeOf(export_types(program, surface)));
}

// ---- run a TEST-LAYOUT component's @test exports (mirrors runWorker's test mode) ----
// Instantiate the test component, invoke each named nullary @test export, and report pass/fail: a clean
// return = pass, a trap/throw = fail. A source name (`one_plus_one`) crosses the boundary as kebab/camel
// (`one-plus-one`/`onePlusOne`), so match by a normalized key (strip -/_ + lowercase).
const normName = (n) => n.replace(/[-_]/g, "").toLowerCase();
async function runTestExports(componentBytes, testNames) {
  const prog = await loadComponent(componentBytes, "prog");
  const heap = await getHeap();
  // The example PROGRAM also links the value-heap runtime, so it too imports cadenza:nfc/normalize and
  // needs the NFC shim (not just the shared runtime in getHeap) — supply it at every program-instantiate.
  const imports = heap
    ? { [HEAP_IMPORT]: heap, [NFC_IMPORT]: nfcHostImport }
    : { [NFC_IMPORT]: nfcHostImport };
  const root = await prog.instantiate(prog.getCore, imports);
  const byNorm = new Map();
  for (const [name, v] of Object.entries(root)) if (typeof v === "function") byNorm.set(normName(name), v);
  const results = [];
  for (const name of testNames) {
    const fn = byNorm.get(normName(name));
    if (typeof fn !== "function") { results.push({ name, pass: false, error: "export not found" }); continue; }
    try { fn(); results.push({ name, pass: true }); }
    catch (e) { results.push({ name, pass: false, error: String(e && e.message ? e.message : e).slice(0, 60) }); }
  }
  return results;
}

// ---- drive SCALAR-param property tests (mirrors runWorker's scalar driver) ----
// A scalar-param @test keeps its params on the export (compound:false in param_test_signatures), so the
// driver generates a value per param type and calls fn(...args) over trials; a throw = a failing trial,
// shrunk toward the minimal counterexample. Keeps the gate in lockstep with the in-browser driver.
const INT_RANGE = {
  int8: [-128n, 127n], int16: [-32768n, 32767n], int32: [-2147483648n, 2147483647n],
  int64: [-9223372036854775808n, 9223372036854775807n],
  uint8: [0n, 255n], uint16: [0n, 65535n], uint32: [0n, 4294967295n], uint64: [0n, 18446744073709551615n],
};
const lcg = (s) => (s * 6364136223846793005n + 1442695040888963407n) & 0xffffffffffffffffn;
function genArgFor(type, state) {
  const next = lcg(state);
  if (type === "bool") return { arg: (next & 1n) === 0n, state: next };
  if (type === "float32" || type === "float64") return { arg: Number(next % 2048n) - 1024, state: next };
  const range = INT_RANGE[type];
  if (!range) return { arg: 0n, state: next };
  const span = range[1] - range[0] + 1n;
  return { arg: range[0] + (((next % span) + span) % span), state: next };
}
function genArgsFor(paramTypes, seed) {
  let state = seed; const args = [];
  for (const t of paramTypes) { const { arg, state: s } = genArgFor(t, state); args.push(arg); state = s; }
  return args;
}
async function runScalarProps(componentBytes, sigs) {
  const scalar = sigs.filter((s) => !s.compound);
  if (scalar.length === 0) return [];
  const prog = await loadComponent(componentBytes, "prog");
  const heap = await getHeap();
  const root = await prog.instantiate(
    prog.getCore,
    heap
      ? { [HEAP_IMPORT]: heap, [NFC_IMPORT]: nfcHostImport }
      : { [NFC_IMPORT]: nfcHostImport },
  );
  const byNorm = new Map();
  for (const [name, v] of Object.entries(root)) if (typeof v === "function") byNorm.set(normName(name), v);
  const results = [];
  for (const sig of scalar) {
    const name = sig.name;
    // The raw wasm binding returns a `ParamTestSignature` class with snake_case `param_types` (the TS
    // client wrapper is what renames it to `paramTypes`; here we call the wasm export directly).
    const paramTypes = sig.param_types;
    const fn = byNorm.get(normName(name));
    if (typeof fn !== "function") { results.push({ name, pass: false, error: "property export not found" }); continue; }
    const runArgs = (args) => { try { fn(...args); return false; } catch { return true; } };
    let failing = null;
    for (let t = 0; t < 100; t++) { const a = genArgsFor(paramTypes, BigInt(t) + 1n); if (runArgs(a)) { failing = a; break; } }
    if (!failing) { results.push({ name, pass: true }); continue; }
    // SHRINK the failing args toward the minimal counterexample (mirrors runWorker's runScalarProperty +
    // the native shrink_pool), then RECORD the counterexample — so the gate stays in lockstep with the
    // in-browser driver AND pins the counterexample-render feature (a failing property surfaces its value).
    const best = failing.slice();
    for (let i = 0; i < best.length; i++) {
      let v = best[i];
      while (typeof v === "bigint" && v !== 0n) {
        const cand = best.slice(); cand[i] = v / 2n;
        if (runArgs(cand)) { best[i] = cand[i]; v = cand[i]; } else break;
      }
      while (typeof v === "number" && v !== 0) {
        const cand = best.slice(); cand[i] = Math.trunc(v / 2);
        if (runArgs(cand)) { best[i] = cand[i]; v = cand[i]; } else break;
      }
    }
    const rendered = `${name}(${best.map((a) => (typeof a === "bigint" ? a.toString() : String(a))).join(", ")})`;
    results.push({ name, pass: false, error: "property failed", counterexample: { args: rendered, seed: 0 } });
  }
  return results;
}

// ---- drive COMPOUND-param property tests (mirrors runWorker's compound driver — keeps the gate in lockstep) ----
// A compound-param @test (List/tuple/record/…) compiles to a NULLARY `-gen` wrapper that builds its argument
// guest-side by consuming a seeded int stream via the `Test.gen-int` host op (jco binds the kebab op as the
// camelCase member `genInt`). Per trial: instantiate with a `test.gen-int` pool + invoke; a throw = a failing
// trial. On failure, shrink over the int pool (truncate trailing draws, then halve leaves toward 0), replaying
// a preset pool that pads exhausted draws with 0 (so truncation is a faithful shrink). Byte-for-byte the same
// contract as the in-browser runWorker compound driver.
class GenPool {
  constructor(seed, preset) { this.state = seed; this.replay = preset !== undefined; this.values = preset ? preset.slice() : []; this.i = 0;
    this.next = () => { if (this.i >= this.values.length) { if (this.replay) return 0n; this.state = lcg(this.state); this.values.push(this.state & 0xffffffffffffffffn); } return this.values[this.i++]; }; }
}
async function runCompoundProps(componentBytes, sigs) {
  const compound = sigs.filter((s) => s.compound);
  if (compound.length === 0) return [];
  const heap = await getHeap();
  const results = [];
  const runPool = async (name, pool) => {
    const prog = await loadComponent(componentBytes, "prog");
    const root = await prog.instantiate(prog.getCore, { [HEAP_IMPORT]: heap, [NFC_IMPORT]: nfcHostImport, test: { "gen-int": pool.next, genInt: pool.next } });
    const byNorm = new Map(Object.entries(root).filter(([, v]) => typeof v === "function").map(([k, v]) => [normName(k), v]));
    const fn = byNorm.get(normName(name));
    if (typeof fn !== "function") throw new Error("compound property export not found");
    try { await fn(); return false; } catch (e) { if (/gen-int|test\.gen|unhandled|host/i.test(String(e && e.message ? e.message : e))) throw e; return true; }
  };
  for (const sig of compound) {
    const name = sig.name;
    let failing = null;
    try {
      for (let t = 0; t < 100; t++) { const p = new GenPool(BigInt(t) + 1n); if (await runPool(name, p)) { failing = p.values.slice(); break; } }
    } catch (e) { results.push({ name, pass: false, error: `compound driver: ${String(e && e.message ? e.message : e).slice(0, 60)}` }); continue; }
    if (!failing) { results.push({ name, pass: true }); continue; }
    let best = failing.slice();
    for (let len = best.length - 1; len >= 1; len--) { const c = best.slice(0, len); if (await runPool(name, new GenPool(0n, c))) best = c; else break; }
    for (let i = 0; i < best.length; i++) { let v = best[i]; while (v !== 0n) { const c = best.slice(); c[i] = v / 2n; if (await runPool(name, new GenPool(0n, c))) { best[i] = c[i]; v = c[i]; } else break; } }
    results.push({ name, pass: false, error: "property failed", counterexample: { args: `${name}(<generated> pool:[${best.map((n) => n.toString()).join(",")}])`, seed: 0 } });
  }
  return results;
}

// ---- run a `mode="test"` snippet in ONE surface: compile-tests, run each @test, assert expected pass/fail ----
// `snippet` is the test-defs source ALREADY in `surface`. Returns null on success, else a reason string.
async function runTestInSurface(ex, snippet, surface, where) {
  const brief = snippet.replace(/\n/g, " ").slice(0, 80);
  // Prepend the shared assert prelude unless the example opted out (prelude={false}) — mirrors <Runnable
  // mode="test">, so the gate compiles+runs exactly what the reader's example runs.
  const program = ex.prelude ? `${assertPreludeFor(surface)}\n${snippet}` : snippet;
  let r;
  try {
    r = compile_tests(program, surface);
  } catch (e) {
    return `${ex.file} [test] (${where}): parse error — ${String(e.message || e).slice(0, 80)}\n    ${brief}`;
  }
  if (!r.component) {
    const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
    return `${ex.file} [test] (${where}): test compile DECLINED — ${d ? `${d.code} ${d.message}` : "no @test / no component"}\n    ${brief}`;
  }
  // Gather property signatures so a parameterized @test is DRIVEN, not treated as "nothing to run".
  // param_test_signatures classifies each: compound:false = scalar (arg-driver), compound:true = a `-gen`
  // wrapper (gen-int-pool driver). BOTH run live now — in lockstep with the in-browser runWorker drivers.
  let sigs = [];
  try { sigs = param_test_signatures(program, surface) ?? []; } catch { sigs = []; }
  const propSigs = sigs.filter((s) => s.compound || !s.compound); // all classified params are drivable
  if (r.nullary_test_names.length === 0 && propSigs.length === 0) {
    return `${ex.file} [test] (${where}): a mode="test" example has no runnable @test defs (no nullary, no property)\n    ${brief}`;
  }
  const nullaryResults = r.nullary_test_names.length > 0
    ? await runTestExports(r.component, r.nullary_test_names)
    : [];
  const propResults = await runScalarProps(r.component, sigs);
  const compoundResults = await runCompoundProps(r.component, sigs);
  const results = [...nullaryResults, ...propResults, ...compoundResults];
  const failed = results.filter((t) => !t.pass);
  if (ex.expect === "error") {
    // A teaching example demonstrating a FAILING test: at least one @test must fail.
    if (failed.length > 0) return null;
    return `${ex.file} [test] (${where}): expect="error" but every @test PASSED\n    ${brief}`;
  }
  // Default: every nullary @test must pass.
  if (failed.length === 0) return null;
  const detail = failed
    .map((t) => `${t.name}: ${t.error ?? "failed"}${t.counterexample ? ` [counterexample: ${t.counterexample.args}]` : ""}`)
    .join("; ");
  return `${ex.file} [test] (${where}): @test(s) FAILED — ${detail}\n    ${brief}`;
}

// ---- check a `mode="test"` example in BOTH surfaces (the reader can toggle) ----
// The reader toggles the surface, so a test example RUNS in whichever surface is active — the ML render+run
// path must work too, NOT just the authored surface. (This cross-surface pass is what would have caught the
// kebab-prelude bug: the ML assert prelude once used `assert_eq`, but an ML `assert-eq` call resolves to the
// KEBAB name, so every rendered-to-ML test failed while the authored s-expr pass stayed green.) We render the
// authored snippet to the other surface via `render_syntax` and run it there with THAT surface's prelude.
async function checkTestProgram(ex) {
  const authored = ex.surface ?? "sexpr";
  const authoredFail = await runTestInSurface(ex, ex.snippet, authored, authored === "ml" ? "ML" : "s-expr");
  if (authoredFail) return authoredFail;
  const other = authored === "ml" ? "sexpr" : "ml";
  let otherSnippet;
  try {
    otherSnippet = renderTestSnippet(ex.snippet, authored, other);
  } catch (e) {
    return `${ex.file} [test] (${other === "ml" ? "ML" : "s-expr"} toggle): render threw — ${String(e.message || e).slice(0, 80)}`;
  }
  return runTestInSurface(ex, otherSnippet, other, `${other === "ml" ? "ML" : "s-expr"} toggle`);
}

// ---- extract `source=`/`solution=`/`expected=`/`expect=` from a chapter's TSX ----
// cookTemplate / extractFilesProp / extractExamples now live in ./example-extract.mjs (imported above),
// shared verbatim with scripts/shred-examples.mjs so the gate and the shred can never drift.

// ---- gather every example across the content (skipped under LIB_ONLY — the shred enumerates per-case) ----
let files = [];
let examples = [];
if (!LIB_ONLY) {
const chaptersDir = join(guideRoot, "src/content/chapters");
files = [
  ...readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).map((f) => join(chaptersDir, f)),
  join(guideRoot, "src/components/HomePage.tsx"),
];
examples = files.flatMap((f) => {
  try {
    return extractExamples(readFileSync(f, "utf8"), f.replace(guideRoot + "/", ""));
  } catch (e) {
    // Do NOT silently drop a file's examples on an extraction error — a swallowed throw here would
    // quietly shrink the checked set (a broken read/parse becomes "0 examples, 0 failed" = false green).
    // Fail loud so a regression in extraction can't hide behind a green gate.
    console.error(`check-examples: could not extract examples from ${f} — ${String(e && e.message ? e.message : e)}`);
    process.exit(1);
  }
});

// Vacuous-pass floor: if the glob/extraction breaks, `examples` could be empty and the gate would print
// "checked 0 examples … 0 failed" and exit 0 — a silent false green. The guide has 37 chapters + HomePage
// and hundreds of examples; assert a sane minimum so a broken discovery path FAILS instead of passing on
// nothing. (Mirrors proseEmDash.test.ts's "guards a vacuous pass" assertion.)
if (files.length < 30) {
  console.error(`check-examples: expected ≥30 content files (37 chapters + HomePage), found ${files.length} — the chapter glob likely broke.`);
  process.exit(1);
}
if (examples.length < 100) {
  console.error(`check-examples: expected ≥100 examples across the guide, found ${examples.length} — extraction likely broke (a vacuous pass would ship an unchecked guide).`);
  process.exit(1);
}
} // end if (!LIB_ONLY) — extraction + vacuous-pass floor

// ---- the playground's Examples-dropdown programs (src/playground/examples.ts) ----
// These are FULL modules (the playground compiles its buffer verbatim, no wrapping) authored in the
// s-expr surface — the reader loads one, then may toggle to ML. They ship in the dropdown, so they're
// exactly the "every example is a test" surface: each must compile AND run. Loaded via node's
// type-stripping (like wrapModule.ts above; needs Node ≥ 22.6 / ≥ 20.19). Marked `noWrap` (already whole
// modules) and their authored surface, so `checkProgram` compiles+runs each in the surface it's written
// in — the same real browser compiler the reader hits.
try {
  const { EXAMPLES: PLAYGROUND } = await import(join(guideRoot, "src/playground/examples.ts"));
  // Vacuous-pass floor for the playground set (mirrors the chapter floor above): if a bad merge, a
  // rename, or an errant edit empties/shrinks `EXAMPLES`, the loop below would push zero examples and the
  // gate would go GREEN on an UNCHECKED playground — the same silent false-green the chapter floor guards.
  // The dropdown ships 59 examples; assert a sane minimum so a broken/gutted array FAILS instead of
  // passing on nothing. (A legit prune below the floor should lower it deliberately, not slip past.)
  // Floor tracks the grown library with a small margin for intentional churn — a gutted/halved array
  // must FAIL, not squeak past an over-loose bound.
  if (!Array.isArray(PLAYGROUND) || PLAYGROUND.length < 55) {
    console.error(
      `check-examples: expected ≥55 playground examples in src/playground/examples.ts, found ` +
        `${Array.isArray(PLAYGROUND) ? PLAYGROUND.length : "a non-array export"} — the EXAMPLES array was ` +
        `gutted/renamed (a vacuous pass would ship an unchecked playground dropdown).`,
    );
    process.exit(1);
  }
  // UNIQUE-ID invariant: the playground UI keys off `id` — deep-links resolve `EXAMPLES.find(e => e.id
  // === reqId)` (a dup silently loads the FIRST match, so a deep-link/Run-this-example lands on the wrong
  // program) and the dropdown renders `<option key={e.id}>` (a dup id is a React key collision). A
  // copy-paste slip that duplicates an id compiles + runs fine, so neither this harness nor the compiler
  // catches it — pin it here. (Also require a non-empty string id, since `find`/`key` both need one.)
  const idCounts = new Map();
  for (const p of PLAYGROUND) {
    if (typeof p.id !== "string" || p.id.length === 0) {
      console.error(`check-examples: a playground example has a missing/empty \`id\` (name="${p.name ?? "?"}") — the dropdown/deep-link key off id.`);
      process.exit(1);
    }
    idCounts.set(p.id, (idCounts.get(p.id) ?? 0) + 1);
  }
  const dupIds = [...idCounts].filter(([, n]) => n > 1).map(([id]) => id);
  if (dupIds.length) {
    console.error(
      `check-examples: duplicate playground example id(s) in src/playground/examples.ts: ${dupIds.join(", ")} — ` +
        `the dropdown keys <option key={id}> (React key collision) and deep-links resolve EXAMPLES.find(e => e.id === id) ` +
        `(loads the FIRST match, so a deep-link lands on the wrong example). Give each example a unique id.`,
    );
    process.exit(1);
  }
  // Pin the negative-case coverage: the playground must retain at least one `expectError` example (the
  // "see the squiggle" type-error teaching case). Without this, dropping that example would silently
  // remove the only assertion that the compiler still REJECTS bad code in the playground path.
  if (!PLAYGROUND.some((p) => p.expectError)) {
    console.error(
      `check-examples: no playground example carries \`expectError: true\` — the intentional "see the ` +
        `squiggle" type-error case was dropped, removing the sole assertion that the playground path still ` +
        `rejects invalid programs. Restore an expectError example.`,
    );
    process.exit(1);
  }
  // THEME-VALIDITY invariant: the sidebar's "Examples" section groups by `theme` (v-guide-infra renders
  // the buckets from this data). examples.ts is imported here with type-stripping (no typecheck), so a
  // mistyped theme ("algorithm", a stray new bucket) would pass this sweep silently yet render a broken or
  // empty nav bucket in the browser. Pin the closed set the Example union declares so a typo fails LOUDLY.
  const THEMES = new Set(["basics", "algorithms", "data-and-collections", "numbers"]);
  const badTheme = PLAYGROUND.filter((p) => !THEMES.has(p.theme));
  if (badTheme.length) {
    console.error(
      `check-examples: playground example(s) with an unknown \`theme\`: ` +
        `${badTheme.map((p) => `${p.id}="${p.theme}"`).join(", ")} — the sidebar groups by theme and only ` +
        `renders {${[...THEMES].join(", ")}}. A typo'd/new theme ships an example into a broken/empty nav ` +
        `bucket. Use a declared theme (or extend both the Example union and this guard together).`,
    );
    process.exit(1);
  }
  // SURFACE-VALIDITY invariant: `surface` must be one of the compiler's declared surfaces (Surface =
  // "ml" | "sexpr", src/compiler/worker.ts). Same type-stripping gap as theme — a typo ("sexp", "sexpr ")
  // isn't caught by the (stripped) union, and would surface only as a confusing downstream compile error
  // in the wrong surface rather than a pointed one here. (This does NOT restrict to sexpr — ml stays a
  // valid authored surface; the separate guard below only forbids a non-sexpr `expected` PIN.)
  const SURFACES = new Set(["ml", "sexpr"]);
  const badSurface = PLAYGROUND.filter((p) => !SURFACES.has(p.surface));
  if (badSurface.length) {
    console.error(
      `check-examples: playground example(s) with an unknown \`surface\`: ` +
        `${badSurface.map((p) => `${p.id}="${p.surface}"`).join(", ")} — must be one of ` +
        `{${[...SURFACES].join(", ")}}. A typo compiles the example in a bogus surface (confusing failure).`,
    );
    process.exit(1);
  }
  for (const p of PLAYGROUND) {
    // The intentional "see the squiggle" example is authored to NOT compile — check it as expect="error".
    // Keyed off the example's explicit `expectError` field (declared in examples.ts), NOT sniffed from the
    // source: re-authoring the example's body to a different type error must not silently reclassify it as a
    // value-example (which would then misreport the intended decline as a sweep failure).
    const expect = p.expectError ? "error" : "value";
    // An example MAY pin its exact result via `expected` — then the gate asserts the program runs to
    // THAT value, not merely "to some value", so a future compiler change that flips e.g. Collatz from
    // 111 to 42 is caught instead of silently accepted (a true regression test). A playground example
    // may pin EITHER a scalar OR a compound value: it has no in-browser Check (unlike a graded exercise),
    // so an s-expr-canonical compound like `(: (list 1 2 3) (List Int64))` is stable to pin. The value is
    // compared on the S-EXPR pass (`checkProgram`'s `if (surface === "sexpr")` block); `checkExample` runs
    // BOTH surfaces, so a pin IS checked whether the example is sexpr-authored (its authored pass) or
    // ml-authored (its render_syntax'd sexpr TOGGLE pass). The catch: an ml-authored pin is asserted only
    // against the RENDERED sexpr output, so its value depends on the ML→s-expr render being byte-stable —
    // brittle, and the pin reads in a different surface than it's maintained in. So require sexpr-authored
    // pins: all playground examples are sexpr, and a pin should live in the same surface it's asserted in.
    if (p.expected != null && p.surface !== "sexpr")
      throw new Error(
        `playground example "${p.id}" pins \`expected\` but is authored surface="${p.surface}"; ` +
          `\`expected\` is compared on the s-expr pass, so an ml-authored pin is only checked against the ` +
          `RENDERED s-expr toggle output (brittle — depends on a byte-stable ML→s-expr render). ` +
          `Author it in s-expr (all playground examples are), or drop the \`expected\`.`,
      );
    examples.push({
      file: "src/playground/examples.ts",
      kind: "Runnable",
      snippet: p.source,
      surface: p.surface,
      expect,
      expected: p.expected ?? null,
      noWrap: true,
    });
  }
} catch (e) {
  console.error(`check-examples: could not load playground examples — ${String(e && e.message ? e.message : e)}`);
  process.exit(1);
}

// ---- guide-accuracy guard: an `expect="error"` example must demonstrate a real SEMANTIC error, NOT a
// construct the compiler doesn't model. A decline of the "unbound name `X` at the top level … it is not one
// this compiler models" class (rcdzc compile.rs, unknown_top_forms / unbound_bare_name_items) means the
// example documents a NON-EXISTENT construct — the exact hole the operator hit (guide-editor 2026-09-02): a
// documented `@invariant`-style feature that silently regresses to "unbound" declines that way and, masked by
// `expect="error"`, PASSES the gate — so a documented-but-nonexistent feature ships unnoticed. Treat that
// decline class as a FAILURE (the guide is documenting something the compiler doesn't model), while a genuine
// semantic decline (a real CDZ code — type mismatch, out-of-range, non-exhaustive match) IS the intended
// teaching decline and still passes. Returns the offending diagnostic, or undefined if no diagnostic matches.
function unmodeledConstructDecline(diagnostics) {
  return (diagnostics ?? []).find((d) => {
    const m = String(d && d.message ? d.message : "");
    return /\bunbound name\b[\s\S]*\bat the top level\b/i.test(m) || /not one this compiler models/i.test(m);
  });
}

// ---- check one program (already wrapped) in one surface, returning null on success or a reason ----
async function checkProgram(program, surface, ex, where) {
  const brief = ex.snippet.replace(/\n/g, " ").slice(0, 80);
  let r;
  try {
    r = compile(program, surface);
  } catch (e) {
    // A throw = a parse error. Fine only if the example is meant to fail.
    if (ex.expect === "error") return null;
    return `${ex.file} [${ex.kind}] (${where}): parse error — ${String(e.message || e).slice(0, 80)}\n    ${brief}`;
  }
  const declined = !r.component;
  if (ex.expect === "error") {
    // "meant to fail" = a compile decline OR a runtime trap (e.g. `(UInt8.of 300)`); accept either — EXCEPT a
    // decline because the CONSTRUCT IS UNMODELED (unbound at the top level), which means the guide documents a
    // feature the compiler doesn't model, not the intended semantic error (see unmodeledConstructDecline).
    if (declined) {
      const unmodeled = unmodeledConstructDecline(r.diagnostics);
      if (unmodeled)
        return `${ex.file} [${ex.kind}] (${where}): expect="error" but the example DECLINES because the CONSTRUCT IS UNMODELED, not because of the intended semantic error — "${String(unmodeled.message).slice(0, 130)}". A documented construct the compiler declines as unbound/not-modeled is a corpus-is-paramount violation (the guide documents a non-existent feature): demonstrate a real semantic error, or route the compiler gap to its owner + block the example in example-blocklist.json.\n    ${brief}`;
      return null;
    }
    try { await runComponent(r.component); } catch { return null; }
    return `${ex.file} [${ex.kind}] (${where}): expect="error" but it compiled AND ran to a value\n    ${brief}`;
  }
  if (declined) {
    const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
    return `${ex.file} [${ex.kind}] (${where}): expected to compile but DECLINED — ${d ? `${d.code} ${d.message}` : "no component"}\n    ${brief}`;
  }
  // Compiles. Now RUN it (on the s-expr surface — running once per example is enough; the ML pass only
  // guards the wrap/strip round-trip, not a second execution). Compiling is NOT enough: the operator
  // hit an intro example that compiled but CRASHED in the browser ("Maximum call stack size exceeded").
  // A guide example that throws/traps/stack-overflows at RUN time is exactly the trust-breaker the
  // "every example is a test" mandate targets — so every non-error example must reach a value here.
  if (surface === "sexpr") {
    // A graded EXERCISE MUST return a SCALAR. The browser's Check (Exercise.tsx) compares the result
    // rendered in the reader's CURRENT surface, but this harness renders s-expr canonical — a scalar
    // (bare number/bool) reads identically in both, a COMPOUND does NOT (`(: (map …) …)` vs ML
    // `#{…} : Map(…)`). So a compound `expected` would pass here yet FAIL the in-browser Check in ML.
    // Reject it at authoring time; return the compound as a Runnable (ungraded) instead.
    // A PLAYGROUND example (from examples.ts) is exempt: it has NO in-browser Check — it's just loaded
    // and Run — and the harness asserts its value ONLY in this authored s-expr surface (the ML-toggle
    // call has surface==="ml" and skips this block), so an s-expr-canonical compound is stable to pin.
    const isPlayground = ex.file === "src/playground/examples.ts";
    if (!isPlayground && ex.expected != null && /^\(:/.test(ex.expected.trim()))
      return `${ex.file} [Exercise] (${where}): \`expected\` is a COMPOUND value (${JSON.stringify(ex.expected.slice(0, 40))}…) — graded exercises must return a SCALAR (it's compared in the reader's surface, and a compound renders differently in ML vs s-expr). Show the compound as a Runnable instead.\n    ${brief}`;
    let got;
    try {
      got = await runComponent(r.component, program, surface);
    } catch (e) {
      // A run failure is the trust-breaker: a compiled example that crashes/traps/stack-overflows.
      const label = ex.expected != null ? "solution" : ex.kind;
      const emsg = String(e.message || e);
      // A wasm TYPE-mismatch at run ("expected i32, found i64", "type mismatch", "failed to parse
      // WebAssembly module") is the signature of a compiler EMIT bug in the *staged* wasm — which is often
      // one the compiler has already FIXED on trunk since this local `src/wasm/pkg` was built. Locally that
      // reads as a scary miscompile; in CI (which rebuilds guide-wasm fresh every run) it's usually green.
      // So on that error class, hint the most common cause before anyone escalates a false regression. (The
      // staged store is internally consistent — compiler hash == runtime hash — so a hash-guard can't catch
      // this; only rebuilding from trunk can. See the guide-infra stale-store trap.)
      const staleHint = /expected i(32|64), found i(32|64)|type mismatch|failed to parse WebAssembly/i.test(emsg)
        ? `\n    ↳ HINT: a wasm type-mismatch at RUN often means your LOCAL src/wasm/pkg is a STALE compiler build (a `
          + `since-fixed emit bug). If this appeared after a trunk update, rebuild with \`cargo xtask guide-wasm\` `
          + `and re-run before treating it as a real regression — CI rebuilds fresh, so it may already be green there.`
        : "";
      return `${ex.file} [${ex.kind}] (${where}): ${label} compiled but FAILED TO RUN — ${emsg.slice(0, 100)}\n    ${brief}${staleHint}`;
    }
    // A graded exercise additionally asserts the rendered scalar equals its stated `expected`.
    // Normalize LAYOUT before comparing: the renderer pretty-prints a large nested COMPOUND value with
    // newlines + indentation, but the token structure — not the line wrapping — is the contract. Collapse
    // any whitespace run that contains a newline to a single space so a pinned compound can be authored on
    // one line regardless of how the renderer chooses to wrap it. This is a no-op for a scalar (no
    // newlines), so it never weakens a bare-number/bool exercise pin.
    const normLayout = (s) => String(s).replace(/\s*\n\s*/g, " ").trim();
    if (ex.expected != null && normLayout(got) !== normLayout(ex.expected))
      return `${ex.file} [Exercise] (${where}): solution ran to ${JSON.stringify(String(got))}, expected ${JSON.stringify(ex.expected)}\n    ${brief}`;
  }
  return null;
}

// ---- check a MULTI-FILE Runnable: lower the file set + compile it together via compile_with_preloaded ----
// Reuses lowerToCompile (the SAME lowering the app's MultiFileRunnable uses) so the gate compiles exactly
// what ships. Compiled in the entry file's surface; run to a value (or asserted to decline for expect=error),
// and if the Runnable carries `expected`, the rendered value must equal it. A single compile+run (not a
// both-surface toggle) — a multi-file example's files carry their own complete modules, no wrap/strip round-trip.
async function checkMultiFile(ex) {
  const brief = `${ex.files.map((f) => f.name).join(" + ")}`;
  const low = lowerToCompile(ex.files);
  if (!low.ok) return `${ex.file} [Runnable multi-file] (${brief}): file set won't lower — ${low.reason}`;
  const { text, from, names, sources, formats } = low.lowered;
  let r;
  try {
    r = compile_with_preloaded(text, from, names, sources, formats);
  } catch (e) {
    if (ex.expect === "error") return null;
    return `${ex.file} [Runnable multi-file] (${brief}): parse error — ${String(e.message || e).slice(0, 80)}`;
  }
  const declined = !r.component;
  if (ex.expect === "error") {
    if (declined) {
      // Same guard as the single-file path: a decline because the construct is UNMODELED (unbound at the top
      // level) is NOT an acceptable teaching decline — it documents a feature the compiler doesn't model.
      const unmodeled = unmodeledConstructDecline(r.diagnostics);
      if (unmodeled)
        return `${ex.file} [Runnable multi-file] (${brief}): expect="error" but the file set DECLINES because the CONSTRUCT IS UNMODELED, not because of the intended semantic error — "${String(unmodeled.message).slice(0, 130)}". A documented construct declined as unbound/not-modeled is a corpus-is-paramount violation: demonstrate a real semantic error, or route the compiler gap + block the example.`;
      return null;
    }
    try { await runComponent(r.component, text, from); } catch { return null; }
    return `${ex.file} [Runnable multi-file] (${brief}): expect="error" but the file set compiled AND ran to a value`;
  }
  if (declined) {
    const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
    return `${ex.file} [Runnable multi-file] (${brief}): expected to compile but DECLINED — ${d ? `${d.code} ${d.message}` : "no component"}`;
  }
  let got;
  try {
    got = await runComponent(r.component, text, from);
  } catch (e) {
    return `${ex.file} [Runnable multi-file] (${brief}): compiled but FAILED TO RUN — ${String(e.message || e).slice(0, 100)}`;
  }
  if (ex.expected != null) {
    // Compare `got` vs `expected` after normalizing BOTH the same way, so an author can pin EITHER the bare
    // value (e.g. `asked-model; …`, matching the single-file scalar convention — a String result renders via
    // the compound path as `(: "…" String)`, which is verbose) OR the full render form. normValue: collapse
    // layout, strip a `(: <value> <type>)` ascription wrapper, and unquote a `"…"` string leaf. Applied to
    // both sides so it's backward-compatible (a landed render-form pin still matches) AND ergonomic (a bare
    // trace matches too) — v-guide-editor's authoring nicety without breaking the shipped agent-loop pin.
    if (normValue(got) !== normValue(ex.expected))
      return `${ex.file} [Runnable multi-file] (${brief}): ran to ${JSON.stringify(String(got))}, expected ${JSON.stringify(ex.expected)}`;
  }
  return null;
}

/// Normalize a multi-file result / expected for comparison: collapse layout whitespace, strip an outer
/// `(: <value> <type>)` ascription (the render form of a non-scalar), and unquote a `"…"` string leaf — so
/// `(: "hi" String)`, `"hi"`, and `hi` all compare equal. A scalar (bare number/bool) is untouched (no
/// ascription, no quotes). Pure + deterministic; unit-tested in check-examples' multi-file self-check.
function normValue(s) {
  let t = String(s).replace(/\s*\n\s*/g, " ").trim();
  // strip a single outer (: <value> <type>) ascription — the value is everything between `(:␠` and the
  // LAST top-level space+type. Simplest robust form for the common `(: "…" String)` / `(: 3 Int64)` shapes:
  const asc = t.match(/^\(:\s+([\s\S]*)\s+[A-Za-z][\w.]*\)$/);
  if (asc) t = asc[1].trim();
  // unquote a "…" string leaf (resolve the common \" and \\ escapes), leaving a bare scalar as-is.
  if (t.length >= 2 && t.startsWith('"') && t.endsWith('"')) {
    t = t.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
  }
  return t;
}

// ---- check one example in BOTH surfaces (the reader can toggle); null on success, else a reason ----
async function checkExample(ex) {
  // A MULTI-FILE Runnable (`files={[…]}`) compiles the file SET together via compile_with_preloaded (the
  // explorer seam), NOT the single-snippet wrap path — a distinct path checked here.
  if (ex.files) return checkMultiFile(ex);
  // A `mode="test"` Runnable is a program of @test defs run as tests (like `cdz test`) — a distinct path
  // (compile_tests + invoke each @test export), not the eval-main path. Checked in BOTH surfaces (the
  // reader toggles): the authored surface + the render_syntax'd other surface, so the ML render+run path
  // is gated too (this closes the gap that let the kebab-prelude bug ship).
  if (ex.isTest) return checkTestProgram(ex);
  // A PLAYGROUND example is a FULL module authored in its own `surface`; the reader loads it, then may
  // toggle. It's compiled verbatim (`noWrap`) in its authored surface, then RE-RENDERED whole to the
  // other surface (`render_syntax`) and compiled again — so a broken toggle round-trip is caught. This
  // is a distinct path from the chapter-snippet wrap/strip path below (which is UNCHANGED).
  if (ex.surface) {
    const authored = ex.surface;
    const authoredFail = await checkProgram(ex.snippet.trim(), authored, ex, authored === "ml" ? "ML" : "s-expr");
    if (authoredFail) return authoredFail;
    const other = authored === "ml" ? "sexpr" : "ml";
    try {
      const otherProgram = render_syntax(ex.snippet.trim(), authored, other);
      const otherFail = await checkProgram(otherProgram, other, ex, `${other === "ml" ? "ML" : "s-expr"} toggle`);
      if (otherFail) return otherFail;
    } catch (e) {
      return `${ex.file} [${ex.kind}] (${other} toggle): render threw — ${String(e.message || e).slice(0, 80)}`;
    }
    return null;
  }

  // An ML-AUTHORED chapter snippet (authoredIn="ml"): the mirror of the default path — wrap+compile in ML
  // FIRST (the authored surface the reader sees), then render the ML source to s-expr and wrap+compile THAT
  // for the toggle pass. Reducer forms (`type … | Ctor(Record(…))`, `def apply(…) -> List(…)`) read
  // naturally only in ML, so the chapter authors them there; both surfaces are still gated.
  if (!ex.noWrap && ex.authoredIn === "ml") {
    const mlProgram = wrapModule(ex.snippet, "ml");
    const mlFail = await checkProgram(mlProgram, "ml", ex, "ML");
    if (mlFail) return mlFail;
    try {
      // Toggle pass — the EXACT mirror of renderToMl in the ml→s-expr direction, so both toggle
      // directions exercise identical wrap → render → STRIP → rewrap. The reader's real toggle re-renders
      // the wrapped ML to s-expr, STRIPS back to the bare displayed snippet, then REWRAPS to compile/run;
      // compiling the un-stripped render output would gate a shape the reader never runs and let a
      // strip/wrap bug for the s-expr surface slip past (the scaffolding-bug class this arc keeps hitting).
      const sexprSnippet = stripModule(render_syntax(mlProgram, "ml", "sexpr"), "sexpr");
      const sexprProgram = wrapModule(sexprSnippet, "sexpr");
      const sexprFail = await checkProgram(sexprProgram, "sexpr", ex, "s-expr toggle");
      if (sexprFail) return sexprFail;
    } catch (e) {
      return `${ex.file} [${ex.kind}] (s-expr toggle): render/strip/wrap threw — ${String(e.message || e).slice(0, 80)}`;
    }
    return null;
  }

  // 1. s-expr — the authored surface.
  const sexprProgram = ex.noWrap ? ex.snippet.trim() : wrapModule(ex.snippet, "sexpr");
  const sexprFail = await checkProgram(sexprProgram, "sexpr", ex, "s-expr");
  if (sexprFail) return sexprFail;

  // 2. ML — what the reader sees after toggling. Render the snippet to ML, then wrap + compile THAT.
  //    This catches wrap/strip round-trip bugs that only bite on the ML surface (e.g. a `;`-in-a-
  //    do-block snippet whose wrapper skipped the export). `noWrap` snippets are full modules already.
  if (!ex.noWrap) {
    try {
      const mlProgram = wrapModule(renderToMl(ex.snippet), "ml");
      const mlFail = await checkProgram(mlProgram, "ml", ex, "ML toggle");
      if (mlFail) return mlFail;
    } catch (e) {
      return `${ex.file} [${ex.kind}] (ML toggle): render/wrap threw — ${String(e.message || e).slice(0, 80)}`;
    }
  }
  return null;
}

// The whole-guide check RUN (skipped under LIB_ONLY — the per-example shred drives checkExample per case).
if (!LIB_ONLY) {
let pass = 0;
const failures = []; // real, unexpected failures — these FAIL the gate.
const stillBlocked = []; // known-blocked examples that (correctly) still fail — reported, not fatal.
const recovered = []; // blocklist entries that now PASS — the entry should be removed + the example ships.
const matchedEntries = new Set(); // blocklist entries that matched ≥1 example (to find stale ones).
for (const ex of examples) {
  // Reclaim the previous example's fresh runtime + program instances (each reserves a large V8 wasm-memory
  // guard region) so this long-lived process's virtual address space stays bounded instead of climbing into
  // the hundreds of GB. Runs under `--expose-gc` (package.json check:examples); a no-op if the flag is absent.
  if (globalThis.gc) globalThis.gc();
  const block = blockedBy(ex, blocklist);
  const fail = await checkExample(ex);
  if (block) {
    matchedEntries.add(block);
    // A known-blocked example: it's EXPECTED to fail until its owner fixes the root cause.
    if (fail) stillBlocked.push({ block, ex });
    else recovered.push({ block, ex }); // it started passing — un-block it.
    continue;
  }
  if (fail) { failures.push(fail); continue; }
  pass++;
}
// ---- the /notebook route's shipped example notebooks (src/notebook/examples.ts) ----
// A notebook example is markdown interleaved with Cadenza CODE CELLS. Each non-widget code cell is
// compiled the way the live route compiles it: `assembleForRun` builds its (buffer, entry) — widget
// bindings (at their DEFAULTS) + prior cells' defs + this cell's def-block in the buffer, entry a call —
// and `repl_eval(buffer, entry, "sexpr", exact=true)` compiles it in the notebook's EXACT surface. The
// examples.test.ts unit gate only PARSES cells (well-formed, defines main), so a cell that parses but
// FAILS TO COMPILE (an inference gap, a bad annotation, a `do`-wrapped pragma) ships broken and only
// shows up when a reader opens the route. This closes that gap: every notebook example cell must compile.
// (Runs are browser-only via jco; compiling is the check that catches the "compiles-but-crashes" class.)
let notebookPass = 0;
try {
  const { EXAMPLES: NOTEBOOK } = await import(join(guideRoot, "src/notebook/examples.ts"));
  const { parseDocument, renderDocToSurface } = await import(join(guideRoot, "src/notebook/parseDocument.ts"));
  const { parseWidgets } = await import(join(guideRoot, "src/notebook/parseWidgets.ts"));
  const { assembleForRun } = await import(join(guideRoot, "src/notebook/assembleForRun.ts"));
  // Compile every non-widget code cell of `md` (its authored surface = s-expr) the way the live route does —
  // returns the count of cells that compiled, pushing a failure per decline. Reused for the authored doc AND
  // the surface-round-tripped doc (so a toggle regression is caught, not just the authored form).
  const compileCells = (md, label) => {
    const cells = parseDocument(md);
    const widgets = cells
      .filter((c) => c.kind === "code" && c.directive.kind === "widget")
      .flatMap((c) => parseWidgets(c.source).widgets);
    let ok = 0;
    cells.forEach((c, i) => {
      if (c.kind !== "code" || c.directive.kind === "widget") return;
      const { buffer, entry } = assembleForRun(cells, i, widgets, {}, "sexpr");
      let r;
      try {
        r = repl_eval(buffer, entry, "sexpr", true); // exact=true — the notebook's NOTEBOOK_EXACT mode
      } catch (e) {
        failures.push(`src/notebook/examples.ts [notebook] (${label} cell ${i}): parse error — ${String(e && e.message ? e.message : e).slice(0, 80)}`);
        return;
      }
      if (!r.component) {
        const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
        failures.push(`src/notebook/examples.ts [notebook] (${label} cell ${i}): compile DECLINED — ${d ? `${d.code ?? ""} ${d.message ?? ""}`.trim() : "no component"}`);
        return;
      }
      ok++;
    });
    return ok;
  };
  // RESULT-TYPE guard: compiling a cell catches "won't compile", but NOT "compiles to the wrong VALUE" — the
  // exact class the operator hit (a formula cell doing Int64/Int64 integer division, 3/4 → 0, instead of an
  // exact Rational; both compile). So for a cell that signals an exact-fraction intent (`Rational.of`), assert
  // its solved result type is `Rational` via export_types — so `Rational.of` silently ceasing to be Rational
  // (a prelude/inference change) fails here. Targeted (only cells using Rational.of), no false positives on
  // scalar/quantity cells. ⚠ SCOPE: this pins the Rational.of MECHANISM stays Rational; it does NOT catch a
  // full re-edit back to bare `(/ num den)` (that removes the marker) — the author owns the choice of op, and
  // examples.test's per-example assertions + this type-pin together guard the shipped exact-fraction intent.
  const assertCellType = (md, label) => {
    const cells = parseDocument(md);
    const widgets = cells
      .filter((c) => c.kind === "code" && c.directive.kind === "widget")
      .flatMap((c) => parseWidgets(c.source).widgets);
    cells.forEach((c, i) => {
      if (c.kind !== "code" || c.directive.kind === "widget") return;
      if (!/\bRational\.of\b/.test(c.source)) return; // only cells asserting an exact-fraction intent
      const { buffer } = assembleForRun(cells, i, widgets, {}, "sexpr");
      let types;
      try {
        types = export_types(`${buffer}\n(export main)`, "sexpr");
      } catch {
        return; // a compile decline is already reported by compileCells; don't double-count
      }
      const mainType = (types.split("\n").find((l) => l.startsWith("main\t")) ?? "").split("\t")[1] ?? "";
      if (mainType !== "Rational") {
        failures.push(
          `src/notebook/examples.ts [notebook] (${label} cell ${i}): a Rational.of cell must yield Rational (exact fraction), got \`${mainType}\` — integer division (Int64/Int64) regressed?`,
        );
      }
    });
  };
  const render = async (t, f, to) => render_syntax(t, f, to);
  for (const ex of NOTEBOOK) {
    // 1) Authored (s-expr) cells must compile.
    notebookPass += compileCells(ex.markdown, ex.slug);
    // 1b) RESULT-TYPE: a Rational.of cell must actually be Rational-typed (guards the operator's int-division bug).
    assertCellType(ex.markdown, ex.slug);
    // 2) SURFACE ROUND-TRIP: the reader can toggle ML↔s-expr, which re-renders the doc through
    // `renderDocToSurface`. A regression in that helper or `render_syntax` (e.g. dropping a `main`, mangling a
    // multi-form `(do …)`, mis-handling list/tuple heads — all bugs this arc hit) would break the toggled doc
    // even though the authored doc compiles. Render s-expr→ML→s-expr and require every cell STILL compiles.
    try {
      const ml = await renderDocToSurface(ex.markdown, "sexpr", "ml", render);
      const back = await renderDocToSurface(ml, "ml", "sexpr", render);
      compileCells(back, `${ex.slug} [s-expr→ML→s-expr round-trip]`);
    } catch (e) {
      failures.push(`src/notebook/examples.ts [notebook] (${ex.slug} surface round-trip): ${String(e && e.message ? e.message : e).slice(0, 80)}`);
    }
  }
} catch (e) {
  console.error(`check-examples: could not load notebook examples — ${String(e && e.message ? e.message : e)}`);
  process.exit(1);
}

// A blocklist entry that matched NO example is stale — the example was renamed/removed/rewritten so the
// entry no longer identifies anything. Flag it (loud, not fatal) so the blocklist doesn't rot silently.
const staleEntries = blocklist.filter((b) => !matchedEntries.has(b));

console.log(
  `\nchecked ${examples.length} examples across ${files.length} files (both surfaces): ` +
    `${pass} ok, ${failures.length} failed, ${stillBlocked.length} known-blocked, ${recovered.length} recovered` +
    ` · notebook cells compiled: ${notebookPass}`,
);

if (stillBlocked.length) {
  // Group by blocklist entry so one root cause reports once (with its example count), not N times.
  const byEntry = new Map();
  for (const { block } of stillBlocked) byEntry.set(block, (byEntry.get(block) ?? 0) + 1);
  console.log("\nKNOWN-BLOCKED (routed to their owner; kept OUT of the shipped guide until green):");
  for (const [block, n] of byEntry) {
    console.log(
      `  ⏸ ${block.file} (${n} example${n > 1 ? "s" : ""}) — ${block.kind} bug, owner ${block.owner}: ${block.reason}`,
    );
  }
}

if (recovered.length) {
  // A blocked example started passing — the root cause landed. Tell the operator to un-block it.
  // This is NOT fatal (the fix is good news), but it's LOUD so the blocklist doesn't rot.
  console.log(
    "\n✅ BLOCKLIST ENTRY CAN BE REMOVED (these examples now RUN — delete their blocklist entry so they ship):\n" +
      recovered
        .map(({ block, ex }) => `  ✔ ${block.file} [${ex.kind}] "${block.match}" — was: ${block.reason}`)
        .join("\n"),
  );
}

if (staleEntries.length) {
  // A blocklist entry that identifies no current example — the example was rewritten/renamed/removed.
  // Loud so the entry gets deleted; NOT fatal (a stale block is harmless, just clutter).
  console.log(
    "\n⚠️  STALE BLOCKLIST ENTRY (matches no current example — delete it from example-blocklist.json):\n" +
      staleEntries
        .map((b) => `  ⚠ ${b.file} "${JSON.stringify(b.match)}" — ${b.reason}`)
        .join("\n"),
  );
}

// ---- ATTR-ABOVE invariant (OPERATOR #16): a `@annotation` on a def renders on its OWN LINE ABOVE the def ----
// The guide displays ML source via render_syntax(_, _, "ml") (playground toggle, notebook, editor). An
// `@test`/`@tag(...)` annotation must render attr-above (the annotation line, then the def line) — the readable
// convention, NOT inline `@test def f() = 1`. This is v-syntax's ML printer behavior (convert → printer::print);
// pin it here so a future render_syntax/printer change that regressed it (back to inline-@) would fail the guide
// gate, since the guide's whole @-annotation display depends on it. Pure render check (no run), so it's stable.
for (const [src, ann] of [["@test\ndef attr_above_probe() = 1", "@test"], ['@tag("slow")\ndef attr_above_tag_probe() = 2', '@tag("slow")']]) {
  let rendered;
  try {
    rendered = render_syntax(src, "ml", "ml");
  } catch (e) {
    failures.push(`[attr-above invariant] render_syntax threw on \`${ann}\`: ${String(e.message || e).slice(0, 80)}`);
    continue;
  }
  const lines = rendered.split("\n").map((l) => l.trimEnd());
  const annIdx = lines.findIndex((l) => l.trim() === ann);
  const defIdx = lines.findIndex((l) => l.trim().startsWith("def "));
  // The annotation must be on its OWN line (exactly `@…`, nothing after it) and IMMEDIATELY above the def.
  if (annIdx < 0 || defIdx < 0 || annIdx >= defIdx) {
    failures.push(
      `[attr-above invariant] \`${ann}\` did NOT render attr-above (OPERATOR #16) — expected the annotation on its ` +
        `own line above \`def\`, got:\n      ${rendered.replace(/\n/g, "\n      ")}`,
    );
  }
}

// MULTI-FILE extractor self-check: exercise extractFilesProp + lowerToCompile on a known `files={[…]}` TSX
// fixture on EVERY run, so the multi-file path is gated even before a chapter authors one (else the whole
// path is dead code that could regress silently — the vacuous-pass class this suite guards). Asserts the
// authored shape extracts to the right file set AND lowers to the compile_with_preloaded args (entry as
// `text`, the rest as equal-length preload arrays). Compile itself is exercised by real chapters once
// authored; here we pin the deterministic extraction + lowering (no wasm, so no stale-store false negative).
{
  const fixture = `<Runnable
    files={[
      { name: "events",  source: \`(do (def turn (list)) (export turn))\`, surface: "sexpr" },
      { name: "reducer", source: \`(do (import "events" (turn)) (def (main) turn) (export main))\`, surface: "sexpr", entry: true },
    ]}
    expect="value"
  />`;
  try {
    const got = extractExamples(fixture, "<multi-file self-check>");
    const mf = got.find((e) => e.files);
    if (!mf) throw new Error("extractExamples did not yield a multi-file example from the fixture");
    if (mf.files.length !== 2) throw new Error(`expected 2 files, got ${mf.files.length}`);
    if (mf.files.filter((f) => f.entry).length !== 1) throw new Error("expected exactly one entry file");
    const low = lowerToCompile(mf.files);
    if (!low.ok) throw new Error(`fixture won't lower: ${low.reason}`);
    if (low.lowered.names.length !== 1 || low.lowered.names[0] !== "events")
      throw new Error(`expected preloaded names ["events"], got ${JSON.stringify(low.lowered.names)}`);
    if (!/\(def \(main\) turn\)/.test(low.lowered.text))
      throw new Error("entry (reducer) is not the lowered `text`");
    if (low.lowered.names.length !== low.lowered.sources.length || low.lowered.names.length !== low.lowered.formats.length)
      throw new Error("preload arrays are not equal length");
    // normValue ergonomics + backward-compat: a bare value, a quoted string, and the full ascription form
    // must all compare equal, so an author can pin `expected="asked-model; …"` OR `(: "…" String)`.
    const cases = [
      ['(: "asked-model; done" String)', "asked-model; done"],
      ['"asked-model; done"', "asked-model; done"],
      ["asked-model; done", "asked-model; done"],
      ["(: 3 Int64)", "3"],
      ["true", "true"], // scalar untouched
    ];
    for (const [input, want] of cases) {
      if (normValue(input) !== want) throw new Error(`normValue(${JSON.stringify(input)}) = ${JSON.stringify(normValue(input))}, want ${JSON.stringify(want)}`);
    }
    // and the three forms of the same value are mutually equal (the whole point):
    if (normValue('(: "x" String)') !== normValue('"x"') || normValue('"x"') !== normValue("x"))
      throw new Error("normValue does not unify render-form / quoted / bare for the same string");
  } catch (e) {
    failures.push(`[multi-file extractor self-check] ${String(e.message || e)}`);
  }
}

// cookTemplate self-check: pin that the extractor cooks a captured template body EXACTLY as JS would,
// so the gate keeps compiling the same string the live <Runnable> runs. The motivating case is a `#\`
// char literal (authored with a doubled backslash), which regressed to a hard CDZ0002 when the extractor
// read raw text; a future "simplify" that dropped the cooking would silently reintroduce that divergence,
// so gate it here. Each pair is [raw-body-between-backticks, cooked-value]. (String.raw keeps the fixtures
// readable: the left side is the literal source text a `.tsx` carries between its backticks.)
{
  const cook = [
    [String.raw`#\\a`, String.raw`#\a`], // doubled backslash cooks to one → a valid char literal (the fix)
    [String.raw`(< #\\a #\\z)`, String.raw`(< #\a #\z)`],
    [String.raw`b"\\x00\\xff"`, String.raw`b"\x00\xff"`], // byte-string escapes survive for the cadenza lexer
    [String.raw`\\n`, String.raw`\n`], // doubled → single backslash-n (a cadenza escape, not a JS newline)
    ["(Char.to-int c)", "(Char.to-int c)"], // backslash-free source is byte-identical
    ["\\n", "\n"], // a SINGLE backslash-n cooks to a real newline, exactly as JS would
    ["\\x41", "A"], // \xHH hex escape
    ["\\u0041", "A"], // \uHHHH escape
    ["\\u{1F600}", "\u{1F600}"], // \u{…} code-point escape
    ["\\\\", "\\"], // an escaped backslash cooks to one
    ["a\\", "a\\"], // a trailing lone backslash is preserved, not dropped
  ];
  for (const [raw, want] of cook) {
    if (cookTemplate(raw) !== want) {
      failures.push(`[cookTemplate self-check] cookTemplate(${JSON.stringify(raw)}) = ${JSON.stringify(cookTemplate(raw))}, want ${JSON.stringify(want)}`);
    }
  }
}

// unmodeled-construct guard self-check: the guide-accuracy audit must FLAG an `expect="error"` example that
// declines because a documented CONSTRUCT IS UNMODELED (unbound at the top level — the operator's @invariant
// hole, guide-editor 2026-09-02), and must NOT over-fire on a genuine semantic decline (a real CDZ code). Pin
// both directions here so a future edit can't quietly reopen the hole (a documented non-feature masked by
// expect="error") nor start rejecting legitimate teaching declines. Uses the real compiler (loaded above).
{
  const unmodeledEx = { file: "<unmodeled-guard self-check>", kind: "Runnable", snippet: "(frobnicate 1)", expect: "error" };
  const fired = await checkProgram("(do (frobnicate 1) (def (main) 1) (export main))", "sexpr", unmodeledEx, "self-check");
  if (fired == null || !/UNMODELED/.test(fired))
    failures.push(`[unmodeled-construct guard self-check] the guard must FLAG an unbound-at-top-level decline masked by expect="error", got ${JSON.stringify(fired)}`);
  const semanticEx = { file: "<unmodeled-guard self-check>", kind: "Runnable", snippet: "(+ 1 true)", expect: "error" };
  const notFired = await checkProgram("(do (def (main) (+ 1 true)) (export main))", "sexpr", semanticEx, "self-check");
  if (notFired != null)
    failures.push(`[unmodeled-construct guard self-check] a genuine semantic decline (CDZ0203 type mismatch) must PASS expect="error", but the guard flagged it: ${notFired}`);
}

if (failures.length) {
  console.error("\nFAILURES:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log(
  "✓ every guide example compiles + runs in both surfaces (graded exercises to their expected value); " +
    "known-blocked examples are tracked + routed; @annotations render attr-above (OPERATOR #16); " +
    "the multi-file <Runnable files={…}> extractor + lowering are exercised.",
);
} // end if (!LIB_ONLY) — the whole-guide check run

// ---- exports for the per-example shred (check-one-example.mjs imports these; the compiler + helpers above
// load at top-level on import, from CDZ_WASM_PKG). checkExample dispatches one example (files/isTest/surface/
// authoredIn/noWrap) to checkProgram (compile via cdz-wasm + run + grade). ----
export { checkExample, checkProgram, checkTestProgram, runComponent, wrapModule, renderToMl, stripModule };
