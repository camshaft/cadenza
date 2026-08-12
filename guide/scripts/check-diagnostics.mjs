#!/usr/bin/env node
/// DIAGNOSTIC-MESSAGE conformance: pin the EXACT compiler diagnostics that guide PROSE quotes, so a future
/// diagnostic reword can't silently drift the chapter text (a green gate otherwise can't catch prose-vs-message
/// drift — expect="error" Runnables assert only THAT a snippet declines, not WHAT it says). This is that guard.
///
/// The SizedIntegers "Why isn't Int a type?" section leans on the bare-`Int` CDZ0203 wording; it drifted three
/// times (#1066/#1093/#1117) plus the message itself changed (v-diagnostics #1111). This probe compiles the
/// exact snippet the chapter shows and asserts the diagnostic still SAYS what the prose claims — by load-bearing
/// PHRASE, not full-string (a benign reword, e.g. reordering the width list, must NOT false-fail; dropping
/// "width constructor"/"Int64" MUST fail so the prose gets updated in lockstep).
///
/// Run: `npm run check:diagnostics` (needs staged wasm — `cargo xtask guide-wasm` first). Node ≥ 20.19.
/// Reuses the REAL compile + wrapModule the app/check-examples use (no private copy — the gate must match ship).

import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const pkgDir = join(guideRoot, "src/wasm/pkg");

const { default: init, compile } = await import(pathToFileURL(join(pkgDir, "cdz_wasm.js")).href);
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });
// `wrapModule.ts` is loaded directly VIA TYPE-STRIPPING — needs Node ≥ 22.6 (on by default) or ≥ 20.19 with
// --experimental-strip-types. On an older Node the import fails with a cryptic "Unknown file extension .ts"
// loader error; catch it and say exactly what's wrong + how to fix (mirrors check-examples.mjs's guard).
let wrapModule;
try {
  ({ wrapModule } = await import(pathToFileURL(join(guideRoot, "src/components/wrapModule.ts")).href));
} catch (e) {
  const msg = String(e && e.message ? e.message : e);
  if (/Unknown file extension|strip.?types|\.ts/i.test(msg)) {
    console.error(
      `\ncheck-diagnostics: cannot load src/components/wrapModule.ts — this Node (${process.version}) doesn't\n` +
        `strip TypeScript types. Use Node ≥ 22.6 (type-stripping on by default), or run with\n` +
        `\`node --experimental-strip-types scripts/check-diagnostics.mjs\` on Node ≥ 20.19.\n` +
        `(underlying error: ${msg})`,
    );
    process.exit(1);
  }
  throw e;
}

/// Each PIN: a snippet that MUST decline, its expected diagnostic code, and the load-bearing phrases its message
/// MUST contain (case-insensitive substring). `chapter` documents which prose depends on it. Add a pin whenever a
/// chapter QUOTES or PARAPHRASES a specific diagnostic's wording. `noWrap: true` compiles the snippet AS-IS (a
/// TOP-LEVEL construct like a `(world …)` declaration can't sit inside the `(def (main) …)` wrapper — wrapping it
/// would fire the wrong diagnostic, e.g. unbound-name, instead of the top-level rule the prose depends on).
const PINS = [
  {
    name: "bare Int in type position → CDZ0203 width-constructor",
    chapter: "SizedIntegers.tsx — 'Why isn't Int a type?'",
    snippet: `(def (f (: a Int)) a)`,
    code: "CDZ0203",
    phrases: ["width constructor", "Int64"],
  },
  {
    name: "missing match arm → CDZ0210 non-exhaustive",
    // Ordering.tsx quotes the EXACT string "non-exhaustive match: pattern `Greater` not covered"; PatternMatching.tsx
    // leans on "non-exhaustive match" too. A reworded exhaustiveness message would silently drift that prose.
    chapter: "Ordering.tsx — 'The Ordering value' / PatternMatching.tsx",
    snippet: `(match (compare 3 9)
  ((Less _) 1)
  ((Equal _) 0))`,
    code: "CDZ0210",
    phrases: ["non-exhaustive match", "not covered"],
  },
  {
    name: "function as map/set key → CDZ0216 no-canonical-identity",
    // Ordering.tsx "What can't be a key: a function" quotes the load-bearing phrases "map/set key" and
    // "no canonical identity" / "neither equatable nor orderable". A reworded non-equatable message would
    // silently drift that prose (same class as the CDZ0203 / CDZ0210 / CDZ0301 pins).
    chapter: "Ordering.tsx — 'What can't be a key: a function'",
    snippet: `(do (def (main)
      (Set.len (Set.of (list (fn (x) (+ x 1))))))
    (export main))`,
    code: "CDZ0216",
    phrases: ["map/set key", "no canonical identity", "neither equatable nor orderable"],
  },
  {
    name: "mixed float widths → CDZ0301 precisions differ",
    // Floats.tsx quotes the exact phrase "floating-point precisions differ" as the diagnostic text. A reworded
    // width-mismatch message would silently drift that prose (same class as the CDZ0203 / CDZ0210 pins).
    chapter: "Floats.tsx — the no-silent-widening Note",
    snippet: `(+ (Float32.of 1.0) (Float64.of 2.0))`,
    code: "CDZ0301",
    phrases: ["floating-point precisions differ"],
  },
  {
    name: "perform in a match-arm guard → CDZ0407 EffectInGuard",
    // Effects.tsx "A guard must be side-effect-free" quotes the load-bearing phrases "side-effect-free" and
    // "speculatively or repeatedly", and shows the fix the message names (lift to a `let` before the `match`).
    // A reworded guard-purity message would silently drift that prose (same class as the CDZ0203/CDZ0210/CDZ0216
    // pins). Verified through the cdz_wasm path: declines with exactly one CDZ0407 diagnostic.
    chapter: "Effects.tsx — 'A guard must be side-effect-free'",
    snippet: `(effect Ask (op ask (-> Unit Int64)))
(def (main)
  (handle Ask unit
    ((ask () s (resume 5 s)))
    (match 3
      ((guard x (< x (Ask.ask))) 1)
      (_ 0))))`,
    code: "CDZ0407",
    phrases: ["side-effect-free", "speculatively or repeatedly", "lift it to a `let`"],
  },
  {
    name: "two top-level world decls → CDZ0201 at-most-one-world",
    // Modules.tsx "Declaring the world a module targets" states a module may declare AT MOST ONE top-level
    // world (a reducer targets a single world). This is a TOP-LEVEL rule, so the snippet is noWrap — wrapping
    // two `(world …)` forms inside `(def (main) …)` fires unbound-name/more-than-one-body instead of the rule.
    chapter: "Modules.tsx — 'Declaring the world a module targets'",
    snippet: `(world Reducer (export fold (member apply (func (param event Bytes) (result Bytes)))))
(world Other (export fold (member apply (func (param event Bytes) (result Bytes)))))`,
    code: "CDZ0201",
    phrases: ["at most one world", "top-level"],
    noWrap: true,
  },
];

// Vacuous-pass floor: this gate's whole job is to pin diagnostics, so a run with ZERO pins (a botched edit
// or bad merge that empties PINS) must FAIL, not silently print "✓ … 0 pinned" and exit 0 — that false
// green is exactly what a conformance gate must never do (cf. the check:examples/prose/music-preload floors).
if (PINS.length === 0) {
  console.error("\n✗ diagnostic-message conformance: PINS is EMPTY — nothing is being pinned. This gate must pin ≥1 diagnostic; refusing a vacuous pass.\n");
  process.exit(1);
}

const failures = [];
for (const pin of PINS) {
  // A top-level construct (e.g. a `(world …)` decl) is compiled AS-IS; everything else wraps in `(def (main) …)`.
  const program = pin.noWrap ? pin.snippet : wrapModule(pin.snippet, "sexpr");
  let r;
  try { r = compile(program, "sexpr"); }
  catch (e) { failures.push(`${pin.name}: expected a ${pin.code} DIAGNOSTIC but parse threw — ${String(e.message || e).slice(0, 80)}`); continue; }
  if (r.component) { failures.push(`${pin.name}: expected to DECLINE (${pin.code}) but it compiled — the diagnostic no longer fires; update ${pin.chapter}`); continue; }
  const diags = r.diagnostics ?? [];
  const d = diags.find((x) => x.code === pin.code) ?? diags.find((x) => x.error) ?? diags[0];
  if (!d) { failures.push(`${pin.name}: declined but produced NO diagnostic to match against`); continue; }
  if (d.code !== pin.code) { failures.push(`${pin.name}: expected code ${pin.code}, got ${d.code} — "${d.message}"; update ${pin.chapter}`); continue; }
  const msg = String(d.message ?? "");
  const missing = pin.phrases.filter((p) => !msg.toLowerCase().includes(p.toLowerCase()));
  if (missing.length) {
    failures.push(`${pin.name}: ${pin.code} message no longer contains [${missing.join(", ")}] — MESSAGE DRIFTED, update ${pin.chapter} to match:\n    now: "${msg}"`);
  }
}

if (failures.length) {
  console.error(`\n✗ diagnostic-message conformance FAILED (${failures.length}) — a compiler diagnostic the guide prose quotes has drifted:\n  ${failures.join("\n  ")}\n`);
  process.exit(1);
}
console.log(`✓ diagnostic-message conformance: ${PINS.length} pinned diagnostic(s) still fire with the wording the guide prose depends on.`);
