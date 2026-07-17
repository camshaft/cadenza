/// Extract chart data (series of numeric points) from a cell's rendered value, for the `chart:line` /
/// `chart:bar` / `chart:scatter` output renderers (Increment 3). The hand-rolled SVG chart component
/// (design D3, ratified: no charting dep for the first cut) consumes this shape.
///
/// Accepted value shapes (all parsed from the CANONICAL s-expr render, never the display surface — the
/// /cad lesson):
///   - a List of `(tuple x y)`             → one unnamed series of (x, y) points
///   - a List of `(tuple label y)` where   → still (x, y): a non-numeric x becomes the categorical label
///       x is a string/symbol                 (bar/line over categories); its numeric index is the x.
///   - a List of numbers `y`               → a series of (i, y) points (x = the 0-based index)
///   - a List of `(tuple x y1 y2 …)`       → multiple series sharing the x (series "y0","y1",…)
///
/// PURE (no worker/React) — unit-testable under `node --test`.

import { parseSexpr, stripAscription, isAtom, isList, unquoteAtom, type Node } from "./sexpr.ts";

/// One plotted point. `x` is numeric (a category's index when the source x was a label); `label` carries
/// the original categorical x when it wasn't numeric, so the axis can show it.
export interface Point {
  x: number;
  y: number;
  label?: string;
}

/// A named series of points. `name` is "" for a single unnamed series; "y0","y1",… for multi-series rows.
export interface Series {
  name: string;
  points: Point[];
}

export type ChartResult = { ok: true; series: Series[] } | { ok: false; reason: string };

function head(n: Node): string | null {
  if (isList(n) && n.list.length > 0 && isAtom(n.list[0])) return n.list[0].atom;
  return null;
}

/// Parse an atom node as a finite number, or null if it isn't one (a rational `n/d` evaluates to n/d).
function asNumber(n: Node): number | null {
  if (!isAtom(n)) return null;
  const a = n.atom;
  const rat = /^(-?\d+)\/(-?\d+)$/.exec(a);
  if (rat) {
    const d = Number(rat[2]);
    return d === 0 ? null : Number(rat[1]) / d;
  }
  const v = Number(a);
  return Number.isFinite(v) ? v : null;
}

/// Shape a rendered value into one or more numeric series. Returns a typed `{ ok: false }` (never throws)
/// when the value isn't chartable, so the renderer can fall back to the table / plain-value view.
export function extractChart(rendered: string): ChartResult {
  let node: Node;
  try {
    node = stripAscription(parseSexpr(rendered));
  } catch (e) {
    return { ok: false, reason: `unparseable value: ${(e as Error).message}` };
  }
  if (head(node) !== "list") {
    return { ok: false, reason: "value is not a `list` — a chart cell must return a List of points" };
  }
  const elems = (node as { list: Node[] }).list.slice(1);
  if (elems.length === 0) return { ok: true, series: [] };

  // A list of bare numbers → a single series of (index, y).
  if (elems.every((e) => asNumber(e) !== null)) {
    return {
      ok: true,
      series: [{ name: "", points: elems.map((e, i) => ({ x: i, y: asNumber(e)! })) }],
    };
  }

  // A list of records → each record is a point: the FIRST field is x (numeric, else a category label with
  // x = row index), and each subsequent numeric field is a y-series named by its field. Mirrors the tuple
  // path with named columns, and matches how the table renderer accepts record-lists.
  if (elems.every((e) => head(e) === "record")) return recordSeries(elems);

  // Otherwise every element must be a tuple with ≥2 fields.
  if (!elems.every((e) => head(e) === "tuple")) {
    return { ok: false, reason: "chart rows must be all numbers, all `tuple` points, or all `record` points" };
  }
  const tuples = elems.map((e) => (e as { list: Node[] }).list.slice(1));
  const yCount = Math.min(...tuples.map((t) => t.length)) - 1; // shared y-columns (x is field 0)
  if (yCount < 1) return { ok: false, reason: "each `tuple` point needs an x and at least one y" };

  const seriesCount = yCount;
  const series: Series[] = Array.from({ length: seriesCount }, (_, s) => ({
    name: seriesCount === 1 ? "" : `y${s}`,
    points: [] as Point[],
  }));

  tuples.forEach((t, i) => {
    const xNode = t[0];
    const xNum = asNumber(xNode);
    // A non-numeric x is a category label; its x is the row index.
    const x = xNum ?? i;
    const label = xNum === null ? (isAtom(xNode) ? unquoteAtom(xNode.atom) : undefined) : undefined;
    for (let s = 0; s < seriesCount; s++) {
      const yNum = asNumber(t[s + 1]);
      if (yNum === null) continue; // skip a non-numeric y cell rather than aborting the whole chart
      series[s].points.push(label !== undefined ? { x, y: yNum, label } : { x, y: yNum });
    }
  });

  return { ok: true, series };
}

/// Derive the ordered categorical x-axis labels for a chart, or null if the chart isn't categorical.
/// A chart is categorical when its points carry `label`s (a non-numeric x was mapped to its row index +
/// a label, e.g. `(tuple "jan" 10)`). Returns one label per integer x-slot `0..maxX`, taking the label
/// seen at that x across all series (a slot with no label — shouldn't happen for a categorical chart —
/// falls back to its bare index). ChartView uses this to print category names under the axis instead of
/// the numeric `0..N` indices. Returns null when NO point has a label (a purely numeric chart), so the
/// caller keeps the numeric min/max ticks.
export function categoryLabels(series: Series[]): string[] | null {
  const byX = new Map<number, string>();
  for (const s of series) {
    for (const p of s.points) {
      // Only integer x-slots can be category positions; a labelled point always has an integer index x.
      if (p.label !== undefined && Number.isInteger(p.x)) byX.set(p.x, p.label);
    }
  }
  if (byX.size === 0) return null;
  const maxX = Math.max(...byX.keys());
  return Array.from({ length: maxX + 1 }, (_, i) => byX.get(i) ?? `${i}`);
}

/// A `(name value)` field of a record; returns [name, valueNode] or null if malformed.
function recordField(f: Node): [string, Node] | null {
  if (isList(f) && f.list.length === 2 && isAtom(f.list[0])) return [f.list[0].atom, f.list[1]];
  return null;
}

/// Shape a list of `(record (f v) …)` rows into series: the FIRST field is x (numeric, else a category
/// label with x = the row index); each SUBSEQUENT numeric field is a y-series named by its field name.
/// The y-series set is taken from the first row (`(record (year 1) (bal 10))` → x=year, series "bal").
function recordSeries(elems: Node[]): ChartResult {
  const firstFields = (elems[0] as { list: Node[] }).list.slice(1).map(recordField);
  if (firstFields.some((f) => f === null) || firstFields.length < 2) {
    return { ok: false, reason: "each `record` point needs a first (x) field and at least one numeric y field" };
  }
  const yNames = firstFields.slice(1).map((f) => f![0]); // series names = the non-x field names
  const series: Series[] = yNames.map((name) => ({ name: yNames.length === 1 ? "" : name, points: [] as Point[] }));

  elems.forEach((e, i) => {
    const fields = (e as { list: Node[] }).list.slice(1).map(recordField).filter((f): f is [string, Node] => f !== null);
    const byName = new Map(fields);
    const xNode = fields[0]?.[1];
    const xNum = xNode ? asNumber(xNode) : null;
    const x = xNum ?? i;
    const label = xNum === null && xNode && isAtom(xNode) ? unquoteAtom(xNode.atom) : undefined;
    yNames.forEach((name, s) => {
      const v = byName.get(name);
      const yNum = v ? asNumber(v) : null;
      if (yNum === null) return; // skip a missing/non-numeric y in this row
      series[s].points.push(label !== undefined ? { x, y: yNum, label } : { x, y: yNum });
    });
  });

  return { ok: true, series };
}
