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
  semantic_tokens as wasmSemanticTokens,
  disposition as wasmDisposition,
  emit_rust as wasmEmitRust,
  core_module as wasmCoreModule,
  repl_eval as wasmReplEval,
  defined_names as wasmDefinedNames,
  render_syntax as wasmRenderSyntax,
  render_syntax_display as wasmRenderSyntaxDisplay,
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
  /**
   * A proposed structural repair, or null when the compiler has no actionable suggestion. The fix
   * edits the target byte range `[fix.from, fix.to)` (UTF-8, over the SAME compiled text as
   * `from`/`to`) per `fix.kind` — see `DiagFix`.
   */
  fix: DiagFix | null;
}

/** A proposed structural repair carried by a diagnostic (`spec/capabilities/diagnostics.md`). */
export interface DiagFix {
  /**
   * How to apply `replacement` at `[from, to)`:
   *   - `"replace"` — replace the range with `replacement`;
   *   - `"insert"`  — insert `replacement` (rendered child forms, e.g. missing match arms) just
   *                   before `to` (the end of the target list, before its closing paren);
   *   - `"wrap"`    — replace the range with `replacement`, in which the char `…` (U+2026) marks
   *                   where the range's ORIGINAL text goes (`(Some …)` → `(Some <expr>)`).
   */
  kind: string;
  /** The surface payload (the spelling / child forms / wrap template). */
  replacement: string;
  /** Target byte range [from, to) (UTF-8, over the compiled text). */
  from: number;
  to: number;
  /** true = compiler-proven (machine-applicable); false = a heuristic the user should confirm. */
  verified: boolean;
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

/**
 * One semantically-classified token — a source byte range plus the ROLE the compiler assigned it
 * (`type`/`constructor`/`function`/`param`/`variable`/`effect`/`label`/`keyword`/`number`/`string`/
 * `char`/`bytes`/`symbol`/`literal`/`unbound`). The editor maps `kind` to a colour. Byte offsets.
 */
export interface SemanticTok {
  from: number;
  to: number;
  kind: string;
}

/**
 * How the compiler compiled the definition under the cursor — for a "what did the compiler do?" hover.
 * `disposition` is `inlined` / `specialized` / `emitted` / `transformed→COPY` / `unreferenced` (a
 * `+`-joined set when several apply); `instances` lists each concrete monomorphization (only for
 * `specialized`), each an `arg, arg, …` string. `from`/`to` are the definition name's byte range.
 */
export interface DispositionInfo {
  name: string;
  disposition: string;
  instances: string[];
  from: number;
  to: number;
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
  fix_kind: string;
  fix_replacement: string;
  fix_from: number;
  fix_to: number;
  fix_verified: boolean;
}): Diag {
  // The wasm `Diagnostic` carries the fix flattened into `fix_*` columns (empty `fix_kind` = no fix).
  const fix: DiagFix | null = d.fix_kind
    ? {
        kind: d.fix_kind,
        replacement: d.fix_replacement,
        from: d.fix_from,
        to: d.fix_to,
        verified: d.fix_verified,
      }
    : null;
  return { error: d.error, code: d.code, message: d.message, node: d.node, from: d.from, to: d.to, fix };
}

/// A parse-error throw (a `JsError` from `wasmCompile`/`wasmReplEval` on unparseable text) as a
/// decline diagnostic, so a syntax error reads like any other "no" instead of rejecting the promise.
/// Unanchored (byte 0) — the message carries the byte offset the wasm reported.
function parseErrorDiag(e: unknown): Diag {
  const message = e instanceof Error ? e.message : String(e);
  return { error: true, code: "", message, node: 0, from: 0, to: 0, fix: null };
}

const api = {
  async compile(text: string, from: Surface): Promise<CompileOutcome> {
    await ensureReady();
    // `wasmCompile` THROWS (a JsError) when the text doesn't even parse — a syntax error, not a
    // type/semantic decline. Catch it and surface it as a normal decline diagnostic so the caller
    // shows the error rather than the promise rejecting (which would leave a Run stuck on
    // "Compiling & running…" forever, since the caller doesn't set a done state on a rejection).
    let r: ReturnType<typeof wasmCompile>;
    try {
      r = wasmCompile(text, from);
    } catch (e) {
      return { component: null, diagnostics: [parseErrorDiag(e)] };
    }
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

  /// Semantic syntax-highlight tokens for the whole buffer — each a byte range + a role the compiler
  /// classified (see `SemanticTok`). Empty when the buffer doesn't parse (the editor keeps its lexical
  /// colours). The editor overlays these on top of the fast lexical tokenizer.
  async semanticTokens(text: string, from: Surface): Promise<SemanticTok[]> {
    await ensureReady();
    return wasmSemanticTokens(text, from).map((t) => ({ from: t.from, to: t.to, kind: t.kind }));
  },

  /// The compilation disposition of the definition whose name is at a UTF-8 byte offset (for a hover),
  /// or null when the offset isn't on a definition name. Rides the `Instantiations` sidecar query.
  async disposition(text: string, from: Surface, byteOffset: number): Promise<DispositionInfo | null> {
    await ensureReady();
    const d = wasmDisposition(text, from, byteOffset);
    return d
      ? { name: d.name, disposition: d.disposition, instances: [...d.instances], from: d.from, to: d.to }
      : null;
  },

  // `to` may be a surface or an output-only view ("debug"/"flat"); the wasm accepts the wider set.
  async renderSyntax(text: string, from: Surface, to: string): Promise<string> {
    await ensureReady();
    return wasmRenderSyntax(text, from, to);
  },

  // Render for human DISPLAY (a calculator result): a rational bare, a quantity in its concise
  // `<value> <unit>` surface, the result type annotation dropped. Non-round-tripping by design; the
  // playground uses `renderSyntax` (canonical) instead.
  async renderSyntaxDisplay(text: string, from: Surface, to: string): Promise<string> {
    await ensureReady();
    return wasmRenderSyntaxDisplay(text, from, to);
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

  /// Evaluate a REPL `expr` against the `buffer`'s definitions — the playground's mini-REPL. Returns a
  /// CompileOutcome (component + diagnostics) exactly like `compile`, so the caller runs the component
  /// through the same run worker. The buffer's exports are dropped; the expression becomes the sole
  /// entry, so a scalar OR compound result flows through the normal run path.
  async replEval(buffer: string, expr: string, from: Surface, exact = false): Promise<CompileOutcome> {
    await ensureReady();
    // Like `compile`, `wasmReplEval` throws on unparseable buffer/expr — surface it as a decline so a
    // syntax error in a REPL entry doesn't reject the promise (which would hang the REPL call).
    // `exact` selects the calculator's forced-rational mode (a bare `1 / 3` is `1/3`); the playground
    // passes false.
    let r: ReturnType<typeof wasmReplEval>;
    try {
      r = wasmReplEval(buffer, expr, from, exact);
    } catch (e) {
      return { component: null, diagnostics: [parseErrorDiag(e)] };
    }
    const component = r.component ? new Uint8Array(r.component) : null;
    return { component, diagnostics: r.diagnostics.map(toDiag) };
  },

  /// The names of every top-level definition the buffer declares — for the REPL's autocomplete.
  async definedNames(buffer: string, from: Surface): Promise<string[]> {
    await ensureReady();
    return wasmDefinedNames(buffer, from);
  },

  async runtimeHash(): Promise<string> {
    await ensureReady();
    return wasmRuntimeHash();
  },
};

export type CompilerApi = typeof api;
Comlink.expose(api);
