/// The full-screen playground: a toolbar, a resizable editor|output split, and a status bar. It ties
/// together the shared IDE machinery (as-you-type diagnostics + hover live in PlaygroundEditor) with
/// Run (reusing the guide's runner), the surface toggle, an examples dropdown, share links, and an
/// AST view.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { Link } from "react-router-dom";
import { PlaygroundEditor } from "./PlaygroundEditor.tsx";
import { OutputPanel, type RunView, type CompiledInfo } from "./OutputPanel.tsx";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { decodeShareHash, encodeShareHash } from "./share.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { compile, renderSyntax, type Diag, type Surface } from "../compiler/client.ts";
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
  const viewRef = useRef<EditorView | null>(null);
  // The surface the current `text` is written in — so a toggle re-renders the buffer (preserving
  // edits) rather than trying to parse it in the wrong surface.
  const shownSurface = useRef(surface);
  // Becomes true once the mount effect has chosen the starting buffer, so the persist effect doesn't
  // save the initial default over a restored buffer.
  const seeded = useRef(false);

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
    // runtime carries the well-known import name in its bytes; a scalar one is self-contained.
    const marker = new TextEncoder().encode("cadenza:runtime/heap");
    setCompiled({ bytes: out.component.length, importsRuntime: indexOfBytes(out.component, marker) >= 0 });
    const r = await runComponent(out.component);
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
    async (name: string) => {
      const ex = EXAMPLES.find((e) => e.name === name);
      if (!ex) return;
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

  const errorCount = useMemo(() => diags.filter((d) => d.error).length, [diags]);
  const warnCount = diags.length - errorCount;

  return (
    <div className="flex h-screen flex-col bg-slate-950 text-slate-200">
      {/* Toolbar */}
      <div className="flex items-center gap-2 border-b border-slate-800 px-3 py-2">
        <Link to="/" className="mr-1 text-sm font-bold tracking-tight text-slate-100">
          Cadenza
        </Link>
        <span className="mr-2 hidden text-xs text-slate-500 sm:inline">playground</span>
        <button
          onClick={doRun}
          className="rounded-md bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition hover:bg-cadenza-500"
        >
          ▶ Run
        </button>
        <button onClick={format} className="rounded px-2 py-1 text-xs text-slate-400 hover:bg-slate-800/60 hover:text-slate-200">
          Format
        </button>
        <select
          onChange={(e) => {
            void loadExample(e.target.value);
            e.target.selectedIndex = 0;
          }}
          className="rounded bg-slate-800/60 px-2 py-1 text-xs text-slate-300"
          defaultValue=""
        >
          <option value="" disabled>
            Examples…
          </option>
          {EXAMPLES.map((e) => (
            <option key={e.name} value={e.name}>
              {e.name}
            </option>
          ))}
        </select>
        <div className="ml-auto flex items-center gap-2">
          <SurfaceToggle surface={surface} setSurface={setSurface} />
          <button onClick={share} className="rounded px-2 py-1 text-xs text-slate-400 hover:bg-slate-800/60 hover:text-slate-200">
            Share
          </button>
        </div>
      </div>

      {/* Editor | Output */}
      <PanelGroup direction="horizontal" autoSaveId="cdz-playground" className="min-h-0 flex-1">
        <Panel defaultSize={55} minSize={30} className="min-w-0">
          <PlaygroundEditor
            value={text}
            onChange={setText}
            surface={surface}
            onDiagnostics={setDiags}
            onCursor={(line, col) => setCursor({ line, col })}
            onView={(v) => (viewRef.current = v)}
          />
        </Panel>
        <PanelResizeHandle className="w-1.5 bg-slate-800 transition hover:bg-cadenza-600/50" />
        <Panel defaultSize={45} minSize={25} className="min-w-0">
          <OutputPanel run={runView} diagnostics={diags} ast={ast} compiled={compiled} onJumpTo={jumpTo} />
        </Panel>
      </PanelGroup>

      {/* Status bar */}
      <div className="flex items-center gap-4 border-t border-slate-800 px-3 py-1 text-[11px] text-slate-500">
        <span>
          Ln {cursor.line}, Col {cursor.col}
        </span>
        <span className={errorCount > 0 ? "text-rose-400" : ""}>
          {errorCount} error{errorCount === 1 ? "" : "s"}
        </span>
        <span className={warnCount > 0 ? "text-amber-400" : ""}>
          {warnCount} warning{warnCount === 1 ? "" : "s"}
        </span>
        <span className="hidden text-slate-600 sm:inline">⌘/Ctrl↵ to run</span>
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
          className={
            "rounded-md px-2.5 py-0.5 text-xs font-medium transition " +
            (s === surface ? "bg-cadenza-600 text-white" : "text-slate-400 hover:text-slate-200")
          }
        >
          {s === "ml" ? "Conventional" : "S-expr"}
        </button>
      ))}
    </div>
  );
}
