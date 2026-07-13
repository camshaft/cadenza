/// The compile worker. Runs the Cadenza compiler (cdz-wasm) off the UI thread so a compile never
/// janks the page. Exposes a small Comlink API the main thread calls with `await`.
///
/// The compiler is pure and fast for guide-sized snippets, so this worker is long-lived and reused
/// (unlike the RUN worker, which is disposable so an infinite loop can be killed). It never executes
/// user output — it only turns source text into a component, type-checks it, and re-renders syntax.

import * as Comlink from "comlink";
import init, {
  compile as wasmCompile,
  diagnostics as wasmDiagnostics,
  type_at as wasmTypeAt,
  define_at as wasmDefineAt,
  references_at as wasmReferencesAt,
  emit_rust as wasmEmitRust,
  core_module as wasmCoreModule,
  render_syntax as wasmRenderSyntax,
  render_value as wasmRenderValue,
  required_runtime_hash as wasmRuntimeHash,
} from "../wasm/pkg/cdz_wasm.js";
// The wasm binary as a URL Vite fingerprints; `--target web` init fetches + compiles it.
import wasmUrl from "../wasm/pkg/cdz_wasm_bg.wasm?url";

export type Surface = "ml" | "sexpr";

export interface Diag {
  /** true = error (denies a component); false = warning. */
  error: boolean;
  /** The stable CDZ#### code, or "" for an uncoded decline. */
  code: string;
  message: string;
  /** AST node index (u32::MAX when unanchored). */
  node: number;
  /** Source byte range [from, to) (UTF-8), resolved in Rust. 0,0 when unanchored. */
  from: number;
  to: number;
}

export interface CompileOutcome {
  /** The emitted component bytes, or null if compilation failed. */
  component: Uint8Array | null;
  diagnostics: Diag[];
}

/** The inferred type at a source offset, for hover. */
export interface TypeAtInfo {
  typeName: string;
  from: number;
  to: number;
}

/** The definition a reference at a source offset points to, for go-to-definition. Byte offsets. */
export interface DefineAtInfo {
  from: number;
  to: number;
  refFrom: number;
  refTo: number;
}

let ready: Promise<void> | null = null;
function ensureReady(): Promise<void> {
  ready ??= init({ module_or_path: wasmUrl }).then(() => undefined);
  return ready;
}

function toDiag(d: {
  error: boolean;
  code: string;
  message: string;
  node: number;
  from: number;
  to: number;
}): Diag {
  return { error: d.error, code: d.code, message: d.message, node: d.node, from: d.from, to: d.to };
}

const api = {
  async compile(text: string, from: Surface): Promise<CompileOutcome> {
    await ensureReady();
    const r = wasmCompile(text, from);
    const component = r.component ? new Uint8Array(r.component) : null;
    const diagnostics: Diag[] = r.diagnostics.map(toDiag);
    return { component, diagnostics };
  },

  /// Type-check `text` and return all diagnostics with source spans — no component built, no export
  /// required. The as-you-type entry.
  async diagnostics(text: string, from: Surface): Promise<Diag[]> {
    await ensureReady();
    return wasmDiagnostics(text, from).map(toDiag);
  },

  /// The inferred type at a UTF-8 byte offset (for a hover tooltip), or null if there's nothing there.
  async typeAt(text: string, from: Surface, byteOffset: number): Promise<TypeAtInfo | null> {
    await ensureReady();
    const t = wasmTypeAt(text, from, byteOffset);
    return t ? { typeName: t.type_name, from: t.from, to: t.to } : null;
  },

  /// The definition a reference at a UTF-8 byte offset points to (go-to-definition), or null.
  async defineAt(text: string, from: Surface, byteOffset: number): Promise<DefineAtInfo | null> {
    await ensureReady();
    const d = wasmDefineAt(text, from, byteOffset);
    return d ? { from: d.from, to: d.to, refFrom: d.ref_from, refTo: d.ref_to } : null;
  },

  /// Byte ranges of every occurrence referencing the name at a UTF-8 byte offset (find-all-references),
  /// as a flat [from0,to0,from1,to1,…]. Empty when the cursor isn't on a referenced name.
  async referencesAt(text: string, from: Surface, byteOffset: number): Promise<Uint32Array> {
    await ensureReady();
    return new Uint32Array(wasmReferencesAt(text, from, byteOffset));
  },

  // `to` may be a surface or an output-only view ("debug"/"flat"); the wasm accepts the wider set.
  async renderSyntax(text: string, from: Surface, to: string): Promise<string> {
    await ensureReady();
    return wasmRenderSyntax(text, from, to);
  },

  async renderValue(bytes: Uint8Array): Promise<string> {
    await ensureReady();
    return wasmRenderValue(bytes);
  },

  /// Emit the program as Rust source — sync or (gas-metered) async — for the "Compiled" output views.
  async emitRust(text: string, from: Surface, isAsync: boolean): Promise<string> {
    await ensureReady();
    return wasmEmitRust(text, from, isAsync);
  },

  /// The program's embedded CORE MODULE bytes (DWARF-free, unwrapped from the component) — for the WAT
  /// view. Null if the program declines. The caller prints these with `wasm-tools print`.
  async coreModule(text: string, from: Surface): Promise<Uint8Array | null> {
    await ensureReady();
    const bytes = wasmCoreModule(text, from);
    return bytes ? new Uint8Array(bytes) : null;
  },

  async runtimeHash(): Promise<string> {
    await ensureReady();
    return wasmRuntimeHash();
  },
};

export type CompilerApi = typeof api;
Comlink.expose(api);
