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
  gatherTestForms,
  ungatherTestForms,
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

test("a leading compiler pragma is a top-level statement, not a bare expr (both surfaces)", () => {
  // Regression: wrapModule used to misclassify a leading `@!`/`(pragma …)` as a bare expression and wrap
  // it `def main() = @!default-fraction Rational` (malformed) → false CDZ0101 squiggles in the editor.
  // A pragma-led snippet is a defs block: the pragma stays top-level, `main` is the entry.
  const ml = "@!default-fraction Rational\ndef main() = 1 / 3 + 1 / 3 + 1 / 3";
  assert.equal(wrapModule(ml, "ml"), `${ml}\nexport { main }`);
  const sexpr = "(pragma default-fraction Rational)\n(def (main) (+ (/ 1 3) (/ 1 3)))";
  assert.equal(wrapModule(sexpr, "sexpr"), `(do ${sexpr} (export main))`);
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

test("stripModule dedents multi-line (do …) children so top-level siblings sit flush-left", () => {
  // operator seq-256 bug: the canonical printer indents `(do` children 2 spaces + blank-line-separates
  // top-level defs; unwrapping must dedent every sibling, not just trim the first line (which left the
  // second def indented 2 spaces — the "weird indentation").
  const printed =
    "(do\n" +
    "  (def (dbl (: x Int64)) (* x 2))\n" +
    "\n" +
    "  (def (main) (dbl 21))\n" +
    "\n" +
    "  (export main))";
  assert.equal(
    stripModule(printed, "sexpr"),
    "(def (dbl (: x Int64)) (* x 2))\n\n(def (main) (dbl 21))",
  );
  // nested body indentation is preserved RELATIVELY (only the common `(do`-child indent is removed)
  const nested =
    "(do\n" +
    "  (def (f x)\n" +
    "    (let ((y 1))\n" +
    "      (+ x y)))\n" +
    "\n" +
    "  (def (main) (f 5))\n" +
    "\n" +
    "  (export main))";
  assert.equal(
    stripModule(nested, "sexpr"),
    "(def (f x)\n  (let ((y 1))\n    (+ x y)))\n\n(def (main) (f 5))",
  );
});

test("stripModule unwraps a multi-line bare (def (main) …) expr with canonical indentation", () => {
  // a bare expression wraps to `(def (main) <expr>)`; when the expr breaks multi-line the printer indents
  // the `def` body 2 spaces, so the unwrapped expr's children must be dedented to their own canonical
  // indent (NOT left over-indented by the leaked def-body 2sp). Mirrors the ML single-main-body case.
  const printed =
    "(do\n" +
    "  (def\n" +
    "    (main)\n" +
    "    (let\n" +
    "      ((first-long-binding 1) (second-long-binding 2) (third-binding 3))\n" +
    "      (+ first-long-binding (+ second-long-binding third-binding))))\n" +
    "\n" +
    "  (export main))";
  assert.equal(
    stripModule(printed, "sexpr"),
    "(let\n" +
      "  ((first-long-binding 1) (second-long-binding 2) (third-binding 3))\n" +
      "  (+ first-long-binding (+ second-long-binding third-binding)))",
  );
  // single-line bare main still unwraps to the bare expression (regression guard)
  assert.equal(stripModule(wrapModule("(+ 2 3)", "sexpr"), "sexpr"), "(+ 2 3)");
});

test("wrapPrefixOf is the UTF-8 byte offset of the snippet within the wrapped program", () => {
  const wrapped = wrapModule("(+ 2 3)", "sexpr");
  const prefix = wrapPrefixOf("(+ 2 3)", wrapped);
  // The wrapper is `(do (def (main) ` before the snippet body.
  assert.equal(wrapped.slice(0, prefix), "(do (def (main) ");
  // A snippet that isn't embedded (already-complete) reports 0.
  assert.equal(wrapPrefixOf("done", "unrelated"), 0);
});

// A `mode="test"` panel renders bare `@test`/`def` forms (wrap=false). The pretty-printer takes ONE
// top-level form, but a test snippet is usually MULTIPLE (several @tests, or a helper + a @test). S-expr
// has no bare multi-form top level, so a multi-form snippet must be gathered under `(do …)` before
// rendering — the miss that fed raw s-expr to the ML parser on the testing page ("expected a name" on
// the first, multi-@test, examples). These pin the gather so app + gate stay in lockstep.
test("gatherTestForms wraps a multi-form s-expr snippet under (do …), leaves ML untouched", () => {
  const sexpr = "(@ test (def (t1) unit))\n(@ test (def (t2) unit))";
  assert.equal(gatherTestForms(sexpr, "sexpr"), `(do ${sexpr})`);
  const ml = "`@`(test, def t1() = unit)\n`@`(test, def t2() = unit)";
  assert.equal(gatherTestForms(ml, "ml"), ml);
});

test("ungatherTestForms peels the (do …) for an s-expr output, trims an ML output", () => {
  // s-expr output: strip the outer `(do …)` back to the bare forms.
  assert.equal(ungatherTestForms("(do (@ test (def (t1) unit)))", "sexpr"), "(@ test (def (t1) unit))");
  // ML output: already native multi-form top level — returned as-is (trimmed).
  const ml = "`@`(test, def t1() = unit)";
  assert.equal(ungatherTestForms(`${ml}\n`, "ml"), ml);
});

test("gatherTestForms is round-tripped by ungatherTestForms in the same s-expr surface", () => {
  const sexpr = "(@ test (def (t1) unit)) (@ test (def (t2) unit))";
  assert.equal(ungatherTestForms(gatherTestForms(sexpr, "sexpr"), "sexpr"), sexpr);
});
