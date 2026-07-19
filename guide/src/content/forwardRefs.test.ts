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
/// Scope: only `<Ch to="/slug">` links (which carry a resolvable slug) paired with a retrospective phrase on
/// the same line. Bare `<strong>Chapter Name</strong>` mentions aren't slug-resolvable, so they're out of
/// scope here (caught by editorial review). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");

/// slug → reading-order index, straight from the registry array order (the reader's linear path).
const indexOf = new Map<string, number>(CHAPTERS.map((c, i) => [c.slug, i]));

/// slug → its chapter TSX filename, from the registry's lazy import (same regex as links.test.ts).
function fileForSlug(): Map<string, string> {
  const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");
  const re = /slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g;
  const m = new Map<string, string>();
  for (let x = re.exec(registrySrc); x; x = re.exec(registrySrc)) m.set(x[1], x[2]);
  return m;
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
      for (const target of chLinksOnLine(line)) {
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
});
