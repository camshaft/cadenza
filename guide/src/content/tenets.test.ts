/// Tenet-box coverage — the guide's narrative spine. Philosophy promises the reader: "Each chapter
/// flags the tenet at work in a ✦ Why it's this way box." That <Why> box is how every chapter ties its
/// mechanics back to the handful of ideas the language is built on — it's the through-line that makes
/// the tour a story rather than a feature list. A *teaching* chapter that shipped without one would
/// quietly break that promise, and nothing else would catch it. This test pins the invariant: every
/// chapter in a *teaching* section carries at least one <Why> tenet box. Non-teaching sections are
/// exempt: "Wrapping up" (the playground tool-tour + the recap) and "Example applications" (the app
/// showcase gallery) don't introduce a concept, so they have no tenet to flag. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS, NON_TEACHING_SECTIONS, pillarOf } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");

// NON_TEACHING_SECTIONS (the sections exempt from the tenet-box invariant — "Wrapping up" wraps up rather
// than teaches; "Example applications" is a showcase whose tenets live in the chapters it cross-links to)
// is imported from chapters.ts, a single source of truth shared with opener.test.ts so the two gates can't
// drift on which sections count as teaching.

/// slug → TSX filename, parsed from the registry (same idiom as chapters.test.ts / exercises.test.ts).
function fileForSlug(): Map<string, string> {
  const entryRe = /slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g;
  const map = new Map<string, string>();
  for (let m = entryRe.exec(registrySrc); m; m = entryRe.exec(registrySrc)) map.set(m[1], m[2]);
  return map;
}

function whyBoxCount(tsx: string): number {
  return (tsx.match(/<Why\b/g) ?? []).length;
}

test("every teaching chapter (outside Wrapping up) has at least one <Why> tenet box", () => {
  const map = fileForSlug();
  const missing: string[] = [];
  for (const c of CHAPTERS) {
    // The <Why> tenet spine is a LANGUAGE-pillar promise (Philosophy's "each chapter flags the tenet at
    // work"). The PLATFORM pillar is concept-level and early-stage; its chapters will grow their own
    // framing as the kernel design settles, so they're exempt from the language tenet-box invariant for now.
    if (pillarOf(c) !== "language") continue;
    if (NON_TEACHING_SECTIONS.has(c.section)) continue;
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(here, "chapters", file), "utf8");
    if (whyBoxCount(tsx) === 0) missing.push(`${c.slug} (${c.section})`);
  }
  assert.equal(
    missing.length,
    0,
    `teaching chapter(s) with no <Why> tenet box — the "every chapter flags a tenet" spine is broken:\n  ${missing.join(
      "\n  ",
    )}`,
  );
});

test("the tenet-box scan finds the expected coverage (guards against a broken matcher)", () => {
  // A broken <Why> matcher would pass the coverage test vacuously (0 == 0). Assert the total across
  // teaching chapters is healthy, so a regex/refactor break trips here instead of hiding.
  const map = fileForSlug();
  let total = 0;
  let teaching = 0;
  for (const c of CHAPTERS) {
    if (NON_TEACHING_SECTIONS.has(c.section)) continue;
    const file = map.get(c.slug);
    if (!file) continue;
    teaching++;
    total += whyBoxCount(readFileSync(join(here, "chapters", file), "utf8"));
  }
  // One box per teaching chapter is the floor; several carry more.
  assert.ok(total >= teaching, `expected >= ${teaching} <Why> boxes across teaching chapters, found ${total}`);
});
