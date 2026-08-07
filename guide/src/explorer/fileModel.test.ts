/// Pins the explorer's pure multi-file lowering (E0). The lowered arrays feed compileWithPreloaded, whose
/// names/sources/formats MUST be equal length (preloadArity) and whose entry is the genesis `text` — so a
/// bug here would mis-wire the whole explorer compile. Covers: happy path, the three reject reasons
/// (no-entry / multi-entry / dup-name / empty / empty-name), order stability, and arity-by-construction.

import { test } from "node:test";
import assert from "node:assert/strict";
import { lowerToCompile, type ExplorerFile } from "./fileModel.ts";

const f = (name: string, source: string, surface: "ml" | "sexpr" = "sexpr", entry = false): ExplorerFile => ({
  name,
  source,
  surface,
  entry,
});

test("lowers a genesis + preloaded modules to the compileWithPreloaded shape", () => {
  const files = [
    f("main", "(do (def (main) (greet)) (export main))", "sexpr", true),
    f("greeting", "(def (greet) 42)", "sexpr"),
    f("helpers", "(def (aux) 1)", "ml"),
  ];
  const r = lowerToCompile(files);
  assert.ok(r.ok, "should lower");
  if (!r.ok) return;
  assert.equal(r.lowered.text, "(do (def (main) (greet)) (export main))");
  assert.equal(r.lowered.from, "sexpr");
  // preloaded modules in model order, entry excluded, all three arrays aligned:
  assert.deepEqual(r.lowered.names, ["greeting", "helpers"]);
  assert.deepEqual(r.lowered.sources, ["(def (greet) 42)", "(def (aux) 1)"]);
  assert.deepEqual(r.lowered.formats, ["sexpr", "ml"]);
});

test("the three preload arrays are always equal length (satisfies preloadArity by construction)", () => {
  const files = [f("a", "…", "sexpr", true), f("b", "…"), f("c", "…"), f("d", "…")];
  const r = lowerToCompile(files);
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(r.lowered.names.length, r.lowered.sources.length);
  assert.equal(r.lowered.names.length, r.lowered.formats.length);
  assert.equal(r.lowered.names.length, 3); // 4 files − 1 entry
});

test("a single entry file lowers with empty preload arrays (still valid)", () => {
  const r = lowerToCompile([f("only", "(do (def (main) 1) (export main))", "sexpr", true)]);
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.deepEqual(r.lowered.names, []);
  assert.deepEqual(r.lowered.sources, []);
  assert.deepEqual(r.lowered.formats, []);
  assert.equal(r.lowered.text, "(do (def (main) 1) (export main))");
});

test("rejects an empty file set", () => {
  const r = lowerToCompile([]);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /empty file set/);
});

test("rejects when no file is the entry", () => {
  const r = lowerToCompile([f("a", "…"), f("b", "…")]);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /no entry file/);
});

test("rejects when more than one file is the entry", () => {
  const r = lowerToCompile([f("a", "…", "sexpr", true), f("b", "…", "sexpr", true)]);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /multiple entry files/);
  assert.match(r.reason, /a, b/);
});

test("rejects duplicate file names (imports resolve by name; a dup would shadow)", () => {
  const r = lowerToCompile([f("main", "…", "sexpr", true), f("dup", "…"), f("dup", "…")]);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /duplicate file name/);
  assert.match(r.reason, /dup/);
});

test("rejects an empty file name", () => {
  const r = lowerToCompile([f("main", "…", "sexpr", true), f("", "…")]);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /non-empty `name`/);
});

test("preload order follows model order, not name sort (deep-links/tabs rely on stable order)", () => {
  const files = [f("z-entry", "…", "sexpr", true), f("m", "…"), f("a", "…"), f("q", "…")];
  const r = lowerToCompile(files);
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.deepEqual(r.lowered.names, ["m", "a", "q"]); // model order preserved, NOT ["a","m","q"]
});
