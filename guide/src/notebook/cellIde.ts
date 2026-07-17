/// The per-cell IDE `prepare` for a notebook code cell — the seam that makes the editor's language
/// service (squiggles / hover / semantic tokens) CORRECT for a cell edited in the stacked per-cell layout.
///
/// THE PROBLEM it solves (notebook IDE #13): a notebook is markdown interleaved with code cells, and code
/// cells share SEQUENTIAL scope (each cell sees the prior cells' defs + the widget bindings — `assembleCell`
/// / `assembleForRun`). A single whole-doc editor fed to the Cadenza language service mis-lints EVERYTHING
/// (prose isn't Cadenza; a cell's names resolve against a sibling cell's scope, not the concatenation). The
/// fix is a per-cell editor whose linter compiles the cell IN CONTEXT and maps diagnostics back onto the
/// cell's own text.
///
/// HOW: `CodeEditor`'s `IdeConfig.prepare(editorText, surface) -> { compiled, wrapPrefixBytes }` already IS
/// the span-mapping seam — the linter compiles `compiled` and SUBTRACTS `wrapPrefixBytes` (UTF-8) to map a
/// compiled-text offset back to the editor text, dropping any diagnostic outside the editor's own range
/// (see `playground/cadenzaLint.ts`). So a cell's `prepare` returns:
///   compiled       = <widget bindings> + <prior code cells' sources> + <this cell's live text>
///   wrapPrefixBytes = UTF-8 byte length of everything BEFORE this cell's text
/// A diagnostic in the cell maps back onto the cell editor; a diagnostic in the prior-context prefix falls
/// outside `[0, cellText)` and is dropped. This diagnoses the cell against its REAL sequential scope — no
/// new editor primitive, purely a `prepare` over the existing (tested) `assembleCell` scope model.
///
/// PURE (no React / worker / compiler imports) so it is unit-testable under `node --test`, mirroring
/// `assembleCell` / `assembleForRun`.

import type { Cell } from "./parseDocument.ts";
import type { Surface } from "../compiler/worker.ts";
import type { Widget } from "./parseWidgets.ts";
import type { WidgetValues } from "./assembleForRun.ts";
import { assembleCell } from "./assembleCell.ts";
import { widgetBinding } from "./assembleForRun.ts";

/// The UTF-8 byte length of a string (the unit `wrapPrefixBytes` is measured in — the compiler reports
/// byte offsets, and `cadenzaLint` maps them back by subtracting this prefix).
function utf8Len(s: string): number {
  return new TextEncoder().encode(s).length;
}

/// Build the `prepare` output for linting the code cell at `index`: its live `cellText` compiled with the
/// prior-cell scope + widget bindings prepended, and the byte length of that prefix so diagnostics map back
/// onto the cell's own text. `widgets`/`values` supply the in-scope widget bindings (a widget's live value,
/// falling back to its default) — the same bindings `assembleForRun` folds in, so the linter sees exactly
/// the names a run sees. Mirrors `assembleForRun`'s buffer assembly, but keeps THIS cell's text LAST and
/// un-wrapped (it's the editor content whose spans we map), rather than moving it into the buffer.
export function prepareCell(
  cells: Cell[],
  index: number,
  widgets: Widget[],
  values: WidgetValues,
  surface: Surface,
  cellText: string,
): { compiled: string; wrapPrefixBytes: number } {
  const { buffer: scopeBuffer } = assembleCell(cells, index, surface);
  const widgetBindings = widgets
    .map((w) => widgetBinding(w, values[w.name] ?? w.default, surface))
    .join("\n");

  // The prefix is the in-scope context (widget bindings + prior cells' sources); the cell's own live text
  // comes AFTER it. Only non-empty parts join, with blank lines between (both surfaces accept
  // newline-separated top-level forms — the `assembleForRun` convention).
  const prefixParts = [widgetBindings, scopeBuffer].filter((s) => s.trim().length > 0);
  const prefix = prefixParts.length > 0 ? prefixParts.join("\n\n") + "\n\n" : "";
  return { compiled: prefix + cellText, wrapPrefixBytes: utf8Len(prefix) };
}

/// The full `IdeConfig` for a notebook code cell's editor — `surface` fixed to the notebook surface, and a
/// `prepare` bound to this cell's index + the current cells/widgets/values. The caller (NotebookPage)
/// rebuilds this when the cell list / widget values change (cheap — it's a closure), so the linter always
/// diagnoses against the current scope. `editorText` (the live cell buffer) is the text the linter passes,
/// so the diagnosed cell text tracks keystrokes.
export function cellIde(
  cells: Cell[],
  index: number,
  widgets: Widget[],
  values: WidgetValues,
  surface: Surface,
): { surface: () => Surface; prepare: (editorText: string, s: Surface) => { compiled: string; wrapPrefixBytes: number } } {
  return {
    surface: () => surface,
    prepare: (editorText: string) => prepareCell(cells, index, widgets, values, surface, editorText),
  };
}
