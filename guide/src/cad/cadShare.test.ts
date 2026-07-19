/// Unit tests for /cad share-link encode/decode (`cadShare.ts`) — the `#cad/` payload round-trip. Pins:
/// the source+surface round-trip, the optional exact-fraction params (a shared parametric model restores
/// its dragged dims), the KIND guard (a playground `#code/` hash isn't decodable here), and malformed
/// rejection. (`encodeCadShareUrl` reads `location`, absent under node, so it's not covered — same as the
/// playground/shareLink tests.) Run with `npm run test:unit`.

import test from "node:test";
import assert from "node:assert/strict";
import { decodeCadShare } from "./cadShare.ts";
import { encodeShareHash } from "../share/shareLink.ts";

// Encode a `#cad/` hash the same way the module's encodeCadShareUrl does (via the generic shareLink), so we
// can round-trip through decodeCadShare without needing `location`.
const cadHash = (payload: unknown) => encodeShareHash("cad", payload);

test("round-trips a plain (non-parametric) model: source + surface", () => {
  for (const shared of [
    { s: "ml" as const, src: "def main() = lower(Solid.Sphere(2))" },
    { s: "sexpr" as const, src: "(def (main) (lower ((. Solid Sphere) 2)))" },
  ]) {
    assert.deepEqual(decodeCadShare(cadHash(shared)), shared);
    assert.deepEqual(decodeCadShare("#" + cadHash(shared)), shared, "leading # tolerated");
  }
});

test("round-trips a PARAMETRIC model with exact-fraction params (shared 7/2 comes back 7/2)", () => {
  const shared = {
    s: "ml" as const,
    src: "@!param(...) thickness : Rational\ndef main() = ...",
    params: { thickness: { num: 7, den: 2 }, width: { num: 50, den: 1 } },
  };
  const back = decodeCadShare(cadHash(shared));
  assert.deepEqual(back, shared);
  assert.equal(back?.params?.thickness.num, 7);
  assert.equal(back?.params?.thickness.den, 2);
});

test("KIND guard: a playground #code/ hash is NOT decodable as /cad", () => {
  const codeHash = encodeShareHash("code", { s: "ml", src: "2 + 3" });
  assert.equal(decodeCadShare(codeHash), null);
});

test("rejects malformed payloads", () => {
  assert.equal(decodeCadShare(""), null);
  assert.equal(decodeCadShare("#/cad"), null); // route-like, not a payload
  assert.equal(decodeCadShare("cad/@@@bad-lz@@@"), null); // corrupt payload
  assert.equal(decodeCadShare(cadHash({ s: "python", src: "x" })), null); // bad surface
  assert.equal(decodeCadShare(cadHash({ s: "ml" })), null); // missing src
  assert.equal(decodeCadShare(cadHash({ s: "ml", src: "x", params: { t: { num: "no" } } })), null); // bad param
});
