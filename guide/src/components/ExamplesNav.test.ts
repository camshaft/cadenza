/// Nav-invariant integrity for the Examples sidebar (ExamplesNav.tsx). The operator's directive is that
/// EVERY example is individually discoverable in the nav; ExamplesNav renders that from data — the three
/// EXAMPLES arrays plus playground's `theme` field — so a data drift can silently drop an example from the
/// sidebar with nothing else in the suite catching it. Three ways that happens:
///   1. a playground example's `theme` isn't one of the buckets ExamplesNav renders → it's filtered out and
///      vanishes from the nav (buildGroups only emits the PLAYGROUND_THEMES buckets);
///   2. two examples share a deep-link key (`id` for playground, `slug` for cad/notebook) → `?example=` is
///      ambiguous and the duplicate nav entry collides on its React key;
///   3. an empty key → the deep-link points at the surface with no example selected.
/// This pins "every example is reachable, uniquely, through the nav" so a future example/theme edit fails
/// here instead of shipping a silently-unreachable example. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { EXAMPLES as PLAYGROUND_EXAMPLES } from "../playground/examples.ts";
import { EXAMPLES as CAD_EXAMPLES } from "../cad/examples.ts";
import { EXAMPLES as NOTEBOOK_EXAMPLES } from "../notebook/examples.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/components

/// The playground theme buckets ExamplesNav actually RENDERS, derived from its source (the PLAYGROUND_THEMES
/// list) rather than hard-coded — so adding/removing a bucket in the nav tracks here automatically, and the
/// "no example dropped" check below is measured against what the nav really shows, not a stale copy.
function renderedThemes(): Set<string> {
  const src = readFileSync(join(here, "ExamplesNav.tsx"), "utf8");
  const themes = new Set<string>();
  // matches `{ theme: "basics", label: "Basics" },` entries in the PLAYGROUND_THEMES array
  for (const m of src.matchAll(/\{\s*theme:\s*"([^"]+)",\s*label:/g)) themes.add(m[1]);
  return themes;
}

test("every playground example's theme is a bucket ExamplesNav renders (none silently dropped from the nav)", () => {
  const rendered = renderedThemes();
  const orphaned = PLAYGROUND_EXAMPLES.filter((e) => !rendered.has(e.theme)).map(
    (e) => `${e.id} → theme "${e.theme}"`,
  );
  assert.equal(
    orphaned.length,
    0,
    `playground example(s) whose theme is not a rendered nav bucket (they'd vanish from the sidebar):\n  ${orphaned.join("\n  ")}\n  rendered buckets: ${[...rendered].join(", ")}`,
  );
});

test("playground example ids are unique and non-empty (deep-link keys don't collide)", () => {
  const seen = new Map<string, number>();
  for (const e of PLAYGROUND_EXAMPLES) seen.set(e.id, (seen.get(e.id) ?? 0) + 1);
  const dupes = [...seen.entries()].filter(([, n]) => n > 1).map(([id, n]) => `${id} ×${n}`);
  const empty = PLAYGROUND_EXAMPLES.filter((e) => !e.id || !e.id.trim()).map((e) => e.name);
  assert.equal(dupes.length, 0, `duplicate playground id(s) — ?example= would be ambiguous: ${dupes.join(", ")}`);
  assert.equal(empty.length, 0, `playground example(s) with an empty id: ${empty.join(", ")}`);
});

test("cad + notebook slugs are unique and non-empty (their deep-link keys)", () => {
  for (const [surface, arr] of [["cad", CAD_EXAMPLES], ["notebook", NOTEBOOK_EXAMPLES]] as const) {
    const seen = new Map<string, number>();
    for (const e of arr) seen.set(e.slug, (seen.get(e.slug) ?? 0) + 1);
    const dupes = [...seen.entries()].filter(([, n]) => n > 1).map(([s, n]) => `${s} ×${n}`);
    const empty = arr.filter((e) => !e.slug || !e.slug.trim()).map((e) => e.title);
    assert.equal(dupes.length, 0, `duplicate ${surface} slug(s): ${dupes.join(", ")}`);
    assert.equal(empty.length, 0, `${surface} example(s) with an empty slug: ${empty.join(", ")}`);
  }
});

test("the nav data scan found examples + buckets (guards against a vacuous pass)", () => {
  // A broken import or regex would make the invariants above pass on empty sets. Assert healthy counts so a
  // refactor that breaks the extraction (or empties an array) trips here instead of hiding a dropped example.
  assert.ok(PLAYGROUND_EXAMPLES.length >= 30, `expected many playground examples, found ${PLAYGROUND_EXAMPLES.length}`);
  assert.ok(CAD_EXAMPLES.length >= 5, `expected several cad examples, found ${CAD_EXAMPLES.length}`);
  assert.ok(NOTEBOOK_EXAMPLES.length >= 5, `expected several notebook examples, found ${NOTEBOOK_EXAMPLES.length}`);
  assert.equal(renderedThemes().size, 4, `expected 4 rendered playground theme buckets, found ${renderedThemes().size}`);
});
