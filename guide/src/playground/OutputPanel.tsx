/// The playground's tabbed output pane: Result (the run outcome), Diagnostics (every fault, click to
/// jump), and AST (the raw tree the compiler sees — a nearly-free differentiator).

import { useState } from "react";
import type { Diag, Surface } from "../compiler/client.ts";
import { StatusIcon } from "../components/StatusIcon.tsx";
import { ReplPanel, type ReplEntry } from "./ReplPanel.tsx";
import { fixConfidence, fixIsApplicable } from "./applyFix.ts";

export type RunView =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

type Tab = "result" | "repl" | "diagnostics" | "ast" | "compiled";

/// What the program compiled to, for the "Compiled" tab. `wat`/`rustSync`/`rustAsync` are filled
/// lazily (null until computed) so a run doesn't pay for views the reader hasn't opened.
export interface CompiledInfo {
  bytes: number;
  /** true if the emitted component imports the value-heap runtime (a compound result). */
  importsRuntime: boolean;
  wat: string | null;
  rustSync: string | null;
  rustAsync: string | null;
  /** The lowered-optimized Cadenza source (`--target cadenza`), sexpr — the cadenza-backend's output view. */
  cadenza: string | null;
}

/// Which sub-view of the Compiled tab is shown; the page fills the corresponding field on demand.
export type CompiledView = "summary" | "wat" | "rust" | "rustAsync" | "cadenza";

interface Props {
  run: RunView;
  diagnostics: Diag[];
  ast: string;
  compiled: CompiledInfo | null;
  /** The surface the REPL input is written in (mirrors the editor). */
  surface: Surface;
  /** Evaluate a REPL expression against the current buffer (see `ReplPanel`). */
  onReplEval: (expr: string) => Promise<ReplEntry["result"]>;
  /** The names the REPL can complete (the buffer's definitions); fetched on demand. */
  onReplNames: () => Promise<string[]>;
  /** Jump the editor to a diagnostic's source range. */
  onJumpTo: (from: number, to: number) => void;
  /** Apply a diagnostic's structural fix to the buffer. */
  onApplyFix: (d: Diag) => void;
  /** Ask the page to compute a Compiled sub-view (WAT / Rust / Rust-async) on demand. */
  onNeedCompiledView: (view: CompiledView) => void;
}

export function OutputPanel({ run, diagnostics, ast, compiled, surface, onReplEval, onReplNames, onJumpTo, onApplyFix, onNeedCompiledView }: Props) {
  const [tab, setTab] = useState<Tab>("result");
  const errorCount = diagnostics.filter((d) => d.error).length;

  return (
    <div className="flex h-full flex-col bg-slate-900/40">
      <div className="flex items-center gap-1 border-b border-slate-700/60 bg-slate-800/50 px-2 py-1">
        <TabButton active={tab === "result"} onClick={() => setTab("result")}>
          Result
        </TabButton>
        <TabButton active={tab === "repl"} onClick={() => setTab("repl")}>
          REPL
        </TabButton>
        <TabButton active={tab === "diagnostics"} onClick={() => setTab("diagnostics")}>
          Diagnostics
          {diagnostics.length > 0 && (
            <span
              className={
                "ml-1.5 rounded-full px-1.5 text-[10px] " +
                (errorCount > 0 ? "bg-rose-500/20 text-rose-300" : "bg-amber-500/20 text-amber-300")
              }
            >
              {diagnostics.length}
            </span>
          )}
        </TabButton>
        <TabButton active={tab === "ast"} onClick={() => setTab("ast")}>
          AST
        </TabButton>
        <TabButton active={tab === "compiled"} onClick={() => setTab("compiled")}>
          Compiled
        </TabButton>
      </div>

      {/* The REPL owns its own scroll + a pinned input row, so it gets the bare flex box (no
          `overflow-auto`); the static views share the scrolling, padded wrapper. */}
      {tab === "repl" ? (
        <div className="min-h-0 flex-1 p-3 font-mono text-[13px]">
          <ReplPanel surface={surface} onEval={onReplEval} onNames={onReplNames} />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto p-3 font-mono text-[13px]">
          {tab === "result" && <ResultBody run={run} />}
          {tab === "diagnostics" && <DiagnosticsBody diagnostics={diagnostics} surface={surface} onJumpTo={onJumpTo} onApplyFix={onApplyFix} />}
          {tab === "ast" && (
            <pre className="whitespace-pre-wrap text-slate-400">{ast || "— run or edit to see the tree —"}</pre>
          )}
          {tab === "compiled" && <CompiledBody compiled={compiled} onNeed={onNeedCompiledView} />}
        </div>
      )}
    </div>
  );
}

function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      // Mobile touch target: 44px-tall tab below `sm` (the touch guideline), compact at sm+.
      className={
        "flex min-h-11 items-center rounded px-2.5 text-xs font-medium transition sm:min-h-0 sm:py-1 " +
        (active ? "bg-slate-700/60 text-slate-100" : "text-slate-400 hover:text-slate-200")
      }
    >
      {children}
    </button>
  );
}

function ResultBody({ run }: { run: RunView }) {
  switch (run.kind) {
    case "idle":
      return <span className="text-slate-500">Press Run (or ⌘/Ctrl-Enter) to compile and execute.</span>;
    case "busy":
      return <span className="text-slate-400">Compiling &amp; running…</span>;
    case "value":
      return (
        <span className="text-emerald-300">
          <StatusIcon kind="ok" /> <code>{run.text}</code>
        </span>
      );
    case "trap":
      return (
        <span className="text-amber-300">
          <StatusIcon kind="trap" /> trap: {run.message}
        </span>
      );
    case "timeout":
      return (
        <span className="text-amber-300">
          <StatusIcon kind="trap" /> timed out — a possible infinite loop was stopped.
        </span>
      );
    case "error":
      return (
        <span className="text-rose-300">
          <StatusIcon kind="error" /> {run.message}
        </span>
      );
  }
}

function CompiledBody({
  compiled,
  onNeed,
}: {
  compiled: CompiledInfo | null;
  onNeed: (view: CompiledView) => void;
}) {
  const [view, setView] = useState<CompiledView>("summary");
  if (!compiled) {
    return <span className="text-slate-500">Run a well-formed program to see what it compiles to.</span>;
  }
  // Ask the page to fill a view the first time it's opened (lazy).
  function open(v: CompiledView) {
    setView(v);
    if (v === "wat" && compiled!.wat == null) onNeed("wat");
    if (v === "rust" && compiled!.rustSync == null) onNeed("rust");
    if (v === "rustAsync" && compiled!.rustAsync == null) onNeed("rustAsync");
    if (v === "cadenza" && compiled!.cadenza == null) onNeed("cadenza");
  }
  const pending = (s: string | null) => (s == null ? "// computing…" : s);
  return (
    <div className="flex h-full flex-col">
      <div className="mb-2 flex gap-1 text-[11px]">
        <SubTab active={view === "summary"} onClick={() => open("summary")}>Summary</SubTab>
        <SubTab active={view === "cadenza"} onClick={() => open("cadenza")}>Cadenza</SubTab>
        <SubTab active={view === "wat"} onClick={() => open("wat")}>WAT</SubTab>
        <SubTab active={view === "rust"} onClick={() => open("rust")}>Rust</SubTab>
        <SubTab active={view === "rustAsync"} onClick={() => open("rustAsync")}>Rust (async)</SubTab>
      </div>
      {view === "summary" && (
        <div className="space-y-2 text-slate-300">
          <div>
            <span className="text-slate-500">Component size:</span> {compiled.bytes.toLocaleString()} bytes
          </div>
          <div>
            <span className="text-slate-500">Value-heap runtime:</span>{" "}
            {compiled.importsRuntime ? (
              <span className="text-cadenza-300">imported (a compound result crosses the boundary)</span>
            ) : (
              <span className="text-emerald-300">not needed (self-contained scalar/unit result)</span>
            )}
          </div>
          <p className="pt-1 text-xs text-slate-500">
            Cadenza is target-neutral above its backend seam: the same program lowers to a WebAssembly
            component (see WAT), or to Rust source — synchronous, or gas-metered async.
          </p>
        </div>
      )}
      {view === "wat" && (
        <>
          <p className="mb-2 text-[11px] text-slate-500">
            The executed WebAssembly core module — unwrapped from the component and stripped of debug
            info, so you see just the code.
          </p>
          <pre className="whitespace-pre text-slate-400">{pending(compiled.wat)}</pre>
        </>
      )}
      {view === "rust" && <pre className="whitespace-pre text-slate-400">{pending(compiled.rustSync)}</pre>}
      {view === "rustAsync" && <pre className="whitespace-pre text-slate-400">{pending(compiled.rustAsync)}</pre>}
      {view === "cadenza" && (
        <>
          <p className="mb-2 text-[11px] text-slate-500">
            The program lowered + optimized back to Cadenza (the <code>--target cadenza</code> backend): the
            same optimizations the WebAssembly and Rust targets get, printed as Cadenza source.
          </p>
          <pre className="whitespace-pre text-slate-400">{pending(compiled.cadenza)}</pre>
        </>
      )}
    </div>
  );
}

function SubTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={
        "rounded px-2 py-0.5 transition " +
        (active ? "bg-slate-700/60 text-slate-100" : "text-slate-500 hover:text-slate-300")
      }
    >
      {children}
    </button>
  );
}

function DiagnosticsBody({
  diagnostics,
  surface,
  onJumpTo,
  onApplyFix,
}: {
  diagnostics: Diag[];
  surface: Surface;
  onJumpTo: (f: number, t: number) => void;
  onApplyFix: (d: Diag) => void;
}) {
  if (diagnostics.length === 0) return <span className="text-emerald-400">No problems — the program is well-formed.</span>;
  return (
    <ul className="space-y-1.5">
      {diagnostics.map((d, i) => (
        <li key={i}>
          <button
            onClick={() => onJumpTo(d.from, d.to)}
            className={
              "block w-full rounded px-2 py-1 text-left transition hover:bg-slate-800/60 " +
              (d.error ? "text-rose-300" : "text-amber-300")
            }
          >
            <StatusIcon kind={d.error ? "error" : "trap"} />{" "}
            {d.code && <span className="font-semibold">{d.code} </span>}
            {d.message}
          </button>
          {d.fix && fixIsApplicable(d.fix, surface) && (
            <button
              onClick={() => onApplyFix(d)}
              title={d.fix.verified ? "Compiler-proven — safe to apply" : "A suggestion — confirm it matches your intent"}
              className="ml-6 mt-0.5 inline-flex items-center gap-1 rounded border border-cadenza-600/40 bg-cadenza-600/10 px-2 py-0.5 text-[11px] text-cadenza-200 transition hover:bg-cadenza-600/20"
            >
              💡 {fixActionLabel(d)}
              <span className="text-cadenza-400/70">· {fixConfidence(d.fix)}</span>
            </button>
          )}
        </li>
      ))}
    </ul>
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
