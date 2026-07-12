/// The embeddable Cadenza code editor — a thin wrapper over CodeMirror 6 (`@uiw/react-codemirror`)
/// with the Cadenza tokenizer + highlight style. `extensions` is memoized at module scope so a
/// syntax toggle (which changes `value`) reconfigures the doc rather than remounting the editor.

import CodeMirror from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { cadenzaLanguage } from "./cadenzaLanguage.ts";
import { cadenzaHighlighting } from "./theme.ts";

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
});

// Stable across renders — never rebuild this array (would thrash every editor on a global toggle).
const EXTENSIONS: Extension[] = [cadenzaLanguage, cadenzaHighlighting, editorTheme];

interface Props {
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  minHeight?: string;
}

export function CodeEditor({ value, onChange, readOnly, minHeight = "auto" }: Props) {
  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      editable={!readOnly}
      readOnly={readOnly}
      extensions={EXTENSIONS}
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
