/// The playground's CodeMirror editor: line numbers, lint gutter + inline squiggles (an async
/// `linter()` source that compiles in the worker), and type-on-hover. It captures the `EditorView`
/// so the page can jump the selection to a diagnostic's span. The buffer is a full module compiled
/// verbatim, so the wrap prefix is empty (identity `prepare`).

import { useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { cadenzaLanguage } from "../editor/cadenzaLanguage.ts";
import { cadenzaHighlighting, cadenzaSemanticTheme } from "../editor/theme.ts";
import { cadenzaHover } from "./cadenzaHover.ts";
import { cadenzaGotoDef } from "./cadenzaGotoDef.ts";
import { cadenzaHighlightRefs } from "./cadenzaHighlightRefs.ts";
import { cadenzaSemanticHighlight } from "./cadenzaSemanticHighlight.ts";
import { cadenzaLinter, lintGutter } from "./lintField.ts";
import type { Diag, Surface } from "../compiler/client.ts";

const editorTheme = EditorView.theme({
  "&": { fontSize: "13.5px", height: "100%", backgroundColor: "transparent" },
  "&.cm-editor": { height: "100%" },
  ".cm-scroller": {
    fontFamily: "ui-monospace, SFMono-Regular, 'JetBrains Mono', Menlo, Consolas, monospace",
    overflow: "auto",
  },
  ".cm-content": { padding: "12px 0" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "#4b5563" },
  "&.cm-focused": { outline: "none" },
  ".cm-cadenza-hover": {
    fontFamily: "ui-monospace, monospace",
    fontSize: "12.5px",
    color: "#7dd3fc",
    background: "#0f172a",
    border: "1px solid #334155",
    borderRadius: "6px",
    padding: "3px 8px",
    maxWidth: "42ch",
  },
  // The compilation-disposition section of the hover (below the type): the disposition word in amber,
  // its gloss dimmed, and each concrete instantiation indented.
  ".cm-cadenza-hover-disp": {
    color: "#fbbf24",
    marginTop: "3px",
    paddingTop: "3px",
    borderTop: "1px solid #1e293b",
  },
  ".cm-cadenza-hover-gloss": { color: "#64748b", fontSize: "11.5px" },
  ".cm-cadenza-hover-inst": { color: "#a5b4fc", paddingLeft: "10px" },
  // Every occurrence of the name the caret rests on (find-all-references).
  ".cm-cadenza-ref": {
    backgroundColor: "rgba(251, 191, 36, 0.18)",
    borderRadius: "2px",
  },
});

// Identity prepare: the playground buffer is already a full module compiled verbatim.
const identityPrepare = (compiled: string) => ({ compiled, wrapPrefixBytes: 0 });

interface Props {
  value: string;
  onChange: (v: string) => void;
  surface: Surface;
  onDiagnostics?: (diags: Diag[]) => void;
  onCursor?: (line: number, col: number) => void;
  onView?: (view: EditorView) => void;
}

export function PlaygroundEditor({ value, onChange, surface, onDiagnostics, onCursor, onView }: Props) {
  const [, force] = useState(0);
  // The linter/hover extensions are built ONCE (a stable extensions array so a re-render doesn't
  // reconfigure the editor); they read the live surface/callbacks through refs.
  const surfaceRef = useRef(surface);
  surfaceRef.current = surface;
  const onDiagRef = useRef(onDiagnostics);
  onDiagRef.current = onDiagnostics;
  const onCursorRef = useRef(onCursor);
  onCursorRef.current = onCursor;

  const extensions = useMemo<Extension[]>(
    () => [
      cadenzaLanguage,
      cadenzaHighlighting,
      cadenzaSemanticTheme,
      editorTheme,
      lintGutter(),
      cadenzaLinter({
        surface: () => surfaceRef.current,
        prepare: identityPrepare,
        onDiagnostics: (d) => onDiagRef.current?.(d),
      }),
      cadenzaHover({ surface: () => surfaceRef.current, prepare: identityPrepare }),
      cadenzaGotoDef({ surface: () => surfaceRef.current, prepare: identityPrepare }),
      cadenzaHighlightRefs({ surface: () => surfaceRef.current, prepare: identityPrepare }),
      cadenzaSemanticHighlight({ surface: () => surfaceRef.current, prepare: identityPrepare }),
      EditorView.updateListener.of((u) => {
        if (u.selectionSet) {
          const head = u.state.selection.main.head;
          const line = u.state.doc.lineAt(head);
          onCursorRef.current?.(line.number, head - line.from + 1);
        }
      }),
    ],
    [],
  );

  return (
    <CodeMirror
      onCreateEditor={(v) => {
        onView?.(v);
        force((n) => n + 1);
      }}
      value={value}
      onChange={onChange}
      extensions={extensions}
      basicSetup={{
        lineNumbers: true,
        foldGutter: false,
        highlightActiveLine: true,
        autocompletion: false,
      }}
      theme="dark"
      height="100%"
      style={{ height: "100%" }}
    />
  );
}
