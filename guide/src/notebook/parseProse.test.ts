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

// ── inline + block math `$…$` / `$$…$$` (operator: KaTeX-quality formula rendering) — parse to raw-TeX
// spans/blocks the KaTeX render layer typesets. Content is LITERAL (no nested inline); graceful on a lone
// `$` (currency) and an unclosed delimiter. ──
test("inline math $…$ parses to a math span carrying the raw TeX", () => {
  assert.deepEqual(parseInline("mass-energy $E = mc^2$ holds"), [
    { t: "text", text: "mass-energy " },
    { t: "math", tex: "E = mc^2" },
    { t: "text", text: " holds" },
  ]);
});

test("inline math content is LITERAL — no nested inline parsing inside $…$", () => {
  // `_` and `*` inside math must NOT become em/strong (they're TeX subscripts/operators).
  assert.deepEqual(parseInline("$a_i * b^*$"), [{ t: "math", tex: "a_i * b^*" }]);
});

test("a lone $ (currency) with no closer stays literal text (never throws)", () => {
  assert.deepEqual(parseInline("it costs $5 today"), [{ t: "text", text: "it costs $5 today" }]);
});

test("TWO currency amounts don't get swallowed into math (tight-delimiter rule)", () => {
  // Regression: `$5 and $10` — WITHOUT the tight-close rule, `$5 and $` parsed as math(`5 and `). The close
  // `$` here is preceded by a space (` $10`), so it's not a valid math close; the run stays literal currency.
  // (This notebook is finance-flavored — compound interest / loan — so currency-in-prose is a real case.)
  assert.deepEqual(parseInline("price $5 and $10 total"), [{ t: "text", text: "price $5 and $10 total" }]);
});

test("tight-delimiter rule: a leading- or trailing-space $ flank is NOT math", () => {
  // `$ x$` (space after open) and `$x $` (space before close) are rejected — a bare `$` next to whitespace
  // reads as a literal dollar (currency/text), not a math delimiter. Real math still allowed (internal spaces).
  assert.equal(parseInline("a $ x$ b").some((s) => s.t === "math"), false, "leading-space open is not math");
  assert.equal(parseInline("a $x $ b").some((s) => s.t === "math"), false, "trailing-space close is not math");
  assert.deepEqual(parseInline("$E = mc^2$"), [{ t: "math", tex: "E = mc^2" }], "internal spaces still fine");
});

test("an empty $$ mid-text is NOT inline math (it's the block delimiter) — stays literal", () => {
  // `$$` inline shouldn't swallow to a far `$`; the leading `$` is followed by `$`, so the inline rule skips.
  const spans = parseInline("a $$ b");
  assert.equal(spans.some((s) => s.t === "math"), false, "no inline math span from a bare $$");
});

test("two inline math spans on one line both parse", () => {
  assert.deepEqual(parseInline("$x$ and $y$"), [
    { t: "math", tex: "x" },
    { t: "text", text: " and " },
    { t: "math", tex: "y" },
  ]);
});

test("inline `code` OUTRANKS math — $…$ inside backticks stays literal code, not a math span", () => {
  // A programming notebook's prose routinely puts `$` inside inline code: a shell var `$HOME`, a template
  // literal `` `${x}` ``, a currency literal in a code snippet. `code` is scanned before math (higher
  // precedence, its content literal), so the `$…$` inside never becomes a math span. Pinned so a future
  // reorder of the inline scan can't silently start typesetting the innards of a code span.
  assert.deepEqual(parseInline("run `echo $x$y` now"), [
    { t: "text", text: "run " },
    { t: "code", text: "echo $x$y" },
    { t: "text", text: " now" },
  ]);
  // And the reverse ordering (math before code) still keeps each in its own span — no cross-swallow.
  assert.deepEqual(parseInline("$a$ then `b`"), [
    { t: "math", tex: "a" },
    { t: "text", text: " then " },
    { t: "code", text: "b" },
  ]);
});

test("math inside **strong** stays LITERAL — strong content is not re-parsed for math", () => {
  // Strong (like code) takes its content literally — a `$…$` inside `**…**` is part of the bold text, not a
  // nested math span (parseInline does not recurse into a strong run). Pinned so bold prose containing a `$`
  // (bold currency, a bolded formula the author wants shown as-typed) never spuriously typesets.
  assert.deepEqual(parseInline("**cost is $5 flat**"), [{ t: "strong", text: "cost is $5 flat" }]);
});

test("a single-line display-math block $$…$$ parses to a mathblock", () => {
  const b = parseProse("$$\\int_0^1 x\\,dx$$");
  assert.equal(b.length, 1);
  assert.deepEqual(b[0], { t: "mathblock", tex: "\\int_0^1 x\\,dx" });
});

test("a multi-line display-math block collects its TeX until the closing $$", () => {
  const b = parseProse("intro\n\n$$\na = b\n+ c\n$$\n\nafter");
  assert.deepEqual(b.map((x) => x.t), ["paragraph", "mathblock", "paragraph"]);
  const mb = b[1];
  assert.equal(mb.t, "mathblock");
  if (mb.t === "mathblock") assert.equal(mb.tex, "a = b\n+ c");
});

test("an unclosed $$ block runs to EOF with what it accumulated (never drops content)", () => {
  const b = parseProse("$$\na = b");
  assert.equal(b.length, 1);
  assert.deepEqual(b[0], { t: "mathblock", tex: "a = b" });
});

test("a display-math block is not mistaken for a paragraph (block precedence)", () => {
  const b = parseProse("$$x^2$$");
  assert.equal(b[0].t, "mathblock");
});
