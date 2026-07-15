/// Unit tests for the pure chunk-load-error detection + one-shot auto-reload guard.

import { test } from "node:test";
import assert from "node:assert/strict";
import { isChunkLoadError, shouldAutoReload, clearAutoReloadGuard, type KVStore } from "./chunkError.ts";

test("isChunkLoadError matches the known per-browser dynamic-import failures", () => {
  assert.equal(isChunkLoadError(new Error("Failed to fetch dynamically imported module: https://x/Rationals-abc.js")), true);
  assert.equal(isChunkLoadError(new Error("error loading dynamically imported module")), true);
  assert.equal(isChunkLoadError(new Error("Importing a module script failed.")), true);
  // a plain string or an error-like object, not just Error instances
  assert.equal(isChunkLoadError("Failed to fetch dynamically imported module: /x.js"), true);
  assert.equal(isChunkLoadError({ message: "Importing a module script failed." }), true);
});

test("isChunkLoadError does NOT match an ordinary render error", () => {
  assert.equal(isChunkLoadError(new Error("Cannot read properties of undefined")), false);
  assert.equal(isChunkLoadError(new Error("boom")), false);
  assert.equal(isChunkLoadError(null), false);
  assert.equal(isChunkLoadError(undefined), false);
});

function fakeStore(): KVStore & { map: Map<string, string> } {
  const map = new Map<string, string>();
  return {
    map,
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
    removeItem: (k) => void map.delete(k),
  };
}

test("shouldAutoReload fires exactly once per session, then blocks the loop", () => {
  const store = fakeStore();
  assert.equal(shouldAutoReload(store), true); // first failure → reload
  assert.equal(shouldAutoReload(store), false); // repeat within session → don't loop
  assert.equal(shouldAutoReload(store), false);
});

test("clearAutoReloadGuard re-arms auto-reload after a successful load", () => {
  const store = fakeStore();
  assert.equal(shouldAutoReload(store), true);
  assert.equal(shouldAutoReload(store), false);
  clearAutoReloadGuard(store); // a route loaded fine → re-arm
  assert.equal(shouldAutoReload(store), true);
});

test("shouldAutoReload is a safe no-op when there is no storage", () => {
  assert.equal(shouldAutoReload(null), false);
  assert.equal(shouldAutoReload(undefined), false);
  // a throwing store (privacy mode) → false, never throws
  const throwing: KVStore = {
    getItem: () => { throw new Error("blocked"); },
    setItem: () => { throw new Error("blocked"); },
    removeItem: () => { throw new Error("blocked"); },
  };
  assert.equal(shouldAutoReload(throwing), false);
});
