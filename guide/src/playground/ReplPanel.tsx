/// The playground's mini-REPL: call any function your module defines by typing an expression in the
/// same surface you're editing in, and see its value — scalar or compound — rendered like a normal run.
///
/// It doesn't re-implement evaluation: `onEval` compiles the buffer's definitions + the typed
/// expression into one module (via `repl_eval`) and runs it through the very same pipeline the Run
/// button uses, so a REPL result is identical to what `main` would have produced had it been that
/// expression. The panel owns the input, the call history, keyboard affordances, and name completion.

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
  /** Fetch the names the REPL can complete (the buffer's definitions), refreshed on focus. */
  onNames: () => Promise<string[]>;
}

/// A one-line example prompt per surface, so the empty REPL isn't a blank void.
const PLACEHOLDER: Record<Surface, string> = {
  sexpr: "(a-function 1 2)",
  ml: "a-function(1, 2)",
};

/// Common prelude names worth completing even though they aren't in the buffer — the collection/text
/// operations and the standard sum constructors a reader reaches for. The buffer's own definitions are
/// merged in ahead of these (fetched via `onNames`), so a local name always wins a tie.
const PRELUDE_NAMES = [
  "List.push", "List.concat", "List.len", "List.at", "List.empty",
  "String.concat", "String.scalar-len", "String.byte-len", "String.at",
  "Option", "Result", "Some", "None", "Ok", "Err",
  "Float64.of-int", "Float32.of-int",
  "true", "false",
];

/// The identifier fragment ending at `caret` in `text` — the run of identifier characters (letters,
/// digits, `-`, `.`, `_`) immediately left of the caret. Empty when the caret isn't on an identifier.
/// Cadenza names are kebab-and-dotted (`String.scalar-len`), so `-`/`.` are part of the token.
function fragmentBefore(text: string, caret: number): { frag: string; start: number } {
  let start = caret;
  while (start > 0 && /[A-Za-z0-9_.\-]/.test(text[start - 1])) start--;
  return { frag: text.slice(start, caret), start };
}

export function ReplPanel({ surface, onEval, onNames }: Props) {
  const [expr, setExpr] = useState("");
  const [history, setHistory] = useState<ReplEntry[]>([]);
  const [busy, setBusy] = useState(false);
  // Index into a reverse walk of past expressions (arrow-up recall); -1 = editing a fresh line.
  const [recall, setRecall] = useState(-1);
  // Completion state: the candidate names, which are currently shown, and the highlighted one.
  const [names, setNames] = useState<string[]>([]);
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [sel, setSel] = useState(0);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Keep the newest entry in view as history grows.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [history, busy]);

  // Refresh the completion pool when the input gains focus (the buffer may have changed since last time).
  async function refreshNames() {
    const buf = await onNames().catch(() => [] as string[]);
    // Buffer names first (deduped), then prelude names not already present.
    const seen = new Set(buf);
    setNames([...buf, ...PRELUDE_NAMES.filter((n) => !seen.has(n))]);
  }

  /// Recompute the suggestion list from the current input + caret. Called on every edit.
  function recomputeSuggestions(value: string, caret: number) {
    const { frag } = fragmentBefore(value, caret);
    if (frag.length < 1) {
      setSuggestions([]);
      return;
    }
    const lower = frag.toLowerCase();
    const matches = names
      .filter((n) => n.toLowerCase().startsWith(lower) && n !== frag)
      .slice(0, 8);
    setSuggestions(matches);
    setSel(0);
  }

  /// Accept `name`, replacing the identifier fragment left of the caret with it.
  function accept(name: string) {
    const input = inputRef.current;
    const caret = input ? input.selectionStart ?? expr.length : expr.length;
    const { start } = fragmentBefore(expr, caret);
    const next = expr.slice(0, start) + name + expr.slice(caret);
    setExpr(next);
    setSuggestions([]);
    // Restore focus + place the caret just after the inserted name.
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (el) {
        el.focus();
        const pos = start + name.length;
        el.setSelectionRange(pos, pos);
      }
    });
  }

  async function submit() {
    const trimmed = expr.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setExpr("");
    setSuggestions([]);
    setRecall(-1);
    const result = await onEval(trimmed);
    setHistory((h) => [...h, { expr: trimmed, result }]);
    setBusy(false);
  }

  function onChange(e: React.ChangeEvent<HTMLInputElement>) {
    const value = e.target.value;
    setExpr(value);
    recomputeSuggestions(value, e.target.selectionStart ?? value.length);
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    const hasSuggestions = suggestions.length > 0;

    // When the completion list is open, it owns Enter/Tab/arrows/Escape.
    if (hasSuggestions) {
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        accept(suggestions[sel]);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSel((s) => (s + 1) % suggestions.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSel((s) => (s - 1 + suggestions.length) % suggestions.length);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setSuggestions([]);
        return;
      }
    }

    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
      return;
    }
    // Up/Down recall through prior expressions when no completion list is open.
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
            below, scalar or compound, in your chosen syntax. Names complete as you type.
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

      <div className="relative mt-2 border-t border-slate-800 pt-2">
        {/* Completion dropdown, anchored above the input. */}
        {suggestions.length > 0 && (
          <ul className="absolute bottom-full left-4 mb-1 max-h-48 w-56 overflow-auto rounded-md border border-slate-700 bg-slate-800 py-1 shadow-lg">
            {suggestions.map((s, i) => (
              <li key={s}>
                <button
                  // `onMouseDown` (not click) so it fires before the input's blur.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    accept(s);
                  }}
                  className={
                    "block w-full px-2 py-0.5 text-left text-[13px] " +
                    (i === sel ? "bg-cadenza-600 text-white" : "text-slate-300 hover:bg-slate-700/60")
                  }
                >
                  {s}
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-1.5">
          <span aria-hidden className="select-none text-cadenza-400">
            ›
          </span>
          <input
            ref={inputRef}
            value={expr}
            onChange={onChange}
            onKeyDown={onKeyDown}
            onFocus={refreshNames}
            onBlur={() => setSuggestions([])}
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
