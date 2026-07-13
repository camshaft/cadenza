/// A graded, interactive exercise — the guide's active-learning primitive.
///
/// The reader edits a starter program (usually with a hole to fill), presses Check, and the exercise
/// compiles + runs it and compares the produced value against `expected`. It reports success, or a
/// specific mismatch/decline, and offers progressive help: an optional hint, then a "reveal solution"
/// that fills the editor with a known-good answer (Svelte-tutorial / Rust-Book style — the solution is
/// there when you're stuck, with a nudge not to lean on it).
///
/// Grading is on the RENDERED result string (e.g. "5" or "(: (tuple 1 2) …)"), which is exactly what
/// the runner produces, so an exercise author writes `expected` as the value they expect to see.

import { useState } from "react";
import { CodeEditor } from "../editor/CodeEditor.tsx";
import { useCadenzaEditor } from "./useCadenzaEditor.ts";
import { renderSnippet } from "./useCadenzaEditor.ts";
import { StatusIcon } from "./StatusIcon.tsx";
import { useProgress } from "../progress/ProgressContext.tsx";
import type { Surface } from "../syntax/SyntaxContext.tsx";

interface Props {
  /** Stable id for progress tracking, e.g. "basics:1". Completing it (passing Check) is remembered. */
  id: string;
  /** What to do — the exercise prompt (prose/JSX). */
  prompt: React.ReactNode;
  /** Starter code (with a hole to fill), authored in `authoredIn`. */
  starter: string;
  /** A known-good solution, authored in `authoredIn`. Revealed on demand and used for "show answer". */
  solution: string;
  /** The rendered result string the correct program must produce (e.g. "15"). */
  expected: string;
  /** Surface the starter/solution are written in. Default s-expr. */
  authoredIn?: Surface;
  /** Wrap a bare expression into a module before compiling. Default true. */
  wrap?: boolean;
  /** An optional hint, shown when the reader asks for one. */
  hint?: React.ReactNode;
}

type Check =
  | { phase: "idle" }
  | { phase: "busy" }
  | { phase: "pass"; text: string }
  | { phase: "wrong"; text: string }
  | { phase: "declined"; message: string }
  | { phase: "trap"; message: string };

export function Exercise({
  id,
  prompt,
  starter,
  solution,
  expected,
  authoredIn = "sexpr",
  wrap = true,
  hint,
}: Props) {
  const editor = useCadenzaEditor(starter, authoredIn, wrap);
  const { complete, isComplete } = useProgress();
  const [check, setCheck] = useState<Check>({ phase: "idle" });
  const [showHint, setShowHint] = useState(false);
  // A previously-completed exercise shows its earned checkmark even before the reader re-checks it.
  const alreadyDone = isComplete(id);

  async function doCheck() {
    setCheck({ phase: "busy" });
    const out = await editor.run();
    switch (out.kind) {
      case "value":
        if (out.text.trim() === expected.trim()) {
          complete(id);
          setCheck({ phase: "pass", text: out.text });
        } else {
          setCheck({ phase: "wrong", text: out.text });
        }
        break;
      case "declined":
        setCheck({
          phase: "declined",
          message: out.diags.map((d) => (d.code ? `${d.code} ${d.message}` : d.message)).join("; "),
        });
        break;
      case "trap":
        setCheck({ phase: "trap", message: out.message });
        break;
      case "timeout":
        setCheck({ phase: "trap", message: "timed out (possible infinite loop)" });
        break;
    }
  }

  async function revealSolution() {
    // Render the authored solution into the surface the editor is currently showing.
    const shown = editor.surface;
    const text =
      authoredIn === shown ? solution : await renderSnippet(solution, authoredIn, shown, wrap).catch(() => solution);
    editor.setText(text);
    setCheck({ phase: "idle" });
  }

  const busy = check.phase === "busy";
  const done = check.phase === "pass" || alreadyDone;

  return (
    <div
      className={
        "my-6 overflow-hidden rounded-xl border shadow-lg transition-colors " +
        (done ? "border-emerald-600/60 bg-emerald-950/10" : "border-cadenza-700/50 bg-slate-900/70")
      }
    >
      <div className="flex items-center gap-2 border-b border-slate-700/60 bg-slate-800/50 px-3 py-2">
        <span className="rounded bg-cadenza-600/20 px-2 py-0.5 text-xs font-semibold uppercase tracking-wide text-cadenza-300">
          Exercise
        </span>
        <span className="flex-1 text-sm text-slate-300">{prompt}</span>
        {done && (
          <span className="shrink-0 text-emerald-400" title="Completed" aria-label="Completed">
            ✓
          </span>
        )}
      </div>

      <CodeEditor value={editor.text} onChange={editor.setText} />

      <div className="flex flex-wrap items-center gap-2 border-t border-slate-700/60 bg-slate-800/40 px-3 py-2">
        <button
          onClick={doCheck}
          disabled={busy}
          className="rounded-md bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition hover:bg-cadenza-500 disabled:opacity-50"
        >
          {busy ? "Checking…" : "Check"}
        </button>
        <button
          onClick={() => {
            editor.reset();
            setCheck({ phase: "idle" });
          }}
          className="rounded px-2 py-1 text-xs text-slate-400 transition hover:bg-slate-700/60 hover:text-slate-200"
        >
          Reset
        </button>
        {hint && (
          <button
            onClick={() => setShowHint((s) => !s)}
            className="rounded px-2 py-1 text-xs text-slate-400 transition hover:bg-slate-700/60 hover:text-slate-200"
          >
            {showHint ? "Hide hint" : "Hint"}
          </button>
        )}
        <button
          onClick={revealSolution}
          className="ml-auto rounded px-2 py-1 text-xs text-slate-500 transition hover:bg-slate-700/60 hover:text-slate-300"
          title="Try not to rely on this too much!"
        >
          Show solution
        </button>
      </div>

      {showHint && hint && (
        <div className="border-t border-slate-700/60 bg-slate-800/30 px-4 py-2.5 text-sm text-slate-400">
          💡 {hint}
        </div>
      )}

      <CheckPane check={check} expected={expected} />
    </div>
  );
}

function CheckPane({ check, expected }: { check: Check; expected: string }) {
  if (check.phase === "idle") return null;

  let tone = "text-slate-300";
  let bg = "bg-slate-800/40";
  let icon: React.ReactNode = null;
  let body: React.ReactNode = null;

  switch (check.phase) {
    case "busy":
      body = "Checking…";
      break;
    case "pass":
      tone = "text-emerald-300";
      bg = "bg-emerald-950/30";
      icon = <StatusIcon kind="ok" />;
      body = (
        <>
          Correct — it produced <code>{check.text}</code>. Nicely done!
        </>
      );
      break;
    case "wrong":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      icon = <StatusIcon kind="trap" />;
      body = (
        <>
          Not quite: it produced <code>{check.text}</code>, but the goal is <code>{expected}</code>. Keep going!
        </>
      );
      break;
    case "declined":
      tone = "text-rose-300";
      bg = "bg-rose-950/30";
      icon = <StatusIcon kind="error" />;
      body = <>The compiler declined it: {check.message}</>;
      break;
    case "trap":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      icon = <StatusIcon kind="trap" />;
      body = <>{check.message}</>;
      break;
  }

  return (
    <div className={`flex items-start gap-2 border-t border-slate-700/60 px-4 py-2.5 font-mono text-[13px] ${bg} ${tone}`}>
      {icon && <span className="mt-0.5 shrink-0">{icon}</span>}
      <div className="min-w-0">{body}</div>
    </div>
  );
}
