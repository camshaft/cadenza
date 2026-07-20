/// Pure, dep-free scaffolding for /music's PRELOADED-library model buffer — NO React, NO worker/compiler
/// imports — so it is unit-testable under `node --test` (which strips types but can't load a `.tsx`
/// module). `MusicPage` imports these; the tests in `musicPreload.test.ts` pin the invariants. This is the
/// direct parallel of /cad's `preloadModel.ts` (the seam generalized to a 3rd preloaded-lib surface).
///
/// /music compiles a BARE showcase buffer against the preloaded music libs (`schedule`/`pitch`/
/// `interval-ratio`/`chord`/`piece`/…, staged from implementation/music/src) via `compileWithPreloaded`:
/// the reader edits only the model, the music vocabulary is link-merged. The host AUTO-INJECTS the import
/// clauses + an `export main` around the reader's buffer — this module is that injection.
///
/// v1 renders the EVENT-STRUCTURE story (schedule()→balanced() provably-no-stuck-keys), NOT Web Audio — so
/// the imported vocab is the MIDI/event surface (piece/schedule) + the rational-theory surface
/// (interval-ratio/chord/pitch) the three showcases (R1/R2/R3) use.

import type { Surface } from "../compiler/client.ts";

/// The preloaded music modules' names — the `import from "<name>"` link targets (bare filename, all authored
/// in ML `.cdz`). The AUTHORITATIVE staging list lives in `stage-wasm.mjs`'s `musicLibs` (v-music owns it);
/// this is the subset a SHOWCASE BUFFER imports from (the injected vocab). The full closure is preloaded (a
/// buffer's imported module must be preloaded, and those modules cross-import) — `MUSIC_PRELOAD_NAMES` below.
export const MUSIC_INTERVAL_NAME = "interval-ratio";
export const MUSIC_CHORD_NAME = "chord";
export const MUSIC_PITCH_NAME = "pitch";
export const MUSIC_PIECE_NAME = "piece";
export const MUSIC_SCHEDULE_NAME = "schedule";
export const MUSIC_LIB_FORMAT: Surface = "ml";

/// The full set of music modules PRELOADED for every showcase (names + their staged `.cdz` — the compiler
/// link-merges the closure). A buffer imports only what it needs; unused preloads are benign (as /cad proved).
/// This is the closure the showcases' imports transitively need — kept in lockstep with `stage-wasm.mjs`
/// `musicLibs` (v-music-authoritative; they ping v-guide-infra on a new importable lib). `synth.cdz` is
/// excluded (Web Audio, not an event-structure dep). Order is not significant to the linker.
export const MUSIC_PRELOAD_NAMES = [
  "schedule", "pitch", "interval-ratio", "scale-ratio", "scale",
  "chord-ratio", "chord", "rhythm", "rhythm-ratio", "compose", "piece",
] as const;

/// The names a /music showcase buffer imports, per authoritative module (v-music via v-guide-editor). These
/// are the SUPERSET across the three v1 showcases — a buffer only uses the ones it needs (an unused import is
/// benign, verified, like /cad's superset injection), so ALL clauses are injected and every showcase resolves:
///   - R1 (rational interval identity), `interval-ratio`: the RInterval algebra.
///   - R2 (chord→MIDI), `chord` + `pitch` + `interval-ratio`: build a triad from a MIDI pitch → List(Int64).
///   - R3 (piece end-to-end), `piece` + `schedule`: the I-V-vi-IV progression → balanced MIDI event stream.
export const MUSIC_INTERVAL_IMPORTS = [
  "RInterval", "rinterval", "octaves", "semitones-r", "r-octave",
  "r-perfect-fifth", "r-perfect-fourth", "r-complement", "r-eq", "r-add", "to-semitones-exact",
] as const;
export const MUSIC_CHORD_IMPORTS = ["major-triad", "chord-notes", "chord-stack"] as const;
export const MUSIC_PITCH_IMPORTS = ["pitch", "note"] as const;
export const MUSIC_PIECE_IMPORTS = ["progression"] as const;
export const MUSIC_SCHEDULE_IMPORTS = ["schedule", "balanced", "play-order"] as const;

/// Auto-inject the music `import` clauses + a trailing `export main` around the reader's showcase buffer
/// before compiling — so the buffer shows ONLY the model (mirrors /cad's ruling: no import boilerplate).
/// The reader's text is embedded VERBATIM and CONTIGUOUS so the linter's span-mapping (`wrapPrefixOf`) can
/// map a diagnostic's byte span back onto the editor buffer (the injected imports form a clean PREFIX):
///   - ML: the `import { … } from "<mod>"` lines, then the editor text, then a trailing `export { main }`.
///   - s-expr: the reader edits the INNER forms, wrapped in `(do (import "<mod>" (…)) … <editor> (export main))`.
/// (No default-fraction pragma — unlike /cad, music showcases don't lean on a bare-`n/d`→Rational default;
/// the rational-interval libs construct their own Rationals. Add one here only if a showcase needs it.)
export function injectImport(editorText: string, surface: Surface): string {
  const t = editorText.trim();
  const clauses: [string, readonly string[]][] = [
    [MUSIC_INTERVAL_NAME, MUSIC_INTERVAL_IMPORTS],
    [MUSIC_CHORD_NAME, MUSIC_CHORD_IMPORTS],
    [MUSIC_PITCH_NAME, MUSIC_PITCH_IMPORTS],
    [MUSIC_PIECE_NAME, MUSIC_PIECE_IMPORTS],
    [MUSIC_SCHEDULE_NAME, MUSIC_SCHEDULE_IMPORTS],
  ];
  if (surface === "sexpr") {
    // s-expr import spec is a bare name LIST (no commas): `(import "chord" (major-triad chord-notes …))`.
    const imports = clauses.map(([mod, names]) => `(import "${mod}" (${names.join(" ")}))`).join("\n");
    return `(do\n${imports}\n${t}\n(export main))`;
  }
  const imports = clauses.map(([mod, names]) => `import { ${names.join(", ")} } from "${mod}"`).join("\n");
  return `${imports}\n${t}\nexport { main }`;
}
