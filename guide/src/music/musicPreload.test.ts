/// Unit tests for /music's preloaded-model injection (`musicPreload.ts`) — the scaffolding that wraps a bare
/// showcase buffer with the music `import { … } from "<mod>"` clauses + a trailing `export main` so it
/// compiles against the preloaded music libs. The reader's buffer is CLEAN (no import — auto-injected). The
/// load-bearing invariant is CONTIGUITY: the reader's verbatim text must appear as a single substring of the
/// injected output, or the linter's `wrapPrefixOf` span-mapping misplaces every squiggle. Mirrors
/// `cad/preloadModel.test.ts`. (The showcases' end-to-end compile against the staged libs is gated by
/// `check-music-preload.mjs`, which needs the staged wasm; these tests are pure + wasm-free.)

import test from "node:test";
import assert from "node:assert/strict";
import {
  injectImport,
  MUSIC_INTERVAL_NAME,
  MUSIC_CHORD_NAME,
  MUSIC_PITCH_NAME,
  MUSIC_PIECE_NAME,
  MUSIC_SCHEDULE_NAME,
  MUSIC_LIB_FORMAT,
  MUSIC_PRELOAD_NAMES,
  MUSIC_SCHEDULE_IMPORTS,
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
  assert.ok(out.trimEnd().endsWith("export { main }"), "export is appended");
});

test("s-expr: wraps the inner forms in (do (import interval-ratio) … (import schedule) … (export main))", () => {
  const out = injectImport(SEXPR_MODEL, "sexpr");
  assert.ok(/^\(do\n\(import "interval-ratio" \([^)]*\bRInterval\b[^)]*\)\)\n/.test(out), "opens with (do (import interval-ratio …))");
  assert.ok(/\(import "chord" \([^)]*\bmajor-triad\b[^)]*\)\)/.test(out), "includes (import chord …)");
  assert.ok(/\(import "pitch" \([^)]*\bpitch\b[^)]*\)\)/.test(out), "includes (import pitch …)");
  assert.ok(/\(import "piece" \([^)]*\bprogression\b[^)]*\)\)/.test(out), "includes (import piece …)");
  assert.ok(/\(import "schedule" \([^)]*\bbalanced\b[^)]*\bplay-order\b[^)]*\)\)/.test(out), "includes (import schedule … balanced play-order)");
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
  for (const m of [MUSIC_INTERVAL_NAME, MUSIC_CHORD_NAME, MUSIC_PITCH_NAME, MUSIC_PIECE_NAME, MUSIC_SCHEDULE_NAME]) {
    assert.ok(MUSIC_PRELOAD_NAMES.includes(m), `preload closure includes the imported module '${m}'`);
  }
  assert.ok(!MUSIC_PRELOAD_NAMES.includes("synth"), "synth is NOT preloaded (Web Audio, not an event-structure dep)");
  // The event-stream correctness surface (balanced + play-order) is in the schedule import set — the R3 payoff.
  assert.ok(MUSIC_SCHEDULE_IMPORTS.includes("balanced") && MUSIC_SCHEDULE_IMPORTS.includes("play-order"), "schedule imports include balanced + play-order");
});
