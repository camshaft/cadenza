/// Highlight-all-references: when the caret rests on a name, softly highlight every occurrence that
/// references the same top-level definition (via the compiler's `UsesOf`). A CodeMirror StateField of
/// mark decorations, refreshed (debounced) whenever the selection moves; the async compiler answer is
/// pushed back in through a StateEffect.
///
/// Positions convert UTF-16 (editor) ↔ UTF-8 bytes (compiler); a wrapped inline snippet shifts the
/// cursor into the compiled text and the returned ranges back out.

import { EditorView, Decoration, type DecorationSet } from "@codemirror/view";
import { StateField, StateEffect, type Extension, RangeSetBuilder } from "@codemirror/state";
import { references_at, type Surface } from "../compiler/client.ts";
import { byteToUtf16, utf16ToByte } from "./offsets.ts";

export interface HighlightContext {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

/// The effect that carries a fresh set of editor-coordinate ranges to highlight.
const setRefs = StateEffect.define<{ from: number; to: number }[]>();

const refMark = Decoration.mark({ class: "cm-cadenza-ref" });

const refField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(deco, tr) {
    // Map existing marks through edits so they don't drift before the next refresh.
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setRefs)) {
        const b = new RangeSetBuilder<Decoration>();
        // RangeSetBuilder requires ascending `from`; the compiler's use list isn't sorted by position.
        const sorted = [...e.value].filter((r) => r.to > r.from).sort((a, z) => a.from - z.from);
        for (const r of sorted) b.add(r.from, r.to, refMark);
        deco = b.finish();
      }
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export function cadenzaHighlightRefs(ctx: HighlightContext): Extension {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let gen = 0;

  const watcher = EditorView.updateListener.of((u) => {
    if (!u.selectionSet && !u.docChanged) return;
    clearTimeout(timer);
    const myGen = ++gen;
    const view = u.view;
    timer = setTimeout(async () => {
      const editorText = view.state.doc.toString();
      const head = view.state.selection.main.head;
      const surface = ctx.surface();
      const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
      let flat: Uint32Array;
      try {
        flat = await references_at(compiled, surface, utf16ToByte(editorText, head) + wrapPrefixBytes);
      } catch {
        return;
      }
      if (myGen !== gen) return; // superseded by a newer caret move / edit
      const ranges: { from: number; to: number }[] = [];
      // Only highlight when there's more than one occurrence — a lone name isn't worth a mark.
      if (flat.length > 2) {
        for (let i = 0; i + 1 < flat.length; i += 2) {
          const fromByte = flat[i] - wrapPrefixBytes;
          const toByte = flat[i + 1] - wrapPrefixBytes;
          if (toByte < 0) continue;
          ranges.push({
            from: byteToUtf16(editorText, Math.max(0, fromByte)),
            to: byteToUtf16(editorText, Math.max(0, toByte)),
          });
        }
      }
      view.dispatch({ effects: setRefs.of(ranges) });
    }, 200);
  });

  return [refField, watcher];
}
