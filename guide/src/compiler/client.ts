/// Main-thread handle to the compile worker. A single shared worker instance wrapped with Comlink,
/// so any component can `await compiler.compile(...)` as if it were a local async function.

import * as Comlink from "comlink";
import type { CompilerApi, CompileOutcome, TestCompileOutcome, ParamTestSig, ParamManifestEntry, Diag, DiagFix, Surface, TypeAtInfo, DefineAtInfo, SemanticTok, DispositionInfo } from "./worker.ts";

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

/// Compile with PRELOADED library modules link-merged (parallel name/source/format arrays). `text` may
/// `import` a preloaded module by name; the compiler links the supplied source. /cad compiles a bare model
/// buffer against the preloaded CAD library so the reader edits only the model.
export function compileWithPreloaded(
  text: string,
  from: Surface,
  names: string[],
  sources: string[],
  formats: string[],
): Promise<CompileOutcome> {
  return client().compileWithPreloaded(text, from, names, sources, formats);
}

/// Compile in TEST-LAYOUT mode: each `@test` becomes an invocable export. Returns component + diagnostics
/// + the discovered test names (nullary vs parameterized). The run worker invokes the nullary exports.
export function compileTests(text: string, from: Surface): Promise<TestCompileOutcome> {
  return client().compileTests(text, from);
}

/// The signatures of every PARAMETERIZED `@test` in `text` — for the property-test driver. Each carries the
/// test name, its scalar param types (arg-driver), and `compound` (a `-gen` wrapper → the gen-int-pool driver).
export function paramTestSignatures(text: string, from: Surface): Promise<ParamTestSig[]> {
  return client().paramTestSignatures(text, from);
}

/// The `@param` widget manifest of `text` — one entry per `@param` the model declares (name, reduced type,
/// widget/range/default). /cad single-mode reads this on each recompile to auto-surface a slider per param
/// the model itself declares. Empty for a param-free or unparseable buffer.
export function paramManifest(text: string, from: Surface): Promise<ParamManifestEntry[]> {
  return client().paramManifest(text, from);
}

/// Type-check without building a component (as-you-type diagnostics). No export required upstream.
export function diagnostics(text: string, from: Surface): Promise<Diag[]> {
  return client().diagnostics(text, from);
}

/// Type-check with PRELOADED library modules link-merged (as-you-type sibling of `compileWithPreloaded`).
/// Faults map to the USER text spans (faults inside a preloaded library are dropped). /cad lints a bare
/// model against the preloaded CAD library so the preloaded vocab doesn't show as unbound.
export function diagnosticsWithPreloaded(
  text: string,
  from: Surface,
  names: string[],
  sources: string[],
  formats: string[],
): Promise<Diag[]> {
  return client().diagnosticsWithPreloaded(text, from, names, sources, formats);
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

/// Semantic tokens with PRELOADED library modules link-merged (highlighting sibling of
/// `diagnosticsWithPreloaded`). A name resolving into a preloaded library colours as what it truly is;
/// tokens map to the USER text spans. /cad highlights the preloaded CAD vocab (`Solid`/`v3r`/`lower`).
export function semanticTokensWithPreloaded(
  text: string,
  from: Surface,
  names: string[],
  sources: string[],
  formats: string[],
): Promise<SemanticTok[]> {
  return client().semanticTokensWithPreloaded(text, from, names, sources, formats);
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

/// Render a canonical binary-AST (bytes) to a surface, per fragment KIND (`expr`/`type`/`pattern`).
/// Decodes the AST + prints via the canonical per-surface printer — the render-from-binary path (cdz-wasm
/// `render_binary`), so a stored AST toggles surfaces with NO text re-parse. Used by inline `<Cadenza>`
/// (the `(cdz …)` tag's embedded AST). `kind` defaults to `expr` (the only fully-idiomatic kind today;
/// type/pattern render faithfully but not yet in idiomatic type/pattern position — v-syntax-render-ty).
export function renderBinary(bytes: Uint8Array, to: Surface, kind: string = "expr"): Promise<string> {
  return client().renderBinary(bytes, to, kind);
}

/// Emit the program as Rust source (sync, or gas-metered async) — for the playground's output views.
export function emitRust(text: string, from: Surface, isAsync: boolean): Promise<string> {
  return client().emitRust(text, from, isAsync);
}

/// Emit the program's lowered-optimized CADENZA source (`--target cadenza`) in `syntax` — for the playground's
/// output views. Returns a `; declined: …` note verbatim when the cadenza backend declines the program.
export function emitCadenza(text: string, from: Surface, syntax: Surface): Promise<string> {
  return client().emitCadenza(text, from, syntax);
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

export type { CompileOutcome, TestCompileOutcome, ParamTestSig, ParamManifestEntry, Surface, Diag, DiagFix, TypeAtInfo, DefineAtInfo, SemanticTok, DispositionInfo };
