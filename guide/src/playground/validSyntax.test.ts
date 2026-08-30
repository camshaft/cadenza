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
};

const wasmPath = fileURLToPath(new URL("../wasm/pkg/cdz_wasm_bg.wasm", import.meta.url));

if (!existsSync(wasmPath)) {
  test("valid-syntax guard SKIPPED — browser wasm not staged (run `npm run wasm`)", { skip: true }, () => {});
} else {
  const { default: init, diagnostics } = await import("../wasm/pkg/cdz_wasm.js");
  await init({ module_or_path: readFileSync(wasmPath) });

  for (const [name, src] of Object.entries(VALID_ML)) {
    test(`editor does not squiggle valid ML syntax: ${name}`, () => {
      const errors = (diagnostics(src, "ml") as { error: boolean; code?: string; message: string }[]).filter(
        (d) => d.error,
      );
      assert.deepEqual(
        errors.map((d) => `${d.code ?? ""}: ${d.message}`),
        [],
        `"${name}" produced editor error diagnostic(s) on valid syntax — the guide would show a red squiggle`,
      );
    });
  }
}
