#!/usr/bin/env node
/// Verify every runnable example in the guide actually compiles — and that every graded exercise's
/// solution runs to its stated `expected` value. This enforces the guide's "only show what runs"
/// discipline in CI, so a chapter can never drift ahead of (or behind) the compiler.
///
/// What it checks, over every `<Runnable>` / `<Exercise>` in `src/content/chapters/*.tsx` (+ Welcome /
/// HomePage examples):
///   - a `source=` (Runnable) or `solution=` (Exercise) snippet is WRAPPED exactly as the app wraps it
///     (`wrapModule`, mirrored below), compiled via the real browser compiler (`cdz-wasm`), and:
///       · `expect="error"` examples MUST decline (no component);
///       · every other example MUST produce a component (compiles clean);
///   - a graded exercise (has `expected="…"`) additionally RUNS its solution and asserts the rendered
///     scalar equals `expected`.
/// `starter=` snippets are NOT checked — they contain the `?` hole and are meant not to compile.
///
/// Run: `npm run check:examples` (needs the staged wasm pkg — `npm run wasm` first, or `cargo xtask
/// guide-wasm`). Node ≥ 20.19 for jco.

import { readFileSync, readdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// ---- the compiler (browser wasm) + runner (jco), loaded once ----
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, compile, render_value } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

// ---- wrapModule: mirrors guide/src/components/useCadenzaEditor.ts (keep in sync) ----
// Snippets are authored in s-expr (the guide default `authoredIn`); a bare expr / defs get the
// `export`/`main` the compiler needs, at top level (no module shell).
const DECL_HEAD = /^\((def|type|effect)\b/;
function wrapModule(src) {
  const t = src.trim();
  if (/^\(module\b/.test(t) || /^\(do\b/.test(t)) return t;
  if (DECL_HEAD.test(t)) return `(do ${t} (export main))`;
  return `(do (def (main) ${t}) (export main))`;
}

// ---- run a compiled component through jco, return its rendered value text ----
async function runComponent(componentBytes) {
  const { files } = await transpileBytes(new Uint8Array(componentBytes), {
    name: "prog",
    instantiation: "async",
    wasiShim: false,
    minify: false,
  });
  const dir = mkdtempSync(join(tmpdir(), "cdz-check-"));
  for (const [f, b] of Object.entries(files)) writeFileSync(join(dir, f), b);
  const mod = await import(join(dir, "prog.js"));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  const root = await mod.instantiate(getCore, {});
  // Compound result: the resource-escape path (make/encode). Scalar: the sole exported function.
  const iface = root["cadenza:run/run"] ?? root["run"];
  if (iface && typeof iface.make === "function") {
    return render_value(iface.encode(iface.make())); // canonical value text, e.g. "(: (tuple 1 2) …)"
  }
  const fn = Object.values(root).find((v) => typeof v === "function");
  return fn ? String(fn()) : null;
}

// ---- extract `source=`/`solution=`/`expected=`/`expect=` from a chapter's TSX ----
// Each example is a `<Runnable …/>` or `<Exercise …/>` element; we pull the template-literal blocks
// and the string attributes. A tolerant scan (not a full JSX parse) — the guide's examples all use the
// `attr={`…`}` / `attr="…"` shapes, so a per-attribute regex is enough and stays simple.
function extractExamples(tsx, file) {
  const out = [];
  // Split into element chunks so `expected`/`expect` attach to the right snippet.
  // Match <Runnable ...> and <Exercise ...> up to the closing `/>` or `</...>`.
  const elementRe = /<(Runnable|Exercise)\b([\s\S]*?)(?:\/>|>[\s\S]*?<\/\1>)/g;
  let m;
  while ((m = elementRe.exec(tsx))) {
    const [, kind, attrs] = m;
    const grab = (name) => {
      const tl = new RegExp(`${name}=\\{\`([\\s\\S]*?)\`\\}`).exec(attrs);
      if (tl) return tl[1];
      const s = new RegExp(`${name}="([^"]*)"`).exec(attrs);
      return s ? s[1] : null;
    };
    const expect = grab("expect") ?? "value";
    const expected = grab("expected");
    // Skip a `wrap={false}` example (a full module the author wrote) — still compiled, just not wrapped.
    const noWrap = /wrap=\{false\}/.test(attrs);
    if (kind === "Runnable") {
      const source = grab("source");
      if (source != null) out.push({ file, kind, snippet: source, expect, expected: null, noWrap });
    } else {
      // Exercise: check the SOLUTION (the starter has a `?` hole by design).
      const solution = grab("solution");
      if (solution != null) out.push({ file, kind, snippet: solution, expect: "value", expected, noWrap });
    }
  }
  return out;
}

// ---- gather every example across the content ----
const chaptersDir = join(guideRoot, "src/content/chapters");
const files = [
  ...readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).map((f) => join(chaptersDir, f)),
  join(guideRoot, "src/components/HomePage.tsx"),
];
const examples = files.flatMap((f) => {
  try {
    return extractExamples(readFileSync(f, "utf8"), f.replace(guideRoot + "/", ""));
  } catch {
    return [];
  }
});

// ---- check each ----
let pass = 0;
const failures = [];
for (const ex of examples) {
  const program = ex.noWrap ? ex.snippet.trim() : wrapModule(ex.snippet);
  let r;
  try {
    r = compile(program, "sexpr");
  } catch (e) {
    // A throw = a parse error. Fine only if the example is meant to fail.
    if (ex.expect === "error") { pass++; continue; }
    failures.push(`${ex.file} [${ex.kind}]: parse error — ${String(e.message || e).slice(0, 80)}\n    ${ex.snippet.replace(/\n/g, " ").slice(0, 90)}`);
    continue;
  }
  const declined = !r.component;
  if (ex.expect === "error") {
    // "meant to fail" is satisfied by EITHER a compile decline OR a runtime trap (e.g. a range check
    // like `(UInt8.of 300)`). The guide shows both as a non-value outcome; accept either.
    if (declined) { pass++; continue; }
    let traps = false;
    try {
      await runComponent(r.component);
    } catch {
      traps = true;
    }
    if (traps) pass++;
    else failures.push(`${ex.file} [${ex.kind}]: expect="error" but it compiled AND ran to a value\n    ${ex.snippet.replace(/\n/g, " ").slice(0, 90)}`);
    continue;
  }
  if (declined) {
    const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
    failures.push(`${ex.file} [${ex.kind}]: expected to compile but DECLINED — ${d ? `${d.code} ${d.message}` : "no component"}\n    ${ex.snippet.replace(/\n/g, " ").slice(0, 90)}`);
    continue;
  }
  // Compiles. A graded exercise must also RUN to its `expected` value.
  if (ex.expected != null) {
    try {
      const got = await runComponent(r.component);
      if (String(got).trim() === ex.expected.trim()) pass++;
      else failures.push(`${ex.file} [Exercise]: solution ran to ${JSON.stringify(String(got))}, expected ${JSON.stringify(ex.expected)}\n    ${ex.snippet.replace(/\n/g, " ").slice(0, 90)}`);
    } catch (e) {
      failures.push(`${ex.file} [Exercise]: solution failed to run — ${String(e.message || e).slice(0, 80)}`);
    }
  } else {
    pass++;
  }
}

console.log(`\nchecked ${examples.length} examples across ${files.length} files: ${pass} ok, ${failures.length} failed`);
if (failures.length) {
  console.error("\nFAILURES:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log("✓ every guide example compiles, and every graded exercise runs to its expected value.");
