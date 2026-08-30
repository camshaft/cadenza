/// Guard the operator's bar (seq-266): the guide editor must NEVER red-squiggle syntax the compiler
/// accepts. Editor squiggles come from the browser compiler wasm's `diagnostics` (mapped by cadenzaLint) —
/// NOT the colorizer (src/editor/cadenzaLanguage.ts, which only tones tokens) — so this asserts the wasm
/// `diagnostics` path reports NO error for a battery of valid, current-syntax constructs the guide teaches.
/// Since the wasm is built from the same rcdzc frontend as the CLI, a construct that squiggles here would be
/// a real editor/compiler regression (or a stale wasm) — this catches it in the gate, so the highlighter can
/// never silently drift stale vs the grammar again.
///
/// Loads the staged browser wasm (guide/src/wasm/pkg — staged before test:unit in the nix gate via
/// stage-wasm.mjs; locally after `npm run wasm`). If it isn't staged, the suite SKIPS (so a pure-logic
/// `npm run test:unit` without wasm staging doesn't fail) — but it RUNS in the gate, where wasm is present.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Valid ML constructs the guide uses — each a complete module that compiles clean. Kept to SYNTAX (no
// stdlib-API-specific calls) so a stdlib rename can't make this guard flap; it pins that the grammar/parser
// + the editor's diagnostics path accept these shapes. record-destructure-let is the operator's seq-266 case.
const VALID_ML: Record<string, string> = {
  "record-destructure let": `def main() = let { x, y } = { x = 3, y = 4 } in x + y\nexport { main }`,
  "tuple-destructure let": `def main() = let (a, b) = (3, 4) in a + b\nexport { main }`,
  "record literal + field access": `def main() = { x = 1, y = 2 }.x\nexport { main }`,
  "match expression": `def main() = match 3 with | 0 => 1 | _ => 2\nexport { main }`,
  "if / then / else": `def main() = if true then 1 else 2\nexport { main }`,
  "pipe to a named fn": `def inc(n) = n + 1\ndef main() = 5 |> inc\nexport { main }`,
  "lambda literal applied": `def main() = (fn(x) => x * 2)(21)\nexport { main }`,
  "list literal": `def main() = [1, 2, 3]\nexport { main }`,
  "user sum type + constructor + match": `type Color = | Red | Green\ndef main() = match Red with | Red => 1 | Green => 2\nexport { main }`,
  // v-syntax's suggested additions (grammar-expert review of the seq-266 battery), verified clean:
  "BigInt literal (N suffix)": `def main() = 100N\nexport { main }`,
  "Rational literal (R suffix)": `def main() = 0.5R\nexport { main }`,
  "@test annotation above a def": `@test\ndef t() = 1 == 1\nexport { t }`,
  "@tag annotation above a def": `@tag("slow")\ndef t() = 1 == 1\nexport { t }`,
};

// Native SEXPR compound literals + patterns — the playground's authored surface (all 59 examples are sexpr),
// so a squiggle here would break the dropdown programs. Also from v-syntax's battery review. (The `(.. v)`
// spread surface is deferred — v-syntax will ping when Phase 2 lands so it can be added.)
const VALID_SEXPR: Record<string, string> = {
  "native list literal #list": `(do (def (main) #list(1 2 3)) (export main))`,
  "native tuple literal #tuple": `(do (def (main) #tuple(1 2)) (export main))`,
  "native record literal #record": `(do (def (main) #record((= a 1) (= b 2))) (export main))`,
  "native map literal #map": `(do (def (main) #map((= 1 10))) (export main))`,
  "native set literal #set": `(do (def (main) #set(1 2 3)) (export main))`,
  "tuple pattern in match": `(do (def (main) (match #tuple(1 2) (#tuple(a b) (+ a b)))) (export main))`,
  "record pattern in let": `(do (def (main) (let (((record (= a x) (= b y)) #record((= a 1) (= b 2)))) (+ x y))) (export main))`,
  "single-arm irrefutable match": `(do (def (main) (match #tuple(3 4) (#tuple(a b) (+ a b)))) (export main))`,
};

const wasmPath = fileURLToPath(new URL("../wasm/pkg/cdz_wasm_bg.wasm", import.meta.url));

if (!existsSync(wasmPath)) {
  test("valid-syntax guard SKIPPED — browser wasm not staged (run `npm run wasm`)", { skip: true }, () => {});
} else {
  const { default: init, diagnostics } = await import("../wasm/pkg/cdz_wasm.js");
  await init({ module_or_path: readFileSync(wasmPath) });

  const assertClean = (name: string, src: string, surface: "ml" | "sexpr") => {
    const errors = (diagnostics(src, surface) as { error: boolean; code?: string; message: string }[]).filter(
      (d) => d.error,
    );
    assert.deepEqual(
      errors.map((d) => `${d.code ?? ""}: ${d.message}`),
      [],
      `"${name}" produced editor error diagnostic(s) on valid syntax — the guide would show a red squiggle`,
    );
  };

  for (const [name, src] of Object.entries(VALID_ML)) {
    test(`editor does not squiggle valid ML syntax: ${name}`, () => assertClean(name, src, "ml"));
  }
  for (const [name, src] of Object.entries(VALID_SEXPR)) {
    test(`editor does not squiggle valid sexpr syntax: ${name}`, () => assertClean(name, src, "sexpr"));
  }
}
