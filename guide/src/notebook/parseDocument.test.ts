/// Unit tests for the notebook document/cell parser (Increment 1). Pins the structural split of a
/// markdown notebook into ordered prose + Cadenza code cells, the fence-directive parse, and the
/// robustness corners (unclosed fence, non-cadenza fence passthrough, ~~~ fences, blank-prose elision).
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseDocument, parseDirective, cellRanges, serializeDocument, setCellSource, setProseSource, renderDocToSurface, assignIds, type Cell } from "./parseDocument.ts";

test("a plain prose-only document is one prose cell", () => {
  const cells = parseDocument("# Title\n\nsome **markdown** prose.");
  assert.deepEqual(cells, [{ kind: "prose", markdown: "# Title\n\nsome **markdown** prose." }]);
});

test("prose then a cadenza code cell then prose splits into 3 ordered cells", () => {
  const doc = ["intro prose", "```cadenza", "def (main) = 1 + 2", "```", "outro prose"].join("\n");
  const cells = parseDocument(doc);
  assert.equal(cells.length, 3);
  assert.deepEqual(cells[0], { kind: "prose", markdown: "intro prose" });
  assert.deepEqual(cells[1], { kind: "code", source: "def (main) = 1 + 2", directive: { kind: "none" } });
  assert.deepEqual(cells[2], { kind: "prose", markdown: "outro prose" });
});

test("a multi-line code cell keeps its body verbatim, fence lines stripped", () => {
  const doc = ["```cadenza", "def (a) = 1", "def (main) = a + a", "```"].join("\n");
  const cells = parseDocument(doc);
  assert.deepEqual(cells, [
    { kind: "code", source: "def (a) = 1\ndef (main) = a + a", directive: { kind: "none" } },
  ]);
});

test("fence directives parse: table, chart:line/bar/scatter, formula, widget, hidden", () => {
  const dir = (info: string) => parseDirective(info);
  assert.deepEqual(dir("cadenza table"), { kind: "table" });
  assert.deepEqual(dir("cadenza chart:line"), { kind: "chart", chart: "line" });
  assert.deepEqual(dir("cadenza chart:bar"), { kind: "chart", chart: "bar" });
  assert.deepEqual(dir("cadenza chart:scatter"), { kind: "chart", chart: "scatter" });
  assert.deepEqual(dir("cadenza formula"), { kind: "formula" });
  assert.deepEqual(dir("cadenza widget"), { kind: "widget" });
  assert.deepEqual(dir("cadenza hidden"), { kind: "hidden" });
});

test("a bare `chart` directive defaults to a line chart (not a silently-degraded plain value)", () => {
  // A reader who writes ` ```cadenza chart ` wants a chart. Without a `:kind` we default to `line` (the
  // most common shape) so the cell plots rather than falling through to `none` (a plain value with no
  // plot + no signal). An UNKNOWN kind still stays `none` — we default a MISSING kind, not a bogus one.
  assert.deepEqual(parseDirective("cadenza chart"), { kind: "chart", chart: "line" });
  assert.deepEqual(parseDirective("cadenza chart:zorp"), { kind: "none" }); // bogus kind stays none
});

test("an absent or unknown directive is `none` (forward-compatible, never throws)", () => {
  assert.deepEqual(parseDirective("cadenza"), { kind: "none" });
  assert.deepEqual(parseDirective("cadenza wibble"), { kind: "none" });
  assert.deepEqual(parseDirective("cadenza chart:pie"), { kind: "none" }); // unknown chart kind
});

test("a directive rides through a real code cell's fence", () => {
  const doc = ["```cadenza chart:line", "def (main) = points", "```"].join("\n");
  const cells = parseDocument(doc);
  assert.deepEqual(cells[0], {
    kind: "code",
    source: "def (main) = points",
    directive: { kind: "chart", chart: "line" },
  });
});

test("a NON-cadenza fenced block stays verbatim in prose (a notebook can show other languages)", () => {
  const doc = ["look:", "```js", "const x = 1;", "```", "done"].join("\n");
  const cells = parseDocument(doc);
  // The js block is NOT a code cell — it's part of the prose (fences included) so a markdown renderer
  // shows it as a normal fenced block.
  assert.equal(cells.length, 1);
  assert.equal(cells[0].kind, "prose");
  assert.match((cells[0] as Extract<Cell, { kind: "prose" }>).markdown, /```js\nconst x = 1;\n```/);
});

test("~~~ fences work too, and mixed ~~~ / ``` don't cross-close", () => {
  const doc = ["~~~cadenza", "def (main) = 42", "~~~"].join("\n");
  assert.deepEqual(parseDocument(doc), [
    { kind: "code", source: "def (main) = 42", directive: { kind: "none" } },
  ]);
  // A ``` inside a ~~~ cadenza block does NOT close it (different fence char) — it's part of the source.
  const doc2 = ["~~~cadenza", "def (main) = 1", "```", "~~~"].join("\n");
  assert.deepEqual(parseDocument(doc2), [
    { kind: "code", source: 'def (main) = 1\n```', directive: { kind: "none" } },
  ]);
});

test("an UNCLOSED cadenza fence still yields a code cell with the accumulated source", () => {
  const doc = ["intro", "```cadenza", "def (main) = 1"].join("\n"); // no closing fence
  const cells = parseDocument(doc);
  assert.equal(cells.length, 2);
  assert.deepEqual(cells[0], { kind: "prose", markdown: "intro" });
  assert.deepEqual(cells[1], { kind: "code", source: "def (main) = 1", directive: { kind: "none" } });
});

test("blank runs between cells don't produce empty prose cells", () => {
  const doc = ["```cadenza", "def (main) = 1", "```", "", "   ", "", "```cadenza", "def (main) = 2", "```"].join("\n");
  const cells = parseDocument(doc);
  // Two code cells, no empty prose cell for the blank gap between them.
  assert.equal(cells.length, 2);
  assert.equal(cells[0].kind, "code");
  assert.equal(cells[1].kind, "code");
});

test("a longer closing fence closes a shorter opener; a shorter one does not (CommonMark)", () => {
  // Open ```, close ```` (longer, same char) → closes.
  const doc = ["```cadenza", "def (main) = 1", "````"].join("\n");
  assert.deepEqual(parseDocument(doc), [
    { kind: "code", source: "def (main) = 1", directive: { kind: "none" } },
  ]);
  // Open ````, a ``` (shorter) does NOT close it — stays in the source.
  const doc2 = ["````cadenza", "def (main) = 1", "```", "````"].join("\n");
  assert.deepEqual(parseDocument(doc2), [
    { kind: "code", source: "def (main) = 1\n```", directive: { kind: "none" } },
  ]);
});

test("indented fences are recognized (leading whitespace before the fence chars)", () => {
  const doc = ["  ```cadenza", "  def (main) = 1", "  ```"].join("\n");
  const cells = parseDocument(doc);
  assert.equal(cells.length, 1);
  assert.equal(cells[0].kind, "code");
});

test("CRLF (and lone CR) line endings are normalized — no stray \\r in code-cell source (PR #471)", () => {
  // A Windows \r\n document must not leave a trailing \r on the code cell's source — that \r would ride
  // into the downstream Cadenza compile and break it.
  const crlf = ["intro", "```cadenza", "def main() = 1 + 2", "```", "outro"].join("\r\n");
  const cells = parseDocument(crlf);
  assert.equal(cells.length, 3);
  assert.deepEqual(cells[1], { kind: "code", source: "def main() = 1 + 2", directive: { kind: "none" } });
  // The code source has NO carriage returns anywhere.
  assert.equal((cells[1] as Extract<Cell, { kind: "code" }>).source.includes("\r"), false);
  // A multi-line code cell under CRLF keeps clean \n joins, no \r.
  const multi = ["```cadenza", "def a = 1", "def main() = a + a", "```"].join("\r\n");
  assert.deepEqual(parseDocument(multi), [
    { kind: "code", source: "def a = 1\ndef main() = a + a", directive: { kind: "none" } },
  ]);
  // Lone classic-Mac \r is normalized too.
  const cr = "```cadenza\rdef main() = 7\r```";
  assert.deepEqual(parseDocument(cr), [
    { kind: "code", source: "def main() = 7", directive: { kind: "none" } },
  ]);
});

// ── cellRanges: the line-range map for the cell-aware LSP (operator P0 #13, v-lsp contract) ──

test("cellRanges maps code cells to half-open [start,end) line ranges EXCLUDING the fence lines", () => {
  const md = ["intro", "```cadenza", "def main() = 1 + 2", "```", "outro"].join("\n");
  //          0        1             2                     3       4
  const r = cellRanges(md, "ml");
  assert.deepEqual(r, [
    { startLine: 0, endLine: 1, kind: "prose", surface: "ml" }, // "intro"
    { startLine: 2, endLine: 3, kind: "code", directive: { kind: "none" }, surface: "ml" }, // source only, no fences
    { startLine: 4, endLine: 5, kind: "prose", surface: "ml" }, // "outro"
  ]);
  // The code range covers exactly the source line, not the ``` fences.
  const lines = md.split("\n");
  assert.deepEqual(lines.slice(2, 3), ["def main() = 1 + 2"]);
});

test("cellRanges tags widget + directive so the LSP can exclude widget/prose from Cadenza diagnostics", () => {
  const md = ["# h", "```cadenza widget", "x : Int64 = slider(0, 10)", "```", "```cadenza table", "(def (main) (list))", "```"].join("\n");
  const r = cellRanges(md, "sexpr");
  const code = r.filter((c) => c.kind === "code");
  assert.equal(code.length, 2);
  assert.deepEqual(code[0].directive, { kind: "widget" });
  assert.deepEqual(code[1].directive, { kind: "table" });
  // The LSP's rule: check only code cells that are NOT widgets.
  const cadenzaCells = r.filter((c) => c.kind === "code" && c.directive?.kind !== "widget");
  assert.equal(cadenzaCells.length, 1);
  assert.deepEqual(cadenzaCells[0].directive, { kind: "table" });
});

test("cellRanges: an unclosed cadenza fence still yields a code range to EOF (robustness)", () => {
  const md = ["```cadenza", "def main() = 1"].join("\n"); // no closing fence
  const r = cellRanges(md, "ml");
  assert.deepEqual(r, [{ startLine: 1, endLine: 2, kind: "code", directive: { kind: "none" }, surface: "ml" }]);
});

test("cellRanges: a non-cadenza fence stays inside prose (not a code range)", () => {
  const md = ["before", "```js", "let x = 1;", "```", "after"].join("\n");
  const r = cellRanges(md, "sexpr");
  // The whole thing is one prose run (the ```js block is prose passthrough) — no code range.
  assert.deepEqual(r, [{ startLine: 0, endLine: 5, kind: "prose", surface: "sexpr" }]);
});

test("cellRanges surface defaults to sexpr (the notebook's pinned surface) and rides onto every cell", () => {
  const md = ["```cadenza", "(def (main) 1)", "```"].join("\n");
  assert.equal(cellRanges(md)[0].surface, "sexpr");
  assert.equal(cellRanges(md, "ml")[0].surface, "ml");
});

// ── serializeDocument / setCellSource: the per-cell-edit round trip (P0 #13, per-cell editors) ──

test("serializeDocument is the inverse of parseDocument (round-trips every directive)", () => {
  const md = [
    "# Title",
    "",
    "intro prose",
    "",
    "```cadenza",
    "(def (main) 1)",
    "```",
    "",
    "```cadenza table",
    "(def (main) (list))",
    "```",
    "",
    "```cadenza chart:bar",
    "(def (main) (list))",
    "```",
    "",
    "```cadenza widget",
    "x : Int64 = slider(0, 10)",
    "```",
  ].join("\n");
  const cells = parseDocument(md);
  // parseDocument∘serializeDocument∘parseDocument is stable (the fixpoint parseDocument already normalizes to).
  assert.deepEqual(parseDocument(serializeDocument(cells)), cells);
});

test("serializeDocument preserves each directive's fence token (none has no token)", () => {
  const cells: Cell[] = [
    { kind: "code", source: "(def (main) 1)", directive: { kind: "none" } },
    { kind: "code", source: "(def (main) 2)", directive: { kind: "chart", chart: "scatter" } },
    { kind: "code", source: "(def (main) 3)", directive: { kind: "hidden" } },
  ];
  const md = serializeDocument(cells);
  assert.match(md, /```cadenza\n\(def \(main\) 1\)/); // none → bare `cadenza`
  assert.match(md, /```cadenza chart:scatter\n/);
  assert.match(md, /```cadenza hidden\n/);
  assert.deepEqual(parseDocument(md), cells);
});

test("setCellSource replaces one code cell's source immutably, preserving the directive + other cells", () => {
  const cells = parseDocument("```cadenza table\n(def (main) (list))\n```\n\nprose\n\n```cadenza\n(def (main) 1)\n```");
  const next = setCellSource(cells, 0, "(def (main) (list 1 2))");
  // A fresh array + fresh cell; the edited cell keeps its `table` directive.
  assert.notEqual(next, cells);
  assert.notEqual(next[0], cells[0]);
  assert.deepEqual(next[0], { kind: "code", source: "(def (main) (list 1 2))", directive: { kind: "table" } });
  // Other cells are untouched (same object identity).
  assert.equal(next[1], cells[1]);
  assert.equal(next[2], cells[2]);
  // The original array is unmodified.
  assert.equal((cells[0] as Extract<Cell, { kind: "code" }>).source, "(def (main) (list))");
});

test("setCellSource throws on a bad index or a prose cell (like assembleCell)", () => {
  const cells = parseDocument("prose\n\n```cadenza\n(def (main) 1)\n```");
  assert.throws(() => setCellSource(cells, 99, "x"), RangeError);
  assert.throws(() => setCellSource(cells, 0, "x"), TypeError); // cell 0 is prose
});

test("an edit then re-serialize survives a full parse round trip (the live edit flow)", () => {
  const cells = parseDocument("```cadenza\n(def (main) 1)\n```");
  const edited = setCellSource(cells, 0, "(def (main) 42)");
  const md = serializeDocument(edited);
  assert.deepEqual(parseDocument(md), edited);
  assert.match(md, /\(def \(main\) 42\)/);
});

// ── assignIds / id preservation: stable React keys for a stacked per-cell UI (P0 #13) ──

test("assignIds stamps a stable monotonic id in document order (parseDocument leaves cells id-less)", () => {
  const cells = parseDocument("prose\n\n```cadenza\n(def (main) 1)\n```\n\nmore\n\n```cadenza table\n(def (main) (list))\n```");
  // parseDocument itself assigns NO ids (pure structural split).
  assert.equal(cells.every((c) => c.id === undefined), true);
  const withIds = assignIds(cells);
  assert.deepEqual(withIds.map((c) => c.id), [0, 1, 2, 3]);
  // A fresh array (immutable) — the originals are untouched.
  assert.notEqual(withIds, cells);
  assert.equal(cells[0].id, undefined);
  // The cell content is otherwise preserved.
  assert.equal(withIds[1].kind, "code");
  assert.equal((withIds[1] as Extract<Cell, { kind: "code" }>).source, "(def (main) 1)");
});

test("setCellSource preserves a cell's id across an edit (the React key survives)", () => {
  const cells = assignIds(parseDocument("```cadenza\n(def (main) 1)\n```\n\nprose"));
  assert.equal(cells[0].id, 0);
  const next = setCellSource(cells, 0, "(def (main) 42)");
  assert.equal(next[0].id, 0); // same id → the per-cell editor keeps focus/state
  assert.equal((next[0] as Extract<Cell, { kind: "code" }>).source, "(def (main) 42)");
  assert.equal(next[1].id, cells[1].id); // untouched cell keeps its id
});

test("serializeDocument ignores id (id is a UI concern, not doc content) — round trip still holds", () => {
  const cells = assignIds(parseDocument("```cadenza\n(def (main) 1)\n```"));
  const md = serializeDocument(cells);
  // Re-parsing yields id-less cells (parseDocument doesn't assign); assignIds re-stamps identically.
  assert.deepEqual(assignIds(parseDocument(md)), cells);
});

// ── setProseSource: the PROSE-cell edit (operator UX #3 — editing a notebook's prose, not just code) ──
test("setProseSource replaces one prose cell's markdown immutably, preserving other cells + ids", () => {
  const cells = assignIds(parseDocument("intro prose\n\n```cadenza\n(def (main) 1)\n```"));
  assert.equal(cells[0].kind, "prose");
  const next = setProseSource(cells, 0, "# New heading\n\nedited prose");
  assert.equal((next[0] as Extract<Cell, { kind: "prose" }>).markdown, "# New heading\n\nedited prose");
  assert.equal(next[0].id, cells[0].id); // stable id → the in-place prose editor keeps focus
  assert.equal(next[1], cells[1]); // the code cell is untouched (same reference)
  assert.notEqual(next, cells); // immutable — a new array
});

test("setProseSource throws on a bad index or a code cell (the code counterpart is setCellSource)", () => {
  const cells = parseDocument("prose\n\n```cadenza\n(def (main) 1)\n```");
  assert.throws(() => setProseSource(cells, 99, "x"), RangeError);
  assert.throws(() => setProseSource(cells, 1, "x"), TypeError); // cell 1 is code, not prose
});

test("a prose edit round-trips through serializeDocument (re-parse yields the edited prose)", () => {
  const cells = parseDocument("old prose\n\n```cadenza\n(def (main) 1)\n```");
  const edited = setProseSource(cells, 0, "new prose text");
  const reparsed = parseDocument(serializeDocument(edited));
  assert.equal(reparsed[0].kind, "prose");
  assert.equal((reparsed[0] as Extract<Cell, { kind: "prose" }>).markdown, "new prose text");
  // The code cell survives the round-trip unchanged.
  assert.equal((reparsed[1] as Extract<Cell, { kind: "code" }>).source, "(def (main) 1)");
});

// ── renderDocToSurface: re-render CADENZA code cells to the selected surface (operator UX #2, my half) ──
// A fake `render` tags the source with its target; it receives the GATHERED cell (a `(do …)` wrap for
// s-expr multi-form) and the helper ungathers the result. These stay pure/node-only (no wasm).
const fakeRender = async (text: string, _from: string, to: string) => `[${to}] ${text}`;

test("renderDocToSurface renders CODE cells through the target surface, leaving prose + widgets untouched", async () => {
  const md = "intro\n\n```cadenza widget\nrate : Float64 = slider(0, 1)\n```\n\n```cadenza\n(def (main) rate)\n```";
  const out = await renderDocToSurface(md, "sexpr", "ml", fakeRender);
  const cells = parseDocument(out);
  assert.equal((cells[0] as Extract<Cell, { kind: "prose" }>).markdown, "intro"); // prose untouched
  assert.equal((cells[1] as Extract<Cell, { kind: "code" }>).source, "rate : Float64 = slider(0, 1)"); // WIDGET untouched (not Cadenza)
  // The code cell was GATHERED to `(do (def (main) rate))`, fake-rendered (tagged), then ungathered (ML
  // trim). The `[ml]` tag confirms it went through the render path.
  assert.match((cells[2] as Extract<Cell, { kind: "code" }>).source, /^\[ml\] /);
  assert.ok((cells[2] as Extract<Cell, { kind: "code" }>).source.includes("(def (main) rate)"));
});

test("renderDocToSurface gathers a MULTI-form cell so the single-form renderer accepts it", async () => {
  // A cell with a helper `def` + `main` (the quadratic-value-cell shape) must be gathered into one `(do …)`
  // form for the render, else `renderSyntax` throws "trailing input". The fake render echoes what it got;
  // we assert it received the gathered single form (starts with `(do `), not the raw two-form source.
  let seen = "";
  const capture = async (text: string) => { seen = text; return text; };
  const md = "```cadenza\n(def (helper) 1)\n(def (main) helper)\n```";
  await renderDocToSurface(md, "sexpr", "ml", capture);
  assert.ok(seen.startsWith("(do "), `multi-form cell should be gathered, render saw: ${seen}`);
  assert.ok(seen.includes("(def (helper) 1)") && seen.includes("(def (main) helper)"));
});

test("renderDocToSurface is a normalizing no-op when from === to (does not call render)", async () => {
  let called = false;
  const md = "```cadenza\n(def (main) 1)\n```";
  const out = await renderDocToSurface(md, "sexpr", "sexpr", async (t) => { called = true; return t; });
  assert.equal(called, false, "render is not called when from === to");
  assert.equal((parseDocument(out)[0] as Extract<Cell, { kind: "code" }>).source, "(def (main) 1)");
});

test("renderDocToSurface keeps a cell's original source when render REJECTS (never drops content)", async () => {
  const rejectRender = async () => { throw new Error("transient parse error"); };
  const md = "```cadenza\n(def (main) 1)\n```";
  const out = await renderDocToSurface(md, "sexpr", "ml", rejectRender);
  assert.equal((parseDocument(out)[0] as Extract<Cell, { kind: "code" }>).source, "(def (main) 1)"); // original preserved
});

test("renderDocToSurface un-gathers a `(do …)` whose head has a NEWLINE, not just a space", async () => {
  // The s-expr pretty-printer emits `(do\n  (def …))` for a multi-LINE body — matching only `(do ` (space)
  // left large multi-form cells wrapped, dropping their top-level defs (the loan/projectile toggle break).
  // This is the ML→s-expr direction (`to: "sexpr"`) — the only leg where the s-expr printer emits `(do …)`.
  const newlineDoRender = async () => "(do\n  (def (year1) 1)\n\n  (def (main) year1))";
  const out = await renderDocToSurface("```cadenza\ndef main() = 1\n```", "ml", "sexpr", newlineDoRender);
  const src = (parseDocument(out)[0] as Extract<Cell, { kind: "code" }>).source;
  assert.ok(!src.startsWith("(do"), `the (do …) wrapper must be peeled, got: ${src}`);
  assert.ok(src.includes("(def (year1) 1)") && src.includes("(def (main) year1)"), `defs promoted to top level, got: ${src}`);
});
