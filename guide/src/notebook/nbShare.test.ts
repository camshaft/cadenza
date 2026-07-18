/// Unit tests for notebook share-link encode/decode (`nbShare.ts`) — the `#nb/` payload round-trip. Pins
/// the doc+surface round-trip, the KIND guard (a playground `#code/` or a `#cad/` hash isn't decodable
/// here), and malformed rejection. (`encodeNbShareUrl` reads `location`, absent under node, so it's not
/// covered — same as the sibling share tests.) Run with `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { decodeNbShare } from "./nbShare.ts";
import { encodeShareHash } from "../share/shareLink.ts";

const nbHash = (payload: unknown) => encodeShareHash("nb", payload);

test("round-trips a notebook document + surface", () => {
  for (const shared of [
    { s: "sexpr" as const, doc: "# Title\n\n```cadenza\n(+ 2 3)\n```\n" },
    { s: "ml" as const, doc: "# H\n\nprose\n\n```cadenza\n2 + 3\n```\n" },
    { s: "sexpr" as const, doc: "" }, // empty doc
  ]) {
    assert.deepEqual(decodeNbShare(nbHash(shared)), shared);
    assert.deepEqual(decodeNbShare("#" + nbHash(shared)), shared, "leading # tolerated");
  }
});

test("preserves multi-byte + markdown structure verbatim", () => {
  const shared = { s: "ml" as const, doc: "# café 你 😀\n\n- a\n- b\n\n```cadenza\nlet x = 1 in x\n```\n" };
  assert.deepEqual(decodeNbShare(nbHash(shared)), shared);
});

test("KIND guard: a playground #code/ or a #cad/ hash is NOT decodable as notebook", () => {
  assert.equal(decodeNbShare(encodeShareHash("code", { s: "ml", src: "2 + 3" })), null);
  assert.equal(decodeNbShare(encodeShareHash("cad", { s: "ml", src: "x" })), null);
});

test("rejects malformed payloads", () => {
  assert.equal(decodeNbShare(""), null);
  assert.equal(decodeNbShare("#/notebook"), null);
  assert.equal(decodeNbShare("nb/@@@bad-lz@@@"), null);
  assert.equal(decodeNbShare(nbHash({ s: "python", doc: "x" })), null); // bad surface
  assert.equal(decodeNbShare(nbHash({ s: "ml" })), null); // missing doc
  assert.equal(decodeNbShare(nbHash({ s: "ml", doc: 42 })), null); // non-string doc
});
