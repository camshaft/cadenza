/// The lint extension bundle for the editor — diagnostics as you type.
///
/// We use `@codemirror/lint`'s async `linter()` source (it owns debouncing via `delay`, and draws
/// both the inline squiggles and, with `lintGutter()`, the margin markers). The source calls the
/// compile worker for the current buffer's diagnostics, maps them to CodeMirror ranges, and also
/// hands the raw list to `onDiagnostics` (for the Diagnostics tab + status counts). We deliberately
/// don't also push via `setDiagnostics` — a `linter()` source and manual pushes fight over the same
/// lint state; the source is the single owner.

import { linter, lintGutter, type Diagnostic as CmDiagnostic } from "@codemirror/lint";
import type { Extension } from "@codemirror/state";
import {
  diagnostics as workerDiagnostics,
  diagnosticsWithPreloaded as workerDiagnosticsWithPreloaded,
  type Diag,
  type Surface,
} from "../compiler/client.ts";
import { toCmDiagnostics } from "./cadenzaLint.ts";

export { lintGutter };

export interface LinterContext {
  /** The surface the editor text is currently in. */
  surface: () => Surface;
  /** Wrap the editor text into a compilable module + report the wrapper's UTF-8 byte length, so spans
   *  map back to the editor text. Identity (no wrap) for a playground buffer already a full module. */
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
  /** Optional PRELOADED library modules (three parallel name/source/format arrays) link-merged for
   *  diagnostics. When set, the compiled text is type-checked with `diagnosticsWithPreloaded` so a buffer
   *  importing a preloaded module (e.g. /cad's model) doesn't fault the preloaded vocab as unbound. */
  preload?: () => { names: string[]; sources: string[]; formats: string[] };
  /** Receive each fresh raw diagnostics set (for a Diagnostics tab / status counts). */
  onDiagnostics?: (diags: Diag[]) => void;
}

/// The Cadenza linter extension (async worker source). Debounced by CodeMirror (`delay`).
export function cadenzaLinter(ctx: LinterContext): Extension {
  return linter(
    async (view): Promise<CmDiagnostic[]> => {
      const editorText = view.state.doc.toString();
      const surface = ctx.surface();
      const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
      const preload = ctx.preload?.();
      let diags: Diag[];
      try {
        diags =
          preload && preload.names.length
            ? await workerDiagnosticsWithPreloaded(compiled, surface, preload.names, preload.sources, preload.formats)
            : await workerDiagnostics(compiled, surface);
      } catch {
        return [];
      }
      ctx.onDiagnostics?.(diags);
      const editorBytes = new TextEncoder().encode(editorText).length;
      return toCmDiagnostics(diags, { editorText, surface, wrapPrefixBytes, editorBytes });
    },
    { delay: 300 },
  );
}
