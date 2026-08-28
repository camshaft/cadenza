#!/usr/bin/env node
/// RUNTIME-collection-in-a-Tuple host-boundary guard — the browser-side regression guard for the
/// "show DATA + RESULT together" example pattern (operator's show-real-values directive): an example that
/// returns a tuple of a RUNTIME-built list + a computed scalar, e.g. `(ages-list, average)`.
///
/// BACKGROUND: a RUNTIME-built collection (a list built at runtime-determined depth) nested inside a Tuple
/// could not cross the host boundary — the value-form `encode()` walker's Tuple arm fell to a None template
/// and DECLINED ("value-form walker that loops to a runtime-determined depth … not yet emitted"). A BARE
/// runtime collection, and one nested in a SUM/variant payload, always worked; only the Tuple arm didn't.
/// v-effects fixed it (route the Tuple None-template to `sum_shape_descriptor`, the walker a bare collection
/// uses). A CONSTANT list folds at compile time and dodges the walker, so this guard uses a RUNTIME build.
///
/// SELF-ACTIVATING: until the fix is present, the program DECLINES with that specific message → this check
/// SKIPS with a clear "pending v-effects tuple-walker fix" note (NOT a failure — the fix is a compiler
/// increment out of the guide's hands). Once the fix lands, the program compiles + runs + renders → this
/// check ASSERTS the rendered value. So it flips from skip to assert automatically when the fix lands, and
/// then guards against a regression (a re-decline, or a wrong render).
///
/// Run: `npm run check:tuple-collection` (needs the staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.

import { readFileSync, mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, compile, render_value } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

const HEAP_IMPORT = "cadenza:runtime/heap";
const runtimePath = join(guideRoot, "src/wasm/runtime.wasm");
// FINDING#23: the runtime imports cadenza:nfc/normalize (separate NFC component) — supply the JS shim so it
// instantiates. NFC of well-formed UTF-8 is String.prototype.normalize('NFC') over the list<u8> boundary.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes) => new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};

async function loadComponent(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), "tc-"));
  for (const [f, b] of Object.entries(files)) {
    const p = join(dir, f);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, b);
  }
  const mod = await import(join(dir, `${name}.js`));
  return { instantiate: mod.instantiate, getCore: async (p) => WebAssembly.compile(readFileSync(join(dir, p))) };
}

let heapPromise = null;
async function getHeap() {
  if (!heapPromise) {
    heapPromise = (async () => {
      const rt = await loadComponent(readFileSync(runtimePath), "heap");
      const root = await rt.instantiate(rt.getCore, { [NFC_IMPORT]: nfcHostImport });
      return root[HEAP_IMPORT] ?? root["heap"];
    })();
  }
  return heapPromise;
}

// A RUNTIME-built list (recursion to a runtime-determined depth — NOT a foldable constant) nested in a
// tuple with a scalar: the canonical "data + result" shape the operator wants.
const BUILD = "(def (build n) (if (= n 0) (: (list) (List Int64)) (List.push (build (- n 1)) n)))";
const PROGRAM = `(do ${BUILD} (def (main) (tuple (build 3) 30)) (export main))`;
const EXPECTED = "(: #tuple(#list(1 2 3) 30) (Tuple (List Int64) Int64))";
// The specific decline emitted while the walker gap is unfixed — used to SKIP (not fail) pending the fix.
const PENDING_MARKER = "value-form walker that loops to a runtime-determined depth";

const r = compile(PROGRAM, "sexpr");
if (!r.component) {
  const msg = r.diagnostics.map((d) => `${d.code} ${d.message}`).join("; ");
  if (msg.includes(PENDING_MARKER)) {
    console.log("check:tuple-collection SKIPPED — runtime-collection-in-tuple host escape not yet emitted");
    console.log("  (pending v-effects tuple-walker fix; flips to an assertion once it lands). Decline:");
    console.log(`  ${msg.slice(0, 140)}`);
    process.exit(0);
  }
  console.error(`\n✗ check:tuple-collection: the repro DECLINED for an UNEXPECTED reason (not the known walker gap):\n  ${msg.slice(0, 200)}`);
  process.exit(1);
}

// Compiles → the fix is present. Run it through the jco host-boundary path + assert the rendered value.
let rendered;
try {
  const prog = await loadComponent(r.component, "prog");
  const heap = await getHeap();
  const root = await prog.instantiate(prog.getCore, heap ? { [HEAP_IMPORT]: heap } : {});
  const iface = root["cadenza:run/run"] ?? root["run"];
  if (!iface || typeof iface.make !== "function") {
    console.error("\n✗ check:tuple-collection: expected a compound (resource-escape) result, got none.");
    process.exit(1);
  }
  rendered = render_value(iface.encode(iface.make()));
} catch (e) {
  console.error(`\n✗ check:tuple-collection: compiled but FAILED to run/encode — ${String(e.message || e).slice(0, 160)}`);
  process.exit(1);
}

if (rendered.trim() !== EXPECTED) {
  console.error(`\n✗ check:tuple-collection: rendered value mismatch\n  got:  ${rendered}\n  want: ${EXPECTED}`);
  process.exit(1);
}
console.log(`✓ check:tuple-collection: a runtime-built list nested in a tuple crosses the host boundary + renders`);
console.log(`  ${rendered}`);
