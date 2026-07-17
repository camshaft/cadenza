/// Registry invariants for the chapter list — the data that drives routing, the sidebar, prev/next, and
/// the exercise-progress badges. A bad slug 404s a route; a wrong `exercises` count makes the sidebar
/// badge lie (e.g. "2/3" when the chapter has 2 exercises). These are cheap structural checks that catch
/// a content edit drifting the registry out of sync. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS, chapterAt } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url));

test("every chapter has a URL-safe slug, and slugs are unique", () => {
  const seen = new Set<string>();
  for (const c of CHAPTERS) {
    assert.match(c.slug, /^[a-z0-9]+(?:-[a-z0-9]+)*$/, `bad slug: ${JSON.stringify(c.slug)}`);
    assert.ok(!seen.has(c.slug), `duplicate slug: ${c.slug}`);
    seen.add(c.slug);
  }
});

test("every chapter has a non-empty title, blurb, and section", () => {
  for (const c of CHAPTERS) {
    assert.ok(c.title.trim().length > 0, `empty title for ${c.slug}`);
    assert.ok(c.blurb.trim().length > 0, `empty blurb for ${c.slug}`);
    assert.ok(c.section.trim().length > 0, `empty section for ${c.slug}`);
  }
});

// Titles and blurbs are the reader's map of the tour: they label every sidebar row and chapter card.
// Two chapters sharing a title (or a blurb) makes the sidebar read as two identical rows — the classic
// symptom of copy-pasting a registry entry and forgetting to retitle it. The slug check above catches a
// duplicate route, but a distinct slug can still carry a cloned title/blurb, so pin distinctness here.
test("chapter titles and blurbs are each distinct across the registry", () => {
  const titleOf = new Map<string, string>(); // normalized title → first slug that used it
  const blurbOf = new Map<string, string>();
  const dupeTitles: string[] = [];
  const dupeBlurbs: string[] = [];
  for (const c of CHAPTERS) {
    const t = c.title.trim();
    if (titleOf.has(t)) dupeTitles.push(`"${t}" (in ${c.slug} and ${titleOf.get(t)})`);
    else titleOf.set(t, c.slug);
    const b = c.blurb.trim();
    if (blurbOf.has(b)) dupeBlurbs.push(`"${b}" (in ${c.slug} and ${blurbOf.get(b)})`);
    else blurbOf.set(b, c.slug);
  }
  assert.equal(dupeTitles.length, 0, `duplicate chapter title(s) — the sidebar would show identical rows:\n  ${dupeTitles.join("\n  ")}`);
  assert.equal(dupeBlurbs.length, 0, `duplicate chapter blurb(s) — likely a copy-pasted registry entry:\n  ${dupeBlurbs.join("\n  ")}`);
});

test("exercises count is a non-negative integer when present", () => {
  for (const c of CHAPTERS) {
    if (c.exercises !== undefined) {
      assert.ok(Number.isInteger(c.exercises) && c.exercises >= 0, `bad exercises for ${c.slug}`);
    }
  }
});

test("chapterAt resolves a known slug (with its index) and returns null for an unknown one", () => {
  const first = CHAPTERS[0];
  const found = chapterAt(first.slug);
  assert.ok(found, "chapterAt should resolve the first chapter's slug");
  assert.equal(found!.chapter.slug, first.slug);
  assert.equal(found!.index, 0);
  assert.equal(chapterAt("no-such-chapter-xyz"), null);
});

// The registry's `exercises` count drives the sidebar progress badge; it must equal the number of
// <Exercise> elements actually authored in the chapter's TSX. Parse the slug→file map out of the
// registry source (the import path lives inside a `lazy(() => import("./chapters/X.tsx"))` closure),
// then count <Exercise in each file.
test("each chapter's declared `exercises` equals the <Exercise> count in its TSX", () => {
  const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");
  // Map each `slug: "x"` to the nearest following `import("./chapters/File.tsx")`.
  const entryRe = /slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g;
  const fileForSlug = new Map<string, string>();
  for (let m = entryRe.exec(registrySrc); m; m = entryRe.exec(registrySrc)) {
    fileForSlug.set(m[1], m[2]);
  }
  for (const c of CHAPTERS) {
    const file = fileForSlug.get(c.slug);
    assert.ok(file, `no TSX import found for slug ${c.slug}`);
    const tsx = readFileSync(join(here, "chapters", file!), "utf8");
    const exerciseCount = (tsx.match(/<Exercise\b/g) ?? []).length;
    const declared = c.exercises ?? 0;
    assert.equal(
      declared,
      exerciseCount,
      `${c.slug}: registry says exercises:${declared} but ${file} has ${exerciseCount} <Exercise>`,
    );
  }
});
