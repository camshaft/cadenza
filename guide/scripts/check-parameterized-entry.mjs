#!/usr/bin/env node
/// PARAMETERIZED-ENTRY host-boundary contract guard — the browser-side regression guard for the
/// operator-reported playground bug: "any program with an ARGUMENT fails / the result is coerced to a
/// BigInt", e.g. `def main(a: Int64) = (a, a); export { main }`.
///
/// ROOT CAUSE: a compiled program surfaces a COMPOUND result via the resource-escape interface
/// `cadenza:run/run` with `make()` + `encode()`. The maker mirrors the ENTRY POINT's ARITY — a nullary
/// `def main() = …` yields a nullary `make()`, but a PARAMETERIZED `def main(a: Int64) = …` yields
/// `make(a)`. The run worker (`runWorker.ts`) unconditionally called `make()` with NO argument; for an
/// arity-N maker that lowered the missing i64 from `undefined` and threw "Cannot convert undefined to a
/// BigInt" — which surfaced to the operator as a mysterious failure / BigInt coercion. The fix
/// (`runEntry.ts`'s `selectRunEntry`, unit-tested in runEntry.test.ts) detects `make.length > 0` and
/// returns a helpful "call it in the REPL / give the entry no parameters" message instead of crashing.
///
/// THIS guard pins the COMPILER↔GLUE CONTRACT the fix relies on end-to-end: through the real compiler +
/// jco the argful entry MUST expose an arity-N maker (so the glue's arity guard fires) and calling it with
/// no argument MUST throw the BigInt error (so the guard is genuinely necessary); the nullary entry MUST
/// expose a nullary maker (so a normal compound program still runs). If a future compiler change altered
/// how a parameterized entry is emitted, this flips red and tells the glue owner to re-check the guard.
///
/// Run: `npm run check:parameterized-entry` (needs the staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.

import { readFileSync, mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, compile } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

const HEAP_IMPORT = "cadenza:runtime/heap";
const NFC_IMPORT = "cadenza:nfc/normalize";
const runtimePath = join(guideRoot, "src/wasm/runtime.wasm");
const nfcHostImport = {
  nfc: (bytes) => new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};

async function loadComponent(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), "pe-"));
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

/// Compile + instantiate a program and return its `cadenza:run/run` maker interface (or null if the
/// program didn't surface a compound entry). Mirrors the run worker's instantiate wiring (heap + nfc).
async function runIfaceOf(src, surface) {
  const r = compile(src, surface);
  if (!r.component) {
    const msg = r.diagnostics.map((d) => `${d.code} ${d.message}`).join("; ");
    fail(`the program unexpectedly DECLINED: ${msg.slice(0, 200)}`);
  }
  const prog = await loadComponent(r.component, "prog");
  const marker = new TextEncoder().encode(HEAP_IMPORT);
  const bytes = new Uint8Array(r.component);
  let importsRuntime = false;
  outer: for (let i = 0; i + marker.length <= bytes.length; i++) {
    for (let j = 0; j < marker.length; j++) if (bytes[i + j] !== marker[j]) continue outer;
    importsRuntime = true;
    break;
  }
  const heap = importsRuntime ? await getHeap() : null;
  const root = await prog.instantiate(prog.getCore, heap ? { [HEAP_IMPORT]: heap } : {});
  return root["cadenza:run/run"] ?? root["run"] ?? null;
}

function fail(msg) {
  console.error(`\n✗ check:parameterized-entry: ${msg}`);
  process.exit(1);
}

// 1. NULLARY compound entry → a nullary maker (a normal compound program Run can invoke directly).
const nullaryIface = await runIfaceOf("def main() = (1, 2)\nexport { main }", "ml");
if (!nullaryIface || typeof nullaryIface.make !== "function") fail("nullary `main` did not expose a `cadenza:run/run` maker");
if (nullaryIface.make.length !== 0) fail(`nullary \`main\` maker has arity ${nullaryIface.make.length}, expected 0`);

// 2. ARGFUL compound entry (the operator's repro) → an arity-N maker the glue must NOT call with no argument.
const argfulIface = await runIfaceOf("def main(a: Int64) = (a, a)\nexport { main }", "ml");
if (!argfulIface || typeof argfulIface.make !== "function") fail("argful `main` did not expose a `cadenza:run/run` maker");
if (argfulIface.make.length < 1) fail(`argful \`main\` maker has arity ${argfulIface.make.length}, expected ≥ 1 — the glue's arity guard would silently stop firing`);

// 3. And confirm the guard is genuinely necessary: calling the arity-N maker with no argument DOES throw the
//    "Cannot convert undefined to a BigInt" error (or similar lowering error) the fix exists to prevent.
let threw = false;
try {
  argfulIface.encode(argfulIface.make());
} catch (e) {
  threw = true;
  const m = String(e && e.message ? e.message : e);
  if (!/BigInt|undefined|convert/i.test(m)) {
    console.log(`  note: argful maker threw (as expected), message differs from the classic BigInt error: ${m.slice(0, 120)}`);
  }
}
if (!threw) fail("calling the argful maker with no argument did NOT throw — the arity guard may no longer be needed; re-check runEntry.ts");

console.log("✓ check:parameterized-entry: an argful entry emits an arity-N `make()` (glue arity guard fires); nullary entry emits a nullary maker (runs directly)");
