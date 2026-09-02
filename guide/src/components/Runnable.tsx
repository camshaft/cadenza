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

import { useEffect, useState } from "react";
import { LazyCodeEditor as CodeEditor } from "../editor/LazyCodeEditor.tsx";
import { useCadenzaEditor, wrapModule, type EditorOutcome } from "./useCadenzaEditor.ts";
import { useRunnableRegistry } from "./RunnableRegistry.tsx";
import { StatusIcon } from "./StatusIcon.tsx";
import { OpenInPlayground } from "./OpenInPlayground.tsx";
import type { Surface } from "../syntax/SyntaxContext.tsx";
import type { Diag } from "../compiler/client.ts";
import { fixConfidence, fixIsApplicable } from "../playground/applyFix.ts";
import { runTests, type TestRunOutcome } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { assertPreludeFor } from "./assertPrelude.ts";
import { MultiFileRunnable, type RunnableFile } from "./MultiFileRunnable.tsx";

/// Shared example-control button classes. Both carry a 44px mobile touch target (`min-h-11` below `sm`,
/// the touch guideline) that collapses to the compact desktop density at `sm+`. `BTN_PRIMARY` is the
/// accent action (Run / Check); `BTN_SECONDARY` is a muted action (Reset / Hint / Show solution). Kept as
/// constants so every example control stays in lockstep (a per-button edit would drift the touch sizing).
export const BTN_PRIMARY =
  "flex min-h-11 items-center justify-center rounded-md bg-cadenza-600 px-3 text-xs font-semibold text-white transition hover:bg-cadenza-500 disabled:opacity-50 sm:min-h-0 sm:py-1";
export const BTN_SECONDARY =
  "flex min-h-11 items-center justify-center rounded px-2 text-xs text-slate-400 transition hover:bg-slate-700/60 hover:text-slate-200 sm:min-h-0 sm:py-1";

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
  /** Stable id for clickable "change X to Y" prose to target. A `<TryChange example="<id>">` elsewhere
   *  on the page resolves to THIS Runnable (via the RunnableRegistry) and drives its buffer + run. Only
   *  needed on a Runnable that clickable prose refers to; the id must be unique on the page (gated). */
  id?: string;
  /** MULTI-FILE mode (operator-mandated): a set of files — one `entry`, the rest link-merged modules it can
   *  `import` — compiled together via compileWithPreloaded (the platform-explorer seam). When present, `source`
   *  is ignored and the example renders as a multi-file editor (tabs). Absent → the single-`source` path below,
   *  unchanged. Use this to show a decoupled boundary, e.g. an events file + a reducer file. */
  files?: readonly RunnableFile[];
  /** The snippet source, in `authoredIn` surface. Made runnable (export/main supplied) if `wrap`. Optional
   *  only because a multi-file (`files`) Runnable carries its sources in the file set instead. */
  source?: string;
  /** Surface the `source` prop is written in. Default s-expr (the corpus form). */
  authoredIn?: Surface;
  /** Supply the `export`/`main` a bare snippet needs (`(do (def (main) <expr>) (export main))`). Default true. */
  wrap?: boolean;
  /** What this example is meant to do — tunes the status pane. */
  expect?: "value" | "error";
  /** (MULTI-FILE only) The exact rendered value the file set must run to — when set, the check-examples gate
   *  asserts the run result equals it (not just that it runs), and the result pane shows it as the expected
   *  value. A single-file Runnable pins values via `<Exercise>` instead, so this is specifically for a
   *  `files=` runnable that wants to pin a trace (e.g. the agent-loop's terminal fold value). */
  expected?: string;
  /** Optional caption shown above the editor. */
  title?: string;
  /** "run" (default) = compile + run the entry, showing its value. "test" = run the snippet's `@test`
   *  defs as tests (like `cdz test`), showing inline pass/fail per test. In test mode the source is a
   *  program with `@test`-annotated defs (no hardcoded main), and `wrap` is ignored (a test build lays
   *  its boundary out from the @test defs, not an export). */
  mode?: "run" | "test";
  /** (test mode) The assert prelude prepended before the `@test` defs so examples can call
   *  `assert`/`assert-eq`/`assert-ne` without redefining them. `true` (default) = the shared
   *  surface-appropriate prelude; `false` = none (the example defines its own asserts, e.g. one teaching
   *  example that shows them). */
  prelude?: boolean;
}

type Status = { phase: "idle" } | { phase: "busy" } | { phase: "done"; outcome: EditorOutcome };
type TestStatus =
  | { phase: "idle" }
  | { phase: "busy" }
  | { phase: "done"; outcome: TestRunOutcome };

export function Runnable({ id, files, source, authoredIn = "sexpr", wrap = true, expect = "value", expected, title, mode = "run", prelude = true }: Props) {
  // MULTI-FILE mode (operator-mandated): a set of files compiled together via compileWithPreloaded. Takes
  // precedence over the single-source paths; the explorer seam owns the file-set invariants + run wiring.
  if (files) return <MultiFileRunnable files={files} expect={expect} expected={expected} title={title} />;
  // Below here `source` is required — a non-multi-file Runnable must carry one. A missing source is an
  // authoring bug; render it loudly rather than compiling an empty program.
  if (source == null) {
    return (
      <div className="my-6 rounded-xl border border-rose-700/60 bg-rose-950/30 px-4 py-3 font-mono text-[13px] text-rose-300">
        &lt;Runnable&gt; needs either a `source` or a `files` prop.
      </div>
    );
  }
  if (mode === "test") return <TestRunnable source={source} authoredIn={authoredIn} title={title} prelude={prelude} />;
  return <RunRunnable id={id} source={source} authoredIn={authoredIn} wrap={wrap} expect={expect} title={title} />;
}

function RunRunnable({ id, source, authoredIn = "sexpr", wrap = true, expect = "value", title }: Props & { source: string }) {
  // `expect="error"` ⇒ the example is SUPPOSED to trap; tell the runner so a stale-runtime mismatch shows
  // the REAL trap, not the misleading hard-reload advice (an intentional trap and a corruption trap both
  // surface as `unreachable`, so the expectation is the only reliable discriminator).
  const editor = useCadenzaEditor(source, authoredIn, wrap, expect === "error");
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  // The minimal IDE (squiggles + hover) turns on once the reader focuses the editor, so a page full
  // of examples doesn't fire a compile per editor on load.
  const [ideOn, setIdeOn] = useState(false);

  // Register this Runnable under its `id` so clickable "change X to Y" prose (`<TryChange>`) can drive
  // it: apply a variant/patch to the buffer + run, showing the result inline here. Only when an `id` is
  // set (the common Runnable has none). The handle closes over `editor`, so re-register when it changes;
  // the phase-setting wrappers below route the run outcome into THIS pane's status just like doRun.
  const registry = useRunnableRegistry();
  useEffect(() => {
    if (!id || !registry) return;
    // Drive the buffer via an apply-and-run primitive, routing its outcome into THIS pane's status (busy
    // → done) exactly like doRun. A patch that failed the exactly-once rule resolves to null (no run) —
    // leave the pane untouched and propagate the null so the caller (<TryChange>) can react.
    const runInto = async (apply: Promise<EditorOutcome | null>): Promise<EditorOutcome | null> => {
      setStatus({ phase: "busy" });
      const outcome = await apply;
      setStatus(outcome === null ? { phase: "idle" } : { phase: "done", outcome });
      return outcome;
    };
    registry.register(id, {
      applyVariant: (src, s) => runInto(editor.applyAuthored(src, s)) as Promise<EditorOutcome>,
      applyPatch: (find, replace) => runInto(editor.applyPatch(find, replace)),
      reset: () => { editor.reset(); setStatus({ phase: "idle" }); },
    });
    return () => registry.unregister(id);
  }, [id, registry, editor]);

  async function doRun() {
    setStatus({ phase: "busy" });
    try {
      setStatus({ phase: "done", outcome: await editor.run() });
    } catch (e) {
      // A run should never reject (the worker turns even a parse error into a decline), but if one
      // ever does, land on a shown error rather than leaving the pane stuck on "Compiling & running…".
      const message = e instanceof Error ? e.message : String(e);
      setStatus({
        phase: "done",
        outcome: { kind: "declined", diags: [{ error: true, code: "", message, node: -1, from: 0, to: 0, fix: null }], wrapPrefixBytes: 0 },
      });
    }
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
            className={BTN_SECONDARY}
          >
            Reset
          </button>
          <button
            onClick={doRun}
            disabled={busy}
            className={BTN_PRIMARY}
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
        <StatusPane
          busy={busy}
          outcome={status.phase === "done" ? status.outcome : null}
          expect={expect}
          surface={editor.surface}
          onApplyFix={editor.applyFix}
        />
      )}
    </div>
  );
}

function StatusPane({
  busy,
  outcome,
  expect,
  surface,
  onApplyFix,
}: {
  busy: boolean;
  outcome: EditorOutcome | null;
  expect: "value" | "error";
  surface: Surface;
  onApplyFix: (d: Diag, wrapPrefixBytes: number) => void;
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
              <div>
                {d.code && <span className="font-semibold">{d.code} </span>}
                {d.message}
              </div>
              {d.fix && fixIsApplicable(d.fix, surface) && (
                <button
                  onClick={() => onApplyFix(d, outcome.wrapPrefixBytes)}
                  title={d.fix.verified ? "Compiler-proven — safe to apply" : "A suggestion — confirm it matches your intent"}
                  className="mt-1 inline-flex items-center gap-1 rounded border border-cadenza-600/40 bg-cadenza-600/10 px-2 py-0.5 text-[11px] text-cadenza-200 transition hover:bg-cadenza-600/20"
                >
                  💡 {fixActionLabel(d)}
                  <span className="text-cadenza-400/70">· {fixConfidence(d.fix)}</span>
                </button>
              )}
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

/// The concrete edit a fix performs, for the Apply button's label — derived from kind + payload since
/// the compiler's prose label isn't carried over the wasm ABI.
function fixActionLabel(d: Diag): string {
  const fix = d.fix!;
  switch (fix.kind) {
    case "wrap":
      return `Wrap in \`${fix.replacement}\``;
    case "insert":
      return "Add the missing arms";
    default:
      return `Replace with \`${fix.replacement}\``;
  }
}

/// A Runnable in TEST mode: the snippet is a program with `@test`-annotated defs, and Run executes them
/// as tests (like `cdz test`), showing inline ✓/✗ per test. `wrap` is off — a test build lays its
/// boundary out from the @test defs, not an export/main. Uses the shared editor (same Cadenza IDE
/// highlighting) so the reader sees + edits real test code.
function TestRunnable({ source, authoredIn = "sexpr", title, prelude = true }: Pick<Props, "authoredIn" | "title" | "prelude"> & { source: string }) {
  const editor = useCadenzaEditor(source, authoredIn, false);
  const { surface } = useSyntax();
  const [status, setStatus] = useState<TestStatus>({ phase: "idle" });
  const [ideOn, setIdeOn] = useState(false);

  async function doRun() {
    setStatus({ phase: "busy" });
    try {
      // Prepend the shared assert prelude (unless prelude={false}), so the example's @test defs can call
      // assert/assert-eq/assert-ne without redefining them. The editor shows just the author's @test defs.
      const program = prelude ? `${assertPreludeFor(surface)}\n${editor.text}` : editor.text;
      setStatus({ phase: "done", outcome: await runTests(program, surface) });
    } catch (e) {
      setStatus({ phase: "done", outcome: { kind: "error", message: e instanceof Error ? e.message : String(e) } });
    }
  }

  // The IDE lint/hover path must see the SAME text `doRun` compiles — i.e. with the assert prelude
  // prepended (unless prelude={false}) — or `assert`/`assert-eq`/`assert-ne` read as unbound (a false
  // CDZ0101). `wrapPrefixBytes` is the prelude's UTF-8 byte length (prelude + the `\n`) so diagnostic
  // spans map back onto the editor text. Mirrors the Run path's prepend at `doRun`.
  function prepareTest(t: string): { compiled: string; wrapPrefixBytes: number } {
    if (!prelude) return { compiled: t, wrapPrefixBytes: 0 };
    const pre = `${assertPreludeFor(surface)}\n`;
    return { compiled: `${pre}${t}`, wrapPrefixBytes: new TextEncoder().encode(pre).length };
  }

  const busy = status.phase === "busy";
  return (
    <div className="my-6 overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/70 shadow-lg">
      <div className="flex items-center justify-between border-b border-slate-700/60 bg-slate-800/50 px-3 py-1.5">
        <span className="text-xs font-medium text-slate-400">{title ?? "Tests"}</span>
        <div className="flex items-center gap-2">
          <button
            onClick={() => { editor.reset(); setStatus({ phase: "idle" }); }}
            className={BTN_SECONDARY}
          >
            Reset
          </button>
          <button
            onClick={doRun}
            disabled={busy}
            className={BTN_PRIMARY}
          >
            {busy ? "Running…" : "▶ Run tests"}
          </button>
        </div>
      </div>

      <div onFocusCapture={() => setIdeOn(true)}>
        <CodeEditor
          value={editor.text}
          onChange={editor.setText}
          ide={ideOn ? { surface: () => surface, prepare: prepareTest } : undefined}
        />
      </div>

      {status.phase !== "idle" && <TestResultsPane busy={busy} outcome={status.phase === "done" ? status.outcome : null} />}
    </div>
  );
}

/// Renders a test run's outcome: inline ✓/✗ per `@test` (failing tests show the trap message), deferred
/// property tests as a muted note, or a compile error (e.g. "no `@test` definition"). Mirrors `cdz test`'s
/// per-test lines + a passed/failed footer.
function TestResultsPane({ busy, outcome }: { busy: boolean; outcome: TestRunOutcome | null }) {
  if (busy || !outcome) {
    return (
      <div className="border-t border-slate-700/60 bg-slate-800/40 px-4 py-2.5 font-mono text-[13px] text-slate-400">
        Compiling &amp; running tests…
      </div>
    );
  }
  if (outcome.kind === "error") {
    return (
      <div className="border-t border-slate-700/60 bg-slate-800/40 px-4 py-2.5 font-mono text-[13px] text-rose-300">
        {outcome.message}
      </div>
    );
  }
  const ran = outcome.results.filter((r) => !r.deferred);
  const deferred = outcome.results.filter((r) => r.deferred);
  const passed = ran.filter((r) => r.pass).length;
  const failed = ran.length - passed;
  return (
    <div className="border-t border-slate-700/60 bg-slate-800/40 px-4 py-2.5 font-mono text-[13px]">
      <ul className="space-y-1">
        {ran.map((r) => (
          <li key={r.name} className={r.pass ? "text-emerald-300" : "text-rose-300"}>
            {r.pass ? "✓" : "✗"} {r.name}
            {r.pass && r.trials ? <span className="text-emerald-400/70"> ({r.trials} trials)</span> : null}
            {!r.pass && r.error ? <span className="text-rose-400/80"> — {r.error}</span> : null}
            {/* A FAILED property test carries the shrunk COUNTEREXAMPLE (the concrete minimal failing input +
                the replay seed) — the whole point of shrinking, and the operator's explicitly-requested
                feature. The browser runner computes it (runWorker.ts) and it flows here intact; surface it,
                or the guide shows only "property failed" with no value. */}
            {!r.pass && r.counterexample ? (
              <div className="pl-4 text-rose-400/80">
                counterexample: {r.counterexample.args}{" "}
                <span className="text-slate-500">(seed {r.counterexample.seed})</span>
              </div>
            ) : null}
          </li>
        ))}
        {deferred.map((r) => (
          <li key={r.name} className="text-slate-500">
            • {r.name} <span className="text-slate-600">— property test (deferred)</span>
          </li>
        ))}
      </ul>
      <div className={`mt-2 text-xs ${failed > 0 ? "text-rose-400" : "text-emerald-400"}`}>
        {passed} passed{failed > 0 ? `, ${failed} failed` : ""}{deferred.length > 0 ? `, ${deferred.length} deferred` : ""}
      </div>
    </div>
  );
}
