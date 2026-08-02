/// Chapter-opener consistency — every chapter's on-ramp. A reader walking the guide meets each chapter
/// the same way: an <H1> title, then a <Lede> — the one-paragraph hook that says what this chapter is
/// about and why it's next. That opener is the per-chapter entry point of the narrative; it's what makes
/// the tour feel authored rather than a pile of reference pages, and the guide reached full <Lede>
/// coverage by hand. Nothing else in the suite pins it: a new chapter (or a rewrite that drops the hook)
/// could ship straight into an <H2>/<P> with no lede and no test would notice — the reader just lands
/// with no on-ramp. This pins two shape invariants: every chapter carries an <H1>, and every *teaching*
/// chapter opens with a <Lede>. Non-teaching sections are exempt from the lede (mirroring tenets.test.ts):
/// "Wrapping up" (the playground tour + recap + toolchain) and "Example applications" (the showcase
/// gallery) are not concept chapters, so they don't owe the reader a concept-hook lede. Run with
/// `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS, NON_TEACHING_SECTIONS, pillarOf } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");
const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");

// NON_TEACHING_SECTIONS (the sections exempt from the <Lede>/<Runnable> teaching-chapter invariants) is
// imported from chapters.ts — a single source of truth shared with tenets.test.ts, so the two gates can't
// drift on which sections count as teaching.

/// slug → TSX filename, parsed from the registry (same idiom as chapters.test.ts / tenets.test.ts).
function fileForSlug(): Map<string, string> {
  const entryRe = /slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g;
  const map = new Map<string, string>();
  for (let m = entryRe.exec(registrySrc); m; m = entryRe.exec(registrySrc)) map.set(m[1], m[2]);
  return map;
}

test("every chapter carries an <H1> title", () => {
  const map = fileForSlug();
  const missing: string[] = [];
  for (const c of CHAPTERS) {
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(chaptersDir, file), "utf8");
    if (!/<H1\b/.test(tsx)) missing.push(`${c.slug} (${file})`);
  }
  assert.equal(
    missing.length,
    0,
    `chapter(s) with no <H1> title — the reader lands with no heading:\n  ${missing.join("\n  ")}`,
  );
});

test("every teaching chapter opens with a <Lede> hook", () => {
  const map = fileForSlug();
  const missing: string[] = [];
  for (const c of CHAPTERS) {
    if (NON_TEACHING_SECTIONS.has(c.section)) continue;
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(chaptersDir, file), "utf8");
    if (!/<Lede\b/.test(tsx)) missing.push(`${c.slug} (${file}, section ${JSON.stringify(c.section)})`);
  }
  assert.equal(
    missing.length,
    0,
    `teaching chapter(s) with no <Lede> hook — the reader gets no on-ramp into the chapter:\n  ${missing.join("\n  ")}`,
  );
});

/// Chapters that teach a concept but deliberately carry NO runnable example. `philosophy` is the guide's
/// tenets essay — a prose manifesto in "Getting started" that argues *why* the language is shaped as it is,
/// before any code; it earns its place without a <Runnable>. Any OTHER teaching chapter with no runnable is
/// a broken promise (see the invariant below), so this exemption is a single named entry, not a section.
const RUNNABLE_EXEMPT = new Set(["philosophy"]);

test("every teaching chapter carries at least one <Runnable> (Welcome's 'every example is live' promise)", () => {
  // Welcome tells the reader "every example below is live: you can edit the code and press Run". That
  // interactivity IS the guide's pitch — a teaching chapter that presents only static prose quietly breaks
  // it. Pin it: each teaching chapter (outside the non-teaching sections and the deliberately-essay
  // `philosophy`) has a <Runnable>. Nothing else in the suite checks the interactive surface exists.
  const map = fileForSlug();
  const missing: string[] = [];
  for (const c of CHAPTERS) {
    // Only the LANGUAGE pillar makes the "every example is live" promise — its chapters run in-browser.
    // The PLATFORM pillar is concept-level (the agent kernel has no in-browser runtime yet; the eventual
    // "platform explorer" is the future payoff), so its concept chapters are exempt from the <Runnable>
    // requirement rather than forced to carry a fake one.
    if (pillarOf(c) !== "language") continue;
    if (NON_TEACHING_SECTIONS.has(c.section) || RUNNABLE_EXEMPT.has(c.slug)) continue;
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(chaptersDir, file), "utf8");
    if (!/<Runnable\b/.test(tsx)) missing.push(`${c.slug} (${file}, section ${JSON.stringify(c.section)})`);
  }
  assert.equal(
    missing.length,
    0,
    `teaching chapter(s) with no <Runnable> — the reader gets static prose, breaking the "every example is live" promise:\n  ${missing.join("\n  ")}`,
  );
});

test("the opener scan resolves files + sees both teaching and non-teaching chapters (guards a vacuous pass)", () => {
  // A broken registry parse or a mis-set exemption would make the invariants pass on nothing. Assert the
  // machinery reads real chapters and that BOTH buckets are non-empty (so neither test is checking zero rows).
  const map = fileForSlug();
  assert.ok(map.size >= 30, `expected the registry to map many slugs to files, got ${map.size}`);
  const teaching = CHAPTERS.filter((c) => !NON_TEACHING_SECTIONS.has(c.section));
  const nonTeaching = CHAPTERS.filter((c) => NON_TEACHING_SECTIONS.has(c.section));
  assert.ok(teaching.length >= 20, `expected many teaching chapters, got ${teaching.length}`);
  assert.ok(nonTeaching.length >= 1, `expected at least one non-teaching (lede-exempt) chapter, got ${nonTeaching.length}`);
  // Every runnable-exempt slug must name a REAL chapter — a typo'd exemption would silently over-exempt
  // (waive the <Runnable> requirement for a chapter that doesn't exist while a real one slips through).
  const slugs = new Set(CHAPTERS.map((c) => c.slug));
  for (const ex of RUNNABLE_EXEMPT) {
    assert.ok(slugs.has(ex), `RUNNABLE_EXEMPT names "${ex}", which is not a registered chapter slug`);
  }
});
