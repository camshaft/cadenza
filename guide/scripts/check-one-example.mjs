/// PER-EXAMPLE + BATCH shred entry point (operator directive 2026-08-30: SHRED check:examples — run each
/// guide example on a FRESH cdz-wasm compiler instance; the monolithic check reused ONE instance across 409
/// calls and leaked its linear memory until an OOB, masking the per-example wasm-path bugs behind one red).
///
/// TWO MODES (concierge chose BATCH-N over strict per-example — amortizes node + 5MB-wasm-instantiate + jco
/// startup, and adds a hard per-example timeout so an infinite guest can't hang the batch):
///   • STANDALONE:  node --expose-gc check-one-example.mjs <CASE_DIR>
///       Checks ONE guideShred case dir; exit 0 = passed, 1 = failed, 2 = skipped/harness-gap.
///   • BATCH:       node --expose-gc check-one-example.mjs --batch <CASE_DIR...>
///                  node --expose-gc check-one-example.mjs --batch --cases-file <newline-list>
///       Runs a LIST of case dirs in ONE process; exit 0 IFF every runnable case passed.
///       - Each case runs in a WORKER_THREAD the supervisor can TERMINATE, giving a HARD per-example
///         wall-clock timeout (CDZ_CASE_TIMEOUT_MS, default 60000): a Promise.race can't kill a CPU-bound
///         infinite guest (it blocks the event loop), but worker.terminate() does. A timed-out case is
///         marked failed and the worker respawned.
///       - The worker is RESPAWNED every CDZ_BATCH_RELOAD_EVERY cases (default 25): a fresh worker = a fresh
///         compiler instance with empty linear memory, bounding the ~246-call accumulation OOB (the proven
///         reload-every-25 cadence). Respawn also happens on any timeout/worker-error.
///
/// env (both modes): CDZ_WASM_PKG (staged cdz-wasm pkg dir) + CADENZA_STORE + CDZ_RUNTIME_WASM (staged runtime).
///
/// SCOPE: inc-1 single-file (chapter Runnable/Exercise + playground, both surfaces via checkProgram) + inc-2
/// multi-file `(files …)` (reconstruct the ExplorerFile set → the shared checkExample→checkMultiFile). A
/// mode="test"/deferred case is SKIPPED (needs the @test-export driver, a later shred kind). All checking goes
/// through check-examples.mjs's SHARED checkProgram/checkExample, so zero drift from the monolithic gate.
import { readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { isMainThread, Worker, parentPort } from "node:worker_threads";

const args = isMainThread ? process.argv.slice(2) : [];
const isBatch = isMainThread && args[0] === "--batch";

// The cdz-wasm compiler is only needed where cases actually RUN — the worker, and standalone-main. The batch
// SUPERVISOR (isBatch) only orchestrates workers, so it skips the ~5MB compiler load. LIB_ONLY makes
// check-examples.mjs expose the checker functions without running its whole-guide loop.
let checkProgram, checkExample;
if (!isBatch) {
  process.env.CHECK_EXAMPLES_LIB_ONLY = "1";
  ({ checkProgram, checkExample } = await import("./check-examples.mjs"));
}

// ---- the WASM-lane residual blocklist (guide/example-wasm-blocklist.json) ----
// Cases that fail-wasm/pass-native (owner v-memory-safety). The MAIN thread (standalone verdict + batch
// supervisor) re-labels a blocked case's fail as "wasm-blocked" (tracked, NON-failing) and a blocked case that
// now PASSES as "recovered" (surfaced so the entry is removed) — mirroring how the native check-examples.mjs
// consumes example-blocklist.json, and enabling the RE-RUN/recovery loop (a case filtered out upstream is
// never run, so a v-memory-safety fix is never detected; running blocked cases + re-labeling detects it). The
// worker doesn't need it — it just runs + returns pass/fail; main interprets. Matched by the case dir's
// basename (the blocklist stores `dir` as the <NNNN>-slug; the harness receives a full/relative path).
const wasmBlockedDirs = (() => {
  if (!isMainThread) return new Set();
  try {
    const guideRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
    const raw = JSON.parse(readFileSync(join(guideRoot, "example-wasm-blocklist.json"), "utf8"));
    return new Set((raw.wasmBlocked ?? []).map((e) => e.dir));
  } catch {
    return new Set(); // absent/unreadable blocklist ⇒ nothing blocked (fail-open; the file is optional)
  }
})();
const baseName = (p) => String(p).replace(/\/+$/, "").split("/").pop();
// Re-interpret a raw verdict against the wasm blocklist: a blocked case's fail/timeout → "wasm-blocked"
// (tracked, not a gate failure); a blocked case that PASSES → "recovered" (loud, entry removable, not a
// failure); everything else unchanged. Non-blocked verdicts pass through untouched.
function applyWasmBlocklist(caseDir, v) {
  if (!wasmBlockedDirs.has(baseName(caseDir))) return v;
  if (v.status === "pass") return { status: "recovered", detail: `WASM-BLOCKLIST ENTRY CAN BE REMOVED (now passes in wasm) — ${v.detail}` };
  if (v.status === "fail" || v.status === "timeout") return { status: "wasm-blocked", detail: `known wasm-residual (example-wasm-blocklist.json; owner v-memory-safety) — ${v.detail}` };
  return v;
}

// ---- the reusable per-case check: returns a verdict {status, detail}, NEVER process.exit ----
// status: "pass" | "fail" | "skip" (deferred/test-mode) | "harness-error" (bad meta / missing artifact).
async function checkOneCase(caseDir) {
  const readIf = (name) => (existsSync(join(caseDir, name)) ? readFileSync(join(caseDir, name), "utf8") : null);
  let meta;
  try {
    meta = JSON.parse(readFileSync(join(caseDir, "meta.json"), "utf8"));
  } catch (e) {
    return { status: "harness-error", detail: `unreadable meta.json — ${String(e.message || e)}` };
  }
  // A DEFERRED case (mode="test" runnable) carries NO program — the shred defers it to the @test-export
  // driver (a later shred kind). Skip (not fail) so the flake routes it rather than greening an unchecked case.
  if (meta.deferred || meta.kind === "test-mode") {
    return { status: "skip", detail: `kind=${meta.kind} deferred (${meta.reason ?? "no program"})` };
  }
  const expectKind = (readIf("expect-kind") ?? "value").trim();
  const expected = readIf("expected"); // null for an ungraded Runnable

  // MULTI-FILE (inc-2): entry = program.<from>, each preloaded peer = module-<name>.<surface>; reconstruct the
  // ExplorerFile set + run the SHARED checkExample→checkMultiFile (lowerToCompile + compile_with_preloaded +
  // run + grade). Single surface, no ml toggle.
  if (meta.multiFile || meta.kind === "multi-file") {
    const from = (Array.isArray(meta.surfaces) && meta.surfaces[0]) || "sexpr";
    const peers = Array.isArray(meta.peers) ? meta.peers : [];
    const entrySource = readIf(`program.${from}`);
    if (entrySource == null) return { status: "harness-error", detail: `multi-file case missing program.${from}` };
    // The shred drops the entry file's authored name (keeping only entryName); lowerToCompile uses it only for
    // its one-entry/unique-name checks (the entry is compiled as `text`), so synthesize a name unique vs peers.
    const peerNames = new Set(peers.map((p) => p.name));
    let entryName = meta.entryName || "main";
    while (peerNames.has(entryName)) entryName += "_";
    const files = [{ name: entryName, source: entrySource, surface: from, entry: true }];
    for (const p of peers) {
      const src = readIf(`module-${p.name}.${p.surface}`);
      if (src == null) return { status: "harness-error", detail: `multi-file peer module-${p.name}.${p.surface} missing` };
      files.push({ name: p.name, source: src, surface: p.surface, entry: false });
    }
    const ex = { file: meta.file ?? caseDir, files, expect: expectKind === "error" ? "error" : undefined, expected };
    const fail = await checkExample(ex);
    return fail ? { status: "fail", detail: fail } : { status: "pass", detail: `multi-file (${from}; ${peers.length} peer(s))` };
  }

  // SINGLE-FILE (inc-1): check every present surface (guideShred pre-renders both); checkProgram grades the
  // `expected` scalar only on the s-expr pass (the ML pass guards the wrap/render round-trip).
  const surfaces = Array.isArray(meta.surfaces) && meta.surfaces.length ? meta.surfaces : ["sexpr", "ml"];
  for (const surface of surfaces) {
    const program = readIf(`program.${surface}`);
    if (program == null) continue; // surface not emitted for this case
    const ex = {
      snippet: program,
      file: meta.file ?? caseDir,
      kind: meta.kind ?? "Runnable",
      expect: expectKind === "error" ? "error" : undefined,
      expected,
    };
    const fail = await checkProgram(program, surface, ex, surface === "ml" ? "ML" : "s-expr");
    if (fail) return { status: "fail", detail: fail };
  }
  return { status: "pass", detail: `${meta.kind ?? "?"} (${surfaces.join("+")})` };
}

// ---- WORKER: compiler loaded at module top; check one case per message, post the verdict ----
if (!isMainThread) {
  parentPort.on("message", async (caseDir) => {
    let v;
    try {
      v = await checkOneCase(caseDir);
    } catch (e) {
      v = { status: "harness-error", detail: `worker threw: ${String(e && e.message ? e.message : e)}` };
    }
    parentPort.postMessage(v);
  });
  // Signal readiness AFTER the top-level compiler load (the import above) completed.
  parentPort.postMessage({ ready: true });
} else if (isBatch) {
  // ---- BATCH SUPERVISOR ----
  const TIMEOUT_MS = Number(process.env.CDZ_CASE_TIMEOUT_MS) || 60000;
  const RELOAD_EVERY = Number(process.env.CDZ_BATCH_RELOAD_EVERY) || 25;
  const SPAWN_TIMEOUT_MS = Number(process.env.CDZ_BATCH_SPAWN_TIMEOUT_MS) || 180000; // worker + compiler load

  // Case list: `--batch <dir...>` or `--batch --cases-file <file>` (one dir per line, blanks/#comments skipped).
  let cases;
  if (args[1] === "--cases-file") {
    const listPath = args[2];
    if (!listPath) { console.error("usage: --batch --cases-file <file>"); process.exit(2); }
    cases = readFileSync(listPath, "utf8").split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
  } else {
    cases = args.slice(1);
  }
  if (cases.length === 0) { console.error("check-one-example --batch: no case dirs given"); process.exit(2); }

  const selfPath = fileURLToPath(import.meta.url);
  let worker = null;
  let workerReady = null;
  function spawnWorker() {
    // No execArgv override: the worker INHERITS the supervisor's process.execArgv (this batch is launched with
    // `node --expose-gc`), so the worker gets --expose-gc too (check-examples' per-run globalThis.gc() sweep).
    // (--expose-gc is a V8 flag, not a valid per-worker execArgv entry, so it can only be inherited.)
    worker = new Worker(selfPath);
    workerReady = new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`worker spawn/compiler-load exceeded ${SPAWN_TIMEOUT_MS}ms`)), SPAWN_TIMEOUT_MS);
      worker.once("message", (m) => {
        if (m && m.ready) { clearTimeout(t); resolve(); }
      });
      worker.once("error", (e) => { clearTimeout(t); reject(e); });
    });
  }
  async function killWorker() {
    if (worker) { const w = worker; worker = null; try { await w.terminate(); } catch {} }
  }
  // Run ONE case with a hard wall-clock timeout. On timeout the worker is dead-in-the-water (its event loop is
  // blocked by the runaway guest), so the caller TERMINATES + respawns it.
  function runCaseInWorker(caseDir) {
    return new Promise((resolve) => {
      let settled = false;
      const t = setTimeout(() => {
        if (settled) return;
        settled = true;
        resolve({ status: "timeout", detail: `exceeded ${TIMEOUT_MS}ms — worker terminated` });
      }, TIMEOUT_MS);
      const onMsg = (m) => {
        if (settled) return;
        settled = true;
        clearTimeout(t);
        worker.off("error", onErr);
        resolve(m);
      };
      const onErr = (e) => {
        if (settled) return;
        settled = true;
        clearTimeout(t);
        worker.off("message", onMsg);
        resolve({ status: "harness-error", detail: `worker error: ${String(e && e.message ? e.message : e)}` });
      };
      worker.once("message", onMsg);
      worker.once("error", onErr);
      worker.postMessage(caseDir);
    });
  }

  spawnWorker();
  let passed = 0, failed = 0, skipped = 0, wasmBlocked = 0, recovered = 0;
  const failures = [];
  const recoveredDirs = [];
  for (let i = 0; i < cases.length; i++) {
    // Reload cadence: a fresh worker (= fresh compiler) every RELOAD_EVERY cases, before the accumulation OOB.
    if (i > 0 && i % RELOAD_EVERY === 0) { await killWorker(); spawnWorker(); }
    const caseDir = cases[i];
    let v;
    try {
      await workerReady;
      v = await runCaseInWorker(caseDir);
    } catch (e) {
      v = { status: "harness-error", detail: `worker unavailable: ${String(e && e.message ? e.message : e)}` };
    }
    // Respawn on the RAW status: a timed-out / errored worker is dead (blocked or not), so replace it before
    // re-interpreting the verdict.
    if (v.status === "timeout" || v.status === "harness-error") { await killWorker(); spawnWorker(); }
    // Re-label against the wasm blocklist (blocked+fail → wasm-blocked, non-failing; blocked+pass → recovered).
    v = applyWasmBlocklist(caseDir, v);
    const glyph = v.status === "pass" ? "✓" : v.status === "skip" ? "·" : v.status === "wasm-blocked" ? "⊘" : v.status === "recovered" ? "★" : "✗";
    console.log(`  ${glyph} ${caseDir} [${v.status}]${v.detail ? " — " + String(v.detail).replace(/\n/g, " ").slice(0, 160) : ""}`);
    if (v.status === "pass") passed++;
    else if (v.status === "skip") skipped++;
    else if (v.status === "wasm-blocked") wasmBlocked++;
    else if (v.status === "recovered") { recovered++; recoveredDirs.push(baseName(caseDir)); }
    else { failed++; failures.push(`${caseDir} [${v.status}]: ${String(v.detail).replace(/\n/g, " ").slice(0, 200)}`); }
  }
  await killWorker();
  console.log(`check-one-example --batch: ${cases.length} case(s) — ${passed} passed, ${failed} failed/timed-out, ${skipped} skipped, ${wasmBlocked} wasm-blocked, ${recovered} recovered`);
  if (recovered) {
    // A previously-blocked wasm-residual now PASSES — surface it loudly so the example-wasm-blocklist.json
    // entry is removed (the case can ship). Non-fatal (recovery is good), like the native check's report.
    console.log(`  ★ WASM-BLOCKLIST ENTRIES CAN BE REMOVED (now pass in wasm): ${recoveredDirs.join(", ")} — drop them from example-wasm-blocklist.json`);
  }
  if (failed) {
    console.error("FAILURES:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
    process.exit(1);
  }
  process.exit(0);
} else {
  // ---- STANDALONE per-example (the inc-1/inc-2 contract; one case, one verdict) ----
  const caseDir = args[0];
  if (!caseDir) {
    console.error("usage: node --expose-gc check-one-example.mjs <case-dir>  |  --batch <case-dir...>");
    process.exit(2);
  }
  const v = applyWasmBlocklist(caseDir, await checkOneCase(caseDir));
  if (v.status === "pass") { console.log(`✓ ${caseDir} [${v.detail}]`); process.exit(0); }
  if (v.status === "recovered") { console.log(`★ ${caseDir} — ${v.detail}`); process.exit(0); } // now passes; entry removable (non-fatal)
  if (v.status === "wasm-blocked") { console.error(`⊘ ${caseDir} — ${v.detail}`); process.exit(2); } // known residual, tracked (not a hard fail)
  if (v.status === "skip") { console.error(`check-one-example: ${caseDir}: ${v.detail} — needs the @test-export driver (a later shred kind)`); process.exit(2); }
  if (v.status === "harness-error") { console.error(`check-one-example: ${caseDir}: ${v.detail}`); process.exit(2); }
  console.error(`✗ ${caseDir} — ${v.detail}`);
  process.exit(1);
}
