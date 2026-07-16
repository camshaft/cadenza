/// The `/notebook` route — a Jupyter-like notebook over the real Cadenza language (GH #468). A markdown
/// document interleaves prose with runnable ```cadenza code cells; each cell compiles+runs in-browser
/// (reusing the guide's compile + run workers, exactly like /calculator and /cad), and its result renders
/// as a typed output (value / table / chart / formula). A ```cadenza widget cell renders interactive
/// controls (slider/number/text/checkbox/dropdown) whose values are spliced into downstream cells as
/// `def name = <value>` — so dragging a widget RECOMPUTES the dependent cells reactively (the novel core).
///
/// The heavy work lives in tested pure modules (parseDocument / assembleForRun / recomputePlan /
/// renderOutput / parse*). This component is the orchestration + React state: a SERIALIZED run queue
/// (the run worker is one-at-a-time — v-guide-infra's constraint), and a debounced widget→recompute loop.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { replEval } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import type { Surface } from "../compiler/worker.ts";
import { parseDocument, type Cell } from "./parseDocument.ts";
import { parseWidgets, type Widget } from "./parseWidgets.ts";
import { assembleForRun, type WidgetValues } from "./assembleForRun.ts";
import { recomputePlan, initialRunOrder } from "./recomputePlan.ts";
import { renderOutput, type CellOutput, type RunOutcome } from "./renderOutput.ts";
import { ProseView } from "./ProseView.tsx";
import { OutputView } from "./OutputView.tsx";
import { WidgetControls } from "./WidgetControls.tsx";
import { DEFAULT_EXAMPLE } from "./examples.ts";

/// The starter notebook the route opens with — the flagship compound-interest example (from the shared
/// `examples` module, so the route, the docs, and check:visual all draw from one source of truth).
const STARTER = DEFAULT_EXAMPLE.markdown;

/// Per-code-cell run state, keyed by the cell's index in the parsed cell list.
type CellState = { phase: "idle" } | { phase: "running" } | { phase: "done"; output: CellOutput };

export default function NotebookPage() {
  const { surface } = useSyntax();
  // The notebook document. Editing the whole doc (an editor pane) is a later slice; for now it's the
  // starter, and code cells run/recompute against it. `setDoc` is wired when the doc editor lands.
  const [doc] = useState(STARTER);
  const cells = useMemo<Cell[]>(() => parseDocument(doc), [doc]);

  // All widgets declared across every widget cell, and their live values.
  const widgets = useMemo<Widget[]>(
    () => cells.flatMap((c) => (c.kind === "code" && c.directive.kind === "widget" ? parseWidgets(c.source).widgets : [])),
    [cells],
  );
  const [values, setValues] = useState<WidgetValues>({});
  // Seed any widget missing a value with its default (on doc/widget change).
  useEffect(() => {
    setValues((prev) => {
      const next = { ...prev };
      let changed = false;
      for (const w of widgets) if (!(w.name in next)) { next[w.name] = w.default; changed = true; }
      return changed ? next : prev;
    });
  }, [widgets]);

  const [states, setStates] = useState<Record<number, CellState>>({});

  // A SERIALIZED run queue: the run worker is one-at-a-time (v-guide-infra), so we chain runs on a single
  // promise. A newer enqueue supersedes nothing — each planned cell runs in order. `runToken` lets a
  // fresh recompute (or doc edit) abandon stale in-flight renders.
  const runChain = useRef<Promise<void>>(Promise.resolve());
  const runToken = useRef(0);

  const runCells = useCallback(
    (indices: number[], vals: WidgetValues, from: Surface, token: number) => {
      for (const i of indices) setStates((s) => ({ ...s, [i]: { phase: "running" } }));
      runChain.current = runChain.current.then(async () => {
        for (const i of indices) {
          if (runToken.current !== token) return; // superseded — stop the stale chain
          const { buffer, entry } = assembleForRun(cells, i, widgets, vals, from);
          let output: CellOutput;
          try {
            const compiled = await replEval(buffer, entry, from);
            if (!compiled.component) {
              const d = compiled.diagnostics.find((x) => x.error) ?? compiled.diagnostics[0];
              output = { render: "error", message: d ? `${d.code} ${d.message}`.trim() : "compile declined" };
            } else {
              // Run + render in s-expr (the canonical machine form the extractors parse — the /cad lesson).
              const outcome = (await runComponent(compiled.component, "sexpr")) as RunOutcome;
              const cell = cells[i];
              output = renderOutput(cell.kind === "code" ? cell.directive : { kind: "none" }, outcome);
            }
          } catch (e) {
            output = { render: "error", message: e instanceof Error ? e.message : String(e) };
          }
          if (runToken.current !== token) return;
          setStates((s) => ({ ...s, [i]: { phase: "done", output } }));
        }
      });
    },
    [cells, widgets],
  );

  // Initial + on-doc-change: run every code cell top-to-bottom.
  useEffect(() => {
    const token = ++runToken.current;
    setStates({});
    runCells(initialRunOrder(cells), { ...defaultsOf(widgets), ...values }, surface, token);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cells, surface]);

  // A widget change: debounce (a slider drag fires many events), then recompute only the dependent cells.
  const debounce = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onWidgetChange = useCallback(
    (name: string, value: number | boolean | string) => {
      setValues((v) => ({ ...v, [name]: value }));
      if (debounce.current) clearTimeout(debounce.current);
      debounce.current = setTimeout(() => {
        const token = ++runToken.current;
        setValues((v) => {
          const next = { ...v, [name]: value };
          runCells(recomputePlan(cells, widgets, name, surface), next, surface, token);
          return next;
        });
      }, 150);
    },
    [cells, widgets, surface, runCells],
  );

  return (
    <div className="mx-auto min-h-screen max-w-4xl px-4 py-6">
      <div className="mb-4 flex items-baseline justify-between gap-3">
        <h1 className="text-lg font-bold text-slate-100 sm:text-xl">Cadenza Notebook</h1>
        <Link to="/playground" className="shrink-0 text-xs text-cadenza-400 hover:text-cadenza-300">
          Playground →
        </Link>
      </div>

      <div className="space-y-2" data-testid="notebook">
        {cells.map((cell, i) =>
          cell.kind === "prose" ? (
            <ProseView key={i} markdown={cell.markdown} />
          ) : cell.directive.kind === "widget" ? (
            <WidgetControls key={i} widgets={parseWidgets(cell.source).widgets} values={values} onChange={onWidgetChange} />
          ) : (
            <CodeCellView key={i} source={cell.source} hidden={cell.directive.kind === "hidden"} state={states[i]} />
          ),
        )}
      </div>
    </div>
  );
}

function defaultsOf(widgets: Widget[]): WidgetValues {
  const v: WidgetValues = {};
  for (const w of widgets) v[w.name] = w.default;
  return v;
}

/// A code cell: its source (unless hidden) + its computed output.
function CodeCellView({ source, hidden, state }: { source: string; hidden: boolean; state?: CellState }) {
  return (
    <div className="my-3 rounded-lg border border-slate-800 bg-slate-900/40">
      {!hidden && (
        <pre className="overflow-x-auto border-b border-slate-800 px-3 py-2 font-mono text-sm text-slate-300">
          {source}
        </pre>
      )}
      <div className="px-3 py-2" data-testid="cell-output">
        {!state || state.phase === "idle" ? (
          <span className="text-xs text-slate-600">not run</span>
        ) : state.phase === "running" ? (
          <span className="text-xs text-slate-500">running…</span>
        ) : (
          <OutputView output={state.output} />
        )}
      </div>
    </div>
  );
}
