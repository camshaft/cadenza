/// The playground's tabbed output pane: Result (the run outcome), Diagnostics (every fault, click to
/// jump), and AST (the raw tree the compiler sees — a nearly-free differentiator).

import { useState } from "react";
import type { Diag } from "../compiler/client.ts";
import { StatusIcon } from "../components/StatusIcon.tsx";

export type RunView =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

type Tab = "result" | "diagnostics" | "ast" | "compiled";

/// A summary of the last successful compile, for the "Compiled" tab.
export interface CompiledInfo {
  bytes: number;
  /** true if the emitted component imports the value-heap runtime (a compound result). */
  importsRuntime: boolean;
}

interface Props {
  run: RunView;
  diagnostics: Diag[];
  ast: string;
  compiled: CompiledInfo | null;
  /** Jump the editor to a diagnostic's source range. */
  onJumpTo: (from: number, to: number) => void;
}

export function OutputPanel({ run, diagnostics, ast, compiled, onJumpTo }: Props) {
  const [tab, setTab] = useState<Tab>("result");
  const errorCount = diagnostics.filter((d) => d.error).length;

  return (
    <div className="flex h-full flex-col bg-slate-900/40">
      <div className="flex items-center gap-1 border-b border-slate-700/60 bg-slate-800/50 px-2 py-1">
        <TabButton active={tab === "result"} onClick={() => setTab("result")}>
          Result
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

      <div className="min-h-0 flex-1 overflow-auto p-3 font-mono text-[13px]">
        {tab === "result" && <ResultBody run={run} />}
        {tab === "diagnostics" && <DiagnosticsBody diagnostics={diagnostics} onJumpTo={onJumpTo} />}
        {tab === "ast" && (
          <pre className="whitespace-pre-wrap text-slate-400">{ast || "— run or edit to see the tree —"}</pre>
        )}
        {tab === "compiled" && <CompiledBody compiled={compiled} />}
      </div>
    </div>
  );
}

function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={
        "flex items-center rounded px-2.5 py-1 text-xs font-medium transition " +
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

function CompiledBody({ compiled }: { compiled: CompiledInfo | null }) {
  if (!compiled) {
    return <span className="text-slate-500">Run a well-formed program to see what it compiles to.</span>;
  }
  return (
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
        Cadenza compiles to a sandboxed WebAssembly component. A program that returns a compound value
        imports a shared value-heap runtime; a scalar one is fully self-contained.
      </p>
    </div>
  );
}

function DiagnosticsBody({ diagnostics, onJumpTo }: { diagnostics: Diag[]; onJumpTo: (f: number, t: number) => void }) {
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
        </li>
      ))}
    </ul>
  );
}
