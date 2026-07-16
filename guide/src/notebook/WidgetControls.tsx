/// Render a widget cell's interactive controls (slider / number / text / checkbox / dropdown) and report
/// changes to the notebook's reactive engine. Purely presentational + the onChange callback; the widget
/// descriptors come from the tested parseWidgets, and the recompute logic lives in NotebookPage.

import type { Widget } from "./parseWidgets.ts";
import type { WidgetValues } from "./assembleForRun.ts";

export function WidgetControls({
  widgets,
  values,
  onChange,
}: {
  widgets: Widget[];
  values: WidgetValues;
  onChange: (name: string, value: number | boolean | string) => void;
}) {
  if (widgets.length === 0) return null;
  return (
    <div className="my-3 flex flex-col gap-3 rounded-lg border border-cadenza-800/40 bg-slate-900/40 px-4 py-3" data-testid="widgets">
      {widgets.map((w) => (
        <label key={w.name} className="flex items-center gap-3 text-sm text-slate-300">
          <span className="w-28 shrink-0 font-mono text-cadenza-300">{w.name}</span>
          <Control widget={w} value={values[w.name] ?? w.default} onChange={(v) => onChange(w.name, v)} />
        </label>
      ))}
    </div>
  );
}

function Control({
  widget,
  value,
  onChange,
}: {
  widget: Widget;
  value: number | boolean | string;
  onChange: (v: number | boolean | string) => void;
}) {
  switch (widget.control) {
    case "slider":
      return (
        <span className="flex flex-1 items-center gap-2">
          <input
            type="range"
            min={widget.min}
            max={widget.max}
            step={widget.step}
            value={Number(value)}
            onChange={(e) => onChange(Number(e.target.value))}
            className="flex-1 accent-cadenza-500"
            data-testid={`widget-${widget.name}`}
          />
          <span className="w-16 shrink-0 text-right font-mono text-xs text-slate-400">{String(value)}</span>
        </span>
      );
    case "number":
      return (
        <input
          type="number"
          min={widget.min}
          max={widget.max}
          step={widget.step}
          value={Number(value)}
          onChange={(e) => onChange(Number(e.target.value))}
          className="w-32 rounded border border-slate-700 bg-slate-800 px-2 py-1 font-mono text-sm text-slate-200"
          data-testid={`widget-${widget.name}`}
        />
      );
    case "text":
      return (
        <input
          type="text"
          value={String(value)}
          onChange={(e) => onChange(e.target.value)}
          className="flex-1 rounded border border-slate-700 bg-slate-800 px-2 py-1 font-mono text-sm text-slate-200"
          data-testid={`widget-${widget.name}`}
        />
      );
    case "checkbox":
      return (
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(e) => onChange(e.target.checked)}
          className="h-4 w-4 accent-cadenza-500"
          data-testid={`widget-${widget.name}`}
        />
      );
    case "dropdown":
      return (
        <select
          value={String(value)}
          onChange={(e) => onChange(e.target.value)}
          className="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-sm text-slate-200"
          data-testid={`widget-${widget.name}`}
        >
          {widget.options.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      );
  }
}
