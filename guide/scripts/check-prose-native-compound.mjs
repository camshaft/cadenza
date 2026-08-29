#!/usr/bin/env node
/// Lint the guide's PROSE for LEGACY name-head compound VALUE literals — `(list …)`, `(tuple …)`,
/// `(record …)`, `(set …)`, `(map …)` — that should now be written as the native `#`-forms
/// (`#list(…)`, `#tuple(…)`, `#record((= f v) …)`, `#map((= k v) …)`, `#set(…)`).
///
/// WHY this gate exists: the run-every-example harness (check-examples) compiles + runs every
/// `<Runnable>`/`<Exercise>` SOURCE, so a legacy compound literal in a source is caught the moment the
/// legacy recognizers are deleted (Phase-2) — it stops compiling. But PROSE is never executed, so a
/// reader-facing `<C>(list 1 2 3)</C>` teaching the OLD syntax would sit green forever. This is the
/// automated backstop for that class of regression (the M3 prose-nativization guard), the exact sibling
/// of check-prose-annotations.mjs. Run: `npm run check:prose-native`. No deps, no wasm — pure text scan.
///
/// SCOPE: PROSE only. Runnable/Exercise SOURCE template literals (`source=`/`solution=`/`starter=`/
/// `source:` + `expected=`) are v-guide-infra's lane and are stripped before scanning. NOT flagged:
///   - native `#list(…)`/`#tuple(…)`/… — the `(` follows the head, so `\((head` never matches them.
///   - ML-surface reader forms `[a, b]` / `(a, b)` / `{x = 1}` / `#{k = v}` / `#(e)` — different shape.
///   - `(Tuple …)`/`(Record …)`/`(List …)`/`(Set …)`/`(Map …)` TYPE constructors — capitalized.
///   - `(fn …)` lambdas — `fn` is not a compound head.
///   - `{1, 2, 3}` mathematical set notation used pedagogically — not a Cadenza name-head literal.
/// KNOWN false-positive shape: a bare `(map f xs)` HOF call written in PROSE would match `(map `. The
/// guide uses `List.map`, not a bare `map` HOF, so this does not occur today; if it ever does, rewrite
/// the prose to `List.map` or wrap the HOF differently — do NOT weaken this lint to allow `(map …)`.

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

const chaptersDir = join(guideRoot, "src/content/chapters");
const files = [
  ...readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).map((f) => join(chaptersDir, f)),
  join(guideRoot, "src/components/HomePage.tsx"),
];

// Vacuous-pass floor: a broken chapter glob must FAIL, not pass on nothing (mirrors check-prose-annotations).
if (files.length < 30) {
  console.error(
    `check:prose-native: expected ≥30 content files (chapters + HomePage), found ${files.length} — ` +
      `the chapter glob likely broke; refusing a vacuous pass.`,
  );
  process.exit(1);
}

// A `(` immediately followed by a lowercase compound head + a space or `)`. Native `#head(` never
// matches (the `(` follows the head); capitalized TYPE ctors never match (head is lowercase-anchored).
const LEGACY_HEAD = /\((list|tuple|record|set|map)(?=[\s)])/g;
// Native `#`-forms in prose — the machinery witness (the guide demonstrably teaches these post-M3).
const NATIVE_HEAD = /#(list|tuple|record|set|map)\(/g;

const failures = [];
let nativeSeen = 0;
for (const file of files) {
  let src;
  try {
    src = readFileSync(file, "utf8");
  } catch (e) {
    console.error(`check:prose-native: could not read ${file} — ${String(e && e.message ? e.message : e)}`);
    process.exit(1);
  }
  const rel = file.replace(guideRoot + "/", "");
  // Strip Runnable/Exercise SOURCE template literals + expected-output assertions (v-guide-infra's lane,
  // and test data — NOT prose). A JS template literal is delimited by backticks and cannot contain an
  // unescaped backtick, so `[\s\S]*?` to the next backtick captures each block (single- OR multi-line).
  const prose = src.replace(/(source|solution|starter|expected)\s*[:=]\s*\{?`[\s\S]*?`/g, "$1=`«source»`");

  for (const m of prose.matchAll(NATIVE_HEAD)) nativeSeen++;

  for (const m of prose.matchAll(LEGACY_HEAD)) {
    const at = m.index ?? 0;
    const ctx = prose.slice(Math.max(0, at - 25), at + 35).replace(/\s+/g, " ");
    failures.push(`${rel}: legacy compound literal (${m[1]} …) in prose — write the native #${m[1]}(…) form (…${ctx}…)`);
  }
}

if (failures.length) {
  console.error(
    `\nprose-native-compound lint: ${failures.length} legacy name-head compound literal(s) in guide PROSE:\n` +
      failures.map((f) => "  ✗ " + f).join("\n") +
      `\n\nGuide teaching prose must use the native #-forms (M3): (list …)→#list(…), (tuple …)→#tuple(…), ` +
      `(record (= f v) …)→#record((= f v) …), (map …)→#map((= k v) …), (set …)→#set(…). ` +
      `Runnable/Exercise SOURCES are stripped before scanning, so this is a PROSE fix.`,
  );
  process.exit(1);
}
// Machinery witness: the guide demonstrably teaches native #-forms in prose (Lists/RecordsTuples/…), so a
// scan finding ZERO means the read/strip silently broke — refuse a vacuous pass.
if (nativeSeen === 0) {
  console.error(
    `check:prose-native: scanned ${files.length} files but matched ZERO native #-forms in prose — the ` +
      `scan likely broke (the guide teaches #list(…)/#tuple(…)/…); refusing a vacuous pass.`,
  );
  process.exit(1);
}
console.log(
  `✓ prose-native-compound lint: no legacy name-head compound literals in guide prose ` +
    `(saw ${nativeSeen} native #-form reference(s) across ${files.length} files).`,
);
