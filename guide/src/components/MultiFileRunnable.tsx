/// A MULTI-FILE runnable example (operator-mandated: "multi-file runnables so the events are decoupled from
/// the reducer implementation — clearer boundaries"). Where <Runnable source={…}/> is one editable snippet,
/// this shows a SET of files — one marked the entry (the genesis program), the rest link-merged modules it
/// can `import` — compiled together in-browser via the SAME seam the platform explorer uses: the pure
/// workspace reducer (explorer/workspace.ts) + lowerToCompile → compileWithPreloaded (explorer/explorerView.ts).
///
/// Authored as <Runnable files={[{name, source, surface, entry}]} …/> — the single-source <Runnable> path is
/// UNCHANGED (this only runs when `files` is present), so the ~100 existing runnables are untouched. Building
/// on the explorer's tested pure cores means the file-set invariants (unique names, exactly one entry, live
/// active tab) are enforced by the reducer, not re-derived here; this component is a thin editable view + the
/// run wiring, exactly like ExplorerPage but embedded inline in a chapter.

import { useCallback, useMemo, useRef, useState } from "react";
import { LazyCodeEditor as CodeEditor } from "../editor/LazyCodeEditor.tsx";
import { StatusIcon } from "./StatusIcon.tsx";
import { BTN_PRIMARY } from "./Runnable.tsx";
import { compileWithPreloaded } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import type { Surface } from "../compiler/client.ts";
import type { ExplorerFile } from "../explorer/fileModel.ts";
import {
  createWorkspace,
  updateSource,
  setActive,
  activeFile,
  type Workspace,
} from "../explorer/workspace.ts";
import { treeItems, lowerWorkspace } from "../explorer/explorerView.ts";

/// A file as authored in a chapter's <Runnable files={…}/>. Same shape as ExplorerFile (the explorer's
/// model), so a chapter's file set and an explorer workspace are the same data — one multi-file model.
export interface RunnableFile {
  name: string;
  source: string;
  surface: Surface;
  entry?: boolean;
}

type Status =
  | { phase: "idle" }
  | { phase: "busy" }
  | { phase: "value"; text: string }
  | { phase: "error"; message: string };

export function MultiFileRunnable({
  files,
  expect = "value",
  expected,
  title,
}: {
  files: readonly RunnableFile[];
  expect?: "value" | "error";
  /** When set, the exact rendered value the file set must run to — the result pane shows it (and flags a
   *  mismatch), and the check-examples gate asserts the run result equals it. */
  expected?: string;
  title?: string;
}) {
  // Seed the workspace once from the authored files. A malformed set (no entry / dup names) is an AUTHORING
  // bug — surface it inline (and the check-examples gate compiles the set, so it can't ship broken).
  const seeded = useMemo(() => createWorkspace(files as ExplorerFile[]), [files]);
  const [ws, setWs] = useState<Workspace | null>(seeded.ok ? seeded.workspace : null);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const [ideOn, setIdeOn] = useState(false);
  const runningRef = useRef(false);
  const wsRef = useRef(ws);
  wsRef.current = ws;

  // The active buffer's IDE preload: the OTHER files, so a buffer that `import`s a sibling doesn't fault the
  // imported names as unbound (same as ExplorerPage's ide). `prepare` is identity — each file is a COMPLETE
  // module (its own imports/exports), so there's no wrapping to map spans through.
  const ide = useMemo(
    () => ({
      surface: () => activeFile(wsRef.current!)?.surface ?? ("sexpr" as Surface),
      prepare: (editorText: string) => ({ compiled: editorText, wrapPrefixBytes: 0 }),
      preload: () => {
        const cur = wsRef.current!;
        const others = cur.files.filter((f) => f.name !== cur.activeName);
        return {
          names: others.map((f) => f.name),
          sources: others.map((f) => f.source),
          formats: others.map((f) => f.surface),
        };
      },
    }),
    [],
  );

  const onRun = useCallback(async () => {
    if (runningRef.current || !wsRef.current) return;
    const lowered = lowerWorkspace(wsRef.current);
    if (!lowered.ok) {
      setStatus({ phase: "error", message: lowered.reason });
      return;
    }
    runningRef.current = true;
    setStatus({ phase: "busy" });
    try {
      const { text, from, names, sources, formats } = lowered.lowered;
      const out = await compileWithPreloaded(text, from, names, sources, formats);
      if (!out.component) {
        const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
        setStatus({ phase: "error", message: d ? `${d.code} ${d.message}`.trim() : "compile declined" });
        return;
      }
      // `expect="error"` ⇒ this example is SUPPOSED to trap; tell the runner so a stale-runtime mismatch
      // shows the REAL trap rather than the misleading stale-build/hard-reload advice.
      const result = await runComponent(out.component, "sexpr", false, undefined, expect === "error");
      if (result.kind !== "value") {
        const msg =
          result.kind === "trap" ? `trap: ${result.message}`
          : result.kind === "timeout" ? "timed out — a possible infinite loop was stopped after 5s."
          : `error: ${result.message}`;
        setStatus({ phase: "error", message: msg });
        return;
      }
      setStatus({ phase: "value", text: result.text });
    } catch (e) {
      setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
    } finally {
      runningRef.current = false;
    }
  }, []);

  if (!ws) {
    return (
      <div className="my-6 rounded-xl border border-rose-700/60 bg-rose-950/30 px-4 py-3 font-mono text-[13px] text-rose-300">
        multi-file example is malformed: {seeded.ok ? "" : seeded.reason}
      </div>
    );
  }

  const items = treeItems(ws);
  const active = activeFile(ws);
  const busy = status.phase === "busy";

  return (
    <div className="my-6 overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/70 shadow-lg">
      <div className="flex items-center justify-between border-b border-slate-700/60 bg-slate-800/50 px-3 py-1.5">
        <span className="text-xs font-medium text-slate-400">{title ?? "Multi-file example"}</span>
        <button onClick={() => void onRun()} disabled={busy} className={BTN_PRIMARY}>
          {busy ? "Running…" : "▶ Run"}
        </button>
      </div>

      {/* Tab strip — one per file in model order; the entry (the program that runs) is marked, active tab
          highlighted. This IS the operator's point: the reader SEES the events/reducer boundary as files. */}
      <div className="flex flex-wrap items-center gap-1 border-b border-slate-700/60 bg-slate-800/30 px-2 py-1" data-testid="mfr-tabs">
        {items.map((it) => (
          <button
            key={it.name}
            onClick={() => setWs((w) => { const r = setActive(w!, it.name); return r.ok ? r.workspace : w; })}
            className={
              "flex items-center gap-1 rounded px-2 py-0.5 text-xs " +
              (it.isActive ? "bg-slate-700 text-slate-100" : "text-slate-400 hover:text-slate-200")
            }
          >
            {it.isEntry && <span title="entry (the program that runs)" className="text-cadenza-400">▶</span>}
            <span className="font-mono">{it.name}</span>
          </button>
        ))}
      </div>

      <div onFocusCapture={() => setIdeOn(true)}>
        {active && (
          <CodeEditor
            key={active.name}
            value={active.source}
            onChange={(v) => setWs((w) => { const r = updateSource(w!, active.name, v); return r.ok ? r.workspace : w; })}
            ide={ideOn ? ide : undefined}
          />
        )}
      </div>

      {status.phase !== "idle" && (
        <StatusLine status={status} expect={expect} expected={expected} />
      )}
    </div>
  );
}

function StatusLine({ status, expect, expected }: { status: Status; expect: "value" | "error"; expected?: string }) {
  if (status.phase === "busy") {
    return (
      <div className="border-t border-slate-700/60 bg-slate-800/40 px-4 py-2.5 font-mono text-[13px] text-slate-400">
        Compiling &amp; running…
      </div>
    );
  }
  if (status.phase === "value") {
    // When the author pinned an `expected` value, compare the run result against it: a match reads as a
    // confirmed assertion (green), a mismatch as a problem (rose) — the same VALUE the check-examples gate
    // asserts, shown to the reader. Normalize layout (newlines→space) so a pinned compound is stable.
    const norm = (s: string) => s.replace(/\s*\n\s*/g, " ").trim();
    const mismatch = expected != null && norm(status.text) !== norm(expected);
    return (
      <div className={`flex items-start gap-2 border-t border-slate-700/60 px-4 py-2.5 font-mono text-[13px] ${mismatch ? "bg-rose-950/30 text-rose-300" : "bg-emerald-950/30 text-emerald-300"}`}>
        <span className="mt-0.5 shrink-0"><StatusIcon kind={mismatch ? "error" : "ok"} /></span>
        <div className="min-w-0">
          <code>{status.text}</code>
          {mismatch && <div className="mt-1 text-rose-400/80">expected: <code>{expected}</code></div>}
        </div>
      </div>
    );
  }
  if (status.phase === "error") {
    // An expect="error" example WANTS the decline — render it as the intended outcome, not a problem.
    const expected = expect === "error";
    return (
      <div className={`flex items-start gap-2 border-t border-slate-700/60 px-4 py-2.5 font-mono text-[13px] ${expected ? "bg-sky-950/30 text-sky-300" : "bg-rose-950/30 text-rose-300"}`}>
        <span className="mt-0.5 shrink-0"><StatusIcon kind={expected ? "declined" : "error"} /></span>
        <div className="min-w-0">{status.message}</div>
      </div>
    );
  }
  return null;
}
