/// Unit tests for the calculator's PURE state model — the `let`-chain wrapper and the variables-panel
/// dedup. These must stay byte-identical to the native cdz-calc crate's `wrap_in_lets`, so the exact
/// nesting order (oldest OUTERMOST) and shadowing behavior are worth pinning. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { type Binding, visibleBindings, wrapInLets } from "./letChain.ts";

const b = (name: string, src: string, display = ""): Binding => ({ name, src, display });

test("wrapInLets: no bindings returns the expr unwrapped", () => {
  assert.equal(wrapInLets("ml", [], "1 + 2"), "1 + 2");
  assert.equal(wrapInLets("sexp", [], "(+ 1 2)"), "(+ 1 2)");
});

test("wrapInLets ML: one binding wraps in a single `let … in`", () => {
  assert.equal(wrapInLets("ml", [b("x", "5")], "x + 1"), "let x = 5 in x + 1");
});

test("wrapInLets ML: bindings nest OLDEST OUTERMOST", () => {
  // x bound first → outermost; y bound second → sees x; expr innermost.
  assert.equal(
    wrapInLets("ml", [b("x", "5"), b("y", "x + 1")], "y * 2"),
    "let x = 5 in let y = x + 1 in y * 2",
  );
});

test("wrapInLets ML: a re-binding shadows via a NEW inner let (append-only, `ans = ans + 5`)", () => {
  // The classic re-binding: `ans` appended twice. The inner (newer) `ans` reads the outer (older) one.
  assert.equal(
    wrapInLets("ml", [b("ans", "10"), b("ans", "ans + 5")], "ans"),
    "let ans = 10 in let ans = ans + 5 in ans",
  );
});

test("wrapInLets s-expr: mirrors the ML nesting with `(let ((n v)) …)`", () => {
  assert.equal(wrapInLets("sexp", [b("x", "5")], "(+ x 1)"), "(let ((x 5)) (+ x 1))");
  assert.equal(
    wrapInLets("sexp", [b("x", "5"), b("y", "(+ x 1)")], "(* y 2)"),
    "(let ((x 5)) (let ((y (+ x 1))) (* y 2)))",
  );
});

test("visibleBindings: empty state is empty", () => {
  assert.deepEqual(visibleBindings([]), []);
});

test("visibleBindings: distinct names in insertion order, reading stored display (no re-run)", () => {
  assert.deepEqual(
    visibleBindings([b("x", "5", "5"), b("y", "6", "6")]),
    [{ name: "x", text: "5" }, { name: "y", text: "6" }],
  );
});

test("visibleBindings: a shadowed name shows its NEWEST value, at its NEWEST-occurrence position", () => {
  // x re-bound to 9 after y. The scan is newest-first then reversed, so a re-bound name takes the slot
  // of its LATEST binding — x moves after y (matching the `let`-chain order the newest binding sits in).
  assert.deepEqual(
    visibleBindings([b("x", "5", "5"), b("y", "6", "6"), b("x", "9", "9")]),
    [{ name: "y", text: "6" }, { name: "x", text: "9" }],
  );
});

test("visibleBindings: `ans` re-bound repeatedly collapses to one newest entry", () => {
  assert.deepEqual(
    visibleBindings([b("ans", "1", "1"), b("ans", "2", "2"), b("ans", "3", "3")]),
    [{ name: "ans", text: "3" }],
  );
});
