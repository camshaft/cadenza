/// The compile worker. Runs the Cadenza compiler (cdz-wasm) off the UI thread so a compile never
/// janks the page. Exposes a small Comlink API the main thread calls with `await`.
///
/// The compiler is pure and fast for guide-sized snippets, so this worker is long-lived and reused
/// (unlike the RUN worker, which is disposable so an infinite loop can be killed). It never executes
/// user output — it only turns source text into a component and re-renders syntax.

import * as Comlink from "comlink";
import init, {
  compile as wasmCompile,
  render_syntax as wasmRenderSyntax,
  render_value as wasmRenderValue,
  required_runtime_hash as wasmRuntimeHash,
} from "../wasm/pkg/cdz_wasm.js";
// The wasm binary as a URL Vite fingerprints; `--target web` init fetches + compiles it.
import wasmUrl from "../wasm/pkg/cdz_wasm_bg.wasm?url";

export type Surface = "ml" | "sexpr";

export interface Diag {
  error: boolean;
  code: string;
  message: string;
  node: number;
}

export interface CompileOutcome {
  /** The emitted component bytes, or null if compilation failed. */
  component: Uint8Array | null;
  diagnostics: Diag[];
}

let ready: Promise<void> | null = null;
function ensureReady(): Promise<void> {
  ready ??= init({ module_or_path: wasmUrl }).then(() => undefined);
  return ready;
}

const api = {
  async compile(text: string, from: Surface): Promise<CompileOutcome> {
    await ensureReady();
    const r = wasmCompile(text, from);
    // Copy the component bytes out of wasm memory before the CompileResult is dropped.
    const component = r.component ? new Uint8Array(r.component) : null;
    const diagnostics: Diag[] = r.diagnostics.map((d) => ({
      error: d.error,
      code: d.code,
      message: d.message,
      node: d.node,
    }));
    return { component, diagnostics };
  },

  async renderSyntax(text: string, from: Surface, to: Surface): Promise<string> {
    await ensureReady();
    return wasmRenderSyntax(text, from, to);
  },

  async renderValue(bytes: Uint8Array): Promise<string> {
    await ensureReady();
    return wasmRenderValue(bytes);
  },

  async runtimeHash(): Promise<string> {
    await ensureReady();
    return wasmRuntimeHash();
  },
};

export type CompilerApi = typeof api;
Comlink.expose(api);
