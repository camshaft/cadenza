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

import { readFileSync, readdirSync, mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");

// ---- the compiler (browser wasm) + runner (jco), loaded once ----
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, compile, render_value, render_syntax } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

// ---- wrapModule / stripModule / renderSnippet: mirror guide/src/components/useCadenzaEditor.ts ----
// (keep in sync). Snippets are authored in s-expr (the guide default `authoredIn`); a bare expr / defs
// get the `export`/`main` the compiler needs, at top level (no module shell). The reader also TOGGLES
// to ML, so we check that surface too (see `renderSnippet` + the ML pass below) — the surface where the
// wrap/strip round-trip is most likely to bite.
const DECL = "def|type|effect";
// A top-level STATEMENT that isn't def/type/effect but still needs an export appended (never wrapped as
// a bare expr). `Unit.define` (custom unit) only resolves at top level. Keep in sync with useCadenzaEditor.
const STMT = "Unit\\.define";
function wrapModule(src, surface) {
  const t = src.trim();
  if (surface === "sexpr") {
    if (/^\(module\b/.test(t) || /^\(do\b/.test(t)) return t;
    if (new RegExp(`^\\((${DECL}|${STMT})\\b`).test(t)) return `(do ${t} (export main))`;
    return `(do (def (main) ${t}) (export main))`;
  }
  if (/^module\b/.test(t) || /(^|\n)\s*export\b/.test(t)) return t;
  if (new RegExp(`^(${DECL}|${STMT})\\b`).test(t)) return `${t}\nexport { main }`;
  return `def main() = ${t}\nexport { main }`;
}
function dedent(s) {
  const lines = s.split("\n");
  const min = Math.min(...lines.filter((l) => l.trim()).map((l) => l.match(/^ */)[0].length), Infinity);
  return Number.isFinite(min) ? lines.map((l) => l.slice(min)).join("\n") : s;
}
function stripModule(rendered, surface) {
  const t = rendered.trim();
  if (surface === "sexpr" ? /^\(module\b/.test(t) : /^module\b/.test(t)) return rendered;
  if (surface === "sexpr") {
    const m = /^\(do\b([\s\S]*)\)\s*$/.exec(t);
    const body = (m ? m[1] : t).trim().replace(/\(export\s+[^)]*\)\s*$/, "").trim();
    const bare = /^\(def\s+\(main\)\s+([\s\S]*)\)$/.exec(body);
    if (bare && !/\(def\b|\(type\b/.test(bare[1])) return bare[1].trim();
    return body;
  }
  const lines = t.split("\n").filter((l) => !/^\s*export\s*[({]/.test(l));
  const last = lines.reduce((a, l, i) => (l.trim() ? i : a), -1);
  const body = lines.map((l, i) => (/^\S/.test(l) || i === last ? l.replace(/;\s*$/, "") : l)).join("\n").trim();
  const bare = /^def\s+main\(\)\s*=[^\S\n]*([\s\S]*)$/.exec(body);
  if (bare && !/^\s*(def|type)\b/m.test(bare[1])) return dedent(bare[1]).trim();
  return body;
}
/// The ML the reader sees after toggling: wrap the s-expr snippet, render to ML, strip the scaffolding.
function renderToMl(snippet) {
  return stripModule(render_syntax(wrapModule(snippet, "sexpr"), "sexpr", "ml"), "ml");
}

// ---- transpile a component to disk and load its `instantiate` (mirrors runWorker.ts loadComponent) ----
async function loadComponent(componentBytes, name) {
  const { files } = await transpileBytes(new Uint8Array(componentBytes), {
    name,
    instantiation: "async",
    wasiShim: false,
    minify: false,
  });
  const dir = mkdtempSync(join(tmpdir(), "cdz-check-"));
  // jco emits nested files (e.g. `interfaces/cadenza-run-run.d.ts`) whenever the result is a COMPOUND
  // value that escapes via a resource interface — so create each file's parent dir before writing, or
  // the write ENOENTs and a tuple/map/record-returning solution looks like a run failure.
  for (const [f, b] of Object.entries(files)) {
    const p = join(dir, f);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, b);
  }
  const mod = await import(join(dir, `${name}.js`));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  return { instantiate: mod.instantiate, getCore };
}

// The value-heap runtime, instantiated once and reused. A COMPOUND-returning program imports
// `cadenza:runtime/heap`; without it, instantiation throws "Cannot destructure property 'boxInt'/…".
// The browser runner (runWorker.ts) wires this exact interface — the harness MUST too, or every
// list/map/record/tuple-returning example looks like a run failure when it actually works in-app.
const HEAP_IMPORT = "cadenza:runtime/heap";
const runtimePath = join(guideRoot, "src/wasm/runtime.wasm");
let heapPromise = null;
async function getHeap() {
  if (!heapPromise) {
    heapPromise = (async () => {
      const rt = await loadComponent(readFileSync(runtimePath), "heap");
      const root = await rt.instantiate(rt.getCore, {});
      return root[HEAP_IMPORT] ?? root["heap"];
    })();
  }
  return heapPromise;
}

// ---- run a compiled component through jco, return its rendered value text ----
async function runComponent(componentBytes) {
  const prog = await loadComponent(componentBytes, "prog");
  const heap = await getHeap();
  const imports = heap ? { [HEAP_IMPORT]: heap } : {};
  const root = await prog.instantiate(prog.getCore, imports);
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
  // Chunk the file by element-open positions. `<Runnable>`/`<Exercise>` are ALWAYS self-closing
  // (`… />`) and never nest, so each element's attributes run from its opening tag up to the next
  // element's opening tag (or EOF). A prior non-greedy `…*?(?:/>|>…</\1>)` regex truncated `attrs`
  // at the first `/>`-or-`>` inside a JSX prompt fragment (`prompt={<>…</>}`), so every Exercise's
  // `solution`/`expected` (which follow `prompt`) fell off the end and ALL 45 exercises were silently
  // skipped — the suite validated runnables only. Chunking to the next open tag is robust to that.
  const openRe = /<(Runnable|Exercise)\b/g;
  const opens = [];
  let om;
  while ((om = openRe.exec(tsx))) opens.push({ kind: om[1], start: om.index });
  for (let i = 0; i < opens.length; i++) {
    const { kind, start } = opens[i];
    const end = i + 1 < opens.length ? opens[i + 1].start : tsx.length;
    const attrs = tsx.slice(start, end);
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

// ---- check one program (already wrapped) in one surface, returning null on success or a reason ----
async function checkProgram(program, surface, ex, where) {
  const brief = ex.snippet.replace(/\n/g, " ").slice(0, 80);
  let r;
  try {
    r = compile(program, surface);
  } catch (e) {
    // A throw = a parse error. Fine only if the example is meant to fail.
    if (ex.expect === "error") return null;
    return `${ex.file} [${ex.kind}] (${where}): parse error — ${String(e.message || e).slice(0, 80)}\n    ${brief}`;
  }
  const declined = !r.component;
  if (ex.expect === "error") {
    // "meant to fail" = a compile decline OR a runtime trap (e.g. `(UInt8.of 300)`); accept either.
    if (declined) return null;
    try { await runComponent(r.component); } catch { return null; }
    return `${ex.file} [${ex.kind}] (${where}): expect="error" but it compiled AND ran to a value\n    ${brief}`;
  }
  if (declined) {
    const d = r.diagnostics.find((x) => x.error) ?? r.diagnostics[0];
    return `${ex.file} [${ex.kind}] (${where}): expected to compile but DECLINED — ${d ? `${d.code} ${d.message}` : "no component"}\n    ${brief}`;
  }
  // Compiles. A graded exercise must also RUN to its `expected` value (checked on the s-expr surface).
  if (ex.expected != null && surface === "sexpr") {
    // A graded exercise MUST return a SCALAR. The browser's Check (Exercise.tsx) compares the result
    // rendered in the reader's CURRENT surface, but this harness renders s-expr canonical — a scalar
    // (bare number/bool) reads identically in both, a COMPOUND does NOT (`(: (map …) …)` vs ML
    // `#{…} : Map(…)`). So a compound `expected` would pass here yet FAIL the in-browser Check in ML.
    // Reject it at authoring time; return the compound as a Runnable (ungraded) instead.
    if (/^\(:/.test(ex.expected.trim()))
      return `${ex.file} [Exercise] (${where}): \`expected\` is a COMPOUND value (${JSON.stringify(ex.expected.slice(0, 40))}…) — graded exercises must return a SCALAR (it's compared in the reader's surface, and a compound renders differently in ML vs s-expr). Show the compound as a Runnable instead.\n    ${brief}`;
    try {
      const got = await runComponent(r.component);
      if (String(got).trim() !== ex.expected.trim())
        return `${ex.file} [Exercise] (${where}): solution ran to ${JSON.stringify(String(got))}, expected ${JSON.stringify(ex.expected)}\n    ${brief}`;
    } catch (e) {
      return `${ex.file} [Exercise] (${where}): solution failed to run — ${String(e.message || e).slice(0, 80)}`;
    }
  }
  return null;
}

// ---- check each example in BOTH surfaces (the reader can toggle) ----
let pass = 0;
const failures = [];
for (const ex of examples) {
  // 1. s-expr — the authored surface.
  const sexprProgram = ex.noWrap ? ex.snippet.trim() : wrapModule(ex.snippet, "sexpr");
  const sexprFail = await checkProgram(sexprProgram, "sexpr", ex, "s-expr");
  if (sexprFail) { failures.push(sexprFail); continue; }

  // 2. ML — what the reader sees after toggling. Render the snippet to ML, then wrap + compile THAT.
  //    This catches wrap/strip round-trip bugs that only bite on the ML surface (e.g. a `;`-in-a-
  //    do-block snippet whose wrapper skipped the export). `noWrap` snippets are full modules already.
  if (!ex.noWrap) {
    let mlFail;
    try {
      const mlProgram = wrapModule(renderToMl(ex.snippet), "ml");
      mlFail = await checkProgram(mlProgram, "ml", ex, "ML toggle");
    } catch (e) {
      mlFail = `${ex.file} [${ex.kind}] (ML toggle): render/wrap threw — ${String(e.message || e).slice(0, 80)}`;
    }
    if (mlFail) { failures.push(mlFail); continue; }
  }
  pass++;
}

console.log(`\nchecked ${examples.length} examples across ${files.length} files (both surfaces): ${pass} ok, ${failures.length} failed`);
if (failures.length) {
  console.error("\nFAILURES:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log("✓ every guide example compiles in both surfaces, and every graded exercise runs to its expected value.");
