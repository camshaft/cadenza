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
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const repoRoot = join(guideRoot, "..");
const chaptersDir = join(guideRoot, "src/content/chapters");

// ENGINE (cadenza-docs I5): the guide sexp→TSX codegen is the Rust xtask `xtask-codegen-guide` — the MAIN
// cadenza-syntax-sexpr parser reads each chapter .sexp into the binary AST + emits the TSX (operator: one
// parser, no rust+node duplication; the node chapterModel.ts is retired as the engine). Build it here via
// `cargo build -p …` (the `-p` form falls through the all-nix cargo-shim to real cargo) — the guide-examples
// nix derivation already has cargo (it runs `cargo xtask guide-wasm`), so this works in the gate too, PROVIDED
// the crate is in that derivation's src fileset (v-nix).
// Binary resolution: the build-time nix derivation provides a PREBUILT xtask via $CDZ_XTASK_CODEGEN_GUIDE
// (v-nix's standalone-derivation, fork-2 — the guide-examples gate has no cargo vendor, so it can't build
// here). Absent the env (local dev), build it via `cargo build -p …` (the `-p` form falls through the shim).
let xtaskBin = process.env.CDZ_XTASK_CODEGEN_GUIDE;
if (!xtaskBin) {
  try {
    execFileSync("cargo", ["build", "-p", "xtask-codegen-guide", "--quiet"], { cwd: repoRoot, stdio: "inherit" });
  } catch (e) {
    console.error(`codegen-chapters: could not build xtask-codegen-guide — ${String(e.message || e).slice(0, 160)}`);
    process.exit(1);
  }
  xtaskBin = join(repoRoot, "target/debug/xtask-codegen-guide");
}

const MODE = process.argv.includes("--check") ? "check" : "write";
const sexpFiles = readdirSync(chaptersDir).filter((f) => f.endsWith(".sexp")).sort();

const FLOOR = 1;
if (sexpFiles.length < FLOOR) {
  console.error(`codegen-chapters: found ${sexpFiles.length} .sexp in ${chaptersDir}, expected ≥ ${FLOOR}.`);
  console.error("A codegen step that processes zero chapters is a broken glob / moved dir, not a green pass.");
  process.exit(1);
}

let drift = 0;
for (const sexp of sexpFiles) {
  const sexpPath = join(chaptersDir, sexp);
  let tsx;
  try {
    tsx = execFileSync(xtaskBin, [sexpPath], { encoding: "utf8" });
  } catch (e) {
    console.error(`codegen-chapters: ${sexp} — xtask render failed: ${String(e.message || e).slice(0, 200)}`);
    process.exit(1);
  }
  // The .sexp stem is PascalCase(slug) == the .tsx stem (chapters.ts imports resolve unchanged).
  const outName = sexp.replace(/\.sexp$/, ".tsx");
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
