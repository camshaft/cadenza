/// The compile worker. Runs the Cadenza compiler (cdz-wasm) off the UI thread so a compile never
/// janks the page. Exposes a small Comlink API the main thread calls with `await`.
///
/// The compiler is pure and fast for guide-sized snippets, so this worker is long-lived and reused
/// (unlike the RUN worker, which is disposable so an infinite loop can be killed). It never executes
/// user output — it only turns source text into a component, type-checks it, and re-renders syntax.

import * as Comlink from "comlink";
import init, {
  compile as wasmCompile,
  compile_with_preloaded as wasmCompileWithPreloaded,
  compile_tests as wasmCompileTests,
  param_test_signatures as wasmParamTestSignatures,
  param_manifest as wasmParamManifest,
  diagnostics as wasmDiagnostics,
  diagnostics_with_preloaded as wasmDiagnosticsWithPreloaded,
  type_at as wasmTypeAt,
  define_at as wasmDefineAt,
  references_at as wasmReferencesAt,
  semantic_tokens as wasmSemanticTokens,
  semantic_tokens_with_preloaded as wasmSemanticTokensWithPreloaded,
  disposition as wasmDisposition,
  emit_rust as wasmEmitRust,
  emit_cadenza as wasmEmitCadenza,
  core_module as wasmCoreModule,
  repl_eval as wasmReplEval,
  defined_names as wasmDefinedNames,
  render_syntax as wasmRenderSyntax,
  render_syntax_display as wasmRenderSyntaxDisplay,
  render_value as wasmRenderValue,
  render_binary as wasmRenderBinary,
  required_runtime_hash as wasmRuntimeHash,
  export_types as wasmExportTypes,
} from "../wasm/pkg/cdz_wasm.js";
// The wasm binary as a URL Vite fingerprints; `--target web` init fetches + compiles it.
import wasmUrl from "../wasm/pkg/cdz_wasm_bg.wasm?url";
// Pure boundary guard: the three preload arrays must be equal length (see preloadArity.ts). Kept in a
// wasm-free module so `node --test` can pin it (preloadArity.test.ts); worker.ts just composes it.
import { preloadArityError } from "./preloadArity.ts";

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

/** A test-layout compile (`compileTests`): the component (boundary = the `@test` defs), diagnostics, and
 *  the discovered test names split into nullary (run now) and parameterized (deferred property tests). */
export interface TestCompileOutcome {
  component: Uint8Array | null;
  diagnostics: Diag[];
  nullaryTestNames: string[];
  paramTestNames: string[];
}

/**
 * One parameterized `@test`'s signature, for the property-test driver. `compound: false` = a scalar-param
 * test whose params are on the export (the scalar driver generates a JS arg per `paramTypes` entry and calls
 * the export); `compound: true` = a synthesized nullary `-gen` wrapper (a compound param built guest-side by
 * consuming a `Test.gen-int` pool) which the COMPOUND driver runs over that pool. `paramTypes` is a stable
 * lowercase enum per param (`int8`..`uint64`|`bool`|`float32`|`float64`|`other`); empty for a `-gen` wrapper.
 */
export interface ParamTestSig {
  name: string;
  paramTypes: string[];
  compound: boolean;
}

/** One `@param` site's widget metadata, from `param_manifest` — what a parametric host (/cad single-mode)
 *  renders a control from. `typeName` is always present (the declared, reduced type, e.g. `Int64` /
 *  `Rational` / `(Qty Rational meter)`); the rest are the `@param`'s config, undefined when omitted.
 *  `rangeLo`/`rangeHi`/`default` are rendered as STRINGS (an exact Rational default like `1/4` survives —
 *  the host parses per type). The exact value crosses at RUN time via the `Param.<name>-num/-den` pair. */
export interface ParamManifestEntry {
  name: string;
  typeName: string;
  widget?: string;
  rangeLo?: string;
  rangeHi?: string;
  default?: string;
  /** EXACT bound/default as a num/den pair (strings — BigInt them), present for a RATIONAL `@param` (from
   *  the compiler's Core::ConstRational, gcd-reduced). Undefined for an Int64 param (read the string fields)
   *  or when the config omits that bound. Lets /cad's fraction sliders carry the exact value (7/2), not a
   *  parsed float. `typeName === "Rational"` (or a Qty) discriminates. */
  rangeLoNum?: string;
  rangeLoDen?: string;
  rangeHiNum?: string;
  rangeHiDen?: string;
  defaultNum?: string;
  defaultDen?: string;
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
///
/// DEFENSIVE BACKSTOP for a deep-recursion overflow in the WORKER: the browser compiles in a Worker, whose
/// JS/wasm stack is smaller than the main thread's, so a very deep compiler recursion can overflow the worker
/// stack ("Maximum call stack size exceeded") or trip a wasm "memory access out of bounds" for a program that
/// compiles cleanly natively + on the main thread. We surface that as a CLEAN, actionable diagnostic instead
/// of a scary raw runtime error.
///
/// NOTE: the original trigger — the module-qualified-call P0 — is FIXED (v-inference's `arrow_lambdas_in_progress`
/// re-entry guard dropped resolution depth from 1000+ to ~5; the guide's module examples now compile+run in the
/// worker, gated by the worker-conformance case in `check-examples.mjs`). This backstop stays only for
/// GENUINELY pathological programs (e.g. an extremely deeply-nested user construction) — it's no longer a known
/// bug, so the message no longer promises a fix or blames module-qualified forms.
function parseErrorDiag(e: unknown): Diag {
  const message = e instanceof Error ? e.message : String(e);
  if (/maximum call stack|out of bounds|stack overflow|recursion/i.test(message)) {
    return {
      error: true,
      code: "",
      message:
        "the browser compiler ran out of stack on this program (an unusually deeply-nested form). " +
        "Try simplifying it, or compile it with the `cdz` CLI, which has a larger stack.",
      node: 0,
      from: 0,
      to: 0,
      fix: null,
    };
  }
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

  /// Compile `text` with PRELOADED library modules link-merged (`compile_with_preloaded`): the three
  /// parallel arrays declare each preloaded module's name / source / surface. `text` can `import` from a
  /// preloaded module by name and the compiler links against the supplied source instead of requiring the
  /// module in-tree. /cad uses this to compile a bare model buffer against the preloaded CAD library
  /// (Solid.*/Vec3.*/v3r) — the reader edits only the model, the vocab is preloaded. Same parse-error
  /// handling as `compile` (a syntax error → a decline diagnostic, not a promise rejection).
  async compileWithPreloaded(
    text: string,
    from: Surface,
    names: string[],
    sources: string[],
    formats: string[],
  ): Promise<CompileOutcome> {
    await ensureReady();
    const arity = preloadArityError(names, sources, formats);
    if (arity) return { component: null, diagnostics: [arity] };
    let r: ReturnType<typeof wasmCompileWithPreloaded>;
    try {
      r = wasmCompileWithPreloaded(text, from, names, sources, formats);
    } catch (e) {
      return { component: null, diagnostics: [parseErrorDiag(e)] };
    }
    const component = r.component ? new Uint8Array(r.component) : null;
    const diagnostics: Diag[] = r.diagnostics.map(toDiag);
    return { component, diagnostics };
  },

  /// Compile `text` in TEST-LAYOUT mode (`compile_tests`): the component's boundary is the program's
  /// `@test` defs, so each `@test` is an invocable export. Returns the component + diagnostics + the
  /// discovered test names (nullary = run now, param = deferred property tests). The run worker then
  /// invokes each nullary export to report pass/fail. Same parse-error handling as `compile`.
  async compileTests(text: string, from: Surface): Promise<TestCompileOutcome> {
    await ensureReady();
    let r: ReturnType<typeof wasmCompileTests>;
    try {
      r = wasmCompileTests(text, from);
    } catch (e) {
      return { component: null, diagnostics: [parseErrorDiag(e)], nullaryTestNames: [], paramTestNames: [] };
    }
    const component = r.component ? new Uint8Array(r.component) : null;
    const diagnostics: Diag[] = r.diagnostics.map(toDiag);
    return {
      component,
      diagnostics,
      nullaryTestNames: r.nullary_test_names,
      paramTestNames: r.param_test_names,
    };
  },

  /// The signatures of every PARAMETERIZED `@test` in `text` (`param_test_signatures`) — the metadata the
  /// property-test driver needs to generate inputs: each param test's name, its scalar param types (for the
  /// arg-driver), and whether it's a `-gen` wrapper (`compound` → the gen-int-pool driver). A parse error / no
  /// `@test` yields an empty list (this is metadata, not a compile — an unparseable buffer just has none).
  async paramTestSignatures(text: string, from: Surface): Promise<ParamTestSig[]> {
    await ensureReady();
    let sigs: ReturnType<typeof wasmParamTestSignatures>;
    try {
      sigs = wasmParamTestSignatures(text, from);
    } catch {
      return [];
    }
    return sigs.map((s) => ({ name: s.name, paramTypes: s.param_types, compound: s.compound }));
  },

  /// The `@param` WIDGET MANIFEST of `text` (`param_manifest`) — one entry per `@param` site the model
  /// declares, so /cad single-mode renders a slider per param a model itself declares (not a hardcoded
  /// list). `widget`/`range_lo`/`range_hi`/`default` are `undefined` (optional wasm-bindgen fields) when the
  /// `@param` omits that config; the snake_case rendered strings map to camelCase here. A parse error / no
  /// `@param` yields an empty list (metadata, not a compile — an unparseable buffer just has no params).
  async paramManifest(text: string, from: Surface): Promise<ParamManifestEntry[]> {
    await ensureReady();
    let entries: ReturnType<typeof wasmParamManifest>;
    try {
      entries = wasmParamManifest(text, from);
    } catch {
      return [];
    }
    return entries.map((e) => ({
      name: e.name,
      typeName: e.type_name,
      widget: e.widget ?? undefined,
      rangeLo: e.range_lo ?? undefined,
      rangeHi: e.range_hi ?? undefined,
      default: e.default ?? undefined,
      // Exact num/den (Rational params) — snake→camel; undefined for Int64 / when the config omits the bound.
      rangeLoNum: e.range_lo_num ?? undefined,
      rangeLoDen: e.range_lo_den ?? undefined,
      rangeHiNum: e.range_hi_num ?? undefined,
      rangeHiDen: e.range_hi_den ?? undefined,
      defaultNum: e.default_num ?? undefined,
      defaultDen: e.default_den ?? undefined,
    }));
  },

  /// Type-check `text` and return all diagnostics with source spans — no component built, no export
  /// required. The as-you-type entry.
  async diagnostics(text: string, from: Surface): Promise<Diag[]> {
    await ensureReady();
    return wasmDiagnostics(text, from).map(toDiag);
  },

  /// Type-check `text` with PRELOADED library modules link-merged (`diagnostics_with_preloaded`), the
  /// as-you-type sibling of `compileWithPreloaded`. The three parallel arrays declare each preloaded
  /// module's name / source / surface; `text` can `import` from a preloaded module by name and resolves
  /// against the supplied source. Faults are returned over the USER text spans (a fault inside a preloaded
  /// library is dropped), so a squiggle lands in the buffer the reader edits — /cad lints a bare model
  /// against the preloaded CAD library without the preloaded vocab (`Solid`/`v3r`/`lower`) showing as
  /// unbound. Empty arrays are byte-identical to plain `diagnostics`.
  async diagnosticsWithPreloaded(
    text: string,
    from: Surface,
    names: string[],
    sources: string[],
    formats: string[],
  ): Promise<Diag[]> {
    await ensureReady();
    const arity = preloadArityError(names, sources, formats);
    if (arity) return [arity];
    return wasmDiagnosticsWithPreloaded(text, from, names, sources, formats).map(toDiag);
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

  /// Semantic tokens with PRELOADED library modules link-merged (`semantic_tokens_with_preloaded`), the
  /// highlighting sibling of `diagnosticsWithPreloaded`. Classifies the whole linked package so a name
  /// resolving into a preloaded library colours as the function/type/ctor it truly is (/cad's `Solid`/
  /// `v3r`/`lower` stop rendering blank/unbound); tokens are demuxed back to the USER text spans and
  /// library-internal tokens dropped. Empty arrays are byte-identical to plain `semanticTokens`.
  async semanticTokensWithPreloaded(
    text: string,
    from: Surface,
    names: string[],
    sources: string[],
    formats: string[],
  ): Promise<SemanticTok[]> {
    await ensureReady();
    return wasmSemanticTokensWithPreloaded(text, from, names, sources, formats).map((t) => ({
      from: t.from,
      to: t.to,
      kind: t.kind,
    }));
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

  // Render a canonical binary-AST to a surface, per fragment kind — the render-from-binary path (no text
  // re-parse). Used by inline <Cadenza> for the (cdz …) tag's embedded AST.
  async renderBinary(bytes: Uint8Array, to: string, kind: string): Promise<string> {
    await ensureReady();
    return wasmRenderBinary(bytes, to, kind);
  },

  /// Emit the program as Rust source — sync or (gas-metered) async — for the "Compiled" output views.
  async emitRust(text: string, from: Surface, isAsync: boolean): Promise<string> {
    await ensureReady();
    return wasmEmitRust(text, from, isAsync);
  },

  /// Emit the program's lowered-optimized CADENZA source (`--target cadenza`) in `syntax` (sexpr/ml) — for
  /// the "Compiled" output views. A program the backend declines returns a `; declined: …` note verbatim.
  async emitCadenza(text: string, from: Surface, syntax: Surface): Promise<string> {
    await ensureReady();
    return wasmEmitCadenza(text, from, syntax);
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

  /// The program's exported names paired with their solved types, as `name<TAB>type` lines. The run
  /// path uses `main`'s type to render a whole-number Float scalar with its `.0` (jco lowers it to a
  /// bare JS number that would otherwise print as an int). Empty when nothing parses / no exports.
  async exportTypes(text: string, from: Surface): Promise<string> {
    await ensureReady();
    return wasmExportTypes(text, from);
  },
};

export type CompilerApi = typeof api;
Comlink.expose(api);
