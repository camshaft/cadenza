#!/usr/bin/env node
/// Lint the guide's PROSE for invented `@annotations`. The run-every-example harness (check-examples)
/// compiles + runs every `<Runnable>`/`<Exercise>`, but it does NOT validate API names mentioned in
/// PROSE — inline `<C>@foo</C>`, `<Note>` text, paragraphs. That's a real gap: the property-testing
/// chapter once shipped an invented "property" API precisely because the fake name lived in prose, not
/// a Runnable (the operator caught it by hand). This is the automated backstop for that class of bug,
/// scoped to the highest-signal, lowest-false-positive surface: `@annotations`.
///
/// WHY annotations specifically: a `@word` in guide text is UNAMBIGUOUSLY an annotation claim (unlike a
/// bare `<C>ans</C>` or `<C>5.0</C>`, which are values/fragments, not API names) — so checking each
/// against the set the compiler actually recognizes has near-zero false positives. A fake `@property`
/// would fail this gate. (A broader prose-identifier lint — Module.member, `cdz` subcommands — is a
/// bigger, more false-positive-prone design; annotations are the tight first increment.)
///
/// The ALLOWLIST is maintained here against the compiler's recognized annotations. If the compiler gains
/// a new annotation, add it here (a visible, one-line maintenance cost — the tradeoff for a sound lint).
/// Run: `npm run check:prose` (or wired into the gate). No deps, no wasm — pure text scan.

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// Annotations the compiler recognizes (rcdzc). Keep in sync with the compiler's annotation handling:
//   - @test / @exhaustive — the test runner (proptest_gen.rs); a parameterized @test is a property test.
//   - @tag("…")           — labels a test for `cdz test --tag …`.
//   - @inline-always / @inline-never — the inline policy (lower.rs).
// A `@word` in guide prose NOT in this set is almost certainly an invented API → fail.
const KNOWN_ANNOTATIONS = new Set([
  "test",
  "exhaustive",
  "tag",
  "inline-always",
  "inline-never",
  "requires",
  "ensures",
]);

const chaptersDir = join(guideRoot, "src/content/chapters");
const files = [
  ...readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).map((f) => join(chaptersDir, f)),
  join(guideRoot, "src/components/HomePage.tsx"),
];

const failures = [];
for (const file of files) {
  let src;
  try {
    src = readFileSync(file, "utf8");
  } catch {
    continue;
  }
  const rel = file.replace(guideRoot + "/", "");
  // Every `@word` occurrence (annotation-shaped: `@` + a kebab identifier). This catches prose in
  // `<C>@foo</C>`, `<Note>`, paragraphs — anywhere in the chapter source. A `@tag("slow")` matches
  // `@tag` (the annotation) and NOT `slow` (its string argument), which is correct.
  for (const m of src.matchAll(/@([a-z][a-z0-9-]*)/g)) {
    const name = m[1];
    if (!KNOWN_ANNOTATIONS.has(name)) {
      // Report with a little context so the author can find it.
      const at = m.index ?? 0;
      const ctx = src.slice(Math.max(0, at - 30), at + name.length + 20).replace(/\s+/g, " ");
      failures.push(`${rel}: unknown annotation @${name} — not a compiler annotation (…${ctx}…)`);
    }
  }
}

if (failures.length) {
  console.error(
    `\nprose-annotation lint: ${failures.length} unknown @annotation(s) in guide prose:\n` +
      failures.map((f) => "  ✗ " + f).join("\n") +
      `\n\nIf the compiler really gained one of these, add it to KNOWN_ANNOTATIONS in ` +
      `scripts/check-prose-annotations.mjs. Otherwise it's an invented API — fix the prose.`,
  );
  process.exit(1);
}
console.log(
  `✓ prose-annotation lint: every @annotation in the guide prose is a real compiler annotation ` +
    `(checked ${files.length} files against {${[...KNOWN_ANNOTATIONS].join(", ")}}).`,
);
