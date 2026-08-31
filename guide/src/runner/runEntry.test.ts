import { test } from "node:test";
import assert from "node:assert/strict";
import { selectRunEntry, parameterizedEntryMessage, exportedFunctions } from "./runEntry.ts";

/// These pin the run-entry classification that fixed the operator-reported playground bug: a program whose
/// entry point takes an argument (`def main(a: Int64) = (a, a)`) compiles to an arity-N `make(a)` maker (or
/// `main(a)` for a scalar entry). The pre-fix worker called `make()` with no argument, lowering `undefined`
/// to the missing i64 and throwing "Cannot convert undefined to a BigInt" — surfaced as "any program with an
/// argument fails / result coerced to a BigInt". `selectRunEntry` must classify it `parameterized` instead.

test("nullary compound entry → compound plan (make/encode)", () => {
  const enc = () => new Uint8Array([1]);
  const root = { "cadenza:run/run": { make: () => "handle", encode: enc } };
  const plan = selectRunEntry(root);
  assert.equal(plan.kind, "compound");
  if (plan.kind === "compound") assert.equal(plan.iface.encode, enc);
});

test("ARGFUL compound entry → parameterized (the BigInt-coercion bug: make(a) never invoked with no arg)", () => {
  // `make` declares one parameter (arity 1) — the pre-fix code called `make()` and threw on the missing arg.
  const root = { "cadenza:run/run": { make: (_a: unknown) => "handle", encode: () => new Uint8Array() } };
  const plan = selectRunEntry(root);
  assert.equal(plan.kind, "parameterized");
  if (plan.kind === "parameterized") {
    assert.equal(plan.arity, 1);
    assert.equal(plan.name, null); // the maker doesn't carry the entry's source name
  }
});

test("nullary bare function → scalar plan", () => {
  const fn = () => 42n;
  const plan = selectRunEntry({ main: fn });
  assert.equal(plan.kind, "scalar");
  if (plan.kind === "scalar") assert.equal(plan.fn, fn);
});

test("argful bare function → parameterized, carrying the export name + arity", () => {
  const plan = selectRunEntry({ inc: (_x: unknown) => 0n });
  assert.equal(plan.kind, "parameterized");
  if (plan.kind === "parameterized") {
    assert.equal(plan.name, "inc");
    assert.equal(plan.arity, 1);
  }
});

test("prefers a NULLARY bare function even when an argful one is also exported", () => {
  const good = () => 1n;
  const plan = selectRunEntry({ inc: (_x: unknown) => 0n, main: good });
  assert.equal(plan.kind, "scalar");
  if (plan.kind === "scalar") assert.equal(plan.fn, good);
});

test("no runnable export → none", () => {
  assert.equal(selectRunEntry({}).kind, "none");
  assert.equal(selectRunEntry({ someField: 3 }).kind, "none");
});

test("exportedFunctions returns only functions, with names", () => {
  const fns = exportedFunctions({ a: () => 1, b: 2, c: () => 3 });
  assert.deepEqual(fns.map((f) => f.name), ["a", "c"]);
});

test("parameterizedEntryMessage names the export + suggests a REPL call when the name is known", () => {
  const m = parameterizedEntryMessage("inc", 1);
  assert.match(m, /`inc` takes an argument/);
  assert.match(m, /`inc\(…\)`/); // REPL call hint
  assert.match(m, /def main\(\) = inc\(…\)/); // wrap-in-main hint
});

test("parameterizedEntryMessage falls back to a generic entry when the name is unknown (compound maker)", () => {
  const m = parameterizedEntryMessage(null, 2);
  assert.match(m, /entry point takes 2 arguments/);
  assert.match(m, /main\(…, …\)/); // arity-2 REPL hint
  assert.match(m, /give the entry point no parameters/);
});
