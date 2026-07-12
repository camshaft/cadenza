/// Type-on-hover: a CodeMirror `hoverTooltip` source that asks the compiler for the inferred type at
/// the hovered offset and shows it in a small bubble. Async (a worker query) is first-class for
/// `hoverTooltip`. Positions convert UTF-16 (editor) ↔ UTF-8 bytes (compiler); when the editor text
/// is wrapped into a module before compiling, `wrapPrefixBytes` shifts the cursor into the compiled
/// text and shifts the returned range back.

import { hoverTooltip, type Tooltip } from "@codemirror/view";
import { typeAt, type Surface } from "../compiler/client.ts";
import { byteToUtf16, utf16ToByte } from "./offsets.ts";

export interface HoverContext {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

export function cadenzaHover(ctx: HoverContext) {
  return hoverTooltip(
    async (view, pos): Promise<Tooltip | null> => {
      const editorText = view.state.doc.toString();
      const surface = ctx.surface();
      const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
      const byteInEditor = utf16ToByte(editorText, pos);
      const info = await typeAt(compiled, surface, byteInEditor + wrapPrefixBytes);
      if (!info) return null;
      // Map the returned byte range back into editor coordinates.
      const fromByte = info.from - wrapPrefixBytes;
      const toByte = info.to - wrapPrefixBytes;
      if (toByte < 0) return null; // inside the wrapper, not the user's text
      const from = byteToUtf16(editorText, Math.max(0, fromByte));
      const to = byteToUtf16(editorText, Math.max(0, toByte));
      return {
        pos: from,
        end: to,
        above: true,
        create() {
          const dom = document.createElement("div");
          dom.className = "cm-cadenza-hover";
          dom.textContent = info.typeName;
          return { dom };
        },
      };
    },
    { hoverTime: 300, hideOnChange: true },
  );
}
