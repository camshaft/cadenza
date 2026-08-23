// Stage the built compiler wasm and the value-heap runtime into the guide's source tree.
//
// Run after `wasm-pack build --target web` in ../implementation/seed/crates/cdz-wasm. Copies:
//   - the wasm-pack `pkg/` (JS glue + cdz_wasm_bg.wasm) -> src/wasm/pkg/
//   - the value-heap runtime component whose content address == cdz_wasm's `required_runtime_hash()`
//     (a base62 `Hash::of(Blob, bytes)`, 45 chars — design §8), found
//     in the cadenza store, -> src/wasm/runtime.wasm  (the guide bundles exactly the runtime the
//     compiler pins, so a compound program's `cadenza:runtime/heap@0.0.0+<hash>` import resolves).
//
// Keeping these in src/ (not public/) lets Vite fingerprint + serve them as hashed assets and lets
// the workers `?url`-import them. The staged files are git-ignored; `npm run wasm` regenerates them.

import { readFile, writeFile, mkdir, cp, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guide = join(here, "..");
const crate = join(guide, "..", "implementation", "seed", "crates", "cdz-wasm");
const pkg = join(crate, "pkg");
const dest = join(guide, "src", "wasm");

if (!existsSync(pkg)) {
  console.error(`[stage-wasm] no pkg/ at ${pkg} — run \`wasm-pack build --target web\` first.`);
  process.exit(1);
}

await mkdir(join(dest, "pkg"), { recursive: true });
await cp(pkg, join(dest, "pkg"), { recursive: true });
console.log("[stage-wasm] staged compiler pkg/ -> src/wasm/pkg/");

// The runtime hash the compiler pins is `required_runtime_hash()` — a real wasm export, the SAME value
// the app run path uses. Read it AUTHORITATIVELY by instantiating the just-staged compiler wasm, rather
// than scraping the data section for /[0-9a-f]{64}/: that scrape yields ~139 candidates (the real hash
// plus a `00010203…` byte run whose overlapping matches all look like hashes), and the old "pick whichever
// candidate happens to name a .wasm in a store" disambiguation picked WRONG on a REQUIRED_RUNTIME_HASH bump
// — under the nix local-gate the store holds the runtime by the NEW hash, but a stale/spurious candidate
// could win (or none matched → fell through to an arbitrary candidate with runtimePath=null → runtime.wasm
// never staged → check-examples.mjs ENOENT, blocking every hash-bumping MR fleet-wide). The export is ground
// truth: one hash, no store-dependent guessing.
let hash;
try {
  const stagedPkg = join(dest, "pkg");
  const { default: initWasm, required_runtime_hash } = await import(
    pathToFileURL(join(stagedPkg, "cdz_wasm.js")).href
  );
  await initWasm({ module_or_path: await readFile(join(stagedPkg, "cdz_wasm_bg.wasm")) });
  hash = required_runtime_hash();
} catch (e) {
  console.error(`[stage-wasm] could not read required_runtime_hash() from the staged compiler wasm: ${e}`);
  process.exit(1);
}
// The content address is a 45-char base62 `Hash` text (`0-9A-Za-z`, tag + blake3 digest — design §8),
// NOT the pre-flip 64-hex digest; validate that shape so a genuinely malformed value still fails loud.
if (!/^[0-9A-Za-z]{45}$/.test(hash)) {
  console.error(`[stage-wasm] required_runtime_hash() returned a non-hash value: ${JSON.stringify(hash)}`);
  process.exit(1);
}

// Search likely store locations: an explicit CADENZA_STORE (passed by `cargo xtask guide-wasm` AND the nix
// local-gate's guide-examples derivation, both export it pointing at their componentStore), CDZ_STORE (the
// other name the nix derivations export the same componentStore under), then the worktree + main-repo stores.
const stores = [
  process.env.CADENZA_STORE,
  process.env.CDZ_STORE,
  join(guide, "..", "target", "cadenza-store"),
  join(guide, "..", "..", "..", "..", "target", "cadenza-store"),
].filter(Boolean);
let runtimePath = null;
for (const s of stores) {
  const candidate = join(s, `${hash}.wasm`);
  if (existsSync(candidate)) {
    runtimePath = candidate;
    break;
  }
}
if (!runtimePath) {
  console.error(
    `[stage-wasm] runtime ${hash}.wasm not found in any store (${stores.join(", ")}).\n` +
      `  Build it with \`cargo xtask build\` so the store holds the compiler's pinned runtime.`,
  );
  // Not fatal for scalar-only development — the guide runs scalar examples without a runtime.
  console.error("[stage-wasm] continuing WITHOUT a bundled runtime (scalar examples only).");
} else {
  await writeFile(join(dest, "runtime.wasm"), await readFile(runtimePath));
  await writeFile(join(dest, "runtime-hash.txt"), hash);
  console.log(`[stage-wasm] staged runtime ${hash.slice(0, 12)}… -> src/wasm/runtime.wasm`);
}

// Stage the CAD library sources into the guide tree so /cad can PRELOAD them via `compile_with_preloaded`
// — the reader's buffer holds only the model, the CAD vocab is link-merged from these preloaded modules
// (operator P5, ruling A). `exact.cdz` is the base geometry lib (Solid/Vec3/v3r/lower/…); `helpers.cdz` is
// the ergonomic surface (box/cyl/hole-through/…) the PARAMETRIC showcase models import; `units.cdz` is the
// UNIT edge constructors (inch/…) the units-parametric showcase uses (a slider read in inches, converted
// exactly over Rational to model mm). They live OUTSIDE guide/src (a raw `../../../implementation/cad/src/
// *.cdz` import is blocked by Vite's dev `server.fs.allow` with project root = guide/), so staging them here
// (git-ignored, regenerated with the wasm — SAME pattern as runtime.wasm) is the single-source, dev-and-
// build-safe way. CadPage `?raw`-imports the staged copies. Non-fatal if absent (only /cad needs them).
const cadLibs = ["exact.cdz", "helpers.cdz", "units.cdz"];
await mkdir(join(dest, "cad"), { recursive: true });
for (const lib of cadLibs) {
  const src = join(guide, "..", "implementation", "cad", "src", lib);
  if (existsSync(src)) {
    await writeFile(join(dest, "cad", lib), await readFile(src));
    console.log(`[stage-wasm] staged CAD lib ${lib} -> src/wasm/cad/${lib}`);
  } else {
    console.error(`[stage-wasm] CAD lib not found at ${src} — /cad preload of '${lib}' will be unavailable (non-fatal).`);
  }
}

// Stage the MUSIC library sources so the upcoming /music showcase page can PRELOAD them via
// `compile_with_preloaded` — same pattern as the CAD libs (a reader's buffer imports the music vocab; the
// modules are link-merged). The showcases are IMPORT-DEPENDENT on these (a bare <Runnable> can't cross-module
// import), which is why /music is a live page like /cad. v1 is the EVENT-STRUCTURE story (schedule()→
// balanced() no-stuck-keys), so `synth.cdz` (Web Audio synthesis) is EXCLUDED — no event-structure lib
// imports it (verified). The libs cross-import each other (piece→schedule/chord/pitch/compose, …), so ALL of
// the event-structure closure is staged. Staging extra (unused) libs is harmless — an unused preload is
// benign (CAD proved this), and it avoids a missing-lib break if a showcase imports one not anticipated here.
// NOTE: the exact per-buffer IMPORT SURFACE (which symbols a showcase buffer imports) is v-music's authority
// and not yet frozen — this stages the LIBS; the import clauses live in the MusicPreload module (pending).
// AUTHORITATIVE list per v-music (feature authority) — keep synced to implementation/music/src/*.cdz; a
// showcase importing a lib NOT here is a silent preload gap (CDZ0201). v-music pings when adding an
// importable lib. `synth.cdz` EXCLUDED (Web Audio graph, not a MIDI/event-structure dep — v1 is event
// structure). `des-piece.cdz` (DES-composed-piece demo, vendors a minimal Sim) + `pipeline.cdz` (MIDI-pipeline
// transform) landed with v-music's phase-2 MR 10203 and are staged for future showcases (harmless if unused).
const musicLibs = [
  "schedule.cdz", "pitch.cdz", "interval-ratio.cdz", "scale-ratio.cdz", "scale.cdz",
  "chord-ratio.cdz", "chord.cdz", "rhythm.cdz", "rhythm-ratio.cdz", "compose.cdz", "piece.cdz",
  "des-piece.cdz", "pipeline.cdz",
  // `pattern.cdz` (Strudel/Tidal live-coding — /music R4, the page climax; v-guide authors + value-pins it).
  // Its imports are ONLY schedule/compose/pitch — all three above — so it's self-consistent staged alone.
  // STAGING-ONLY: not added to musicPreload.ts MUSIC_PRELOAD_NAMES yet (no showcase imports it until R4
  // lands), and an unused staged lib is benign (the existsSync loop below + check-music-preload read their
  // own name lists, not this array), so this add is gate-safe on its own.
  "pattern.cdz",
  // `interval-vector.cdz` (Allen Forte interval-class vector: an `Icv` record + interval-class /
  // interval-class-vector / icv-count / ic-tritone / icv-total) and `set-class.cdz` (pitch-class set
  // transposition: transpose-pc-set / transposition-between / same-transposition-class). Both are pure
  // pitch-class arithmetic over Int64, TOTAL, self-contained (no imports beyond their own pc-set fold) —
  // self-consistent staged alone (landed #1088 + follow-up). STAGING-ONLY, same as pattern.cdz above:
  // NOT in musicPreload.ts MUSIC_PRELOAD_NAMES (no showcase imports them yet), and an unused staged lib is
  // benign (the ⊆ gate allows musicLibs ⊇ MUSIC_PRELOAD_NAMES; existsSync loop + check-music-preload read
  // their own name lists, not this array). v-music/v-guide bump MUSIC_PRELOAD_NAMES + PRELOAD_SOURCES +
  // preloadArity in lockstep IF/WHEN a showcase imports them.
  "interval-vector.cdz", "set-class.cdz",
];
await mkdir(join(dest, "music"), { recursive: true });
for (const lib of musicLibs) {
  const src = join(guide, "..", "implementation", "music", "src", lib);
  if (existsSync(src)) {
    await writeFile(join(dest, "music", lib), await readFile(src));
    console.log(`[stage-wasm] staged music lib ${lib} -> src/wasm/music/${lib}`);
  } else {
    console.error(`[stage-wasm] music lib not found at ${src} — /music preload of '${lib}' will be unavailable (non-fatal).`);
  }
}

// Sanity: report what we staged.
const staged = await readdir(join(dest, "pkg"));
console.log(`[stage-wasm] pkg contents: ${staged.join(", ")}`);
