/// The reactive recompute planner — the pure dataflow core of the notebook's interactive-widget feature
/// (Increment 4). When a reader drags a widget, WHICH code cells must re-run, and in what order?
///
/// Model (sequential scope, design D1): each code cell PRODUCES its top-level def names and CONSUMES the
/// names it references; each widget PRODUCES its name (spliced downstream as `def name = <value>`, §5).
/// A widget change makes its name dirty; walking cells in DOCUMENT ORDER, a cell is dirty if it consumes
/// any dirty name, and a dirtied cell's produced names become dirty too (it was recomputed) — so a change
/// propagates transitively downstream. The dirty cells, in document order, are the recompute plan.
///
/// This is correctness-first (over-approximate: a cell that merely mentions a dirty name re-runs even if
/// the mention is shadowed — never MISSES a needed recompute). A later refinement can prune. PURE (no
/// worker/React) — unit-testable under `node --test`.

import type { Cell } from "./parseDocument.ts";
import type { Widget } from "./parseWidgets.ts";
import type { Surface } from "../compiler/worker.ts";
import { topLevelDefNames } from "../components/wrapModule.ts";

/// A code cell's producer/consumer summary, plus its index in the full cell list (prose cells are
/// skipped, so `index` is NOT contiguous).
interface CellNode {
  index: number;
  produces: string[];
  consumes: string[];
}

/// Whole-word, kebab-aware membership test: does `name` appear as a standalone token in `source`?
/// Identifier chars are letters/digits/`_`/`-`/`.` (matches the def-name grammar), so `rate` must not
/// match inside `rate-adjusted`. Mirrors assembleCell.cellDependencies' matcher (kept local so this
/// module doesn't depend on assembleCell's Assembled shape).
function references(source: string, name: string): boolean {
  const escaped = name.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&");
  return new RegExp(`(^|[^A-Za-z0-9_.\\-])${escaped}([^A-Za-z0-9_.\\-]|$)`).test(source);
}

/// Build the producer/consumer graph over the notebook's code cells. `candidates` is the universe of
/// names that can create a dependency: every widget name + every code cell's produced def names. A cell
/// CONSUMES a candidate name if it references it (and doesn't itself produce it as its first definition —
/// we keep it simple: a name a cell both produces and references still counts as consumed, which only
/// ever over-approximates). Widget/hidden cells: a widget cell contributes no def-buffer (its names come
/// via the widget list), a hidden code cell participates normally.
function buildNodes(cells: Cell[], widgets: Widget[], surface: Surface): { nodes: CellNode[]; candidates: Set<string> } {
  const widgetNames = widgets.map((w) => w.name);
  const nodes: CellNode[] = [];
  const candidates = new Set<string>(widgetNames);

  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    if (cell.kind !== "code" || cell.directive.kind === "widget") continue;
    const produces = topLevelDefNames(cell.source.trim(), surface);
    for (const p of produces) candidates.add(p);
    nodes.push({ index: i, produces, consumes: [] });
  }

  // Second pass: now that every producible name is known, compute each cell's consumed names.
  for (const node of nodes) {
    const source = (cells[node.index] as Extract<Cell, { kind: "code" }>).source;
    node.consumes = [...candidates].filter((name) => references(source, name));
  }
  return { nodes, candidates };
}

/// Compute the recompute plan for a widget change: the DOCUMENT-ORDER list of code-cell indices to
/// re-run when `changedWidget` changes. Empty if no cell (transitively) depends on it.
export function recomputePlan(
  cells: Cell[],
  widgets: Widget[],
  changedWidget: string,
  surface: Surface,
): number[] {
  const { nodes } = buildNodes(cells, widgets, surface);
  const dirty = new Set<string>([changedWidget]);
  const plan: number[] = [];
  // Document order: `nodes` is already in ascending index order.
  for (const node of nodes) {
    if (node.consumes.some((n) => dirty.has(n))) {
      plan.push(node.index);
      // This cell was recomputed → its produced names are now dirty for downstream cells.
      for (const p of node.produces) dirty.add(p);
    }
  }
  return plan;
}

/// The FULL initial run order — every code cell in document order (widget cells excluded; they render
/// controls, not program output). Used for the first render of a notebook (run everything top-to-bottom).
export function initialRunOrder(cells: Cell[]): number[] {
  const order: number[] = [];
  for (let i = 0; i < cells.length; i++) {
    const c = cells[i];
    if (c.kind === "code" && c.directive.kind !== "widget") order.push(i);
  }
  return order;
}
