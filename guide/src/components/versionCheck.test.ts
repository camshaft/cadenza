/// Unit tests for the pure deploy-detection logic.

import { test } from "node:test";
import assert from "node:assert/strict";
import { isNewerVersion, parseVersion } from "./versionCheck.ts";

test("isNewerVersion is true only for a non-empty polled id that differs from the running one", () => {
  assert.equal(isNewerVersion("100", "200"), true); // a newer deploy
  assert.equal(isNewerVersion("100", "100"), false); // same build
  assert.equal(isNewerVersion("100", ""), false); // blank → not an update
  assert.equal(isNewerVersion("100", null), false); // fetch failed
  assert.equal(isNewerVersion("100", undefined), false);
});

test("isNewerVersion does not nag when the running id is unknown but polled matches emptiness", () => {
  // A running id of "" (define missing in some odd build) still only flips on a real, different value.
  assert.equal(isNewerVersion("", ""), false);
  assert.equal(isNewerVersion("", "300"), true);
});

test("parseVersion extracts a string version, else null", () => {
  assert.equal(parseVersion({ version: "abc" }), "abc");
  assert.equal(parseVersion({ version: 123 }), null); // non-string
  assert.equal(parseVersion({}), null); // missing
  assert.equal(parseVersion(null), null);
  assert.equal(parseVersion("not an object"), null);
  assert.equal(parseVersion(undefined), null);
});
