/// The `/explorer` route — the in-browser PLATFORM EXPLORER's first user-visible surface (E1-panel): a
/// multi-file editor (tab strip + per-file buffers) over the pure workspace reducer, that compiles the whole
/// file set with the genesis file as `text` and the rest link-merged, then runs it. This is the multi-file
/// generalization of /music + /cad (which drive `compileWithPreloaded` with a FIXED preloaded library): here
/// the reader's OWN files are the preloaded modules, so a genesis that `import`s a sibling file just works.
///
/// THE SPLIT: this vertical (guide-infra) owns the route + shell + the machinery (the pure model/reducer/
/// projections in fileModel.ts + workspace.ts + explorerView.ts, all node-tested, and this thin React panel
/// projecting over them). The panel holds NO invariant logic — every mutation goes through a reducer
/// transition (addFile/deleteFile/setEntry/updateSource/setActive), each returning an ok/reason result the
/// panel surfaces inline. Later increments layer the System Inspector (session logs / CAS / metrics) on top.
///
/// SURFACE: each file carries its own surface (ml | sexpr) and its COMPLETE module text (imports + exports
/// visible — unlike /music, which hides the import boilerplate). The compiled value is run + rendered in
/// s-expr (the canonical machine form), matching /cad + /music.

import { useCallback, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compileWithPreloaded } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";
import type { Surface } from "../compiler/client.ts";
import {
  addFile,
  deleteFile,
  setEntry,
  setActive,
  updateSource,
  activeFile,
  type Workspace,
} from "./workspace.ts";
import { starterWorkspace, treeItems, lowerWorkspace } from "./explorerView.ts";

// The run outcome. `value` = the program's rendered result text; `error` = a compile decline / run trap /
// an un-lowerable file set (e.g. no entry) reported as a human-readable reason.
type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "value"; text: string }
  | { phase: "error"; message: string };

export default function ExplorerPage() {
  const [ws, setWs] = useState<Workspace>(starterWorkspace);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const [notice, setNotice] = useState<string | null>(null);
  const runningRef = useRef(false);

  const items = treeItems(ws);
  const active = activeFile(ws);

  // Apply a reducer transition: commit the next workspace on ok, or surface the decline reason inline (a
  // transient notice) on reject — the panel never mutates the file set directly, so the invariants (unique
  // names / one entry / live active tab) hold by construction.
  const apply = useCallback((r: { ok: true; workspace: Workspace } | { ok: false; reason: string }) => {
    if (r.ok) {
      setWs(r.workspace);
      setNotice(null);
    } else {
      setNotice(r.reason);
    }
  }, []);

  // The IDE linter config for the active buffer: diagnose it in its own surface, with the OTHER files
  // link-merged as preload so a buffer that `import`s a sibling doesn't fault the imported names as unbound.
  // Read through a live ref (like /music's musicIde) so the extension array isn't rebuilt when the active
  // file / surface changes. `prepare` is identity — each explorer file is a COMPLETE module (its own imports
  // + exports), so there's no wrapping to map spans through (wrapPrefixBytes = 0).
  const wsRef = useRef(ws);
  wsRef.current = ws;
  const ide = useMemo(
    () => ({
      surface: () => activeFile(wsRef.current)?.surface ?? ("sexpr" as Surface),
      prepare: (editorText: string) => ({ compiled: editorText, wrapPrefixBytes: 0 }),
      preload: () => {
        const cur = wsRef.current;
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

  // THE run path: lower the whole file set (genesis = text, the rest preloaded), compile, run, render the
  // value in s-expr. An un-lowerable set (no entry / dup name) is reported as the lowering reason, never a
  // crash. Serialized via runningRef so a double-click can't overlap two runs.
  const onRun = useCallback(async () => {
    if (runningRef.current) return;
    const lowered = lowerWorkspace(wsRef.current);
    if (!lowered.ok) {
      setStatus({ phase: "error", message: lowered.reason });
      return;
    }
    runningRef.current = true;
    setStatus({ phase: "running" });
    try {
      const { text, from, names, sources, formats } = lowered.lowered;
      const out = await compileWithPreloaded(text, from, names, sources, formats);
      if (!out.component) {
        const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
        setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
        return;
      }
      const result = await runComponent(out.component, "sexpr");
      if (result.kind !== "value") {
        const msg =
          result.kind === "trap" ? `trap: ${result.message}`
          : result.kind === "timeout" ? "timed out"
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

  const onAddFile = () => {
    const name = window.prompt("New file name (the import link target):")?.trim();
    if (!name) return;
    apply(addFile(ws, name, active?.surface ?? "sexpr"));
  };

  return (
    <article className="mx-auto flex max-w-5xl flex-col gap-4 px-4 py-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-2xl font-bold">Platform explorer</h1>
        <Link to="/" className="text-sm text-cadenza-400 hover:underline">← guide</Link>
      </div>
      <p className="text-sm text-slate-400">
        A multi-file workspace compiled + run in your browser: one file is the <em>entry</em> (the genesis
        program), the rest are link-merged modules it can <code>import</code>. Edit any file, mark the entry,
        and Run — the whole set compiles together.
      </p>

      {/* Tab strip — one row per file in model order; the entry is marked, the active tab highlighted. */}
      <div className="flex flex-wrap items-center gap-1" data-testid="explorer-tabs">
        {items.map((it) => (
          <button
            key={it.name}
            onClick={() => apply(setActive(ws, it.name))}
            className={
              "flex items-center gap-1 rounded-t border-b-2 px-2 py-1 text-xs " +
              (it.isActive
                ? "border-cadenza-500 bg-slate-800 text-slate-100"
                : "border-transparent bg-slate-900 text-slate-400 hover:text-slate-200")
            }
          >
            {it.isEntry && <span title="entry (genesis)" className="text-cadenza-400">▶</span>}
            <span className="font-mono">{it.name}</span>
            <span className="text-[10px] uppercase text-slate-500">{it.surface}</span>
          </button>
        ))}
        <button
          onClick={onAddFile}
          className="rounded px-2 py-1 text-xs text-slate-400 hover:text-slate-100"
          data-testid="explorer-add-file"
        >+ file</button>
      </div>

      {notice && <div className="text-xs text-amber-300" data-testid="explorer-notice">{notice}</div>}

      <div className="flex flex-col gap-4 md:flex-row">
        <div className="flex flex-col gap-2 md:min-w-0 md:flex-[2]">
          {active ? (
            <LazyCodeEditor
              key={active.name}
              value={active.source}
              onChange={(v) => apply(updateSource(ws, active.name, v))}
              ide={ide}
              minHeight="12rem"
            />
          ) : (
            <div className="text-sm text-slate-500">No file selected.</div>
          )}

          {/* Per-file actions on the active file: make it the entry, or delete it. */}
          <div className="flex items-center gap-2">
            {active && !active.entry && (
              <button
                onClick={() => apply(setEntry(ws, active.name))}
                className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800"
                data-testid="explorer-set-entry"
              >Make entry</button>
            )}
            {active && !active.entry && (
              <button
                onClick={() => apply(deleteFile(ws, active.name))}
                className="rounded border border-slate-700 px-2 py-1 text-xs text-rose-300 hover:bg-slate-800"
                data-testid="explorer-delete-file"
              >Delete</button>
            )}
            <button
              onClick={() => void onRun()}
              disabled={status.phase === "running"}
              className="ml-auto min-h-11 rounded bg-cadenza-600 px-3 text-xs font-semibold text-white enabled:hover:bg-cadenza-500 disabled:opacity-40 sm:min-h-0 sm:py-1"
            >▶ Run</button>
          </div>
        </div>

        {/* Result pane — the run value, or a compile/run/lowering error. */}
        <div data-testid="explorer-result" className="rounded-lg border border-slate-800 bg-slate-950 p-3 md:min-w-[18rem] md:flex-[3]">
          <div className="font-mono text-xs" data-testid="explorer-status">
            {status.phase === "error" ? <span className="text-rose-300">{status.message}</span>
              : status.phase === "running" ? <span className="text-slate-400">running…</span>
              : status.phase === "value" ? <pre className="overflow-x-auto whitespace-pre-wrap text-slate-200">{status.text}</pre>
              : <span className="text-slate-500">Run to see the result.</span>}
          </div>
        </div>
      </div>
    </article>
  );
}
