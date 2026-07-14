/// The `/calculator` page — a focused calculator over the real language: exact rationals, dimensioned
/// quantities, big integers, and variables you assign and recall. It is NOT the playground (no code
/// editor, no compiled views); it is an input line + a running tape + a variables panel, over the shared
/// `Calculator` engine (which reuses `replEval` + the run worker — the same pipeline the playground and
/// the native `cdz-calc` use, so a result here is identical to a real run).
///
/// The global surface toggle (ML ↔ s-expr) applies: switching re-creates the engine in the new surface
/// and clears the session, because a stored binding's SOURCE was typed in the old surface (it can't be
/// blindly reinterpreted). A small notice explains that on toggle.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { Calculator, type Eval } from "./engine.ts";

/// Track the VISUAL viewport height — the area actually visible, EXCLUDING the on-screen keyboard. On
/// mobile the software keyboard shrinks `window.visualViewport` but NOT `100dvh` (which stays the full
/// screen), so a `100dvh` container is pushed partly behind the keyboard and the user has to scroll up
/// and down to reach the input. Sizing the calculator to `visualViewport.height` instead makes it scale
/// down when the keyboard opens (and back up when it closes) — no awkward scrolling. Returns `null` until
/// measured / when the API is absent (older browsers, SSR), so the caller falls back to the CSS `100dvh`.
function useVisualViewportHeight(): number | null {
  const [h, setH] = useState<number | null>(null);
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const update = () => setH(vv.height);
    update();
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    return () => {
      vv.removeEventListener("resize", update);
      vv.removeEventListener("scroll", update);
    };
  }, []);
  return h;
}

/// One line of the tape: what the user entered and what it produced.
interface TapeEntry {
  input: string;
  result: Eval;
}

/// A per-surface example expression, so the empty calculator suggests something to try. In EXACT MODE
/// (on by default) a bare `1 / 3` is the exact fraction 1/3 — no `R` suffix needed.
const PLACEHOLDER: Record<string, string> = {
  ml: "1 / 3 + 1 / 3 + 1 / 3",
  sexpr: "(+ (+ (/ 1 3) (/ 1 3)) (/ 1 3))",
};

/// A few starter expressions per surface — click to fill the input. They showcase the calculator's
/// reason for existing: exact fractions (bare `1 / 3` = 1/3, exact by default), dimensioned quantities,
/// and variables.
const STARTERS: Record<string, { label: string; expr: string }[]> = {
  ml: [
    { label: "exact thirds", expr: "1 / 3 + 1 / 3 + 1 / 3" },
    { label: "km + m", expr: "Qty.of(1, Unit.of(#\"kilometer\")) + Qty.of(500, Unit.of(#\"meter\"))" },
    { label: "assign a variable", expr: "x = 6 * 7" },
  ],
  sexpr: [
    { label: "exact thirds", expr: "(+ (+ (/ 1 3) (/ 1 3)) (/ 1 3))" },
    { label: "km + m", expr: "(+ (Qty.of 1 (Unit.of #\"kilometer\")) (Qty.of 500 (Unit.of #\"meter\")))" },
    { label: "assign a variable", expr: "x = (* 6 7)" },
  ],
};

export default function CalculatorPage() {
  const { surface } = useSyntax();
  // The engine is re-created whenever the surface changes (bindings can't cross surfaces — see header).
  const engineRef = useRef<Calculator>(new Calculator(surface));
  const [tape, setTape] = useState<TapeEntry[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  // The in-scope variables (name → current value), refreshed after each successful line.
  const [vars, setVars] = useState<{ name: string; text: string }[]>([]);
  // Up-arrow recall through prior inputs; -1 = editing a fresh line.
  const [recall, setRecall] = useState(-1);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Size the page to the VISIBLE viewport so the mobile keyboard shrinks it (no scroll-to-reach-input).
  const vvHeight = useVisualViewportHeight();

  // On a surface change, start a fresh session in the new surface (a stored source was typed in the old
  // one, so it can't be reinterpreted). The tape is kept but marked stale by a one-time notice.
  const surfaceRef = useRef(surface);
  useEffect(() => {
    if (surfaceRef.current !== surface) {
      surfaceRef.current = surface;
      engineRef.current = new Calculator(surface);
      setVars([]);
      setTape((t) =>
        t.length > 0
          ? [...t, { input: "", result: { kind: "error", message: `— switched to ${surface} surface; variables cleared —` } }]
          : t,
      );
    }
  }, [surface]);

  // Keep the newest tape entry in view.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [tape, busy]);

  const submit = useCallback(
    async (raw: string) => {
      const line = raw.trim();
      if (!line || busy) return;
      setBusy(true);
      setInput("");
      setRecall(-1);
      const result = await engineRef.current.eval(line);
      setTape((t) => [...t, { input: line, result }]);
      // Refresh the variables panel from the engine's STORED values (synchronous — no re-run, so it
      // never contends with the run worker or races the next input).
      setVars(engineRef.current.values());
      setBusy(false);
      inputRef.current?.focus();
    },
    [busy],
  );

  const priorInputs = useMemo(() => tape.filter((e) => e.input).map((e) => e.input), [tape]);

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      void submit(input);
      return;
    }
    // Up/Down recall through prior inputs.
    if (e.key === "ArrowUp" && priorInputs.length > 0) {
      e.preventDefault();
      const next = recall < 0 ? priorInputs.length - 1 : Math.max(0, recall - 1);
      setRecall(next);
      setInput(priorInputs[next]);
    } else if (e.key === "ArrowDown" && recall >= 0) {
      e.preventDefault();
      const next = recall + 1;
      if (next >= priorInputs.length) {
        setRecall(-1);
        setInput("");
      } else {
        setRecall(next);
        setInput(priorInputs[next]);
      }
    }
  }

  function reset() {
    engineRef.current.clear();
    setTape([]);
    setVars([]);
    setRecall(-1);
    inputRef.current?.focus();
  }

  return (
    <div
      className="mx-auto flex max-w-3xl flex-col px-4 py-4"
      // Bind to the visual viewport (shrinks with the mobile keyboard); fall back to 100dvh when the
      // VisualViewport API is unavailable so desktop / older browsers still fill the screen.
      style={{ height: vvHeight != null ? `${vvHeight}px` : "100dvh" }}
    >
      {/* Header */}
      <div className="mb-3 flex items-baseline justify-between">
        <div>
          <h1 className="text-xl font-bold text-slate-100">Cadenza calculator</h1>
          <p className="text-sm text-slate-500">
            Exact by default — <code className="text-slate-400">1 / 3</code> is{" "}
            <code className="text-slate-400">1/3</code>, not 0. Fractions, units, and big integers in the
            real language. Assign variables with <code className="text-slate-400">name = expr</code>;
            recall the last result with <code className="text-slate-400">ans</code>.
          </p>
        </div>
        <div className="flex items-center gap-3 text-xs">
          <button onClick={reset} className="text-slate-400 hover:text-slate-200">
            Clear
          </button>
          <Link to="/playground" className="text-cadenza-400 hover:text-cadenza-300">
            Playground →
          </Link>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 gap-4">
        {/* The tape + input */}
        <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 bg-slate-900/40">
          <div ref={scrollRef} className="min-h-0 flex-1 space-y-2 overflow-auto p-3 font-mono text-sm">
            {tape.length === 0 && (
              <div className="space-y-2 text-slate-500">
                <p>Type an expression and press Enter. Try:</p>
                <ul className="space-y-1">
                  {(STARTERS[surface] ?? STARTERS.ml).map((s) => (
                    <li key={s.label}>
                      <button
                        onClick={() => {
                          setInput(s.expr);
                          inputRef.current?.focus();
                        }}
                        className="text-left text-cadenza-400 hover:text-cadenza-300"
                        title={s.expr}
                      >
                        {s.label}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {tape.map((entry, i) => (
              <TapeLine key={i} entry={entry} />
            ))}
            {busy && <p className="text-slate-500">evaluating…</p>}
          </div>
          <div className="flex items-center gap-2 border-t border-slate-800 px-3 py-2">
            <span aria-hidden className="select-none text-cadenza-400">
              ›
            </span>
            <input
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder={PLACEHOLDER[surface] ?? PLACEHOLDER.ml}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              autoFocus
              className="min-w-0 flex-1 bg-transparent font-mono text-sm text-slate-100 placeholder:text-slate-600 focus:outline-none"
            />
            <button
              onClick={() => void submit(input)}
              disabled={busy || input.trim() === ""}
              className="rounded bg-cadenza-600 px-2.5 py-1 text-xs font-semibold text-white transition enabled:hover:bg-cadenza-500 disabled:opacity-40"
            >
              =
            </button>
          </div>
        </div>

        {/* Variables panel */}
        <div className="hidden w-48 shrink-0 flex-col rounded-lg border border-slate-800 bg-slate-900/40 p-3 sm:flex">
          <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Variables</h2>
          {vars.length === 0 ? (
            <p className="text-xs text-slate-600">none yet — assign with name = expr</p>
          ) : (
            <ul className="space-y-1 font-mono text-xs">
              {vars.map((v) => (
                <li key={v.name} className="truncate" title={`${v.name} = ${v.text}`}>
                  <span className="text-cadenza-300">{v.name}</span>{" "}
                  <span className="text-slate-500">=</span>{" "}
                  <span className="text-slate-300">{v.text}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

/// One tape line: the echoed input above, its result below (coloured by kind).
function TapeLine({ entry }: { entry: TapeEntry }) {
  const { input, result } = entry;
  return (
    <div className="space-y-0.5">
      {input && (
        <div className="flex items-baseline gap-1.5">
          <span aria-hidden className="select-none text-slate-600">
            ›
          </span>
          <code className="text-slate-300">{input}</code>
        </div>
      )}
      <ResultLine result={result} />
    </div>
  );
}

function ResultLine({ result }: { result: Eval }) {
  switch (result.kind) {
    case "value":
      return (
        <div className="pl-4 text-emerald-300">
          = <code>{result.text}</code>
        </div>
      );
    case "bound":
      return (
        <div className="pl-4 text-sky-300">
          <code>{result.name}</code> = <code>{result.text}</code>
        </div>
      );
    case "trap":
      return <div className="pl-4 text-amber-300">trap: {result.message}</div>;
    case "timeout":
      return <div className="pl-4 text-amber-300">timed out — a possible infinite loop was stopped.</div>;
    case "error":
      return <div className="pl-4 text-rose-300">{result.message}</div>;
  }
}
