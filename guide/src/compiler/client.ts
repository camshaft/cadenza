/// Main-thread handle to the compile worker. A single shared worker instance wrapped with Comlink,
/// so any component can `await compiler.compile(...)` as if it were a local async function.

import * as Comlink from "comlink";
import type { CompilerApi, CompileOutcome, TestCompileOutcome, Diag, DiagFix, Surface, TypeAtInfo, DefineAtInfo, SemanticTok, DispositionInfo } from "./worker.ts";

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

/// Compile in TEST-LAYOUT mode: each `@test` becomes an invocable export. Returns component + diagnostics
/// + the discovered test names (nullary vs parameterized). The run worker invokes the nullary exports.
export function compileTests(text: string, from: Surface): Promise<TestCompileOutcome> {
  return client().compileTests(text, from);
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

/// Semantic syntax-highlight tokens for the whole buffer — each a byte range + a compiler-classified
/// role. Empty when the buffer doesn't parse (the editor keeps its lexical colours).
export function semanticTokens(text: string, from: Surface): Promise<SemanticTok[]> {
  return client().semanticTokens(text, from);
}

/// How the compiler compiled the definition whose name is at a byte offset (inlined / specialized /
/// emitted / transformed / unreferenced) + its concrete monomorphizations — for a "what did the
/// compiler do?" hover. Null when the offset isn't on a definition name.
export function disposition(text: string, from: Surface, byteOffset: number): Promise<DispositionInfo | null> {
  return client().disposition(text, from, byteOffset);
}

/// `to` may be a surface (`ml`/`sexpr`) or an output-only view (`debug`/`flat`) for "show the raw AST".
export type RenderTarget = Surface | "debug" | "flat";

export function renderSyntax(text: string, from: Surface, to: RenderTarget): Promise<string> {
  return client().renderSyntax(text, from, to);
}

/// Like `renderSyntax`, but renders the ML target for human DISPLAY (a rational bare, a quantity in its
/// concise `<value> <unit>` surface, the result type annotation dropped). Used by the calculator to show
/// a result readably; the playground keeps the canonical, re-readable `renderSyntax`.
export function renderSyntaxDisplay(text: string, from: Surface, to: RenderTarget): Promise<string> {
  return client().renderSyntaxDisplay(text, from, to);
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
/// `exact` selects the calculator's forced-rational mode (a bare numeric literal grounds to Rational, so
/// `1 / 3` is `1/3`); the playground passes false (ordinary defaults). Defaults false.
export function replEval(
  buffer: string,
  expr: string,
  from: Surface,
  exact = false,
): Promise<CompileOutcome> {
  return client().replEval(buffer, expr, from, exact);
}

/// The names of every top-level definition the buffer declares — for the REPL's autocomplete.
export function definedNames(buffer: string, from: Surface): Promise<string[]> {
  return client().definedNames(buffer, from);
}

export function runtimeHash(): Promise<string> {
  return client().runtimeHash();
}

/// The program's exported names + solved types (`name<TAB>type` lines) — used by the run path to render
/// a whole-number Float scalar with its `.0` (see `runner/scalarFormat.ts`).
export function exportTypes(text: string, from: Surface): Promise<string> {
  return client().exportTypes(text, from);
}

export type { CompileOutcome, TestCompileOutcome, Surface, Diag, DiagFix, TypeAtInfo, DefineAtInfo, SemanticTok, DispositionInfo };
