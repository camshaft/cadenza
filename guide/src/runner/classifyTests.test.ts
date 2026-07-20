/// Unit tests for the in-browser @test classification helpers (`classifyTests.ts`) — the SCALAR/COMPOUND/
/// DEFERRED partition and the whole-suite name union. These were inline in client.ts (which imports Worker,
/// so `node --test` couldn't reach them); extracting them pins two silent-drop failure modes: a property
/// test landing in the wrong driver / vanishing, and a timed-out property vanishing from the report. Run
/// with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { classifyParamTests, allTestNames, type ParamTestSig } from "./classifyTests.ts";

test("classifyParamTests splits scalar vs compound and carries paramTypes", () => {
  const sigs: ParamTestSig[] = [
    { name: "prop_add", compound: false, paramTypes: ["Int64", "Int64"] },
    { name: "prop_list_gen", compound: true, paramTypes: [] },
  ];
  const c = classifyParamTests(sigs, ["prop_add", "prop_list_gen"]);
  assert.deepEqual(c.scalarProps, [{ name: "prop_add", paramTypes: ["Int64", "Int64"] }]);
  assert.deepEqual(c.compoundProps, [{ name: "prop_list_gen" }]);
  assert.deepEqual(c.deferredNames, [], "both are classified, nothing deferred");
});

test("a paramTestName with NO signature is DEFERRED (not dropped, not failed)", () => {
  // The compiler couldn't synthesize a generator for this shape → no sig row, but it IS a param test.
  const sigs: ParamTestSig[] = [{ name: "prop_ok", compound: false, paramTypes: ["Int64"] }];
  const c = classifyParamTests(sigs, ["prop_ok", "prop_weird_shape"]);
  assert.deepEqual(c.scalarProps, [{ name: "prop_ok", paramTypes: ["Int64"] }]);
  assert.deepEqual(c.compoundProps, []);
  assert.deepEqual(c.deferredNames, ["prop_weird_shape"], "the un-signatured param test defers, never vanishes");
});

test("classifyParamTests handles empty inputs (no param tests at all)", () => {
  const c = classifyParamTests([], []);
  assert.deepEqual(c.scalarProps, []);
  assert.deepEqual(c.compoundProps, []);
  assert.deepEqual(c.deferredNames, []);
});

test("a scalar-classified name is never also deferred (driven set covers scalar)", () => {
  const sigs: ParamTestSig[] = [{ name: "prop_x", compound: false, paramTypes: ["Bool"] }];
  const c = classifyParamTests(sigs, ["prop_x"]);
  assert.equal(c.deferredNames.length, 0, "a driven scalar must not also appear as deferred");
});

test("a compound-classified name is never also deferred (driven set covers compound)", () => {
  const sigs: ParamTestSig[] = [{ name: "prop_g", compound: true, paramTypes: [] }];
  const c = classifyParamTests(sigs, ["prop_g"]);
  assert.equal(c.deferredNames.length, 0, "a driven compound must not also appear as deferred");
});

test("deferred preserves paramTestNames order and only excludes the driven ones", () => {
  const sigs: ParamTestSig[] = [{ name: "b", compound: false, paramTypes: [] }];
  const c = classifyParamTests(sigs, ["a", "b", "c"]);
  assert.deepEqual(c.deferredNames, ["a", "c"], "order preserved, only b (driven) removed");
});

test("allTestNames unions nullary + scalar + compound (a timed-out property can't vanish)", () => {
  const names = allTestNames(
    ["t_nullary1", "t_nullary2"],
    [{ name: "prop_scalar" }],
    [{ name: "prop_compound" }],
  );
  assert.deepEqual(names, ["t_nullary1", "t_nullary2", "prop_scalar", "prop_compound"]);
});

test("allTestNames with only nullary tests (no properties)", () => {
  assert.deepEqual(allTestNames(["only"], [], []), ["only"]);
});

test("allTestNames with only properties (no nullary tests)", () => {
  assert.deepEqual(allTestNames([], [{ name: "ps" }], [{ name: "pc" }]), ["ps", "pc"]);
});
