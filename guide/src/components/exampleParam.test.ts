/// Unit tests for the `?example=<slug>` deep-link param (`exampleParam.ts`) — the read/resolve logic behind
/// per-example nav deep-links. `writeExampleParam` touches `window.history`/`location` (a DOM global absent
/// under node), so it's exercised headlessly, not here; the pure read/resolve is pinned below. Run with
/// `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { readExampleParam, resolveExampleParam } from "./exampleParam.ts";

test("readExampleParam: extracts the example slug from a search string", () => {
  assert.equal(readExampleParam("?example=hollow-tube"), "hollow-tube");
  assert.equal(readExampleParam("?syntax=ml&example=units-bracket"), "units-bracket");
  assert.equal(readExampleParam("?example=hollow-tube&syntax=sexpr"), "hollow-tube");
});

test("readExampleParam: null when absent or empty", () => {
  assert.equal(readExampleParam(""), null);
  assert.equal(readExampleParam("?syntax=ml"), null);
  assert.equal(readExampleParam("?example="), null); // present but empty → null
});

test("resolveExampleParam: a KNOWN slug in the param wins", () => {
  const known = ["cube-with-dent", "hollow-tube", "units-bracket"];
  assert.equal(resolveExampleParam(known, "cube-with-dent", "?example=hollow-tube"), "hollow-tube");
});

test("resolveExampleParam: an UNKNOWN/typo'd slug falls back to the default (no blank surface)", () => {
  const known = ["cube-with-dent", "hollow-tube"];
  assert.equal(resolveExampleParam(known, "cube-with-dent", "?example=does-not-exist"), "cube-with-dent");
  assert.equal(resolveExampleParam(known, "cube-with-dent", ""), "cube-with-dent"); // no param → default
});

test("resolveExampleParam: works across surfaces' slug sets", () => {
  // notebook slugs
  assert.equal(resolveExampleParam(["compound-interest", "loan", "quadratic"], "compound-interest", "?example=quadratic"), "quadratic");
  // playground (slug added by v-guide-editor)
  assert.equal(resolveExampleParam(["hello", "fib", "list-basics"], "hello", "?example=fib"), "fib");
});
