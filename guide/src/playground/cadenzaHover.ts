/// Hover tooltip: asks the compiler about the token under the pointer and shows what it learns in a
/// small bubble. Two facts, one bubble:
///   - the inferred TYPE at the offset (`typeAt`), and
///   - when the offset is on a DEFINITION name, how the compiler COMPILED it (`disposition`): inlined,
///     specialized (with each concrete monomorphization), emitted as a standalone function, transformed
///     into an accumulator loop, or unreferenced — the reverse of "one source def, one function".
///
/// Async (worker queries) is first-class for `hoverTooltip`. Positions convert UTF-16 (editor) ↔ UTF-8
/// bytes (compiler); when the editor text is wrapped into a module before compiling, `wrapPrefixBytes`
/// shifts the cursor into the compiled text and shifts returned ranges back. The two queries run
/// concurrently; the disposition section is simply omitted when the token isn't a definition name.

import { hoverTooltip, type Tooltip } from "@codemirror/view";
import { typeAt, disposition, type Surface } from "../compiler/client.ts";
import { byteToUtf16, utf16ToByte } from "./offsets.ts";

export interface HoverContext {
  surface: () => Surface;
  prepare: (editorText: string, surface: Surface) => { compiled: string; wrapPrefixBytes: number };
}

/// A one-line human gloss for a disposition — what it MEANS, so the bare word is self-explanatory. A
/// `transformed→copy` tag or a `+`-joined combination carries its own words (no extra gloss).
function dispositionGloss(disp: string): string {
  switch (disp) {
    case "inlined":
      return "β-reduced into each call site — no standalone function emitted";
    case "specialized":
      return "monomorphized — one function per instantiation:";
    case "emitted":
      return "emitted as one standalone function and called";
    case "unreferenced":
      return "never called, inlined, specialized, or exported";
    default:
      return disp.startsWith("transformed")
        ? "its recursion was rewritten into an accumulator loop"
        : "";
  }
}

export function cadenzaHover(ctx: HoverContext) {
  return hoverTooltip(
    async (view, pos): Promise<Tooltip | null> => {
      const editorText = view.state.doc.toString();
      const surface = ctx.surface();
      const { compiled, wrapPrefixBytes } = ctx.prepare(editorText, surface);
      const byteInEditor = utf16ToByte(editorText, pos);
      const off = byteInEditor + wrapPrefixBytes;
      // Both facts about the same offset, concurrently. `disposition` is null unless the offset is on a
      // definition name; `typeAt` is null only over dead space.
      const [info, disp] = await Promise.all([
        typeAt(compiled, surface, off),
        disposition(compiled, surface, off),
      ]);
      if (!info && !disp) return null;
      // Anchor to the type range when present (the exact sub-expression), else the definition name.
      const range = info ?? disp!;
      const fromByte = range.from - wrapPrefixBytes;
      const toByte = range.to - wrapPrefixBytes;
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
          // The inferred type (the primary fact).
          if (info) {
            const typeEl = document.createElement("div");
            typeEl.textContent = info.typeName;
            dom.appendChild(typeEl);
          }
          // How the compiler compiled this definition (only when the token names one).
          if (disp) {
            const head = document.createElement("div");
            head.className = "cm-cadenza-hover-disp";
            head.textContent = `${disp.disposition}`;
            const g = dispositionGloss(disp.disposition);
            if (g) {
              const glossEl = document.createElement("div");
              glossEl.className = "cm-cadenza-hover-gloss";
              glossEl.textContent = g;
              head.appendChild(glossEl);
            }
            dom.appendChild(head);
            for (const inst of disp.instances) {
              const line = document.createElement("div");
              line.className = "cm-cadenza-hover-inst";
              line.textContent = `${disp.name}[${inst}]`;
              dom.appendChild(line);
            }
          }
          return { dom };
        },
      };
    },
    { hoverTime: 300, hideOnChange: true },
  );
}
