/// Map Cadenza compiler diagnostics (byte spans over the COMPILED text) to CodeMirror lint
/// `Diagnostic`s (UTF-16 offsets over the EDITOR text).
///
/// The compiled text may differ from the editor text when a bare snippet is wrapped into a module
/// before compiling (as the inline `Runnable` does). `wrapPrefixBytes` is the UTF-8 byte length of
/// the wrapper text inserted BEFORE the editor content; we subtract it to map a compiled-text byte
/// offset back to an editor-text byte offset, then convert to UTF-16. A diagnostic that falls outside
/// the editor content (i.e. inside the wrapper) is clamped to the nearest editor edge or dropped.

import type { Diagnostic as CmDiagnostic } from "@codemirror/lint";
import type { EditorView } from "@codemirror/view";
import type { Diag, Surface } from "../compiler/client.ts";
import { byteToUtf16 } from "./offsets.ts";
import { applyFix, fixConfidence, fixIsApplicable } from "./applyFix.ts";

export interface LintMapping {
  /** The exact text shown in the editor. */
  editorText: string;
  /** The surface the editor text is in — gates the quick-fix action (see `fixIsApplicable`). */
  surface: Surface;
  /** UTF-8 byte length of the wrapper prefix before the editor text (0 when the editor text was
   *  compiled verbatim, e.g. the playground where the buffer is already a full module). */
  wrapPrefixBytes?: number;
  /** UTF-8 byte length of the editor content (to clamp/drop diagnostics that land in the suffix). */
  editorBytes?: number;
}

export function toCmDiagnostics(diags: Diag[], m: LintMapping): CmDiagnostic[] {
  const prefix = m.wrapPrefixBytes ?? 0;
  const editorBytes = m.editorBytes ?? byteLen(m.editorText);
  const out: CmDiagnostic[] = [];
  for (const d of diags) {
    // Unanchored (from==to==0 with no code region) → attach to the document start so it's still seen.
    let fromByte = d.from - prefix;
    let toByte = d.to - prefix;
    if (d.from === 0 && d.to === 0) {
      fromByte = 0;
      toByte = 0;
    }
    // Drop a diagnostic wholly inside the wrapper (before the content) — it's about generated glue.
    if (toByte < 0) continue;
    // Clamp into the editor content range.
    fromByte = Math.max(0, Math.min(fromByte, editorBytes));
    toByte = Math.max(0, Math.min(toByte, editorBytes));
    const from = byteToUtf16(m.editorText, fromByte);
    let to = byteToUtf16(m.editorText, toByte);
    if (to <= from) to = Math.min(from + 1, m.editorText.length); // ensure a non-empty mark
    const cm: CmDiagnostic = {
      from,
      to,
      severity: d.error ? "error" : "warning",
      source: "cadenza",
      message: d.code ? `${d.code}: ${d.message}` : d.message,
    };
    // A structural fix becomes a quick-fix ACTION (CodeMirror's lightbulb): applying it splices the
    // repair into the buffer. Offered only when the fix has a usable target range (see
    // `fixIsApplicable`). The `apply` recomputes the whole editor text via the shared `applyFix` and
    // replaces the document, so a multi-byte edit stays offset-exact.
    if (d.fix && fixIsApplicable(d.fix, m.surface)) {
      const fix = d.fix;
      cm.actions = [
        {
          name: `Fix: ${fixLabel(d)} (${fixConfidence(fix)})`,
          apply(view: EditorView) {
            const next = applyFix(view.state.doc.toString(), fix, prefix);
            if (next == null) return;
            view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
          },
        },
      ];
    }
    out.push(cm);
  }
  return out;
}

/// A concise action label for a fix — the concrete edit, so the lightbulb reads like an intent
/// ("replace with `at`", "wrap in `(Some …)`", "add the missing arms"). Derived from the fix kind +
/// payload since the compiler's prose label isn't carried over the wasm ABI.
function fixLabel(d: Diag): string {
  const fix = d.fix!;
  switch (fix.kind) {
    case "wrap":
      return `wrap in \`${fix.replacement}\``;
    case "insert":
      return "add the missing arms";
    default:
      return `replace with \`${fix.replacement}\``;
  }
}

function byteLen(str: string): number {
  return new TextEncoder().encode(str).length;
}
