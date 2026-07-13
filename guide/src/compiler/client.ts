/// Main-thread handle to the compile worker. A single shared worker instance wrapped with Comlink,
/// so any component can `await compiler.compile(...)` as if it were a local async function.

import * as Comlink from "comlink";
import type { CompilerApi, CompileOutcome, Diag, Surface, TypeAtInfo, DefineAtInfo } from "./worker.ts";

let proxy: Comlink.Remote<CompilerApi> | null = null;

function client(): Comlink.Remote<CompilerApi> {
  if (!proxy) {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), {
      type: "module",
    });
    proxy = Comlink.wrap<CompilerApi>(worker);
  }
  return proxy;
}

export function compile(text: string, from: Surface): Promise<CompileOutcome> {
  return client().compile(text, from);
}

/// Type-check without building a component (as-you-type diagnostics). No export required upstream.
export function diagnostics(text: string, from: Surface): Promise<Diag[]> {
  return client().diagnostics(text, from);
}

/// The inferred type at a UTF-8 byte offset, for a hover tooltip.
export function typeAt(text: string, from: Surface, byteOffset: number): Promise<TypeAtInfo | null> {
  return client().typeAt(text, from, byteOffset);
}

/// The definition a reference at a UTF-8 byte offset points to, for go-to-definition.
export function defineAt(text: string, from: Surface, byteOffset: number): Promise<DefineAtInfo | null> {
  return client().defineAt(text, from, byteOffset);
}

/// Byte ranges of every occurrence referencing the name at a byte offset (find-all-references), flat.
export function references_at(text: string, from: Surface, byteOffset: number): Promise<Uint32Array> {
  return client().referencesAt(text, from, byteOffset);
}

/// `to` may be a surface (`ml`/`sexpr`) or an output-only view (`debug`/`flat`) for "show the raw AST".
export type RenderTarget = Surface | "debug" | "flat";

export function renderSyntax(text: string, from: Surface, to: RenderTarget): Promise<string> {
  return client().renderSyntax(text, from, to);
}

export function renderValue(bytes: Uint8Array): Promise<string> {
  return client().renderValue(bytes);
}

/// Emit the program as Rust source (sync, or gas-metered async) — for the playground's output views.
export function emitRust(text: string, from: Surface, isAsync: boolean): Promise<string> {
  return client().emitRust(text, from, isAsync);
}

/// The program's DWARF-free core module bytes, unwrapped from the component — for the WAT view. Null
/// if the program declines.
export function coreModule(text: string, from: Surface): Promise<Uint8Array | null> {
  return client().coreModule(text, from);
}

/// Evaluate a REPL expression against a buffer's definitions (the playground's mini-REPL). Returns a
/// compile outcome; the caller runs the component through the run client just like a normal example.
export function replEval(buffer: string, expr: string, from: Surface): Promise<CompileOutcome> {
  return client().replEval(buffer, expr, from);
}

/// The names of every top-level definition the buffer declares — for the REPL's autocomplete.
export function definedNames(buffer: string, from: Surface): Promise<string[]> {
  return client().definedNames(buffer, from);
}

export function runtimeHash(): Promise<string> {
  return client().runtimeHash();
}

export type { CompileOutcome, Surface, Diag, TypeAtInfo, DefineAtInfo };
