/// End-to-end check for the browser calculator engine (C4). Drives the SAME pipeline the page uses —
/// compile the `let`-wrapped expression via the staged `cdz-wasm` pkg, run the component through jco —
/// for a handful of scenarios that exercise the calculator's reason for existing (exact rationals,
/// units, big integers) and its state model (variables, `ans`, self-referential re-bind via `let`
/// shadowing). Mirrors `check-examples.mjs`'s compile+run plumbing.
///
/// This validates the engine's `wrapInLets` model against the real compiler+runtime WITHOUT a browser —
/// the logic that `CalculatorPage` drives. Run: `node scripts/check-calculator.mjs` (needs a staged pkg
/// + runtime, i.e. `cargo xtask guide-wasm` first; Node ≥20.19 for jco).

import { readFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(here, "..", "src", "wasm", "pkg");

const { default: init, compile, repl_eval, render_value, render_syntax } = await import(join(pkgDir, "cdz_wasm.js"));
await init(readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")));
const { transpileBytes } = await import("@bytecodealliance/jco-transpile");

// The SAME mixed-number transform the browser engine applies to a rational result (`47/12` → `3 + 11/12`,
// operator's Q-b ruling). Imported from source so the round-trip gate below pins the ACTUAL render, not a
// re-spelling of it.
const { toMixed } = await import(join(here, "..", "src", "calculator", "mixed.ts"));

const runtimeBytes = readFileSync(join(here, "..", "src", "wasm", "runtime.wasm"));

// FINDING#23: the value-heap runtime imports `cadenza:nfc/normalize` (a separate NFC component; the Unicode
// tables live there, not in the runtime). Supply it as a JS shim so the runtime instantiates — NFC of
// well-formed UTF-8 is String.prototype.normalize('NFC') over the `list<u8>` (Uint8Array) boundary.
const NFC_IMPORT = "cadenza:nfc/normalize";
const nfcHostImport = {
  nfc: (bytes) => new TextEncoder().encode(new TextDecoder("utf-8").decode(bytes).normalize("NFC")),
};

/// Load a transpiled component from a temp dir, writing ONLY the runtime files jco needs (`.wasm`/`.js`)
/// — NOT the `interfaces/*.d.ts` TypeScript declarations, which the entry `.js` imports for types only
/// and which crash a Node `import()` if absent-but-referenced. This mirrors the real run worker
/// (`runWorker.ts`), which loads `.wasm`/`.js` from blob URLs and ignores `.d.ts`. Node ≥20.19 (jco).
async function loadComponent(bytes, name) {
  const { files } = await transpileBytes(new Uint8Array(bytes), {
    name,
    instantiation: "async",
    wasiShim: false,
    minify: false,
  });
  const dir = mkdtempSync(join(tmpdir(), `cdz-calc-${name}-`));
  for (const [f, b] of Object.entries(files)) {
    if (f.endsWith(".d.ts")) continue; // types only — the run worker never writes/loads these
    const dest = join(dir, f);
    writeFileSync(dest, b);
  }
  const mod = await import(join(dir, `${name}.js`));
  const getCore = async (p) => WebAssembly.compile(readFileSync(join(dir, p)));
  return { mod, getCore };
}

/// Instantiate + run a compiled component through jco, composing the value-heap runtime if the program
/// imports it. Returns the canonical value text (`(: 1/1 Rational)`, `(: 1500 …)`, or a bare scalar).
async function runComponent(componentBytes) {
  // Bind the value-heap runtime under its import name if the program needs it (a compound/Rational
  // result escapes via the heap). A scalar program imports nothing → the empty import object is fine.
  let imports = {};
  const rt = await loadComponent(runtimeBytes, "heap");
  const rroot = await rt.mod.instantiate(rt.getCore, { [NFC_IMPORT]: nfcHostImport });
  const heapIface = rroot["cadenza:runtime/heap"] ?? rroot["heap"];

  const prog = await loadComponent(componentBytes, "prog");
  // Discover the program's heap import name (a hashed `cadenza:runtime/heap@<hash>`), bind the runtime.
  imports = heapIface ? { "cadenza:runtime/heap": heapIface } : {};
  let root;
  try {
    root = await prog.mod.instantiate(prog.getCore, imports);
  } catch {
    // The import name may be hashed; retry binding under every plausible key the program declares.
    root = await prog.mod.instantiate(prog.getCore, { heap: heapIface });
  }
  const iface = root["cadenza:run/run"] ?? root["run"];
  if (iface && typeof iface.make === "function") {
    return render_value(iface.encode(iface.make()));
  }
  const fn = Object.values(root).find((v) => typeof v === "function");
  return fn ? String(fn()) : null;
}

/// Mirror the engine's wrap_in_lets (ML): nest `let name = src in …`, oldest outermost.
function wrapInLetsMl(bindings, expr) {
  let wrapped = expr;
  for (let i = bindings.length - 1; i >= 0; i--) {
    wrapped = `let ${bindings[i][0]} = ${bindings[i][1]} in ${wrapped}`;
  }
  return wrapped;
}

/// Compile + run one ML expression, returning the rendered value or throwing on decline. The plain path
/// wraps it as `def main() = <expr>`. This checks the ordinary (non-exact) calculator path.
async function evalMl(expr) {
  const out = compile(`def main() = ${expr}\nexport { main }`, "ml");
  if (!out.component) {
    const err = (out.diagnostics || []).find((d) => d.error);
    throw new Error(err ? `${err.code || ""} ${err.message}`.trim() : "declined");
  }
  return await runComponent(out.component);
}

/// Compile + run an EXACT-MODE expression: an S-EXPR program wrapping the (s-expr) expression in the same
/// do-local `(pragma default-fraction Rational)` module `assemble_repl_program_exact` builds, so bare
/// literals ground to Rational. Built in s-expr (unambiguous) — this checks that the exact wrapper the
/// calculator emits actually makes `(/ 1 3)` exact. Mirrors `repl_eval(exact=true)`'s assembly.
async function evalExactSexpr(sexprExpr) {
  // The module is do-local INSIDE main's body (a top-level module sibling is not visible to `main`) —
  // exactly the shape `assemble_repl_program_exact` builds.
  const prog = `(do (def (main) (do (module Exact (pragma default-fraction Rational) (def (v) ${sexprExpr})) ((. Exact v) unit))) (export main))`;
  const out = compile(prog, "sexpr");
  if (!out.component) {
    const err = (out.diagnostics || []).find((d) => d.error);
    throw new Error(err ? `${err.code || ""} ${err.message}`.trim() : "declined");
  }
  return await runComponent(out.component);
}

/// Evaluate an ML expression through the EXACT calculator path — `repl_eval(buffer, expr, "ml", true)`,
/// the exact call the browser engine makes (buffer `"0"`, all state in the expr) — and return the
/// display-rendered value. This is the real pipeline a pasted-back mixed number travels, so it's what the
/// round-trip gate must use (not the s-expr helper).
async function evalExactMlDisplay(mlExpr) {
  const out = repl_eval("0", mlExpr, "ml", true);
  if (!out.component) {
    const err = (out.diagnostics || []).find((d) => d.error);
    throw new Error(err ? `${err.code || ""} ${err.message}`.trim() : "declined");
  }
  return await runComponent(out.component);
}

let pass = 0;
let fail = 0;
async function check(label, expr, bindings, wantSubstr) {
  const wrapped = wrapInLetsMl(bindings, expr);
  try {
    const got = await evalMl(wrapped);
    const ok = got != null && String(got).includes(wantSubstr);
    console.log(`${ok ? "ok  " : "FAIL"}  ${label}: ${wrapped}  =>  ${got}`);
    ok ? pass++ : fail++;
  } catch (e) {
    console.log(`FAIL  ${label}: ${wrapped}  =>  threw ${e.message}`);
    fail++;
  }
}

// --- the scenarios (ML surface) ---
// Exact rationals — the marquee. NOTE the suffix precedence: `1R / 3R` is one-third (each literal
// suffixed, then rational `/`); `1/3R` would be `1 ÷ 3R` = an Int64/Rational mismatch (CDZ0301).
await check("exact thirds (suffix)", "1R / 3R + 1R / 3R + 1R / 3R", [], "1/1");
await check("rational add", "Rational.of(1, 3) + Rational.of(1, 6)", [], "1/2");
// A variable, recalled.
await check("var recall", "x * x", [["x", "5"]], "25");
// ans composition (the innermost let shadows — the regression that stack-overflowed natively).
await check("ans + 5", "ans + 5", [["ans", "20"]], "25");
// Self-referential re-bind via let shadowing: outer n=1, inner n=n+1 → 2.
await check("counter n=n+1", "n", [["n", "1"], ["n", "n + 1"]], "2");
// A variable holding a rational, composed.
await check("rational var", "r + r + r", [["r", "1R / 3R"]], "1/1");
// A plain scalar.
await check("scalar", "2 + 3", [], "5");

// --- EXACT MODE (C6b): a bare literal grounds to Rational, so `1 / 3` is `1/3` with NO `R` suffix ---
async function checkExact(label, sexprExpr, wantSubstr) {
  try {
    const got = await evalExactSexpr(sexprExpr);
    const ok = got != null && String(got).includes(wantSubstr);
    console.log(`${ok ? "ok  " : "FAIL"}  exact ${label}: ${sexprExpr}  =>  ${got}`);
    ok ? pass++ : fail++;
  } catch (e) {
    console.log(`FAIL  exact ${label}: ${sexprExpr}  =>  threw ${e.message}`);
    fail++;
  }
}
// THE MARQUEE: bare `1 / 3` is exact 1/3 (not integer-truncated 0) — forced rationals by default.
await checkExact("bare 1/3 is exact", "(/ 1 3)", "1/3");
// Bare thirds sum to exactly 1.
await checkExact("bare thirds sum to 1", "(+ (+ (/ 1 3) (/ 1 3)) (/ 1 3))", "1/1");
// A bare decimal grounds to its exact fraction (0.1 + 0.2 = 3/10, not 0.30000…004).
await checkExact("bare decimal exact", "(+ 0.1 0.2)", "3/10");

// --- MIXED-NUMBER ROUND-TRIP (operator Q-b: `47/12` shows as explicit-plus `3 + 11/12`) ---
// The load-bearing property of the ruling: the rendered mixed form is itself VALID Cadenza that
// re-evaluates to the SAME rational. Pin it against the real compiler: evaluate an improper rational,
// render it with the ACTUAL `toMixed` transform (imported from source), feed that string BACK through the
// exact ML path, and assert the paste-back yields the same canonical value. This is what stops the render
// from ever drifting into a form that doesn't re-parse (a plain-space `3 11/12` would fail here).
async function checkMixedRoundTrip(label, mlExpr) {
  try {
    const canonical = String(await evalExactMlDisplay(mlExpr)); // e.g. "(: 13/4 Rational)"
    // The display surface the engine renders is the bare rational inside the `(: … Rational)` wrapper.
    const bare = (canonical.match(/(-?\d+\/\d+)/) || [])[1];
    if (!bare) throw new Error(`no bare rational in ${canonical}`);
    const mixed = toMixed(bare);
    if (!mixed.includes(" + ")) throw new Error(`toMixed did not produce a mixed form: ${bare} -> ${mixed}`);
    // Paste the mixed render back in as a fresh line.
    const roundTripped = String(await evalExactMlDisplay(mixed));
    const ok = roundTripped === canonical;
    console.log(`${ok ? "ok  " : "FAIL"}  mixed round-trip ${label}: ${mlExpr} => ${bare} => "${mixed}" => ${roundTripped}`);
    ok ? pass++ : fail++;
  } catch (e) {
    console.log(`FAIL  mixed round-trip ${label}: ${mlExpr} => threw ${e.message}`);
    fail++;
  }
}
await checkMixedRoundTrip("positive 13/4", "13R / 4R");
await checkMixedRoundTrip("positive 47/12", "47R / 12R");
await checkMixedRoundTrip("negative -13/4", "-13R / 4R");

console.log(`\ncalculator check: ${pass} pass, ${fail} fail`);
// Vacuous-pass floor: the scenarios are inline `await check(...)` calls, so if a refactor ever dropped
// them the loop would run zero cases and print "0 pass, 0 fail" then exit 0 — a false green. Assert the
// harness actually exercised the calculator engine. (Completes the vacuous-pass audit across the guide's
// gate scripts — cf. check-examples / check-prose / check-music-preload floors.)
if (pass + fail === 0) {
  console.error("calculator check: ZERO scenarios ran — the check body likely broke; refusing a vacuous pass.");
  process.exit(1);
}
process.exit(fail === 0 ? 0 : 1);
