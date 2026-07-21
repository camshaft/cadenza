/// Unit tests for /music's preloaded-model injection (`musicPreload.ts`) — the scaffolding that wraps a bare
/// showcase buffer with the music `import { … } from "<mod>"` clauses + a trailing `export main` so it
/// compiles against the preloaded music libs. The reader's buffer is CLEAN (no import — auto-injected). The
/// load-bearing invariant is CONTIGUITY: the reader's verbatim text must appear as a single substring of the
/// injected output, or the linter's `wrapPrefixOf` span-mapping misplaces every squiggle. Mirrors
/// `cad/preloadModel.test.ts`. (The showcases' end-to-end compile against the staged libs is gated by
/// `check-music-preload.mjs`, which needs the staged wasm; these tests are pure + wasm-free.)

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import {
  injectImport,
  MUSIC_INTERVAL_NAME,
  MUSIC_CHORD_NAME,
  MUSIC_PITCH_NAME,
  MUSIC_PIECE_NAME,
  MUSIC_SCHEDULE_NAME,
  MUSIC_PATTERN_NAME,
  MUSIC_LIB_FORMAT,
  MUSIC_PRELOAD_NAMES,
  MUSIC_SCHEDULE_IMPORTS,
  MUSIC_PATTERN_IMPORTS,
} from "./musicPreload.ts";

// Clean showcase buffers — NO import (auto-injected; this is what the reader edits).
const ML_MODEL = `def main() = balanced(schedule(progression))`;
const SEXPR_MODEL = `(def (main) (balanced (schedule progression)))`;

test("ML: injects the interval-ratio/chord/pitch/piece/schedule import lines + a trailing export", () => {
  const out = injectImport(ML_MODEL, "ml");
  assert.ok(/^import \{ [^}]*\bRInterval\b[^}]* \} from "interval-ratio"\n/.test(out), "interval-ratio import is the first line");
  assert.ok(/\nimport \{ [^}]*\bmajor-triad\b[^}]* \} from "chord"\n/.test(out), "chord import follows");
  assert.ok(/\nimport \{ [^}]*\bpitch\b[^}]* \} from "pitch"\n/.test(out), "pitch import follows");
  assert.ok(/\nimport \{ [^}]*\bprogression\b[^}]* \} from "piece"\n/.test(out), "piece import follows");
  assert.ok(/\nimport \{ [^}]*\bbalanced\b[^}]*\bplay-order\b[^}]* \} from "schedule"\n/.test(out), "schedule import (incl balanced + play-order) follows");
  assert.ok(/\nimport \{ [^}]*\brender-pattern\b[^}]*\beuclid\b[^}]* \} from "pattern"\n/.test(out), "pattern import (render-pattern + euclid, R4) follows");
  assert.ok(out.trimEnd().endsWith("export { main }"), "export is appended");
});

test("s-expr: wraps the inner forms in (do (import interval-ratio) … (import schedule) … (export main))", () => {
  const out = injectImport(SEXPR_MODEL, "sexpr");
  assert.ok(/^\(do\n\(import "interval-ratio" \([^)]*\bRInterval\b[^)]*\)\)\n/.test(out), "opens with (do (import interval-ratio …))");
  assert.ok(/\(import "chord" \([^)]*\bmajor-triad\b[^)]*\)\)/.test(out), "includes (import chord …)");
  assert.ok(/\(import "pitch" \([^)]*\bpitch\b[^)]*\)\)/.test(out), "includes (import pitch …)");
  assert.ok(/\(import "piece" \([^)]*\bprogression\b[^)]*\)\)/.test(out), "includes (import piece …)");
  assert.ok(/\(import "schedule" \([^)]*\bbalanced\b[^)]*\bplay-order\b[^)]*\)\)/.test(out), "includes (import schedule … balanced play-order)");
  assert.ok(/\(import "pattern" \([^)]*\brender-pattern\b[^)]*\beuclid\b[^)]*\)\)/.test(out), "includes (import pattern render-pattern euclid) (R4)");
  assert.ok(out.trimEnd().endsWith("(export main))"), "closes with (export main))");
  // The s-expr import spec is a bare name LIST — no commas (commas are an ML-surface artifact).
  assert.ok(!/\(import "[a-z-]+" \([^)]*,/.test(out), "no commas in the s-expr import specs");
});

// CONTIGUITY — the invariant the linter's span-mapping depends on. Both surfaces.
test("ML: the reader's verbatim text is contiguous in the injected output", () => {
  assert.ok(injectImport(ML_MODEL, "ml").includes(ML_MODEL.trim()), "editor text embedded contiguously");
});
test("s-expr: the reader's verbatim text is contiguous in the injected output", () => {
  assert.ok(injectImport(SEXPR_MODEL, "sexpr").includes(SEXPR_MODEL.trim()), "editor text embedded contiguously");
});

test("trims surrounding whitespace before embedding (stable prefix length)", () => {
  const out = injectImport(`\n\n  ${ML_MODEL}  \n`, "ml");
  assert.ok(out.includes(ML_MODEL.trim()), "leading/trailing whitespace trimmed");
  assert.ok(!out.includes("\n\n\n"), "no stray blank runs from the raw padding");
});

test("the preloaded-library constants match what MusicPage passes the compiler", () => {
  assert.equal(MUSIC_INTERVAL_NAME, "interval-ratio");
  assert.equal(MUSIC_CHORD_NAME, "chord");
  assert.equal(MUSIC_PITCH_NAME, "pitch");
  assert.equal(MUSIC_PIECE_NAME, "piece");
  assert.equal(MUSIC_SCHEDULE_NAME, "schedule");
  assert.equal(MUSIC_LIB_FORMAT, "ml"); // the music/*.cdz libs are authored in ML
  // The preload closure must include the modules a showcase imports from (else CDZ0201 unknown-package) —
  // kept in lockstep with stage-wasm.mjs's musicLibs (v-music-authoritative). `synth` is excluded (Web Audio).
  for (const m of [MUSIC_INTERVAL_NAME, MUSIC_CHORD_NAME, MUSIC_PITCH_NAME, MUSIC_PIECE_NAME, MUSIC_SCHEDULE_NAME, MUSIC_PATTERN_NAME]) {
    assert.ok(MUSIC_PRELOAD_NAMES.includes(m), `preload closure includes the imported module '${m}'`);
  }
  assert.ok(!MUSIC_PRELOAD_NAMES.includes("synth"), "synth is NOT preloaded (Web Audio, not an event-structure dep)");
  // The event-stream correctness surface (balanced + play-order) is in the schedule import set — the R3 payoff.
  assert.ok(MUSIC_SCHEDULE_IMPORTS.includes("balanced") && MUSIC_SCHEDULE_IMPORTS.includes("play-order"), "schedule imports include balanced + play-order");
  // R4 (live-coding) imports render-pattern + euclid from `pattern`.
  assert.equal(MUSIC_PATTERN_NAME, "pattern");
  assert.ok(MUSIC_PATTERN_IMPORTS.includes("render-pattern") && MUSIC_PATTERN_IMPORTS.includes("euclid"), "pattern imports include render-pattern + euclid");
});

// The silent-failure trap (v-music, relayed by v-guide 2026-07-20): a module in MUSIC_PRELOAD_NAMES that is
// NOT staged by stage-wasm.mjs's `musicLibs` → check-music-preload throws "staged music lib missing", and in
// the browser the preload-import fails silently. The two lists live in different files (musicPreload.ts here,
// stage-wasm.mjs the staging script) so they can drift. Pin the containment invariant — every preloaded name
// must be staged — by parsing musicLibs out of the script and asserting MUSIC_PRELOAD_NAMES ⊆ musicLibs. This
// makes an unstaged preload name FAIL test:unit (in CI) instead of only surfacing at runtime. (We DON'T assert
// the reverse — musicLibs MAY stage extra libs a showcase doesn't preload yet, which is the intended
// lazy-per-showcase staging: staged-but-not-preloaded is benign; preloaded-but-not-staged is the bug.)
function stagedMusicLibs(): string[] {
  const here = dirname(fileURLToPath(import.meta.url)); // src/music
  const script = readFileSync(join(here, "../../scripts/stage-wasm.mjs"), "utf8");
  const m = script.match(/const musicLibs\s*=\s*\[([\s\S]*?)\]/);
  assert.ok(m, "could not find `const musicLibs = [...]` in stage-wasm.mjs (did the seam move?)");
  // Extract each "<name>.cdz" string literal, strip the .cdz to match MUSIC_PRELOAD_NAMES' bare names.
  return [...m![1].matchAll(/"([a-z0-9-]+)\.cdz"/g)].map((x) => x[1]);
}

test("every MUSIC_PRELOAD_NAMES entry is staged by stage-wasm.mjs musicLibs (no silent preload-import failure)", () => {
  const staged = stagedMusicLibs();
  assert.ok(staged.length >= 10, `expected the musicLibs parse to find many libs, got ${staged.length} — the regex may be stale`);
  for (const name of MUSIC_PRELOAD_NAMES) {
    assert.ok(staged.includes(name), `MUSIC_PRELOAD_NAMES has "${name}" but stage-wasm.mjs musicLibs does not stage "${name}.cdz" — it would fail to preload (add it to musicLibs)`);
  }
});

// The PRELOAD ARITY trap (OPERATOR bug 2026-07-20): compile_with_preloaded requires names/sources/formats to be
// EQUAL LENGTH. In MusicPage.tsx, PRELOAD_NAMES derives from MUSIC_PRELOAD_NAMES and FORMATS is mapped off NAMES
// (so those can't drift), but PRELOAD_SOURCES is a HAND-maintained list of `?raw` imports — one per module. When
// `pattern` was added to MUSIC_PRELOAD_NAMES without a matching `pattern.cdz?raw` import + SOURCES entry, names=12
// vs sources=11 → the whole /music page threw "names/sources/formats must be equal length". MusicPage.tsx has a
// runtime module-load guard now, but that only fires when the page is imported; this pins it in CI (test:unit) by
// parsing MusicPage.tsx and asserting every preloaded name has a matching `../wasm/music/<name>.cdz?raw` import.
function musicPageRawImports(): string[] {
  const here = dirname(fileURLToPath(import.meta.url)); // src/music
  const src = readFileSync(join(here, "MusicPage.tsx"), "utf8");
  // Each `import X from "../wasm/music/<name>.cdz?raw"` → capture <name> (bare, matches MUSIC_PRELOAD_NAMES).
  return [...src.matchAll(/from\s+"\.\.\/wasm\/music\/([a-z0-9-]+)\.cdz\?raw"/g)].map((m) => m[1]);
}

test("MusicPage PRELOAD_SOURCES has a ?raw import for every MUSIC_PRELOAD_NAMES entry (equal-length arity)", () => {
  const rawImports = musicPageRawImports();
  assert.ok(rawImports.length >= 10, `expected many ?raw music imports in MusicPage.tsx, found ${rawImports.length} — the regex may be stale`);
  // Every preloaded NAME must have a matching ?raw SOURCE import, else names.length !== sources.length and
  // compile_with_preloaded throws "must be equal length" — breaking the whole page (the pattern/R4 regression).
  for (const name of MUSIC_PRELOAD_NAMES) {
    assert.ok(
      rawImports.includes(name),
      `MUSIC_PRELOAD_NAMES has "${name}" but MusicPage.tsx has no matching \`../wasm/music/${name}.cdz?raw\` import ` +
        `→ PRELOAD_SOURCES would be short a slot → compile_with_preloaded "names/sources/formats must be equal length".`,
    );
  }
  // And no EXTRA source import without a preloaded name (the reverse arity drift — sources longer than names).
  for (const raw of rawImports) {
    assert.ok(
      (MUSIC_PRELOAD_NAMES as readonly string[]).includes(raw),
      `MusicPage.tsx imports "${raw}.cdz?raw" but "${raw}" is not in MUSIC_PRELOAD_NAMES → PRELOAD_SOURCES longer than PRELOAD_NAMES.`,
    );
  }
});
