/// The RUN worker — executes a compiled Cadenza component and returns the encoded result value.
///
/// DISPOSABLE by design: the main thread starts a watchdog timer and, if a program runs past the
/// budget (an infinite loop — wasm can't be cooperatively interrupted), it `terminate()`s this
/// worker and spawns a fresh one. So this worker handles one job and posts one result.
///
/// Pipeline (mirrors the reference runner `cdz-run`, but on jco instead of native wasmtime):
///   1. jco-transpile the program component to an ES module + core wasm (in memory, browser wasm).
///   2. If the program imports the value-heap runtime, transpile + instantiate the runtime and wire
///      its `heap` interface into the program's import. jco strips the version+hash, so the import
///      key is the bare `cadenza:runtime/heap`.
///   3. Instantiate the program; take the resource-escape path `make()` -> handle, `encode(handle)`
///      -> canonical value-form bytes (rendered to text on the main thread), or, for a scalar/unit
///      program, call the sole bare function export and return its value directly.

import { transpileBytes } from "@bytecodealliance/jco-transpile";

const HEAP_IMPORT = "cadenza:runtime/heap";

export interface RunJob {
  component: Uint8Array;
  /** The value-heap runtime component bytes, or null when none is bundled (scalar-only). */
  runtime: Uint8Array | null;
}

export type RunResult =
  | { kind: "value-bytes"; bytes: Uint8Array }
  | { kind: "scalar"; value: string }
  | { kind: "trap"; message: string }
  | { kind: "error"; message: string };

interface Transpiled {
  instantiate: (
    getCoreModule: (path: string) => Promise<WebAssembly.Module>,
    imports: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>;
  getCoreModule: (path: string) => Promise<WebAssembly.Module>;
}

/// Transpile a component to in-memory files, then load its entry module from a blob URL with its
/// relative interface imports rewritten to blob URLs. Returns the module's `instantiate` plus a
/// `getCoreModule` that compiles the transpiled `.core.wasm` chunks.
async function loadComponent(bytes: Uint8Array, name: string): Promise<Transpiled> {
  const { files } = await transpileBytes(bytes, {
    name,
    instantiation: "async",
    // No minify, no optimize, no asm.js — those are the only Node-native code paths; keeping them
    // off means the transpile runs on the vendored WASM alone, which is browser-safe.
  });

  const cores = new Map<string, Uint8Array>();
  const jsUrls = new Map<string, string>();
  const decoder = new TextDecoder();
  const entryName = `${name}.js`;

  for (const [path, data] of Object.entries(files)) {
    if (path.endsWith(".wasm")) cores.set(path, data as Uint8Array);
  }
  for (const [path, data] of Object.entries(files)) {
    if (path.endsWith(".js") && path !== entryName) {
      jsUrls.set(path, blobUrl(decoder.decode(data as Uint8Array)));
    }
  }

  let entrySrc = decoder.decode(files[entryName] as Uint8Array);
  entrySrc = entrySrc.replace(/from\s+['"]\.\/(.+?)['"]/g, (m, rel: string) => {
    const url = jsUrls.get(rel);
    return url ? `from '${url}'` : m;
  });
  const entryUrl = blobUrl(entrySrc);

  const mod = (await import(/* @vite-ignore */ entryUrl)) as {
    instantiate: Transpiled["instantiate"];
  };
  const getCoreModule = async (path: string) => {
    const core = cores.get(path);
    if (!core) throw new Error(`missing core module ${path}`);
    return WebAssembly.compile(core as BufferSource);
  };
  return { instantiate: mod.instantiate, getCoreModule };
}

function blobUrl(source: string): string {
  return URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
}

async function runComponent(job: RunJob): Promise<RunResult> {
  // Prepare the runtime's heap interface if the program imports it.
  let heap: Record<string, unknown> | null = null;
  if (job.runtime) {
    const rt = await loadComponent(job.runtime, "heap");
    const root = await rt.instantiate(rt.getCoreModule, {});
    heap = (root[HEAP_IMPORT] ?? root["heap"]) as Record<string, unknown>;
  }

  const prog = await loadComponent(job.component, "prog");
  const imports = heap ? { [HEAP_IMPORT]: heap } : {};
  const root = await prog.instantiate(prog.getCoreModule, imports);

  // Compound result: the resource-escape path exposes `cadenza:run/run` with make()/encode().
  const runIface = (root["cadenza:run/run"] ?? root["run"]) as
    | { make: () => unknown; encode: (h: unknown) => Uint8Array }
    | undefined;
  if (runIface && typeof runIface.make === "function") {
    const handle = runIface.make();
    const bytes = runIface.encode(handle);
    return { kind: "value-bytes", bytes: new Uint8Array(bytes) };
  }

  // Scalar/unit result: a bare function export. Prefer a NULLARY entry (the runnable `main` shape) —
  // Run produces a value with no input, so a nullary export is what it can invoke.
  const fns = exportedFunctions(root);
  const nullary = fns.find((f) => f.fn.length === 0);
  if (nullary) return { kind: "scalar", value: String(nullary.fn()) };

  // The only runnable export takes arguments (e.g. `export { inc }` where `inc(x: Int64)`). Calling it
  // with no argument would lower `undefined` to an i64 and throw a cryptic "Cannot convert undefined to
  // a BigInt". Explain instead: Run needs a zero-argument entry; a parameterized fn is called from the
  // REPL or via a nullary `main` that applies it.
  const param = fns[0];
  if (param) {
    const n = param.fn.length;
    const args = n === 1 ? "an argument" : `${n} arguments`;
    const call = `${param.name}(${Array.from({ length: n }, () => "…").join(", ")})`;
    return {
      kind: "error",
      message:
        `\`${param.name}\` takes ${args}, so Run can't produce a value on its own. ` +
        `Call it in the REPL (e.g. \`${call}\`), or add \`def main() = ${param.name}(…)\` and export \`main\`.`,
    };
  }

  return { kind: "error", message: "component exported no runnable entry" };
}

/// The component's exported FUNCTIONS, each with its name and the JS function (whose `.length` is the
/// declared parameter count). Used to pick a nullary entry to run, or to explain a parameterized one.
function exportedFunctions(root: Record<string, unknown>): { name: string; fn: (...a: unknown[]) => unknown }[] {
  return Object.entries(root)
    .filter(([, v]) => typeof v === "function")
    .map(([name, v]) => ({ name, fn: v as (...a: unknown[]) => unknown }));
}

self.onmessage = async (e: MessageEvent<RunJob>) => {
  try {
    const result = await runComponent(e.data);
    (self as unknown as Worker).postMessage(result);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    const kind = /unreachable|trap|RuntimeError/i.test(message) ? "trap" : "error";
    (self as unknown as Worker).postMessage({ kind, message } as RunResult);
  }
};
