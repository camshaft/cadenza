/// Unit tests for the minimal markdown prose parser (Increment 2b prep). Pins the supported block set
/// (headings, paragraphs, lists, blockquotes) + inline spans (strong/em/code/link) + the documented
/// graceful degradation (unclosed delimiters → literal text). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseProse, parseInline } from "./parseProse.ts";

test("inline: bold / italic / code / link", () => {
  assert.deepEqual(parseInline("a **b** c"), [
    { t: "text", text: "a " },
    { t: "strong", text: "b" },
    { t: "text", text: " c" },
  ]);
  assert.deepEqual(parseInline("*i* and _j_"), [
    { t: "em", text: "i" },
    { t: "text", text: " and " },
    { t: "em", text: "j" },
  ]);
  assert.deepEqual(parseInline("use `code` here"), [
    { t: "text", text: "use " },
    { t: "code", text: "code" },
    { t: "text", text: " here" },
  ]);
  assert.deepEqual(parseInline("see [the guide](/guide)"), [
    { t: "text", text: "see " },
    { t: "link", text: "the guide", href: "/guide" },
  ]);
});

test("inline: `**` takes precedence over `*` (bold before italic)", () => {
  assert.deepEqual(parseInline("**strong**"), [{ t: "strong", text: "strong" }]);
});

test("inline: an unclosed delimiter renders as literal text (never throws)", () => {
  assert.deepEqual(parseInline("a * b"), [{ t: "text", text: "a * b" }]);
  assert.deepEqual(parseInline("`unclosed"), [{ t: "text", text: "`unclosed" }]);
  assert.deepEqual(parseInline("[label](no-close"), [{ t: "text", text: "[label](no-close" }]);
});

test("inline code content is literal — no nested inline parsing", () => {
  assert.deepEqual(parseInline("`a *b* c`"), [{ t: "code", text: "a *b* c" }]);
});

test("headings parse at each level with inline spans", () => {
  const b = parseProse("# Title\n\n### Sub *emph*");
  assert.deepEqual(b[0], { t: "heading", level: 1, spans: [{ t: "text", text: "Title" }] });
  assert.deepEqual(b[1], { t: "heading", level: 3, spans: [{ t: "text", text: "Sub " }, { t: "em", text: "emph" }] });
});

test("consecutive non-blank lines coalesce into one paragraph; a blank line splits paragraphs", () => {
  const b = parseProse("line one\nline two\n\nsecond para");
  assert.equal(b.length, 2);
  assert.deepEqual(b[0], { t: "paragraph", spans: [{ t: "text", text: "line one line two" }] });
  assert.deepEqual(b[1], { t: "paragraph", spans: [{ t: "text", text: "second para" }] });
});

test("an unordered list coalesces its items; ordered lists are marked ordered", () => {
  const ul = parseProse("- one\n- two\n- three");
  assert.deepEqual(ul, [{ t: "list", ordered: false, items: [
    [{ t: "text", text: "one" }],
    [{ t: "text", text: "two" }],
    [{ t: "text", text: "three" }],
  ] }]);
  const ol = parseProse("1. first\n2. second");
  assert.equal(ol[0].t, "list");
  if (ol[0].t === "list") assert.equal(ol[0].ordered, true);
});

test("list items carry inline spans", () => {
  const b = parseProse("- a **bold** item");
  if (b[0].t === "list") assert.deepEqual(b[0].items[0], [
    { t: "text", text: "a " },
    { t: "strong", text: "bold" },
    { t: "text", text: " item" },
  ]);
});

test("blockquotes coalesce consecutive > lines", () => {
  const b = parseProse("> quoted line one\n> quoted line two");
  assert.deepEqual(b, [{ t: "blockquote", spans: [{ t: "text", text: "quoted line one quoted line two" }] }]);
});

test("a mixed document parses into ordered blocks of the right kinds", () => {
  const doc = "# H\n\nintro para\n\n- a\n- b\n\n> note";
  const b = parseProse(doc);
  assert.deepEqual(b.map((x) => x.t), ["heading", "paragraph", "list", "blockquote"]);
});

test("CRLF prose is normalized (no stray \\r leaking into spans)", () => {
  const b = parseProse("# H\r\n\r\npara");
  assert.deepEqual(b[0], { t: "heading", level: 1, spans: [{ t: "text", text: "H" }] });
  assert.deepEqual(b[1], { t: "paragraph", spans: [{ t: "text", text: "para" }] });
});
