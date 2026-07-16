/// Assemble the `(buffer, entry)` pair to run a notebook code cell, folding in the CURRENT widget values
/// on top of sequential cell scope. This is the seam where the novel runtime-input mechanism (§5) meets
/// the compile path: a widget's live value becomes a `def name = <literal>` line prepended to the buffer,
/// so the cell's program sees `name` as an ordinary in-scope binding — no language change, exactly the
/// calculator's accumulating-buffer trick generalized to widgets + prior cells.
///
/// PURE (no worker/React) — unit-testable under `node --test`. The route (NotebookPage) calls this, then
/// hands the result to `replEval(buffer, entry, surface)` + the run worker.

import type { Cell } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";
import type { Surface } from "../compiler/worker.ts";
import { assembleCell } from "./assembleCell.ts";
import { literalFor } from "./parseWidgets.ts";

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

/// Build the `(buffer, entry)` for the code cell at `index`. `buffer` = widget bindings (one binding per
/// widget, in the widget list's order) followed by the prior code cells' sources (sequential scope).
/// `entry` = this cell's own source. A widget whose value is absent from `values` falls back to its
/// declared `default`, so a cell always sees a defined binding.
export function assembleForRun(
  cells: Cell[],
  index: number,
  widgets: Widget[],
  values: WidgetValues,
  surface: Surface,
): { buffer: string; entry: string } {
  const { buffer: scopeBuffer, entry } = assembleCell(cells, index, surface);

  const widgetBindings = widgets
    .map((w) => widgetBinding(w, values[w.name] ?? w.default, surface))
    .join("\n");

  const buffer = [widgetBindings, scopeBuffer].filter((s) => s.trim().length > 0).join("\n\n");
  return { buffer, entry };
}
