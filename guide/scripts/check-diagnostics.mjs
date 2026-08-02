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
const { wrapModule } = await import(pathToFileURL(join(guideRoot, "src/components/wrapModule.ts")).href);

/// Each PIN: a snippet that MUST decline, its expected diagnostic code, and the load-bearing phrases its message
/// MUST contain (case-insensitive substring). `chapter` documents which prose depends on it. Add a pin whenever a
/// chapter QUOTES or PARAPHRASES a specific diagnostic's wording.
const PINS = [
  {
    name: "bare Int in type position → CDZ0203 width-constructor",
    chapter: "SizedIntegers.tsx — 'Why isn't Int a type?'",
    snippet: `(def (f (: a Int)) a)`,
    code: "CDZ0203",
    phrases: ["width constructor", "Int64"],
  },
];

const failures = [];
for (const pin of PINS) {
  const program = wrapModule(pin.snippet, "sexpr");
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
