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

// TYPE-ONLY import (erased at compile time — keeps this module worker/runtime-free per the purity note
// above): `Surface` is a bare string-union, so importing its type pulls in no worker code.
import type { Surface } from "../compiler/worker.ts";

/// Wrap a cell's source into ONE top-level form the single-form `renderSyntax` accepts. A notebook code
/// cell may hold several top-level `def`s (a helper + `main`); s-expr has no bare multi-form top level, so
/// several forms are gathered under a `(do …)`. ML's top level IS natively multi-form, so it passes through.
///
/// ⚠ We do NOT reuse `wrapModule.gatherTestForms`/`ungatherTestForms`: their un-gather is `stripModule`,
/// which also strips a `(def (main) …)` (it assumes any top-level `main` is a wrapper-SYNTHESIZED entry to
/// peel). Notebook cells legitimately DEFINE `main`, so that would DELETE the cell's `main` def on the
/// ML→s-expr leg (→ CDZ0101 "unbound name main" — co-verified by v-guide-infra). These helpers peel ONLY
/// the `(do …)` we added, never a real `def`.
function gatherForRender(src: string, surface: Surface): string {
  return surface === "sexpr" ? `(do ${src.trim()})` : src.trim();
}
/// Inverse of `gatherForRender` over a RENDERED cell: peel exactly the ONE `(do …)` wrapper back off an
/// s-expr render (leaving every `def` intact); an ML render is already native multi-form. The head is
/// `(do` + ANY whitespace — the s-expr pretty-printer emits `(do\n  …)` for a multi-LINE body, so matching
/// only `(do ` (a space) would leave a large multi-form cell wrapped (its defs then aren't top-level →
/// downstream cells + `main` go unbound — the loan/projectile toggle break).
function ungatherAfterRender(rendered: string, to: Surface): string {
  const t = rendered.trim();
  if (to === "sexpr" && /^\(do\s/.test(t) && t.endsWith(")")) return t.slice(3, -1).trim();
  return t;
}

/// A render directive on a ```cadenza fence info string (the token after `cadenza`). `none` = auto-detect
/// the output renderer from the value's shape (Increment 3). `hidden` runs the cell but shows no source.
export type CellDirective =
  | { kind: "none" }
  | { kind: "table" }
  | { kind: "chart"; chart: "line" | "bar" | "scatter" }
  | { kind: "formula" }
  | { kind: "widget" }
  | { kind: "hidden" };

/// One cell of a parsed notebook, in document order. `id` is an OPTIONAL stable identity for a stacked
/// per-cell UI (React keys that survive edits/reorder — P0 #13): `parseDocument` does NOT set it (it stays
/// a pure structural split), a separate `assignIds` pass stamps it, and `setCellSource` preserves it. It's
/// optional so a manually-constructed cell (tests, programmatic edits) needn't carry one.
export type Cell =
  /// Markdown prose between code fences (or a non-cadenza fenced block, kept verbatim as prose).
  | { kind: "prose"; markdown: string; id?: number }
  /// A ```cadenza code cell: its Cadenza source (fence lines stripped) + its render directive.
  | { kind: "code"; source: string; directive: CellDirective; id?: number };

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
  // A bare `chart` (no `:kind`) defaults to a line chart — the most common shape — so a reader who writes
  // ` ```cadenza chart ` gets a chart, not a silently-degraded plain value (a `none` directive renders the
  // value with no plot). An explicit `chart:line|bar|scatter` selects the kind; an UNKNOWN kind
  // (`chart:zorp`) still falls through to `none` (forward-compatible: don't guess a bogus kind).
  if (d === "chart" || d === "chart:line") return { kind: "chart", chart: "line" };
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

/// A line-range map of a notebook document, for the LSP (operator P0 #13): the editor/LSP must run
/// Cadenza diagnostics ONLY on CODE cells, not on prose or the notebook's widget-DSL cells. Each entry
/// gives, per v-lsp's contract:
///   - `startLine`/`endLine`: 0-based, HALF-OPEN `[startLine, endLine)`. For a code cell these bound the
///     Cadenza SOURCE only — the ` ```cadenza ` opener and the closing ` ``` ` fence lines are OUTSIDE the
///     range (feeding the backticks to the parser would inject a parse error).
///   - `kind`: `"code"` | `"prose"`.
///   - `directive`: the code cell's render directive (`undefined` for prose). The LSP checks a cell only
///     when `kind === "code" && directive.kind !== "widget"` — a widget cell is the notebook DSL, not
///     Cadenza, so it (and prose) get NO Cadenza diagnostics.
///   - `surface`: `"ml"` | `"sexpr"` — which reader the LSP uses for this cell (notebook code cells are a
///     WHOLE program / def-block, not a bare fragment, so no synthetic module wrap is needed).
export interface CellRange {
  startLine: number;
  endLine: number;
  kind: "code" | "prose";
  directive?: CellDirective;
  surface: "ml" | "sexpr";
}

/// Compute the line-range map of a notebook (see `CellRange`). `surface` is the document's editing surface
/// (the notebook is surface-pinned — s-expr today), carried per-cell so the LSP picks the right reader.
/// A prose range spans its lines inclusively-as-half-open; a code range covers only the fenced SOURCE
/// (fence lines excluded). An unclosed cadenza fence still yields a code range to its last line (matches
/// `parseDocument`'s robustness — a half-typed doc never drops content).
export function cellRanges(markdown: string, surface: "ml" | "sexpr" = "sexpr"): CellRange[] {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
  const ranges: CellRange[] = [];
  // Accumulate a prose run [proseStart, i) and flush it only if it has non-blank content (mirrors
  // parseDocument's flushProse: blank-only runs don't become cells).
  let proseStart = 0;
  const flushProse = (end: number) => {
    let hasContent = false;
    for (let k = proseStart; k < end; k++) if (lines[k].trim().length > 0) { hasContent = true; break; }
    if (hasContent) ranges.push({ startLine: proseStart, endLine: end, kind: "prose", surface });
  };

  let i = 0;
  while (i < lines.length) {
    const m = FENCE_RE.exec(lines[i]);
    if (m) {
      const fenceChar = m[2][0];
      const fenceLen = m[2].length;
      const info = m[3];
      const cadenza = isCadenzaInfo(info);
      // Find the matching closing fence.
      let j = i + 1;
      let closed = false;
      for (; j < lines.length; j++) {
        const cm = FENCE_RE.exec(lines[j]);
        if (cm && cm[2][0] === fenceChar && cm[2].length >= fenceLen && cm[3].trim() === "") {
          closed = true;
          break;
        }
      }
      if (cadenza) {
        flushProse(i); // close any pending prose BEFORE the opening fence line
        // Source is the lines strictly between the opening fence (i) and the closing fence (j) — fences
        // excluded per the contract. An unclosed fence runs to EOF (j === lines.length).
        ranges.push({ startLine: i + 1, endLine: j, kind: "code", directive: parseDirective(info), surface });
        proseStart = closed ? j + 1 : j;
      }
      // A non-cadenza fence stays part of the surrounding prose run — don't flush, don't reset proseStart.
      i = closed ? j + 1 : j;
      continue;
    }
    i++;
  }
  flushProse(lines.length);
  return ranges;
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
  // Normalize line endings up front so the parser is line-ending agnostic: a Windows `\r\n` (or a lone
  // classic-Mac `\r`) document would otherwise leave a trailing `\r` on every split line, which ends up
  // INSIDE a code cell's source and breaks its downstream Cadenza compile/render (reported on PR #471).
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");
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

/// The fence info-string token for a code cell's directive — the inverse of `parseDirective`, for
/// `serializeDocument`. `none` has NO token (a bare ` ```cadenza `); every other directive round-trips to
/// its keyword (`table`, `formula`, `widget`, `hidden`, `chart:line`/`bar`/`scatter`).
function directiveToInfo(d: CellDirective): string {
  switch (d.kind) {
    case "none": return "cadenza";
    case "chart": return `cadenza chart:${d.chart}`;
    default: return `cadenza ${d.kind}`;
  }
}

/// Serialize a cell list back to a notebook markdown string — the inverse of `parseDocument`. A prose cell
/// is emitted verbatim; a code cell becomes a ` ```cadenza<directive> ` fence + its source + a closing
/// ` ``` `. Cells are joined with a blank line so they re-split cleanly. `parseDocument(serializeDocument(
/// cells))` round-trips a parsed document (modulo the blank-run normalization `parseDocument` already does).
/// This is the doc-side of per-cell editing (P0 #13): a per-cell editor edits one cell's `source`
/// (`setCellSource`), then the notebook re-serializes to markdown for storage / the run path.
export function serializeDocument(cells: Cell[]): string {
  return cells
    .map((cell) =>
      cell.kind === "prose"
        ? cell.markdown
        : `\`\`\`${directiveToInfo(cell.directive)}\n${cell.source}\n\`\`\``,
    )
    .join("\n\n");
}

/// Re-render a notebook's CADENZA CODE cells from surface `from` to surface `to`, returning the new
/// markdown (the operator's "notebook always uses s-expr" fix — my half of the surface toggle). Notebook
/// examples are authored in ONE surface (s-expr); to let a reader EDIT in the global surface, each code
/// cell's source is rendered THROUGH the selected surface for display (the /cad + playground `renderSnippet`
/// pattern), while the RUN/lint paths stay s-expr-canonical (the caller keeps those fixed). `render` is
/// INJECTED (the compiler client's `renderSyntax`) so this module stays pure/worker-free + node-testable.
///
/// ⚠ WIDGET cells are NOT Cadenza (the notebook's `name : T = control(...)` DSL) — rendering one through the
/// Cadenza surface converter FAILS ("s-expr parse error"), so they pass through UNCHANGED, as does prose. A
/// cell whose render REJECTS (a transient parse error mid-edit) keeps its original source rather than
/// dropping content. Cell `id`s are preserved (stable React keys). Async: renders run concurrently.
export async function renderDocToSurface(
  markdown: string,
  from: Surface,
  to: Surface,
  render: (text: string, from: Surface, to: Surface) => Promise<string>,
): Promise<string> {
  const cells = parseDocument(markdown);
  if (from === to) return serializeDocument(cells); // no-op transform (normalizes, doesn't re-render)
  const rendered = await Promise.all(
    cells.map(async (cell) => {
      // Only real Cadenza code cells convert; prose + the widget DSL pass through verbatim.
      if (cell.kind !== "code" || cell.directive.kind === "widget") return cell;
      try {
        // GATHER multi-form cells into one top-level form (a cell may hold a helper `def` + `main`), render
        // that single form, then UNGATHER — else `renderSyntax` rejects a multi-form cell ("trailing input").
        const rendered = await render(gatherForRender(cell.source, from), from, to);
        return { ...cell, source: ungatherAfterRender(rendered, to) };
      } catch {
        return cell; // a transient/invalid render keeps the original source (never drop content)
      }
    }),
  );
  return serializeDocument(rendered);
}

/// Return a NEW cell list with the CODE cell at `index` given `newSource` (immutable — a fresh array +
/// a fresh cell object, so React state updates cleanly and the other cells keep their identity). Throws on
/// an out-of-range index or a non-code cell (a prose cell isn't edited through this path), mirroring
/// `assembleCell`'s guards. The directive is preserved — only the source changes.
export function setCellSource(cells: Cell[], index: number, newSource: string): Cell[] {
  const cell = cells[index];
  if (!cell) throw new RangeError(`setCellSource: no cell at index ${index}`);
  if (cell.kind !== "code") throw new TypeError(`setCellSource: cell ${index} is prose, not code`);
  const next = cells.slice();
  // Preserve the cell's stable `id` (React key) across the edit — only the source changes. Spread the old
  // cell so an id-LESS cell stays id-less (no injected `id: undefined`) and an id-bearing one keeps its id.
  next[index] = { ...cell, kind: "code", source: newSource, directive: cell.directive };
  return next;
}

/// Rewrite the markdown of the PROSE cell at `index` (the prose counterpart of `setCellSource` — operator
/// UX: editing a notebook's prose, not just its code cells). Preserves the cell's stable `id` (React key)
/// so an in-place prose editor keeps focus/cursor across the edit. Throws if `index` is a code cell (the
/// caller edits code via `setCellSource`) — the two kinds carry different fields (`markdown` vs `source`),
/// so a caller must pick the right setter. Round-trips through `serializeDocument` (which emits a prose
/// cell's `markdown` verbatim), so re-parsing the serialized doc yields the edited prose.
export function setProseSource(cells: Cell[], index: number, newMarkdown: string): Cell[] {
  const cell = cells[index];
  if (!cell) throw new RangeError(`setProseSource: no cell at index ${index}`);
  if (cell.kind !== "prose") throw new TypeError(`setProseSource: cell ${index} is code, not prose`);
  const next = cells.slice();
  next[index] = { ...cell, kind: "prose", markdown: newMarkdown };
  return next;
}

/// Stamp each cell with a stable monotonic `id` (document order: 0, 1, 2, …), returning a NEW cell array.
/// A stacked per-cell UI keys its editor list by `id` so a cell keeps identity across edits (an edit via
/// `setCellSource` preserves the id) — index keys would remount/lose focus on insert/reorder/delete (P0
/// #13). `parseDocument` deliberately does NOT assign ids (it stays a pure structural split); the UI calls
/// `assignIds(parseDocument(md))` once. Idempotent in effect (re-stamps 0..n-1); a caller that inserts a
/// cell should give the new cell a fresh id beyond the current max, not re-run `assignIds` (that would
/// renumber existing cells and break their keys).
export function assignIds(cells: Cell[]): Cell[] {
  return cells.map((cell, i) => ({ ...cell, id: i }));
}

/// The next fresh `id` for a cell inserted into `cells`: one past the current MAX id (never a renumber —
/// see `assignIds`, which warns that re-stamping on insert would break existing React keys). Returns 0 for
/// an id-less list (no ids in play yet). Used by `insertCell` so a newly-added cell gets a stable key that
/// can't collide with an existing one, while every other cell keeps its identity.
function nextId(cells: Cell[]): number {
  let max = -1;
  for (const c of cells) if (c.id !== undefined && c.id > max) max = c.id;
  return max + 1;
}

/// Insert `newCell` at position `index` (0..length — `length` appends), returning a NEW cell list (immutable:
/// a fresh array, existing cells keep their identity/ids). The add half of markdown-structure editing
/// (operator #2 — "how do I add a new section?"): the UI's "+ insert below cell k" calls `insertCell(cells,
/// k + 1, blank)`. If the list carries `id`s (a live UI keys by them), the new cell is stamped a FRESH id
/// past the current max (never renumbering the others — stable React keys, per `assignIds`); an id-LESS list
/// stays id-less (the new cell inherits whatever id it came with, typically none). Throws on an index outside
/// [0, length] (an unreachable insert slot is a caller bug, like `setCellSource`'s range guard).
export function insertCell(cells: Cell[], index: number, newCell: Cell): Cell[] {
  if (index < 0 || index > cells.length) throw new RangeError(`insertCell: index ${index} out of range [0, ${cells.length}]`);
  // Stamp a fresh id ONLY when the list is id-bearing and the incoming cell lacks one — so a live keyed UI
  // gets a collision-free key, but an id-less doc model (fresh parseDocument output) stays id-less.
  const idBearing = cells.some((c) => c.id !== undefined);
  const stamped = idBearing && newCell.id === undefined ? { ...newCell, id: nextId(cells) } : newCell;
  const next = cells.slice();
  next.splice(index, 0, stamped);
  return next;
}

/// Remove the cell at `index`, returning a NEW cell list (immutable; the survivors keep their identity/ids —
/// so a keyed UI only unmounts the deleted editor). The delete half of markdown-structure editing (operator
/// #2 — "how do I remove one?"): the per-cell trash control calls `removeCell(cells, k)`. Throws on an
/// out-of-range index (like `setCellSource`). Ids are NOT renumbered — deleting cell 1 of [0,1,2] leaves
/// ids [0,2], so no surviving editor remounts (a renumber would shift every key after the hole).
export function removeCell(cells: Cell[], index: number): Cell[] {
  if (index < 0 || index >= cells.length) throw new RangeError(`removeCell: index ${index} out of range [0, ${cells.length})`);
  const next = cells.slice();
  next.splice(index, 1);
  return next;
}

/// Move the cell at `from` to position `to`, returning a NEW cell list (immutable; every cell keeps its
/// identity/id — a reorder only changes ORDER, so a keyed UI animates rather than remounts). The reorder
/// half of markdown-structure editing (operator #2 — "how do I reorder them?"): drag-to-reorder and the
/// up/down fallback both land here — up = `moveCell(cells, k, k - 1)`, down = `moveCell(cells, k, k + 1)`.
/// `to` is the target index in the ORIGINAL coordinate space (the cell ends up at `to` after removal +
/// reinsert). A no-op `from === to` returns a fresh copy unchanged. Throws if either index is out of range.
export function moveCell(cells: Cell[], from: number, to: number): Cell[] {
  if (from < 0 || from >= cells.length) throw new RangeError(`moveCell: from ${from} out of range [0, ${cells.length})`);
  if (to < 0 || to >= cells.length) throw new RangeError(`moveCell: to ${to} out of range [0, ${cells.length})`);
  const next = cells.slice();
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}
