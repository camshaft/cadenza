/// Extract a tabular view from a cell's rendered value, for the `table` output renderer (Increment 3).
///
/// A table cell's program returns a `List` of rows; each row is a `tuple` (positional columns) or a
/// `record` (named columns). We parse the canonical s-expr render (`run(component, "sexpr")`, NOT the
/// display surface — the /cad lesson) and shape it into { columns, rows } an HTML <table> renders.
///
/// PURE (no worker/React) — unit-testable under `node --test`. The route's table component (Inc 2/3)
/// calls this on the rendered value and maps the result to <thead>/<tbody>.

import { parseSexpr, stripAscription, isAtom, isList, type Node } from "./sexpr.ts";
import { displayNode } from "./formatValue.ts";

/// A shaped table: column headers + rows of already-stringified cells (display text). For a tuple list
/// the columns are positional (`col 0`, `col 1`, …); for a record list they're the field names (union
/// of all rows' fields, first-seen order). A cell missing in a given row renders as "".
export interface Table {
  columns: string[];
  rows: string[][];
}

/// The result of trying to shape a value as a table: the table, or why it isn't one (so the renderer can
/// fall back to the plain value view rather than showing a broken table).
export type TableResult = { ok: true; table: Table } | { ok: false; reason: string };

/// Render a cell's display text via the SHARED friendly node renderer (formatValue.displayNode): a
/// "quoted string" loses its quotes; a number/symbol/rational shows bare; a `(quantity 5 meter)` shows
/// as `5 meter` (matching plain-value cells); any other nested compound shows compactly. One display
/// path, so a quantity in a table reads the same as a quantity in a value cell.
const atomText = displayNode;

/// The head symbol of a list node (`(list …)` → "list"), or null for an atom / empty list.
function head(n: Node): string | null {
  if (isList(n) && n.list.length > 0 && isAtom(n.list[0])) return n.list[0].atom;
  return null;
}

/// Shape a rendered value (the raw s-expr string from the run worker) into a table. Returns a typed
/// `{ ok: false }` — never throws — when the value isn't a list-of-rows.
export function extractTable(rendered: string): TableResult {
  let node: Node;
  try {
    node = stripAscription(parseSexpr(rendered));
  } catch (e) {
    return { ok: false, reason: `unparseable value: ${(e as Error).message}` };
  }

  // A BARE record (not wrapped in a list) → a single-row table: its fields are the columns, one row of
  // values. This is the natural "one labeled row" / struct-of-results display, so a cell can return a
  // record directly without wrapping it in a list.
  if (head(node) === "record") return recordTable([node]);

  if (head(node) !== "list") {
    return { ok: false, reason: "value is not a `list` or `record` — a table cell must return one of those" };
  }
  const elems = (node as { list: Node[] }).list.slice(1); // drop the `list` head
  if (elems.length === 0) return { ok: true, table: { columns: [], rows: [] } };

  const firstHead = head(elems[0]);
  if (firstHead === "record") return recordTable(elems);
  if (firstHead === "tuple") return tupleTable(elems);
  // A list of scalars → a single unnamed column (a 1-D table is still useful).
  return {
    ok: true,
    table: { columns: ["value"], rows: elems.map((e) => [atomText(e)]) },
  };
}

/// A list of `(tuple a b …)` rows → positional columns. Column count = the widest row; short rows pad "".
function tupleTable(elems: Node[]): TableResult {
  let width = 0;
  for (const e of elems) {
    if (head(e) !== "tuple") return { ok: false, reason: "mixed row shapes — expected all `tuple` rows" };
    width = Math.max(width, (e as { list: Node[] }).list.length - 1);
  }
  const columns = Array.from({ length: width }, (_, i) => `col ${i}`);
  const rows = elems.map((e) => {
    const cells = (e as { list: Node[] }).list.slice(1).map(atomText);
    while (cells.length < width) cells.push("");
    return cells;
  });
  return { ok: true, table: { columns, rows } };
}

/// A list of `(record (field val) …)` rows → named columns. Columns = union of all fields (first-seen
/// order). A field absent in a row renders "".
function recordTable(elems: Node[]): TableResult {
  const columns: string[] = [];
  const rowMaps: Map<string, string>[] = [];
  for (const e of elems) {
    if (head(e) !== "record") return { ok: false, reason: "mixed row shapes — expected all `record` rows" };
    const fields = (e as { list: Node[] }).list.slice(1); // drop the `record` head
    const map = new Map<string, string>();
    for (const f of fields) {
      // Each field is `(name value)`. A malformed field is skipped rather than aborting the table.
      if (!isList(f) || f.list.length < 2 || !isAtom(f.list[0])) continue;
      const name = f.list[0].atom;
      // A field with >1 value node (rare) is rendered as the friendly join of its value nodes.
      const value = f.list.length === 2 ? atomText(f.list[1]) : f.list.slice(1).map(displayNode).join(" ");
      if (!columns.includes(name)) columns.push(name);
      map.set(name, value);
    }
    rowMaps.push(map);
  }
  const rows = rowMaps.map((m) => columns.map((c) => m.get(c) ?? ""));
  return { ok: true, table: { columns, rows } };
}
