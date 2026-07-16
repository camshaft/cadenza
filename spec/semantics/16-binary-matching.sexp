; Binary matching and construction — witnesses the binary-syntax decision
; (options/binary-syntax/). One `bin` form serves two directions, reusing the constructor/pattern
; duality the language already has (a variant `(Some 5)` builds and `(Some n)` destructures): in
; expression position `(bin <segment>...)` CONSTRUCTS a Bytes value; in pattern position it
; DESTRUCTURES a Bytes scrutinee inside an ordinary `match`. No new control construct — a binary pattern
; is a pattern like any other, and the `bin` head constrains the scrutinee to Bytes exactly as `(Some n)`
; constrains it to a sum.
;
; A `bin` is a sequence of segments, each written like a constructor `(<kind> <slot> <modifier>...)`.
; On CONSTRUCTION a fixed-width integer segment REQUIRES the width-matching typed value (the type carries
; the width, so the value provably fits — see the fit paragraph below):
;   (u8 v) (u16 v) (u32 v) (u64 v)   unsigned N-bit integer, BIG-ENDIAN by default; v : UInt<N>
;   (i8 v) (i16 v) (i32 v) (i64 v)   signed N-bit integer (two's complement); v : Int<N>
;   (uNN v le)                        the `le` modifier selects little-endian byte order
;   (bits v k)                        the low k bits of v; k is a COMPILE-TIME CONSTANT; v : (UInt k)
;   (bytes b)                         splice all of b (build); bind the REST (match, final segment only)
;   (bytes b n)                       exactly n bytes; n MAY be a name bound by an earlier segment
;                                       (dependent size) — the crown jewel, entirely value-level
; A literal in the slot means match-by-equality (magic numbers, opcodes) — the direct analogue of the
; existing literal patterns `(match 2 (2 "two") …)`. In MATCH position a segment binder decodes a general
; integer (a `(u16 m)` binder is a plain integer, losslessly holding the decoded field).
;
; Byte-alignment is STATIC: the whole `bin` must be byte-aligned. `bits` widths are compile-time
; constants so their running sum is checkable at compile time; a `bin` whose bits do not close a byte, a
; non-final unsized `bytes`, or a `bits` width that is not a constant is an ILL-FORMED BINARY FORM,
; rejected CDZ0220 (options/diagnostics-schema/, the CDZ02xx types-and-patterns band). FIT is by TYPE, not
; a runtime check: a fixed-width segment requires its exact width type (`(u8 v)` takes `UInt8`, `(bits v k)`
; takes `(UInt k)`), so a value that does not fit the segment is a COMPILE-TIME TYPE ERROR (CDZ0203), never
; a runtime trap — construction is TOTAL. A bare literal that overflows its segment grounds to that width
; and is a provable range error (CDZ0304 / CDZ0220). Narrowing a wider value is the CALLER's job and is
; explicit: `UInt8.wrap` truncates to the low 8 bits, `UInt8.of` narrows CHECKED. (This is why there is no
; "binary value does not fit segment" runtime trap — the width-typed value provably fits.) Exhaustiveness
; is the existing rule: a `bin` pattern never covers every byte
; sequence (empty input, wrong length, an unequal literal all fail to match), so a match over Bytes needs
; a catch-all arm or it is rejected CDZ0210 (core-semantics.md #Matching Is Exhaustive Or Rejected) — no
; special case, exactly like a non-exhaustive sum match.
;
; A later generation realizes the `bin` form (it subsumes the seed's
; `bytes` value form; options/realized-capability-set/). The seed does not realize it, so it
; DECLINES these cases — they pin the contract the realization must meet.

; ============================================================================================
; Construction — `(bin …)` in expression position builds a Bytes value
; ============================================================================================

(case "a u16 segment encodes an integer big-endian by default"
  (doc    "`(bin (u16 258))` encodes 258 (0x0102) as two bytes, most-significant first — big-endian is
           the default byte order, so the result is `(Bytes.of (list 1 2))`. Pins the default-endianness
           construction the wasm/network-order idiom depends on.")
  (input  (= (bin (u16 258)) (Bytes.of (list 1 2))))
  (output (: true Bool)))

(case "the le modifier encodes a u16 little-endian"
  (doc    "`(bin (u16 258 le))` selects little-endian with the `le` modifier, so 0x0102 is emitted
           least-significant byte first: `(Bytes.of (list 2 1))`. Pins that byte order is explicit and
           the modifier reverses the default, never an implicit host-endianness choice.")
  (input  (= (bin (u16 258 le)) (Bytes.of (list 2 1))))
  (output (: true Bool)))

(case "a u32 segment encodes four big-endian bytes"
  (doc    "`(bin (u32 0x89504E47))` encodes the magic number as four big-endian bytes
           `(Bytes.of (list 137 80 78 71))` — the fixed-width, whole-byte encoding a magic-number header
           is built from. Written as a hex literal (01-literals.sexp), the value reads as its bytes at a
           glance. Pins u32 width and byte order together.")
  (input  (= (bin (u32 0x89504E47)) (Bytes.of (list 137 80 78 71))))
  (output (: true Bool)))

(case "a signed i8 segment encodes as two's complement"
  (doc    "`(bin (i8 -1))` encodes -1 in a signed 8-bit segment as the two's-complement byte 255. Pins
           that a SIGNED segment admits a negative value (unlike an unsigned segment, which traps on one —
           see below), encoding it in two's complement.")
  (input  (= (bin (i8 -1)) (Bytes.of (list 255))))
  (output (: true Bool)))

(case "a u64 segment encodes eight big-endian bytes"
  (doc    "`(bin (u64 258))` encodes 258 as eight big-endian bytes — six leading zeros then 0x0102 —
           `(Bytes.of (list 0 0 0 0 0 0 1 2))`. Pins the widest fixed-width segment and that its width is
           eight bytes regardless of how small the value is, the companion of the u16 and u32 cases.")
  (input  (= (bin (u64 258)) (Bytes.of (list 0 0 0 0 0 0 1 2))))
  (output (: true Bool)))

(case "a multi-segment bin concatenates mixed-width signed and unsigned segments in order"
  (doc    "A `bin` with several integer segments of different widths and signedness lays them out
           left-to-right, each encoded independently. `(bin (u8 1) (u16 258) (i8 -1))` produces the u8 byte
           1, then the big-endian u16 258 = 0x0102 = two bytes 1 2, then the signed i8 −1 = two's-complement
           255 — the four bytes `(Bytes.of (list 1 1 2 255))`. Pins that segment widths, endianness, and
           signedness are applied per-segment and the results concatenated in source order (a builder that
           mis-ordered the segments, dropped a width, or sign-mishandled the i8 would differ), the
           integration of the single-segment u8/u16/i8 cases above into one bin.")
  (input  (= (bin (u8 1) (u16 258) (i8 -1)) (Bytes.of (list 1 1 2 255))))
  (output (: true Bool)))

(case "bit-field segments pack sub-byte values into one byte"
  (doc    "`(bin (bits 1 1) (bits 2 3) (bits 5 4))` packs a 1-bit flag (1), a 3-bit tag (2 = 0b010), and a
           4-bit value (5 = 0b0101), most-significant field first: 1·010·0101 = 0b1010_0101 = 165. The
           three widths sum to 8, so the `bin` closes exactly one byte. The expected byte is written as a
           binary literal `0b1010_0101` so the packed bit-fields are legible. Pins sub-byte bit-field
           packing and the most-significant-field-first order.")
  (input  (= (bin (bits 1 1) (bits 2 3) (bits 5 4)) (Bytes.of (list 0b1010_0101))))
  (output (: true Bool)))

(case "a bit-field wider than a byte packs across the byte boundary big-endian"
  (doc    "A `(bits v k)` segment may be WIDER than 8 bits, spanning multiple bytes: `(bin (bits 258 16))`
           packs 258 = 0x0102 into a 16-bit field, closing two bytes big-endian = `(Bytes.of (list 1 2))`.
           And bit-fields whose widths sum across a byte boundary pack contiguously: `(bin (bits 1 4) (bits
           0 8) (bits 0 4))` lays a nibble 1, then 8 zero bits, then a zero nibble — 16 bits = two bytes
           `(Bytes.of (list 16 0))` (the leading nibble 1 is the high 4 bits of byte 0 = 0x10). Pins that
           bit-field packing crosses byte boundaries most-significant-bit-first, not only sub-byte fields
           that close a single byte.")
  (input  (= (bin (bits 258 16)) (Bytes.of (list 1 2))))
  (output (: true Bool)))

(case "a bit-field value that needs more bits than its width does not fit its segment"
  (doc    "A `(bits v k)` segment holds only the low `k` bits, so a value needing more than `k` bits has no
           defined encoding — construction is rejected (`binary value does not fit segment`, CDZ0304, the
           value-overflow companion of a u8 given 256). `(bin (bits 16 4) (bits 0 4))` gives 16, which needs
           5 bits, to a 4-bit field — rejected. Pins that a bit-field range-checks its value against its
           declared width, not silently truncating it to the low bits (which would encode 0 and lose data).")
  (input  (bin (bits 16 4) (bits 0 4)))
  (error  CDZ0304))

(case "a length-prefixed frame is built from a size segment and a bytes segment"
  (doc    "`(bin (u16 (UInt16.of (Bytes.len payload))) (bytes payload))` writes payload's length as a
           big-endian u16 prefix, then splices payload — the length-framing idiom that replaces hand-rolled
           `(Bytes.concat (Bytes.of (list (& (>> len 8) 255) (& len 255))) payload)`. The `u16` segment takes
           a `UInt16`, so the caller narrows the `Int64` length with `UInt16.of` (a CHECKED narrow — a
           payload too long to frame in 16 bits is a real error, not a silent truncation); the length 3 fits.
           Pins that a computed value narrowed to the segment width feeds a size segment and an unsized
           `(bytes …)` splices a whole Bytes value.")
  (input  (= (bin (u16 (UInt16.of (Bytes.len (Bytes.of (list 10 20 30))))) (bytes (Bytes.of (list 10 20 30))))
             (Bytes.of (list 0 3 10 20 30))))
  (output (: true Bool)))

(case "an empty binary form is the empty byte sequence"
  (doc    "`(bin)` with no segments is the zero-length Bytes value, equal to `(Bytes.of (list))`. Pins the
           degenerate construction — the identity a fold over segments starts from.")
  (input  (= (bin) (Bytes.of (list))))
  (output (: true Bool)))

(case "the length of a fixed-width construction is the sum of its segment widths"
  (doc    "`(Bytes.len (bin (u32 0)))` = 4: a u32 segment is four bytes wide regardless of the value it
           carries. Pins that fixed-width segments contribute their width, not a value-dependent length.")
  (input  (Bytes.len (bin (u32 0))))
  (output (: 4 Int64)))

; ============================================================================================
; Matching — `(bin …)` in pattern position destructures a Bytes scrutinee
; ============================================================================================

(case "a bin pattern binds an integer read from a fixed-width segment"
  (doc    "Matching `(bin (u16 258))` against the pattern `(bin (u16 n))` reads the big-endian u16 back
           into n, round-tripping the construction: n = 258. Pins that construction and matching are
           inverse over the same segment grammar.")
  (input  (match (bin (u16 258))
            ((bin (u16 n)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "a le pattern segment reads a fixed-width integer little-endian"
  (doc    "The construction `(bin (u16 258 le))` emitted `(list 2 1)`; matching those same bytes against
           `(bin (u16 n le))` reads them back least-significant byte first, recovering n = 258. Pins that
           the `le` modifier is honored in pattern position exactly as in expression position — matching is
           the inverse of construction over the same modifier, not big-endian-only on the way in.")
  (input  (match (bin (u16 258 le))
            ((bin (u16 n le)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "a signed pattern segment reads a two's-complement integer as negative"
  (doc    "The byte 255 read through a SIGNED `(i8 n)` pattern is -1, not 255 — a signed segment
           interprets its two's-complement bits as a signed integer, the inverse of the `(i8 -1)`
           construction that emitted 255. Pins that signedness governs matching too: the same byte reads
           back as -1 under `i8` and as 255 under `u8` (next case).")
  (input  (match (Bytes.of (list 255))
            ((bin (i8 n)) n)
            (_ 0)))
  (output (: -1 Int64)))

(case "an unsigned pattern segment reads the same byte as a non-negative integer"
  (doc    "The companion of the signed-read case: the byte 255 read through an UNSIGNED `(u8 n)` pattern is
           255. The one byte reads back as -1 under `i8` and 255 under `u8`, so signedness is a property of
           the segment, not the bytes. Pins that the two readings of one byte differ precisely by the
           segment's sign, mirroring the construction-side split between `(i8 -1)` and `(u8 -1)`.")
  (input  (match (Bytes.of (list 255))
            ((bin (u8 n)) n)
            (_ 0)))
  (output (: 255 Int64)))

(case "a signed segment reads the sign-bit-only byte as the minimum, not its magnitude"
  (doc    "The 0x80 boundary, sharper than the 0xFF cases above (which read the trivial all-ones byte as
           -1): the byte 128 (`0b1000_0000`, only the sign bit set) read through a SIGNED `(i8 n)` pattern
           is -128 — the Int8 MINIMUM, its two's-complement value — NOT 128 (its magnitude) and NOT -0. This
           is where a naive reinterpretation that masks the sign bit and negates the remaining magnitude
           would wrongly give 0 or -0; the correct reading is the two's-complement -128. Pins the
           sign-bit-only extreme of the signed segment read.")
  (input  (match (Bytes.of (list 128))
            ((bin (i8 n)) n)
            (_ 0)))
  (output (: -128 Int64)))

(case "an unsigned segment reads the same sign-bit-only byte as 128"
  (doc    "The unsigned companion: the byte 128 read through `(u8 n)` is 128 — the same byte reads back as
           -128 under `i8` and 128 under `u8`, differing by exactly the segment's sign at the sign-bit
           boundary (as the 0xFF pair does at the all-ones byte). Pins that the two readings of 0x80 split
           by segment sign, so signedness is a property of the segment across the whole byte range, not
           just the all-ones case.")
  (input  (match (Bytes.of (list 128))
            ((bin (u8 n)) n)
            (_ 0)))
  (output (: 128 Int64)))

(case "constructing a signed segment at the minimum emits the sign-bit-only byte"
  (doc    "The construction inverse of the signed 0x80 read: `(bin (i8 -128))` encodes the Int8 minimum in
           two's complement as the byte 0x80 = 128, so it equals `(Bytes.of (list 128))`. With the read
           case above this pins the round-trip at the signed minimum — `-128` emits 0x80, and 0x80 read as
           `i8` is `-128` — the extreme companion of the `(i8 -1)` ⇆ 255 round-trip.")
  (input  (= (bin (i8 -128)) (Bytes.of (list 128))))
  (output (: true Bool)))

; The round-trip cases above use mid-range values (258) and the i8 extremes (-1, -128). These pin the
; MULTI-BYTE-WIDTH extremes — where an off-by-one in the shift/mask byte-assembly or a sign-extension slip
; would surface: u16 at its max (65535 = every bit set across two bytes), u32 at its max (four bytes all
; 0xFF), and i16 at its two's-complement minimum (-32768 = 0x8000). Each constructs then matches back to the
; same value, both backends.

(case "a u16 segment round-trips at its maximum value"
  (doc    "`(bin (u16 65535))` — the u16 maximum, every bit set across two big-endian bytes (0xFF 0xFF) —
           matched by `(bin (u16 n))` reads back 65535. Pins the round-trip at the u16 ceiling: a byte-
           assembly that dropped or mis-shifted the high byte would read a smaller value. The extreme
           companion of the mid-range `(u16 258)` round-trip.")
  (input  (match (bin (u16 65535)) ((bin (u16 n)) n) (_ -1)))
  (output (: 65535 Int64)))

(case "a u32 segment round-trips at its maximum value"
  (doc    "`(bin (u32 4294967295))` — the u32 maximum, four big-endian bytes all 0xFF — matched by `(bin
           (u32 n))` reads back 4294967295. Pins the four-byte big-endian assembly is exact at the ceiling
           (a value past u16 range, so all four bytes carry significant bits); an off-by-one in the 8/16/24-
           bit shifts would corrupt it.")
  (input  (match (bin (u32 4294967295)) ((bin (u32 n)) n) (_ -1)))
  (output (: 4294967295 Int64)))

(case "an i16 segment round-trips at its two's-complement minimum"
  (doc    "`(bin (i16 -32768))` — the Int16 minimum, 0x8000 (only the sign bit set across two bytes) —
           matched by `(bin (i16 n))` reads back -32768, NOT +32768 or a mis-sign-extended value. Pins the
           signed multi-byte round-trip at the sign-bit extreme, the i16 companion of the `(i8 -128)` ⇆
           0x80 round-trip.")
  (input  (match (bin (i16 -32768)) ((bin (i16 n)) n) (_ 0)))
  (output (: -32768 Int64)))

(case "a bits pattern segment reads sub-byte fields back into integers"
  (doc    "The packed byte `0b1010_0101` matched against `(bin (bits a 1) (bits b 2) (bits c 5))` reads the
           three most-significant-field-first sub-byte fields back: a = 1, b = 0b01 = 1, c = 0b00101 = 5,
           whose sum a + b + c is 7. Pins that `bits` segments read in the same most-significant-first order
           they pack in — the inverse of the bit-field construction case — and that a bits-only pattern must
           still close a whole byte to be well-formed.")
  (input  (match (Bytes.of (list 0b1010_0101))
            ((bin (bits a 1) (bits b 2) (bits c 5)) (+ (+ a b) c))
            (_ 0)))
  (output (: 7 Int64)))

(case "a u64 pattern segment reads eight big-endian bytes back into an integer"
  (doc    "The eight bytes `(list 0 0 0 0 0 0 1 2)` matched against `(bin (u64 n))` read back big-endian as
           n = 258, round-tripping the u64 construction case. Pins the widest fixed-width segment in pattern
           position and that it consumes exactly its eight bytes.")
  (input  (match (bin (u64 258))
            ((bin (u64 n)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "a literal segment before a binder dispatches then reads the payload"
  (doc    "The pattern `(bin (u8 1) (u16 n))` fires only when the first byte equals the tag 1, then reads
           the following big-endian u16 as n = 258. Against a leading tag of 1 the arm matches and yields
           258; a fixed literal and a binder compose in one pattern. Pins tag-then-field dispatch, the shape
           a tagged binary record takes.")
  (input  (match (Bytes.of (list 1 1 2))
            ((bin (u8 1) (u16 n)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "a bin pattern must consume the whole scrutinee or it does not match"
  (doc    "The scrutinee is three bytes but the arm `(bin (u16 n))` describes only two, leaving one byte
           unconsumed, so the arm does NOT match and control falls to the catch-all, yielding 0. Pins that a
           `bin` pattern matches the ENTIRE byte sequence — leftover bytes are a non-match, which is why a
           trailing `(bytes rest)` segment is needed to accept a variable-length remainder (the next case).
           This is the whole-scrutinee accounting a length-framing loop relies on.")
  (input  (match (Bytes.of (list 1 2 3))
            ((bin (u16 n)) n)
            (_ 0)))
  (output (: 0 Int64)))

(case "a trailing bytes segment accepts the leftover the fixed segments leave"
  (doc    "The companion of the whole-consumption case: adding a final `(bytes rest)` to the same
           three-byte scrutinee lets the u16 read the first two bytes (n = 258) and `rest` absorb the third,
           so the arm matches and yields n = 258. Pins that a trailing unsized `(bytes …)` is exactly what
           relaxes the whole-scrutinee rule to accept a variable-length tail.")
  (input  (match (Bytes.of (list 1 2 3))
            ((bin (u16 n) (bytes rest)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "an empty scrutinee matches an empty bin pattern"
  (doc    "The zero-length Bytes value matches the empty pattern `(bin)` — no segments to read, nothing
           left over — so the arm fires and yields \"empty\". The inverse of the `(bin)` construction case,
           and the base case a recursive framing parser bottoms out on. Pins that `(bin)` in pattern
           position matches exactly the empty sequence.")
  (input  (match (Bytes.of (list))
            ((bin) "empty")
            (_ "nonempty")))
  (output (: "empty" String)))

(case "a non-empty scrutinee does not match an empty bin pattern"
  (doc    "The companion of the empty-matches-empty case: a one-byte scrutinee has a leftover byte the empty
           pattern `(bin)` does not consume, so the arm does not match and control falls to the catch-all.
           Pins that `(bin)` matches ONLY the empty sequence — the whole-consumption rule applied to the
           zero-segment pattern.")
  (input  (match (Bytes.of (list 0))
            ((bin) "empty")
            (_ "nonempty")))
  (output (: "nonempty" String)))

(case "a dependent-size segment binds exactly the number of bytes an earlier segment named"
  (doc    "Against `(Bytes.of (list 2 10 20 99))`, the pattern `(bin (u8 n) (bytes body n) (bytes rest))`
           reads n = 2 from the first byte, then binds `body` to exactly the next n = 2 bytes
           `(list 10 20)`, leaving `rest` = `(list 99)`. The crown jewel: a segment's size is a value
           bound earlier in the same pattern, all value-level. This case checks `body`; the next checks
           `rest`.")
  (input  (match (Bytes.of (list 2 10 20 99))
            ((bin (u8 n) (bytes body n) (bytes rest)) (= body (Bytes.of (list 10 20))))
            (_ false)))
  (output (: true Bool)))

(case "a final bytes segment binds the remainder after the sized segments"
  (doc    "The companion of the dependent-size case: the same match returns `rest`, the bytes after the
           n-byte body — `(list 99)`. Pins that a final unsized `(bytes rest)` captures everything left,
           the remainder a framing loop iterates on.")
  (input  (match (Bytes.of (list 2 10 20 99))
            ((bin (u8 n) (bytes body n) (bytes rest)) (= rest (Bytes.of (list 99))))
            (_ false)))
  (output (: true Bool)))

(case "a literal segment matches a magic-number header by equality"
  (doc    "The pattern `(bin (u32 0x89504E47) (bytes rest))` matches the scrutinee only when its first
           four bytes equal the magic number (137 80 78 71) — a literal segment matches by equality, the
           direct analogue of a literal value pattern, and the hex literal names the magic number
           legibly. Pins magic-number dispatch on a binary header.")
  (input  (match (Bytes.of (list 137 80 78 71 1 2))
            ((bin (u32 0x89504E47) (bytes rest)) "match")
            (_ "other")))
  (output (: "match" String)))

(case "a bin arm whose fixed-width segment overruns the input falls through"
  (doc    "The scrutinee `(Bytes.of (list 5))` is one byte; the arm `(bin (u16 n) (bytes rest))` needs
           two bytes for its u16, so it cannot match and control falls to the catch-all, yielding 0.
           Pins that too-short input is a non-match (the arm simply does not fire), not a trap — the same
           total-or-trap discipline the corpus pins for a `bytes` segment that overruns.")
  (input  (match (Bytes.of (list 5))
            ((bin (u16 n) (bytes rest)) n)
            (_ 0)))
  (output (: 0 Int64)))

(case "a dependent-size segment that overruns the remaining bytes falls through"
  (doc    "Against `(Bytes.of (list 9 1 2))`, the pattern `(bin (u8 n) (bytes body n))` reads n = 9 but
           only two bytes remain, so `(bytes body 9)` cannot bind nine bytes and the arm falls to the
           catch-all. Pins that a dependent size larger than the remainder is a non-match, not a trap or
           a short read.")
  (input  (match (Bytes.of (list 9 1 2))
            ((bin (u8 n) (bytes body n)) (Bytes.len body))
            (_ -1)))
  (output (: -1 Int64)))

(case "a dependent size of zero binds an empty body and leaves the rest untouched"
  (doc    "Against `(Bytes.of (list 0 42))`, the pattern `(bin (u8 n) (bytes body n) (bytes rest))` reads
           n = 0, so `(bytes body 0)` binds the EMPTY byte sequence and `rest` gets the whole remainder
           `(list 42)`. This case checks `body` is empty. Pins that a zero dependent size is a valid
           non-match-free read (an empty field), not an overrun or a special case — the degenerate framing a
           loop hits on a zero-length record.")
  (input  (match (Bytes.of (list 0 42))
            ((bin (u8 n) (bytes body n) (bytes rest)) (= body (Bytes.of (list))))
            (_ false)))
  (output (: true Bool)))

(case "two dependent-size segments each bind the count a preceding segment named"
  (doc    "Against `(Bytes.of (list 1 2 2 10 20 99))`, the pattern
           `(bin (u8 a) (bytes x a) (u8 b) (bytes y b) (bytes rest))` reads the two length-prefixed fields
           in sequence: a = 1 → `x` = `(list 2)`; then b = 2 → `y` = `(list 10 20)`; the final byte 99 lands
           in `rest`. This case checks `y`. Pins that several dependent sizes chain in one pattern, each
           reading a count bound just before it — the sequential length-prefixed framing a single `bin`
           expresses without a loop.")
  (input  (match (Bytes.of (list 1 2 2 10 20 99))
            ((bin (u8 a) (bytes x a) (u8 b) (bytes y b) (bytes rest)) (= y (Bytes.of (list 10 20))))
            (_ false)))
  (output (: true Bool)))

(case "constructing a sized bytes segment whose value length differs from the size is rejected"
  (doc    "`(bin (bytes (Bytes.of (list 1 2 3)) 2))` splices a three-byte value into a segment declared to
           be two bytes wide; the declared size and the value's length disagree, so there is no defined
           encoding. With CONSTANT operands the mismatch is provable at compile time, so — like every
           compile-provable trap — it FAILS THE BUILD (CDZ0304) rather than shipping a component that traps
           (reference-compiler.md #A Compile-Provable Trap Fails The Build); a runtime `b`/`n` whose lengths
           disagree traps \"binary value does not fit segment\" at that point. Pins that a SIZED `(bytes b
           n)` build is length-checked against n — the whole-value analogue of the u8 out-of-range check,
           and the construction-side counterpart of a matching `(bytes body n)` overrun being a non-match.")
  (input  (bin (bytes (Bytes.of (list 1 2 3)) 2)))
  (error  CDZ0304))

; --- A bin pattern applies only to a Bytes scrutinee -----------------------------------------
; A `(bin …)` pattern DECODES a Bytes value, so it is well-formed only over a Bytes scrutinee. Matching
; it against a definite NON-Bytes scrutinee — an Int64, a String, a List — is a type error (CDZ0203, 'a
; `(bin …)` pattern decodes a Bytes value, but this scrutinee is <T>'), the bin twin of the map-key /
; list-element pattern-type checks. It is caught at the offending arm, not silently accepted (which left
; the arm to fall through to a misleading generic 'pattern not yet supported'). An unsolved (`Any`/`Var`)
; scrutinee is skipped — a runtime Bytes may still flow in — so the reject fires only on a DEFINITE
; non-Bytes type, whether a constant or a runtime parameter.

(case "a bin pattern over an Int64 scrutinee is a type error"
  (doc    "`(match 5 ((bin (u8 x)) x) (_ 0))` matches a `(bin …)` pattern against the Int64 `5`. A bin
           pattern decodes a Bytes value, and an Int64 is not Bytes, so it is rejected (CDZ0203, naming the
           scrutinee's type). Pins that the bin pattern's scrutinee-type check fires on a definite non-Bytes
           scalar — the binary-matching companion of a list pattern over a non-list scrutinee.")
  (input  (do (def (main) (match 5 ((bin (u8 x)) x) (_ 0))) (export main)))
  (error  CDZ0203))

(case "a bin pattern over a String scrutinee is a type error"
  (doc    "`(match \"hi\" ((bin (u8 x)) x) (_ 0))` — a String is not Bytes (text vs a byte sequence are
           distinct types; the bridge is `String.to-bytes`/`from-bytes`), so a bin pattern over it is
           rejected (CDZ0203). Pins that a String scrutinee does not silently decode as bytes — the author
           must encode it first.")
  (input  (do (def (main) (match "hi" ((bin (u8 x)) x) (_ 0))) (export main)))
  (error  CDZ0203))

(case "a bin pattern over a List scrutinee is a type error"
  (doc    "`(match (list 1 2) ((bin (u8 x)) x) (_ 0))` — a `(List Int64)` is not Bytes (a list of integers
           is not a byte sequence; `Bytes.of` is the explicit bridge), rejected CDZ0203. Pins that the
           scrutinee-type check covers a compound collection, not only a scalar.")
  (input  (do (def (main) (match (list 1 2) ((bin (u8 x)) x) (_ 0))) (export main)))
  (error  CDZ0203))

(case "a bin pattern over a runtime non-Bytes scrutinee is a type error"
  (doc    "`(match n ((bin (u8 x)) x) (_ 0))` with `n` a runtime Int64 parameter — the scrutinee is a
           definite non-Bytes type known statically even though its value arrives at run time, so the reject
           still fires (CDZ0203). Pins that the check is on the scrutinee's static TYPE, not whether it is a
           constant; a runtime Int64 is rejected exactly as the constant `5` is (distinct from an unsolved
           `Any`/`Var` scrutinee, which is skipped because a runtime Bytes may flow in).")
  (input  (do (def (main (: n Int64)) (match n ((bin (u8 x)) x) (_ 0))) (export main)))
  (error  CDZ0203))

; ============================================================================================
; Protocol round-trips — construct and match are inverse over a whole realistic layout
; ============================================================================================

(case "a tag-length-value record round-trips through construct then match"
  (doc    "A TLV record is built `(bin (u8 7) (u16 3) (bytes payload))` — a one-byte tag, a big-endian u16
           length, then the payload — and matched back with `(bin (u8 7) (u16 n) (bytes body n))`: the
           literal tag 7 dispatches, the length n = 3 sizes the dependent `body`, which recovers the
           original payload. Pins the canonical tag-length-value shape in one expression, showing the
           literal, fixed-width, and dependent-size segments compose into a real record grammar.")
  (input  (match (bin (u8 7) (u16 3) (bytes (Bytes.of (list 100 101 102))))
            ((bin (u8 7) (u16 n) (bytes body n)) (= body (Bytes.of (list 100 101 102))))
            (_ false)))
  (output (: true Bool)))

(case "a magic header and a length-prefixed chunk parse together"
  (doc    "A PNG-style layout — the u32 magic `0x89504E47`, a u32 chunk length, then that many data bytes —
           is built `(bin (u32 0x89504E47) (u32 2) (bytes data))` and parsed with
           `(bin (u32 0x89504E47) (u32 len) (bytes body len))`: the literal magic segment gates the parse
           and the u32 length sizes the chunk body. Pins that a magic-number guard and a dependent-size
           chunk chain in one pattern — the shape a chunked container format (PNG, RIFF) is read with.")
  (input  (match (bin (u32 0x89504E47) (u32 2) (bytes (Bytes.of (list 65 66))))
            ((bin (u32 0x89504E47) (u32 len) (bytes body len)) (= body (Bytes.of (list 65 66))))
            (_ false)))
  (output (: true Bool)))

(case "parsing a length-framed message and rebuilding it yields the original bytes"
  (doc    "The strongest inverse statement: a length-framed message is matched with
           `(bin (u16 n) (bytes body n))`, then rebuilt from the bound `n` and `body` with
           `(bin (u16 n) (bytes body))`, and the rebuilt bytes equal the original frame. Pins that
           construction and matching are genuinely inverse — parse-then-serialize is the identity on a
           well-formed frame — not merely that each direction works in isolation.")
  (input  (let ((frame (Bytes.of (list 0 3 10 20 30))))
            (match frame
              ((bin (u16 n) (bytes body n)) (= (bin (u16 n) (bytes body)) frame))
              (_ false))))
  (output (: true Bool)))

(case "a header of packed nibbles and a length-prefixed body round-trips"
  (doc    "A header packs a 4-bit version and 4-bit flags into one byte, followed by a u16 length and that
           many payload bytes: built `(bin (bits 1 4) (bits 2 4) (u16 2) (bytes payload))`, matched with
           `(bin (bits ver 4) (bits flags 4) (u16 n) (bytes body n))`, then rebuilt and compared to the
           original. Pins that sub-byte bit-fields participate in a full round-trip alongside byte-aligned
           segments — the mixed bit-and-byte header a wire protocol actually uses.")
  (input  (let ((msg (bin (bits 1 4) (bits 2 4) (u16 2) (bytes (Bytes.of (list 9 9))))))
            (match msg
              ((bin (bits ver 4) (bits flags 4) (u16 n) (bytes body n))
               (= (bin (bits ver 4) (bits flags 4) (u16 n) (bytes body)) msg))
              (_ false))))
  (output (: true Bool)))

; ============================================================================================
; Exhaustiveness — a match over Bytes needs a catch-all (existing CDZ0210 rule, no special case)
; ============================================================================================

(case "a match over bytes with only a bin arm and no catch-all is non-exhaustive"
  (doc    "A `bin` pattern never covers every byte sequence — the empty sequence, a shorter sequence, or
           one whose literal segments differ all fail to match — so a match whose only arm is a `(bin …)`
           pattern does not cover the scrutinee's type and is rejected CDZ0210, exactly as a sum match
           missing a variant is. Pins that binary matching reuses the exhaustiveness rule rather than a
           special case. A generation that does not yet cover the rule declines (todo), not miscompiles.")
  (input  (match (Bytes.of (list 1 2))
            ((bin (u16 n)) n)))
  (error  CDZ0210))

; ============================================================================================
; Ill-formed binary forms — static rejection CDZ0220 (byte-alignment and well-formedness)
; ============================================================================================

(case "a binary form whose bit-fields do not close a byte is ill-formed"
  (doc    "`(bin (bits 1 1) (bits 0 3))` has bit-field widths summing to 4, so the form is not
           byte-aligned and no whole number of bytes is emitted. Because the widths are compile-time
           constants the misalignment is caught statically: an ill-formed binary form, rejected CDZ0220.
           Pins the byte-alignment discipline as a compile-time check, not a runtime surprise.")
  (input  (bin (bits 1 1) (bits 0 3)))
  (error  CDZ0220))

(case "a non-final unsized bytes segment is ill-formed"
  (doc    "`(bin (bytes a) (u8 1))` places an unsized `(bytes a)` before another segment: an unsized
           bytes segment consumes all remaining bytes, so anything after it can never be reached, an
           ill-formed binary form rejected CDZ0220. Pins that an unsized `bytes` is legal only as the
           final segment (a sized `(bytes a n)` may appear anywhere).")
  (input  (bin (bytes (Bytes.of (list 1 2))) (u8 1)))
  (error  CDZ0220))

(case "a bit-field width that is not a compile-time constant is ill-formed"
  (doc    "`(bin (bits 1 k))` uses a run-time value k as a bit-field width; a `bits` width must be a
           compile-time constant so the form's byte-alignment is statically checkable. A non-constant
           width is an ill-formed binary form rejected CDZ0220. Pins that widths are static even though
           the values filling them are dynamic.")
  (input  (let ((k 3)) (bin (bits 1 k))))
  (error  CDZ0220))

; ============================================================================================
; Unrecognized segment KIND — the kind head names no segment (CDZ0201, distinct from CDZ0220)
; ============================================================================================
; A segment's KIND head must be one of the closed vocabulary `bits`/`bytes`/`utf8`/`u8..u64`/`i8..i64`.
; A head outside it names no segment — a type error (CDZ0201), distinct from the CDZ0220 well-formedness
; failures above (which have a valid kind but a bad LAYOUT: an unclosed byte, a non-final unsized segment,
; a non-constant width). A CONFIDENT typo of a known kind (`byte`→`bytes`, `utf`→`utf8`, `bit`→`bits`)
; carries a rename suggestion — the bin-segment analogue of the member/variant did-you-mean; a wrong
; INTEGER WIDTH (`u9`) keeps its own dedicated "must be a byte-aligned width" message pointing at `bits`;
; a far miss keeps the plain "unrecognized kind" message. All CDZ0201; the message differs by how close.

(case "a misspelled segment kind is rejected with a rename suggestion"
  (doc    "`(bin (byte 5))` uses `byte` where the kind is `bytes` — a confident typo of a known segment
           kind. The kind head names no segment, so it is rejected (CDZ0201, 'unrecognized bin segment kind
           `byte` — did you mean `bytes`?'), the bin-segment did-you-mean. Pins the misspelled-kind path
           (the rename fix), distinct from a valid kind with a bad layout (CDZ0220 above).")
  (input  (do (def (main) (bin (byte 5))) (export main)))
  (error  CDZ0201))

(case "a misspelled utf8 segment kind is rejected"
  (doc    "`(bin (utf 65))` uses `utf` for `utf8` — another confident kind typo, rejected CDZ0201 with the
           `utf8` rename suggestion. Pins that the did-you-mean covers the text-segment kind as well as
           `bytes`, so a near-miss on any closed-vocabulary kind is named.")
  (input  (do (def (main) (bin (utf 65))) (export main)))
  (error  CDZ0201))

(case "a fixed-width integer segment of a non-byte-aligned width is rejected"
  (doc    "`(bin (u9 5))` names a 9-bit unsigned integer segment — `uNN` segments are the byte-aligned
           widths u8/u16/u32/u64 only, so `u9` is not a segment kind (CDZ0201, with the dedicated message
           pointing at `(bits v k)` for an arbitrary bit width). Distinct from a plain typo: the head is a
           recognizable `u`-integer shape at a width the byte-aligned segments do not offer, so it keeps its
           own message rather than a rename. Pins the wrong-integer-width path.")
  (input  (do (def (main) (bin (u9 5))) (export main)))
  (error  CDZ0201))

(case "a far-miss segment kind keeps the plain unrecognized message"
  (doc    "`(bin (zzz 5))` uses `zzz` — a head that is not close to any known kind. It is rejected
           (CDZ0201) with the plain 'unrecognized bin segment kind (expected uNN/iNN/bits/bytes/utf8)'
           message, no rename fix (there is no confident correction). Pins the far-miss path, the
           complement of the misspelled-kind case: the vocabulary is closed and a head outside it — near or
           far — is rejected.")
  (input  (do (def (main) (bin (zzz 5))) (export main)))
  (error  CDZ0201))

; ============================================================================================
; Fit — a segment REQUIRES its width-typed value. A CONSTANT literal grounds to that width and a
; provable overflow FAILS THE BUILD (CDZ0304 / CDZ0220); a NON-CONSTANT value of a different integer
; type is a COMPILE-TIME TYPE ERROR (CDZ0203). A value that fits its type has no out-of-range case, so
; construction NEVER traps — an out-of-range value is a type failure, not a runtime failure.
; ============================================================================================

(case "constructing a u8 segment from a value above its range is rejected"
  (doc    "`(bin (u8 256))` asks an 8-bit unsigned segment to hold 256, which needs nine bits and has no
           8-bit encoding — it does NOT truncate to 0. The literal grounds to the segment's `UInt8` and the
           overflow is provable, so it FAILS THE BUILD (CDZ0304) — the compile-provable-trap rule
           (reference-compiler.md #A Compile-Provable Trap Fails The Build). The companion of the Bytes
           out-of-range check, at the segment boundary. (A non-constant value of the wrong type is a
           CDZ0203 type error — see the runtime section.)")
  (input  (bin (u8 256)))
  (error  CDZ0304))

(case "constructing an unsigned segment from a negative value is rejected"
  (doc    "`(bin (u8 -1))` gives a negative value to an UNSIGNED segment, which has no negative encoding —
           it does NOT wrap to 255 (that is the meaning of the SIGNED `(i8 -1)` case above). The literal
           grounds to the segment's `UInt8` and the negative is out of range, so it FAILS THE BUILD
           (CDZ0304). Pins that unsigned and signed segments differ on a negative value: the signed one
           encodes it in two's complement, the unsigned one has no encoding.")
  (input  (bin (u8 -1)))
  (error  CDZ0304))

(case "constructing a bit-field from a value wider than its width is rejected"
  (doc    "`(bin (bits 2 1))` gives the value 2 (which needs two bits) to a 1-bit field, so it does not fit.
           With a CONSTANT operand the misfit is provable at compile time, so the ill-formed bit-field is
           rejected (CDZ0220 — the binary well-formedness code). Pins that a bit-field's value is
           range-checked against its width, the sub-byte companion of the u8-overflow check.")
  (input  (bin (bits 2 1)))
  (error  CDZ0220))

(case "a narrower-typed runtime value is not silently widened into a wider segment"
  (doc    "The segment's required type is EXACT in both width and signedness — it is a TYPE match, not a
           value-range fit. A runtime `UInt8` fed to a `u16` segment is a COMPILE-TIME type error (CDZ0203),
           even though every `UInt8` value trivially fits 16 bits: widening is as explicit as narrowing, never
           implicit (the caller writes `UInt16.of` / `UInt16.wrap`). Pins that `(u16 v)` requires a `UInt16`,
           not merely an integer that fits — so a future change cannot quietly accept any narrower unsigned
           value. The widening companion of the wider-value rejection (an `Int64` into `u8`).")
  (input  (do (def (main (: n UInt8)) (Bytes.len (bin (u16 n)))) (export main)))
  (error  CDZ0203))

(case "an unsigned runtime value is not accepted by a signed segment of the same width"
  (doc    "Signedness is strict too: a runtime `UInt8` fed to a SIGNED `i8` segment is a COMPILE-TIME type
           error (CDZ0203) — `(i8 v)` requires an `Int8`, not a same-width unsigned value (their encodings of
           a high value differ: a `UInt8` 200 is not the `Int8` −56). Pins that the segment match is on BOTH
           axes, the signedness companion of the no-silent-widening case.")
  (input  (do (def (main (: n UInt8)) (Bytes.len (bin (i8 n)))) (export main)))
  (error  CDZ0203))

; ============================================================================================
; Runtime segments — a `bin` whose segment value / scrutinee is a RUNTIME value (a def parameter,
; not a compile-time constant). A fixed-width integer segment REQUIRES the width-matching typed value —
; `(u16 v)` takes a `UInt16`, `(bits v k)` takes a `(UInt k)` — so the value provably fits the segment and
; construction NEVER traps: an out-of-range value is a COMPILE-TIME TYPE ERROR (CDZ0203), and narrowing
; (`UInt8.wrap` to truncate, `UInt8.of` to check) is the caller's job. Matching decodes the scrutinee's
; bytes at run time, binding a general integer. Each threads its value through `main`'s parameter (via
; `(call …)`) so the `bin` cannot fold to a constant.
; ============================================================================================

(case "a runtime value is constructed into a fixed-width segment and its length read"
  (doc    "`(bin (u16 n))` with `n` a RUNTIME `UInt16` parameter (not a constant) builds a two-byte sequence
           on the byte heap at run time — the construction does not fold. Reading its length back yields 2,
           the segment's width. Pins that a `bin` construction whose value is only known at run time still
           produces a well-formed Bytes, and that a `u16` segment takes a `UInt16` value.")
  (input  (do (def (main (: n UInt16)) (Bytes.len (bin (u16 n)))) (export main)))
  (call   main (: 258 UInt16))
  (output (: 2 Int64)))

(case "a runtime fixed-width segment is emitted big-endian and read back by index"
  (doc    "`(bin (u16 258))` built at run time encodes 258 = 0x0102 BIG-ENDIAN, so byte 0 is the
           most-significant `0x01` = 1. Reads it back with `Bytes.at`. Pins that a runtime construction
           lays its bytes most-significant-first, the same order the constant fold and the pattern decode
           agree on.")
  (input  (do (def (main (: n UInt16))
                (match (Bytes.at (bin (u16 n)) 0) ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 258 UInt16))
  (output (: 1 Int64)))

(case "a wider runtime value in a fixed-width segment is a compile-time type error"
  (doc    "`(bin (u8 n))` with `n` a runtime `Int64` is a COMPILE-TIME TYPE ERROR (CDZ0203): a `u8` segment
           takes a `UInt8`, and an `Int64` may exceed the 8-bit range, so it is NOT silently accepted and
           range-checked-then-trapped at run time. The value that does not fit becomes a type failure, not a
           runtime failure — the caller must convert explicitly (`UInt8.wrap` to truncate to the low 8 bits,
           `UInt8.of` for a checked narrow). Replaces the former runtime out-of-range TRAP: a width-typed
           segment provably fits its value, so construction never traps.")
  (input  (do (def (main (: n Int64)) (Bytes.len (bin (u8 n)))) (export main)))
  (error  CDZ0203))

(case "a runtime value narrowed to the segment width constructs without trapping"
  (doc    "The idiom the type error above directs the caller to: `(bin (u8 (UInt8.wrap n)))` with a runtime
           `Int64 n` narrows `n` to `UInt8` (truncating to the low 8 bits, numeric-model.md #wrap Never
           Traps) BEFORE placing it in the segment, so the `u8` segment gets a value it provably fits and
           construction is total. `n = 258` wraps to 2, so the byte sequence is one byte and its length is 1.
           Pins that the caller's explicit narrowing makes the runtime construction total — no trap, the
           point of requiring the width type.")
  (input  (do (def (main (: n Int64)) (Bytes.len (bin (u8 (UInt8.wrap n))))) (export main)))
  (call   main (: 258 Int64))
  (output (: 1 Int64)))

; A runtime `(bin …)` construction result IS a Bytes value (this file's opening: "expression position
; `(bin …)` CONSTRUCTS a Bytes value"), so it must be `=`-comparable like any Bytes. It builds a FRESH
; owned Bytes on the rope heap — exactly as `Bytes.of` does — so as an operand of the borrowing `=` it is
; owned and reclaimed after the compare. A runtime bin result compared against the Bytes it builds is true;
; against different content, false. The runtime `Bytes.of` control (same content, runtime) and the CONSTANT
; bin control (folds to a comparable Bytes) both already compare — these pin the RUNTIME bin result joins
; them. (The wasm backend runs these; the RUST backend does not yet render a runtime `(bin …)` value at all
; — a broader rust gap — so on rust they decline, exactly as the runtime bin-MATCH cases below do.)

(case "a runtime bin construction result compares equal to the Bytes it builds"
  (doc    "`(= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))` with v=5: the runtime bin builds the one-byte
           Bytes 0x05, equal to `(Bytes.of (list 5))` → true. A runtime `(bin …)` result is a Bytes value
           (this file's opening), so it is `=`-comparable like any Bytes — it builds a fresh owned Bytes on
           the rope heap exactly as `Bytes.of` does. Before, the runtime bin result as a `value-eq` operand
           was not recognized as an owned heap producer, so `=` DECLINED (an aliasing-can't-prove reject,
           not a miscompile) though a runtime `Bytes.of` of the same content compared fine. Pins the runtime
           bin result compares by content.")
  (input  (do (def (main (: v Int64)) (= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))) (export main)))
  (call   main (: 5 Int64))
  (output (: true Bool)))

(case "a runtime bin construction result unequal to different content is false"
  (doc    "The discriminator: the same `(= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))` with v=9 builds
           the byte 0x09 ≠ 0x05 → false. Pins the runtime bin `=` is a genuine content compare, not a
           blanket true — the companion of the equal case.")
  (input  (do (def (main (: v Int64)) (= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))) (export main)))
  (call   main (: 9 Int64))
  (output (: false Bool)))

(case "an explicit Bytes annotation on a runtime bin result compares the same"
  (doc    "The annotated form: `(= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of (list 5)))` with v=5 → true.
           The annotation asserts the runtime bin's Bytes type explicitly; it compares exactly as the
           unannotated case. Pins the gap was never type inference failing to see it as Bytes (the annotation
           confirms Bytes) — the `=` lowering simply had to recognize the runtime bin result as an owned
           Bytes producer, which it now does.")
  (input  (do (def (main (: v Int64)) (= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of (list 5)))) (export main)))
  (call   main (: 5 Int64))
  (output (: true Bool)))

(case "a runtime Bytes.of value is equality-comparable (the runtime-Bytes control)"
  (doc    "The control that always worked: `(= (Bytes.of (list (UInt8.wrap v))) (Bytes.of (list 5)))` with
           v=5 → true. Runtime `Bytes.of` `=` is fine; pins the runtime bin case joins it (the gap was
           specific to the runtime `bin` construction result, not runtime Bytes equality in general).")
  (input  (do (def (main (: v Int64)) (= (Bytes.of (list (UInt8.wrap v))) (Bytes.of (list 5)))) (export main)))
  (call   main (: 5 Int64))
  (output (: true Bool)))

(case "a runtime bin pattern decodes a fixed-width segment from a runtime scrutinee"
  (doc    "A `(bin …)` pattern matches a RUNTIME Bytes scrutinee: `(bin (u16 n))` is built from a runtime
           `UInt16` parameter, then matched back with `(bin (u16 m))`, binding `m = n = 258`. The decode
           reads the scrutinee's bytes at run time (a length probe + a big-endian assemble), round-tripping
           the construction. Pins that construction (which takes the width type) and matching (which binds a
           general integer) are inverse over a runtime value, not just a constant.")
  (input  (do (def (main (: n UInt16)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -1))) (export main)))
  (call   main (: 258 UInt16))
  (output (: 258 Int64)))

(case "a runtime bin match dispatches on a literal tag across arms"
  (doc    "A multi-arm `bin` match over a RUNTIME scrutinee: a leading LITERAL tag segment selects the arm
           (tag 1 vs tag 2), and a runtime `u16` field fills the payload. The construction takes a `UInt8`
           tag and a `UInt16` field (the segments' width types). Built with tag 2, so the second arm fires:
           `y = 300`, `y + 1000 = 1300`. Pins tag-then-field dispatch across arms at run time — the shape a
           tagged binary format's parser takes.")
  (input  (do (def (main (: t UInt8) (: v UInt16))
                (match (bin (u8 t) (u16 v))
                  ((bin (u8 1) (u16 x)) x)
                  ((bin (u8 2) (u16 y)) (+ y 1000))
                  (_ -1)))
              (export main)))
  (call   main (: 2 UInt8) (: 300 UInt16))
  (output (: 1300 Int64)))

(case "a runtime bytes value is spliced into a length-prefixed frame"
  (doc    "`(bin (u16 3) (bytes b))` with `b` a RUNTIME `Bytes` value splices `b` after a two-byte header,
           building the frame at run time (a `bytes-concat` of the emitted header and the runtime body).
           `mk` builds a three-byte body from a runtime `n`; the frame's length is 2 (header) + 3 (body) =
           5. Pins the length-prefixed-frame builder — a fixed header composed with a runtime-length body,
           the construction companion of the dependent-size MATCH.")
  (input  (do (def (frame (: b Bytes)) (Bytes.len (bin (u16 3) (bytes b))))
              (def (mk (: n Int64)) (Bytes.of (list (UInt8.wrap n) 20 30)))
              (def (main (: n Int64)) (frame (mk n)))
              (export main)))
  (call   main (: 7 Int64))
  (output (: 5 Int64)))

(case "a runtime bin match binds the tail after a fixed header via a final rest segment"
  (doc    "A `(bin …)` pattern ending in a FINAL UNSIZED `(bytes rest)` over a RUNTIME scrutinee: a fixed
           one-byte header then a variable-length tail. The length probe accepts any length `>= 1` (the
           header, not an exact width), the header binds via a fixed-offset read, and the tail binds as
           `bytes-slice(scrutinee, 1, len - 1)`. Built from a runtime tag with a three-byte payload, so
           `rest` is those three bytes and `Bytes.len rest = 3`. Pins the header-plus-rest parser shape —
           a tag followed by an opaque remainder — over a runtime value.")
  (input  (do (def (main (: n UInt8))
                (let ((payload (Bytes.of (list 1 2 3))))
                  (match (bin (u8 n) (bytes payload))
                    ((bin (u8 t) (bytes rest)) (Bytes.len rest))
                    (_ -9))))
              (export main)))
  (call   main (: 5 UInt8))
  (output (: 3 Int64)))

(case "a runtime final rest segment binds the empty tail when the scrutinee is only the header"
  (doc    "The final `(bytes rest)` binds an EMPTY tail when the runtime scrutinee is exactly the fixed
           header: `bytes-len >= 1` still holds (the header is present), and the tail slice is `[1, 0)` —
           an empty Bytes, so `Bytes.len rest = 0`. Pins that a rest segment absorbs zero remaining bytes
           without trapping (the degenerate case of the header-plus-rest parser).")
  (input  (do (def (main (: n UInt8))
                (let ((payload (Bytes.of (list))))
                  (match (bin (u8 n) (bytes payload))
                    ((bin (u8 t) (bytes rest)) (Bytes.len rest))
                    (_ -9))))
              (export main)))
  (call   main (: 7 UInt8))
  (output (: 0 Int64)))

(case "a runtime bit-field packs a runtime value into a nibble"
  (doc    "`(bin (bits ((UInt 4).wrap n) 4) (bits 5 4))` with a RUNTIME `n` packs the low nibble of `n`
           into the HIGH nibble and the constant 5 into the low nibble of one byte (most-significant field
           first). A `(bits v 4)` field takes a `(UInt 4)`, so the caller narrows `n` with `(UInt 4).wrap`
           (truncating to the low four bits) BEFORE the segment — the segment then provably fits its value
           and never traps. n=10 → (10<<4)|5 = 0xA5 = 165. Reads byte 0 back. Pins runtime bit-field
           packing — the companion of the constant `(bits 1 1)(bits 2 3)(bits 5 4)` case over a runtime
           value, with the caller responsible for narrowing to the field width.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (bits ((UInt 4).wrap n) 4) (bits 5 4)) 0)
                  ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 10 Int64))
  (output (: 165 Int64)))

(case "a runtime bit-field run spans two bytes and composes with an int segment"
  (doc    "A runtime bit-field RUN that spans a byte boundary and is followed by a byte-aligned int
           segment: `(bits ((UInt 4).wrap n) 4) (bits 1 4) (u8 42)` packs the low nibble of `n` and the
           constant 1 into byte 0 = (n<<4)|1, then writes 42 as byte 1. The 4-bit field takes a `(UInt 4)`
           (the caller narrows `n` with `(UInt 4).wrap`); the trailing `(u8 42)` takes a `UInt8` (a bare
           literal grounds to it). n=3 → byte 1 = 42. Pins that a runtime bit-field run closes to a whole
           byte before the int segment (CDZ0220 byte-alignment) and the int byte follows immediately.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (bits ((UInt 4).wrap n) 4) (bits 1 4) (u8 42)) 1)
                  ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 3 Int64))
  (output (: 42 Int64)))

(case "a decoded field re-encodes into the same-width segment without an explicit narrow"
  (doc    "The decode/encode dual is SYMMETRIC: a `(u16 m)` PATTERN binder decodes a field, and that same
           binder feeds a `(u16 m)` CONSTRUCTION directly — no `UInt16.wrap`/`UInt16.of` needed. The binder
           types as the segment's own width (`UInt16`), which is exactly what the re-encoding segment
           requires, so a parse-then-rebuild round-trip type-checks with no conversion. Here `main` decodes a
           runtime `(bin (u16 n))`, re-encodes the bound field, and reads byte 0 back — 258 = 0x0102 big-
           endian, byte 0 = 1. Pins that the width-typed construction contract does NOT break the natural
           decode→re-encode round-trip a binary transcoder is built from (a decoded field is already the
           right type for its own segment).")
  (input  (do (def (main (: n UInt16))
                (match (bin (u16 n))
                  ((bin (u16 m)) (match (Bytes.at (bin (u16 m)) 0) ((Some b) b) ((None _) -1)))
                  (_ -9)))
              (export main)))
  (call   main (: 258 UInt16))
  (output (: 1 Int64)))

(case "a multi-byte runtime bit-field takes the matching wide unsigned type"
  (doc    "A `(bits v k)` field with `k` wider than a byte requires `v : (UInt k)` — the width type follows
           the field width, not a fixed byte. `(bin (bits n 16))` closes exactly two bytes from a runtime
           `(UInt 16)`; n=258 = 0x0102 big-endian, so byte 0 = 1. Pins that the width-typed contract extends
           to MULTI-BYTE bit-fields (the arbitrary-width `(UInt k)` requirement), not just sub-byte ones.")
  (input  (do (def (main (: n UInt16))
                (match (Bytes.at (bin (bits n 16)) 0) ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 258 UInt16))
  (output (: 1 Int64)))

(case "a wrong-width value in a multi-byte bit-field is a compile-time type error"
  (doc    "The width match is exact for multi-byte bit-fields too: a runtime `Int64` fed to a 16-bit field is
           a COMPILE-TIME type error (CDZ0203, the field wants `UInt16`), the multi-byte companion of the
           sub-byte `bits` case. Pins that a wide bit-field is not a loophole around the width-typed rule.")
  (input  (do (def (main (: n Int64)) (Bytes.len (bin (bits n 16)))) (export main)))
  (error  CDZ0203))

(case "a non-byte-aligned bit-field width takes its exact arbitrary-width unsigned type"
  (doc    "The `(UInt k)` requirement holds for a NON-power-of-two, non-byte-aligned width too: a 12-bit
           field takes a `(UInt 12)`, closed to a whole byte by a trailing 4-bit constant (12+4 = 16 = two
           bytes, CDZ0220 byte-aligned). The caller narrows a runtime `Int64` with `(UInt 12).wrap`
           (truncating to the low 12 bits) — the arbitrary-width analogue of the `(UInt 4).wrap` nibble
           idiom. n=0x123 packs 0x123 into the high 12 bits then a zero nibble → bytes 0x12 0x30; byte 0 =
           0x12 = 18. Pins that an arbitrary bit width has a matching arbitrary-width unsigned type, and that
           `(UInt k).wrap` narrows to it, not a rounding to the nearest byte type.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (bits ((UInt 12).wrap n) 12) (bits 0 4)) 0) ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 291 Int64))
  (output (: 18 Int64)))

(case "a decoded length field re-encodes a length-prefixed frame end to end"
  (doc    "The full length-prefixed-frame TRANSCODER round-trip under the width-typed rule: a `(bin (u8 n)
           (bytes body n) (bytes rest))` pattern decodes a `u8` length `n`, binds exactly `n` body bytes
           (the dependent size), and the tail; then it RE-ENCODES `(bin (u8 n) (bytes body))` — the SAME
           decoded `n` serves as both the `(bytes body n)` dependent SIZE and the `(u8 n)` re-encode value,
           with NO explicit narrow (the decoded field already types as the header's `UInt8`). Built at run
           time from a `(list 2 10 20 99)` scrutinee (length 2, body [10 20], rest [99]); the re-encoded
           frame is `[2, 10, 20]` and its length is 3. Pins that a decoded length round-trips through both a
           dependent-size bind and a same-width re-encode — the core move a binary reframer/transcoder is
           built from — without breaking under the width-typed contract.")
  (input  (do (def (reframe (: b Bytes))
                (match b
                  ((bin (u8 n) (bytes body n) (bytes rest)) (bin (u8 n) (bytes body)))
                  (_ (bin))))
              (def (main (: k Int64)) (Bytes.len (reframe (Bytes.of (list 2 10 20 99)))))
              (export main)))
  (call   main (: 0 Int64))
  (output (: 3 Int64)))

(case "a runtime signed little-endian segment round-trips a negative value"
  (doc    "The three orthogonal axes combined over a RUNTIME value: SIGNED (two's complement) + LITTLE-ENDIAN
           (byte reversal) + the width-typed contract. `(i16 n le)` takes a runtime `Int16` n = -2 = 0xFFFE;
           little-endian lays it LSB-first as bytes [0xFE, 0xFF], and matching `(i16 m le)` reassembles the
           two's-complement value back to -2. Pins that sign-extension and endianness compose correctly over
           a runtime construct→match round-trip (a constant `(i16 -1)`/`le u16` cover each axis singly; this
           is the runtime intersection with a NEGATIVE multi-byte value).")
  (input  (do (def (main (: n Int16)) (match (bin (i16 n le)) ((bin (i16 m le)) m) (_ 0))) (export main)))
  (call   main (: -2 Int16))
  (output (: -2 Int64)))

(case "a runtime signed little-endian segment lays the low byte first"
  (doc    "The byte-order half of the signed-le round-trip: `(bin (i16 n le))` with a runtime `Int16` n = -2
           = 0xFFFE emits the LEAST-significant byte first, so byte 0 = 0xFE = 254 (not 0xFF). Reads it back
           with `Bytes.at`. Pins that `le` reverses a SIGNED segment's bytes the same way it does an unsigned
           one — the two's-complement bit pattern is laid low-byte-first, so a reader that ignored `le` on a
           signed segment (emitting big-endian 0xFF) would differ here.")
  (input  (do (def (main (: n Int16))
                (match (Bytes.at (bin (i16 n le)) 0) ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: -2 Int16))
  (output (: 254 Int64)))
