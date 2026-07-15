; Binary matching and construction — witnesses the binary-syntax decision
; (options/binary-syntax/). One `bin` form serves two directions, reusing the constructor/pattern
; duality the language already has (a variant `(Some 5)` builds and `(Some n)` destructures): in
; expression position `(bin <segment>...)` CONSTRUCTS a Bytes value; in pattern position it
; DESTRUCTURES a Bytes scrutinee inside an ordinary `match`. No new control construct — a binary pattern
; is a pattern like any other, and the `bin` head constrains the scrutinee to Bytes exactly as `(Some n)`
; constrains it to a sum.
;
; A `bin` is a sequence of segments, each written like a constructor `(<kind> <slot> <modifier>...)`:
;   (u8 v) (u16 v) (u32 v) (u64 v)   unsigned N-bit integer, BIG-ENDIAN by default
;   (i8 v) (i16 v) (i32 v) (i64 v)   signed N-bit integer (two's complement)
;   (uNN v le)                        the `le` modifier selects little-endian byte order
;   (bits v k)                        the low k bits of v; k is a COMPILE-TIME CONSTANT
;   (bytes b)                         splice all of b (build); bind the REST (match, final segment only)
;   (bytes b n)                       exactly n bytes; n MAY be a name bound by an earlier segment
;                                       (dependent size) — the crown jewel, entirely value-level
; A literal in the slot means match-by-equality (magic numbers, opcodes) — the direct analogue of the
; existing literal patterns `(match 2 (2 "two") …)`.
;
; Byte-alignment is STATIC: the whole `bin` must be byte-aligned. `bits` widths are compile-time
; constants so their running sum is checkable at compile time; a `bin` whose bits do not close a byte, a
; non-final unsized `bytes`, or a `bits` width that is not a constant is an ILL-FORMED BINARY FORM,
; rejected CDZ0220 (options/diagnostics-schema/, the CDZ02xx types-and-patterns band). At RUN time, a
; value that does not fit its segment (a u8 given 256, a u8 given -1, a `bits k` value that needs more
; than k bits) has no defined encoding, so construction TRAPS with reason "binary value does not fit
; segment" (core-semantics.md #Partial Operations Have A Defined Outcome), the companion of the Bytes
; out-of-range trap. Exhaustiveness is the existing rule: a `bin` pattern never covers every byte
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
  (doc    "`(bin (u16 (Bytes.len payload)) (bytes payload))` writes payload's length as a big-endian
           u16 prefix, then splices payload — the length-framing idiom that replaces hand-rolled
           `(Bytes.concat (Bytes.of (list (& (>> len 8) 255) (& len 255))) payload)`. Pins that a computed
           value feeds a size segment and an unsized `(bytes …)` splices a whole Bytes value.")
  (input  (= (bin (u16 (Bytes.len (Bytes.of (list 10 20 30)))) (bytes (Bytes.of (list 10 20 30))))
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
; Fit — a value that does not fit its segment has no encoding (rejected when provable, else traps)
; ============================================================================================

(case "constructing a u8 segment from a value above its range is rejected"
  (doc    "`(bin (u8 256))` asks an 8-bit unsigned segment to hold 256, which needs nine bits and has no
           8-bit encoding — it does NOT truncate to 0. With a CONSTANT operand the overflow is provable, so
           it FAILS THE BUILD (CDZ0304) — the compile-provable-trap rule (reference-compiler.md #A
           Compile-Provable Trap Fails The Build); a runtime value out of range traps \"binary value does
           not fit segment\" at that point. The companion of the Bytes out-of-range check, at the segment
           boundary.")
  (input  (bin (u8 256)))
  (error  CDZ0304))

(case "constructing an unsigned segment from a negative value is rejected"
  (doc    "`(bin (u8 -1))` gives a negative value to an UNSIGNED segment, which has no negative encoding —
           it does NOT wrap to 255 (that is the meaning of the SIGNED `(i8 -1)` case above). With a CONSTANT
           operand the out-of-range value is provable, so it FAILS THE BUILD (CDZ0304); a runtime negative
           traps \"binary value does not fit segment\". Pins that unsigned and signed segments differ on a
           negative value: the signed one encodes it in two's complement, the unsigned one has no encoding.")
  (input  (bin (u8 -1)))
  (error  CDZ0304))

(case "constructing a bit-field from a value wider than its width is rejected"
  (doc    "`(bin (bits 2 1))` gives the value 2 (which needs two bits) to a 1-bit field, so it does not fit.
           With a CONSTANT operand the misfit is provable at compile time, so the ill-formed bit-field is
           rejected (CDZ0220 — the binary well-formedness code); a runtime value wider than the field traps
           \"binary value does not fit segment\". Pins that a bit-field's value is range-checked against its
           width, the sub-byte companion of the u8-overflow check.")
  (input  (bin (bits 2 1)))
  (error  CDZ0220))

; ============================================================================================
; Runtime segments — a `bin` whose segment value / scrutinee is a RUNTIME value (a def parameter,
; not a compile-time constant). Construction builds on the byte heap and range-checks at run time;
; matching decodes the scrutinee's bytes at run time. Each threads its value through `main`'s
; parameter (via `(call …)`) so the `bin` cannot fold to a constant.
; ============================================================================================

(case "a runtime value is constructed into a fixed-width segment and its length read"
  (doc    "`(bin (u16 n))` with `n` a RUNTIME parameter (not a constant) builds a two-byte sequence on the
           byte heap at run time — the construction does not fold. Reading its length back yields 2, the
           segment's width. Pins that a `bin` construction whose value is only known at run time still
           produces a well-formed Bytes.")
  (input  (do (def (main (: n Int64)) (Bytes.len (bin (u16 n)))) (export main)))
  (call   main (: 258 Int64))
  (output (: 2 Int64)))

(case "a runtime fixed-width segment is emitted big-endian and read back by index"
  (doc    "`(bin (u16 258))` built at run time encodes 258 = 0x0102 BIG-ENDIAN, so byte 0 is the
           most-significant `0x01` = 1. Reads it back with `Bytes.at`. Pins that a runtime construction
           lays its bytes most-significant-first, the same order the constant fold and the pattern decode
           agree on.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (u16 n)) 0) ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 258 Int64))
  (output (: 1 Int64)))

(case "constructing a runtime value that does not fit its segment traps"
  (doc    "`(bin (u8 n))` with a RUNTIME `n = 256` has no 8-bit encoding, so construction TRAPS with reason
           \"binary value does not fit segment\" rather than truncating to 0 — the runtime companion of the
           constant out-of-range rejection (which fails the build). Pins that the segment range-check is
           enforced at run time for a value not known at compile time.")
  (input  (do (def (main (: n Int64)) (Bytes.len (bin (u8 n)))) (export main)))
  (call   main (: 256 Int64))
  (trap   "binary value does not fit segment"))

(case "a runtime bin pattern decodes a fixed-width segment from a runtime scrutinee"
  (doc    "A `(bin …)` pattern matches a RUNTIME Bytes scrutinee: `(bin (u16 n))` is built from a runtime
           parameter, then matched back with `(bin (u16 m))`, binding `m = n = 258`. The decode reads the
           scrutinee's bytes at run time (a length probe + a big-endian assemble), round-tripping the
           construction. Pins that construction and matching are inverse over a runtime value, not just a
           constant.")
  (input  (do (def (main (: n Int64)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -1))) (export main)))
  (call   main (: 258 Int64))
  (output (: 258 Int64)))

(case "a runtime bin match dispatches on a literal tag across arms"
  (doc    "A multi-arm `bin` match over a RUNTIME scrutinee: a leading LITERAL tag segment selects the arm
           (tag 1 vs tag 2), and a runtime `u16` field fills the payload. Built with tag 2, so the second
           arm fires: `y = 300`, `y + 1000 = 1300`. Pins tag-then-field dispatch across arms at run time —
           the shape a tagged binary format's parser takes.")
  (input  (do (def (main (: t Int64) (: v Int64))
                (match (bin (u8 t) (u16 v))
                  ((bin (u8 1) (u16 x)) x)
                  ((bin (u8 2) (u16 y)) (+ y 1000))
                  (_ -1)))
              (export main)))
  (call   main (: 2 Int64) (: 300 Int64))
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
  (input  (do (def (main (: n Int64))
                (let ((payload (Bytes.of (list 1 2 3))))
                  (match (bin (u8 n) (bytes payload))
                    ((bin (u8 t) (bytes rest)) (Bytes.len rest))
                    (_ -9))))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 3 Int64)))

(case "a runtime final rest segment binds the empty tail when the scrutinee is only the header"
  (doc    "The final `(bytes rest)` binds an EMPTY tail when the runtime scrutinee is exactly the fixed
           header: `bytes-len >= 1` still holds (the header is present), and the tail slice is `[1, 0)` —
           an empty Bytes, so `Bytes.len rest = 0`. Pins that a rest segment absorbs zero remaining bytes
           without trapping (the degenerate case of the header-plus-rest parser).")
  (input  (do (def (main (: n Int64))
                (let ((payload (Bytes.of (list))))
                  (match (bin (u8 n) (bytes payload))
                    ((bin (u8 t) (bytes rest)) (Bytes.len rest))
                    (_ -9))))
              (export main)))
  (call   main (: 7 Int64))
  (output (: 0 Int64)))

(case "a runtime bit-field packs a runtime value into a nibble"
  (doc    "`(bin (bits (UInt8.wrap n) 4) (bits 5 4))` with a RUNTIME `n` packs the low nibble of `n` into
           the HIGH nibble and the constant 5 into the low nibble of one byte (most-significant field
           first). n=10 → (10<<4)|5 = 0xA5 = 165. Reads byte 0 back. Pins runtime bit-field packing — the
           companion of the constant `(bits 1 1)(bits 2 3)(bits 5 4)` case over a runtime value.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (bits (UInt8.wrap n) 4) (bits 5 4)) 0)
                  ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 10 Int64))
  (output (: 165 Int64)))

(case "a runtime bit-field run spans two bytes and composes with an int segment"
  (doc    "A runtime bit-field RUN that spans a byte boundary and is followed by a byte-aligned int
           segment: `(bits (UInt8.wrap n) 4) (bits 1 4) (u8 42)` packs the low nibble of `n` and the
           constant 1 into byte 0 = (n<<4)|1, then writes 42 as byte 1. n=3 → byte 1 = 42. Pins that a
           runtime bit-field run closes to a whole byte before the int segment (CDZ0220 byte-alignment)
           and the int byte follows immediately.")
  (input  (do (def (main (: n Int64))
                (match (Bytes.at (bin (bits (UInt8.wrap n) 4) (bits 1 4) (u8 42)) 1)
                  ((Some b) b) ((None _) -1)))
              (export main)))
  (call   main (: 3 Int64))
  (output (: 42 Int64)))
