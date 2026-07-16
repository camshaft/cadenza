/// Render a code cell's computed output (a CellOutput from renderOutput) as React: a value, a table, a
/// hand-rolled SVG chart (design D3, no charting dep), a formula, or a trap/timeout/error status. The
/// shape decisions live in the pure renderOutput.ts; this is only the JSX + the SVG chart drawing.

import type { CellOutput } from "./renderOutput.ts";
import type { Series } from "./extractChart.ts";
import type { Table } from "./extractTable.ts";

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

/// A dependency-free SVG chart. Line/scatter plot points at their (x,y); bar draws a column per point of
/// the first series. Axes are auto-scaled to the data extent. Multiple series get distinct stroke hues.
const HUES = ["#38bdf8", "#f472b6", "#a3e635", "#fbbf24", "#c084fc"];
const W = 520, H = 240, PAD = 32;

function ChartView({ chart, series }: { chart: "line" | "bar" | "scatter"; series: Series[] }) {
  const pts = series.flatMap((s) => s.points);
  if (pts.length === 0) return <p className="text-sm text-slate-500">no data</p>;
  const xs = pts.map((p) => p.x), ys = pts.map((p) => p.y);
  const minX = Math.min(...xs), maxX = Math.max(...xs);
  const minY = Math.min(0, ...ys), maxY = Math.max(...ys);
  const spanX = maxX - minX || 1, spanY = maxY - minY || 1;
  const sx = (x: number) => PAD + ((x - minX) / spanX) * (W - 2 * PAD);
  const sy = (y: number) => H - PAD - ((y - minY) / spanY) * (H - 2 * PAD);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full max-w-2xl" role="img" aria-label="chart">
      {/* axes */}
      <line x1={PAD} y1={H - PAD} x2={W - PAD} y2={H - PAD} stroke="#475569" strokeWidth={1} />
      <line x1={PAD} y1={PAD} x2={PAD} y2={H - PAD} stroke="#475569" strokeWidth={1} />
      {series.map((s, si) => {
        const hue = HUES[si % HUES.length];
        if (chart === "bar") {
          const bw = Math.max(2, ((W - 2 * PAD) / Math.max(1, s.points.length)) * 0.7);
          return s.points.map((p, i) => (
            <rect key={i} x={sx(p.x) - bw / 2} y={sy(p.y)} width={bw} height={H - PAD - sy(p.y)} fill={hue} opacity={0.8} />
          ));
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
      return <pre className="overflow-x-auto font-mono text-base text-cadenza-200">{output.text}</pre>;
    case "trap":
      return <p className="font-mono text-sm text-rose-400">trap: {output.message}</p>;
    case "timeout":
      return <p className="font-mono text-sm text-rose-400">timed out (possible infinite loop)</p>;
    case "error":
      return <p className="font-mono text-sm text-rose-400">error: {output.message}</p>;
  }
}
