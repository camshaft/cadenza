/// No-forward-reference invariant (operator directive: every chapter builds only on EARLIER material).
///
/// A chapter may point FORWARD as a teaser ("next you'll see X", "the X chapter later builds…") — that's
/// fine. What's NOT fine is RETROSPECTIVE phrasing ("you saw / met / learned in X", "the X chapter
/// covered…", "made interactive in X") that names a chapter which comes LATER in the reading order: a
/// linear reader is told they've already seen something they haven't. That's the exact drift that once let
/// "Errors & absence" sit before "The numeric model" while its prose said "you saw that in The numeric
/// model" (see arc.test.ts). arc.test pins section ORDER; this pins per-LINK direction for the retrospective
/// case, which nothing else checks.
///
/// Scope: two ways a chapter names another on a retrospective line — (1) a `<Ch to="/slug">` link, resolved
/// by slug; (2) a bare `<strong>Chapter Title</strong>` mention, resolved by matching the full title against
/// the registry. The `<strong>` case is NOT out of scope: the original "Errors & absence"/"The numeric model"
/// defect was written as prose that said "you saw that in <strong>The numeric model</strong>" (a title, not a
/// <Ch> link), so leaving titles unchecked would let that exact regression back in. Only a COMPLETE registered
/// title matches (normalized for `&amp;`/case), so a generic <strong>records</strong> / <strong>tuples</strong>
/// emphasis — which is not itself a chapter title — never trips it. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS } from "./chapters.ts";
import { fileForSlug } from "./registryFiles.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");

/// slug → reading-order index, straight from the registry array order (the reader's linear path).
const indexOf = new Map<string, number>(CHAPTERS.map((c, i) => [c.slug, i]));

/// Normalize a chapter title for matching against `<strong>…</strong>` prose: HTML-decode the entities the
/// guide actually uses in titles (`&amp;` in "Maps & sets", curly apostrophe in "…the way it is") and
/// lower-case, so a title mention matches regardless of how it was escaped in the JSX. Kept to the exact
/// entities in use — a title is a fixed string, not arbitrary markup.
function normTitle(s: string): string {
  return s
    .replace(/&amp;/g, "&")
    .replace(/&#8217;|&rsquo;/g, "’")
    .trim()
    .toLowerCase();
}

/// normalized full chapter title → slug. Only a COMPLETE title resolves; a generic <strong>records</strong>
/// emphasis (not itself a chapter title) is absent from this map and so is never treated as a chapter mention.
const titleToSlug = new Map<string, string>(CHAPTERS.map((c) => [normTitle(c.title), c.slug]));

/// Every `<strong>Chapter Title</strong>` on a line that resolves to a registered chapter, with its slug.
function titleMentionsOnLine(line: string): string[] {
  const out: string[] = [];
  for (const m of line.matchAll(/<strong>([^<]+)<\/strong>/g)) {
    const slug = titleToSlug.get(normTitle(m[1]));
    if (slug != null) out.push(slug);
  }
  return out;
}

/// Retrospective phrasing: prose that presents a referenced chapter as ALREADY-SEEN. Kept deliberately
/// narrow (past-tense "you …" / "the … chapter covered" / "made interactive") so a forward TEASER
/// ("next", "later", "you'll see") is never flagged.
const RETROSPECTIVE = /\b(you (saw|met|learned|already)|chapter (covered|showed|introduced)|shape from|made interactive|from the previous|as you)\b/i;

/// Every `<Ch to="/slug">` on a line, with its slug.
function chLinksOnLine(line: string): string[] {
  const out: string[] = [];
  for (const m of line.matchAll(/<Ch\s+to="\/([a-z0-9-]+)"/g)) out.push(m[1]);
  return out;
}

test("no chapter retrospectively references a LATER chapter (no forward references)", () => {
  const files = fileForSlug();
  const violations: string[] = [];
  for (const [slug, file] of files) {
    const from = indexOf.get(slug);
    if (from == null) continue;
    const src = readFileSync(join(chaptersDir, file), "utf8").split("\n");
    for (let i = 0; i < src.length; i++) {
      const line = src[i];
      if (!RETROSPECTIVE.test(line)) continue;
      for (const target of [...chLinksOnLine(line), ...titleMentionsOnLine(line)]) {
        const to = indexOf.get(target);
        if (to != null && to > from) {
          violations.push(`${file}:${i + 1} — chapter "${slug}" (#${from}) speaks of "${target}" (#${to}) as already-seen, but it comes later`);
        }
      }
    }
  }
  assert.equal(
    violations.length,
    0,
    `forward reference(s) — retrospective prose pointing at a later chapter:\n  ${violations.join("\n  ")}`,
  );
});

test("the forward-ref scan resolves slugs + reads chapters (guards a vacuous pass)", () => {
  // A broken regex or empty map would make the invariant pass on nothing. Assert the machinery works.
  const files = fileForSlug();
  assert.ok(files.size >= 30, `expected the registry to map many slugs to files, got ${files.size}`);
  assert.ok(indexOf.size >= 30, `expected many chapters in reading order, got ${indexOf.size}`);
  // A known retrospective back-reference exists and is correctly NOT flagged (ExampleApps → effects, backward).
  assert.ok(RETROSPECTIVE.test('idea you met in <Ch to="/effects">'), "retrospective pattern should match a known phrase");
  // The title-mention machinery resolves a real chapter title, and only a COMPLETE title (not a generic
  // <strong> emphasis) resolves — this is what makes the <strong> arm precise rather than a false-positive mill.
  assert.ok(titleToSlug.size >= 30, `expected many chapter titles resolvable, got ${titleToSlug.size}`);
  assert.deepEqual(
    titleMentionsOnLine('you saw that in <strong>The numeric model</strong>'),
    ["numbers"],
    "a full chapter-title mention should resolve to its slug",
  );
  assert.deepEqual(
    titleMentionsOnLine('a record whose fields are <strong>functions</strong>'),
    [],
    "a generic <strong> emphasis that is not a chapter title must NOT resolve",
  );
});
