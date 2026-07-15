/// Unit tests for the pure snippet-scaffolding logic. Run with `npm run test:unit` (node's built-in
/// test runner strips the TS types). These are the guide's first fast, dependency-free tests: they
/// need no browser, no wasm, no jco — so they run in every gate and catch a regression in the
/// wrap/strip/export logic instantly. The jco harness (`check:examples`) is the slow e2e complement.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  wrapModule,
  stripModule,
  wrapPrefixOf,
  topLevelDefNames,
  exportNames,
} from "./wrapModule.ts";

test("bare expression → def main() + export (both surfaces)", () => {
  assert.equal(wrapModule("(+ 2 3)", "sexpr"), "(do (def (main) (+ 2 3)) (export main))");
  assert.equal(wrapModule("2 + 3", "ml"), "def main() = 2 + 3\nexport { main }");
});

test("a def BLOCK containing main exports only main", () => {
  const sexpr = "(def (helper x) (+ x 1)) (def (main) (helper 4))";
  assert.equal(wrapModule(sexpr, "sexpr"), `(do ${sexpr} (export main))`);
  const ml = "def helper(x: Int64) = x + 1\ndef main() = helper(4)";
  assert.equal(wrapModule(ml, "ml"), `${ml}\nexport { main }`);
});

// Bug (C): a lone NON-main def was wrapped with `export { main }`, producing the phantom-name
// CDZ0101 "export `main` names no definition". The fix exports the def's OWN name.
test("bug (C): a lone non-main def exports its own name, not main", () => {
  assert.equal(
    wrapModule("def c-to-f(c) = c * 9 / 5 + 32", "ml"),
    "def c-to-f(c) = c * 9 / 5 + 32\nexport { c-to-f }",
  );
  assert.equal(
    wrapModule("(def (c-to-f c) (+ (/ (* c 9) 5) 32))", "sexpr"),
    "(do (def (c-to-f c) (+ (/ (* c 9) 5) 32)) (export c-to-f))",
  );
});

test("bug (C): a multi-def block with no main exports every top-level def name", () => {
  assert.equal(wrapModule("def a = 1\ndef b = 2", "ml"), "def a = 1\ndef b = 2\nexport { a, b }");
  assert.equal(
    wrapModule("(def a 1) (def b 2)", "sexpr"),
    "(do (def a 1) (def b 2) (export a b))",
  );
});

test("already-complete / hand-authored programs are left untouched", () => {
  const withExport = "def f() = 1\nexport { f }";
  assert.equal(wrapModule(withExport, "ml"), withExport);
  const mod = "module M { def f() = 1 }";
  assert.equal(wrapModule(mod, "ml"), mod);
  const doForm = "(do (def (main) 1) (export main))";
  assert.equal(wrapModule(doForm, "sexpr"), doForm);
});

test("topLevelDefNames finds hyphenated names in source order, deduped", () => {
  assert.deepEqual(topLevelDefNames("def c-to-f(c) = 1\ndef helper = 2", "ml"), ["c-to-f", "helper"]);
  assert.deepEqual(topLevelDefNames("(def (f x) 1) (def g 2)", "sexpr"), ["f", "g"]);
});

test("exportNames prefers main when present, else the def names, else falls back to main", () => {
  assert.deepEqual(exportNames("def helper = 1\ndef main() = 2", "ml"), ["main"]);
  assert.deepEqual(exportNames("def a = 1\ndef b = 2", "ml"), ["a", "b"]);
  assert.deepEqual(exportNames("effect Ask { ask : Unit -> Int64 }", "ml"), ["main"]);
});

test("stripModule is the inverse of wrapModule for a bare expression", () => {
  assert.equal(stripModule(wrapModule("(+ 2 3)", "sexpr"), "sexpr"), "(+ 2 3)");
  assert.equal(stripModule(wrapModule("2 + 3", "ml"), "ml"), "2 + 3");
});

test("stripModule keeps a def block minus its trailing export", () => {
  assert.equal(stripModule("def a = 1\ndef b = 2\nexport { a, b }", "ml"), "def a = 1\ndef b = 2");
});

test("wrapPrefixOf is the UTF-8 byte offset of the snippet within the wrapped program", () => {
  const wrapped = wrapModule("(+ 2 3)", "sexpr");
  const prefix = wrapPrefixOf("(+ 2 3)", wrapped);
  // The wrapper is `(do (def (main) ` before the snippet body.
  assert.equal(wrapped.slice(0, prefix), "(do (def (main) ");
  // A snippet that isn't embedded (already-complete) reports 0.
  assert.equal(wrapPrefixOf("done", "unrelated"), 0);
});
