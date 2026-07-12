/// A runnable, editable Cadenza example — the heart of the guide.
///
/// A snippet is authored ONCE in a canonical surface (`source` in `authoredIn`, default s-expr). It
/// is displayed in the reader's globally-chosen surface: when that changes, the current editor text
/// is re-rendered through the other printer (lossless — same AST). Run compiles the current text and
/// executes the component in the disposable run worker, showing the value / trap / diagnostics.
///
/// Snippets that are meant to fail (to teach a diagnostic) set `expect="error"`; the status chip then
/// reads a declined compile as success rather than a problem.

import { useCallback, useEffect, useRef, useState } from "react";
import { CodeEditor } from "../editor/CodeEditor.tsx";
import { compile, renderSyntax, type Diag } from "../compiler/client.ts";
import { run, type RunOutcome } from "../runner/client.ts";
import { useSyntax, type Surface } from "../syntax/SyntaxContext.tsx";

interface Props {
  /** The snippet source, in `authoredIn` surface. Wrapped in a module automatically if `wrap`. */
  source: string;
  /** Surface the `source` prop is written in. Default s-expr (the corpus form). */
  authoredIn?: Surface;
  /** Wrap a bare expression as `(module m (def (main) <expr>) (export main))`. Default true. */
  wrap?: boolean;
  /** What this example is meant to do — tunes the status chip. */
  expect?: "value" | "error";
  /** Optional caption shown above the editor. */
  title?: string;
}

type Status =
  | { phase: "idle" }
  | { phase: "compiling" }
  | { phase: "running" }
  | { phase: "ok"; text: string }
  | { phase: "trap"; message: string }
  | { phase: "timeout" }
  | { phase: "declined"; diags: Diag[] };

/// Wrap a bare expression into a minimal runnable module in the given surface. The compiler needs an
/// `(export …)`; a bare expression alone declines. Authored snippets are usually bare expressions.
function wrapModule(src: string, surface: Surface): string {
  const trimmed = src.trim();
  if (surface === "sexpr") {
    // Already a full module? leave it.
    if (/^\(module\b/.test(trimmed)) return trimmed;
    return `(module m (def (main) ${trimmed}) (export main))`;
  }
  // ML surface
  if (/^module\b/.test(trimmed)) return trimmed;
  return `module m {\n  def main() = ${trimmed}\n  export(main)\n}`;
}

export function Runnable({
  source,
  authoredIn = "sexpr",
  wrap = true,
  expect = "value",
  title,
}: Props) {
  const { surface } = useSyntax();
  const [text, setText] = useState(source);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  // Track the surface the editor text currently reflects, to re-render on toggle without clobbering
  // user edits with the original `source`.
  const shownSurface = useRef<Surface>(authoredIn);

  // Initial render: convert the authored source into the active surface once on mount.
  useEffect(() => {
    let cancelled = false;
    if (authoredIn !== surface) {
      renderSyntax(source, authoredIn, surface)
        .then((rendered) => {
          if (!cancelled) {
            setText(rendered);
            shownSurface.current = surface;
          }
        })
        .catch(() => {});
    } else {
      shownSurface.current = authoredIn;
    }
    return () => {
      cancelled = true;
    };
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // React to a global surface change: re-render the CURRENT text (preserving edits) into the new
  // surface. Guarded so we don't loop or re-render into the surface we already show.
  useEffect(() => {
    if (surface === shownSurface.current) return;
    const from = shownSurface.current;
    let cancelled = false;
    renderSyntax(text, from, surface)
      .then((rendered) => {
        if (!cancelled) {
          setText(rendered);
          shownSurface.current = surface;
        }
      })
      .catch(() => {
        // A mid-edit unparseable buffer can't be re-rendered; keep the text, update the marker so we
        // don't retry every keystroke.
        shownSurface.current = surface;
      });
    return () => {
      cancelled = true;
    };
  }, [surface, text]);

  const doRun = useCallback(async () => {
    setStatus({ phase: "compiling" });
    const program = wrap ? wrapModule(text, shownSurface.current) : text;
    const out = await compile(program, shownSurface.current);
    if (!out.component) {
      setStatus({ phase: "declined", diags: out.diagnostics });
      return;
    }
    setStatus({ phase: "running" });
    const result: RunOutcome = await run(out.component);
    switch (result.kind) {
      case "value":
        setStatus({ phase: "ok", text: result.text });
        break;
      case "trap":
        setStatus({ phase: "trap", message: result.message });
        break;
      case "timeout":
        setStatus({ phase: "timeout" });
        break;
      case "error":
        setStatus({ phase: "declined", diags: [{ error: true, code: "", message: result.message, node: -1 }] });
        break;
    }
  }, [text, wrap]);

  const reset = useCallback(() => {
    const target = shownSurface.current;
    if (authoredIn === target) {
      setText(source);
    } else {
      renderSyntax(source, authoredIn, target).then(setText).catch(() => setText(source));
    }
    setStatus({ phase: "idle" });
  }, [authoredIn, source]);

  const running = status.phase === "compiling" || status.phase === "running";

  return (
    <div className="my-6 overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/70 shadow-lg">
      <div className="flex items-center justify-between border-b border-slate-700/60 bg-slate-800/50 px-3 py-1.5">
        <span className="text-xs font-medium text-slate-400">
          {title ?? "Example"}
        </span>
        <div className="flex items-center gap-2">
          <button
            onClick={reset}
            className="rounded px-2 py-1 text-xs text-slate-400 transition hover:bg-slate-700/60 hover:text-slate-200"
          >
            Reset
          </button>
          <button
            onClick={doRun}
            disabled={running}
            className="rounded-md bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition hover:bg-cadenza-500 disabled:opacity-50"
          >
            {running ? "Running…" : "▶ Run"}
          </button>
        </div>
      </div>

      <CodeEditor value={text} onChange={setText} />

      <StatusPane status={status} expect={expect} />
    </div>
  );
}

function StatusPane({ status, expect }: { status: Status; expect: "value" | "error" }) {
  if (status.phase === "idle") return null;

  let body: React.ReactNode = null;
  let tone = "text-slate-300";
  let bg = "bg-slate-800/40";

  switch (status.phase) {
    case "compiling":
      body = "Compiling…";
      break;
    case "running":
      body = "Running…";
      break;
    case "ok":
      tone = "text-emerald-300";
      bg = "bg-emerald-950/30";
      body = <code>{status.text}</code>;
      break;
    case "trap":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      body = <>trap: {status.message}</>;
      break;
    case "timeout":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      body = "timed out — a possible infinite loop was stopped after 5s.";
      break;
    case "declined": {
      const isExpected = expect === "error";
      tone = isExpected ? "text-sky-300" : "text-rose-300";
      bg = isExpected ? "bg-sky-950/30" : "bg-rose-950/30";
      body = (
        <div className="space-y-1">
          {isExpected && <div className="text-xs opacity-70">The compiler declined this program (as this example intends):</div>}
          {status.diags.map((d, i) => (
            <div key={i}>
              {d.code && <span className="font-semibold">{d.code} </span>}
              {d.message}
            </div>
          ))}
        </div>
      );
      break;
    }
  }

  return (
    <div className={`border-t border-slate-700/60 px-4 py-2.5 font-mono text-[13px] ${bg} ${tone}`}>
      {body}
    </div>
  );
}
