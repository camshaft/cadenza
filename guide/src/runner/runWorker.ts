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
import { genArgs, renderArgs, normalizeName, GenPool } from "./genPool.ts";
import { exportedFunctions, selectRunEntry, parameterizedEntryMessage } from "./runEntry.ts";

const HEAP_IMPORT = "cadenza:runtime/heap";

// FINDING#23: the value-heap runtime imports `cadenza:nfc/normalize` — a separate component carrying the
// Unicode NFC tables (kept out of the runtime). The native/CI path composes the real cdz-nfc component; the
// browser supplies the import as a JS shim. `nfc: list<u8> -> list<u8>` crosses as (Uint8Array) => Uint8Array,
// and NFC of well-formed UTF-8 is exactly String.prototype.normalize('NFC') round-tripped through UTF-8.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes: Uint8Array): Uint8Array =>
    new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};

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
   *  wrapper) test is driven separately via `compoundProps` (below), not here. */
  scalarProps?: { name: string; paramTypes: string[] }[];
  /** COMPOUND-param property `@test`s to drive (from `param_test_signatures` with `compound: true`): the
   *  compiler synthesized a NULLARY `<name>` wrapper that builds its compound argument guest-side by consuming
   *  a seeded int stream via the `Test.gen-int` host op. The driver instantiates the component with a
   *  `test.gen-int` import backed by an LCG pool, invokes the nullary wrapper over `trials` trials (trap =
   *  fail), and shrinks over the INT POOL (the wrapper is deterministic in its gen-int sequence, so a shrunk
   *  pool → a smaller compound). `paramTypes` is empty for a `-gen` wrapper (the shape is guest-side). */
  compoundProps?: { name: string }[];
  /** Property-test trial count (default 100) and base seed (default 0), mirroring `cdz test`. */
  trials?: number;
  seed?: number;
  /** `@param` HOST-RESPONSES for a parametric model (operator's /cad parametric showcase). A Rational
   *  `@param <name>` desugars to two Int64 host accessors `<name>-num`/`<name>-den` (guest recombines
   *  `Rational.of(num, den)`); at the component boundary these are the `param` import's accessor fns, bound
   *  by jco as camelCase `<name>Num`/`<name>Den`. This maps each param NAME → its `{ num, den }` (the
   *  slider's current value as an exact fraction). When present, `instantiateComponent` supplies the
   *  `param` import so a slider drag → new num/den → recompute+re-mesh. Absent for a non-parametric run. */
  params?: Record<string, { num: number; den: number }>;
}

export type RunResult =
  | { kind: "value-bytes"; bytes: Uint8Array }
  | { kind: "scalar"; value: string }
  | { kind: "trap"; message: string }
  | { kind: "error"; message: string }
  | { kind: "tests"; results: TestResult[] };

/// One `@test`'s outcome. `pass` = the test export returned cleanly; `!pass` with `error` = it trapped
/// (assertion failure). Scalar AND compound property `@test`s now run live (scalar via generated call-args,
/// compound via a `Test.gen-int` pool). `deferred` (NOT failed — the UI shows it pending, not dropped) is
/// reported when a parameterized `@test` can't be driven, for either of two reasons: its parameter shape has
/// no synthesized generator yet (a not-yet-supported leaf), OR a property run hit an unanswered/name-drifted
/// gen host op (a wrapper that slipped classification, or a `Test.gen-int` mismatch) rather than a real trap.
export interface TestResult {
  name: string;
  pass: boolean;
  error?: string;
  deferred?: boolean;
  /** For a property test (scalar or compound): how many trials ran (a pass shows "(N trials)"). */
  trials?: number;
  /** For a FAILED property test: the shrunk failing arguments/pool (rendered) + the replay seed. */
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
///
/// `extraImports` are merged into the component's import object — used by the COMPOUND-property driver to
/// supply the `test` interface (the `gen-int` host op) a synthesized `-gen` wrapper performs. A compound
/// `@test def p(xs: List T)` is compiled to a NULLARY `p-gen` wrapper that builds `xs` guest-side by
/// consuming a seeded int stream via `Test.gen-int : Unit -> Int64` (jco binds the kebab op `gen-int` as the
/// camelCase member `genInt` on interface `test`). The driver re-instantiates per trial with a fresh pool.
async function instantiateComponent(
  job: RunJob,
  extraImports: Record<string, unknown> = {},
): Promise<Record<string, unknown>> {
  let heap: Record<string, unknown> | null = null;
  if (job.runtime) {
    const rt = await loadComponent(job.runtime, "heap");
    // FINDING#23: the runtime imports `cadenza:nfc/normalize` (a separate NFC component — the heavy Unicode
    // tables live there, not in the runtime). Supply it as a JS shim: NFC of well-formed UTF-8 is exactly
    // String.prototype.normalize('NFC'), round-tripped through the `list<u8>` boundary (Uint8Array). Without
    // it the runtime component fails to instantiate ("Cannot destructure property 'nfc' of undefined").
    const rtRoot = await rt.instantiate(rt.getCoreModule, { [NFC_IMPORT]: nfcHostImport });
    heap = (rtRoot[HEAP_IMPORT] ?? rtRoot["heap"]) as Record<string, unknown>;
  }
  const prog = await loadComponent(job.component, "prog");
  // A COMPOUND `-gen` test component imports `test.gen-int` — jco's instantiate destructures `const { genInt }
  // = imports.test` EAGERLY, so `test` must be present for the component to instantiate AT ALL, even on the
  // nullary/scalar scan (runTests instantiates once up front to enumerate exports). Supply a benign default
  // (`genInt` → 0n) so instantiation always succeeds; the compound driver OVERRIDES it via extraImports with a
  // real seeded pool per trial. Harmless for a non-compound component (the unused import is ignored).
  const defaultTest = { test: { "gen-int": () => 0n, genInt: () => 0n } };
  // The program links the value-heap runtime, so it also imports cadenza:nfc/normalize — supply the NFC shim
  // here too (not just the shared runtime's instantiate above), or a runtime-linking example fails to load.
  const imports: Record<string, unknown> = { ...defaultTest, [NFC_IMPORT]: nfcHostImport, ...extraImports, ...(heap ? { [HEAP_IMPORT]: heap } : {}) };
  // A parametric model imports a `param` interface — one accessor per `@param`. A Rational `@param <name>`
  // desugars to the WIT accessors `<name>-num`/`<name>-den`, which jco binds as CAMELCASE JS names. jco
  // camelCases the WHOLE kebab identifier, so a KEBAB param name matters: `pa-bolt` → `pa-bolt-num` →
  // `paBoltNum` (NOT `pa-boltNum`). A single-word name (`width` → `widthNum`) is unaffected — which is why
  // single-word params worked but a kebab param (the parametric L-bracket's `pa-bolt`) failed with
  // "undefined instance import 'paBoltDen'". So camelCase the full `<name>-num`/`<name>-den` the same way
  // jco does. Returns the slider's num/den as an i64 (jco lowers i64 ↔ JS bigint, so return BigInt).
  if (job.params) {
    // kebab-case → camelCase, matching jco's WIT-identifier binding (foo-bar-baz → fooBarBaz).
    const camel = (s: string) => s.replace(/-([a-z0-9])/g, (_, c: string) => c.toUpperCase());
    const param: Record<string, () => bigint> = {};
    for (const [name, { num, den }] of Object.entries(job.params)) {
      // A param's host accessor(s) depend on its DECLARED TYPE, which we don't carry here — so bind BOTH
      // shapes; jco wires only the accessors the component actually imports + ignores the rest (extra JS
      // members are harmless):
      //   - RATIONAL `@param name : Rational` → the pair `<name>-num`/`<name>-den` (guest recombines
      //     Rational.of(num, den)) — the mounting-plate/bracket case.
      //   - INT64 `@param name : Int64` → a SINGLE `<name>` accessor returning the whole i64 (num, den=1) —
      //     the snowflake's seed/depth case. Binding only the num/den pair left the single `<name>` (e.g.
      //     `seed`/`depth`) unbound → "undefined instance import 'depth'".
      param[camel(name)] = () => BigInt(num);
      param[camel(`${name}-num`)] = () => BigInt(num);
      param[camel(`${name}-den`)] = () => BigInt(den);
    }
    imports.param = param;
  }
  return prog.instantiate(prog.getCoreModule, imports);
}

/// Run each named `@test` export and report per-test pass/fail. Contract (mirrors `cdz test`): a test
/// export that RETURNS CLEANLY passed; one that TRAPS (an assertion via `trap(msg)`) failed. Belt +
/// suspenders on top of `compile_tests`'s `-gen`-suffix classification: if invoking a nullary export errors
/// in a way that signals an unanswered gen host op (a property `-gen` wrapper that slipped through the
/// classification into the nullary list), it is reported DEFERRED, not failed — the real property drivers
/// (scalar `runScalarProperties` / compound `runCompoundProperties`) handle classified property tests.
/// (`normalizeName` — the kebab/camel boundary-name matcher — is a pure helper in genPool.ts.)
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
  // `-gen` tests are not in `scalarProps` — they're driven by `runCompoundProperties` below (a gen-int pool).
  // REUSE the `root` already instantiated above — a property test calls the same exports (by name) as the
  // nullary tests, so re-instantiating (a second jco transpile + wasm load) would be pure waste (flagged in
  // PR #533). The nullary invokes above don't persist state into the exports we re-call (each @test/property
  // export is a fresh evaluation), so one instance serves both.
  const propResults = await runScalarProperties(job, root);
  // Drive the COMPOUND-param property tests (their param is built guest-side by a nullary `-gen` wrapper that
  // performs `Test.gen-int`). Each needs its OWN instance per trial (a fresh gen-int pool), so this does NOT
  // reuse `root` — it re-instantiates with a `test` import. Empty `compoundProps` → no work, no instances.
  const compoundResults = await runCompoundProperties(job);
  return { kind: "tests", results: [...results, ...propResults, ...compoundResults] };
}

async function runComponent(job: RunJob): Promise<RunResult> {
  const root = await instantiateComponent(job);
  // `selectRunEntry` classifies the entry (compound resource-escape / bare scalar / parameterized / none).
  // Crucially it detects a PARAMETERIZED entry — a `def main(a: Int64) = …` compiles to an arity-N `make(a)`
  // (compound) or `main(a)` (scalar) — up front, so we never call an arity-N maker/function with no argument
  // (which lowers `undefined` to the missing i64 and throws "Cannot convert undefined to a BigInt": the
  // operator-reported "any program with an argument fails / result coerced to a BigInt" playground bug).
  const plan = selectRunEntry(root);
  switch (plan.kind) {
    case "compound": {
      // Resource-escape path: `make()` builds the value (a handle), `encode(handle)` → canonical value bytes.
      const handle = plan.iface.make();
      const bytes = plan.iface.encode(handle);
      return { kind: "value-bytes", bytes: new Uint8Array(bytes) };
    }
    case "scalar":
      // A bare nullary export (the runnable `main` shape) — Run produces a value with no input.
      return { kind: "scalar", value: String(plan.fn()) };
    case "parameterized":
      // Run needs a zero-argument entry; a parameterized one is called from the REPL or via a nullary `main`.
      return { kind: "error", message: parameterizedEntryMessage(plan.name, plan.arity) };
    case "none":
      return { kind: "error", message: "component exported no runnable entry" };
  }
}

/// A property test's default trial count, mirroring `cdz test`. (The generator core — `intRange`, `genArg`,
/// `genArgs`, `lcgStep`, `renderArgs`, and the `GenPool` — is a pure, node-tested twin of the native
/// `proptest_gen.rs` generator, extracted into genPool.ts.)
const DEFAULT_TRIALS = 100;

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
/// inputs — the browser equivalent of `cdz test`'s property driver. Compound (`-gen`) tests are driven by
/// `runCompoundProperties` instead (they need a `Test.gen-int` pool + per-trial instance).
// `preInstantiated` lets the caller (`runTests`) pass the component instance it ALREADY built for the
// nullary tests, so a mixed nullary+property run instantiates ONCE, not twice (PR #533). When called
// standalone (no pre-built root), it instantiates itself — but only after the empty-props early-return, so
// a run with no scalar properties never instantiates here.
async function runScalarProperties(job: RunJob, preInstantiated?: Record<string, unknown>): Promise<TestResult[]> {
  const props = job.scalarProps ?? [];
  if (props.length === 0) return [];
  const root = preInstantiated ?? (await instantiateComponent(job));
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

/// Run ONE compound-param property test: the nullary `-gen` wrapper builds its compound argument from a
/// seeded `gen-int` pool. Per trial (seeded `seed + t + 1`), instantiate with a fresh pool + invoke; a THROW
/// (trap) is a failing trial. On failure, SHRINK the recorded pool toward a minimal counterexample — first
/// truncating trailing draws (shorter collections), then halving each remaining draw toward 0 (smaller
/// leaves) — re-running with the shrunk pool, keeping a step iff the failure persists. Mirrors the native
/// `shrink_pool`: no compound-value introspection in JS, because the wrapper is deterministic in its pool.
async function runCompoundProperty(job: RunJob, name: string, trials: number, seed: number): Promise<TestResult> {
  // Instantiate the wrapper with a given pool + invoke it; returns true iff the trial FAILED (threw/trapped).
  const runPool = async (pool: GenPool): Promise<boolean> => {
    try {
      const root = await instantiateComponent(job, { test: { "gen-int": pool.next, genInt: pool.next } });
      const fn = new Map(exportedFunctions(root).map((f) => [normalizeName(f.name), f.fn])).get(normalizeName(name));
      if (typeof fn !== "function") throw new Error("compound property export not found");
      await (fn as () => unknown)();
      return false;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // A still-unanswered gen op (name drift) is NOT a property failure — surface it so it's not a false ✗.
      if (/gen-int|test\.gen|no enclosing handler|unhandled|host (op|function)/i.test(message)) throw err;
      return true;
    }
  };

  let failing: bigint[] | null = null;
  try {
    for (let t = 0; t < trials; t++) {
      const pool = new GenPool(BigInt(seed) + BigInt(t) + 1n);
      if (await runPool(pool)) {
        failing = pool.values.slice(0, /* only the draws actually consumed */ pool.values.length);
        break;
      }
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { name, pass: false, deferred: true, error: `property test — deferred (${message.slice(0, 60)})` };
  }
  if (!failing) return { name, pass: true, trials };

  // SHRINK over the int pool. (1) Truncate trailing draws while the failure persists (shorter collections).
  let best = failing.slice();
  for (let len = best.length - 1; len >= 1; len--) {
    const cand = best.slice(0, len);
    if (await runPool(new GenPool(0n, cand))) best = cand;
    else break;
  }
  // (2) Halve each remaining draw toward 0 while the failure persists (smaller leaves).
  for (let i = 0; i < best.length; i++) {
    let v = best[i];
    while (v !== 0n) {
      const cand = best.slice();
      cand[i] = v / 2n;
      if (await runPool(new GenPool(0n, cand))) { best[i] = cand[i]; v = cand[i]; }
      else break;
    }
  }
  return {
    name,
    pass: false,
    error: "property failed",
    counterexample: { args: `${name}(<generated>)  [pool: ${best.map((n) => n.toString()).join(", ")}]`, seed },
  };
}

/// Run each COMPOUND-param property test (from `param_test_signatures`, `compound: true`) — the browser
/// equivalent of `cdz test`'s compound generator/shrink. Each is a nullary `-gen` wrapper driven over a
/// `gen-int` pool. Re-instantiates per trial (fresh pool), so an empty `compoundProps` does no work.
async function runCompoundProperties(job: RunJob): Promise<TestResult[]> {
  const props = job.compoundProps ?? [];
  if (props.length === 0) return [];
  const trials = job.trials ?? DEFAULT_TRIALS;
  const seed = job.seed ?? 0;
  const results: TestResult[] = [];
  for (const { name } of props) {
    results.push(await runCompoundProperty(job, name, trials, seed));
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
