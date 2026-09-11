/// Build a SELF-CONTAINED static browser-outpost bundle the operator can open in a REAL browser and click.
/// (browser-outpost milestone: the real-browser end-to-end, the operator's manual test = the real-browser
/// verification the headless-driver gap was blocking.) At build time this jco-transpiles the browser-outpost
/// APP reducer + the value-heap runtime to ES modules, then writes $OUT/{*.js,*.wasm,index.html}. The operator
/// serves $OUT with any static server and opens it: the page instantiates the Cadenza reducer IN THE TAB (jco,
/// the SAME path the guide runs), drives its on-message on button clicks (DOM-as-effect), and shows the render.
///
/// Inputs (Nix): CDZ_APP_WASM (browser-outpost-app mkCadenzaGuest $out), CDZ_RUNTIME_WASM (value-heap runtime),
/// OUT (the output dir). NFC is a JS shim (String.normalize). Also NODE-SMOKES the reducer-driving here so the
/// fold logic is verified at build time; the DOM/ESM-in-browser layer is the operator's manual test.

import { readFileSync, mkdtempSync, writeFileSync, mkdirSync, copyFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

const OUT = process.env.OUT;
const appPath = process.env.CDZ_APP_WASM;
const runtimePath = process.env.CDZ_RUNTIME_WASM;
if (!OUT || !appPath || !runtimePath) {
  console.error("build-browser-outpost-bundle: OUT / CDZ_APP_WASM / CDZ_RUNTIME_WASM must be set");
  process.exit(1);
}
mkdirSync(OUT, { recursive: true });

// Transpile a component to ES-module files under $OUT/<name>/ (async instantiation, browser-safe: no wasiShim,
// no minify → runs on vendored WASM). Returns the entry module's relative URL for the browser to import().
async function emit(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = join(OUT, name);
  mkdirSync(dir, { recursive: true });
  for (const [f, b] of Object.entries(files)) {
    if (f.endsWith(".d.ts")) continue; // types only
    writeFileSync(join(dir, f), b);
  }
  return `./${name}/${name}.js`;
}

const appUrl = await emit(readFileSync(appPath), "browserOutpostApp");
const heapUrl = await emit(readFileSync(runtimePath), "heap");

// ── NODE SMOKE: verify the reducer instantiates + folds here (the same logic the browser runs), so the fold
//    is build-time-verified; only the DOM/ESM-in-browser layer is left to the operator's manual test. ──
async function loadFor(nodeName, url) {
  // For the Node smoke we re-transpile into a temp dir and import (Node import() of $OUT paths is awkward under
  // nix); this mirrors check-browser-outpost-jco.mjs. It only proves the fold; the emitted bundle is for the browser.
  const bytes = readFileSync(nodeName === "heap" ? runtimePath : appPath);
  const { files } = await transpileBytes(new Uint8Array(bytes), { name: nodeName, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), `cdz-bundle-${nodeName}-`));
  for (const [f, b] of Object.entries(files)) { if (f.endsWith(".d.ts")) continue; writeFileSync(join(dir, f), b); }
  const mod = await import(join(dir, `${nodeName}.js`));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  return { mod, getCore };
}
const nfcShim = { nfc: (b) => new TextEncoder().encode(new TextDecoder("utf-8").decode(b).normalize("NFC")) };
const rt = await loadFor("heap", heapUrl);
const rroot = await rt.mod.instantiate(rt.getCore, { "cadenza:nfc/normalize": nfcShim });
const heapKey = Object.keys(rroot).find((k) => k.includes("heap"));
const stateMap = new Map();
const kstr = (k) => Buffer.from(k).toString("latin1");
const stateShim = { get: (k) => stateMap.get(kstr(k)), put: (k, v) => { stateMap.set(kstr(k), v); }, delete: (k) => { stateMap.delete(kstr(k)); } };
const imports = new Proxy({}, { get: (_t, key) => {
  if (typeof key !== "string") return new Proxy({}, { get: () => () => undefined });
  if (key.startsWith("cadenza:runtime/heap")) return rroot[heapKey];
  if (key.includes("/state")) return stateShim;
  return new Proxy({}, { get: () => () => undefined });
} });
const app = await loadFor("app", appUrl);
const aroot = await app.mod.instantiate(app.getCore, imports);
const g = aroot["cadenza:platform/guest"] ?? aroot.guest;
const empty = new Uint8Array();
const clickMsg = { contract: new TextEncoder().encode("cadenza.dom.event.click"), sender: { reducer: empty, host: empty }, payload: empty, token: empty };
const s1 = g.onMessage(clickMsg);
const s1text = Buffer.from(s1.requests[0].payload).toString("latin1");
if (!s1text.includes("clicked once")) { console.error(`bundle smoke FAIL: first click did not render "clicked once" (${s1text.slice(0,60)})`); process.exit(1); }
console.log("build-browser-outpost-bundle: node smoke ok — reducer instantiates + folds a click to a render patch.");

// ── The browser page: a minimal real bootstrap that instantiates the reducer in the tab and drives it on
//    clicks. The render patch is Value.encode(Patch.Text(...)); the thin applier extracts the human text from
//    the patch bytes (longest printable run that is not a type name) — a demo simplification; a proper
//    value-render component is the follow-on. Kept intentionally minimal per the operator's "minimize JS." ──
const indexHtml = `<!doctype html>
<html lang='en'>
<head><meta charset='utf-8'><title>Cadenza Browser Outpost</title>
<style>body{font:16px system-ui;margin:3rem;max-width:40rem}#app{font-size:1.5rem;margin:1rem 0}button{font-size:1rem;margin-right:.5rem}</style></head>
<body>
<h1>Cadenza browser outpost</h1>
<p>A Cadenza reducer, shipped to this tab as a wasm component and driven as a pure fold. Clicking drives its
on-message (DOM-as-effect); the display is the reducer's render patch. "Agent reset" drives the SAME fold as an
agent command would.</p>
<div id='app'>loading reducer…</div>
<button id='click'>click (drive the reducer)</button>
<button id='reset'>agent reset</button>
<pre id='log' style='color:#666;font-size:.8rem'></pre>
<script type='module'>
const log = (m) => { document.getElementById('log').textContent += m + '\\n'; };
const enc = new TextEncoder();
const mount = document.getElementById('app');
// Extract the human-readable text a Text patch carries (demo applier; a value-render component is the follow-on).
const patchText = (payload) => {
  const s = new TextDecoder('latin1').decode(payload);
  const runs = s.match(/[\\x20-\\x7e]{3,}/g) || [];
  const skip = new Set(['Patch','Text','Element','cdzast']);
  const best = runs.filter((r) => !skip.has(r.trim())).sort((a,b)=>b.length-a.length)[0];
  return best ? best.trim() : '(empty)';
};
const apply = (step) => {
  if (step && step.requests && step.requests[0]) mount.textContent = patchText(step.requests[0].payload);
};
try {
  const NFC = { nfc: (b) => enc.encode(new TextDecoder('utf-8').decode(b).normalize('NFC')) };
  const heapMod = await import('${heapUrl}');
  // compile(arrayBuffer) not compileStreaming — the latter requires the server to send application/wasm, which
  // a plain 'python3 -m http.server' may not; this works regardless of the .wasm content-type.
  const heapGetCore = async (p) => WebAssembly.compile(await (await fetch(new URL('./heap/' + p, import.meta.url))).arrayBuffer());
  const hroot = await heapMod.instantiate(heapGetCore, { 'cadenza:nfc/normalize': NFC });
  const heapKey = Object.keys(hroot).find((k) => k.includes('heap'));
  const stateMap = new Map();
  const kstr = (k) => new TextDecoder('latin1').decode(k);
  const stateShim = { get:(k)=>stateMap.get(kstr(k)), put:(k,v)=>{stateMap.set(kstr(k),v);}, 'delete':(k)=>{stateMap.delete(kstr(k));} };
  const stub = new Proxy({}, { get: () => () => undefined });
  const imports = new Proxy({}, { get: (_t,key) => {
    if (typeof key !== 'string') return stub;
    if (key.startsWith('cadenza:runtime/heap')) return hroot[heapKey];
    if (key.includes('/state')) return stateShim;
    return stub;
  }});
  const appMod = await import('${appUrl}');
  const appGetCore = async (p) => WebAssembly.compile(await (await fetch(new URL('./browserOutpostApp/' + p, import.meta.url))).arrayBuffer());
  const aroot = await appMod.instantiate(appGetCore, imports);
  const g = aroot['cadenza:platform/guest'] ?? aroot.guest;
  const e = new Uint8Array();
  const msg = (contract) => ({ contract, sender: { reducer: e, host: e }, payload: e, token: e });
  apply(g.onMessage(msg(e)));                                   // initial render
  document.getElementById('click').onclick = () => apply(g.onMessage(msg(enc.encode('cadenza.dom.event.click'))));
  document.getElementById('reset').onclick = () => apply(g.onMessage(msg(enc.encode('cadenza.agent.reset'))));
  log('reducer instantiated + driven — click the buttons.');
} catch (err) {
  mount.textContent = 'failed to start: ' + (err && err.message ? err.message : err);
  log('ERROR ' + (err && err.stack ? err.stack : err));
}
</script>
</body>
</html>
`;
writeFileSync(join(OUT, "index.html"), indexHtml);
console.log(`build-browser-outpost-bundle: wrote bundle to ${OUT} (index.html + ${readdirSync(OUT).join(", ")}).`);
process.exit(0);
