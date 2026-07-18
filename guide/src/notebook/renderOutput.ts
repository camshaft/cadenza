/// Pure output-renderer dispatch: given a code cell's directive + its run outcome, decide WHAT the cell
/// output component should render — a table, a chart, a formula, a plain value, or an error/fallback.
/// The React component (Increment 2b) becomes a thin switch over this `CellOutput`; all the shape logic
/// (which renderer, graceful fallback when the value doesn't fit the directive) lives here, so it's
/// unit-testable under `node --test` (no worker/React imports — the run outcome is passed in).
///
/// Fallback philosophy: a directive is a HINT. If the value doesn't fit (a `table` cell whose program
/// returns a scalar, a `chart` cell whose value isn't a list of points), we DON'T error — we render the
/// plain value plus a `note` explaining why the requested view didn't apply. A cell that traps / errors /
/// times out shows that status. This keeps a half-written notebook legible instead of a wall of red.

import type { CellDirective } from "./parseDocument.ts";
import { extractTable, type Table } from "./extractTable.ts";
import { extractChart, type Series } from "./extractChart.ts";
import { formatValue } from "./formatValue.ts";
import { classifyFormula, type Formula } from "./formula.ts";

/// The run outcome shape (structural mirror of runner/client.ts's RunOutcome — mirrored, not imported,
/// so this module stays worker-free and node-testable). The route passes the real RunOutcome; it's
/// structurally assignable.
export type RunOutcome =
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

/// What the cell-output component should render. `note` (optional) explains a fallback (e.g. "requested a
/// table but the value isn't a list").
export type CellOutput =
  | { render: "value"; text: string; note?: string }
  | { render: "table"; table: Table }
  | { render: "chart"; chart: "line" | "bar" | "scatter" | "area" | "stacked"; series: Series[]; note?: string }
  | { render: "formula"; formula: Formula }
  | { render: "trap"; message: string }
  | { render: "timeout" }
  | { render: "error"; message: string };

/// Map a directive + run outcome to a CellOutput. Non-value outcomes (trap/timeout/error) pass straight
/// through to their status render regardless of directive.
export function renderOutput(directive: CellDirective, outcome: RunOutcome): CellOutput {
  if (outcome.kind === "trap") return { render: "trap", message: outcome.message };
  if (outcome.kind === "timeout") return { render: "timeout" };
  if (outcome.kind === "error") return { render: "error", message: outcome.message };

  // outcome.kind === "value" — a rendered s-expr `(: value type)` string in `outcome.text`.
  const text = outcome.text;

  // The plain-value display is the FRIENDLY form (`42`, `5/2`, `hi`, `2192 meter`), not the raw ascribed
  // s-expr (`(: 42 Int64)`). The table/chart shape parsers still read the CANONICAL `text` above.
  const display = formatValue(text);

  switch (directive.kind) {
    case "table": {
      const t = extractTable(text);
      if (t.ok) return { render: "table", table: t.table };
      return { render: "value", text: display, note: `not shown as a table: ${t.reason}` };
    }
    case "chart": {
      const c = extractChart(text);
      if (c.ok && c.series.length > 0) return { render: "chart", chart: directive.chart, series: c.series };
      const reason = c.ok ? "the value produced no data points" : c.reason;
      return { render: "value", text: display, note: `not shown as a chart: ${reason}` };
    }
    case "formula":
      // Classify the RAW value (not the friendly display string) so a rational becomes a stacked fraction,
      // a quantity value+unit, etc. — an unrenderable compound surfaces a gap rather than faking it.
      return { render: "formula", formula: classifyFormula(text) };
    case "none":
    case "widget": // a widget cell's own output (if any) renders as a plain value; its CONTROLS are
                   // rendered separately by the reactive engine from parseWidgets (Inc 4b).
    case "hidden": // a hidden cell shows no source, but if it's asked to render an output it's a value.
    default:
      return { render: "value", text: display };
  }
}
