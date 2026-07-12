/// The embeddable Cadenza code editor — a thin wrapper over CodeMirror 6 (`@uiw/react-codemirror`)
/// with the Cadenza tokenizer + highlight style. `extensions` is memoized so a syntax toggle (which
/// changes `value`) reconfigures the doc rather than remounting the editor.
///
/// With `ide` set, it turns on a MINIMAL IDE experience — inline error squiggles + a lint gutter
/// (an async worker linter) and type-on-hover — for the guide's inline `Runnable`/`Exercise` editors.
/// The snippet is usually a bare expression, so `ide.prepare` wraps it into a compilable module and
/// reports the wrapper's byte length, letting spans map back to the editor text.

import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { cadenzaLanguage } from "./cadenzaLanguage.ts";
import { cadenzaHighlighting } from "./theme.ts";
import { cadenzaLinter, lintGutter } from "../playground/lintField.ts";
import { cadenzaHover } from "../playground/cadenzaHover.ts";
import { cadenzaGotoDef } from "../playground/cadenzaGotoDef.ts";
import { cadenzaHighlightRefs } from "../playground/cadenzaHighlightRefs.ts";
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
  },
  ".cm-cadenza-ref": {
    backgroundColor: "rgba(251, 191, 36, 0.18)",
    borderRadius: "2px",
  },
});

// Stable across renders — never rebuild this array (would thrash every editor on a global toggle).
const BASE_EXTENSIONS: Extension[] = [cadenzaLanguage, cadenzaHighlighting, editorTheme];

/// The minimal-IDE hookup for an inline editor: read the live surface + wrap the snippet into a
/// compilable module so diagnostics/hover work on a bare expression.
export interface IdeConfig {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

interface Props {
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  minHeight?: string;
  /** When set, enable inline squiggles + hover (a minimal IDE) for this editor. */
  ide?: IdeConfig;
}

export function CodeEditor({ value, onChange, readOnly, minHeight = "auto", ide }: Props) {
  // Only build a per-instance extensions array when IDE features are on; otherwise share the stable
  // base array (so a page of plain editors doesn't each carry a linter). Keyed on nothing but `ide`
  // presence — the linter/hover read the live surface through the callbacks `ide` provides.
  const extensions = useMemo<Extension[]>(() => {
    if (!ide) return BASE_EXTENSIONS;
    return [
      ...BASE_EXTENSIONS,
      lintGutter(),
      cadenzaLinter({ surface: ide.surface, prepare: ide.prepare }),
      cadenzaHover({ surface: ide.surface, prepare: ide.prepare }),
      cadenzaGotoDef({ surface: ide.surface, prepare: ide.prepare }),
      cadenzaHighlightRefs({ surface: ide.surface, prepare: ide.prepare }),
    ];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [!!ide]);

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
