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
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS, pillarOf, type Chapter } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const DIFFERENTIATORS_SECTION = "What makes Cadenza different";

/// Per-pillar section order, in the order a reader walks each pillar. The two pillars are independent
/// arcs: "Cadenza the Language" is the original tour; "Cadenza the Platform" is the agent-kernel concept
/// section (grows incrementally). If a section is renamed or added, update the right pillar's list
/// deliberately — that edit IS a narrative decision, and forcing it through here is the point.
const SECTION_ORDER: Record<string, string[]> = {
  language: [
    "Getting started",
    "Fundamentals",
    "What makes Cadenza different",
    "Example applications",
    "Wrapping up",
  ],
  platform: [
    "The kernel model",
  ],
};

/// Chapters of one pillar, in registry order.
const inPillar = (pillar: string): Chapter[] => CHAPTERS.filter((c) => pillarOf(c) === pillar);

test("every chapter's section is one of its pillar's known sections", () => {
  const bad = CHAPTERS.filter((c) => !(SECTION_ORDER[pillarOf(c)] ?? []).includes(c.section));
  assert.equal(
    bad.length,
    0,
    `chapter(s) in an unlisted section — add it to SECTION_ORDER[pillar] on purpose:\n  ${bad
      .map((c) => `${c.slug} → ${pillarOf(c)} / ${JSON.stringify(c.section)}`)
      .join("\n  ")}`,
  );
});

test("each section's chapters are contiguous (a section never gets split by another), within its pillar", () => {
  for (const pillar of Object.keys(SECTION_ORDER)) {
    const chapters = inPillar(pillar);
    const seenBlocks: string[] = [];
    chapters.forEach((c) => {
      if (seenBlocks[seenBlocks.length - 1] !== c.section) seenBlocks.push(c.section);
    });
    const counts = new Map<string, number>();
    for (const s of seenBlocks) counts.set(s, (counts.get(s) ?? 0) + 1);
    const split = [...counts.entries()].filter(([, n]) => n > 1).map(([s]) => s);
    assert.equal(
      split.length,
      0,
      `[${pillar}] section(s) split across the registry (chapters of one section are not contiguous): ${split.join(", ")}`,
    );
  }
});

test("sections appear in the intended reading order, within each pillar", () => {
  for (const [pillar, order] of Object.entries(SECTION_ORDER)) {
    const chapters = inPillar(pillar);
    const present = order.filter((s) => chapters.some((c) => c.section === s));
    const firstIndexOf = (section: string) => chapters.findIndex((c) => c.section === section);
    const actual = [...present].sort((a, b) => firstIndexOf(a) - firstIndexOf(b));
    assert.deepEqual(
      actual,
      present,
      `[${pillar}] sections are out of order.\n  expected: ${present.join(" → ")}\n  actual:   ${actual.join(" → ")}`,
    );
  }
});

test("pillars are contiguous and in order (language before platform)", () => {
  // All of a pillar's chapters must sit together, and the language pillar comes first — the platform
  // pillar is appended after the whole language tour. A stray platform chapter mid-language (or vice
  // versa) would scatter the sidebar and break the two-pillar framing.
  const seq: string[] = [];
  CHAPTERS.forEach((c) => {
    const p = pillarOf(c);
    if (seq[seq.length - 1] !== p) seq.push(p);
  });
  assert.deepEqual(seq, ["language", "platform"], `pillars must be contiguous and language-first; got ${seq.join(" → ")}`);
});

test("the language arc opens on Welcome and closes on Where-to-go-next", () => {
  // The language pillar's first and last chapters are load-bearing: the opener sets up the tour, the
  // closer recaps and sends the reader onward. A reorder that displaced either would break the framing.
  const lang = inPillar("language");
  assert.equal(lang[0].slug, "welcome", "the first language chapter should be the Welcome opener");
  assert.equal(
    lang[lang.length - 1].slug,
    "whats-next",
    "the last language chapter should be the Where-to-go-next closer",
  );
});

// The closer (WhatsNext) recaps the tour with "you've seen what makes Cadenza its own language: <list of
// every differentiator, each linked>". That recap is the reader's final mental map of the differentiators;
// if a NEW differentiator chapter is added to the registry but not woven into the recap, the closer
// silently drops it — the reader finishes with an incomplete picture and nothing else in the suite
// notices. (links.test.ts checks the recap's links aren't DEAD; this checks the recap is COMPLETE.) Pin
// it: every chapter in the differentiators section must be linked from the closer.
test("the Where-to-go-next closer links every differentiator chapter in its recap", () => {
  const closer = CHAPTERS.find((c) => c.slug === "whats-next");
  assert.ok(closer, "no whats-next closer chapter");
  const wn = readFileSync(join(here, "chapters", "WhatsNext.tsx"), "utf8");
  const linked = new Set([...wn.matchAll(/to="\/([a-z0-9-]+)"/g)].map((m) => m[1]));
  const differentiators = CHAPTERS.filter((c) => c.section === DIFFERENTIATORS_SECTION).map((c) => c.slug);
  const missing = differentiators.filter((slug) => !linked.has(slug));
  assert.equal(
    missing.length,
    0,
    `the closer (WhatsNext) does not recap these differentiator chapter(s): ${missing.join(", ")} — every differentiator needs a place in the reader's final map`,
  );
});

test("the differentiators-recap scan finds links (guards against a broken scan)", () => {
  // A broken link-scan would make the recap-completeness test pass vacuously. Assert the closer links a
  // healthy number of differentiators so a regex/file-move break trips here instead of hiding.
  const wn = readFileSync(join(here, "chapters", "WhatsNext.tsx"), "utf8");
  const linked = new Set([...wn.matchAll(/to="\/([a-z0-9-]+)"/g)].map((m) => m[1]));
  const differentiators = CHAPTERS.filter((c) => c.section === DIFFERENTIATORS_SECTION).map((c) => c.slug);
  const found = differentiators.filter((slug) => linked.has(slug)).length;
  assert.ok(found >= 5, `expected the closer to link many differentiators, found ${found}`);
});
