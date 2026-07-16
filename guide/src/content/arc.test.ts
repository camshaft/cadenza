/// Narrative-arc structure invariants for the chapter registry. The guide is a *story*: it opens by
/// saying what Cadenza is, teaches the fundamentals, builds to what makes Cadenza different, and winds
/// down — in that order, with each section's chapters contiguous. Those are editorial invariants the
/// prose leans on (forward-bridges say "next we'll…", a section pivot hands off to the next section),
/// but the registry is a plain array anyone can splice: a careless insert can scatter a section or
/// reorder the arc, and nothing else in the suite would notice. (That's the class of drift that let
/// "Errors & absence" sit before "The numeric model" while its prose said "you saw that in The numeric
/// model".) These tests pin the shape so a future registry edit that breaks the arc fails here.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { CHAPTERS } from "./chapters.ts";

/// The sections, in the order a reader walks them. If a section is renamed or a new one added (e.g. an
/// "Example applications" capstone), update this list deliberately — that edit IS a narrative decision,
/// and forcing it through here is the point.
const SECTION_ORDER = [
  "Getting started",
  "Fundamentals",
  "What makes Cadenza different",
  "Wrapping up",
];

test("every chapter's section is one of the known sections", () => {
  const known = new Set(SECTION_ORDER);
  const unknown = CHAPTERS.filter((c) => !known.has(c.section));
  assert.equal(
    unknown.length,
    0,
    `chapter(s) in an unlisted section — add it to SECTION_ORDER on purpose:\n  ${unknown
      .map((c) => `${c.slug} → ${JSON.stringify(c.section)}`)
      .join("\n  ")}`,
  );
});

test("each section's chapters are contiguous (a section never gets split by another)", () => {
  // Record the run of indices where each section appears; a section must occupy exactly one run.
  const firstSeen = new Map<string, number>();
  const seenBlocks: string[] = [];
  CHAPTERS.forEach((c) => {
    if (seenBlocks[seenBlocks.length - 1] !== c.section) seenBlocks.push(c.section);
    if (!firstSeen.has(c.section)) firstSeen.set(c.section, seenBlocks.length - 1);
  });
  // A section is split iff it appears as more than one distinct block.
  const counts = new Map<string, number>();
  for (const s of seenBlocks) counts.set(s, (counts.get(s) ?? 0) + 1);
  const split = [...counts.entries()].filter(([, n]) => n > 1).map(([s]) => s);
  assert.equal(
    split.length,
    0,
    `section(s) split across the registry (chapters of one section are not contiguous): ${split.join(", ")}`,
  );
});

test("sections appear in the intended reading order", () => {
  // The order sections first appear in the registry must match SECTION_ORDER (subset-tolerant: only
  // checks the relative order of sections that are present).
  const present = SECTION_ORDER.filter((s) => CHAPTERS.some((c) => c.section === s));
  const firstIndexOf = (section: string) => CHAPTERS.findIndex((c) => c.section === section);
  const actual = [...present].sort((a, b) => firstIndexOf(a) - firstIndexOf(b));
  assert.deepEqual(
    actual,
    present,
    `sections are out of order.\n  expected: ${present.join(" → ")}\n  actual:   ${actual.join(" → ")}`,
  );
});

test("the arc opens on Welcome and closes on Where-to-go-next", () => {
  // The first and last chapters are load-bearing: the opener sets up the whole tour, the closer recaps
  // and sends the reader onward. A reorder that displaced either would break the framing.
  assert.equal(CHAPTERS[0].slug, "welcome", "the first chapter should be the Welcome opener");
  assert.equal(
    CHAPTERS[CHAPTERS.length - 1].slug,
    "whats-next",
    "the last chapter should be the Where-to-go-next closer",
  );
});
