/// The notebook document model + parser. A notebook is a markdown string; this splits it into an
/// ORDERED list of cells — prose (markdown to render) interleaved with Cadenza CODE cells (a fenced
/// ```cadenza block, optionally carrying a render directive like `chart:line` or `widget`).
///
/// PURE by design — NO compiler/worker/React imports — so it's unit-testable under `node --test`
/// (mirrors `calculator/classify.ts`, which was split out for exactly this reason). The route shell
/// (Increment 2) consumes this model; the reactive engine (Increment 4) walks the widget cells.
///
/// Cell-scope semantics (design D1, ratified sequential/accumulating): the parser only produces the
/// ordered cell list + each cell's raw source; WHICH prior cells a code cell sees is a runtime concern
/// (the run engine concatenates prior code cells' definitions, like the calculator's replEval buffer).
/// Parsing stays a pure structural split.

/// A render directive on a ```cadenza fence info string (the token after `cadenza`). `none` = auto-detect
/// the output renderer from the value's shape (Increment 3). `hidden` runs the cell but shows no source.
export type CellDirective =
  | { kind: "none" }
  | { kind: "table" }
  | { kind: "chart"; chart: "line" | "bar" | "scatter" }
  | { kind: "formula" }
  | { kind: "widget" }
  | { kind: "hidden" };

/// One cell of a parsed notebook, in document order.
export type Cell =
  /// Markdown prose between code fences (or a non-cadenza fenced block, kept verbatim as prose).
  | { kind: "prose"; markdown: string }
  /// A ```cadenza code cell: its Cadenza source (fence lines stripped) + its render directive.
  | { kind: "code"; source: string; directive: CellDirective };

/// The fence markers markdown recognizes. A fence opens with ≥3 of one char; the CLOSING fence must be
/// the SAME char and at least as long (CommonMark). We support both to match reader expectation.
const FENCE_RE = /^(\s*)(`{3,}|~{3,})(.*)$/;

/// Parse a ```cadenza fence's info string into a directive. The info string is everything after the
/// fence chars; the first whitespace token must be `cadenza` for it to be a code cell (checked by the
/// caller). This parses the SECOND token into the render directive. Unknown/absent → `{ kind: "none" }`
/// (forward-compatible: an unrecognized directive renders as a plain value rather than erroring).
export function parseDirective(info: string): CellDirective {
  const tokens = info.trim().split(/\s+/).filter(Boolean);
  // tokens[0] is `cadenza` (the caller already matched it); the directive is tokens[1].
  const d = tokens[1];
  if (!d) return { kind: "none" };
  if (d === "table") return { kind: "table" };
  if (d === "formula") return { kind: "formula" };
  if (d === "widget") return { kind: "widget" };
  if (d === "hidden") return { kind: "hidden" };
  if (d === "chart:line") return { kind: "chart", chart: "line" };
  if (d === "chart:bar") return { kind: "chart", chart: "bar" };
  if (d === "chart:scatter") return { kind: "chart", chart: "scatter" };
  return { kind: "none" };
}

/// Whether a fence info string opens a Cadenza CODE cell — its first token is exactly `cadenza`. A
/// ```js or ```text fence is NOT a code cell; it stays verbatim prose (a notebook can document other
/// languages). Case-insensitive on the language token to be forgiving (Cadenza / CADENZA).
function isCadenzaInfo(info: string): boolean {
  const first = info.trim().split(/\s+/).filter(Boolean)[0];
  return first?.toLowerCase() === "cadenza";
}

/// Parse a whole notebook markdown string into an ordered list of cells.
///
/// The scan walks lines, tracking whether we're inside a fenced block. A fence opens with ≥3 `` ` `` or
/// `~`; it closes on the first line that is a fence of the SAME char and ≥ the opening length with an
/// EMPTY info string (CommonMark's closing-fence rule). A ```cadenza block becomes a `code` cell; any
/// other fenced block (```js, ```) is emitted verbatim (fences included) as prose so a downstream
/// markdown renderer still renders it as a normal code block. Prose runs between fences are coalesced.
///
/// Robustness: an UNCLOSED fence (no matching close before EOF) is still emitted as its cell — a
/// cadenza fence becomes a code cell with whatever source it accumulated, a non-cadenza fence stays
/// prose — so a half-typed document never throws or drops content.
export function parseDocument(markdown: string): Cell[] {
  const lines = markdown.split("\n");
  const cells: Cell[] = [];

  let proseBuf: string[] = [];
  const flushProse = () => {
    // Only emit a prose cell if it has non-whitespace content — leading/trailing blank runs between
    // code cells shouldn't produce empty prose cells (they'd render as blank gaps).
    if (proseBuf.some((l) => l.trim().length > 0)) {
      cells.push({ kind: "prose", markdown: proseBuf.join("\n").replace(/^\n+|\n+$/g, "") });
    }
    proseBuf = [];
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const m = FENCE_RE.exec(line);
    if (m) {
      const fenceChar = m[2][0]; // ` or ~
      const fenceLen = m[2].length;
      const info = m[3];
      const cadenza = isCadenzaInfo(info);

      // Collect the fenced block body until the matching closing fence (same char, ≥ length, empty info).
      const body: string[] = [];
      const rawBlock: string[] = [line]; // the block verbatim (for non-cadenza prose passthrough)
      let j = i + 1;
      let closed = false;
      for (; j < lines.length; j++) {
        const cm = FENCE_RE.exec(lines[j]);
        const isClose =
          cm && cm[2][0] === fenceChar && cm[2].length >= fenceLen && cm[3].trim() === "";
        rawBlock.push(lines[j]);
        if (isClose) {
          closed = true;
          break;
        }
        body.push(lines[j]);
      }

      if (cadenza) {
        // A cadenza code cell: flush pending prose, emit the code cell (fence lines stripped).
        flushProse();
        cells.push({ kind: "code", source: body.join("\n"), directive: parseDirective(info) });
      } else {
        // A non-cadenza fenced block stays verbatim in prose so the markdown renderer shows it as a
        // normal code block. Keep the closing fence only if it was actually present.
        for (const l of rawBlock) proseBuf.push(l);
      }
      // Advance past the block. If it closed, skip the closing fence line too.
      i = closed ? j + 1 : j;
      continue;
    }

    proseBuf.push(line);
    i++;
  }
  flushProse();
  return cells;
}
