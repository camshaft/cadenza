/// Unit tests for the clickable "change X to Y" one-token patch (`tryChangePatch.ts`) — the exactly-once
/// rule that BOTH the runtime (`useCadenzaEditor.applyPatch`) and the build gate (`tryChange.test.ts`)
/// share. The rule (v-guide-editor): a `find=` patch must match exactly once; 0 = not found, >1 =
/// ambiguous, both rejected (a silent mis-patch is worse than a full `variant=`). Run with `test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { countOccurrences, patchOnce } from "./tryChangePatch.ts";

test("countOccurrences: literal, non-overlapping", () => {
  assert.equal(countOccurrences("(+ 2 3)", "2"), 1);
  assert.equal(countOccurrences("(if (< 1 2) 1 2)", "2"), 2);
  assert.equal(countOccurrences("(+ 2 3)", "9"), 0);
  assert.equal(countOccurrences("aaaa", "aa"), 2); // non-overlapping
  assert.equal(countOccurrences("(+ 2 3)", ""), 0); // empty never matches
});

test("countOccurrences: treats find as a LITERAL (regex metachars don't matter)", () => {
  assert.equal(countOccurrences("(< 3 5)", "<"), 1);
  assert.equal(countOccurrences("a/b + c/d", "/"), 2);
  assert.equal(countOccurrences(`(f "sub" x)`, `"sub"`), 1);
});

test("patchOnce: replaces the single occurrence", () => {
  const r = patchOnce("(if (< 3 5) 100 200)", "<", ">");
  assert.deepEqual(r, { ok: true, text: "(if (> 3 5) 100 200)" });
});

test("patchOnce: replace can differ in length (fraction swap)", () => {
  const r = patchOnce("(lower (Cube (v3r 4/1 4/1 4/1)))".replace(/4\/1 4\/1 4\/1/, "4/1 X 4/1"), "X", "6/1");
  assert.ok(r.ok && r.text.includes("6/1"));
});

test("patchOnce: 0 matches → not-found, no patch", () => {
  const r = patchOnce("(+ 2 3)", "9", "0");
  assert.deepEqual(r, { ok: false, reason: "not-found", count: 0 });
});

test("patchOnce: >1 matches → ambiguous, no patch (fail loud)", () => {
  const r = patchOnce("(if (< 1 2) 1 2)", "2", "8");
  assert.deepEqual(r, { ok: false, reason: "ambiguous", count: 2 });
});

test("patchOnce: empty find → empty-find", () => {
  const r = patchOnce("(+ 2 3)", "", "x");
  assert.deepEqual(r, { ok: false, reason: "empty-find", count: 0 });
});

test("patchOnce: replace only the FIRST/only match, not a global replace (single-match guarantee)", () => {
  // With exactly one match, String.replace(string) replaces just that one — verify no accidental global.
  const r = patchOnce("abcXdef", "X", "YY");
  assert.deepEqual(r, { ok: true, text: "abcYYdef" });
});
