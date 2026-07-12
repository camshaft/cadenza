/// Main-thread handle to the compile worker. A single shared worker instance wrapped with Comlink,
/// so any component can `await compiler.compile(...)` as if it were a local async function.

import * as Comlink from "comlink";
import type { CompilerApi, CompileOutcome, Surface } from "./worker.ts";

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

export function renderSyntax(text: string, from: Surface, to: Surface): Promise<string> {
  return client().renderSyntax(text, from, to);
}

export function renderValue(bytes: Uint8Array): Promise<string> {
  // Comlink can't structured-clone a Uint8Array view cheaply across the boundary without a copy;
  // for guide-sized value forms (tens of bytes) that is negligible.
  return client().renderValue(bytes);
}

export function runtimeHash(): Promise<string> {
  return client().runtimeHash();
}

export type { CompileOutcome, Surface };
export type { Diag } from "./worker.ts";
