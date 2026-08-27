/-
Unsigned LEB128 varint codec over `ByteArray`, plus a tiny byte cursor.

The oracle's wire frames (`Oracle.Frame`) length-prefix every count and blob with an unsigned
LEB128 varint, matching the varint discipline of the frozen `spec/contracts/ast-encoding.md`.
This module is the shared, total encode/decode for that varint and the cursor the frame decoder
threads through the input bytes.
-/

namespace Oracle.Leb

/-- Append the unsigned LEB128 encoding of `n` onto `acc`. -/
partial def encodeInto (acc : ByteArray) (n : Nat) : ByteArray :=
  let low := UInt8.ofNat (n % 0x80)
  let rest := n / 0x80
  if rest == 0 then
    acc.push low
  else
    encodeInto (acc.push (low ||| 0x80)) rest

/-- The unsigned LEB128 encoding of `n` as a fresh `ByteArray`. -/
def encode (n : Nat) : ByteArray :=
  encodeInto ByteArray.empty n

/--
A forward-only cursor over an immutable input buffer. `pos ≤ bytes.size` is the intended
invariant; every read returns an `Except` error rather than panicking past the end.
-/
structure Cursor where
  bytes : ByteArray
  pos : Nat
  deriving Inhabited

namespace Cursor

/-- A cursor positioned at the start of `bytes`. -/
def ofBytes (bytes : ByteArray) : Cursor := { bytes, pos := 0 }

/-- Bytes still unread. -/
def remaining (c : Cursor) : Nat := c.bytes.size - c.pos

/-- True once every byte has been consumed. -/
def atEnd (c : Cursor) : Bool := c.pos ≥ c.bytes.size

/-- Read one byte, advancing the cursor. -/
def readByte (c : Cursor) : Except String (UInt8 × Cursor) :=
  if h : c.pos < c.bytes.size then
    .ok (c.bytes[c.pos]'h, { c with pos := c.pos + 1 })
  else
    .error s!"cursor: read past end of input at offset {c.pos}"

/-- Read an unsigned LEB128 varint, advancing the cursor. Rejects an over-long encoding. -/
partial def readUleb (c : Cursor) : Except String (Nat × Cursor) :=
  let rec go (c : Cursor) (shift acc : Nat) : Except String (Nat × Cursor) := do
    let (b, c) ← c.readByte
    let acc := acc ||| ((b.toNat &&& 0x7f) <<< shift)
    if b.toNat &&& 0x80 == 0 then
      .ok (acc, c)
    else
      go c (shift + 7) acc
  go c 0 0

/-- Read exactly `n` bytes as a sub-buffer, advancing the cursor. -/
def readBytes (c : Cursor) (n : Nat) : Except String (ByteArray × Cursor) :=
  if c.pos + n ≤ c.bytes.size then
    .ok (c.bytes.extract c.pos (c.pos + n), { c with pos := c.pos + n })
  else
    .error s!"cursor: need {n} bytes but only {c.remaining} remain at offset {c.pos}"

/-- Read a LEB128 length prefix and then that many bytes. -/
def readLenPrefixed (c : Cursor) : Except String (ByteArray × Cursor) := do
  let (n, c) ← c.readUleb
  c.readBytes n

end Cursor

end Oracle.Leb
