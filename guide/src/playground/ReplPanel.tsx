/// The playground's mini-REPL: call any function your module defines by typing an expression in the
/// same surface you're editing in, and see its value — scalar or compound — rendered like a normal run.
///
/// It doesn't re-implement evaluation: `onEval` compiles the buffer's definitions + the typed
/// expression into one module (via `repl_eval`) and runs it through the very same pipeline the Run
/// button uses, so a REPL result is identical to what `main` would have produced had it been that
/// expression. The panel just owns the input, the call history, and keyboard affordances.

import { useEffect, useRef, useState } from "react";
import type { Surface } from "../compiler/client.ts";
import { StatusIcon } from "../components/StatusIcon.tsx";

/// One evaluated entry in the session history.
export interface ReplEntry {
  expr: string;
  /** The rendered outcome. */
  result:
    | { kind: "value"; text: string }
    | { kind: "trap"; message: string }
    | { kind: "timeout" }
    | { kind: "error"; message: string };
}

interface Props {
  /** The surface the expression input is written in (mirrors the editor's surface). */
  surface: Surface;
  /** Evaluate one expression against the current buffer; resolves to a history entry's `result`. */
  onEval: (expr: string) => Promise<ReplEntry["result"]>;
}

/// A one-line example prompt per surface, so the empty REPL isn't a blank void.
const PLACEHOLDER: Record<Surface, string> = {
  sexpr: "(a-function 1 2)",
  ml: "a-function(1, 2)",
};

export function ReplPanel({ surface, onEval }: Props) {
  const [expr, setExpr] = useState("");
  const [history, setHistory] = useState<ReplEntry[]>([]);
  const [busy, setBusy] = useState(false);
  // Index into a reverse walk of past expressions (arrow-up recall); -1 = editing a fresh line.
  const [recall, setRecall] = useState(-1);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Keep the newest entry in view as history grows.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [history, busy]);

  async function submit() {
    const trimmed = expr.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setExpr("");
    setRecall(-1);
    const result = await onEval(trimmed);
    setHistory((h) => [...h, { expr: trimmed, result }]);
    setBusy(false);
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
      return;
    }
    // Up/Down recall through prior expressions (only when the caret is on a single-line input).
    if (e.key === "ArrowUp" && history.length > 0) {
      e.preventDefault();
      const next = recall < 0 ? history.length - 1 : Math.max(0, recall - 1);
      setRecall(next);
      setExpr(history[next].expr);
    } else if (e.key === "ArrowDown" && recall >= 0) {
      e.preventDefault();
      const next = recall + 1;
      if (next >= history.length) {
        setRecall(-1);
        setExpr("");
      } else {
        setRecall(next);
        setExpr(history[next].expr);
      }
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div ref={scrollRef} className="min-h-0 flex-1 space-y-2 overflow-auto">
        {history.length === 0 && (
          <p className="text-slate-500">
            Call any function your module defines. Type an expression and press Enter — the result shows
            below, scalar or compound, in your chosen syntax.
          </p>
        )}
        {history.map((h, i) => (
          <div key={i} className="space-y-0.5">
            <div className="flex items-baseline gap-1.5">
              <span aria-hidden className="select-none text-cadenza-400">
                ›
              </span>
              <code className="text-slate-300">{h.expr}</code>
            </div>
            <ReplResult result={h.result} />
          </div>
        ))}
        {busy && <p className="text-slate-500">evaluating…</p>}
      </div>

      <div className="mt-2 flex items-center gap-1.5 border-t border-slate-800 pt-2">
        <span aria-hidden className="select-none text-cadenza-400">
          ›
        </span>
        <input
          value={expr}
          onChange={(e) => setExpr(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={PLACEHOLDER[surface]}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          className="min-w-0 flex-1 bg-transparent font-mono text-[13px] text-slate-100 placeholder:text-slate-600 focus:outline-none"
        />
        <button
          onClick={() => void submit()}
          disabled={busy || expr.trim() === ""}
          className="rounded bg-cadenza-600 px-2 py-0.5 text-[11px] font-semibold text-white transition enabled:hover:bg-cadenza-500 disabled:opacity-40"
        >
          Call
        </button>
      </div>
    </div>
  );
}

function ReplResult({ result }: { result: ReplEntry["result"] }) {
  switch (result.kind) {
    case "value":
      return (
        <div className="pl-4 text-emerald-300">
          <StatusIcon kind="ok" /> <code>{result.text}</code>
        </div>
      );
    case "trap":
      return (
        <div className="pl-4 text-amber-300">
          <StatusIcon kind="trap" /> trap: {result.message}
        </div>
      );
    case "timeout":
      return (
        <div className="pl-4 text-amber-300">
          <StatusIcon kind="trap" /> timed out — a possible infinite loop was stopped.
        </div>
      );
    case "error":
      return (
        <div className="pl-4 text-rose-300">
          <StatusIcon kind="error" /> {result.message}
        </div>
      );
  }
}
