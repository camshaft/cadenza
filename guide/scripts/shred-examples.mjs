#!/usr/bin/env node
/// SHRED the guide examples into per-example artifact dirs — the guide analogue of `cdz-corpus records`,
/// so v-nix can wire a per-example, parallel, content-addressed nix build+exec (mkCorpusShred/Build/Exec →
/// mkGuideShred/Build/Exec) and turn the ~10-min SERIAL check:examples into a seconds-scale CACHED matrix
/// (examples never change, so each case cache-HITS unless ITS program or the compiler changes).
///
/// Usage:  node --expose-gc scripts/shred-examples.mjs --out-dir DIR
///   (needs Node ≥ 22.6 for .ts type-stripping + the staged wasm pkg — `cargo xtask guide-wasm`.)
///
/// This is the FRONT-HALF of check-examples.mjs (extract → wrap → lower → render both surfaces), dumping
/// each example's compilable program(s) + expected outcome + metadata INSTEAD of compiling/running inline.
/// It reuses the SAME extraction (./example-extract.mjs) + the SAME wrapModule / lowerToCompile /
/// render_syntax the gate + the live app use, so a shred case can never drift from what ships.
///
/// Per-example dir schema (mirrors the corpus `<idx>-<title>/`), emitted as `<NNNN>-<slug>/`:
///   program.sexpr            the wrapped/lowered program in s-expr (what the gate compiles)
///   program.ml               the ML-toggle form (single-file examples that toggle surfaces)
///   module-<name>.<surface>  preloaded peer sources of a multi-file example (entry = program.*)
///   expected                 the expected rendered value (graded examples only; absent otherwise)
///   expect-kind              "value" | "error"   (error = must decline-or-trap)
///   meta.json                { file, kind, authoredSurface, surfaces:[...], graded, multiFile,
///                              entryName, peers:[{name,surface}], prelude, deferred, reason? }
/// Plus a top-level manifest.json listing every case dir + its metadata, so the nix side enumerates without
/// re-parsing. DEFERRED in v1 (tagged deferred:true, no program emitted): mode="test" @test examples and
/// notebook cells (they need the @test driver / repl_eval path) — a follow-up shred kind.

import { readFileSync, readdirSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { extractExamples } from "./example-extract.mjs";

// ---- args ----
const argv = process.argv.slice(2);
const outIdx = argv.indexOf("--out-dir");
if (outIdx < 0 || !argv[outIdx + 1]) {
  console.error("usage: node --expose-gc scripts/shred-examples.mjs --out-dir DIR");
  process.exit(2);
}
const outDir = argv[outIdx + 1];

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// ---- the browser compiler (for render_syntax only — the shred does not compile) ----
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, render_syntax } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
// Same wrap/lower the gate + live app use — imported from the guide source, so the shred can't drift.
const { wrapModule, stripModule } = await import(join(guideRoot, "src/components/wrapModule.ts"));
const { lowerToCompile } = await import(join(guideRoot, "src/explorer/fileModel.ts"));

/// The ML the reader sees after toggling a default (s-expr-authored) snippet — mirrors check-examples'
/// renderToMl: wrap → render to ML → strip the scaffolding (then the caller re-wraps for ML).
const renderToMl = (snippet) => stripModule(render_syntax(wrapModule(snippet, "sexpr"), "sexpr", "ml"), "ml");

// ---- gather every example (chapters + HomePage + playground); notebook + mode=test deferred in v1 ----
const chaptersDir = join(guideRoot, "src/content/chapters");
const contentFiles = [
  ...readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).map((f) => join(chaptersDir, f)),
  join(guideRoot, "src/components/HomePage.tsx"),
];
const examples = contentFiles.flatMap((f) => extractExamples(readFileSync(f, "utf8"), f.replace(guideRoot + "/", "")));

// Playground programs (full modules, noWrap) — same import the gate uses.
const { EXAMPLES: PLAYGROUND } = await import(join(guideRoot, "src/playground/examples.ts"));
for (const p of PLAYGROUND) {
  examples.push({
    file: "src/playground/examples.ts",
    kind: "Runnable",
    snippet: p.source,
    surface: p.surface,
    expect: p.expectError ? "error" : "value",
    expected: p.expected ?? null,
    noWrap: true,
    playgroundId: p.id,
  });
}

// ---- turn one example into its per-case dir contents (mirrors checkExample's wrap/lower/render) ----
// Returns { files: {name: text}, meta } — or { skip, meta } for a deferred kind.
function shredOne(ex) {
  const files = {};
  const meta = {
    file: ex.file,
    kind: ex.kind === "Exercise" ? "exercise" : ex.playgroundId ? "playground" : ex.files ? "multi-file" : ex.isTest ? "test-mode" : "runnable",
    graded: ex.expected != null,
    expectKind: ex.expect === "error" ? "error" : "value",
  };
  if (ex.playgroundId) meta.playgroundId = ex.playgroundId;

  // DEFER mode="test" @test examples (need the @test-export driver, not the eval-main path).
  if (ex.isTest) {
    meta.deferred = true;
    meta.reason = "mode=test @test examples run via the @test-export driver (v2 shred kind)";
    return { skip: true, meta };
  }

  // MULTI-FILE: lower the file set exactly like the app/gate (compile_with_preloaded args).
  if (ex.files) {
    const low = lowerToCompile(ex.files);
    if (!low.ok) throw new Error(`${ex.file}: multi-file won't lower — ${low.reason}`);
    const { text, from, names, sources, formats } = low.lowered;
    files[`program.${from}`] = text;
    const peers = [];
    for (let i = 0; i < names.length; i++) {
      files[`module-${names[i]}.${formats[i]}`] = sources[i];
      peers.push({ name: names[i], surface: formats[i] });
    }
    meta.authoredSurface = from;
    meta.surfaces = [from];
    meta.multiFile = true;
    meta.entryName = "main";
    meta.peers = peers;
    return { files, meta };
  }

  // PLAYGROUND full module (carries an explicit `surface`): authored verbatim + the render_syntax'd toggle
  // (the reader toggles a playground example, so the gate checks both surfaces).
  if (ex.surface) {
    const authored = ex.surface;
    const other = authored === "ml" ? "sexpr" : "ml";
    files[`program.${authored}`] = ex.snippet.trim();
    files[`program.${other}`] = render_syntax(ex.snippet.trim(), authored, other);
    meta.authoredSurface = authored;
    meta.surfaces = [authored, other];
    return { files, meta };
  }

  // CHAPTER wrap={false} full module (no surface): compiled ONCE as s-expr, NO toggle — mirrors
  // check-examples, whose noWrap chapter path skips the ML-render pass (`if (!ex.noWrap)`).
  if (ex.noWrap) {
    files["program.sexpr"] = ex.snippet.trim();
    meta.authoredSurface = "sexpr";
    meta.surfaces = ["sexpr"];
    return { files, meta };
  }

  // CHAPTER snippet authored in ML: wrap in ML, then strip→render→rewrap to s-expr for the toggle.
  if (ex.authoredIn === "ml") {
    files["program.ml"] = wrapModule(ex.snippet, "ml");
    const sexprSnippet = stripModule(render_syntax(files["program.ml"], "ml", "sexpr"), "sexpr");
    files["program.sexpr"] = wrapModule(sexprSnippet, "sexpr");
    meta.authoredSurface = "ml";
    meta.surfaces = ["ml", "sexpr"];
    return { files, meta };
  }

  // CHAPTER snippet authored in s-expr (default) / Exercise solution: wrap s-expr + the ML toggle.
  files["program.sexpr"] = wrapModule(ex.snippet, "sexpr");
  files["program.ml"] = wrapModule(renderToMl(ex.snippet), "ml");
  meta.authoredSurface = "sexpr";
  meta.surfaces = ["sexpr", "ml"];
  return { files, meta };
}

// ---- emit ----
const slugify = (s) => s.replace(/^.*\//, "").replace(/\.[a-z]+$/i, "").replace(/[^A-Za-z0-9]+/g, "-").replace(/^-|-$/g, "").toLowerCase();
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const manifest = [];
let idx = 0;
let emitted = 0;
let deferred = 0;
for (const ex of examples) {
  idx++;
  const n = String(idx).padStart(4, "0");
  const slug = `${n}-${slugify(ex.file)}`;
  let res;
  try {
    res = shredOne(ex);
  } catch (e) {
    console.error(`shred FAILED on ${ex.file} #${idx}: ${String(e && e.message ? e.message : e)}`);
    process.exit(1);
  }
  const caseDir = join(outDir, slug);
  mkdirSync(caseDir, { recursive: true });
  if (res.skip) {
    deferred++;
  } else {
    for (const [fname, text] of Object.entries(res.files)) writeFileSync(join(caseDir, fname), text);
    if (ex.expected != null) writeFileSync(join(caseDir, "expected"), String(ex.expected));
    writeFileSync(join(caseDir, "expect-kind"), res.meta.expectKind);
    emitted++;
  }
  writeFileSync(join(caseDir, "meta.json"), JSON.stringify(res.meta, null, 2) + "\n");
  manifest.push({ dir: slug, ...res.meta });
  if (globalThis.gc) globalThis.gc();
}

// Vacuous-pass floors (mirror check-examples): a broken glob/extraction must FAIL, not silently shred nothing.
if (contentFiles.length < 30) { console.error(`shred: expected ≥30 content files, found ${contentFiles.length}`); process.exit(1); }
if (emitted < 100) { console.error(`shred: expected ≥100 emitted cases, got ${emitted} — extraction likely broke`); process.exit(1); }

writeFileSync(join(outDir, "manifest.json"), JSON.stringify({ count: manifest.length, emitted, deferred, cases: manifest }, null, 2) + "\n");
console.log(`shred: ${manifest.length} cases across ${contentFiles.length} content files + playground → ${outDir} (${emitted} emitted, ${deferred} deferred[test-mode/notebook])`);
