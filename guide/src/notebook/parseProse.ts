/// A minimal, dependency-free markdown parser for notebook PROSE cells (Increment 2b prep).
///
/// WHY hand-rolled (no markdown-it / react-markdown): the guide has never needed a markdown-string
/// renderer (chapters are hand-authored TSX calling Prose.tsx components — confirmed by v-guide). Rather
/// than pull a new npm dep into the bundle (weight + supply-chain + a sign-off), we render the COMMON
/// markdown subset a notebook's prose needs, mapped onto the guide's existing Prose look. This mirrors
/// the design's D3 "hand-roll first, add a dep only when the subset can't carry it" stance.
///
/// SUPPORTED (block): ATX headings `#`..`######`, unordered lists (`-`/`*`), ordered lists (`1.`),
/// blockquotes (`>`), and paragraphs. (Code fences never reach here — `parseDocument` already split them
/// into code cells.) SUPPORTED (inline, within a block's text): `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, and `[label](url)` links. NOT supported (documented gaps, extend when a notebook needs
/// them): tables, images, nested lists, reference links, HTML passthrough. A construct we don't parse
/// renders as literal text — never throws.
///
/// PURE (no React) — returns a data model (blocks + inline spans) the prose component maps to Prose.tsx
/// elements. Unit-testable under `node --test`.

/// An inline span within a block's text.
export type Inline =
  | { t: "text"; text: string }
  | { t: "strong"; text: string }
  | { t: "em"; text: string }
  | { t: "code"; text: string }
  | { t: "link"; text: string; href: string };

/// A block-level element.
export type Block =
  | { t: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; spans: Inline[] }
  | { t: "paragraph"; spans: Inline[] }
  | { t: "list"; ordered: boolean; items: Inline[][] }
  | { t: "blockquote"; spans: Inline[] }
  /// A GFM pipe table: a header row + body rows, each cell a list of inline spans.
  | { t: "table"; header: Inline[][]; rows: Inline[][][] };

/// Parse inline markdown within a single logical line/paragraph text into spans. A left-to-right scan;
/// the first matching delimiter wins, so `**a**` is strong and `*a*` is em. Unclosed delimiters render
/// as literal text (the run before the next match, or the tail).
export function parseInline(text: string): Inline[] {
  const spans: Inline[] = [];
  let i = 0;
  let plain = "";
  const flushPlain = () => {
    if (plain) {
      spans.push({ t: "text", text: plain });
      plain = "";
    }
  };
  while (i < text.length) {
    // `code` — highest precedence (its content is literal, no nested inline).
    if (text[i] === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i) {
        flushPlain();
        spans.push({ t: "code", text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    // link [label](href)
    if (text[i] === "[") {
      const close = text.indexOf("]", i + 1);
      if (close > i && text[close + 1] === "(") {
        const paren = text.indexOf(")", close + 2);
        if (paren > close) {
          flushPlain();
          spans.push({ t: "link", text: text.slice(i + 1, close), href: text.slice(close + 2, paren) });
          i = paren + 1;
          continue;
        }
      }
    }
    // **strong**
    if (text.startsWith("**", i)) {
      const end = text.indexOf("**", i + 2);
      if (end > i) {
        flushPlain();
        spans.push({ t: "strong", text: text.slice(i + 2, end) });
        i = end + 2;
        continue;
      }
    }
    // *em* or _em_
    if (text[i] === "*" || text[i] === "_") {
      const d = text[i];
      const end = text.indexOf(d, i + 1);
      if (end > i && end !== i + 1) {
        flushPlain();
        spans.push({ t: "em", text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    plain += text[i];
    i++;
  }
  flushPlain();
  return spans;
}

const HEADING_RE = /^(#{1,6})\s+(.*)$/;
const UL_RE = /^[-*]\s+(.*)$/;
const OL_RE = /^\d+\.\s+(.*)$/;
const QUOTE_RE = /^>\s?(.*)$/;
/// A GFM table delimiter row: only `|`, `-`, `:`, and spaces, with at least one `-` (e.g. `|---|:--:|`).
const TABLE_DELIM_RE = /^\|?[\s|:-]*-[\s|:-]*\|?$/;

/// Split a GFM pipe-table row into trimmed cell strings, dropping the optional leading/trailing `|`.
/// (A literal `\|` escape isn't supported — a documented limitation; the common case is plain cells.)
function tableCells(row: string): string[] {
  let s = row.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}

/// Parse a prose cell's markdown into a list of blocks. Blank lines separate blocks; consecutive list
/// items coalesce into one list; consecutive non-special lines coalesce into a paragraph.
export function parseProse(markdown: string): Block[] {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) {
      blocks.push({ t: "paragraph", spans: parseInline(para.join(" ").trim()) });
      para = [];
    }
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (trimmed === "") {
      flushPara();
      i++;
      continue;
    }
    const h = HEADING_RE.exec(trimmed);
    if (h) {
      flushPara();
      blocks.push({ t: "heading", level: h[1].length as 1 | 2 | 3 | 4 | 5 | 6, spans: parseInline(h[2].trim()) });
      i++;
      continue;
    }
    // A GFM pipe table: a header row (contains `|`) immediately followed by a delimiter row (`|---|`),
    // then zero+ body rows (each a `|`-row). The delimiter line is what disambiguates a table from a
    // paragraph that happens to contain a pipe.
    const nextTrimmed = i + 1 < lines.length ? lines[i + 1].trim() : "";
    if (trimmed.includes("|") && TABLE_DELIM_RE.test(nextTrimmed) && nextTrimmed.includes("-")) {
      flushPara();
      const header = tableCells(trimmed).map(parseInline);
      i += 2; // consume header + delimiter
      const rows: Inline[][][] = [];
      while (i < lines.length && lines[i].trim().includes("|") && lines[i].trim() !== "") {
        rows.push(tableCells(lines[i].trim()).map(parseInline));
        i++;
      }
      blocks.push({ t: "table", header, rows });
      continue;
    }
    // A run of list items (all same ordered-ness) → one list block.
    const ulm = UL_RE.exec(trimmed);
    const olm = OL_RE.exec(trimmed);
    if (ulm || olm) {
      flushPara();
      const ordered = olm !== null;
      const items: Inline[][] = [];
      while (i < lines.length) {
        const t = lines[i].trim();
        const m = ordered ? OL_RE.exec(t) : UL_RE.exec(t);
        if (!m) break;
        items.push(parseInline(m[1].trim()));
        i++;
      }
      blocks.push({ t: "list", ordered, items });
      continue;
    }
    const q = QUOTE_RE.exec(trimmed);
    if (q) {
      flushPara();
      // Coalesce consecutive `>` lines into one blockquote.
      const quoteLines: string[] = [];
      while (i < lines.length) {
        const qm = QUOTE_RE.exec(lines[i].trim());
        if (!qm) break;
        quoteLines.push(qm[1]);
        i++;
      }
      blocks.push({ t: "blockquote", spans: parseInline(quoteLines.join(" ").trim()) });
      continue;
    }
    para.push(trimmed);
    i++;
  }
  flushPara();
  return blocks;
}
