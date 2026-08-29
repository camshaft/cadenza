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
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const repoRoot = join(guideRoot, "..");
const fixturesDir = join(guideRoot, "src/content/codegen/fixtures");

// ENGINE (cadenza-docs I5): render via the Rust xtask `xtask-codegen-guide` (the MAIN parser reads the .sexp
// → binary AST → TSX), retiring the node chapterModel.ts. `cargo build -p …` falls through the cargo-shim.
// Prefer the nix-provided prebuilt binary ($CDZ_XTASK_CODEGEN_GUIDE, v-nix standalone-derivation); else
// build via cargo for local dev (the gate has rust but no cargo vendor, so it uses the prebuilt binary).
let xtaskBin = process.env.CDZ_XTASK_CODEGEN_GUIDE;
if (!xtaskBin) {
  try {
    execFileSync("cargo", ["build", "-p", "xtask-codegen-guide", "--quiet"], { cwd: repoRoot, stdio: "inherit" });
  } catch (e) {
    console.error(`check-codegen: could not build xtask-codegen-guide — ${String(e.message || e).slice(0, 160)}`);
    process.exit(1);
  }
  xtaskBin = join(repoRoot, "target/debug/xtask-codegen-guide");
}
const renderSexp = (p) => execFileSync(xtaskBin, [p], { encoding: "utf8" });

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
  const tsx = renderSexp(join(fixturesDir, sexp));
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

// ---- CHAPTER-FIDELITY: each chapters/*.sexp must reproduce the still-hand-written chapters/<Name>.tsx ----
// This STAGES the sexp→TSX cutover on real content BEFORE flipping the source of truth: for every authored
// chapter .sexp, generate its TSX and compare NORMALIZED VISIBLE TEXT (not raw bytes) against the committed
// hand-written chapter. Byte-identity is the wrong bar — the hand file has incidental JSX layout (a link's
// inner text on its own indented line, {" "} spacers) that renders identically but differs byte-wise; what
// must match is the reader-visible DOM. So we strip tags/className/JSX-whitespace-exprs + collapse spaces on
// BOTH sides and require equality. When a chapter's .tsx is later REPLACED by the @generated output (the
// actual cutover), this check still holds (generated vs generated) and guards regressions. A .sexp with no
// sibling .tsx is skipped (a future generated-only chapter); today every .sexp shadows a hand file.
const chaptersSrcDir = join(guideRoot, "src/content/chapters");
const chapterSexps = readdirSync(chaptersSrcDir).filter((f) => f.endsWith(".sexp")).sort();
let fidelityFails = 0;
const visibleText = (tsx) => {
  const m = tsx.match(/<article>([\s\S]*)<\/article>/);
  if (!m) return "(no article)";
  return m[1]
    .replace(/className="[^"]*"/g, "")
    // Model JSX's whitespace collapsing, in the ORDER that matters:
    // 1. A whitespace run CONTAINING A NEWLINE adjacent to a tag (`>`/`<`) is REMOVED by JSX, not rendered as
    //    a space — so `<Link>\n  text\n</Link>, x` renders "text, x" (no stray pre-comma space), and
    //    `Cadenza\n<em>` renders "Cadenza<em>". Do this BEFORE expanding {" "} so an EXPLICIT `{" "}` space
    //    (which contains no newline) survives while incidental source-layout newlines are dropped. This is
    //    exactly why the author WROTE `{" "}` at those wrap points: to force a space JSX would otherwise eat.
    .replace(/>\s*\n\s*/g, ">")
    .replace(/\s*\n\s*</g, "<")
    // 2. Now the explicit author-written spaces/indents become literal text.
    .replace(/\{"([^"]*)"\}/g, "$1")
    .replace(/<br\s*\/?>/g, " ")     // a hard break renders as a line break; for VISIBLE-TEXT it's whitespace
    .replace(/<[^>]+>/g, "")         // strip all remaining JSX tags (Ch/AppLink/em/C/H1/…)
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
};
for (const sexp of chapterSexps) {
  const base = sexp.replace(/\.sexp$/, ""); // .sexp stem == PascalCase(slug) == the .tsx stem
  const handPath = join(chaptersSrcDir, `${base}.tsx`);
  let hand;
  try { hand = readFileSync(handPath, "utf8"); } catch { continue; } // generated-only chapter: nothing to compare
  const genVisible = visibleText(renderSexp(join(chaptersSrcDir, sexp)));
  const handVisible = visibleText(hand);
  if (genVisible !== handVisible) {
    fidelityFails++;
    let i = 0; while (i < genVisible.length && i < handVisible.length && genVisible[i] === handVisible[i]) i++;
    console.error(
      `check-codegen [chapter-fidelity]: ${sexp} does NOT reproduce ${base}.tsx's visible text.\n` +
        `  first divergence @${i}:\n    hand: …${JSON.stringify(handVisible.slice(Math.max(0, i - 40), i + 40))}\n` +
        `    gen:  …${JSON.stringify(genVisible.slice(Math.max(0, i - 40), i + 40))}`,
    );
  }
}
if (fidelityFails > 0) {
  console.error(`check-codegen [chapter-fidelity]: ${fidelityFails} chapter .sexp(s) don't reproduce their hand-written .tsx.`);
  process.exit(1);
}
if (chapterSexps.length) console.log(`check-codegen [chapter-fidelity]: ${chapterSexps.length} chapter .sexp(s) reproduce their hand-written .tsx (visible text) ✓`);
