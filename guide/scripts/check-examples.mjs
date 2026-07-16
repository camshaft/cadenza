#!/usr/bin/env node
/// Verify every runnable example in the guide actually compiles — and that every graded exercise's
/// solution runs to its stated `expected` value. This enforces the guide's "only show what runs"
/// discipline in CI, so a chapter can never drift ahead of (or behind) the compiler.
///
/// What it checks, over every `<Runnable>` / `<Exercise>` in `src/content/chapters/*.tsx` (+ Welcome /
/// HomePage examples):
///   - a `source=` (Runnable) or `solution=` (Exercise) snippet is WRAPPED exactly as the app wraps it
///     (`wrapModule`, imported from the guide source so it can never drift), compiled via the real
///     browser compiler (`cdz-wasm`), and:
///       · `expect="error"` examples MUST decline (no component) or trap;
///       · every other example MUST produce a component (compiles clean) AND RUN to a value without
///         throwing/trapping/stack-overflowing — compiling is NOT enough (the operator hit an intro
///         example that compiled yet crashed in the browser; "every example is a test" means it must
///         actually run). Running once on the s-expr surface is enough; the ML pass guards the
///         wrap/strip round-trip.
///   - a graded exercise (has `expected="…"`) additionally asserts the rendered scalar equals `expected`.
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

// ---- the blocklist: examples that DON'T run yet, classified + routed (operator policy 2026-07-15) ----
// An entry marks a KNOWN failure the guide can't fix on its own (a filed compiler bug, or a content bug
// owned by v-guide). Such an example is reported "known-blocked" (loud + tracked) rather than
// hard-failing the gate — otherwise the gate stays red on something the guide can't fix, and no example
// ships broken. RE-RUN LOOP: each run re-checks every blocked example; when one starts PASSING the
// harness says so, so the entry is removed and the example ships. See example-blocklist.json for shape.
const blocklist = JSON.parse(readFileSync(join(here, "example-blocklist.json"), "utf8")).blocked ?? [];
/// The blocklist entry an example matches, or null. An entry matches when the chapter file agrees AND
/// EVERY substring in `match` (a string or an array — all must be present) appears in the snippet, so an
/// entry can be as precise as needed (e.g. `["Qty.value", "Unit.in"]` blocks only the examples that wrap
/// `Unit.in` in `Qty.value`, not a passing bare `Qty.value` example).
function blockedBy(ex) {
  return (
    blocklist.find((b) => {
      if (ex.file !== b.file) return false;
      const needles = Array.isArray(b.match) ? b.match : [b.match];
      return needles.every((n) => ex.snippet.includes(n));
    }) ?? null
  );
}

// ---- the compiler (browser wasm) + runner (jco), loaded once ----
const pkgDir = join(guideRoot, "src/wasm/pkg");
const { default: init, compile, render_value, render_syntax, export_types } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");
// Mirror the app run path's scalar formatting (a whole-number Float gets its `.0` back from the static
// result type) so the harness validates the SAME rendered text the browser shows.
const { formatScalarByType, resultTypeOf } = await import(join(guideRoot, "src/runner/scalarFormat.ts"));

// ---- wrapModule / stripModule: the ONE real implementation, imported from the guide source ----
// Previously this harness carried a hand-copy of these — which silently DRIFTED from the app (a bug-(C)
// fix to `wrapModule` would have left the harness testing the OLD wrapping). Import the real module so
// the harness wraps snippets EXACTLY as the app does, by construction. `wrapModule.ts` is React-free
// (its only import is a type), so node loads it directly VIA TYPE-STRIPPING — which needs Node ≥ 22.6
// (on by default) or ≥ 20.19 with --experimental-strip-types. On an older Node the import fails with a
// cryptic "Unknown file extension .ts" loader error; catch it and say exactly what's wrong + how to fix.
let wrapModule, stripModule;
try {
  ({ wrapModule, stripModule } = await import(join(guideRoot, "src/components/wrapModule.ts")));
} catch (e) {
  const msg = String(e && e.message ? e.message : e);
  if (/Unknown file extension|strip.?types|\.ts/i.test(msg)) {
    console.error(
      `\ncheck-examples: cannot load src/components/wrapModule.ts — this Node (${process.version}) doesn't\n` +
        `strip TypeScript types. Use Node ≥ 22.6 (type-stripping on by default), or run with\n` +
        `\`node --experimental-strip-types scripts/check-examples.mjs\` on Node ≥ 20.19.\n` +
        `(underlying error: ${msg})`,
    );
    process.exit(1);
  }
  throw e;
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
// `program`/`surface` (optional) let the SCALAR path recover a whole-number Float's `.0` from the static
// export type — the same fix the app run path applies (see runner/scalarFormat.ts). Omitting them (the
// expect="error" probe) just skips that formatting.
async function runComponent(componentBytes, program, surface) {
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
  if (!fn) return null;
  const value = String(fn());
  if (program == null) return value;
  const resultType = resultTypeOf(export_types(program, surface));
  return formatScalarByType(value, resultType);
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
  // Compiles. Now RUN it (on the s-expr surface — running once per example is enough; the ML pass only
  // guards the wrap/strip round-trip, not a second execution). Compiling is NOT enough: the operator
  // hit an intro example that compiled but CRASHED in the browser ("Maximum call stack size exceeded").
  // A guide example that throws/traps/stack-overflows at RUN time is exactly the trust-breaker the
  // "every example is a test" mandate targets — so every non-error example must reach a value here.
  if (surface === "sexpr") {
    // A graded exercise MUST return a SCALAR. The browser's Check (Exercise.tsx) compares the result
    // rendered in the reader's CURRENT surface, but this harness renders s-expr canonical — a scalar
    // (bare number/bool) reads identically in both, a COMPOUND does NOT (`(: (map …) …)` vs ML
    // `#{…} : Map(…)`). So a compound `expected` would pass here yet FAIL the in-browser Check in ML.
    // Reject it at authoring time; return the compound as a Runnable (ungraded) instead.
    if (ex.expected != null && /^\(:/.test(ex.expected.trim()))
      return `${ex.file} [Exercise] (${where}): \`expected\` is a COMPOUND value (${JSON.stringify(ex.expected.slice(0, 40))}…) — graded exercises must return a SCALAR (it's compared in the reader's surface, and a compound renders differently in ML vs s-expr). Show the compound as a Runnable instead.\n    ${brief}`;
    let got;
    try {
      got = await runComponent(r.component, program, surface);
    } catch (e) {
      // A run failure is the trust-breaker: a compiled example that crashes/traps/stack-overflows.
      const label = ex.expected != null ? "solution" : ex.kind;
      return `${ex.file} [${ex.kind}] (${where}): ${label} compiled but FAILED TO RUN — ${String(e.message || e).slice(0, 100)}\n    ${brief}`;
    }
    // A graded exercise additionally asserts the rendered scalar equals its stated `expected`.
    if (ex.expected != null && String(got).trim() !== ex.expected.trim())
      return `${ex.file} [Exercise] (${where}): solution ran to ${JSON.stringify(String(got))}, expected ${JSON.stringify(ex.expected)}\n    ${brief}`;
  }
  return null;
}

// ---- check one example in BOTH surfaces (the reader can toggle); null on success, else a reason ----
async function checkExample(ex) {
  // 1. s-expr — the authored surface.
  const sexprProgram = ex.noWrap ? ex.snippet.trim() : wrapModule(ex.snippet, "sexpr");
  const sexprFail = await checkProgram(sexprProgram, "sexpr", ex, "s-expr");
  if (sexprFail) return sexprFail;

  // 2. ML — what the reader sees after toggling. Render the snippet to ML, then wrap + compile THAT.
  //    This catches wrap/strip round-trip bugs that only bite on the ML surface (e.g. a `;`-in-a-
  //    do-block snippet whose wrapper skipped the export). `noWrap` snippets are full modules already.
  if (!ex.noWrap) {
    try {
      const mlProgram = wrapModule(renderToMl(ex.snippet), "ml");
      const mlFail = await checkProgram(mlProgram, "ml", ex, "ML toggle");
      if (mlFail) return mlFail;
    } catch (e) {
      return `${ex.file} [${ex.kind}] (ML toggle): render/wrap threw — ${String(e.message || e).slice(0, 80)}`;
    }
  }
  return null;
}

let pass = 0;
const failures = []; // real, unexpected failures — these FAIL the gate.
const stillBlocked = []; // known-blocked examples that (correctly) still fail — reported, not fatal.
const recovered = []; // blocklist entries that now PASS — the entry should be removed + the example ships.
const matchedEntries = new Set(); // blocklist entries that matched ≥1 example (to find stale ones).
for (const ex of examples) {
  const block = blockedBy(ex);
  const fail = await checkExample(ex);
  if (block) {
    matchedEntries.add(block);
    // A known-blocked example: it's EXPECTED to fail until its owner fixes the root cause.
    if (fail) stillBlocked.push({ block, ex });
    else recovered.push({ block, ex }); // it started passing — un-block it.
    continue;
  }
  if (fail) { failures.push(fail); continue; }
  pass++;
}
// A blocklist entry that matched NO example is stale — the example was renamed/removed/rewritten so the
// entry no longer identifies anything. Flag it (loud, not fatal) so the blocklist doesn't rot silently.
const staleEntries = blocklist.filter((b) => !matchedEntries.has(b));

console.log(
  `\nchecked ${examples.length} examples across ${files.length} files (both surfaces): ` +
    `${pass} ok, ${failures.length} failed, ${stillBlocked.length} known-blocked, ${recovered.length} recovered`,
);

if (stillBlocked.length) {
  // Group by blocklist entry so one root cause reports once (with its example count), not N times.
  const byEntry = new Map();
  for (const { block } of stillBlocked) byEntry.set(block, (byEntry.get(block) ?? 0) + 1);
  console.log("\nKNOWN-BLOCKED (routed to their owner; kept OUT of the shipped guide until green):");
  for (const [block, n] of byEntry) {
    console.log(
      `  ⏸ ${block.file} (${n} example${n > 1 ? "s" : ""}) — ${block.kind} bug, owner ${block.owner}: ${block.reason}`,
    );
  }
}

if (recovered.length) {
  // A blocked example started passing — the root cause landed. Tell the operator to un-block it.
  // This is NOT fatal (the fix is good news), but it's LOUD so the blocklist doesn't rot.
  console.log(
    "\n✅ BLOCKLIST ENTRY CAN BE REMOVED (these examples now RUN — delete their blocklist entry so they ship):\n" +
      recovered
        .map(({ block, ex }) => `  ✔ ${block.file} [${ex.kind}] "${block.match}" — was: ${block.reason}`)
        .join("\n"),
  );
}

if (staleEntries.length) {
  // A blocklist entry that identifies no current example — the example was rewritten/renamed/removed.
  // Loud so the entry gets deleted; NOT fatal (a stale block is harmless, just clutter).
  console.log(
    "\n⚠️  STALE BLOCKLIST ENTRY (matches no current example — delete it from example-blocklist.json):\n" +
      staleEntries
        .map((b) => `  ⚠ ${b.file} "${JSON.stringify(b.match)}" — ${b.reason}`)
        .join("\n"),
  );
}

if (failures.length) {
  console.error("\nFAILURES:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log(
  "✓ every guide example compiles + runs in both surfaces (graded exercises to their expected value); " +
    "known-blocked examples are tracked + routed.",
);
