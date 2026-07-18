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
import { widgetBinding, defaultFractionPragma } from "./assembleForRun.ts";
import { exportNames, topLevelDefNames } from "../components/wrapModule.ts";

/// The UTF-8 byte length of a string (the unit `wrapPrefixBytes` is measured in — the compiler reports
/// byte offsets, and `cadenzaLint` maps them back by subtracting this prefix).
function utf8Len(s: string): number {
  return new TextEncoder().encode(s).length;
}

/// A top-level `export` form for a cell's definitions, appended as a SUFFIX after the cell text so the
/// linted module has a public entry. Without it, `compile` declines the cell with "no `(export …)`:
/// nothing is public" — an UNANCHORED (from=to=0) diagnostic that `cadenzaLint` pins to the cell's start,
/// so the operator sees the cell (and its `main`) flagged as unused/non-public even though the RUN path
/// (`repl_eval`, which roots the entry implicitly) compiles it clean (operator UX #1). The export names
/// `main` when the cell defines it (the guide convention), else the cell's own top-level defs — the same
/// rule `wrapModule` uses. Empty when the cell declares nothing to export (a bare expression / prose-only
/// edit) so we don't emit a dangling `(export)`. The suffix sits AFTER the cell text, so it never shifts
/// `wrapPrefixBytes`, and `cadenzaLint` clamps any diagnostic landing in it to the cell-content end.
function exportSuffix(cellText: string, surface: Surface, downstreamUsed: readonly string[] = []): string {
  const trimmed = cellText.trim();
  // Export names the cell ACTUALLY defines. `exportNames` synthesizes `main` for a bare expression
  // (because `wrapModule` would rewrite it to `(def (main) <expr>)`), but `prepareCell` does NOT rewrite —
  // it lints the cell text verbatim — so exporting a synthesized `main` that has no `def` would be a
  // dangling export ("export `main` names no definition"). Intersect with the cell's real top-level defs.
  const defined = new Set(topLevelDefNames(trimmed, surface));
  const names = new Set(exportNames(trimmed, surface).filter((n) => defined.has(n)));
  // ALSO export any of THIS cell's defs that a LATER cell consumes — in the notebook's sequential scope
  // those defs ARE used (just not within this cell), so linting the cell in isolation would false-flag them
  // "unused definition" (CDZ0306). Exporting them marks them public → the linter counts them used. (E.g. the
  // loan example's `year3`, defined in the schedule cell but plotted by a later chart cell.)
  for (const n of downstreamUsed) if (defined.has(n)) names.add(n);
  if (names.size === 0) return "";
  const list = [...names];
  return surface === "sexpr" ? `\n(export ${list.join(" ")})` : `\nexport { ${list.join(", ")} }`;
}

/// The set of THIS cell's top-level def names that a LATER code cell references (whole-word, kebab-aware —
/// matching `assembleCell.cellDependencies`). In the sequential-scope model a def used downstream is
/// genuinely "used", so `exportSuffix` marks it public to suppress a false CDZ0306 on the per-cell lint.
function downstreamUsedDefs(cells: Cell[], index: number, cellText: string, surface: Surface): string[] {
  const defs = topLevelDefNames(cellText.trim(), surface).filter((n) => n !== "main");
  if (defs.length === 0) return [];
  // Concatenate every LATER code cell's source (widget cells are the DSL, not Cadenza — skip them).
  let downstream = "";
  for (let i = index + 1; i < cells.length; i++) {
    const c = cells[i];
    if (c.kind === "code" && c.directive.kind !== "widget") downstream += "\n" + c.source;
  }
  return defs.filter((name) => {
    const escaped = name.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&");
    return new RegExp(`(^|[^A-Za-z0-9_.\\-])${escaped}([^A-Za-z0-9_.\\-]|$)`).test(downstream);
  });
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
  // The default-fraction pragma LEADS the prefix so the linter sees the SAME rational-by-default grounding the
  // run path does (else lint types a bare int Int64 while the run makes it Rational — a lint/run mismatch). In
  // the PREFIX so `wrapPrefixBytes` counts it + cell diagnostics still map back exactly (a pragma-line diag drops).
  const prefixParts = [defaultFractionPragma(surface), widgetBindings, scopeBuffer].filter((s) => s.trim().length > 0);
  const prefix = prefixParts.length > 0 ? prefixParts.join("\n\n") + "\n\n" : "";
  // Append an `export` SUFFIX so the linted module has a public entry — otherwise `compile` declines with
  // "nothing is public" and the cell's `main` is flagged unused (operator UX #1). Suffix, not prefix, so
  // `wrapPrefixBytes` (and thus the cell's diagnostic offsets) stay exact. Also export this cell's defs that a
  // LATER cell consumes, so a def used downstream isn't false-flagged "unused" (CDZ0306) by the isolated lint.
  const suffix = exportSuffix(cellText, surface, downstreamUsedDefs(cells, index, cellText, surface));
  return { compiled: prefix + cellText + suffix, wrapPrefixBytes: utf8Len(prefix) };
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
