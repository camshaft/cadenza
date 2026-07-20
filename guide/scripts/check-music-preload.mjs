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

const wasm = await import(pathToFileURL(join(pkgDir, "cdz_wasm.js")).href);
await wasm.default(await readFile(join(pkgDir, "cdz_wasm_bg.wasm")));
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

// Reuse the REAL injection + preload names + parser /music ships (no private copy — the gate must match).
const { injectImport, MUSIC_PRELOAD_NAMES, MUSIC_LIB_FORMAT } = await import(pathToFileURL(join(guideRoot, "src/music/musicPreload.ts")).href);
const { parseMidiEvents, isBalanced } = await import(pathToFileURL(join(guideRoot, "src/music/midiEvents.ts")).href);
const { EXAMPLES } = await import(pathToFileURL(join(guideRoot, "src/music/examples.ts")).href);

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

// (2) The piece-to-events showcase RUNS + yields a non-empty BALANCED event stream (the marquee correctness
// payoff). Headless via jco (like check-cad-preload's mesh stage), heap wired (the event list is a heap value).
async function loadComp(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), { name, instantiation: "async", wasiShim: false, minify: false });
  const dir = mkdtempSync(join(tmpdir(), "musicp-"));
  for (const [f, b] of Object.entries(files)) { const p = join(dir, f); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, b); }
  const mod = await import(pathToFileURL(join(dir, `${name}.js`)).href);
  return { instantiate: mod.instantiate, getCore: async (p) => WebAssembly.compile(readFileSync(join(dir, p))) };
}

const eventsShowcase = EXAMPLES.find((e) => e.slug === "piece-to-events");
if (!eventsShowcase) {
  failures.push("piece-to-events showcase missing from EXAMPLES (the event-stream gate can't run)");
} else if (existsSync(runtimePath)) {
  try {
    const rt = await loadComp(readFileSync(runtimePath), "heap");
    const heap = (await rt.instantiate(rt.getCore, {}))[HEAP_IMPORT];
    const program = injectImport(eventsShowcase.source.sexpr, "sexpr");
    const cr = wasm.compile_with_preloaded(program, "sexpr", names, sources, formats);
    if (!cr.component) throw new Error("no component");
    const prog = await loadComp(new Uint8Array(cr.component), "prog");
    const root = await prog.instantiate(prog.getCore, { [HEAP_IMPORT]: heap });
    const iface = root["cadenza:run/run"] ?? root["run"];
    const handle = iface.make();
    const rendered = wasm.render_value(iface.encode(handle));
    // Guarded dispose (jco resource-drop-glue OOB on a large heap value; see check-cad-preload) — harmless here.
    try { handle?.[Symbol.dispose]?.(); } catch { /* known jco resource-drop OOB — consumed */ }
    const parsed = parseMidiEvents(rendered);
    if (!parsed.ok) {
      failures.push(`piece-to-events: ran but the value did not parse as a MidiEvent stream — ${parsed.error}`);
    } else if (parsed.rows.length === 0) {
      failures.push("piece-to-events: parsed ZERO events (empty schedule?)");
    } else if (!isBalanced(parsed.rows)) {
      failures.push(`piece-to-events: the event stream is NOT balanced (a stuck key) — the no-stuck-keys invariant regressed`);
    } else {
      console.log(`  ✓ piece-to-events: runs → ${parsed.rows.length} MIDI events, BALANCED (every note-on has a matching note-off)`);
    }
  } catch (e) {
    failures.push(`piece-to-events: run/parse THREW ${String(e && e.message ? e.message : e).slice(0, 100)}`);
  }
} else {
  failures.push(`music-preload: runtime.wasm missing at ${runtimePath} (run \`cargo xtask guide-wasm\`)`);
}

if (failures.length) {
  console.error("\n✗ music-preload conformance FAILED — the /music preloaded-library path regressed:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log("\n✓ music-preload conformance: every /music showcase compiles against the preloaded music libs in both surfaces, and the piece schedules to a non-empty BALANCED MIDI event stream — the /music preload + event-structure path stays working.");
