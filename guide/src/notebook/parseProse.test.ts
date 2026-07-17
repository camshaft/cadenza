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

test("inline: an intraword `_` is NOT emphasis — a snake_case identifier stays literal (CommonMark)", () => {
  // The killer case for a programming notebook: `snake_case_name` must not become "snake <em>case</em> name".
  assert.deepEqual(parseInline("use snake_case_name here"), [{ t: "text", text: "use snake_case_name here" }]);
  assert.deepEqual(parseInline("a_b_c"), [{ t: "text", text: "a_b_c" }]);
  // A word-boundary `_italic_` still IS emphasis (flanked by spaces / string edges).
  assert.deepEqual(parseInline("this is _italic_ ok"), [
    { t: "text", text: "this is " },
    { t: "em", text: "italic" },
    { t: "text", text: " ok" },
  ]);
  assert.deepEqual(parseInline("_lead_ and trail _end_"), [
    { t: "em", text: "lead" },
    { t: "text", text: " and trail " },
    { t: "em", text: "end" },
  ]);
  // `*` emphasis has NO intraword restriction (CommonMark) — intraword `*` still emphasizes.
  assert.deepEqual(parseInline("a*b*c"), [
    { t: "text", text: "a" },
    { t: "em", text: "b" },
    { t: "text", text: "c" },
  ]);
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

test("levels 3–6 parse as DISTINCT levels (ProseView renders real h3/h4/h5/h6, not all-h3 — PR #482)", () => {
  const b = parseProse("### h3\n\n#### h4\n\n##### h5\n\n###### h6");
  assert.deepEqual(
    b.map((x) => (x.t === "heading" ? x.level : x.t)),
    [3, 4, 5, 6],
  );
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

test("a GFM pipe table parses into a table block (header + rows, inline spans per cell)", () => {
  const b = parseProse("| Name | Age |\n|------|-----|\n| Ada | 36 |\n| Bob | 40 |");
  assert.equal(b.length, 1);
  assert.equal(b[0].t, "table");
  if (b[0].t === "table") {
    assert.deepEqual(b[0].header, [[{ t: "text", text: "Name" }], [{ t: "text", text: "Age" }]]);
    assert.equal(b[0].rows.length, 2);
    assert.deepEqual(b[0].rows[0], [[{ t: "text", text: "Ada" }], [{ t: "text", text: "36" }]]);
  }
});

test("table cells carry inline spans (bold/code)", () => {
  const b = parseProse("| a | b |\n|---|---|\n| **x** | `y` |");
  if (b[0].t === "table") {
    assert.deepEqual(b[0].rows[0][0], [{ t: "strong", text: "x" }]);
    assert.deepEqual(b[0].rows[0][1], [{ t: "code", text: "y" }]);
  }
});

test("a delimiter row with alignment colons is accepted", () => {
  const b = parseProse("| a | b |\n|:--|--:|\n| 1 | 2 |");
  assert.equal(b[0].t, "table");
});

test("a pipe line with NO delimiter row stays a paragraph (not a table)", () => {
  const b = parseProse("a | b | c\njust prose with pipes");
  assert.equal(b[0].t, "paragraph");
});

test("a table with no body rows is still a table (header only)", () => {
  const b = parseProse("| h1 | h2 |\n|----|----|");
  assert.equal(b[0].t, "table");
  if (b[0].t === "table") assert.equal(b[0].rows.length, 0);
});

test("GFM strikethrough ~~…~~ parses to a del span", () => {
  assert.deepEqual(parseInline("a ~~struck~~ b"), [
    { t: "text", text: "a " },
    { t: "del", text: "struck" },
    { t: "text", text: " b" },
  ]);
});

test("an unclosed ~~ renders as literal text (never throws)", () => {
  assert.deepEqual(parseInline("a ~~ b"), [{ t: "text", text: "a ~~ b" }]);
});
