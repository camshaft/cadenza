/// Unit tests for the pure runtime-hash-guard helpers (run under `npm run test:unit`).

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { hexDigest, explainIfStaleRuntime } from "./runtimeHashGuard.ts";

test("hexDigest formats a Web Crypto digest as lowercase hex matching required_runtime_hash()", async () => {
  const bytes = new Uint8Array([1, 2, 3, 4]);
  // The Node reference for the same bytes (required_runtime_hash returns plain lowercase hex sha-256).
  const expected = createHash("sha256").update(bytes).digest("hex");
  // Feed the SAME bytes through the browser-shaped path (a Web Crypto ArrayBuffer digest).
  const webDigest = await crypto.subtle.digest("SHA-256", bytes.slice().buffer);
  assert.equal(hexDigest(webDigest), expected);
  assert.match(hexDigest(webDigest), /^[0-9a-f]{64}$/);
});

test("explainIfStaleRuntime rewrites ONLY on a known mismatch (false)", () => {
  const trap = "memory access out of bounds";
  const rewritten = explainIfStaleRuntime(trap, false);
  assert.match(rewritten, /stale build/);
  assert.match(rewritten, /Refresh the page/);
  // Keeps the underlying error for debugging.
  assert.match(rewritten, /memory access out of bounds/);
});

test("explainIfStaleRuntime never cries wolf when runtime is verified-good (true) or unknown (null)", () => {
  const trap = "some genuine user-code trap";
  assert.equal(explainIfStaleRuntime(trap, true), trap);
  assert.equal(explainIfStaleRuntime(trap, null), trap);
});
