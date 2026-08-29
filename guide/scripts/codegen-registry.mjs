/// Generate the chapter registry entries (chapters.ts CHAPTERS[]) from the .sexp set + the editorial order
/// (v-guide-editor's #1 ask — kill the title/blurb/section/pillar duplication that could drift). The reading
/// ORDER comes from `chapter-order.mjs` (the sole editorial order lever); every other field is DERIVED from
/// each chapter's `<stem>.sexp`:
///   slug = (slug …)                       title = (nav-title …) ?? (title …)   [sidebar label]
///   blurb = (blurb …)                     pillar = (pillar …), emitted only when "platform"
///   section = (section …)                 exercises = count of (exercise …) blocks, emitted only when > 0
///   Component = lazy(() => import("./chapters/<Stem>.tsx"))
///
/// It replaces ONLY the region between the `// <generated:chapters>` … `// </generated:chapters>` markers in
/// chapters.ts; the hand-written parts (types, PILLARS, pillarOf, NON_TEACHING_SECTIONS, chapterAt) are left
/// untouched. `check-registry-derive.mjs` is the drift-gate that this generation makes trivially hold.
///
/// MODES: default = rewrite the region in place; `--check` = diff in memory, non-zero exit on drift (CI gate).
/// Run: `node scripts/codegen-registry.mjs [--check]` (plain .mjs; no .ts type-strip needed).
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTER_ORDER } from "../src/content/chapter-order.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const contentDir = join(here, "..", "src", "content");
const chaptersDir = join(contentDir, "chapters");
const registryPath = join(contentDir, "chapters.ts");

const BEGIN = "  // <generated:chapters> — DO NOT EDIT; `npm run codegen:registry` (from chapter-order.mjs + each chapter's .sexp)";
const END = "  // </generated:chapters>";

const sx = (text, head) => {
  const m = text.match(new RegExp(`\\(${head}\\s+"((?:[^"\\\\]|\\\\.)*)"\\)`));
  return m ? m[1] : null;
};

// Derive one chapter's registry entry TEXT from its .sexp, matching the hand-written entry format exactly.
function entry(stem) {
  const text = readFileSync(join(chaptersDir, `${stem}.sexp`), "utf8");
  const slug = sx(text, "slug");
  if (!slug) throw new Error(`${stem}.sexp: no (slug …)`);
  const title = sx(text, "nav-title") ?? sx(text, "title");
  const blurb = sx(text, "blurb");
  const section = sx(text, "section");
  const pillar = sx(text, "pillar"); // emit only when platform
  const exercises = (text.match(/\(exercise\b/g) ?? []).length; // emit only when > 0
  const lines = [
    "  {",
    `    slug: ${JSON.stringify(slug)},`,
    `    title: ${JSON.stringify(title)},`,
    `    blurb: ${JSON.stringify(blurb)},`,
  ];
  if (pillar && pillar !== "language") lines.push(`    pillar: ${JSON.stringify(pillar)},`);
  lines.push(`    section: ${JSON.stringify(section)},`);
  if (exercises > 0) lines.push(`    exercises: ${exercises},`);
  lines.push(`    Component: lazy(() => import(${JSON.stringify(`./chapters/${stem}.tsx`)})),`);
  lines.push("  },");
  return lines.join("\n");
}

const generated = CHAPTER_ORDER.map(entry).join("\n");
const block = `${BEGIN}\n${generated}\n${END}`;

const src = readFileSync(registryPath, "utf8");
const bi = src.indexOf(BEGIN);
const ei = src.indexOf(END);
if (bi < 0 || ei < 0 || ei < bi) {
  console.error(`codegen-registry: could not find the generated-region markers in chapters.ts — expected:\n${BEGIN}\n…\n${END}`);
  process.exit(1);
}
const next = src.slice(0, bi) + block + src.slice(ei + END.length);

const CHECK = process.argv.includes("--check");
if (CHECK) {
  if (next !== src) {
    console.error("✗ codegen-registry --check: chapters.ts CHAPTERS[] is OUT OF SYNC with chapter-order.mjs + the .sexp — run `npm run codegen:registry` and commit.");
    process.exit(1);
  }
  console.log(`✓ codegen-registry --check: chapters.ts CHAPTERS[] (${CHAPTER_ORDER.length}) is in sync with the .sexp.`);
} else {
  writeFileSync(registryPath, next);
  console.log(`✓ codegen-registry: regenerated ${CHAPTER_ORDER.length} chapter entries in chapters.ts.`);
}
