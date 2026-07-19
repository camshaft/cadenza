/// Main-thread driver for the disposable RUN worker.
///
/// Runs a compiled component with a WATCHDOG: wasm can't be interrupted cooperatively, so an
/// infinite loop in user code would hang its worker forever. We race the worker's result against a
/// timeout; on timeout we `terminate()` the (now-dead) worker, report "timed out", and drop the
/// reference so the NEXT run spins up a fresh worker. A completed run reuses the same worker.

import { renderValue, renderSyntax, renderSyntaxDisplay, runtimeHash, compileTests, paramTestSignatures, type Surface, type ParamTestSig } from "../compiler/client.ts";
import runtimeUrl from "../wasm/runtime.wasm?url";
import { hexDigest, explainIfStaleRuntime } from "./runtimeHashGuard.ts";
import type { RunJob, RunResult, TestResult } from "./runWorker.ts";

export type { TestResult };

/** How long a single run may take before we assume a runaway loop and kill the worker. */
const RUN_TIMEOUT_MS = 5000;

/// Whether the bundled `runtime.wasm` is the one THIS compiler emits imports against — checked once.
/// A MISMATCH (stale deployment) corrupts memory via the bare `cadenza:runtime/heap` import; see
/// `runtimeHashGuard.ts` for the full why. `null` = not yet determined / couldn't check.
let runtimeMatchesCompiler: boolean | null = null;
async function checkRuntimeHash(bytes: Uint8Array): Promise<void> {
  if (runtimeMatchesCompiler !== null) return; // check once
  try {
    // Copy into a fresh ArrayBuffer — `bytes` may be a view over a larger/SharedArrayBuffer that
    // SubtleCrypto.digest rejects; the copy is a plain ArrayBuffer of exactly the runtime bytes.
    const digest = await crypto.subtle.digest("SHA-256", bytes.slice().buffer);
    const required = await runtimeHash();
    runtimeMatchesCompiler = hexDigest(digest) === required;
  } catch {
    // If we can't compute or fetch the hash (no SubtleCrypto, compiler error), don't block the run —
    // leave the verdict unknown so we never MISReport a good run as stale.
    runtimeMatchesCompiler = null;
  }
}

export type RunOutcome =
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

let worker: Worker | null = null;
let busy = false;

function freshWorker(): Worker {
  return new Worker(new URL("./runWorker.ts", import.meta.url), { type: "module" });
}

/** Lazily fetch the bundled value-heap runtime bytes once; null if none was staged (scalar-only). */
let runtimeBytes: Promise<Uint8Array | null> | null = null;
function loadRuntime(): Promise<Uint8Array | null> {
  runtimeBytes ??= fetch(runtimeUrl)
    .then((r) => (r.ok ? r.arrayBuffer() : null))
    .then((buf) => (buf ? new Uint8Array(buf) : null))
    .catch(() => null);
  return runtimeBytes;
}

/// Render a value form (an s-expression `(: value type)` string) into the reader's chosen surface, so
/// the Result reads in the same syntax as the code (`5 : Int64` vs `(: 5 Int64)`, `tuple(1, 2)` vs
/// `(tuple 1 2)`). A no-op for the s-expr surface; falls back to the raw text if it won't re-render.
/// When `display` is set, the ML target is rendered for human display (a rational bare, a quantity in
/// its concise `<value> <unit>` surface, the result type annotation dropped) — the calculator's mode.
async function renderValueInSurface(sexprValue: string, surface: Surface, display: boolean): Promise<string> {
  if (surface === "sexpr") return sexprValue;
  const render = display ? renderSyntaxDisplay : renderSyntax;
  return render(sexprValue, "sexpr", surface).catch(() => sexprValue);
}

/// Execute a compiled component, rendering a compound value to canonical text in `surface` (default
/// s-expr). `display` selects the human-facing render for a result (the calculator; default off keeps
/// the canonical form the playground shows). Serialized: one run at a time (the guide runs a single
/// example on demand), so a simple `busy` guard suffices.
export async function run(
  component: Uint8Array,
  surface: Surface = "sexpr",
  display = false,
  /** `@param` host-responses for a PARAMETRIC model (/cad parametric showcase): each `@param <name>` →
   *  its current `{ num, den }` (the slider value as an exact fraction). Supplied to the run worker's
   *  `param` import so the model reads live slider values. Omit for a non-parametric run. */
  params?: Record<string, { num: number; den: number }>,
): Promise<RunOutcome> {
  if (busy) return { kind: "error", message: "a run is already in progress" };
  busy = true;
  try {
    const runtime = await loadRuntime();
    // Verify (once) that the bundled runtime matches the compiler — so a stale-deployment mismatch is
    // reported clearly below rather than as an inscrutable memory trap. A compound-returning program
    // needs the runtime; a scalar program (runtime == null) never touches the heap, so skip the check.
    if (runtime) await checkRuntimeHash(runtime);
    worker ??= freshWorker();
    const w = worker;

    const raw = await new Promise<RunResult | { kind: "timeout" }>((resolve) => {
      const timer = setTimeout(() => {
        w.terminate();
        worker = null; // terminated worker is dead; next run makes a fresh one
        resolve({ kind: "timeout" });
      }, RUN_TIMEOUT_MS);

      w.onmessage = (e: MessageEvent<RunResult>) => {
        clearTimeout(timer);
        resolve(e.data);
      };
      w.onerror = (e) => {
        clearTimeout(timer);
        worker = null;
        resolve({ kind: "error", message: e.message } as RunResult);
      };

      const job: RunJob = { component, runtime, params };
      w.postMessage(job);
    });

    switch (raw.kind) {
      case "timeout":
        return { kind: "timeout" };
      case "scalar":
        // A scalar renders identically in both surfaces (a bare number/bool), so no re-render needed.
        return { kind: "value", text: raw.value };
      case "value-bytes":
        return {
          kind: "value",
          text: await renderValueInSurface(await renderValue(raw.bytes), surface, display),
        };
      case "trap":
        return { kind: "trap", message: explainIfStaleRuntime(raw.message, runtimeMatchesCompiler) };
      case "error":
        return { kind: "error", message: explainIfStaleRuntime(raw.message, runtimeMatchesCompiler) };
      default:
        // A normal `run` never posts a test job, so a "tests" result shouldn't reach here — but keep the
        // switch exhaustive rather than reading `.message` off a variant that lacks it.
        return { kind: "error", message: "unexpected run result" };
    }
  } finally {
    busy = false;
  }
}

/// Run a TEST-LAYOUT component (from `compile_tests`) and report each named `@test`'s pass/fail. Same
/// worker + watchdog + stale-runtime guard as `run`; posts a `mode: "test"` job so the worker invokes each
/// `testNames` export (clean return = pass, trap = fail, unanswered-`Test.gen` = deferred). Serialized like
/// `run`. A timeout marks every test as errored (the whole suite ran past the watchdog).
export async function runTestComponent(
  component: Uint8Array,
  testNames: string[],
  scalarProps: { name: string; paramTypes: string[] }[] = [],
  compoundProps: { name: string }[] = [],
): Promise<TestResult[]> {
  if (busy) return testNames.map((name) => ({ name, pass: false, error: "a run is already in progress" }));
  busy = true;
  try {
    const runtime = await loadRuntime();
    if (runtime) await checkRuntimeHash(runtime);
    worker ??= freshWorker();
    const w = worker;

    const raw = await new Promise<RunResult | { kind: "timeout" }>((resolve) => {
      const timer = setTimeout(() => {
        w.terminate();
        worker = null;
        resolve({ kind: "timeout" });
      }, RUN_TIMEOUT_MS);
      w.onmessage = (e: MessageEvent<RunResult>) => {
        clearTimeout(timer);
        resolve(e.data);
      };
      w.onerror = (e) => {
        clearTimeout(timer);
        worker = null;
        resolve({ kind: "error", message: e.message } as RunResult);
      };
      const job: RunJob = { component, runtime, mode: "test", testNames, scalarProps, compoundProps };
      w.postMessage(job);
    });

    if (raw.kind === "tests") return raw.results;
    // A timeout / whole-suite error surfaces against every test AND every scalar/compound property (else a
    // driven property that timed out would silently vanish from the report).
    const allNames = [...testNames, ...scalarProps.map((p) => p.name), ...compoundProps.map((p) => p.name)];
    if (raw.kind === "timeout") return allNames.map((name) => ({ name, pass: false, error: "timed out" }));
    // A whole-suite error (couldn't instantiate the component, etc.) — surface it against every test so the
    // caller shows the failure rather than a silent empty result.
    const message = "message" in raw ? explainIfStaleRuntime(raw.message, runtimeMatchesCompiler) : "test run failed";
    return allNames.map((name) => ({ name, pass: false, error: message }));
  } finally {
    busy = false;
  }
}

/// The full in-browser `@test` runner: compile `source` in test-layout mode, then invoke each nullary
/// `@test` export and report per-test pass/fail — the browser equivalent of `cdz test`. A compile failure
/// (incl. "no `@test`") returns the compile diagnostics as an error outcome; otherwise a `TestResult` per
/// nullary test (clean return = pass, trap = fail), plus a `deferred` entry per PARAMETERIZED `@test`
/// (property/exhaustive — real trials are a follow-up), so the UI can show "N property tests deferred"
/// rather than dropping them. `<Runnable mode="test">` + the check-examples test-branch call this.
export type TestRunOutcome =
  | { kind: "tests"; results: TestResult[] }
  | { kind: "error"; message: string };

export async function runTests(source: string, surface: Surface = "sexpr"): Promise<TestRunOutcome> {
  const compiled = await compileTests(source, surface);
  if (!compiled.component) {
    const firstErr = compiled.diagnostics.find((d) => d.error);
    return {
      kind: "error",
      message: firstErr ? `${firstErr.code || ""} ${firstErr.message}`.trim() : "no `@test` definition to run",
    };
  }
  // Split the parameterized @tests into SCALAR (params on the export → the arg-driver runs them live over
  // generated call-args) and COMPOUND (`-gen` wrappers building their argument guest-side via `Test.gen-int`
  // → the compound driver instantiates with a seeded gen-int pool + shrinks over it). `param_test_signatures`
  // classifies each: `compound: false` = scalar, `compound: true` = a `-gen` wrapper. BOTH now run live.
  const sigs = await paramTestSignatures(source, surface).catch(() => [] as ParamTestSig[]);
  const scalarProps = sigs
    .filter((s) => !s.compound)
    .map((s) => ({ name: s.name, paramTypes: s.paramTypes }));
  const compoundProps = sigs.filter((s) => s.compound).map((s) => ({ name: s.name }));
  const ran = await runTestComponent(compiled.component, compiled.nullaryTestNames, scalarProps, compoundProps);
  // Defer only what the signatures did NOT classify as scalar OR compound (a defensive union — e.g. a param
  // shape the compiler couldn't synthesize a generator for, like a not-yet-supported leaf type).
  const driven = new Set([...scalarProps, ...compoundProps].map((p) => p.name));
  const deferredNames = compiled.paramTestNames.filter((n) => !driven.has(n));
  const deferred: TestResult[] = deferredNames.map((name) => ({
    name,
    pass: false,
    deferred: true,
    error: "property test — deferred (no synthesized generator for this parameter shape yet)",
  }));
  return { kind: "tests", results: [...ran, ...deferred] };
}
