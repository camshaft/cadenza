/// The embeddable Cadenza code editor — a thin wrapper over CodeMirror 6 (`@uiw/react-codemirror`)
/// with the Cadenza tokenizer + highlight style. `extensions` is memoized so a syntax toggle (which
/// changes `value`) reconfigures the doc rather than remounting the editor.
///
/// With `ide` set, it turns on a MINIMAL IDE experience — inline error squiggles + a lint gutter
/// (an async worker linter) and type-on-hover — for the guide's inline `Runnable`/`Exercise` editors.
/// The snippet is usually a bare expression, so `ide.prepare` wraps it into a compilable module and
/// reports the wrapper's byte length, letting spans map back to the editor text.

import { useMemo, useRef } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { cadenzaLanguage } from "./cadenzaLanguage.ts";
import { cadenzaHighlighting, cadenzaSemanticTheme } from "./theme.ts";
import { cadenzaLinter, lintGutter } from "../playground/lintField.ts";
import { cadenzaHover } from "../playground/cadenzaHover.ts";
import { cadenzaGotoDef } from "../playground/cadenzaGotoDef.ts";
import { cadenzaHighlightRefs } from "../playground/cadenzaHighlightRefs.ts";
import { cadenzaSemanticHighlight } from "../playground/cadenzaSemanticHighlight.ts";
import type { Surface } from "../compiler/client.ts";

const editorTheme = EditorView.theme({
  "&": {
    fontSize: "13.5px",
    backgroundColor: "transparent",
  },
  ".cm-content": {
    fontFamily:
      "ui-monospace, SFMono-Regular, 'JetBrains Mono', Menlo, Consolas, monospace",
    padding: "12px 0",
  },
  ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "#4b5563" },
  "&.cm-focused": { outline: "none" },
  ".cm-line": { padding: "0 12px" },
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
  // The compilation-disposition section of the hover (below the type) — see PlaygroundEditor.
  ".cm-cadenza-hover-disp": {
    color: "#fbbf24",
    marginTop: "3px",
    paddingTop: "3px",
    borderTop: "1px solid #1e293b",
  },
  ".cm-cadenza-hover-gloss": { color: "#64748b", fontSize: "11.5px" },
  ".cm-cadenza-hover-inst": { color: "#a5b4fc", paddingLeft: "10px" },
  ".cm-cadenza-ref": {
    backgroundColor: "rgba(251, 191, 36, 0.18)",
    borderRadius: "2px",
  },
});

// Stable across renders — never rebuild this array (would thrash every editor on a global toggle).
const BASE_EXTENSIONS: Extension[] = [cadenzaLanguage, cadenzaHighlighting, editorTheme];
// A PLAIN-text editor (no Cadenza language/highlighting) — for editing content that ISN'T Cadenza, e.g. a
// notebook PROSE cell's markdown (operator UX #4). Just the shared dark theme; no `ide` is passed for these,
// so no linter/hover either. Kept separate + stable so a plain editor never carries the Cadenza stack.
const PLAIN_EXTENSIONS: Extension[] = [editorTheme];

/// The minimal-IDE hookup for an inline editor: read the live surface + wrap the snippet into a
/// compilable module so diagnostics/hover work on a bare expression.
export interface IdeConfig {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
  /** Optional PRELOADED library modules link-merged for diagnostics (three parallel name/source/format
   *  arrays). When set, the linter compiles the prepared text with `diagnosticsWithPreloaded` so a buffer
   *  that `import`s a preloaded module (e.g. /cad's model against the CAD library) doesn't show the
   *  preloaded vocab as unbound. Omitted → the linter uses plain `diagnostics` (unchanged). */
  preload?: () => { names: string[]; sources: string[]; formats: string[] };
}

interface Props {
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  minHeight?: string;
  /** When set, enable inline squiggles + hover (a minimal IDE) for this editor. */
  ide?: IdeConfig;
  /** The editor's content language. `"cadenza"` (default) applies the Cadenza language + highlighting;
   *  `"plain"` is a bare text editor (no Cadenza tokenizer) — for non-Cadenza content like a notebook PROSE
   *  cell's markdown (operator UX #4). `ide` is ignored in `"plain"` mode (there's nothing to lint). */
  language?: "cadenza" | "plain";
}

export function CodeEditor({ value, onChange, readOnly, minHeight = "auto", ide, language = "cadenza" }: Props) {
  // ⚠ The extensions array is memoized on `[!!ide]` (present/absent) — CodeMirror keeps ONE extension
  // instance across re-renders (rebuilding it would remount the editor + drop focus/cursor on every
  // keystroke). So the linter/hover/highlight closures must NOT capture a SNAPSHOT of `ide` — a caller
  // like NotebookPage rebuilds `ide` each render (a fresh `cellIde(...)` closing over the CURRENT surface),
  // and a captured snapshot would freeze the FIRST surface forever. Concretely: the notebook mounts in
  // one surface, then an async re-render flips `docSurface` → a NEW `ide` with the new surface, but the
  // frozen linter would keep diagnosing against the OLD surface → an ML cell linted as s-expr ("unbound
  // name main"). We route every extension through a LIVE ref (`ideRef`) updated each render, so the
  // closures always read the current `ide.surface`/`ide.prepare`/`ide.preload` without rebuilding.
  const ideRef = useRef<IdeConfig | undefined>(ide);
  ideRef.current = ide;
  const extensions = useMemo<Extension[]>(() => {
    // A `"plain"` editor (notebook prose/markdown) carries NO Cadenza language or IDE — just the theme.
    if (language === "plain") return PLAIN_EXTENSIONS;
    if (!ide) return BASE_EXTENSIONS;
    // Stable indirection: each accessor reads the LIVE `ideRef.current` (never the captured `ide`).
    const surface = () => ideRef.current!.surface();
    const prepare = (t: string, s: Surface) => ideRef.current!.prepare(t, s);
    // `preload` is a per-route capability (present on /cad, absent on /notebook) — stable across a route's
    // life, so gate it on the initial `ide`. When present, route through the ref (live), returning an empty
    // set if the live `ide` momentarily lacks it (a no-op — the consumers treat 0 names as "no preload").
    const preload: (() => { names: string[]; sources: string[]; formats: string[] }) | undefined = ide.preload
      ? () => ideRef.current!.preload?.() ?? { names: [], sources: [], formats: [] }
      : undefined;
    return [
      ...BASE_EXTENSIONS,
      lintGutter(),
      cadenzaLinter({ surface, prepare, preload }),
      cadenzaHover({ surface, prepare }),
      cadenzaGotoDef({ surface, prepare }),
      cadenzaHighlightRefs({ surface, prepare }),
      cadenzaSemanticHighlight({ surface, prepare, preload }),
      cadenzaSemanticTheme,
    ];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [!!ide, language]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      editable={!readOnly}
      readOnly={readOnly}
      extensions={extensions}
      basicSetup={{
        lineNumbers: false,
        foldGutter: false,
        highlightActiveLine: !readOnly,
        highlightActiveLineGutter: false,
        autocompletion: false,
        searchKeymap: false,
      }}
      theme="dark"
      style={{ minHeight }}
    />
  );
}
