/// Go-to-definition: ⌘/Ctrl-click an identifier to jump to its definition. A CodeMirror
/// `domEventHandlers` mousedown listener (when the meta/ctrl modifier is held) maps the clicked
/// position to a UTF-8 byte offset, asks the compiler where that reference is defined (`defineAt`),
/// and — if it's a navigable reference — selects the definition's range and scrolls to it.
///
/// Positions convert UTF-16 (editor) ↔ UTF-8 bytes (compiler); when the editor text is wrapped into a
/// module before compiling, `wrapPrefixBytes` shifts the cursor into the compiled text and shifts the
/// returned range back.

import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { defineAt, type Surface } from "../compiler/client.ts";
import { byteToUtf16, utf16ToByte } from "./offsets.ts";

export interface GotoContext {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

export function cadenzaGotoDef(ctx: GotoContext): Extension {
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      // Only on a modifier-click (the conventional go-to-def gesture); a plain click is a caret move.
      if (!(event.metaKey || event.ctrlKey)) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      const editorText = view.state.doc.toString();
      const surface = ctx.surface();
      const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
      const byteInEditor = utf16ToByte(editorText, pos);
      // Fire the async lookup; if it lands on a definition, jump. We prevent the default caret placement
      // only after confirming a hit would feel wrong (the lookup is async), so instead we always let the
      // caret move, then move it again to the definition when the answer arrives — snappy enough.
      void defineAt(compiled, surface, byteInEditor + wrapPrefixBytes).then((d) => {
        if (!d) return;
        const fromByte = d.from - wrapPrefixBytes;
        const toByte = d.to - wrapPrefixBytes;
        if (toByte < 0) return; // inside the generated wrapper, not the user's text
        const from = byteToUtf16(editorText, Math.max(0, fromByte));
        const to = byteToUtf16(editorText, Math.max(0, toByte));
        view.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
        view.focus();
      });
      return false;
    },
  });
}
