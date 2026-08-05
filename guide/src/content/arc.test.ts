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
import { CHAPTERS, pillarOf, type Chapter, type Pillar } from "./chapters.ts";
import { fileForSlug } from "./registryFiles.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const DIFFERENTIATORS_SECTION = "What makes Cadenza different";

/// Per-pillar section order, in the order a reader walks each pillar. The two pillars are independent
/// arcs: "Cadenza the Language" is the original tour; "Cadenza the Platform" is the agent-kernel concept
/// section (grows incrementally). If a section is renamed or added, update the right pillar's list
/// deliberately — that edit IS a narrative decision, and forcing it through here is the point.
// Keyed by the Pillar union (not a bare string) so a mistyped pillar key is a compile error rather than
// silently producing an empty section list (PR#1023 / Copilot polish).
const SECTION_ORDER: Record<Pillar, string[]> = {
  language: [
    "Getting started",
    "Fundamentals",
    "What makes Cadenza different",
    "Example applications",
    "Wrapping up",
  ],
  platform: [
    "The kernel model",
    "Events & state",
    "Doing things safely",
    "The execution model",
    "Writing a reducer",
  ],
};

/// The pillar keys, typed as Pillar (Object.keys widens to string, which would defeat the Pillar-keyed
/// SECTION_ORDER — iterate this instead).
const PILLAR_KEYS = Object.keys(SECTION_ORDER) as Pillar[];

/// Chapters of one pillar, in registry order.
const inPillar = (pillar: Pillar): Chapter[] => CHAPTERS.filter((c) => pillarOf(c) === pillar);

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
  for (const pillar of PILLAR_KEYS) {
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
  for (const pillar of PILLAR_KEYS) {
    const order = SECTION_ORDER[pillar];
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

// The PLATFORM pillar's last chapter closes with a "Where this leaves you" recap that links back to the
// earlier platform chapters — the reader's final map of the kernel model, exactly as WhatsNext is for the
// language differentiators (test above). The same drift applies: add a new platform concept chapter
// BEFORE the closer and, unless it's woven into that recap, the closer silently drops it and the reader
// finishes with an incomplete picture — and nothing else notices (arc pins order; links pins non-dead;
// neither pins COMPLETE). Pin the platform recap the same way. Derives the closer (LAST platform chapter)
// and the expected set (every EARLIER platform chapter) from the registry, so it holds as the pillar grows.
test("the platform pillar's closer recaps every earlier platform chapter", () => {
  const platform = CHAPTERS.filter((c) => pillarOf(c) === "platform");
  // Only meaningful once the pillar has a closer AND at least one earlier chapter to recap.
  if (platform.length < 2) return;

  const closer = platform[platform.length - 1];
  const earlier = platform.slice(0, -1).map((c) => c.slug);

  const file = fileForSlug().get(closer.slug);
  assert.ok(file, `no source file mapped for the platform closer "${closer.slug}"`);
  const src = readFileSync(join(here, "chapters", file!), "utf8");
  const linked = new Set([...src.matchAll(/to="\/([a-z0-9-]+)"/g)].map((m) => m[1]));

  const missing = earlier.filter((slug) => !linked.has(slug));
  assert.equal(
    missing.length,
    0,
    `the platform closer "${closer.slug}" (${file}) does not recap these earlier platform chapter(s): ${missing.join(", ")} — every platform concept needs a place in the reader's final map`,
  );
});

test("the platform-recap scan resolves the closer + finds links (guards a vacuous pass)", () => {
  // A broken resolver/scan would make the recap-completeness test pass vacuously (0 missing of 0). Assert
  // the pillar split is real and the closer links its earlier chapters, so a regex/file-move break trips
  // here instead of hiding.
  const platform = CHAPTERS.filter((c) => pillarOf(c) === "platform");
  assert.ok(platform.length >= 2, `expected a platform pillar with a closer + earlier chapters, got ${platform.length}`);
  const closer = platform[platform.length - 1];
  const file = fileForSlug().get(closer.slug);
  assert.ok(file, `no source file mapped for the platform closer "${closer.slug}"`);
  const linked = new Set([...readFileSync(join(here, "chapters", file!), "utf8").matchAll(/to="\/([a-z0-9-]+)"/g)].map((m) => m[1]));
  const earlier = platform.slice(0, -1).map((c) => c.slug);
  const found = earlier.filter((slug) => linked.has(slug)).length;
  assert.ok(found >= 1, `expected the platform closer to link at least one earlier platform chapter, found ${found}`);
});
