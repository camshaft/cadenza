#!/usr/bin/env node
/// /music PRELOAD conformance: verify the /music route's preloaded-music-library path works end-to-end the
/// way the browser does — WITHOUT a browser (node + the staged wasm, like check-cad-preload). /music compiles
/// a bare showcase buffer against the PRELOADED music libs (staged from implementation/music) via
/// `compile_with_preloaded` — the buffer holds only the model; the music vocab (interval-ratio/chord/pitch/
/// piece/schedule/…) is link-merged. check-examples can't cover this (its examples are self-contained, not
/// preloaded), and check-visual (browser) isn't in CI — so the whole preload path (link-merge + the injected
/// imports + run + the event-stream parse) would be un-gated. This is that regression guard.
///
/// HOW: load the staged wasm + the staged music libs, then for each showcase (from music/examples.ts):
///   (1) compile_with_preloaded(injectImport(buffer), …) → assert a component + 0 error diags, BOTH surfaces;
///   (2) run the piece-to-events showcase headlessly (jco → run → render_value) and assert the MidiEvent
///       parser yields a non-empty, BALANCED event stream (the no-stuck-keys correctness payoff).
/// Reuses the REAL injectImport + parser from src/music (no private copy — the gate must match what ships).
///
/// Run: `npm run check:music-preload` (needs the staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.

import { readFile } from "node:fs/promises";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const pkgDir = join(guideRoot, "src/wasm/pkg");
const HEAP_IMPORT = "cadenza:runtime/heap";
const runtimePath = join(guideRoot, "src/wasm/runtime.wasm");
// FINDING#23: the runtime imports cadenza:nfc/normalize (separate NFC component) — supply the JS shim so it
// instantiates. NFC of well-formed UTF-8 is String.prototype.normalize('NFC') over the list<u8> boundary.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes) => new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};

const wasm = await import(pathToFileURL(join(pkgDir, "cdz_wasm.js")).href);
await wasm.default(await readFile(join(pkgDir, "cdz_wasm_bg.wasm")));
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

// Reuse the REAL injection + preload names + parser /music ships (no private copy — the gate must match).
const { injectImport, MUSIC_PRELOAD_NAMES, MUSIC_LIB_FORMAT } = await import(pathToFileURL(join(guideRoot, "src/music/musicPreload.ts")).href);
const { parseMidiEvents, isBalanced } = await import(pathToFileURL(join(guideRoot, "src/music/midiEvents.ts")).href);
const { EXAMPLES } = await import(pathToFileURL(join(guideRoot, "src/music/examples.ts")).href);

// Vacuous-pass floors: `EXAMPLES` and `MUSIC_PRELOAD_NAMES` are IMPORTED, so a rename/empty-export/bad
// filter could resolve either to `[]` — then the per-showcase compile loop below never runs and this
// gate reports success on nothing (a false green shipping an unverified /music). The specific-slug guards
// (piece-to-events, value-pins) catch some of that, but a floor on the input sets makes an empty import
// FAIL outright. /music ships FOUR showcases (R1 rational-intervals, R2 chord-to-midi, R3 piece-to-events,
// R4 euclidean-pattern) preloaded against ≥1 music lib — the floor tracks that count, so LOSING a showcase
// (drop to 3) fails here rather than silently shipping a shrunk /music. Bump this alongside EXAMPLES.
// (Mirrors the floors in check-examples.mjs / check-prose-annotations.mjs.)
if (!Array.isArray(EXAMPLES) || EXAMPLES.length < 4) {
  console.error(
    `\n✗ music-preload conformance FAILED — expected ≥4 /music showcases in src/music/examples.ts, ` +
      `found ${Array.isArray(EXAMPLES) ? EXAMPLES.length : typeof EXAMPLES}; the EXAMPLES import likely broke ` +
      `or a showcase was dropped. Refusing a vacuous pass.`,
  );
  process.exit(1);
}
if (!Array.isArray(MUSIC_PRELOAD_NAMES) || MUSIC_PRELOAD_NAMES.length < 1) {
  console.error(
    `\n✗ music-preload conformance FAILED — MUSIC_PRELOAD_NAMES is empty (found ` +
      `${Array.isArray(MUSIC_PRELOAD_NAMES) ? MUSIC_PRELOAD_NAMES.length : typeof MUSIC_PRELOAD_NAMES}); ` +
      `nothing would be preloaded, so the conformance is vacuous. Refusing.`,
  );
  process.exit(1);
}

const names = [...MUSIC_PRELOAD_NAMES];
let sources;
try {
  sources = names.map((n) => readFileSync(join(guideRoot, `src/wasm/music/${n}.cdz`), "utf8"));
} catch {
  console.error(`\n✗ music-preload conformance FAILED — a staged music lib (src/wasm/music/*.cdz) is missing (run \`cargo xtask guide-wasm\` to stage it). /music cannot preload without them.`);
  process.exit(1);
}
const formats = names.map(() => MUSIC_LIB_FORMAT);
const failures = [];

// (1) Every showcase compiles as a preloaded buffer in BOTH surfaces.
for (const ex of EXAMPLES) {
  for (const surface of ["ml", "sexpr"]) {
    const program = injectImport(ex.source[surface], surface);
    let cr;
    try {
      cr = wasm.compile_with_preloaded(program, surface, names, sources, formats);
    } catch (e) {
      failures.push(`[${surface}] ${ex.slug}: compile_with_preloaded THREW ${String(e && e.message ? e.message : e).slice(0, 100)}`);
      continue;
    }
    const errs = (cr.diagnostics ?? []).filter((d) => d.error);
    if (!cr.component || errs.length) {
      failures.push(`[${surface}] ${ex.slug}: did not compile against preloaded music libs${errs.length ? ` — ${errs.map((d) => `${d.code ?? ""} ${d.message ?? ""}`.trim()).join("; ")}` : " (no component)"}`);
    } else {
      console.log(`  ✓ [${surface}] ${ex.slug}: compiles against the preloaded music libs → component`);
    }
  }
}

// (2)/(3) The showcases RUN and yield the exact values their prose descriptions claim — headless via jco (like
// check-cad-preload's mesh stage), heap wired (a compound result is a heap value). One shared heap serves all
// three. This closes the gap where the gate only COMPILED R1/R2: their claimed values ("true", "60, 64, 67")
// were never run, so a music-lib semantic regression OR a description that drifts from the runtime would slip
// past. Each showcase runs via the SAME two shapes the guide's runWorker uses: a compound result via the
// resource-escape `cadenza:run/run` make()/encode() path, a scalar result via a nullary function export.
async function loadComp(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), "musicp-"));
  for (const [f, b] of Object.entries(files)) { const p = join(dir, f); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, b); }
  const mod = await import(pathToFileURL(join(dir, `${name}.js`)).href);
  return { instantiate: mod.instantiate, getCore: async (p) => WebAssembly.compile(readFileSync(join(dir, p))) };
}

/// Compile a showcase buffer against the preloaded libs, run it, and return its RENDERED value string — the
/// exact text /music shows the reader. Mirrors runWorker.runComponent: a compound (List/record) result comes
/// out via the resource-escape `run` make()/encode() path (render_value stringifies the encoded bytes); a
/// scalar (Bool/Int) result is a bare nullary export we call directly. Throws on a compile/run failure.
async function runShowcase(ex, heap) {
  const program = injectImport(ex.source.sexpr, "sexpr");
  const cr = wasm.compile_with_preloaded(program, "sexpr", names, sources, formats);
  if (!cr.component) throw new Error("no component");
  const prog = await loadComp(new Uint8Array(cr.component), "prog");
  const root = await prog.instantiate(prog.getCore, { [HEAP_IMPORT]: heap });
  const iface = root["cadenza:run/run"] ?? root["run"];
  if (iface && typeof iface.make === "function") {
    const handle = iface.make();
    const rendered = wasm.render_value(iface.encode(handle));
    // Guarded dispose (jco resource-drop-glue OOB on a large heap value; see check-cad-preload) — harmless here.
    try { handle?.[Symbol.dispose]?.(); } catch { /* known jco resource-drop OOB — consumed */ }
    return { kind: "compound", rendered };
  }
  const nullary = Object.entries(root).find(([, v]) => typeof v === "function" && v.length === 0);
  if (!nullary) throw new Error("no runnable entry (no compound run iface, no nullary export)");
  return { kind: "scalar", rendered: String(nullary[1]()) };
}

if (!existsSync(runtimePath)) {
  failures.push(`music-preload: runtime.wasm missing at ${runtimePath} (run \`cargo xtask guide-wasm\`)`);
} else {
  const rt = await loadComp(readFileSync(runtimePath), "heap");
  const heap = (await rt.instantiate(rt.getCore, { [NFC_IMPORT]: nfcHostImport }))[HEAP_IMPORT];

  // (2) The piece-to-events showcase RUNS + yields a non-empty BALANCED event stream (the marquee payoff).
  // Every EVENT-STREAM showcase RUNS + yields a non-empty BALANCED stream (the no-stuck-keys payoff). Both
  // R3 piece-to-events and R4 euclidean-pattern render into the same MidiEvent table, so both are gated the
  // same way; R4 additionally pins its exact event count (the 6-event Euclidean tresillo — 3 onsets across 8
  // steps, each note-on paired with a note-off). `exactRows` (when set) pins the count so a pattern-lib
  // regression that changes the rhythm is caught, not just a balance break.
  const EVENT_SHOWCASES = [
    { slug: "piece-to-events", label: "the I-V-vi-IV piece" },
    { slug: "euclidean-pattern", label: "a Euclidean tresillo (3 onsets in 8 steps)", exactRows: 6 },
  ];
  for (const es of EVENT_SHOWCASES) {
    const showcase = EXAMPLES.find((e) => e.slug === es.slug);
    if (!showcase) {
      failures.push(`${es.slug} showcase missing from EXAMPLES (the event-stream gate can't run)`);
      continue;
    }
    try {
      const { rendered } = await runShowcase(showcase, heap);
      const parsed = parseMidiEvents(rendered);
      if (!parsed.ok) {
        failures.push(`${es.slug}: ran but the value did not parse as a MidiEvent stream — ${parsed.error}`);
      } else if (parsed.rows.length === 0) {
        failures.push(`${es.slug}: parsed ZERO events (empty schedule?)`);
      } else if (!isBalanced(parsed.rows)) {
        failures.push(`${es.slug}: the event stream is NOT balanced (a stuck key) — the no-stuck-keys invariant regressed`);
      } else if (es.exactRows != null && parsed.rows.length !== es.exactRows) {
        failures.push(`${es.slug}: expected exactly ${es.exactRows} events (${es.label}), got ${parsed.rows.length} — the pattern rhythm drifted`);
      } else {
        console.log(`  ✓ ${es.slug}: runs → ${parsed.rows.length} MIDI events, BALANCED (${es.label})`);
      }
    } catch (e) {
      failures.push(`${es.slug}: run/parse THREW ${String(e && e.message ? e.message : e).slice(0, 100)}`);
    }
  }

  // (3) R1/R2 VALUE-PIN: run each and assert the exact value its description claims. R1 (rational-intervals)
  // is the Bool identity "a fifth + a fourth = an octave" → "true"; R2 (chord-to-midi) is the C-major triad's
  // note numbers → the (list 60 64 67) render (the "60, 64, 67 (C, E, G)" the description states). A drift in
  // either the music lib OR the description trips this. `rendered` for a scalar is the bare "true"; for the
  // list it's the compiler's canonical value render, so we match on the note numbers being present in order.
  const VALUE_PINS = [
    { slug: "rational-intervals", check: (r) => r.trim() === "true", want: `the Bool "true" (a fifth + a fourth is exactly an octave)` },
    { slug: "chord-to-midi", check: (r) => /\b60\b[^0-9]+64\b[^0-9]+67\b/.test(r), want: `a note-number list containing 60, 64, 67 in order (a C-major triad: C, E, G)` },
  ];
  for (const pin of VALUE_PINS) {
    const ex = EXAMPLES.find((e) => e.slug === pin.slug);
    if (!ex) { failures.push(`${pin.slug}: showcase missing from EXAMPLES (value-pin can't run)`); continue; }
    try {
      const { rendered } = await runShowcase(ex, heap);
      if (!pin.check(rendered)) {
        failures.push(`${pin.slug}: ran but the value drifted — expected ${pin.want}, got \`${rendered.trim().slice(0, 80)}\` (music lib regressed OR the description no longer matches the runtime)`);
      } else {
        console.log(`  ✓ ${pin.slug}: runs → ${rendered.trim().slice(0, 40)} (${pin.want})`);
      }
    } catch (e) {
      failures.push(`${pin.slug}: run THREW ${String(e && e.message ? e.message : e).slice(0, 100)}`);
    }
  }
}

if (failures.length) {
  console.error("\n✗ music-preload conformance FAILED — the /music preloaded-library path regressed:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log("\n✓ music-preload conformance: every /music showcase compiles against the preloaded music libs in both surfaces, both event-stream showcases (the I-V-vi-IV piece + the Euclidean tresillo, 6 events) schedule to non-empty BALANCED MIDI streams, and R1/R2 run to the exact values their descriptions claim (true; the C-major triad 60/64/67) — the /music preload + event-structure path + the showcases' claimed values all stay working.");
