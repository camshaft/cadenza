/// No-prose-em-dash invariant for the /cad showcase EXAMPLE METADATA (companion to
/// `src/content/proseEmDash.test.ts`, which scans only the chapter .tsx files, and to
/// `src/music/proseEmDash.test.ts`, which does the same for the /music showcases). The /cad showcases
/// render their `title` + `description` strings as user-facing PROSE alongside the example picker, but
/// those strings live in `src/cad/examples.ts`, OUTSIDE the chapter dir the chapter gate walks — so the
/// tone overhaul's zero-em-dash invariant was UNGUARDED there. That gap let seven prose em-dashes ship in
/// the descriptions (caught by an editorial re-read, then hand-rewritten). This pins the rendered strings
/// so a regression fails a test instead of slipping to trunk.
///
/// SCOPE — only the RENDERED prose fields (`title`, `description`). The `///` doc-comments and the Cadenza
/// `source` bodies in examples.ts are code, not user-facing prose, so an em-dash there is fine and NOT
/// scanned (mirrors the chapter gate exempting <C>/template-literal/JSX-comment regions). EN-dashes
/// (U+2013) are correct typography and NOT flagged; only the em-dash (—, U+2014) is the tone target. We
/// import the live EXAMPLES array so the gate pins the exact rendered values, not a brittle re-parse of the
/// source. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES } from "./examples.ts";

const EM_DASH = "—"; // U+2014, the prose-tone target. NOT the en-dash – (numeric ranges are fine).

test("no /cad showcase title or description has a prose em-dash (subordinate with since/so/which)", () => {
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
    `prose em-dash(es) in /cad showcase metadata — rewrite as a flowing subordinated clause ` +
      `(", since …" / ", so …" / ", which …"). These strings render as user-facing prose on /cad:\n  ` +
      violations.join("\n  "),
  );
});

test("the /cad showcase scan actually reads examples (guards a vacuous pass)", () => {
  // A broken import or emptied array would make the invariant pass on nothing. Assert we see the showcases
  // and that a known one is present, and confirm the detector distinguishes em- from en-dash.
  assert.ok(EXAMPLES.length >= 5, `expected several /cad showcases, found ${EXAMPLES.length}`);
  assert.ok(
    EXAMPLES.every((e) => typeof e.title === "string" && typeof e.description === "string"),
    "every showcase must have a string title + description",
  );
  assert.ok("dent — here".includes(EM_DASH), "an em-dash must be detected by the scan");
  assert.ok(!"range 0–255".includes(EM_DASH), "an en-dash (numeric range) must NOT be flagged as an em-dash");
});
