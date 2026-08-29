/// The full-screen playground: a toolbar, a resizable editor|output split, and a status bar. It ties
/// together the shared IDE machinery (as-you-type diagnostics + hover live in PlaygroundEditor) with
/// Run (reusing the guide's runner), the surface toggle, an examples dropdown, share links, and an
/// AST view.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { useMediaQuery } from "./useMediaQuery.ts";
import { Link } from "react-router-dom";
import { PlaygroundEditor } from "./PlaygroundEditor.tsx";
import { OutputPanel, type RunView, type CompiledInfo } from "./OutputPanel.tsx";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { decodeShareHash, encodeShareHash } from "./share.ts";
import { readExampleParam, writeExampleParam } from "../components/exampleParam.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { compile, renderSyntax, emitRust, emitCadenza, coreModule, replEval, definedNames, type Diag, type Surface } from "../compiler/client.ts";
import type { ReplEntry } from "./ReplPanel.tsx";
import { toWat } from "./wat.ts";
import { applyFix } from "./applyFix.ts";
import type { CompiledView } from "./OutputPanel.tsx";
import { run as runComponent } from "../runner/client.ts";
import type { EditorView } from "@codemirror/view";
import { byteToUtf16 } from "./offsets.ts";

const BUFFER_KEY = "cadenza.playground.buffer";

/// The buffer to open with, decided SYNCHRONOUSLY at first render (no effect race): a shared link
/// (URL hash) wins, then the reader's last saved buffer, else the default example. Returns the source
/// and the surface it's written in; the caller aligns the global surface toggle to match.
function initialBuffer(): { src: string; surface: Surface } {
  const shared = decodeShareHash(window.location.hash);
  if (shared) return { src: shared.src, surface: shared.s };
  // A `?example=<id>` deep-link (operator: per-example nav) opens the playground with THAT example. Below
  // the share hash (a shared edit wins), ABOVE the saved buffer + default: a deep-link is a deliberate
  // "show me this example" that should override the last-session buffer. Unknown id → fall through.
  const reqId = readExampleParam();
  if (reqId) {
    const ex = EXAMPLES.find((e) => e.id === reqId);
    if (ex) return { src: ex.source, surface: ex.surface };
  }
  try {
    const saved = localStorage.getItem(BUFFER_KEY);
    if (saved) {
      const { s, src } = JSON.parse(saved) as { s: Surface; src: string };
      if (typeof src === "string" && src.trim() && (s === "ml" || s === "sexpr")) {
        return { src, surface: s };
      }
    }
  } catch {
    /* fall through */
  }
  return { src: DEFAULT_EXAMPLE.source, surface: DEFAULT_EXAMPLE.surface };
}

export default function PlaygroundPage() {
  const { surface, setSurface } = useSyntax();
  const [initial] = useState(initialBuffer);
  const [text, setText] = useState<string>(initial.src);
  const [runView, setRunView] = useState<RunView>({ kind: "idle" });
  const [diags, setDiags] = useState<Diag[]>([]);
  const [ast, setAst] = useState<string>("");
  const [compiled, setCompiled] = useState<CompiledInfo | null>(null);
  const [cursor, setCursor] = useState<{ line: number; col: number }>({ line: 1, col: 1 });
  // Wide screens split editor|output side-by-side; narrow ones stack them (editor above output).
  const isWide = useMediaQuery("(min-width: 768px)");
  const viewRef = useRef<EditorView | null>(null);
  // The last successfully-emitted component bytes, kept for lazily rendering the WAT view.
  const lastComponent = useRef<Uint8Array | null>(null);
  // The exact `(text, surface)` of the last successful compile. The Compiled views (WAT/Rust) recompile
  // from source, so they must use the SAME inputs the run did — not the live `text`/`shownSurface`,
  // which can drift from what produced `lastComponent` (e.g. `shownSurface` briefly lags the reactive
  // surface right after a share-hash seed, so the buffer's real surface and the ref disagree).
  const lastRun = useRef<{ text: string; surface: Surface }>({ text: initial.src, surface: initial.surface });
  // The surface the current `text` is written in — so a toggle re-renders the buffer (preserving
  // edits) rather than trying to parse it in the wrong surface. Initialized to the SEEDED buffer's
  // surface (known synchronously), NOT the reactive `surface` (which defaults to ML and only reconciles
  // in the mount effect below) — otherwise a callback firing before that effect (e.g. a REPL call on a
  // freshly-shared s-expr buffer) would parse the buffer in the wrong surface.
  const shownSurface = useRef(initial.surface);
  // Becomes true once the mount effect has chosen the starting buffer, so the persist effect doesn't
  // save the initial default over a restored buffer.
  const seeded = useRef(false);
  // Guards the surface-toggle effect against its mount invocation: on mount the buffer already matches
  // `shownSurface` (both are the seeded surface), so there's nothing to convert — and converting then
  // would race the mount effect's `setSurface`, corrupting the buffer (parsing s-expr as ML). Only a
  // genuine post-mount toggle should re-render the buffer.
  const didMountSurface = useRef(false);

  // Seed the buffer on first mount, in priority order: a shared link (URL hash) → the reader's last
  // saved buffer (localStorage) → the default example rendered into the active surface (the default is
  // authored in s-expr, but the surface toggle may be set to ML from the guide).
  useEffect(() => {
    // `text` is already `initial.src` (a synchronous lazy init). Align the global surface toggle to
    // the buffer's surface so the two agree; the surface-toggle effect below then won't re-render it.
    shownSurface.current = initial.surface;
    if (surface !== initial.surface) setSurface(initial.surface);
    seeded.current = true;
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Persist the buffer so a reload restores the reader's work. Gated on `seeded` so the initial
  // render's default `text` doesn't clobber the saved buffer BEFORE the mount effect above restores it
  // (the mount effect sets `seeded.current = true` once it has decided the starting text).
  useEffect(() => {
    if (!seeded.current) return;
    try {
      localStorage.setItem(BUFFER_KEY, JSON.stringify({ s: shownSurface.current, src: text }));
    } catch {
      /* storage full/disabled — persistence is a nicety */
    }
  }, [text, surface]);

  // On a global surface toggle, re-render the current buffer into the new surface (preserving edits).
  useEffect(() => {
    // Skip the mount invocation (see `didMountSurface`): the seeded buffer already matches
    // `shownSurface` (both `initial.surface`), and converting here would race the mount reconciliation.
    if (!didMountSurface.current) {
      didMountSurface.current = true;
      shownSurface.current = initial.surface;
      return;
    }
    if (surface === shownSurface.current) return;
    const from = shownSurface.current;
    let cancelled = false;
    renderSyntax(text, from, surface)
      .then((r) => {
        if (!cancelled) setText(r);
        shownSurface.current = surface;
      })
      .catch(() => {
        shownSurface.current = surface; // unparseable mid-edit: keep text, stop retrying
      });
    return () => {
      cancelled = true;
    };
  }, [surface, text]);

  // Keep the AST view in sync with the buffer, rendered from the surface the buffer is CURRENTLY in
  // (cheap; the debounce lives in the diagnostics loop).
  useEffect(() => {
    let cancelled = false;
    renderSyntax(text, shownSurface.current, "debug")
      .then((a) => !cancelled && setAst(a))
      .catch(() => !cancelled && setAst(""));
    return () => {
      cancelled = true;
    };
  }, [text, surface]);

  const doRun = useCallback(async () => {
    setRunView({ kind: "busy" });
    const out = await compile(text, surface);
    if (!out.component) {
      setCompiled(null);
      const firstErr = out.diagnostics.find((d) => d.error);
      setRunView({ kind: "error", message: firstErr ? `${firstErr.code || ""} ${firstErr.message}`.trim() : "declined" });
      return;
    }
    // Summarize what it compiled to (for the Compiled tab): a component that imports the value-heap
    // runtime carries the well-known import name in its bytes; a scalar one is self-contained. The
    // WAT / Rust sub-views are filled lazily (onNeedCompiledView) when the reader opens them.
    const marker = new TextEncoder().encode("cadenza:runtime/heap");
    lastComponent.current = out.component;
    lastRun.current = { text, surface }; // the exact inputs the Compiled views recompile from
    setCompiled({
      bytes: out.component.length,
      importsRuntime: indexOfBytes(out.component, marker) >= 0,
      wat: null,
      rustSync: null,
      rustAsync: null,
      cadenza: null,
    });
    const r = await runComponent(out.component, shownSurface.current);
    switch (r.kind) {
      case "value":
        setRunView({ kind: "value", text: r.text });
        break;
      case "trap":
        setRunView({ kind: "trap", message: r.message });
        break;
      case "timeout":
        setRunView({ kind: "timeout" });
        break;
      case "error":
        setRunView({ kind: "error", message: r.message });
        break;
    }
  }, [text, surface]);

  // Fill a Compiled sub-view (WAT / Rust / Rust-async) on demand — computed lazily when the reader
  // opens it, so a run doesn't pay for views nobody looks at.
  const needCompiledView = useCallback(
    async (view: CompiledView) => {
      // The Compiled views recompile from the EXACT inputs the run used (`lastRun`), so they can't drift
      // from what `lastComponent` was built from (see `lastRun`'s note re: the share-hash surface race).
      const { text: src, surface: srf } = lastRun.current;
      if (view === "wat") {
        // Show the executed CORE MODULE, not the component wrapper — and DWARF-free (the debug info is
        // for the browser debugger, just noise in human-readable WAT). `coreModule` recompiles the
        // program to a plain (no-DWARF) component and unwraps it to the embedded core module bytes,
        // which we then print. Falls back to the run's component only if the unwrap declines.
        const core = await coreModule(src, srf).catch(() => null);
        const bytes = core ?? lastComponent.current;
        if (!bytes) return;
        const wat = await toWat(bytes);
        setCompiled((c) => (c ? { ...c, wat } : c));
      } else if (view === "rust") {
        const rustSync = await emitRust(src, srf, false).catch((e) => `// error: ${e}`);
        setCompiled((c) => (c ? { ...c, rustSync } : c));
      } else if (view === "rustAsync") {
        const rustAsync = await emitRust(src, srf, true).catch((e) => `// error: ${e}`);
        setCompiled((c) => (c ? { ...c, rustAsync } : c));
      } else if (view === "cadenza") {
        // The lowered-optimized Cadenza (`--target cadenza`), printed as sexpr. A declined program comes
        // back as a `; declined: …` note (not an error) — shown verbatim.
        const cadenza = await emitCadenza(src, srf, "sexpr").catch((e) => `; error: ${e}`);
        setCompiled((c) => (c ? { ...c, cadenza } : c));
      }
    },
    [],
  );

  // Evaluate one REPL expression against the current buffer: compile buffer-defs + expr into one
  // module (`replEval`), then run the emitted component through the SAME run path the Run button uses,
  // rendering the value in the buffer's surface. A compile decline surfaces the first error's message.
  const replCall = useCallback(
    async (expr: string): Promise<ReplEntry["result"]> => {
      const srf = shownSurface.current;
      const out = await replEval(text, expr, srf);
      if (!out.component) {
        const firstErr = out.diagnostics.find((d) => d.error);
        return { kind: "error", message: firstErr ? `${firstErr.code || ""} ${firstErr.message}`.trim() : "declined" };
      }
      const r = await runComponent(out.component, srf);
      switch (r.kind) {
        case "value":
          return { kind: "value", text: r.text };
        case "trap":
          return { kind: "trap", message: r.message };
        case "timeout":
          return { kind: "timeout" };
        default:
          return { kind: "error", message: r.message };
      }
    },
    [text],
  );

  // The names the REPL can complete: every top-level definition the current buffer declares. Fetched
  // on demand (when the REPL input focuses) so it always reflects the latest edits without a subscription.
  const replNames = useCallback(
    () => definedNames(text, shownSurface.current).catch(() => [] as string[]),
    [text],
  );

  // ⌘/Ctrl-Enter runs.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void doRun();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doRun]);

  const format = useCallback(async () => {
    try {
      setText(await renderSyntax(text, surface, surface));
    } catch {
      /* a mid-edit unparseable buffer can't be formatted — leave it */
    }
  }, [text, surface]);

  const share = useCallback(async () => {
    const hash = encodeShareHash({ s: surface, src: text });
    const url = `${location.origin}${location.pathname}#${hash}`;
    window.history.replaceState(null, "", url);
    try {
      await navigator.clipboard.writeText(url);
    } catch {
      /* clipboard may be unavailable; the URL bar still holds it */
    }
  }, [text, surface]);

  const loadExample = useCallback(
    async (id: string) => {
      const ex = EXAMPLES.find((e) => e.id === id);
      if (!ex) return;
      writeExampleParam(id); // reflect the picked example in the URL (?example=…) — a copy-shareable deep-link
      // Examples are authored in s-expr; render into the active surface so the toggle stays honored.
      const src = ex.surface === surface ? ex.source : await renderSyntax(ex.source, ex.surface, surface).catch(() => ex.source);
      setText(src);
      shownSurface.current = surface;
      setRunView({ kind: "idle" });
    },
    [surface],
  );

  const jumpTo = useCallback((fromByte: number, toByte: number) => {
    const v = viewRef.current;
    if (!v) return;
    const doc = v.state.doc.toString();
    const from = byteToUtf16(doc, fromByte);
    const to = byteToUtf16(doc, toByte);
    v.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
    v.focus();
  }, []);

  // Apply a diagnostic's structural fix from the Diagnostics panel. The playground compiles its buffer
  // VERBATIM (a full program, no scaffolding), so the fix's byte range maps directly (prefix 0). Edits
  // via the editor view when present so the change is a proper CodeMirror transaction (undoable); falls
  // back to `setText` otherwise.
  const applyFixToBuffer = useCallback((d: Diag) => {
    if (!d.fix) return;
    const doc = viewRef.current?.state.doc.toString() ?? text;
    const next = applyFix(doc, d.fix, 0);
    if (next == null) return;
    const v = viewRef.current;
    if (v) {
      v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: next } });
      v.focus();
    } else {
      setText(next);
    }
  }, [text]);

  const errorCount = useMemo(() => diags.filter((d) => d.error).length, [diags]);
  const warnCount = diags.length - errorCount;

  return (
    <div className="flex h-screen flex-col bg-slate-950 text-slate-200">
      {/* Toolbar — wraps on narrow screens so every control stays reachable. */}
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-800 px-3 py-2">
        <Link to="/" className="mr-1 flex min-h-11 items-center text-sm font-bold tracking-tight text-slate-100 sm:min-h-0">
          Cadenza
        </Link>
        <span className="mr-2 hidden text-xs text-slate-500 sm:inline">playground</span>
        <button
          onClick={doRun}
          className="flex min-h-11 items-center justify-center rounded-md bg-cadenza-600 px-3 text-xs font-semibold text-white transition hover:bg-cadenza-500 sm:min-h-0 sm:py-1"
        >
          ▶ Run
        </button>
        <button onClick={format} className="flex min-h-11 items-center rounded px-2 text-xs text-slate-400 hover:bg-slate-800/60 hover:text-slate-200 sm:min-h-0 sm:py-1">
          Format
        </button>
        <select
          onChange={(e) => {
            void loadExample(e.target.value);
            e.target.selectedIndex = 0;
          }}
          className="min-h-11 rounded bg-slate-800/60 px-2 text-xs text-slate-300 sm:min-h-0 sm:py-1"
          defaultValue=""
        >
          <option value="" disabled>
            Examples…
          </option>
          {EXAMPLES.map((e) => (
            <option key={e.id} value={e.id}>
              {e.name}
            </option>
          ))}
        </select>
        <div className="ml-auto flex items-center gap-2">
          <SurfaceToggle surface={surface} setSurface={setSurface} />
          <button onClick={share} className="flex min-h-11 items-center rounded px-2 text-xs text-slate-400 hover:bg-slate-800/60 hover:text-slate-200 sm:min-h-0 sm:py-1">
            Share
          </button>
        </div>
      </div>

      {/* Editor + Output: side-by-side on wide screens, stacked (editor above output) on narrow ones.
          Keyed by orientation + persisted separately so each layout keeps its own remembered sizes and
          re-lays out cleanly when the viewport crosses the breakpoint. */}
      <PanelGroup
        key={isWide ? "h" : "v"}
        direction={isWide ? "horizontal" : "vertical"}
        autoSaveId={isWide ? "cdz-playground-h" : "cdz-playground-v"}
        className="min-h-0 flex-1"
      >
        <Panel defaultSize={55} minSize={25} className="min-w-0">
          <PlaygroundEditor
            value={text}
            onChange={setText}
            surface={surface}
            onDiagnostics={setDiags}
            onCursor={(line, col) => setCursor({ line, col })}
            onView={(v) => (viewRef.current = v)}
          />
        </Panel>
        <PanelResizeHandle
          className={
            "bg-slate-800 transition hover:bg-cadenza-600/50 " + (isWide ? "w-1.5" : "h-1.5")
          }
        />
        <Panel defaultSize={45} minSize={25} className="min-w-0">
          <OutputPanel
            run={runView}
            diagnostics={diags}
            ast={ast}
            compiled={compiled}
            surface={surface}
            onReplEval={replCall}
            onReplNames={replNames}
            onJumpTo={jumpTo}
            onApplyFix={applyFixToBuffer}
            onNeedCompiledView={needCompiledView}
          />
        </Panel>
      </PanelGroup>

      {/* Status bar — a dense 11px IDE info strip (Ln/Col, error/warning counts, footer credits). Its
          links are NOT primary controls (an IDE status-line pattern), so they're exempt from the 44px
          mobile tap-target floor — marked for the check:visual `tapTargetsExcept`. */}
      <div data-testid="status-bar" className="flex items-center gap-4 border-t border-slate-800 px-3 py-1 text-[11px] text-slate-500">
        <span>
          Ln {cursor.line}, Col {cursor.col}
        </span>
        <span className={errorCount > 0 ? "text-rose-400" : ""}>
          {errorCount} error{errorCount === 1 ? "" : "s"}
        </span>
        <span className={warnCount > 0 ? "text-amber-400" : ""}>
          {warnCount} warning{warnCount === 1 ? "" : "s"}
        </span>
        <span className="hidden text-slate-600 sm:inline">⌘/Ctrl↵ run · ⌘/Ctrl-click a name to jump to its definition</span>
        <span className="ml-auto">
          Cadenza · <Link to="/" className="hover:text-slate-300">home</Link> ·{" "}
          <Link to="/welcome" className="hover:text-slate-300">the guide</Link>
        </span>
      </div>
    </div>
  );
}

/// Find `needle` within `hay` (naive scan; both small — a component is a few KB, needle ~20 bytes).
function indexOfBytes(hay: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i + needle.length <= hay.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
    return i;
  }
  return -1;
}

function SurfaceToggle({ surface, setSurface }: { surface: "ml" | "sexpr"; setSurface: (s: "ml" | "sexpr") => void }) {
  return (
    <div className="inline-flex rounded-lg border border-slate-700/70 bg-slate-800/60 p-0.5">
      {(["ml", "sexpr"] as const).map((s) => (
        <button
          key={s}
          onClick={() => setSurface(s)}
          // Mobile touch target: 44px-tall segment below `sm`, compact at sm+.
          className={
            "flex min-h-11 items-center rounded-md px-2.5 text-xs font-medium transition sm:min-h-0 sm:py-0.5 " +
            (s === surface ? "bg-cadenza-600 text-white" : "text-slate-400 hover:text-slate-200")
          }
        >
          {s === "ml" ? "Conventional" : "S-expr"}
        </button>
      ))}
    </div>
  );
}
