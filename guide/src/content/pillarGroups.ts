/// The pure grouping transform behind the sidebar's two-level table of contents: chapters grouped by
/// pillar, then by section within each pillar. Extracted out of `Layout.tsx` (which imports
/// react-router-dom and so is unreachable from `node --test`) so this render-layer invariant can be
/// unit-tested in isolation — the sidebar's SHAPE is correctness-critical (a reordered pillar or a
/// dropped empty-pillar filter silently scrambles the nav), and `arc.test.ts` only pins the registry
/// DATA (pillar contiguity/order), not this consuming transform. Parameterized over the chapter list and
/// the pillar list so a test can drive it with fixtures instead of the live registry.

import type { Chapter, Pillar } from "./chapters.ts";
import { pillarOf } from "./chapters.ts";

/// One pillar's sidebar entry: its id + human label, and its sections in registry order, each section
/// carrying its chapters in registry order. Sections is a tuple list (not a Map) so the caller can map
/// over it directly in JSX.
export interface PillarGroup {
  pillar: Pillar;
  label: string;
  sections: [string, Chapter[]][];
}

/// Group `chapters` by pillar, then by section within each pillar. Pillars appear in `pillars` order
/// (NOT registry order); only pillars that actually have at least one chapter are returned; sections keep
/// registry (first-seen) order within a pillar, as does each section's chapter list. When just one pillar
/// survives the filter the caller suppresses the pillar header, so a single-pillar site renders exactly as
/// the old flat section list did.
export function groupByPillar(
  chapters: Chapter[],
  pillars: { id: Pillar; label: string }[],
): PillarGroup[] {
  return pillars
    .map(({ id, label }) => {
      const sections = new Map<string, Chapter[]>();
      for (const c of chapters) {
        if (pillarOf(c) !== id) continue;
        const arr = sections.get(c.section) ?? [];
        arr.push(c);
        sections.set(c.section, arr);
      }
      return { pillar: id, label, sections: [...sections.entries()] };
    })
    .filter((p) => p.sections.length > 0);
}
