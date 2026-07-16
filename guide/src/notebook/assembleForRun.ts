/// Assemble the `(buffer, entry)` pair to run a notebook code cell, folding in the CURRENT widget values
/// on top of sequential cell scope. This is the seam where the novel runtime-input mechanism (§5) meets
/// the compile path: a widget's live value becomes a `def name = <literal>` line in the buffer, so the
/// cell's program sees `name` as an ordinary in-scope binding — no language change, the calculator's
/// accumulating-buffer trick generalized to widgets + prior cells.
///
/// CRITICAL contract (fixed after v-guide-infra found the starter erroring on load): `replEval(buffer,
/// entry, surface)` treats `entry` as an EXPRESSION (the sole export), NOT a def-block. A notebook code
/// cell's source is a def-block (`def (main) …`), so it goes in the BUFFER; `entry` is a CALL to the
/// cell's entry point (`main` by convention, per the guide's exportNames rule). Putting a `def` in the
/// entry slot is what produced "`def` is not an expression here" (s-expr) / "expected a name" (ML).
///
/// PURE (no worker/React) — unit-testable under `node --test`. The route (NotebookPage) calls this, then
/// hands the result to `replEval(buffer, entry, surface)` + the run worker.

import type { Cell } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";
import type { Surface } from "../compiler/worker.ts";
import { assembleCell } from "./assembleCell.ts";
import { literalFor } from "./parseWidgets.ts";
import { topLevelDefNames } from "../components/wrapModule.ts";

/// The current value of each widget, keyed by name (the reactive engine's live control state).
export type WidgetValues = Record<string, number | boolean | string>;

/// A widget's current value as a surface-appropriate top-level binding. ML: `def name = <lit>`; s-expr:
/// `(def (name) <lit>)`. The literal (from `literalFor`) is surface-agnostic (a Float64 `10.0`, a
/// `"str"`, `true`) — only the def SYNTAX differs. (parseWidgets' own `bindingFor` is ML-only; the
/// notebook needs both surfaces, so the binding is built here.)
export function widgetBinding(widget: Widget, value: number | boolean | string, surface: Surface): string {
  const lit = literalFor(widget.type, value);
  return surface === "sexpr" ? `(def (${widget.name}) ${lit})` : `def ${widget.name} = ${lit}`;
}

/// The entry-point NAME a cell's source exposes to run: `main` when the cell defines it (the guide
/// convention), else the cell's first top-level def name, else `main` (a decline the caller surfaces).
/// Mirrors wrapModule.exportNames' rule so a notebook cell behaves like a wrapped guide snippet.
export function entryName(cellSource: string, surface: Surface): string {
  const names = topLevelDefNames(cellSource.trim(), surface);
  if (names.includes("main")) return "main";
  return names[0] ?? "main";
}

/// A surface-appropriate CALL to a nullary entry point: s-expr `(name)`, ML `name()`. This is the
/// EXPRESSION handed to replEval's entry slot. (Cells define nullary entry points — `def (main) …` /
/// `def main() …`; a parameterized entry isn't a notebook cell shape.)
export function entryCall(name: string, surface: Surface): string {
  return surface === "sexpr" ? `(${name})` : `${name}()`;
}

/// Build the `(buffer, entry)` for the code cell at `index`. `buffer` = widget bindings + prior code
/// cells' sources (sequential scope) + THIS cell's own source (its defs) — everything that must be in
/// scope. `entry` = a CALL to this cell's entry point (an EXPRESSION, which is what replEval requires).
/// A widget whose value is absent from `values` falls back to its declared `default`, so a cell always
/// sees a defined binding.
export function assembleForRun(
  cells: Cell[],
  index: number,
  widgets: Widget[],
  values: WidgetValues,
  surface: Surface,
): { buffer: string; entry: string } {
  const { buffer: scopeBuffer, entry: cellSource } = assembleCell(cells, index, surface);

  const widgetBindings = widgets
    .map((w) => widgetBinding(w, values[w.name] ?? w.default, surface))
    .join("\n");

  // The cell's OWN source (a def-block) belongs in the buffer, NOT the entry slot — replEval's entry is
  // an expression. Everything (widgets, prior cells, this cell) becomes the buffer's definitions; the
  // entry is a call to this cell's entry point.
  const buffer = [widgetBindings, scopeBuffer, cellSource].filter((s) => s.trim().length > 0).join("\n\n");
  const entry = entryCall(entryName(cellSource, surface), surface);
  return { buffer, entry };
}
