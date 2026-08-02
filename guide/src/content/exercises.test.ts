/// Exercise-id integrity. Each <Exercise> carries a stable `id` (e.g. "basics:1") that keys the
/// reader's saved progress — completing it is remembered under that id. Two invariants keep progress
/// honest: ids must be globally UNIQUE (a duplicate — say from copy-pasting an exercise — would make
/// two exercises share one completion state, so finishing one silently ticks the other), and each id's
/// prefix must match its chapter's registered slug (a mis-prefixed id, like "basics:1" living in the
/// Floats chapter, attributes progress to the wrong chapter's badge). chapters.test.ts already checks
/// the per-chapter <Exercise> COUNT; this pins the ids themselves. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS } from "./chapters.ts";
import { fileForSlug } from "./registryFiles.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content

/// Every `id="…"` passed to an <Exercise> in a chapter file, in source order.
function exerciseIds(tsx: string): string[] {
  return [...tsx.matchAll(/<Exercise\b[\s\S]*?\bid="([^"]+)"/g)].map((m) => m[1]);
}

test("every exercise id is globally unique across all chapters", () => {
  const seen = new Map<string, string>(); // id → chapter slug that first used it
  const dupes: string[] = [];
  const map = fileForSlug();
  for (const c of CHAPTERS) {
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(here, "chapters", file), "utf8");
    for (const id of exerciseIds(tsx)) {
      if (seen.has(id)) dupes.push(`${id} (in ${c.slug} and ${seen.get(id)})`);
      else seen.set(id, c.slug);
    }
  }
  assert.equal(dupes.length, 0, `duplicate exercise id(s) — progress would collide:\n  ${dupes.join("\n  ")}`);
});

test("every exercise id is prefixed with its own chapter's slug (id = \"<slug>:<n>\")", () => {
  const bad: string[] = [];
  const map = fileForSlug();
  for (const c of CHAPTERS) {
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(here, "chapters", file), "utf8");
    for (const id of exerciseIds(tsx)) {
      const expected = new RegExp(`^${c.slug.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:\\d+$`);
      if (!expected.test(id)) bad.push(`${c.slug}: id "${id}" should look like "${c.slug}:<n>"`);
    }
  }
  assert.equal(bad.length, 0, `mis-prefixed exercise id(s) — progress attributed to the wrong chapter:\n  ${bad.join("\n  ")}`);
});

test("the exercise-id count per chapter equals the registry's declared `exercises`", () => {
  // Complements chapters.test.ts's <Exercise>-tag count by checking it from the id side too, so a
  // tag with no id (or an id outside an <Exercise>) can't drift the two views apart unnoticed.
  const map = fileForSlug();
  for (const c of CHAPTERS) {
    const file = map.get(c.slug);
    if (!file) continue;
    const tsx = readFileSync(join(here, "chapters", file), "utf8");
    assert.equal(
      exerciseIds(tsx).length,
      c.exercises ?? 0,
      `${c.slug}: found ${exerciseIds(tsx).length} exercise id(s) but registry declares exercises:${c.exercises ?? 0}`,
    );
  }
});

test("the exercise scan sees real chapters + exercises (guards a vacuous pass)", () => {
  // All three tests above iterate CHAPTERS and assert a violation set is EMPTY — so if fileForSlug()'s
  // regex broke (empty map) or CHAPTERS were empty, every loop would run zero times and all three would
  // pass on nothing, hiding a real regression (e.g. a mis-prefixed id would sail through). Assert the
  // machinery actually reads a healthy corpus: many chapters mapped to files, and a realistic number of
  // exercise ids found across them (the registry declares dozens), so a broken scan trips here instead.
  const map = fileForSlug();
  assert.ok(map.size >= 30, `expected the registry to map many slugs to files, got ${map.size}`);
  let totalIds = 0;
  for (const c of CHAPTERS) {
    const file = map.get(c.slug);
    if (!file) continue;
    totalIds += exerciseIds(readFileSync(join(here, "chapters", file), "utf8")).length;
  }
  const declared = CHAPTERS.reduce((n, c) => n + (c.exercises ?? 0), 0);
  assert.ok(totalIds >= 20, `expected many exercise ids across the guide, found ${totalIds}`);
  assert.equal(totalIds, declared, `exercise-id scan (${totalIds}) disagrees with total declared exercises (${declared})`);
});
