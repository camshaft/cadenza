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
/// compound via a `Test.gen-int` pool); `deferred` is reported only for a parameterized `@test` whose
/// parameter shape the compiler couldn't synthesize a generator for (a not-yet-supported leaf) — NOT failed,
/// so the UI can show it as pending rather than dropping it.
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
    const rtRoot = await rt.instantiate(rt.getCoreModule, {});
    heap = (rtRoot[HEAP_IMPORT] ?? rtRoot["heap"]) as Record<string, unknown>;
  }
  const prog = await loadComponent(job.component, "prog");
  // A COMPOUND `-gen` test component imports `test.gen-int` — jco's instantiate destructures `const { genInt }
  // = imports.test` EAGERLY, so `test` must be present for the component to instantiate AT ALL, even on the
  // nullary/scalar scan (runTests instantiates once up front to enumerate exports). Supply a benign default
  // (`genInt` → 0n) so instantiation always succeeds; the compound driver OVERRIDES it via extraImports with a
  // real seeded pool per trial. Harmless for a non-compound component (the unused import is ignored).
  const defaultTest = { test: { "gen-int": () => 0n, genInt: () => 0n } };
  const imports: Record<string, unknown> = { ...defaultTest, ...extraImports, ...(heap ? { [HEAP_IMPORT]: heap } : {}) };
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
/// type not here (`"other"`, a compound) is never a SCALAR prop — a compound param is driven by the separate
/// compound driver (a `-gen` wrapper over a `Test.gen-int` pool), not by this call-arg generator.
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

/// A seeded int POOL for the compound `Test.gen-int` driver. Two modes, because the wrapper calls `gen-int`
/// an a-priori-unknown number of times (a `List` gens its length, then each element):
///   - GENERATIVE (initial trial, no `preset`): lazily EXTENDS from the LCG on each draw, capturing the
///     concrete sequence in `values` — every draw deterministic from `seed`, and `values` records exactly
///     what was consumed so the shrinker can replay it.
///   - REPLAY (a `preset` pool, from the shrinker): serves the preset draws, then pads EXHAUSTED draws with
///     `0n` — it does NOT LCG-extend. This is what makes truncation a faithful shrink: a shorter pool means
///     the wrapper genuinely sees fewer/zero draws (→ a shorter collection / zero-valued tail), not a
///     different LCG-seeded tail. (A `0` gen-int typically drives a length/element toward its minimum.)
class GenPool {
  private state: bigint;
  readonly values: bigint[] = [];
  private i = 0;
  private readonly replay: boolean;
  constructor(seed: bigint, preset?: bigint[]) {
    this.state = seed;
    this.replay = preset !== undefined;
    if (preset) this.values = preset.slice();
  }
  /// The `Test.gen-int` host op: yield the next i64 (jco lowers i64 ↔ JS bigint).
  next = (): bigint => {
    if (this.i >= this.values.length) {
      if (this.replay) return 0n; // exhausted a preset pool → pad with 0 (faithful truncation-shrink)
      this.state = lcgStep(this.state);
      this.values.push(this.state & 0xffffffffffffffffn);
    }
    return this.values[this.i++];
  };
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
