/// Main-thread handle to the compile worker. A single shared worker instance wrapped with Comlink,
/// so any component can `await compiler.compile(...)` as if it were a local async function.

import * as Comlink from "comlink";
import type { CompilerApi, CompileOutcome, Diag, Surface, TypeAtInfo } from "./worker.ts";

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

/// `to` may be a surface (`ml`/`sexpr`) or an output-only view (`debug`/`flat`) for "show the raw AST".
export type RenderTarget = Surface | "debug" | "flat";

export function renderSyntax(text: string, from: Surface, to: RenderTarget): Promise<string> {
  return client().renderSyntax(text, from, to);
}

export function renderValue(bytes: Uint8Array): Promise<string> {
  return client().renderValue(bytes);
}

export function runtimeHash(): Promise<string> {
  return client().runtimeHash();
}

export type { CompileOutcome, Surface, Diag, TypeAtInfo };
