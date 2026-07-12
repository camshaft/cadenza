/// Highlight styling for the Cadenza editor, mapping the tokenizer's tags to colors. One style used
/// in both light and dark (the colors read acceptably on both editor backgrounds); a dedicated dark
/// variant can be added later. Kept in a module-level constant so the editor's `extensions` array is
/// stable across renders (important so the global syntax toggle reconfigures rather than remounts).

import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

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
