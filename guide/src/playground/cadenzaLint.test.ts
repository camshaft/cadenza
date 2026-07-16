/// Unit tests for `toCmDiagnostics` — mapping Cadenza compiler diagnostics (UTF-8 byte spans over the
/// COMPILED text) to CodeMirror lint ranges (UTF-16 offsets over the EDITOR text). The span arithmetic
/// (wrapper-prefix subtraction, drop-if-inside-wrapper, clamp-into-content, non-empty-mark, the
/// unanchored 0,0 case, and UTF-8→UTF-16 for astral chars) decides WHERE a squiggle lands, so a bug here
/// mis-marks tokens — the same failure family as the Runnable wrapper-prefix work. Run: npm run test:unit.

import { test } from "node:test";
import assert from "node:assert/strict";
import { toCmDiagnostics } from "./cadenzaLint.ts";
import type { Diag, DiagFix, Surface } from "../compiler/client.ts";

const diag = (over: Partial<Diag> = {}): Diag => ({
  error: true,
  code: "CDZ0101",
  message: "unbound name",
  node: 0,
  from: 0,
  to: 0,
  fix: null,
  ...over,
});

test("verbatim editor text (no wrapper): byte spans pass through as UTF-16 offsets", () => {
  const editorText = "let x = foo";
  const [cm] = toCmDiagnostics([diag({ from: 8, to: 11 })], { editorText, surface: "ml" });
  assert.equal(cm.from, 8);
  assert.equal(cm.to, 11);
  assert.equal(cm.severity, "error");
  assert.equal(cm.source, "cadenza");
  assert.equal(cm.message, "CDZ0101: unbound name");
});

test("a warning maps to severity 'warning'; an uncoded diag drops the code prefix", () => {
  const editorText = "abcdef";
  const [cm] = toCmDiagnostics([diag({ error: false, code: "", message: "shadowed", from: 0, to: 3 })], {
    editorText,
    surface: "ml",
  });
  assert.equal(cm.severity, "warning");
  assert.equal(cm.message, "shadowed"); // no "code: " prefix when code === ""
});

test("wrapPrefixBytes is subtracted: a span in the content maps back to the editor offset", () => {
  // Editor content "foo" was compiled as "let _ = \nfoo" — 9 wrapper bytes before it.
  const editorText = "foo";
  const [cm] = toCmDiagnostics([diag({ from: 9, to: 12 })], {
    editorText,
    surface: "ml",
    wrapPrefixBytes: 9,
  });
  assert.equal(cm.from, 0);
  assert.equal(cm.to, 3);
});

test("a diagnostic wholly inside the wrapper (to < prefix) is DROPPED", () => {
  const editorText = "foo";
  const out = toCmDiagnostics([diag({ from: 2, to: 5 })], {
    editorText,
    surface: "ml",
    wrapPrefixBytes: 9, // to(5) - prefix(9) = -4 < 0 → dropped
  });
  assert.equal(out.length, 0);
});

test("a span running past the editor content is CLAMPED to the content end", () => {
  const editorText = "foo"; // 3 bytes
  const [cm] = toCmDiagnostics([diag({ from: 1, to: 99 })], {
    editorText,
    surface: "ml",
    editorBytes: 3,
  });
  assert.equal(cm.from, 1);
  assert.equal(cm.to, 3);
});

test("a zero-width span is widened to a non-empty mark (to > from)", () => {
  const editorText = "foo";
  const [cm] = toCmDiagnostics([diag({ from: 1, to: 1 })], { editorText, surface: "ml" });
  assert.ok(cm.to > cm.from, "mark must be non-empty");
  assert.equal(cm.from, 1);
  assert.equal(cm.to, 2);
});

test("a zero-width span at end-of-text clamps the widened mark to the doc length", () => {
  const editorText = "ab"; // len 2
  const [cm] = toCmDiagnostics([diag({ from: 2, to: 2 })], { editorText, surface: "ml" });
  assert.equal(cm.from, 2);
  assert.equal(cm.to, 2); // can't widen past doc end → stays at from
});

test("the unanchored 0,0 diag attaches to document start even with a wrapper prefix", () => {
  const editorText = "foo";
  const [cm] = toCmDiagnostics([diag({ from: 0, to: 0 })], {
    editorText,
    surface: "ml",
    wrapPrefixBytes: 9, // the 0,0 special-case ignores the prefix rather than dropping the diag
  });
  assert.equal(cm.from, 0);
  assert.ok(cm.to >= cm.from);
});

test("UTF-8 byte spans past an astral char map to correct UTF-16 offsets", () => {
  // "😀" is 4 UTF-8 bytes but 2 UTF-16 code units. A span after it must account for the surrogate pair.
  const editorText = "😀ab"; // bytes: 4 + 1 + 1 ; utf-16: 2 + 1 + 1
  const [cm] = toCmDiagnostics([diag({ from: 4, to: 5 })], { editorText, surface: "ml" });
  assert.equal(cm.from, 2); // byte 4 → after the emoji → UTF-16 index 2
  assert.equal(cm.to, 3);
});

test("a fix on the sexpr surface with a real range becomes a quick-fix action", () => {
  const fix: DiagFix = { kind: "replace", replacement: "at", from: 20, to: 22, verified: true };
  const editorText = "(get m k)";
  const [cm] = toCmDiagnostics([diag({ from: 1, to: 4, fix })], { editorText, surface: "sexpr" });
  assert.ok(cm.actions && cm.actions.length === 1);
  assert.match(cm.actions![0].name, /replace with `at`.*Verified/);
});

test("the SAME fix on the ml surface is NOT offered (fixIsApplicable gates on sexpr)", () => {
  const fix: DiagFix = { kind: "replace", replacement: "at", from: 20, to: 22, verified: true };
  const editorText = "get m k";
  const [cm] = toCmDiagnostics([diag({ from: 0, to: 3, fix })], { editorText, surface: "ml" as Surface });
  assert.equal(cm.actions, undefined);
});

test("a wrap-kind fix labels with its replacement template", () => {
  const fix: DiagFix = { kind: "wrap", replacement: "(Some …)", from: 5, to: 10, verified: false };
  const editorText = "(f x y z)";
  const [cm] = toCmDiagnostics([diag({ from: 1, to: 2, fix })], { editorText, surface: "sexpr" });
  assert.match(cm.actions![0].name, /wrap in `\(Some …\)`.*Suggested/);
});

test("multiple diagnostics preserve order; a dropped one doesn't shift the rest", () => {
  const editorText = "foobar";
  const out = toCmDiagnostics(
    [
      diag({ from: 9, to: 11, message: "inside wrapper" }), // dropped (to-prefix < 0)
      diag({ from: 12, to: 15, message: "first real" }),
      diag({ from: 15, to: 18, message: "second real" }),
    ],
    { editorText, surface: "ml", wrapPrefixBytes: 12 },
  );
  assert.equal(out.length, 2);
  assert.equal(out[0].message, "CDZ0101: first real");
  assert.equal(out[1].message, "CDZ0101: second real");
});
