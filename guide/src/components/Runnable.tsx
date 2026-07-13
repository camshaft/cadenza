/// A runnable, editable Cadenza example — the workhorse of the guide.
///
/// A snippet is authored ONCE in a canonical surface (`source` in `authoredIn`, default s-expr) and
/// displayed in the reader's globally-chosen surface (the shared `useCadenzaEditor` hook keeps it in
/// sync, preserving edits). Run compiles the current text and executes it, showing the value / trap /
/// diagnostics. A snippet meant to fail (to teach a diagnostic) sets `expect="error"`: a declined
/// compile then reads as the intended outcome rather than a problem.
///
/// On first interaction (focus) the editor upgrades to a minimal IDE — inline error squiggles + a
/// lint gutter + type-on-hover — gated so a chapter full of editors doesn't compile-storm on load.
/// "Open in playground" hands the current buffer to the full `/playground` experience.

import { useState } from "react";
import { CodeEditor } from "../editor/CodeEditor.tsx";
import { useCadenzaEditor, wrapModule, type EditorOutcome } from "./useCadenzaEditor.ts";
import { StatusIcon } from "./StatusIcon.tsx";
import { OpenInPlayground } from "./OpenInPlayground.tsx";
import type { Surface } from "../syntax/SyntaxContext.tsx";

/// Wrap the editor text into a compilable program for diagnostics/hover, AND report the UTF-8 byte
/// length of the wrapper prefix so spans map back to the editor text. `wrapModule` trims the snippet,
/// so we locate the trimmed body within the wrapped output for an exact prefix. When the text is
/// already complete (wrap is a no-op) the prefix is 0.
function prepareWrapped(editorText: string, surface: Surface, wrap: boolean) {
  if (!wrap) return { compiled: editorText, wrapPrefixBytes: 0 };
  const compiled = wrapModule(editorText, surface);
  const trimmed = editorText.trim();
  const idx = trimmed ? compiled.indexOf(trimmed) : -1;
  const wrapPrefixBytes = idx < 0 ? 0 : new TextEncoder().encode(compiled.slice(0, idx)).length;
  return { compiled, wrapPrefixBytes };
}

interface Props {
  /** The snippet source, in `authoredIn` surface. Made runnable (export/main supplied) if `wrap`. */
  source: string;
  /** Surface the `source` prop is written in. Default s-expr (the corpus form). */
  authoredIn?: Surface;
  /** Supply the `export`/`main` a bare snippet needs (`(do (def (main) <expr>) (export main))`). Default true. */
  wrap?: boolean;
  /** What this example is meant to do — tunes the status pane. */
  expect?: "value" | "error";
  /** Optional caption shown above the editor. */
  title?: string;
}

type Status = { phase: "idle" } | { phase: "busy" } | { phase: "done"; outcome: EditorOutcome };

export function Runnable({ source, authoredIn = "sexpr", wrap = true, expect = "value", title }: Props) {
  const editor = useCadenzaEditor(source, authoredIn, wrap);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  // The minimal IDE (squiggles + hover) turns on once the reader focuses the editor, so a page full
  // of examples doesn't fire a compile per editor on load.
  const [ideOn, setIdeOn] = useState(false);

  async function doRun() {
    setStatus({ phase: "busy" });
    setStatus({ phase: "done", outcome: await editor.run() });
  }

  function reset() {
    editor.reset();
    setStatus({ phase: "idle" });
  }

  const busy = status.phase === "busy";

  return (
    <div className="my-6 overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/70 shadow-lg">
      <div className="flex items-center justify-between border-b border-slate-700/60 bg-slate-800/50 px-3 py-1.5">
        <span className="text-xs font-medium text-slate-400">{title ?? "Example"}</span>
        <div className="flex items-center gap-2">
          <OpenInPlayground
            getText={() => prepareWrapped(editor.text, editor.surface, wrap).compiled}
            surface={() => editor.surface}
          />
          <button
            onClick={reset}
            className="rounded px-2 py-1 text-xs text-slate-400 transition hover:bg-slate-700/60 hover:text-slate-200"
          >
            Reset
          </button>
          <button
            onClick={doRun}
            disabled={busy}
            className="rounded-md bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition hover:bg-cadenza-500 disabled:opacity-50"
          >
            {busy ? "Running…" : "▶ Run"}
          </button>
        </div>
      </div>

      <div onFocusCapture={() => setIdeOn(true)}>
        <CodeEditor
          value={editor.text}
          onChange={editor.setText}
          ide={
            ideOn
              ? {
                  surface: () => editor.surface,
                  prepare: (t, s) => prepareWrapped(t, s, wrap),
                }
              : undefined
          }
        />
      </div>

      {status.phase !== "idle" && (
        <StatusPane busy={busy} outcome={status.phase === "done" ? status.outcome : null} expect={expect} />
      )}
    </div>
  );
}

function StatusPane({
  busy,
  outcome,
  expect,
}: {
  busy: boolean;
  outcome: EditorOutcome | null;
  expect: "value" | "error";
}) {
  if (busy || !outcome) {
    return (
      <div className="border-t border-slate-700/60 bg-slate-800/40 px-4 py-2.5 font-mono text-[13px] text-slate-400">
        Compiling &amp; running…
      </div>
    );
  }

  let tone = "text-slate-300";
  let bg = "bg-slate-800/40";
  let icon: React.ReactNode = null;
  let body: React.ReactNode = null;

  switch (outcome.kind) {
    case "value":
      tone = "text-emerald-300";
      bg = "bg-emerald-950/30";
      icon = <StatusIcon kind="ok" />;
      body = <code>{outcome.text}</code>;
      break;
    case "trap":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      icon = <StatusIcon kind="trap" />;
      body = <>trap: {outcome.message}</>;
      break;
    case "timeout":
      tone = "text-amber-300";
      bg = "bg-amber-950/30";
      icon = <StatusIcon kind="trap" />;
      body = "timed out — a possible infinite loop was stopped after 5s.";
      break;
    case "declined": {
      const expected = expect === "error";
      tone = expected ? "text-sky-300" : "text-rose-300";
      bg = expected ? "bg-sky-950/30" : "bg-rose-950/30";
      icon = <StatusIcon kind={expected ? "declined" : "error"} />;
      body = (
        <div className="space-y-1">
          {expected && (
            <div className="text-xs opacity-70">The compiler declined this program (as this example intends):</div>
          )}
          {outcome.diags.map((d, i) => (
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
    <div className={`flex items-start gap-2 border-t border-slate-700/60 px-4 py-2.5 font-mono text-[13px] ${bg} ${tone}`}>
      <span className="mt-0.5 shrink-0">{icon}</span>
      <div className="min-w-0">{body}</div>
    </div>
  );
}
