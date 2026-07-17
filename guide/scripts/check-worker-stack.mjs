#!/usr/bin/env node
/// Worker-STACK conformance: verify that the guide's module-qualified programs COMPILE inside a
/// constrained-stack worker, the way the browser does.
///
/// WHY a separate check: the browser compiles in a Web Worker, whose JS/wasm stack is SMALLER than the
/// main thread's. A deep-but-terminating compiler recursion can therefore overflow ONLY in the worker —
/// a program that compiles cleanly on the main thread (and in `check-examples.mjs`, which runs on Node's
/// main thread) can still crash the real guide. This exact class shipped as the module-qualified-call P0
/// (`(module Temp (def (c-to-f c) …)) … (Temp.c-to-f 100)` overflowed the worker at ~1000-deep resolution
/// while compiling fine everywhere else). v-inference's `arrow_lambdas_in_progress` re-entry guard fixed
/// it (depth 1000+ → ~5); this check is the BROWSER-SIDE regression guard so a worker-stack regression is
/// caught by the gate, not by a reader hitting a crash.
///
/// HOW: compile the repro programs inside a Node `worker_thread` with a small `stackSizeMb` (mimicking the
/// browser worker's constrained stack). If the fix regresses, the worker overflows / throws and this fails.
///
/// Run: `npm run check:worker-stack` (needs the staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.

import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import { Worker } from "node:worker_threads";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
// A data-URL worker resolves imports relative to the data URL, not the filesystem — so a bare path
// import fails. Use absolute `file://` URLs for both the JS glue and the wasm binary.
const pkg = pathToFileURL(join(guideRoot, "src/wasm/pkg/cdz_wasm.js")).href;
const wasmBin = pathToFileURL(join(guideRoot, "src/wasm/pkg/cdz_wasm_bg.wasm")).href;

// The module-qualified programs that regressed the worker before v-inference's fix. Each is a whole
// `(do …)` module program compiled in s-expr — exactly the guide's Modules-chapter examples.
const PROGRAMS = [
  {
    label: "Temp.c-to-f (module-qualified call)",
    src: `(do
  (module Temp
    (def (c-to-f c) (+ (/ (* c 9) 5) 32))
    (export c-to-f))
  (def (main) (Temp.c-to-f 100))
  (export main))`,
  },
  {
    label: "Circle.area (module w/ internal constant)",
    src: `(do
  (module Circle
    (def pi 3)
    (def (area r) (* pi (* r r)))
    (export area))
  (def (main) (Circle.area 10))
  (export main))`,
  },
];

// The worker body: init the wasm, compile each program, report a component-built / diagnostics / throw
// result back. Passed as a data: URL so this stays a single self-contained file (no sibling script).
const WORKER_SRC = `
import { readFile } from "node:fs/promises";
import { parentPort, workerData } from "node:worker_threads";
import * as wasm from ${JSON.stringify(pkg)};
await wasm.default(await readFile(new URL(${JSON.stringify(wasmBin)})));
const out = [];
for (const { label, src } of workerData.programs) {
  try {
    const r = wasm.compile(src, "sexpr");
    out.push({ label, ok: !!r.component, diags: r.diagnostics.length, error: null });
  } catch (e) {
    out.push({ label, ok: false, diags: 0, error: String(e && e.message ? e.message : e).slice(0, 120) });
  }
}
parentPort.postMessage(out);
`;

/// Compile the programs inside a worker with a constrained stack (mimics the browser Worker). `stackSizeMb`
/// of 1 is well below the main thread's default — a deep recursion that overflowed the worker before the
/// fix overflows here too, so this fails loudly on a regression.
function compileInWorker(programs) {
  return new Promise((resolve, reject) => {
    const w = new Worker(new URL(`data:text/javascript,${encodeURIComponent(WORKER_SRC)}`), {
      workerData: { programs },
      resourceLimits: { stackSizeMb: 1 },
    });
    w.once("message", (m) => {
      w.terminate();
      resolve(m);
    });
    w.once("error", (e) => reject(e));
  });
}

let results;
try {
  results = await compileInWorker(PROGRAMS);
} catch (e) {
  console.error(
    `\n✗ worker-stack conformance FAILED — the compile worker itself errored (a stack overflow is exactly this):\n  ${String(e.message || e).slice(0, 160)}`,
  );
  process.exit(1);
}

const failures = [];
for (const r of results) {
  if (r.error) failures.push(`${r.label}: worker threw — ${r.error}`);
  else if (!r.ok) failures.push(`${r.label}: compiled to NO component (diags: ${r.diags})`);
  else console.log(`  ✓ ${r.label}: compiles in a constrained-stack worker (no overflow)`);
}

if (failures.length) {
  console.error(
    "\n✗ worker-stack conformance FAILED — a module-qualified program that must compile in the browser " +
      "worker did not (a worker-stack regression — the module-qualified P0 class):\n" +
      failures.map((f) => "  ✗ " + f).join("\n"),
  );
  process.exit(1);
}

console.log(
  "\n✓ worker-stack conformance: every module-qualified program compiles in a constrained-stack worker " +
    "(the browser-worker regime) — the module-qualified P0 stays fixed.",
);
