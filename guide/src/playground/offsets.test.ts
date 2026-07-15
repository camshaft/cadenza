/// Unit tests for the UTF-16 (CodeMirror) ↔ UTF-8 (compiler) offset conversions. These sit at every
/// editor/compiler boundary — a wrong count misplaces a diagnostic squiggle or sends the hover cursor
/// to the wrong byte — and are surrogate-sensitive, so they're worth pinning. Ground truth for the
/// byte side is `TextEncoder` (real UTF-8). Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { utf16ToByte, byteToUtf16 } from "./offsets.ts";

const enc = new TextEncoder();
/// The true UTF-8 byte length of the first `utf16Offset` UTF-16 units of `str`.
function trueBytes(str: string, utf16Offset: number): number {
  return enc.encode(str.slice(0, utf16Offset)).length;
}

test("ASCII: UTF-16 and byte offsets coincide", () => {
  const s = "(+ 2 3)";
  for (let i = 0; i <= s.length; i++) {
    assert.equal(utf16ToByte(s, i), i);
    assert.equal(byteToUtf16(s, i), i);
  }
});

test("2-byte char (é): a UTF-16 unit past it counts 2 bytes", () => {
  const s = "café x"; // é is 1 UTF-16 unit, 2 UTF-8 bytes
  assert.equal(utf16ToByte(s, 4), trueBytes(s, 4)); // through "café" → 5 bytes
  assert.equal(utf16ToByte(s, 4), 5);
  // byte 5 is the boundary after é → UTF-16 offset 4
  assert.equal(byteToUtf16(s, 5), 4);
});

test("3-byte char (你): CJK counts 3 bytes per UTF-16 unit", () => {
  const s = "a你b";
  assert.equal(utf16ToByte(s, 1), 1); // "a"
  assert.equal(utf16ToByte(s, 2), 4); // "a你" = 1 + 3
  assert.equal(utf16ToByte(s, 3), 5); // "a你b" = 1 + 3 + 1
  assert.equal(byteToUtf16(s, 4), 2);
});

test("4-byte char (emoji, surrogate pair): 2 UTF-16 units = 4 bytes", () => {
  const s = "x😀y"; // 😀 is a surrogate PAIR (2 UTF-16 units), 4 UTF-8 bytes
  assert.equal(s.length, 4);
  assert.equal(utf16ToByte(s, 1), 1); // "x"
  assert.equal(utf16ToByte(s, 3), 5); // "x😀" = 1 + 4 (both surrogate units consumed)
  assert.equal(utf16ToByte(s, 4), 6); // "x😀y"
  // byte 5 is after the emoji → UTF-16 offset 3 (past both surrogate units)
  assert.equal(byteToUtf16(s, 5), 3);
});

test("round-trips: byteToUtf16 ∘ utf16ToByte is identity at every code-point boundary", () => {
  for (const s of ["café", "a你b😀c", "plain", "𝕏 = 𝕐"]) {
    // Walk code points (not units) so offsets land on real boundaries.
    let u = 0;
    for (const cp of s) {
      const b = utf16ToByte(s, u);
      assert.equal(b, trueBytes(s, u), `utf16ToByte mismatch in ${JSON.stringify(s)} @${u}`);
      assert.equal(byteToUtf16(s, b), u, `round-trip mismatch in ${JSON.stringify(s)} @${u}`);
      u += cp.length; // 1 or 2 UTF-16 units
    }
    // End of string
    assert.equal(utf16ToByte(s, s.length), enc.encode(s).length);
    assert.equal(byteToUtf16(s, enc.encode(s).length), s.length);
  }
});

test("out-of-range inputs clamp rather than overrun", () => {
  const s = "hi";
  assert.equal(utf16ToByte(s, 99), 2); // past end → full byte length
  assert.equal(byteToUtf16(s, 99), 2); // past end → full UTF-16 length
  assert.equal(utf16ToByte(s, 0), 0);
  assert.equal(byteToUtf16(s, 0), 0);
});
