/// Unit tests for applyFix — the byte-domain splice that turns a compiler DiagFix into edited text.
/// This logic is easy to get subtly wrong (byte vs char offsets, wrap-hole substitution, the
/// wrapPrefixBytes remap, out-of-range rejection), and it's shared by every fix affordance, so it's
/// worth pinning. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { applyFix, fixConfidence, fixIsApplicable } from "./applyFix.ts";
import type { DiagFix } from "../compiler/client.ts";

function fix(partial: Partial<DiagFix>): DiagFix {
  return { kind: "replace", replacement: "", from: 0, to: 0, verified: false, ...partial };
}

test("replace swaps the byte range for the replacement", () => {
  // "(+ 2 3)" — replace "2" (bytes [3,4)) with "20".
  assert.equal(applyFix("(+ 2 3)", fix({ kind: "replace", replacement: "20", from: 3, to: 4 })), "(+ 20 3)");
});

test("wrap substitutes the U+2026 hole with the original range text", () => {
  // wrap the "x" (bytes [6,7)) as "(Some …)".
  assert.equal(
    applyFix("(Some x)", fix({ kind: "wrap", replacement: "(Some …)", from: 6, to: 7 })),
    "(Some (Some x))",
  );
});

test("insert splices before the target list's closing paren", () => {
  // target is the whole "(match m)" list [0,9); insert "(_ 0)" before the final ")".
  assert.equal(
    applyFix("(match m)", fix({ kind: "insert", replacement: "(_ 0)", from: 0, to: 9 })),
    "(match m (_ 0))",
  );
});

test("insert with no closing paren falls back to appending (defensive)", () => {
  assert.equal(
    applyFix("abc", fix({ kind: "insert", replacement: "X", from: 0, to: 3 })),
    "abc X",
  );
});

test("byte offsets stay exact across multi-byte characters", () => {
  // "café x" — é is 2 UTF-8 bytes, so "x" starts at BYTE 6 (not char 5). Replace it with "y".
  const text = "café x";
  assert.equal(new TextEncoder().encode(text).length, 7); // c a f é(2) space x
  assert.equal(applyFix(text, fix({ kind: "replace", replacement: "y", from: 6, to: 7 })), "café y");
});

test("wrapPrefixBytes maps a compiled-text range back onto the editor text", () => {
  // Editor text "2 + 3"; compiled as "def main() = 2 + 3" (prefix 13 bytes). A fix targeting the "2"
  // at compiled bytes [13,14) maps to editor bytes [0,1).
  const prefix = new TextEncoder().encode("def main() = ").length;
  assert.equal(prefix, 13);
  assert.equal(
    applyFix("2 + 3", fix({ kind: "replace", replacement: "20", from: 13, to: 14 }), prefix),
    "20 + 3",
  );
});

test("a fix targeting the scaffolding (range outside the editor text) is rejected", () => {
  // A range that, after subtracting the prefix, is negative — it targets generated glue.
  assert.equal(applyFix("2 + 3", fix({ kind: "replace", replacement: "x", from: 2, to: 3 }), 10), null);
  // A range past the end of the editor content is rejected too.
  assert.equal(applyFix("abc", fix({ kind: "replace", replacement: "x", from: 2, to: 99 })), null);
  // An inverted range (to < from) is rejected.
  assert.equal(applyFix("abc", fix({ kind: "replace", replacement: "x", from: 2, to: 1 })), null);
});

test("fixConfidence reflects the verified flag", () => {
  assert.equal(fixConfidence(fix({ verified: true })), "Verified");
  assert.equal(fixConfidence(fix({ verified: false })), "Suggested");
});

test("fixIsApplicable gates on s-expr surface + a non-degenerate range", () => {
  assert.equal(fixIsApplicable(fix({ from: 1, to: 3 }), "sexpr"), true);
  assert.equal(fixIsApplicable(fix({ from: 1, to: 3 }), "ml"), false); // ML spans not canonical yet
  assert.equal(fixIsApplicable(fix({ from: 0, to: 0 }), "sexpr"), false); // degenerate (0,0) range
});
