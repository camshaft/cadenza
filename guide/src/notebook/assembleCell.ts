/// Sequential cell-scope assembly (design D1, ratified: accumulating/Jupyter scope).
///
/// A notebook code cell N sees the top-level DEFINITIONS of all prior code cells 1..N-1 — the same
/// "buffer of accumulated definitions" model the calculator uses (`calculator/engine.ts` → `replEval`).
/// This module computes, for a given cell index, the `(buffer, entry)` pair the existing compile path
/// consumes: `buffer` = the prior code cells' sources concatenated (their `def`s become in-scope names),
/// `entry` = this cell's own source (its expression / defs are what actually runs + renders).
///
/// PURE by design — NO worker/React/compiler imports — so it's unit-testable under `node --test`
/// (mirrors `parseDocument.ts` and the calculator's split-out pure modules). The route shell
/// (Increment 2) feeds the result to `replEval(buffer, entry, surface)`; the reactive engine
/// (Increment 4) splices widget values on top of this buffer.

import type { Cell } from "./parseDocument.ts";
import type { Surface } from "../compiler/worker.ts";
import { topLevelDefNames } from "../components/wrapModule.ts";

/// A cell's assembled runnable program, split for `replEval(buffer, entry, surface)`:
/// - `buffer` — all prior code cells' sources joined (their top-level defs, in document order). Empty
///   string when the cell has no prior code cells (a bare-`0` sentinel is the CALLER's concern, matching
///   how the calculator passes `"0"` for an empty buffer — see `engine.ts`).
/// - `entry` — this cell's own source (the expression or defs whose value/output this cell renders).
/// - `inScope` — the top-level def names visible to this cell FROM prior cells (for editor autocomplete /
///   a "this cell can use: …" affordance, and to drive the reactive dependency graph in Inc 4).
export interface Assembled {
  buffer: string;
  entry: string;
  inScope: string[];
}

/// Which cells contribute to sequential scope: only CODE cells (prose renders, it doesn't run; a widget
/// cell's bindings are spliced by the reactive engine in Inc 4, not via this def-buffer). A `hidden`
/// code cell DOES contribute — it runs, it just shows no source, so its defs are in scope downstream.
function isScopeContributing(cell: Cell): boolean {
  return cell.kind === "code" && cell.directive.kind !== "widget";
}

/// Assemble the runnable program for the code cell at `index` in `cells`, under sequential scope.
/// Throws if `index` is out of range or names a non-code cell (the caller only assembles code cells).
///
/// The buffer joins prior scope-contributing code cells' sources with blank lines between them — each
/// is a block of top-level `def`/`type`/`effect` forms, and both surfaces accept newline-separated
/// top-level forms in a buffer (`replEval` gathers them). We do NOT wrap here: `replEval` + `wrapModule`
/// own the export/main scaffolding; this module only decides WHAT source is in scope.
export function assembleCell(cells: Cell[], index: number, surface: Surface): Assembled {
  const cell = cells[index];
  if (!cell) throw new RangeError(`assembleCell: no cell at index ${index}`);
  if (cell.kind !== "code") throw new TypeError(`assembleCell: cell ${index} is prose, not code`);

  const priorSources: string[] = [];
  const inScope: string[] = [];
  for (let i = 0; i < index; i++) {
    const prior = cells[i];
    if (!isScopeContributing(prior)) continue;
    // `prior` is a scope-contributing code cell.
    const src = (prior as Extract<Cell, { kind: "code" }>).source;
    const trimmed = src.trim();
    if (!trimmed) continue; // an empty code cell contributes nothing
    priorSources.push(trimmed);
    for (const name of topLevelDefNames(trimmed, surface)) {
      if (!inScope.includes(name)) inScope.push(name);
    }
  }

  return {
    buffer: priorSources.join("\n\n"),
    entry: cell.source.trim(),
    inScope,
  };
}

/// The def names a cell's ENTRY references that are provided by prior cells (its cross-cell dependencies).
/// Used by the reactive recompute graph (Inc 4) — a widget/upstream change re-runs a cell only if the
/// cell actually references a changed name. A conservative token scan: a name is "used" if it appears as
/// a whole word in the entry source. (Precise free-variable analysis is a later refinement; over-
/// approximating never MISSES a dependency, it only recomputes a bit more — correctness first, per D1.)
export function cellDependencies(assembled: Assembled): string[] {
  const { entry, inScope } = assembled;
  const used: string[] = [];
  for (const name of inScope) {
    // Whole-word match; kebab names contain `-`, so `\b` alone is wrong (it treats `-` as a boundary).
    // Bound the name with a non-identifier char (or string edge) on each side. Identifier chars are
    // letters/digits/`_`/`-`/`.` (matches `topLevelDefNames` + the calculator's `isIdentifier`).
    const escaped = name.replace(/[.*+?^${}()|[\]\\-]/g, "\\$&");
    const re = new RegExp(`(^|[^A-Za-z0-9_.\\-])${escaped}([^A-Za-z0-9_.\\-]|$)`);
    if (re.test(entry)) used.push(name);
  }
  return used;
}
