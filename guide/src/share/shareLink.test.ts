/// Unit tests for the generic share-link module (`shareLink.ts`) — the kind-namespaced LZ-compressed
/// payload-in-URL-hash used by the playground (`code`), /cad (`cad`), and notebook (`nb`). Pins the
/// round-trip, the KIND namespacing (a decoder rejects another kind's hash), and total malformed-input
/// handling. (`encodeShareUrl` reads `location`, absent under node, so it's not covered here — same as the
/// playground's share.test.) Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { encodeShareHash, decodeShareHash } from "./shareLink.ts";

interface Demo {
  a: number;
  b: string;
}
const isDemo = (v: unknown): v is Demo => {
  const o = v as Demo;
  return !!o && typeof o.a === "number" && typeof o.b === "string";
};

test("round-trips a typed payload through the hash under its kind", () => {
  for (const payload of [
    { a: 1, b: "x" },
    { a: 0, b: "" },
    { a: -3, b: "café 你 😀 multi-byte" },
  ]) {
    const hash = encodeShareHash("cad", payload);
    assert.match(hash, /^cad\//, "hash carries the kind prefix");
    assert.deepEqual(decodeShareHash(hash, "cad", isDemo), payload);
    assert.deepEqual(decodeShareHash("#" + hash, "cad", isDemo), payload, "leading # tolerated");
  }
});

test("KIND namespacing: a decoder rejects another kind's hash", () => {
  const hash = encodeShareHash("cad", { a: 1, b: "x" });
  assert.equal(decodeShareHash(hash, "nb", isDemo), null, "cad hash is not decodable as nb");
  assert.equal(decodeShareHash(hash, "code", isDemo), null, "cad hash is not decodable as code");
  assert.deepEqual(decodeShareHash(hash, "cad", isDemo), { a: 1, b: "x" }, "same kind decodes");
});

test("decodeShareHash returns null for anything malformed", () => {
  assert.equal(decodeShareHash("", "cad", isDemo), null);
  assert.equal(decodeShareHash("#/basics", "cad", isDemo), null); // ordinary route hash
  assert.equal(decodeShareHash("cad/", "cad", isDemo), null); // no payload
  assert.equal(decodeShareHash("cad/@@@not-valid-lz@@@", "cad", isDemo), null); // corrupt payload
});

test("decodeShareHash rejects a well-formed payload that fails validate (wrong shape)", () => {
  // Encode a Demo, then decode demanding a DIFFERENT shape guard → rejected.
  const hash = encodeShareHash("cad", { a: 1, b: "x" });
  const wantsStringA = (v: unknown): v is { a: string } => typeof (v as { a: string }).a === "string";
  assert.equal(decodeShareHash(hash, "cad", wantsStringA), null);
});

test("kinds are independent: code / cad / nb round-trip under their own prefix", () => {
  for (const kind of ["code", "cad", "nb"]) {
    const hash = encodeShareHash(kind, { a: 7, b: kind });
    assert.match(hash, new RegExp(`^${kind}/`));
    assert.deepEqual(decodeShareHash(hash, kind, isDemo), { a: 7, b: kind });
  }
});
