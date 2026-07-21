/// Unit tests for the pure preload-arity guard (`preloadArity.ts`) — the worker-boundary check that turns a
/// names/sources/formats length mismatch into a clear decline diagnostic (instead of the wasm's cryptic raw
/// "must be equal length" throw). This is the class of bug that broke /music when `pattern` was added to
/// MUSIC_PRELOAD_NAMES but not PRELOAD_SOURCES. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { preloadArityError } from "./preloadArity.ts";

test("equal-length arrays → null (no error)", () => {
  assert.equal(preloadArityError(["a", "b"], ["s1", "s2"], ["ml", "ml"]), null);
  assert.equal(preloadArityError([], [], []), null, "all-empty is equal-length (a plain compile) → fine");
  assert.equal(preloadArityError(["x"], ["s"], ["sexpr"]), null);
});

test("names longer than sources → a decline diagnostic naming the counts (the /music break shape)", () => {
  const d = preloadArityError(["a", "b", "c"], ["s1", "s2"], ["ml", "ml", "ml"]);
  assert.ok(d, "a mismatch must produce a diagnostic");
  assert.equal(d!.error, true);
  assert.match(d!.message, /names=3, sources=2, formats=3/);
  assert.match(d!.message, /must be equal length/);
});

test("sources longer than names → diagnostic", () => {
  const d = preloadArityError(["a"], ["s1", "s2"], ["ml"]);
  assert.ok(d);
  assert.match(d!.message, /names=1, sources=2, formats=1/);
});

test("formats out of step → diagnostic (all three must match, not just names==sources)", () => {
  const d = preloadArityError(["a", "b"], ["s1", "s2"], ["ml"]);
  assert.ok(d, "names==sources but formats short must still fault");
  assert.match(d!.message, /names=2, sources=2, formats=1/);
});

test("the diagnostic has the inert-span shape the worker expects (node/from/to 0, no fix)", () => {
  const d = preloadArityError(["a"], [], ["ml"]);
  assert.ok(d);
  assert.equal(d!.code, "");
  assert.equal(d!.node, 0);
  assert.equal(d!.from, 0);
  assert.equal(d!.to, 0);
  assert.equal(d!.fix, null);
});
