/// Unit tests for the notebook document/cell parser (Increment 1). Pins the structural split of a
/// markdown notebook into ordered prose + Cadenza code cells, the fence-directive parse, and the
/// robustness corners (unclosed fence, non-cadenza fence passthrough, ~~~ fences, blank-prose elision).
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseDocument, parseDirective, type Cell } from "./parseDocument.ts";

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
