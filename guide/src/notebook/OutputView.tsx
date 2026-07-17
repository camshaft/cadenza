/// Render a code cell's computed output (a CellOutput from renderOutput) as React: a value, a table, a
/// hand-rolled SVG chart (design D3, no charting dep), a formula, or a trap/timeout/error status. The
/// shape decisions live in the pure renderOutput.ts; this is only the JSX + the SVG chart drawing.

import type { CellOutput } from "./renderOutput.ts";
import { categoryLabels, minOf, maxOf, type Series } from "./extractChart.ts";
import type { Table } from "./extractTable.ts";
import type { Formula } from "./formula.ts";

/// Render a classified formula (hand-rolled, no KaTeX): a rational as a stacked fraction, a quantity as
/// value + unit, a plain scalar large, and an unrenderable compound as a surfaced gap (not a fake).
function FormulaView({ formula }: { formula: Formula }) {
  switch (formula.kind) {
    case "fraction":
      return (
        <span className="my-2 inline-flex items-center gap-1 text-cadenza-200" data-testid="formula">
          {formula.negative && <span className="text-lg">−</span>}
          <span className="inline-flex flex-col items-center leading-tight">
            <span className="px-2">{formula.num}</span>
            <span className="border-t border-cadenza-400 px-2">{formula.den}</span>
          </span>
        </span>
      );
    case "quantity":
      return (
        <span className="my-2 inline-flex items-baseline gap-1 text-base text-cadenza-200" data-testid="formula">
          <span className="font-mono">{formula.value}</span>
          <span className="text-slate-400">{formula.unit}</span>
        </span>
      );
    case "plain":
      return <span className="my-2 inline-block font-mono text-base text-cadenza-200" data-testid="formula">{formula.text}</span>;
    case "unrenderable":
      return (
        <div data-testid="formula">
          <pre className="overflow-x-auto font-mono text-sm text-slate-200">{formula.text}</pre>
          <p className="mt-1 text-xs text-amber-400/80">{formula.reason}</p>
        </div>
      );
  }
}

function TableView({ table }: { table: Table }) {
  if (table.columns.length === 0) return <p className="text-sm text-slate-500">empty table</p>;
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr>
            {table.columns.map((c) => (
              <th key={c} className="border-b border-slate-700 px-3 py-1.5 text-left font-semibold text-slate-200">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.map((row, i) => (
            <tr key={i} className="odd:bg-slate-900/30">
              {row.map((cell, j) => (
                <td key={j} className="border-b border-slate-800 px-3 py-1 font-mono text-slate-300">
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/// A dependency-free SVG chart. Line/scatter plot points at their (x,y); bar draws a column per point,
/// with MULTIPLE series grouped side-by-side per x (each series its own hue) — no series is dropped.
/// Axes are auto-scaled to the data extent.
const HUES = ["#38bdf8", "#f472b6", "#a3e635", "#fbbf24", "#c084fc"];
const W = 520, H = 240, PAD = 32;

function ChartView({ chart, series }: { chart: "line" | "bar" | "scatter"; series: Series[] }) {
  const pts = series.flatMap((s) => s.points);
  if (pts.length === 0) return <p className="text-sm text-slate-500">no data</p>;
  const xs = pts.map((p) => p.x), ys = pts.map((p) => p.y);
  // Loop-based extent (minOf/maxOf), NOT Math.min(...xs) — a chart with many points would overflow the JS
  // argument-count limit when spread into Math.min/max (PR #524). minY seeds 0 so the y-axis includes it.
  const minX = minOf(xs), maxX = maxOf(xs);
  const minY = minOf(ys, 0), maxY = maxOf(ys);
  const spanX = maxX - minX || 1, spanY = maxY - minY || 1;
  const sx = (x: number) => PAD + ((x - minX) / spanX) * (W - 2 * PAD);
  const sy = (y: number) => H - PAD - ((y - minY) / spanY) * (H - 2 * PAD);

  // Shared bar-column width: derived ONCE from the widest series' point count, so grouped bars align
  // across series that have different point counts (PR #489). Used by the `chart === "bar"` branch below.
  const maxPoints = maxOf(series.map((s) => s.points.length), 1); // seed 1; no spread (PR #524)
  const barSlot = ((W - 2 * PAD) / maxPoints) * 0.7;

  // Short numeric tick label (a rational-ish value trimmed to ≤2 decimals, integers bare) so the axes
  // read with a scale instead of bare lines.
  const tick = (v: number) => (Number.isInteger(v) ? `${v}` : v.toFixed(2).replace(/\.?0+$/, ""));

  // Categorical x-axis: when the points carry category labels (a non-numeric x, e.g. `(tuple "jan" 10)`),
  // print the category NAMES at their x-slots instead of the numeric `min`/`max` indices. Cap the count so
  // labels don't collide; when there are more categories than we can show, thin them to evenly-spaced ticks.
  const cats = categoryLabels(series);
  const MAX_CAT_TICKS = 8;
  const catTicks =
    cats === null
      ? null
      : cats
          .map((label, x) => ({ label, x }))
          .filter((_, i, arr) => arr.length <= MAX_CAT_TICKS || i % Math.ceil(arr.length / MAX_CAT_TICKS) === 0);

  const svg = (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full max-w-2xl" role="img" aria-label="chart">
      {/* axes */}
      <line x1={PAD} y1={H - PAD} x2={W - PAD} y2={H - PAD} stroke="#475569" strokeWidth={1} />
      <line x1={PAD} y1={PAD} x2={PAD} y2={H - PAD} stroke="#475569" strokeWidth={1} />
      {/* axis ticks: y min/max up the left; x either category names at their slots or numeric min/max */}
      <g fontSize={10} fill="#94a3b8">
        {catTicks ? (
          catTicks.map(({ label, x }) => (
            <text key={x} x={sx(x)} y={H - PAD + 14} textAnchor="middle" data-testid="x-cat-tick">
              {label}
            </text>
          ))
        ) : (
          <>
            <text x={PAD} y={H - PAD + 14} textAnchor="start">{tick(minX)}</text>
            <text x={W - PAD} y={H - PAD + 14} textAnchor="end">{tick(maxX)}</text>
          </>
        )}
        <text x={PAD - 4} y={H - PAD} textAnchor="end">{tick(minY)}</text>
        <text x={PAD - 4} y={PAD + 4} textAnchor="end">{tick(maxY)}</text>
      </g>
      {series.map((s, si) => {
        const hue = HUES[si % HUES.length];
        if (chart === "bar") {
          // Group the series' bars side-by-side within each x slot so multiple series don't overlap.
          // `slot` (the per-x column width) is SHARED across series — derived from the widest series'
          // point count, NOT this series' own `s.points.length`. Per-series slot would misalign bars
          // when series have different point counts (extractChart skips non-numeric y cells) — PR #489.
          // Each of `series.length` series gets an equal `bw` sub-column; keys are series-unique.
          const slot = barSlot;
          const bw = Math.max(1, slot / series.length);
          const off = si * bw - slot / 2;
          return (
            <g key={si}>
              {s.points.map((p, i) => (
                <rect key={`${si}-${i}`} x={sx(p.x) + off} y={sy(p.y)} width={bw} height={H - PAD - sy(p.y)} fill={hue} opacity={0.8} />
              ))}
            </g>
          );
        }
        const dots = s.points.map((p, i) => <circle key={`c${i}`} cx={sx(p.x)} cy={sy(p.y)} r={2.5} fill={hue} />);
        if (chart === "scatter") return <g key={si}>{dots}</g>;
        // line
        const d = s.points.map((p, i) => `${i === 0 ? "M" : "L"}${sx(p.x)},${sy(p.y)}`).join(" ");
        return (
          <g key={si}>
            <path d={d} fill="none" stroke={hue} strokeWidth={1.5} />
            {dots}
          </g>
        );
      })}
    </svg>
  );

  // Legend: only meaningful when there's more than one NAMED series (a single series is unnamed). Shows a
  // colored swatch per series so a reader can tell a multi-y / record-list chart's series apart.
  const named = series.filter((s) => s.name !== "");
  const legend = named.length > 1 && (
    <div className="mt-1 flex flex-wrap gap-3 text-xs text-slate-400" data-testid="chart-legend">
      {series.map((s, si) =>
        // Skip an unnamed series (no blank legend entry), but index the color by the ORIGINAL position
        // so swatches stay aligned with the chart's hues (PR #517).
        s.name === "" ? null : (
          <span key={si} className="inline-flex items-center gap-1">
            <span className="inline-block h-2 w-3 rounded-sm" style={{ backgroundColor: HUES[si % HUES.length] }} />
            {s.name}
          </span>
        ),
      )}
    </div>
  );

  return (
    <div>
      {svg}
      {legend}
    </div>
  );
}

export function OutputView({ output }: { output: CellOutput }) {
  switch (output.render) {
    case "value":
      return (
        <div>
          <pre className="overflow-x-auto font-mono text-sm text-slate-200">{output.text}</pre>
          {output.note && <p className="mt-1 text-xs text-amber-400/80">{output.note}</p>}
        </div>
      );
    case "table":
      return <TableView table={output.table} />;
    case "chart":
      return (
        <div>
          <ChartView chart={output.chart} series={output.series} />
          {output.note && <p className="mt-1 text-xs text-amber-400/80">{output.note}</p>}
        </div>
      );
    case "formula":
      return <FormulaView formula={output.formula} />;
    case "trap":
      return <p className="font-mono text-sm text-rose-400">trap: {output.message}</p>;
    case "timeout":
      return <p className="font-mono text-sm text-rose-400">timed out (possible infinite loop)</p>;
    case "error":
      return <p className="font-mono text-sm text-rose-400">error: {output.message}</p>;
  }
}
