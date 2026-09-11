/// browser-outpost S1b (headless): prove a Cadenza REDUCER ships to the browser via jco. Transpile the
/// browser-outpost reducer-world guest component (a `cadenza:platform/guest`) with @bytecodealliance/
/// jco-transpile — the SAME transpiler the guide's in-tab run worker uses (guide/src/runner/runWorker.ts) —
/// and assert it produces a loadable ES module whose transpiled bindings surface the reducer's WIT `guest`
/// interface (on-message). This is the smallest GATED proof (no browser needed) that a shipped reducer is
/// jco-loadable in a JS engine — the client-side half of the browser outpost.
///
/// The guest wasm is supplied by Nix as CDZ_GUEST_WASM (a mkCadenzaGuest $out — the stripped component file).
/// FOLLOW-UP (once the toolchain lands): instantiate with JS-shim state/blobs/identity/run + value-heap
/// runtime imports and DRIVE on-message end to end (needs the host-import shims); this slice pins the
/// transpile+load layer. Run: `node scripts/check-browser-outpost-jco.mjs` (Node >=20.19 for jco).

import { readFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const wasmPath = process.env.CDZ_GUEST_WASM;
if (!wasmPath) {
  console.error("browser-outpost jco check: CDZ_GUEST_WASM is not set (expected a mkCadenzaGuest $out .wasm)");
  process.exit(1);
}
const bytes = readFileSync(wasmPath);
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

const name = "browserOutpost";
// Same options the run worker uses (guide/src/runner/runWorker.ts:100-106): async instantiation, no WASI
// shim, no minify (keeps the native oxc-minify addon off the executed path — a browser-safe transpile).
const { files } = await transpileBytes(new Uint8Array(bytes), {
  name,
  instantiation: "async",
  wasiShim: false,
  minify: false,
});

let checks = 0;
const fail = (msg) => { console.error(`browser-outpost jco check: FAIL — ${msg}`); process.exit(1); };

// 1. jco produced a transpiled ES module (the reducer is transpilable).
const fileNames = Object.keys(files);
if (fileNames.length === 0) fail("jco produced no files");
checks++;

// 2. the entry module is present and IMPORTS cleanly under Node (valid generated JS). Async-instantiation
//    mode does not touch wasm at import time (the run worker imports the entry, then calls instantiate) — so
//    importing here proves the generated module loads without needing the host imports.
const entryJs = `${name}.js`;
if (!fileNames.includes(entryJs)) fail(`no entry module ${entryJs}; got: ${fileNames.join(", ")}`);
const dir = mkdtempSync(join(tmpdir(), "cdz-bo-jco-"));
for (const [f, b] of Object.entries(files)) {
  if (f.endsWith(".d.ts")) continue; // TypeScript decls — the run worker never writes/loads these
  writeFileSync(join(dir, f), b);
}
const mod = await import(join(dir, entryJs));
if (typeof mod.instantiate !== "function") fail("transpiled entry module exposes no instantiate() function");
checks++;

// 3. the transpiled bindings surface the reducer's WIT `guest` interface (cadenza:platform/guest) and its
//    on-message export (jco camelCases on-message -> onMessage) — i.e. jco bound the reducer contract, not
//    some other component shape. This is what makes the shipped component drivable as a reducer in JS.
const allText = Object.values(files).map((b) => Buffer.from(b).toString("utf8")).join("\n");
const hasGuest = allText.includes("cadenza:platform/guest");
const hasOnMessage = /onMessage/.test(allText) || /on-message/.test(allText);
if (!hasGuest) fail("transpiled bindings do not reference the cadenza:platform/guest interface");
if (!hasOnMessage) fail("transpiled bindings do not surface the reducer on-message export");
checks++;

// 4. INSTANTIATE the component in a JS engine and assert the reducer's guest interface surfaces as a
//    CALLABLE on-message. Supply no-op STUB imports for every host interface the component links
//    (state/blobs/identity/run + the value-heap runtime + nfc): instantiation only LINKS these — the guest
//    calls them at fold time, not at instantiation — so no-op stubs satisfy the linker without hand-writing
//    each WIT shape. This is a step up from "the module loads": it proves the reducer actually INSTANTIATES
//    in a JS engine and its on-message export is a real callable JS function (the step before driving a fold).
const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
// Nested Proxy: imports[anyInterface][anyFunc] → a no-op stub, so jco's import linking is satisfied for any
// host interface the component declares, without enumerating them.
const stubIface = new Proxy({}, { get: () => () => undefined });
const importsProxy = new Proxy({}, { get: () => stubIface });
let root;
try {
  root = await mod.instantiate(getCore, importsProxy);
} catch (e) {
  fail(`component did not instantiate with stub imports: ${e && e.message ? e.message : e}`);
}
const guest = root["cadenza:platform/guest"] ?? root.guest;
if (!guest || typeof guest.onMessage !== "function") {
  fail(`instantiated root exposes no callable cadenza:platform/guest.onMessage (root keys: ${Object.keys(root).join(", ")})`);
}
checks++;

if (checks !== 4) fail(`expected 4 assertions to run, ran ${checks} (vacuous-pass guard)`);
console.log(
  `browser-outpost jco check: ok — the reducer transpiles to a loadable ES module (${fileNames.length} files), ` +
  `INSTANTIATES in a JS engine, and exposes a callable cadenza:platform/guest.onMessage. A Cadenza reducer ` +
  `runs in a JS engine (fold-drive follow-up: real host-import shims + a message).`,
);
process.exit(0);
