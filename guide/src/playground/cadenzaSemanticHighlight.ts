/// COMPILER-DRIVEN semantic syntax highlighting. The editor's `StreamLanguage` tokenizer
/// (`cadenzaLanguage.ts`) paints instantly on every keystroke but classifies by SHAPE only — it can't
/// tell a type from a constructor from a local from an unbound typo. This layer overlays the COMPILER's
/// classification on top: it asks `semanticTokens` (the `Highlight` sidecar query) for every token's
/// ROLE, read off the resolved column + the meta channels a value carries, and re-colours the tokens the
/// compiler recognizes. A name coloured here is coloured by what it MEANS, not how it is spelled.
///
/// HYBRID by design: the lexical tokenizer stays (it owns the instant paint and the ML-only keywords
/// that have no AST node — `then`/`else`/`in`/`with`); this overlay REFINES the tokens that do have
/// nodes, debounced off the UI thread. An unparseable mid-edit buffer yields no tokens, so the editor
/// simply keeps the lexical colours until the next well-parsed edit — highlighting never flickers to
/// "wrong", only "less precise for a moment".
///
/// A CodeMirror StateField of mark decorations, refreshed (debounced) on doc change; the async compiler
/// answer is pushed back in through a StateEffect. Positions convert UTF-16 (editor) ↔ UTF-8 bytes
/// (compiler); a wrapped inline snippet shifts the compiled text, so ranges are mapped back by the
/// wrapper's byte length.

import { EditorView, Decoration, type DecorationSet } from "@codemirror/view";
import { StateField, StateEffect, type Extension, RangeSetBuilder } from "@codemirror/state";
import { semanticTokens, type Surface, type SemanticTok } from "../compiler/client.ts";
import { byteToUtf16, utf16ToByte } from "./offsets.ts";

export interface SemanticHighlightContext {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

/// The effect that carries a fresh set of editor-coordinate tokens to paint.
const setTokens = StateEffect.define<{ from: number; to: number; kind: string }[]>();

/// One reusable mark decoration per kind — `cm-cadenza-tok-<kind>`, themed in `theme.ts`. Built lazily
/// and cached so identical kinds share one `Decoration` instance (CodeMirror de-dups by identity).
const markCache = new Map<string, Decoration>();
function markFor(kind: string): Decoration {
  let m = markCache.get(kind);
  if (!m) {
    m = Decoration.mark({ class: `cm-cadenza-tok-${kind}` });
    markCache.set(kind, m);
  }
  return m;
}

const tokenField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(deco, tr) {
    // Map existing marks through edits so they don't drift before the next refresh.
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setTokens)) {
        const b = new RangeSetBuilder<Decoration>();
        // RangeSetBuilder requires ascending `from` (and non-empty ranges); the compiler emits tokens in
        // ascending node-id order, which is NOT source order, so sort by position first.
        const sorted = [...e.value].filter((t) => t.to > t.from).sort((a, z) => a.from - z.from);
        for (const t of sorted) b.add(t.from, t.to, markFor(t.kind));
        deco = b.finish();
      }
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export function cadenzaSemanticHighlight(ctx: SemanticHighlightContext): Extension {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let gen = 0;

  async function refresh(view: EditorView) {
    const myGen = ++gen;
    const editorText = view.state.doc.toString();
    const surface = ctx.surface();
    const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
    let toks: SemanticTok[];
    try {
      toks = await semanticTokens(compiled, surface);
    } catch {
      return; // a compiler hiccup leaves the lexical colours in place
    }
    if (myGen !== gen) return; // superseded by a newer edit / surface toggle
    const ranges: { from: number; to: number; kind: string }[] = [];
    const editorBytes = utf16ToByte(editorText, editorText.length);
    for (const t of toks) {
      // Shift out of the compiled (wrapped) coordinate space back to the editor text; drop any token
      // that lands inside the wrapper scaffolding (before the snippet, or past its end).
      const fromByte = t.from - wrapPrefixBytes;
      const toByte = t.to - wrapPrefixBytes;
      if (fromByte < 0 || toByte < 0 || fromByte > editorBytes) continue;
      ranges.push({
        from: byteToUtf16(editorText, fromByte),
        to: byteToUtf16(editorText, toByte),
        kind: t.kind,
      });
    }
    view.dispatch({ effects: setTokens.of(ranges) });
  }

  let painted = false;
  const watcher = EditorView.updateListener.of((u) => {
    // Refresh on any content change (a snippet edit, or a surface toggle — which replaces the doc); also
    // ONCE on the first layout, so an editor that is never edited still gets its semantic colours. A pure
    // selection move / scroll needs no refresh — classification is position-independent (unlike the
    // find-refs highlighter, which follows the caret).
    const initial = !painted && u.view.dom.isConnected;
    if (!u.docChanged && !initial) return;
    painted = true;
    clearTimeout(timer);
    const view = u.view;
    timer = setTimeout(() => void refresh(view), 150);
  });

  return [tokenField, watcher];
}
