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
  /** When "test", the component is a test-layout build (from `compile_tests`): invoke each named `@test`
   *  export and report per-test pass/fail instead of running a single entry. Absent = normal run. */
  mode?: "test";
  /** The nullary `@test` export names to run (from `compile_tests`'s `nullary_test_names`). */
  testNames?: string[];
}

export type RunResult =
  | { kind: "value-bytes"; bytes: Uint8Array }
  | { kind: "scalar"; value: string }
  | { kind: "trap"; message: string }
  | { kind: "error"; message: string }
  | { kind: "tests"; results: TestResult[] };

/// One `@test`'s outcome. `pass` = the test export returned cleanly; `!pass` with `error` = it trapped
/// (assertion failure). `deferred` = the export performed `Test.gen` (a property test compiled to a `-gen`
/// wrapper) so a bare invoke can't run it — reported as deferred, NOT failed (property trials are a
/// follow-up that supplies the `Test.gen` host-responses).
export interface TestResult {
  name: string;
  pass: boolean;
  error?: string;
  deferred?: boolean;
}

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

/// Transpile + instantiate the program component (wiring the value-heap runtime if it imports one),
/// returning the exports root. Shared by the normal run path and the test-runner path.
async function instantiateComponent(job: RunJob): Promise<Record<string, unknown>> {
  let heap: Record<string, unknown> | null = null;
  if (job.runtime) {
    const rt = await loadComponent(job.runtime, "heap");
    const rtRoot = await rt.instantiate(rt.getCoreModule, {});
    heap = (rtRoot[HEAP_IMPORT] ?? rtRoot["heap"]) as Record<string, unknown>;
  }
  const prog = await loadComponent(job.component, "prog");
  const imports = heap ? { [HEAP_IMPORT]: heap } : {};
  return prog.instantiate(prog.getCoreModule, imports);
}

/// Run each named `@test` export and report per-test pass/fail. Contract (mirrors `cdz test`): a test
/// export that RETURNS CLEANLY passed; one that TRAPS (an assertion via `trap(msg)`) failed. Belt +
/// suspenders on top of `compile_tests`'s `-gen`-suffix classification: if invoking an export errors in a
/// way that signals an unanswered `Test.gen` host op (a property `-gen` wrapper that slipped through), it
/// is reported DEFERRED, not failed — property-trial driving is a follow-up.
/// Normalize an identifier for cross-naming-convention matching: a Cadenza source name (`one_plus_one`)
/// crosses the component boundary as a kebab WIT name (`one-plus-one`) that jco then binds in JS as
/// camelCase (`onePlusOne`) — so strip `-`/`_` and lowercase to compare source names to actual exports.
function normalizeName(n: string): string {
  return n.replace(/[-_]/g, "").toLowerCase();
}

async function runTests(job: RunJob): Promise<RunResult> {
  const root = await instantiateComponent(job);
  const names = job.testNames ?? [];
  // Map each actual function export by its normalized name, so a source test name (`one_plus_one`) finds
  // its boundary export whatever convention jco bound it under (kebab/camel).
  const exportsByNorm = new Map<string, (...a: unknown[]) => unknown>();
  for (const { name, fn } of exportedFunctions(root)) exportsByNorm.set(normalizeName(name), fn);
  const results: TestResult[] = [];
  for (const name of names) {
    const fn = exportsByNorm.get(normalizeName(name));
    if (typeof fn !== "function") {
      results.push({ name, pass: false, error: "test export not found" });
      continue;
    }
    try {
      // AWAIT the invoke: a test export may be async (returns a Promise) — invoking it synchronously and
      // marking pass:true immediately would record a FALSE PASS for an async test that later rejects
      // (its trap/assertion escaping the sync try/catch). Awaiting a thenable makes an async rejection land
      // in the catch below. A plain (non-thenable) return awaits to itself — no behavior change.
      await (fn as () => unknown)();
      results.push({ name, pass: true });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // An unanswered `Test.gen` host op means this is a property/`-gen` test a bare invoke can't run —
      // defer it (not a failure). Everything else is a genuine assertion trap = fail.
      if (/test\.gen|no enclosing handler|unhandled|host (op|function)/i.test(message)) {
        results.push({ name, pass: false, deferred: true, error: "property test — deferred (needs generated inputs)" });
      } else {
        results.push({ name, pass: false, error: message });
      }
    }
  }
  return { kind: "tests", results };
}

async function runComponent(job: RunJob): Promise<RunResult> {
  const root = await instantiateComponent(job);

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
    const result = e.data.mode === "test" ? await runTests(e.data) : await runComponent(e.data);
    (self as unknown as Worker).postMessage(result);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    const kind = /unreachable|trap|RuntimeError/i.test(message) ? "trap" : "error";
    (self as unknown as Worker).postMessage({ kind, message } as RunResult);
  }
};
