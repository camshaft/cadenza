/// Main-thread driver for the disposable RUN worker.
///
/// Runs a compiled component with a WATCHDOG: wasm can't be interrupted cooperatively, so an
/// infinite loop in user code would hang its worker forever. We race the worker's result against a
/// timeout; on timeout we `terminate()` the (now-dead) worker, report "timed out", and drop the
/// reference so the NEXT run spins up a fresh worker. A completed run reuses the same worker.

import { renderValue } from "../compiler/client.ts";
import runtimeUrl from "../wasm/runtime.wasm?url";
import type { RunJob, RunResult } from "./runWorker.ts";

/** How long a single run may take before we assume a runaway loop and kill the worker. */
const RUN_TIMEOUT_MS = 5000;

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

/// Execute a compiled component, rendering a compound value to canonical text. Serialized: one run
/// at a time (the guide runs a single example on demand), so a simple `busy` guard suffices.
export async function run(component: Uint8Array): Promise<RunOutcome> {
  if (busy) return { kind: "error", message: "a run is already in progress" };
  busy = true;
  try {
    const runtime = await loadRuntime();
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
        return { kind: "value", text: raw.value };
      case "value-bytes":
        return { kind: "value", text: await renderValue(raw.bytes) };
      case "trap":
        return { kind: "trap", message: raw.message };
      default:
        return { kind: "error", message: raw.message };
    }
  } finally {
    busy = false;
  }
}
