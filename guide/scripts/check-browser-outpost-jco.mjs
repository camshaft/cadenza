/// browser-outpost S1b (headless): prove a Cadenza REDUCER ships to the browser via jco AND actually folds a
/// message to a response value in a JS engine. Transpile the browser-outpost reducer-world guest with
/// @bytecodealliance/jco-transpile (the SAME transpiler the guide's in-tab run worker uses,
/// guide/src/runner/runWorker.ts), instantiate it with the REAL value-heap runtime bound as
/// cadenza:runtime/heap, then DRIVE its guest interface — proving a shipped Cadenza reducer runs AND folds in
/// a JS engine with no browser. This is the client-side half of the browser outpost.
///
/// Nix supplies CDZ_GUEST_WASM (the browser-outpost mkCadenzaGuest $out) and CDZ_RUNTIME_WASM (the value-heap
/// runtime component, packages.runtime $out). NFC is a JS shim (String.normalize), per check-calculator.mjs.
/// Run: `node scripts/check-browser-outpost-jco.mjs` (Node >=20.19 for jco).

import { readFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

let checks = 0;
const fail = (msg) => { console.error(`browser-outpost jco check: FAIL — ${msg}`); process.exit(1); };

/// Transpile a component to a loadable ES module in a temp dir (mirrors runWorker.ts / check-calculator.mjs).
/// async instantiation → the entry `import()`s without touching wasm; instantiate is a separate call.
async function loadComponent(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), {
    name, instantiation: "async", wasiShim: false, minify: false,
  });
  const dir = mkdtempSync(join(tmpdir(), `cdz-bo-${name}-`));
  for (const [f, b] of Object.entries(files)) {
    if (f.endsWith(".d.ts")) continue; // types only — never loaded (mirrors runWorker.ts)
    writeFileSync(join(dir, f), b);
  }
  const mod = await import(join(dir, `${name}.js`));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  return { files, mod, getCore };
}

const wasmPath = process.env.CDZ_GUEST_WASM;
if (!wasmPath) fail("CDZ_GUEST_WASM is not set (expected the browser-outpost mkCadenzaGuest $out .wasm)");
const runtimePath = process.env.CDZ_RUNTIME_WASM;
if (!runtimePath) fail("CDZ_RUNTIME_WASM is not set (expected the value-heap runtime component .wasm)");

// 1-3: transpile the guest + assert its bindings surface the reducer's WIT guest interface.
const guest = await loadComponent(readFileSync(wasmPath), "browserOutpost");
const guestFiles = Object.keys(guest.files);
if (guestFiles.length === 0) fail("jco produced no files for the guest");
checks++;
if (typeof guest.mod.instantiate !== "function") fail("guest entry module exposes no instantiate()");
checks++;
const guestText = Object.values(guest.files).map((b) => Buffer.from(b).toString("utf8")).join("\n");
if (!guestText.includes("cadenza:platform/guest")) fail("guest bindings do not reference cadenza:platform/guest");
if (!/onMessage/.test(guestText)) fail("guest bindings do not surface onMessage");
checks++;

// Instantiate the value-heap RUNTIME (with an NFC JS shim, per check-calculator.mjs) → the heap interface the
// guest imports as cadenza:runtime/heap. This is what lets the guest BUILD a response value on the heap.
const NFC = "cadenza:nfc/normalize";
const nfcShim = { nfc: (b) => new TextEncoder().encode(new TextDecoder("utf-8").decode(b).normalize("NFC")) };
const rt = await loadComponent(readFileSync(runtimePath), "heap");
const rroot = await rt.mod.instantiate(rt.getCore, { [NFC]: nfcShim });
const heapKey = Object.keys(rroot).find((k) => k.includes("heap"));
const heapIface = heapKey ? rroot[heapKey] : undefined;
if (!heapIface) fail(`runtime exposes no heap interface (root keys: ${Object.keys(rroot).join(", ")})`);

// 4: INSTANTIATE the guest with the REAL heap bound; no-op stubs satisfy the other host imports
//    (state/blobs/identity/run) that this fold path does not call. Assert the guest interface is callable.
const stubIface = new Proxy({}, { get: () => () => undefined });
// A Map-backed `state` host shim (design §5: IndexedDB in a browser; a Map here — the reducer is host-agnostic).
// jco represents option<bytes> as `Uint8Array | undefined`, so get returns the stored value or undefined (None).
const stateMap = new Map();
const kstr = (k) => Buffer.from(k).toString("latin1");
const stateShim = {
  get: (k) => stateMap.get(kstr(k)),
  put: (k, v) => { stateMap.set(kstr(k), v); },
  delete: (k) => { stateMap.delete(kstr(k)); },
};
const imports = new Proxy({}, {
  get: (_t, key) => {
    if (typeof key !== "string") return stubIface;
    if (key.startsWith("cadenza:runtime/heap")) return heapIface;
    if (key.includes("/state")) return stateShim; // the reducer-world `state` host import
    return stubIface;
  },
});
let root;
try {
  root = await guest.mod.instantiate(guest.getCore, imports);
} catch (e) {
  fail(`guest did not instantiate with the real heap runtime: ${e && e.message ? e.message : e}`);
}
const g = root["cadenza:platform/guest"] ?? root.guest;
if (!g || typeof g.onMessage !== "function") {
  fail(`instantiated root exposes no callable cadenza:platform/guest.onMessage (root keys: ${Object.keys(root).join(", ")})`);
}
checks++;

// 5: DRIVE the inert on-notification → { requests: [], outcome: continue } (a fold executes + returns a step).
const note = g.onNotification({ contract: new Uint8Array(), payload: new Uint8Array() });
if (!note || !Array.isArray(note.requests) || note.requests.length !== 0) {
  fail(`inert on-notification should emit 0 requests, got ${note && JSON.stringify(note.requests)}`);
}
if (!note.outcome || note.outcome.tag !== "continue") {
  fail(`inert on-notification should continue, got outcome tag: ${note.outcome && note.outcome.tag}`);
}
checks++;

// 6: DRIVE on-message to a full RESPONSE VALUE. An empty payload does not decode as a Request, so the
//    browser-outpost router folds to its deny(404, "not found") branch — a Close terminal whose reason is a
//    value BUILT ON THE VALUE HEAP. This proves the reducer folds a message to a real response value in a JS
//    engine using the runtime, the client-side capstone. The canonical-encoded reason carries the ASCII
//    "not found" (a Bytes leaf), so a substring on the reason bytes pins the fold's actual output.
const empty = new Uint8Array();
const msg = { contract: empty, sender: { reducer: empty, host: empty }, payload: empty, token: empty };
let step;
try {
  step = g.onMessage(msg);
} catch (e) {
  fail(`driving on-message threw: ${e && e.message ? e.message : e}`);
}
if (!step || !step.outcome) fail(`on-message returned no step: ${JSON.stringify(step)}`);
if (step.outcome.tag !== "close") {
  fail(`on-message on an undecodable payload should Close (deny), got outcome tag: ${step.outcome.tag}`);
}
const reason = step.outcome.val && step.outcome.val.reason;
if (!reason) fail("close outcome carries no reason payload");
const reasonText = Buffer.from(reason).toString("latin1");
if (!reasonText.includes("not found")) {
  fail(`close reason does not contain the deny text "not found" (first bytes: ${JSON.stringify(reasonText.slice(0, 80))})`);
}
checks++;

// 7: DOM-AS-EFFECT (design §3.1). Drive the browser-outpost APP reducer's on-message and assert it EMITS a
//    render EFFECT carrying a vDOM patch VALUE — Elm architecture: a pure fold emitting a render REQUEST, not
//    an imperative DOM call. The app is instantiated with the same real heap; its on-message returns
//    Continue + exactly one render request whose payload is Value.encode(Patch.Text("hello from a cadenza
//    reducer")). This is the headless half of S2 (the reducer EMITS the effect); the browser host that APPLIES
//    the patch is the browser-driver follow-on.
const appPath = process.env.CDZ_APP_WASM;
if (!appPath) fail("CDZ_APP_WASM is not set (expected the browser-outpost-app mkCadenzaGuest $out .wasm)");
const app = await loadComponent(readFileSync(appPath), "browserOutpostApp");
let appRoot;
try {
  appRoot = await app.mod.instantiate(app.getCore, imports);
} catch (e) {
  fail(`app reducer did not instantiate with the real heap: ${e && e.message ? e.message : e}`);
}
const ag = appRoot["cadenza:platform/guest"] ?? appRoot.guest;
if (!ag || typeof ag.onMessage !== "function") fail("app reducer exposes no callable onMessage");

// Assert a step emits exactly one render EFFECT (Continue + a request on the dom.render contract) whose patch
// payload carries `want`.
const assertRender = (step, want, label) => {
  if (!step || !Array.isArray(step.requests) || step.requests.length !== 1) {
    fail(`${label}: expected exactly 1 render request, got ${step && JSON.stringify(step.requests)}`);
  }
  if (!step.outcome || step.outcome.tag !== "continue") {
    fail(`${label}: should Continue after emitting the render effect, got outcome tag: ${step.outcome && step.outcome.tag}`);
  }
  const r = step.requests[0];
  if (!Buffer.from(r.contract).toString("latin1").includes("cadenza.dom.render")) {
    fail(`${label}: the emitted request is not a render effect (contract: ${JSON.stringify(Buffer.from(r.contract).toString("latin1").slice(0, 40))})`);
  }
  if (!Buffer.from(r.payload).toString("latin1").includes(want)) {
    fail(`${label}: the render patch does not carry ${JSON.stringify(want)} (first bytes: ${JSON.stringify(Buffer.from(r.payload).toString("latin1").slice(0, 80))})`);
  }
};

// 7: DOM-as-effect OUTBOUND (§3.1) — a message with no dom-event contract emits the INITIAL render patch.
assertRender(ag.onMessage(msg), "hello from a cadenza reducer", "initial render");
checks++;

// 8 + 9: DOM-as-effect INBOUND + STATEFUL fold (§3.2 / §5). A CLICK message (on the dom-event contract) folds
//    to a re-render, and STATE CARRIES ACROSS events via the state host: the FIRST click renders "clicked
//    once" (state miss → records the visit); a SECOND click sees the recorded state and renders "clicked
//    again". This proves the reducer receives DOM events as ordinary on-message calls AND maintains state
//    across them — the full stateful Elm event→view loop, driven headlessly (synthetic clicks, state = a Map).
const clickMsg = {
  contract: new TextEncoder().encode("cadenza.dom.event.click"),
  sender: { reducer: empty, host: empty },
  payload: empty,
  token: empty,
};
assertRender(ag.onMessage(clickMsg), "clicked once", "first click (state miss)");
checks++;
assertRender(ag.onMessage(clickMsg), "clicked again", "second click (state hit — carried across events)");
checks++;

// 10: AGENT-DRIVE (§7). An AGENT command (a message on the app's drive contract, NOT a user click) drives the
//     app over the SAME on-message fold: a reset clears the shared state and re-renders "reset by agent".
//     Agent-drive is the federation default — the operator's "agents can drive it directly." To the reducer an
//     agent command and a user click differ only by contract.
const agentReset = {
  contract: new TextEncoder().encode("cadenza.agent.reset"),
  sender: { reducer: empty, host: empty },
  payload: empty,
  token: empty,
};
assertRender(ag.onMessage(agentReset), "reset by agent", "agent reset command");
checks++;
// 11: the agent command shares STATE with user events — after the agent reset, a click is a state MISS again
//     ("clicked once"), proving the agent drove the same fold + the same state a user click does.
assertRender(ag.onMessage(clickMsg), "clicked once", "click after agent reset (shared state cleared)");
checks++;

if (checks !== 11) fail(`expected 11 assertions to run, ran ${checks} (vacuous-pass guard)`);
console.log(
  `browser-outpost jco check: ok — the reducer transpiles (${guestFiles.length} files), INSTANTIATES with the ` +
  `real value-heap runtime, DRIVES on-message to a response value, runs the full STATEFUL DOM-as-effect loop ` +
  `(render + click fold + state across clicks), AND is AGENT-DRIVABLE (an agent reset command drives the same ` +
  `fold + shared state as a user click) — all in a JS engine, no browser. A Cadenza reducer is a stateful, ` +
  `user- and agent-drivable event→view app.`,
);
process.exit(0);
