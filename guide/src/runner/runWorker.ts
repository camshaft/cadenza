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
  /** SCALAR-param property `@test`s to drive (from `param_test_signatures` with `compound: false`): the
   *  export takes its params as function args, so the driver generates a value per `paramTypes` entry and
   *  calls `fn(...args)` over `trials` trials, shrinking a failing input. A `compound: true` (`-gen`
   *  wrapper) test is NOT here — the client still defers it (phase 2). */
  scalarProps?: { name: string; paramTypes: string[] }[];
  /** Property-test trial count (default 100) and base seed (default 0), mirroring `cdz test`. */
  trials?: number;
  seed?: number;
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
  /** For a SCALAR property test: how many trials ran (a pass shows "(N trials)"). */
  trials?: number;
  /** For a FAILED scalar property test: the shrunk failing arguments (rendered) + the replay seed. */
  counterexample?: { args: string; seed: number };
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
  // Drive the SCALAR-param property tests (their params are on the export → generated call-args). Compound
  // `-gen` tests are not in `scalarProps` (the client defers them), so this only adds real property runs.
  const propResults = await runScalarProperties(job);
  return { kind: "tests", results: [...results, ...propResults] };
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

/// A property test's default trial count, mirroring `cdz test`.
const DEFAULT_TRIALS = 100;

/// The inclusive value range for each scalar `paramType` enum (from `param_test_signatures`). Signed widths
/// are [-2^(n-1), 2^(n-1)-1]; unsigned are [0, 2^n-1]; `bool` is 0/1 (mapped to a boolean); float widths
/// generate an integer-valued magnitude (never NaN, matching the compiler's `float-of-int` generator). A
/// type not here (`"other"`, a compound) is never a scalar prop (the client routes those to the deferred
/// phase-2 path), so it is not represented.
function intRange(t: string): { min: bigint; max: bigint } | null {
  switch (t) {
    case "int8": return { min: -128n, max: 127n };
    case "int16": return { min: -32768n, max: 32767n };
    case "int32": return { min: -2147483648n, max: 2147483647n };
    case "int64": return { min: -9223372036854775808n, max: 9223372036854775807n };
    case "uint8": return { min: 0n, max: 255n };
    case "uint16": return { min: 0n, max: 65535n };
    case "uint32": return { min: 0n, max: 4294967295n };
    case "uint64": return { min: 0n, max: 18446744073709551615n };
    default: return null;
  }
}

/// A seeded 64-bit LCG (the same MMIX constants the corpus generators use), stepping a `bigint` state and
/// yielding the low 64 bits. Deterministic from the seed → a failing trial is replayable, and the whole
/// driver is reproducible (property-based-testing.md #Generation Is Seeded And Reproducible).
function lcgStep(state: bigint): bigint {
  const M = 0xffffffffffffffffn;
  return (state * 6364136223846793005n + 1442695040888963407n) & M;
}

/// Generate one JS argument for a scalar `paramType` from the pool state, returning the arg and the advanced
/// state. jco lowers every Cadenza int width to a JS `bigint` at the boundary, `Bool` to `boolean`, and a
/// float to `number` — so an int arg is a `bigint` in its width's range, a bool is the state's low bit, and
/// a float is an integer-valued `number` (never NaN). The generated int is folded into the width range by
/// modulo (a uniform-enough draw for property sampling; the shrinker drives it toward the minimal failure).
function genArg(type: string, state: bigint): { arg: unknown; state: bigint } {
  const next = lcgStep(state);
  if (type === "bool") return { arg: (next & 1n) === 0n, state: next };
  if (type === "float32" || type === "float64") {
    // An integer-valued float in a modest range (matches `Float64.of-int` — total, never NaN).
    return { arg: Number(next % 2048n) - 1024, state: next };
  }
  const range = intRange(type);
  if (!range) return { arg: 0n, state: next }; // unreachable for a scalar prop (client filters "other")
  const span = range.max - range.min + 1n;
  return { arg: range.min + ((next % span) + span) % span, state: next };
}

/// Build one trial's argument vector for a scalar property test from a base pool state (one arg per param
/// type, threading the LCG state). Returns the args + the final state (unused per-trial — each trial reseeds).
function genArgs(paramTypes: string[], seed: bigint): unknown[] {
  let state = seed;
  const args: unknown[] = [];
  for (const t of paramTypes) {
    const { arg, state: s } = genArg(t, state);
    args.push(arg);
    state = s;
  }
  return args;
}

/// Render a trial's args for a counterexample message (a `bigint` prints without the JS `n` suffix so it
/// reads like a Cadenza literal).
function renderArgs(name: string, args: unknown[]): string {
  return `${name}(${args.map((a) => (typeof a === "bigint" ? a.toString() : String(a))).join(", ")})`;
}

/// Drive one SCALAR-param property test: call the export with generated args over `trials` trials, seeded
/// from `seed` (each trial reseeds `seed + trialIndex`, mirroring `cdz test`'s per-trial pool). A trial that
/// THROWS (a trap — assertion failure or a body trap) is a failure; the first failing arg vector is SHRUNK
/// (each numeric arg halved toward 0 while the failure persists) to a minimal counterexample, reported with
/// the base seed to replay. All-pass → `pass` with the trial count.
async function runScalarProperty(
  fn: (...a: unknown[]) => unknown,
  name: string,
  paramTypes: string[],
  trials: number,
  seed: number,
): Promise<TestResult> {
  const runArgs = async (args: unknown[]): Promise<boolean> => {
    // true = the trial FAILED (threw/trapped); false = it passed (returned).
    try {
      await (fn as (...a: unknown[]) => unknown)(...args);
      return false;
    } catch {
      return true;
    }
  };
  let failing: unknown[] | null = null;
  for (let t = 0; t < trials; t++) {
    const args = genArgs(paramTypes, BigInt(seed) + BigInt(t) + 1n);
    if (await runArgs(args)) {
      failing = args;
      break;
    }
  }
  if (!failing) return { name, pass: true, trials };
  // SHRINK: halve each numeric arg toward 0 while the failure persists (greedy per-slot, mirroring the
  // native `shrink_pool`). A boolean arg does not shrink (both values are minimal).
  const best = failing.slice();
  for (let i = 0; i < best.length; i++) {
    let v = best[i];
    while (typeof v === "bigint" && v !== 0n) {
      const cand = best.slice();
      cand[i] = v / 2n;
      if (await runArgs(cand)) {
        best[i] = cand[i];
        v = cand[i] as bigint;
      } else break;
    }
    while (typeof v === "number" && v !== 0) {
      const cand = best.slice();
      cand[i] = Math.trunc(v / 2);
      if (await runArgs(cand)) {
        best[i] = cand[i];
        v = cand[i] as number;
      } else break;
    }
  }
  return {
    name,
    pass: false,
    error: "property failed",
    counterexample: { args: renderArgs(name, best), seed },
  };
}

/// Run each SCALAR-param property test (from `param_test_signatures`, `compound: false`) over generated
/// inputs — the browser equivalent of `cdz test`'s property driver. Compound (`-gen`) tests are not here
/// (the client still defers them).
async function runScalarProperties(job: RunJob): Promise<TestResult[]> {
  const props = job.scalarProps ?? [];
  if (props.length === 0) return [];
  const root = await instantiateComponent(job);
  const exportsByNorm = new Map<string, (...a: unknown[]) => unknown>();
  for (const { name, fn } of exportedFunctions(root)) exportsByNorm.set(normalizeName(name), fn);
  const trials = job.trials ?? DEFAULT_TRIALS;
  const seed = job.seed ?? 0;
  const results: TestResult[] = [];
  for (const { name, paramTypes } of props) {
    const fn = exportsByNorm.get(normalizeName(name));
    if (typeof fn !== "function") {
      results.push({ name, pass: false, error: "property test export not found" });
      continue;
    }
    results.push(await runScalarProperty(fn, name, paramTypes, trials, seed));
  }
  return results;
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
