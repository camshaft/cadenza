/// FEASIBILITY VERIFIER for deriving the chapter registry (chapters.ts CHAPTERS[]) from the .sexp set
/// (v-guide-editor's standing #1 ask; the metadata title/blurb/pillar/section currently lives in BOTH the
/// .sexp AND chapters.ts, which can drift). This does NOT change chapters.ts — it proves that every registry
/// field is DERIVABLE from the sibling .sexp, so a later codegen can make chapters.ts @generated with an
/// ordered file-stem list as the sole editorial ORDER source and everything else read from the .sexp.
///
/// For each CHAPTERS[] entry it reads the entry's Component file stem (slug ≠ PascalCase in general — e.g.
/// `contracts` → DesignByContract), loads `<stem>.sexp`, and compares the DERIVED fields to the committed
/// registry entry:
///   slug      = (slug …)                              title = (nav-title …) ?? (title …)   [sidebar label]
///   blurb     = (blurb …)                             pillar = (pillar …), omitted when "language"
///   section   = (section …)                           exercises = count of (exercise …) blocks, omitted when 0
/// A mismatch is a real drift (the hand registry disagrees with the .sexp) — reported, non-zero exit.
///
/// Run: `node scripts/check-registry-derive.mjs` (node ≥ 22.6 not required — plain .mjs, no .ts import).
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const contentDir = join(here, "..", "src", "content");
const chaptersDir = join(contentDir, "chapters");
const registry = readFileSync(join(contentDir, "chapters.ts"), "utf8");

// ---- parse the committed CHAPTERS[] entries (in order) from chapters.ts ----
// Each entry spans from a `slug:` line to the `Component: lazy(() => import("./chapters/<Stem>.tsx"))`.
// An entry: `{`, then any number of leading `//` comment lines (e.g. the using-the-playground slug note),
// then `slug:` … through the `Component: lazy(() => import("./chapters/<Stem>.tsx"))`.
const entryRe =
  /\{\s*(?:\/\/[^\n]*\n\s*)*slug:\s*"([^"]+)",[\s\S]*?Component:\s*lazy\(\(\)\s*=>\s*import\("\.\/chapters\/([A-Za-z0-9]+)\.tsx"\)\),\s*\}/g;
const field = (block, name) => {
  const m = block.match(new RegExp(`\\b${name}:\\s*"((?:[^"\\\\]|\\\\.)*)"`));
  return m ? m[1] : null;
};
const numField = (block, name) => {
  const m = block.match(new RegExp(`\\b${name}:\\s*(\\d+)`));
  return m ? Number(m[1]) : null;
};

const committed = [];
for (const m of registry.matchAll(entryRe)) {
  const block = m[0];
  committed.push({
    slug: m[1],
    stem: m[2],
    title: field(block, "title"),
    blurb: field(block, "blurb"),
    pillar: field(block, "pillar"), // null when omitted (= "language")
    section: field(block, "section"),
    exercises: numField(block, "exercises") ?? 0, // omitted ⇒ 0
  });
}

// ---- derive the same fields from each entry's .sexp ----
const sx = (text, head) => {
  // A single-line `(head "value")` metadata form. Values are simple (no escaped quotes in slug/pillar/section);
  // blurb/title may contain apostrophes but not embedded double-quotes, matching the committed registry.
  const m = text.match(new RegExp(`\\(${head}\\s+"((?:[^"\\\\]|\\\\.)*)"\\)`));
  return m ? m[1] : null;
};

let mism = 0;
const cmp = (slug, f, got, want) => {
  if (got !== want) {
    mism++;
    console.error(`  ✗ ${slug}.${f}: registry ${JSON.stringify(want)} ≠ derived ${JSON.stringify(got)}`);
  }
};

for (const c of committed) {
  let text;
  try {
    text = readFileSync(join(chaptersDir, `${c.stem}.sexp`), "utf8");
  } catch {
    mism++;
    console.error(`  ✗ ${c.slug}: no ${c.stem}.sexp for the committed Component import`);
    continue;
  }
  const derivedTitle = sx(text, "nav-title") ?? sx(text, "title");
  const derivedPillar = sx(text, "pillar");
  cmp(c.slug, "slug", sx(text, "slug"), c.slug);
  cmp(c.slug, "title", derivedTitle, c.title);
  cmp(c.slug, "blurb", sx(text, "blurb"), c.blurb);
  cmp(c.slug, "section", sx(text, "section"), c.section);
  // pillar: registry omits "language" (null); the .sexp records it explicitly → normalize both to the effective value.
  cmp(c.slug, "pillar", derivedPillar ?? "language", c.pillar ?? "language");
  // exercises: count (exercise …) blocks; registry omits 0.
  const derivedEx = (text.match(/\(exercise\b/g) ?? []).length;
  cmp(c.slug, "exercises", derivedEx, c.exercises);
}

// VACUOUS-PASS GUARD: parsed entries must equal the number of chapter .sexp files. A too-loose floor would
// let a broken regex silently skip an entry (e.g. one with extra comment lines) and still "pass".
const sexpCount = readdirSync(chaptersDir).filter((f) => f.endsWith(".sexp")).length;
if (committed.length !== sexpCount) {
  console.error(
    `✗ check-registry-derive: parsed ${committed.length} CHAPTERS entries but there are ${sexpCount} chapter .sexp — the registry regex missed an entry (or a chapter has no registry entry).`,
  );
  process.exit(1);
}
if (mism > 0) {
  console.error(`\n✗ check-registry-derive: ${mism} field(s) where the hand registry disagrees with the .sexp.`);
  process.exit(1);
}
console.log(`✓ check-registry-derive: all ${committed.length} chapters' registry fields (title/blurb/pillar/section/exercises) are DERIVABLE from their .sexp — the hand registry is fully reproducible.`);
