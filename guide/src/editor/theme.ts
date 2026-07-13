/// Highlight styling for the Cadenza editor. TWO layers, applied in order:
///  1. `cadenzaHighlighting` — the LEXICAL `HighlightStyle` the `StreamLanguage` tokenizer drives. It
///     paints instantly on every keystroke, classifying by SHAPE (keyword set, capitalization, literal
///     form). This is the always-on base coat.
///  2. `cadenzaSemanticTheme` — the SEMANTIC overlay the compiler's `Highlight` query drives
///     (`cadenzaSemanticHighlight.ts`). It re-colours the tokens the compiler recognizes by their ROLE
///     (a type vs a constructor vs a local vs an unbound typo), which shape alone can't tell apart. Its
///     `.cm-cadenza-tok-<kind>` mark classes out-specify the lexical tag styles, so where the compiler
///     has an opinion it wins; where it doesn't (an unparseable mid-edit buffer, or an ML-only keyword
///     with no AST node), the lexical coat shows through.
/// One palette for both light and dark (reads acceptably on both editor backgrounds). Kept in
/// module-level constants so the editor's `extensions` array is stable across renders (so the global
/// syntax toggle reconfigures rather than remounts).

import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

const highlight = HighlightStyle.define([
  { tag: t.keyword, color: "#c084fc", fontWeight: "600" },
  { tag: t.comment, color: "#6b7280", fontStyle: "italic" },
  { tag: t.string, color: "#86efac" },
  { tag: t.number, color: "#f0abfc" },
  { tag: t.atom, color: "#fbbf24" },
  { tag: t.typeName, color: "#7dd3fc" },
  { tag: t.variableName, color: "#e5e7eb" },
  { tag: t.operator, color: "#94a3b8" },
]);

export const cadenzaHighlighting: Extension = syntaxHighlighting(highlight);

/// The SEMANTIC overlay palette — one colour per `HighlightKind` the compiler emits, keyed on the mark
/// class `cadenzaSemanticHighlight` applies. Chosen to READ AS A REFINEMENT of the lexical coat (a type
/// stays sky-blue, a keyword stays violet) while pulling apart what shape conflated: a CONSTRUCTOR reads
/// amber (a value that builds data, not a plain call), a FUNCTION teal-green (a callable), a PARAM
/// italic-blue vs a plain VARIABLE off-white, an EFFECT rose, a LABEL muted violet (it is metadata), and
/// an UNBOUND name red + wavy-underlined (the one thing shape can never flag). Colours only — no layout
/// shift — so the overlay never reflows the editor.
export const cadenzaSemanticTheme: Extension = EditorView.theme({
  ".cm-cadenza-tok-keyword": { color: "#c084fc", fontWeight: "600" },
  ".cm-cadenza-tok-type": { color: "#7dd3fc" },
  ".cm-cadenza-tok-constructor": { color: "#fbbf24" },
  ".cm-cadenza-tok-function": { color: "#5eead4" },
  ".cm-cadenza-tok-param": { color: "#93c5fd", fontStyle: "italic" },
  ".cm-cadenza-tok-variable": { color: "#e5e7eb" },
  ".cm-cadenza-tok-effect": { color: "#fda4af" },
  ".cm-cadenza-tok-label": { color: "#c4b5fd" },
  ".cm-cadenza-tok-number": { color: "#f0abfc" },
  ".cm-cadenza-tok-string": { color: "#86efac" },
  ".cm-cadenza-tok-char": { color: "#86efac" },
  ".cm-cadenza-tok-bytes": { color: "#a7f3d0" },
  ".cm-cadenza-tok-symbol": { color: "#fbbf24" },
  ".cm-cadenza-tok-literal": { color: "#fbbf24" },
  ".cm-cadenza-tok-unbound": {
    color: "#f87171",
    textDecoration: "underline wavy #f87171",
    textUnderlineOffset: "3px",
  },
});
