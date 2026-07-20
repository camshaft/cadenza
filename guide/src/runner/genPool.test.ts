/// Unit tests for the pure property-test generator core (`genPool.ts`) — the browser-side twin of the
/// native `cdz test` generator (rcdzc `proptest_gen.rs`). These were inline in runWorker.ts (which imports
/// jco-transpile + uses worker globals), so `node --test` couldn't reach them; extracting them pins the
/// pool→value mapping so a drift from the native side (or a regression here) fails a test. Run with
/// `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { lcgStep, intRange, genArg, genArgs, renderArgs, normalizeName, GenPool } from "./genPool.ts";

test("lcgStep is deterministic and stays in 64 bits", () => {
  const a = lcgStep(0n);
  const b = lcgStep(0n);
  assert.equal(a, b, "same seed → same step (reproducible)");
  assert.ok(a >= 0n && a <= 0xffffffffffffffffn, "masked to 64 bits");
  assert.notEqual(lcgStep(1n), a, "different seed → different step");
});

test("intRange gives the exact signed/unsigned width bounds, null for non-scalar", () => {
  assert.deepEqual(intRange("int8"), { min: -128n, max: 127n });
  assert.deepEqual(intRange("uint8"), { min: 0n, max: 255n });
  assert.deepEqual(intRange("int64"), { min: -9223372036854775808n, max: 9223372036854775807n });
  assert.deepEqual(intRange("uint64"), { min: 0n, max: 18446744073709551615n });
  assert.equal(intRange("float64"), null, "a float is not a range-generated int");
  assert.equal(intRange("bool"), null);
  assert.equal(intRange("other"), null, "a compound/unknown type has no int range");
});

test("genArg produces an in-range bigint for every int width (fold is always in-bounds)", () => {
  for (const t of ["int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64"]) {
    const r = intRange(t)!;
    // Sweep several states, incl. ones whose raw LCG draw far exceeds the width — the modulo fold must
    // still land in range (a bug here would generate an out-of-range arg that jco rejects at the boundary).
    for (const seed of [0n, 1n, 42n, 999983n, 0xdeadbeefn]) {
      const { arg } = genArg(t, seed);
      assert.equal(typeof arg, "bigint", `${t} lowers to bigint`);
      assert.ok((arg as bigint) >= r.min && (arg as bigint) <= r.max, `${t}@${seed}: ${arg} in [${r.min},${r.max}]`);
    }
  }
});

test("genArg: bool → boolean, float → integer-valued number (never NaN)", () => {
  const b = genArg("bool", 7n);
  assert.equal(typeof b.arg, "boolean");
  for (const t of ["float32", "float64"]) {
    for (const seed of [0n, 3n, 12345n]) {
      const { arg } = genArg(t, seed);
      assert.equal(typeof arg, "number");
      assert.ok(Number.isInteger(arg as number), `${t} float arg is integer-valued`);
      assert.ok(!Number.isNaN(arg as number), "never NaN");
      assert.ok((arg as number) >= -1024 && (arg as number) < 1024, "in the modest float range");
    }
  }
});

test("genArg is deterministic (same type+state → same arg)", () => {
  assert.deepEqual(genArg("int32", 55n), genArg("int32", 55n));
});

test("genArgs threads state across params (distinct args, deterministic)", () => {
  const a = genArgs(["int64", "int64", "int64"], 1n);
  const b = genArgs(["int64", "int64", "int64"], 1n);
  assert.deepEqual(a, b, "reproducible for a fixed seed");
  assert.equal(a.length, 3);
  // Threading the LCG means the three draws are not all identical (a non-threaded bug would repeat the arg).
  assert.ok(new Set(a.map(String)).size > 1, "threaded state yields varied args");
  assert.deepEqual(genArgs([], 1n), [], "no params → no args");
});

test("renderArgs prints bigints without the JS n suffix (Cadenza-literal style)", () => {
  assert.equal(renderArgs("f", [5n, -3n]), "f(5, -3)");
  assert.equal(renderArgs("g", [true, 2.5]), "g(true, 2.5)");
  assert.equal(renderArgs("h", []), "h()");
});

test("normalizeName collapses kebab/camel/snake to one key", () => {
  const k = normalizeName("one-plus-one");
  assert.equal(normalizeName("one_plus_one"), k);
  assert.equal(normalizeName("onePlusOne"), k);
  assert.equal(normalizeName("ONE-PLUS-ONE"), k);
  assert.notEqual(normalizeName("oneplustwo"), k);
});

test("GenPool GENERATIVE mode extends deterministically from the seed and records draws", () => {
  const p1 = new GenPool(42n);
  const p2 = new GenPool(42n);
  const seq1 = [p1.next(), p1.next(), p1.next()];
  const seq2 = [p2.next(), p2.next(), p2.next()];
  assert.deepEqual(seq1, seq2, "same seed → same draw sequence (replayable)");
  assert.deepEqual(p1.values, seq1, "values records exactly what was consumed");
  assert.ok(seq1.every((v) => v >= 0n && v <= 0xffffffffffffffffn), "draws are 64-bit");
});

test("GenPool REPLAY mode serves the preset then pads 0n (faithful truncation-shrink)", () => {
  const p = new GenPool(0n, [10n, 20n]);
  assert.equal(p.next(), 10n);
  assert.equal(p.next(), 20n);
  // Exhausted a preset pool → pad with 0n, do NOT LCG-extend (this is what makes a shorter pool a faithful
  // "fewer/zero draws" shrink rather than a differently-seeded tail).
  assert.equal(p.next(), 0n);
  assert.equal(p.next(), 0n);
  assert.deepEqual(p.values, [10n, 20n], "a replay pool never grows past its preset");
});

test("GenPool replay is independent of seed (preset overrides generation)", () => {
  const a = new GenPool(0n, [7n]);
  const b = new GenPool(999n, [7n]);
  assert.equal(a.next(), 7n);
  assert.equal(b.next(), 7n, "preset draws ignore the seed");
});
