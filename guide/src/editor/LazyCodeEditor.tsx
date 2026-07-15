/// A lazy boundary around `CodeEditor` — so the heavy CodeMirror stack (~406 kB) is NOT on the guide's
/// critical path. `CodeEditor` statically imports all of CodeMirror + the IDE extensions; because every
/// inline `<Runnable>`/`<Exercise>` (and the eager HomePage route) renders one, that stack was
/// modulepreloaded in index.html and fetched before first paint even for a reader who never opens an
/// editor. Splitting it behind `React.lazy` moves it off the critical path: first paint no longer waits
/// on CodeMirror, and the fallback shows the snippet as plain text so the example is legible instantly.

import { lazy, Suspense } from "react";
import type { ComponentProps } from "react";
import type { CodeEditor as CodeEditorType } from "./CodeEditor.tsx";

const CodeEditor = lazy(() =>
  import("./CodeEditor.tsx").then((m) => ({ default: m.CodeEditor })),
);

type CodeEditorProps = ComponentProps<typeof CodeEditorType>;

/// A plain-text stand-in shown while the CodeMirror chunk loads — same monospace look + padding as the
/// editor's content, so the example reads identically and there's no layout jump when it upgrades.
function CodeFallback({ value, minHeight }: { value: string; minHeight?: string }) {
  return (
    <pre
      className="overflow-x-auto px-3 py-3 font-mono text-[13.5px] leading-normal text-slate-200"
      style={{ minHeight, margin: 0 }}
    >
      {value}
    </pre>
  );
}

export function LazyCodeEditor(props: CodeEditorProps) {
  return (
    <Suspense fallback={<CodeFallback value={props.value} minHeight={props.minHeight} />}>
      <CodeEditor {...props} />
    </Suspense>
  );
}
