/// The guide sexp→TSX codegen GATE (cadenza-docs I4). Proves the codegen mechanism end-to-end on committed
/// FIXTURES before any real chapter is cut over: for each `src/content/codegen/fixtures/<name>.sexp`, it
/// regenerates the TSX via the pure core (`src/content/codegen/chapterModel.ts`) and diffs against the
/// committed `<name>.expected.tsx`. This gates two invariants at once:
///   (1) DETERMINISM — the same `.sexp` always renders byte-identical TSX (a non-deterministic renderer, or
///       an accidental change to the core, drifts the expected file and fails here).
///   (2) SCHEMA FIDELITY — the expected TSX is hand-checked to render correctly, so a regression in the
///       parse/render logic (a dropped head, a mangled link, a lost indent) trips this gate.
///
/// WHY fixtures, not the live chapters (yet): converting a published chapter's `.tsx` to a `.sexp` source of
/// truth must be byte-faithful to the reader-visible content — that's a per-chapter reviewed cutover (with
/// v-guide confirming fidelity), sequenced after this mechanism lands. This gate proves the SEAM works so
/// the cutover increments build on a tested, gated codegen path. The `--write` mode regenerates the expected
/// files (run when a fixture's `.sexp` legitimately changes).
///
/// VACUOUS-PASS FLOOR (v-guide-infra discipline): ZERO fixtures discovered ⇒ FAIL loudly. A codegen gate
/// that silently checks nothing (moved dir / broken glob) must not read green. Floor = 1.

import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const fixturesDir = join(guideRoot, "src/content/codegen/fixtures");

let parseChapter, renderChapter;
try {
  ({ parseChapter, renderChapter } = await import(join(guideRoot, "src/content/codegen/chapterModel.ts")));
} catch (e) {
  console.error("check-codegen: could not load the codegen core (need Node ≥22 for --experimental-strip-types).");
  console.error(String(e));
  process.exit(1);
}

const WRITE = process.argv.includes("--write");
const sexpFiles = readdirSync(fixturesDir).filter((f) => f.endsWith(".sexp")).sort();

const FLOOR = 1;
if (sexpFiles.length < FLOOR) {
  console.error(`check-codegen: found ${sexpFiles.length} fixture .sexp in ${fixturesDir}, expected ≥ ${FLOOR}.`);
  console.error("A codegen gate that checks zero fixtures is a broken glob / moved dir, not a green pass.");
  process.exit(1);
}

let drift = 0;
for (const sexp of sexpFiles) {
  const base = sexp.replace(/\.sexp$/, "");
  const src = readFileSync(join(fixturesDir, sexp), "utf8");
  const parsed = parseChapter(src);
  if (!parsed.ok) {
    console.error(`check-codegen: ${sexp} — parse declined: ${parsed.reason}`);
    process.exit(1);
  }
  const tsx = renderChapter(parsed.model);
  // The golden file is `.expected.tsx.txt`, NOT `.tsx`: it's the expected OUTPUT TEXT the codegen emits for
  // a chapter at `src/content/chapters/<Name>.tsx` (its `../../components/Prose.tsx` import is relative to
  // THAT dir). Kept as `.txt` so `tsc -b` doesn't try to compile the fixture as a live module from the
  // wrong directory — the gate byte-compares the string, it isn't a chapter that ships.
  const expectedPath = join(fixturesDir, `${base}.expected.tsx.txt`);

  if (WRITE) {
    writeFileSync(expectedPath, tsx);
    console.log(`check-codegen --write: ${sexp} → ${base}.expected.tsx.txt`);
    continue;
  }

  let expected = null;
  try { expected = readFileSync(expectedPath, "utf8"); } catch { /* missing → drift */ }
  if (expected !== tsx) {
    drift++;
    console.error(`check-codegen: ${base}.expected.tsx.txt is OUT OF SYNC with ${sexp} — run \`node scripts/check-codegen.mjs --write\` and commit.`);
  }
}

if (WRITE) {
  console.log(`check-codegen --write: regenerated ${sexpFiles.length} fixture(s) ✓`);
} else if (drift > 0) {
  console.error(`check-codegen: ${drift} fixture(s) out of sync.`);
  process.exit(1);
} else {
  console.log(`check-codegen: ${sexpFiles.length} fixture(s) render byte-identically ✓`);
}
