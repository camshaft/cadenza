/// The guide sexp→TSX codegen BUILD STEP (cadenza-docs I4, step F — the source-of-truth flip). For every
/// `src/content/chapters/*.sexp`, generate the sibling `<Pascal>.tsx` chapter module via the pure codegen
/// core (chapterModel.ts). Runs BEFORE `tsc -b` in the build so the generated TSX is type-checked
/// (design §4.4: codegen-before-tsc). This is the thin FILESYSTEM wrapper; all schema/render logic is in the
/// node-tested core.
///
/// MODES:
///   (default / `--write`): regenerate each chapter `.tsx` from its `.sexp`. The generated file carries an
///     `@generated` header — the `.sexp` is now the SOURCE OF TRUTH; hand-edits to the `.tsx` are overwritten.
///   `--check`: regenerate in memory and DIFF against the committed `.tsx`; exit non-zero on any drift. This
///     is the CI gate — proves the committed `.tsx` is in sync with its `.sexp` (nobody hand-edited a
///     generated chapter, and the codegen is deterministic).
///
/// The `check-codegen.mjs` chapter-fidelity gate (separate) proves the generated visible-text DOM matches the
/// pre-flip hand-written chapter; THIS script keeps the committed `.tsx` byte-in-sync with the `.sexp` going
/// forward. Registry: chapters.ts still hand-lists each chapter's metadata + `lazy(import(...))`; the
/// generated file NAME/slug is stable (Pascal-cased slug) so those imports resolve unchanged. (Deriving
/// chapters.ts from the .sexp set is a later step; the pilot keeps the hand registry, whose entry the
/// fidelity check confirms matches the .sexp attrs.)
///
/// VACUOUS-PASS FLOOR: zero `.sexp` discovered ⇒ FAIL loudly (a codegen step that writes nothing is a broken
/// glob, not a pass). Floor = 1 (the PlatformOverview pilot); raise as chapters convert.

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const chaptersDir = join(guideRoot, "src/content/chapters");

let parseChapter, renderChapter;
try {
  ({ parseChapter, renderChapter } = await import(join(guideRoot, "src/content/codegen/chapterModel.ts")));
} catch (e) {
  console.error("codegen-chapters: could not load the codegen core (need Node ≥22 for --experimental-strip-types).");
  console.error(String(e));
  process.exit(1);
}

const MODE = process.argv.includes("--check") ? "check" : "write";
const sexpFiles = readdirSync(chaptersDir).filter((f) => f.endsWith(".sexp")).sort();

const FLOOR = 1;
if (sexpFiles.length < FLOOR) {
  console.error(`codegen-chapters: found ${sexpFiles.length} .sexp in ${chaptersDir}, expected ≥ ${FLOOR}.`);
  console.error("A codegen step that processes zero chapters is a broken glob / moved dir, not a green pass.");
  process.exit(1);
}

/// The generated .tsx file name for a chapter model: PascalCase slug + .tsx (matches chapters.ts imports).
function tsxNameFor(model) {
  const pascal = model.slug.split(/[-_]/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join("");
  return `${pascal}.tsx`;
}

let drift = 0;
for (const sexp of sexpFiles) {
  const src = readFileSync(join(chaptersDir, sexp), "utf8");
  const parsed = parseChapter(src);
  if (!parsed.ok) {
    console.error(`codegen-chapters: ${sexp} — parse declined: ${parsed.reason}`);
    process.exit(1);
  }
  const tsx = renderChapter(parsed.model);
  const outName = tsxNameFor(parsed.model);
  const outPath = join(chaptersDir, outName);

  if (MODE === "check") {
    let existing = null;
    try { existing = readFileSync(outPath, "utf8"); } catch { /* missing → drift */ }
    if (existing !== tsx) {
      drift++;
      console.error(`codegen-chapters --check: ${outName} is OUT OF SYNC with ${sexp} — run \`npm run codegen\` and commit.`);
    }
  } else {
    writeFileSync(outPath, tsx);
    console.log(`codegen-chapters: ${sexp} → ${outName}`);
  }
}

if (MODE === "check") {
  if (drift > 0) {
    console.error(`codegen-chapters --check: ${drift} generated chapter(s) out of sync.`);
    process.exit(1);
  }
  console.log(`codegen-chapters --check: ${sexpFiles.length} generated chapter(s) in sync ✓`);
} else {
  console.log(`codegen-chapters: generated ${sexpFiles.length} chapter(s) ✓`);
}
