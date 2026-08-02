/// Cross-pillar forward-bridge invariant. The guide is TWO pillars — "Cadenza the Language" (the linear
/// tour) then "Cadenza the Platform" (the agent-kernel concept section). The language pillar comes first
/// and its closer's whole job is "where to go next"; the platform pillar is otherwise reachable only via
/// the sidebar. So the seam between the pillars is load-bearing: the language closer must hand the reader
/// FORWARD into the platform pillar's opener, or the headline two-pillar restructure is a dead end for a
/// linear reader. That bridge is a single prose paragraph anyone can delete while every other narrative
/// test (arc.test pins section order + the differentiator recap; forwardRefs pins link direction) stays
/// green — nothing else notices the pillars became disconnected. Pin it here.
///
/// Endpoints are derived from the registry (last language chapter → first platform chapter), NOT
/// hard-coded, so the invariant keeps holding as either pillar grows or its boundary chapters change.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS, pillarOf, type Chapter } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");

/// slug → its chapter TSX filename, from the registry's lazy import (same regex as links/forwardRefs tests).
function fileForSlug(): Map<string, string> {
  const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");
  const re = /slug:\s*"([^"]+)"[\s\S]*?import\("\.\/chapters\/([^"]+)"\)/g;
  const m = new Map<string, string>();
  for (let x = re.exec(registrySrc); x; x = re.exec(registrySrc)) m.set(x[1], x[2]);
  return m;
}

/// Every `<Ch to="/slug">` target in a chapter's source.
function chLinkTargets(file: string): Set<string> {
  const src = readFileSync(join(chaptersDir, file), "utf8");
  return new Set([...src.matchAll(/<Ch\s+to="\/([a-z0-9-]+)"/g)].map((m) => m[1]));
}

const languageChapters: Chapter[] = CHAPTERS.filter((c) => pillarOf(c) === "language");
const platformChapters: Chapter[] = CHAPTERS.filter((c) => pillarOf(c) === "platform");

test("the language pillar's closer bridges FORWARD into the platform pillar's opener", () => {
  // Only meaningful once a platform pillar exists; if it's ever emptied, there is no seam to pin.
  if (platformChapters.length === 0) return;

  assert.ok(languageChapters.length > 0, "expected a non-empty language pillar");
  const closer = languageChapters[languageChapters.length - 1];
  const opener = platformChapters[0];

  const files = fileForSlug();
  const closerFile = files.get(closer.slug);
  assert.ok(closerFile, `no source file mapped for the language closer "${closer.slug}"`);

  const links = chLinkTargets(closerFile);
  assert.ok(
    links.has(opener.slug),
    `the language closer "${closer.slug}" must link forward to the platform pillar's opener ` +
      `"${opener.slug}" (the two-pillar seam) — otherwise the Platform pillar is reachable only via the ` +
      `sidebar. Add a <Ch to="/${opener.slug}"> bridge to ${closerFile}.`,
  );
});

test("the bridge scan resolves the registry + reads the closer (guards a vacuous pass)", () => {
  // A broken regex or an empty registry would make the invariant above pass on nothing. Assert the
  // machinery works and the pillar split is real, so a scan/file-move break trips here instead of hiding.
  const files = fileForSlug();
  assert.ok(files.size >= 30, `expected the registry to map many slugs to files, got ${files.size}`);
  assert.ok(languageChapters.length >= 20, `expected a substantial language pillar, got ${languageChapters.length}`);
  assert.ok(platformChapters.length >= 1, `expected at least one platform chapter, got ${platformChapters.length}`);

  // The <Ch> link scan finds a healthy number of targets in the closer (the recap links every
  // differentiator), so a broken regex can't silently make the bridge test vacuous.
  const closerFile = files.get(languageChapters[languageChapters.length - 1].slug)!;
  assert.ok(chLinkTargets(closerFile).size >= 5, "expected the language closer to link many chapters");
});
