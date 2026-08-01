/// Pins the sidebar's grouping transform (`groupByPillar`) — the pure render-layer shape that
/// `arc.test.ts` does NOT cover: arc pins the registry DATA (pillar contiguity/order/sections), this pins
/// the CONSUMING transform (empty-pillar filtering, PILLARS-order not registry-order, section first-seen
/// order, single-pillar collapse). A regression here silently scrambles the two-level nav.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { ComponentType } from "react";
import { CHAPTERS, PILLARS, pillarOf, type Chapter, type Pillar } from "./chapters.ts";
import { groupByPillar } from "./pillarGroups.ts";

// A throwaway component — groupByPillar never renders it, it only reads slug/title/section/pillar.
const C = (() => null) as unknown as ComponentType;
function ch(slug: string, section: string, pillar?: Pillar): Chapter {
  return { slug, title: slug, blurb: "", section, pillar, Component: C };
}
const PS: { id: Pillar; label: string }[] = [
  { id: "language", label: "Cadenza the Language" },
  { id: "platform", label: "Cadenza the Platform" },
];

test("groups chapters by pillar, then by section, preserving registry order within each", () => {
  const chapters = [
    ch("a", "Intro", "language"),
    ch("b", "Intro", "language"),
    ch("c", "Advanced", "language"),
    ch("k", "Kernel", "platform"),
  ];
  const groups = groupByPillar(chapters, PS);
  assert.deepEqual(
    groups.map((g) => [g.pillar, g.sections.map(([s, cs]) => [s, cs.map((c) => c.slug)])]),
    [
      ["language", [["Intro", ["a", "b"]], ["Advanced", ["c"]]]],
      ["platform", [["Kernel", ["k"]]]],
    ],
  );
});

test("a pillar with no chapters is dropped (empty-pillar filter)", () => {
  const groups = groupByPillar([ch("a", "Intro", "language")], PS);
  assert.deepEqual(groups.map((g) => g.pillar), ["language"]);
});

test("pillars appear in PILLARS order, not the order chapters first mention them", () => {
  // platform chapter listed FIRST in the registry, yet language must still lead (PILLARS drives order).
  const chapters = [ch("k", "Kernel", "platform"), ch("a", "Intro", "language")];
  const groups = groupByPillar(chapters, PS);
  assert.deepEqual(groups.map((g) => g.pillar), ["language", "platform"]);
});

test("a section split across the list is coalesced under its first appearance", () => {
  // The transform uses a Map keyed by section, so re-encountering a section appends to the same bucket
  // at its first-seen position rather than opening a second bucket.
  const chapters = [
    ch("a", "Intro", "language"),
    ch("c", "Advanced", "language"),
    ch("b", "Intro", "language"),
  ];
  const [lang] = groupByPillar(chapters, PS);
  assert.deepEqual(
    lang.sections.map(([s, cs]) => [s, cs.map((c) => c.slug)]),
    [["Intro", ["a", "b"]], ["Advanced", ["c"]]],
  );
});

test("the pillar label is carried through from PILLARS", () => {
  const groups = groupByPillar([ch("k", "Kernel", "platform")], PS);
  assert.deepEqual(groups.map((g) => g.label), ["Cadenza the Platform"]);
});

test("a single surviving pillar returns one group (drives the header-suppression collapse)", () => {
  const groups = groupByPillar([ch("a", "Intro")], PS); // no pillar → defaults to language
  assert.equal(groups.length, 1);
  assert.equal(groups[0].pillar, "language");
});

test("smoke: the LIVE registry groups into exactly the pillars that have chapters, in PILLARS order", () => {
  const groups = groupByPillar(CHAPTERS, PILLARS);
  const nonEmpty = PILLARS.filter(({ id }) => CHAPTERS.some((c) => pillarOf(c) === id)).map((p) => p.id);
  assert.deepEqual(groups.map((g) => g.pillar), nonEmpty);
  // Every chapter lands in exactly one group, and no group is empty.
  const grouped = groups.flatMap((g) => g.sections.flatMap(([, cs]) => cs));
  assert.equal(grouped.length, CHAPTERS.length);
  for (const g of groups) assert.ok(g.sections.length > 0, `pillar ${g.pillar} has no sections`);
});
