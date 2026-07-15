/// Main-thread driver for the disposable RUN worker.
///
/// Runs a compiled component with a WATCHDOG: wasm can't be interrupted cooperatively, so an
/// infinite loop in user code would hang its worker forever. We race the worker's result against a
/// timeout; on timeout we `terminate()` the (now-dead) worker, report "timed out", and drop the
/// reference so the NEXT run spins up a fresh worker. A completed run reuses the same worker.

import { renderValue, renderSyntax, renderSyntaxDisplay, runtimeHash, type Surface } from "../compiler/client.ts";
import runtimeUrl from "../wasm/runtime.wasm?url";
import { hexDigest, explainIfStaleRuntime } from "./runtimeHashGuard.ts";
import type { RunJob, RunResult } from "./runWorker.ts";

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

      const job: RunJob = { component, runtime };
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
      default:
        return { kind: "error", message: explainIfStaleRuntime(raw.message, runtimeMatchesCompiler) };
    }
  } finally {
    busy = false;
  }
}
