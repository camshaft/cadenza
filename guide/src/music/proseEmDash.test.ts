/// No-prose-em-dash invariant for the /music showcase EXAMPLE METADATA (companion to
/// `src/content/proseEmDash.test.ts`, which scans only the chapter .tsx files). The three /music
/// showcases render their `title` + `description` strings as user-facing PROSE alongside the picker,
/// but those strings live in `src/music/examples.ts`, OUTSIDE the chapter dir the chapter gate walks —
/// so the tone overhaul's zero-em-dash invariant was UNGUARDED there. That gap let three prose
/// em-dashes ship in the descriptions (caught only by an editorial re-read, then hand-rewritten in
/// `7e893ce1a`). This pins the rendered strings so a regression fails a test instead of slipping to
/// trunk.
///
/// SCOPE — only the RENDERED prose fields (`title`, `description`). The `///` doc-comments and the
/// Cadenza `source` bodies in examples.ts are code, not user-facing prose, so an em-dash there is fine
/// and NOT scanned (mirrors the chapter gate exempting <C>/template-literal/JSX-comment regions).
/// EN-dashes (U+2013) are correct typography and NOT flagged; only the em-dash (—, U+2014) is the
/// tone-overhaul target. We import the live EXAMPLES array so the gate pins the exact rendered values,
/// not a brittle re-parse of the source. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES } from "./examples.ts";

const EM_DASH = "—"; // U+2014, the prose-tone target. NOT the en-dash – (numeric ranges are fine).

test("no /music showcase title or description has a prose em-dash (subordinate with since/so/which)", () => {
  const violations: string[] = [];
  for (const ex of EXAMPLES) {
    for (const [field, text] of [
      ["title", ex.title],
      ["description", ex.description],
    ] as const) {
      if (text.includes(EM_DASH)) {
        violations.push(`${ex.slug}.${field}: ${text}`);
      }
    }
  }
  assert.equal(
    violations.length,
    0,
    `prose em-dash(es) in /music showcase metadata — rewrite as a flowing subordinated clause ` +
      `(", since …" / ", so …" / ", which …"). These strings render as user-facing prose on /music:\n  ` +
      violations.join("\n  "),
  );
});

test("the /music prose scan reads real showcases (guards a vacuous pass)", () => {
  // An empty EXAMPLES array would make the invariant pass on nothing. Assert there's content to scan.
  assert.ok(EXAMPLES.length >= 3, `expected the three v1 music showcases, got ${EXAMPLES.length}`);
  for (const ex of EXAMPLES) {
    assert.ok(ex.title.length > 0, `${ex.slug} has a non-empty title`);
    assert.ok(ex.description.length > 0, `${ex.slug} has a non-empty description`);
  }
  // And a prose em-dash IS caught (else the check above could be vacuous).
  assert.ok(`a prose clause ${EM_DASH} and more`.includes(EM_DASH), "an em-dash must be detectable");
});
