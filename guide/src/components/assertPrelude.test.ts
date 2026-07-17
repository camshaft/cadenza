/// Unit tests for the shared `<Runnable mode="test">` assert prelude (`assertPrelude.ts`) — the three
/// `assert`/`assert-eq`/`assert-ne` helpers prepended to a testing example so authors write just their
/// `@test` defs. This is load-bearing: a bug here breaks EVERY `mode="test"` example. The historical bug
/// this pins: the ML prelude once spelled the names with UNDERSCORES (`assert_eq`), but a Cadenza name is
/// KEBAB across both surfaces (an s-expr `(def (assert-eq …))` renders to ML as `def assert-eq(…)`), so an
/// underscore spelling left every ML @test example failing `CDZ0101 unbound name assert-eq`. These tests
/// keep the names kebab and the two surfaces in lockstep.

import test from "node:test";
import assert from "node:assert/strict";
import { assertPreludeFor, ASSERT_PRELUDE_ML, ASSERT_PRELUDE_SEXPR } from "./assertPrelude.ts";

const HELPERS = ["assert", "assert-eq", "assert-ne"] as const;

test("assertPreludeFor selects the surface's prelude", () => {
  assert.equal(assertPreludeFor("ml"), ASSERT_PRELUDE_ML);
  assert.equal(assertPreludeFor("sexpr"), ASSERT_PRELUDE_SEXPR);
});

test("both preludes define all three helpers, with KEBAB names (never underscores)", () => {
  for (const [label, src] of [
    ["ml", ASSERT_PRELUDE_ML],
    ["sexpr", ASSERT_PRELUDE_SEXPR],
  ] as const) {
    // The regression that broke every ML @test example: `assert_eq`/`assert_ne` with an underscore.
    assert.ok(!/assert_eq|assert_ne/.test(src), `${label} prelude must NOT use underscore names`);
    for (const name of HELPERS) {
      assert.ok(src.includes(name), `${label} prelude defines ${name}`);
    }
  }
});

test("ML prelude uses ML def syntax; s-expr prelude uses s-expr def syntax", () => {
  // ML: `def assert-eq(a, b, msg: String) = …`
  assert.match(ASSERT_PRELUDE_ML, /def assert-eq\([^)]*\) =/);
  assert.match(ASSERT_PRELUDE_ML, /then unit else trap\(msg\)/);
  // s-expr: `(def (assert-eq …) (if … unit (trap msg)))`
  assert.match(ASSERT_PRELUDE_SEXPR, /\(def \(assert-eq /);
  assert.match(ASSERT_PRELUDE_SEXPR, /\(trap msg\)/);
});

test("each prelude ends with a trailing newline (separates it from the prepended example body)", () => {
  assert.ok(ASSERT_PRELUDE_ML.endsWith("\n"), "ML prelude ends with a newline");
  assert.ok(ASSERT_PRELUDE_SEXPR.endsWith("\n"), "s-expr prelude ends with a newline");
});

test("the two surfaces stay in lockstep — same set of helper names defined in each", () => {
  // Every helper name appears in BOTH preludes; neither surface gains or drops a helper silently.
  for (const name of HELPERS) {
    assert.ok(ASSERT_PRELUDE_ML.includes(name), `ML has ${name}`);
    assert.ok(ASSERT_PRELUDE_SEXPR.includes(name), `s-expr has ${name}`);
  }
});
