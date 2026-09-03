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
; A fixed-width segment REQUIRES its exact width type in BOTH axes (width AND sign): `(u8 v)` takes UInt8,
; `(u16 v)` UInt16, `(i8 v)` Int8, `(bits v k)` `(UInt k)`. A value whose concrete integer type differs —
; wider, narrower, or differently-signed (a runtime `Int64` param most commonly) — is a COMPILE-TIME type
; error CDZ0203 ("segment takes …"), never a runtime trap: construction is total, and narrowing is the
; caller's explicit job. The suggested conversion names a spelling that RESOLVES — an aliased width
; ({8,16,32,64}) suggests the bound `UInt8.wrap`/`UInt8.of`; a non-aliased bit width `(UInt 4)` (no bound
; module) suggests the member form `(. (UInt 4) wrap)`, never the unbound `UInt4.wrap`. (migrated from rcdzc
; a_bin_segment_requires_its_width_typed_value_cdz0203.)
(diagnostic-quality)

(case
  "an Int64 value into a u8 segment is a width type error naming the aliased wrap+of conversions"
  (input (do (def (main (: n Int64)) (Bytes.len (bin (u8 n)))) (export main)))
  (error
    CDZ0203
    (message "segment takes")
    (message "UInt8")
    (message "UInt8.wrap to truncate")
    (message "UInt8.of to check")))

(case
  "an Int8 value into an unsigned u8 segment is a signedness mismatch"
  (input (do (def (main (: n Int8)) (Bytes.len (bin (u8 n)))) (export main)))
  (error CDZ0203 (message "segment takes") (message "UInt8")))

(case
  "a narrower UInt8 into a u16 segment is not silently widened — a width type error"
  (input (do (def (main (: n UInt8)) (Bytes.len (bin (u16 n)))) (export main)))
  (error CDZ0203 (message "UInt16")))

(case
  "a wider signed Int16 into a signed i8 segment is a width type error"
  (input (do (def (main (: n Int16)) (Bytes.len (bin (i8 n)))) (export main)))
  (error CDZ0203 (message "Int8")))

(case
  "an Int64 into a non-aliased bits field names the member-form wrap conversion"
  (input (do (def (main (: n Int64)) (Bytes.len (bin (bits n 4) (bits 5 4)))) (export main)))
  (error CDZ0203 (message "UInt4") (message "(. (UInt 4) wrap) to truncate")))

; Beyond WIDTH (CDZ0203 above), a segment's value must match its KIND: an int/bits segment takes an integer,
; a utf8 segment a String. A kind mismatch — `(bin (u8 "x"))` (a String into an int segment), `(bin (utf8 n
; 3))` (an integer into a utf8 segment) — was accepted at build (the segment lowering never type-checked its
; slot) and emitted garbage bytes. Now CDZ0220 names the required KIND + the offending type. (migrated from
; rcdzc a_bin_segment_value_must_match_its_kind; no-false-positive is the valid construction cases below.)
(case
  "a String value in an integer bin segment is a kind mismatch"
  (input (do (def (main) (bin (u8 "x"))) (export main)))
  (error CDZ0220 (message "integer segment takes an integer") (message "String")))

(case
  "an integer value in a utf8 bin segment is a kind mismatch"
  (input (do (def (g (: n Int64)) (bin (utf8 n 3))) (export g)))
  (error CDZ0220 (message "utf8 segment takes a String")))

; A CONSTANT literal segment value that does not fit its width grounds to the segment's width type and
; range-checks: it has no encoding, so the build FAILS (CDZ0304) rather than truncating (256/-1 into u8,
; 256 into an 8-bit field, etc). The message is ACTIONABLE — it names the offending VALUE, the segment's
; width TYPE, and the VALID RANGE, mirroring an annotation-position `(: 300 UInt8)` over-range. (A
; NON-literal value that does not fit is a type error, CDZ0203, not this provable fit-trap.) (migrated from
; rcdzc a_bin_value_out_of_range_for_its_segment_is_a_provable_trap.)
(case
  "a constant value out of range for its unsigned bin segment is a provable trap naming value, type, range"
  (input (do (def (main) (bin (u8 300))) (export main)))
  (error CDZ0304 (message "300") (message "UInt8") (message "0..=255")))

(case
  "a constant value out of range for a non-aliased bit-field bin segment names the (UInt k) type and range"
  (input (do (def (main) (bin (bits 20 4) (bits 0 4))) (export main)))
  (error CDZ0304 (message "20") (message "(UInt 4)") (message "0..=15")))

(case
  "a constant value out of range for a signed bin segment names the signed type and negative-inclusive range"
  (input (do (def (main) (bin (i8 200))) (export main)))
  (error CDZ0304 (message "Int8") (message "-128..=127")))

; A fixed-width integer bin segment is a BYTE-ALIGNED width — u8/u16/u32/u64 or i8/i16/i32/i64. A uNN/iNN
; head with any OTHER width (u24, u7, u128, i0) is rejected (CDZ0201) naming the supported widths AND
; pointing a non-byte-aligned width at the `(bits v k)` segment (not the generic "unrecognized kind" which
; told the author to write what they wrote). A CONFIDENT near-miss (u166→u16, same signedness) carries a
; rename fix; a width too far (u128) keeps the guidance but no misleading rename. A genuinely unrecognized
; head (frob, u, i) keeps the generic message. (migrated from rcdzc
; a_non_byte_aligned_int_bin_segment_names_the_supported_widths.)
(case
  "a non-byte-aligned integer bin segment width names the supported widths and the bits alternative"
  (input (do (def (main) (bin (u24 1))) (export main)))
  (error CDZ0201 (message "u8/u16/u32/u64") (message "(bits v k)") (message "u24")))

(case
  "a genuinely unrecognized bin segment kind keeps the generic message, not the width guidance"
  (input (do (def (main) (bin (frob 1))) (export main)))
  (error CDZ0201 (message "unrecognized bin segment kind")))

(case
  "a near-miss integer bin segment width carries a rename fix to the byte-aligned width"
  (input (do (def (main) (bin (u166 1))) (export main)))
  (error CDZ0201 (fix (kind replace) (replacement "u16") (unverified))))

(case
  "a bin segment width too far from a byte-aligned kind keeps the bits guidance but carries no rename fix"
  (input (do (def (main) (bin (u128 1))) (export main)))
  (error CDZ0201 (message "u8/u16/u32/u64") (message "(bits v k)") (no-fix)))

(case
  "a u16 segment encodes an integer big-endian by default"
  (doc
    "`(bin (u16 258))` encodes 258 (0x0102) as two bytes, most-significant first — big-endian is
           the default byte order, so the result is `(Bytes.of (list 1 2))`. Pins the default-endianness
           construction the wasm/network-order idiom depends on.")
  (input (= (bin (u16 258)) (Bytes.of #list(1 2))))
  (output (: true Bool)))

(case
  "the le modifier encodes a u16 little-endian"
  (doc
    "`(bin (u16 258 le))` selects little-endian with the `le` modifier, so 0x0102 is emitted
           least-significant byte first: `(Bytes.of (list 2 1))`. Pins that byte order is explicit and
           the modifier reverses the default, never an implicit host-endianness choice.")
  (input (= (bin (u16 258 le)) (Bytes.of #list(2 1))))
  (output (: true Bool)))

(case
  "a u32 segment encodes four big-endian bytes"
  (doc
    "`(bin (u32 0x89504E47))` encodes the magic number as four big-endian bytes
           `(Bytes.of (list 137 80 78 71))` — the fixed-width, whole-byte encoding a magic-number header
           is built from. Written as a hex literal (01-literals.sexp), the value reads as its bytes at a
           glance. Pins u32 width and byte order together.")
  (input (= (bin (u32 0x89504e47)) (Bytes.of #list(137 80 78 71))))
  (output (: true Bool)))

(case
  "a signed i8 segment encodes as two's complement"
  (doc
    "`(bin (i8 -1))` encodes -1 in a signed 8-bit segment as the two's-complement byte 255. Pins
           that a SIGNED segment admits a negative value (unlike an unsigned segment, which traps on one —
           see below), encoding it in two's complement.")
  (input (= (bin (i8 -1)) (Bytes.of #list(255))))
  (output (: true Bool)))

(case
  "a u64 segment encodes eight big-endian bytes"
  (doc
    "`(bin (u64 258))` encodes 258 as eight big-endian bytes — six leading zeros then 0x0102 —
           `(Bytes.of (list 0 0 0 0 0 0 1 2))`. Pins the widest fixed-width segment and that its width is
           eight bytes regardless of how small the value is, the companion of the u16 and u32 cases.")
  (input (= (bin (u64 258)) (Bytes.of #list(0 0 0 0 0 0 1 2))))
  (output (: true Bool)))

(case
  "a multi-segment bin concatenates mixed-width signed and unsigned segments in order"
  (doc
    "A `bin` with several integer segments of different widths and signedness lays them out
           left-to-right, each encoded independently. `(bin (u8 1) (u16 258) (i8 -1))` produces the u8 byte
           1, then the big-endian u16 258 = 0x0102 = two bytes 1 2, then the signed i8 −1 = two's-complement
           255 — the four bytes `(Bytes.of (list 1 1 2 255))`. Pins that segment widths, endianness, and
           signedness are applied per-segment and the results concatenated in source order (a builder that
           mis-ordered the segments, dropped a width, or sign-mishandled the i8 would differ), the
           integration of the single-segment u8/u16/i8 cases above into one bin.")
  (input (= (bin (u8 1) (u16 258) (i8 -1)) (Bytes.of #list(1 1 2 255))))
  (output (: true Bool)))

(case
  "bit-field segments pack sub-byte values into one byte"
  (doc
    "`(bin (bits 1 1) (bits 2 3) (bits 5 4))` packs a 1-bit flag (1), a 3-bit tag (2 = 0b010), and a
           4-bit value (5 = 0b0101), most-significant field first: 1·010·0101 = 0b1010_0101 = 165. The
           three widths sum to 8, so the `bin` closes exactly one byte. The expected byte is written as a
           binary literal `0b1010_0101` so the packed bit-fields are legible. Pins sub-byte bit-field
           packing and the most-significant-field-first order.")
  (input (= (bin (bits 1 1) (bits 2 3) (bits 5 4)) (Bytes.of #list(0b10100101))))
  (output (: true Bool)))

(case
  "a bit-field wider than a byte packs across the byte boundary big-endian"
  (doc
    "A `(bits v k)` segment may be WIDER than 8 bits, spanning multiple bytes: `(bin (bits 258 16))`
           packs 258 = 0x0102 into a 16-bit field, closing two bytes big-endian = `(Bytes.of (list 1 2))`.
           And bit-fields whose widths sum across a byte boundary pack contiguously: `(bin (bits 1 4) (bits
           0 8) (bits 0 4))` lays a nibble 1, then 8 zero bits, then a zero nibble — 16 bits = two bytes
           `(Bytes.of (list 16 0))` (the leading nibble 1 is the high 4 bits of byte 0 = 0x10). Pins that
           bit-field packing crosses byte boundaries most-significant-bit-first, not only sub-byte fields
           that close a single byte.")
  (input (= (bin (bits 258 16)) (Bytes.of #list(1 2))))
  (output (: true Bool)))

(case
  "a 16-bit bit-field packs a two-nibble-distinct value across the byte boundary big-endian"
  (doc
    "The byte-boundary bit-field pack where both bytes carry non-trivial content: `(bin (bits 4660
           16))` packs 4660 = 0x1234 into a 16-bit field, closing two bytes big-endian = `(Bytes.of (list
           18 52))` (0x12 = 18, 0x34 = 52). Distinct from the 258 = 0x0102 case above, whose high byte is
           1; here both bytes differ, pinning that the split is a genuine most-significant-first byte
           division of the field value, not a low-byte-only write.")
  (input (= (bin (bits 4660 16)) (Bytes.of #list(18 52))))
  (output (: true Bool)))

(case
  "a byte-closing bit-field composes with a following byte-aligned integer segment"
  (doc
    "A `(bits v k)` run that closes on a byte boundary composes with a subsequent `uNN` integer
           segment, each encoded independently and concatenated: `(bin (bits 255 8) (u8 1))` writes the
           8-bit field 255 as one byte, then the u8 1 as the next = `(Bytes.of (list 255 1))`. Pins that a
           byte-aligned bit-field and an integer segment coexist in one construction (the sub-byte and
           fixed-width segment kinds interleave once the bit-field run is byte-aligned).")
  (input (= (bin (bits 255 8) (u8 1)) (Bytes.of #list(255 1))))
  (output (: true Bool)))

(case
  "a bit-field value that needs more bits than its width does not fit its segment"
  (doc
    "A `(bits v k)` segment holds only the low `k` bits, so a value needing more than `k` bits has no
           defined encoding — construction is rejected (`binary value does not fit segment`, CDZ0304, the
           value-overflow companion of a u8 given 256). `(bin (bits 16 4) (bits 0 4))` gives 16, which needs
           5 bits, to a 4-bit field — rejected. Pins that a bit-field range-checks its value against its
           declared width, not silently truncating it to the low bits (which would encode 0 and lose data).")
  (input (bin (bits 16 4) (bits 0 4)))
  (error CDZ0304))

; STRUCTURAL well-formedness (distinct from the CDZ0304 value-fit above): a `bin` must be byte-aligned, a
; non-final `(bytes …)` (unsized) is ill-formed, and a `bits` width must be a compile-time constant natural —
; each an ILL-FORMED BINARY FORM rejected CDZ0220 (the intro's rule). Migrated from rcdzc
; an_ill_formed_bin_form_is_rejected_cdz0220.
(case
  "a bin whose bit-fields do not close to a whole byte is ill-formed"
  (doc
    "`(bin (bits 1 1) (bits 0 3))` = 4 bits, not byte-aligned → CDZ0220; the message names the concrete
           bit total and the padding to the next byte (rustc-gold): 4 bits, 4 short of 1 byte.")
  (input (do (def (main) (bin (bits 1 1) (bits 0 3))) (export main)))
  (error CDZ0220 (message "total 4 bits") (message "add 4 more bits to reach 1 byte")))

(case
  "a non-final unsized bytes segment is an ill-formed binary form"
  (input (do (def (main) (bin (bytes (Bytes.of #list(1))) (u8 2))) (export main)))
  (error CDZ0220))

(case
  "a bit-field with a negative width is ill-formed (width must be a constant natural)"
  (input (do (def (g (: v Int64)) (bin (bits v -1))) (export g)))
  (error CDZ0220 (message "bit-field width must be a compile-time constant natural")))

(case
  "a bit-field with a non-constant width is ill-formed (width must be a constant natural)"
  (input (do (def (g (: v Int64)) (bin (bits v abc))) (export g)))
  (error CDZ0220 (message "bit-field width must be a compile-time constant natural")))

(case
  "a length-prefixed frame is built from a size segment and a bytes segment"
  (doc
    "`(bin (u16 (UInt16.of (Bytes.len payload))) (bytes payload))` writes payload's length as a
           big-endian u16 prefix, then splices payload — the length-framing idiom that replaces hand-rolled
           `(Bytes.concat (Bytes.of (list (& (>> len 8) 255) (& len 255))) payload)`. The `u16` segment takes
           a `UInt16`, so the caller narrows the `Int64` length with `UInt16.of` (a CHECKED narrow — a
           payload too long to frame in 16 bits is a real error, not a silent truncation); the length 3 fits.
           Pins that a computed value narrowed to the segment width feeds a size segment and an unsized
           `(bytes …)` splices a whole Bytes value.")
  (input
    (=
      (bin
        (u16 (UInt16.of (Bytes.len (Bytes.of #list(10 20 30)))))
        (bytes (Bytes.of #list(10 20 30))))
      (Bytes.of #list(0 3 10 20 30))))
  (output (: true Bool)))

(case
  "an empty binary form is the empty byte sequence"
  (doc
    "`(bin)` with no segments is the zero-length Bytes value, equal to `(Bytes.of (list))`. Pins the
           degenerate construction — the identity a fold over segments starts from.")
  (input (= (bin) (Bytes.of #list())))
  (output (: true Bool)))

(case
  "the length of a fixed-width construction is the sum of its segment widths"
  (doc
    "`(Bytes.len (bin (u32 0)))` = 4: a u32 segment is four bytes wide regardless of the value it
           carries. Pins that fixed-width segments contribute their width, not a value-dependent length.")
  (input (Bytes.len (bin (u32 0))))
  (output (: 4 Int64)))

; ============================================================================================
; Matching — `(bin …)` in pattern position destructures a Bytes scrutinee
; ============================================================================================
(case
  "a bin pattern binds an integer read from a fixed-width segment"
  (doc
    "Matching `(bin (u16 258))` against the pattern `(bin (u16 n))` reads the big-endian u16 back
           into n, round-tripping the construction: n = 258. Pins that construction and matching are
           inverse over the same segment grammar.")
  (input (match (bin (u16 258)) ((bin (u16 n)) n) (_ 0)))
  (output (: 258 Int64)))

; A `(bin …)` pattern decodes a Bytes value, so the `bin` head constrains the scrutinee to Bytes (intro
; above). Over a DEFINITE non-Bytes scrutinee (Int64/String/List) it is a type error CDZ0203 naming the
; `(bin …)`-matches-a-Bytes rule + the real scrutinee type — the bin twin of the map-key / structural-pattern
; scrutinee-kind checks (not the misleading generic "not a scalar literal or `_`" decline it once gave).
; Migrated from rcdzc a_bin_pattern_over_a_non_bytes_scrutinee_is_a_type_error.
(case
  "a bin pattern over an Int64 scrutinee is a type error"
  (input (do (def (f (: n Int64)) (match n ((bin (u8 x)) x) (_ 0))) (export f)))
  (error CDZ0203 (message "`(bin …)` pattern matches a Bytes value") (message "Int64")))

(case
  "a bin pattern over a String scrutinee is a type error"
  (input (do (def (f (: s String)) (match s ((bin (u8 x)) x) (_ 0))) (export f)))
  (error CDZ0203 (message "`(bin …)` pattern matches a Bytes value") (message "String")))

(case
  "a bin pattern over a List scrutinee is a type error"
  (input (do (def (f (: xs (List Int64))) (match xs ((bin (u8 x)) x) (_ 0))) (export f)))
  (error CDZ0203 (message "`(bin …)` pattern matches a Bytes value")))

(case
  "a bin pattern over a Bytes scrutinee is NOT a type error (declines only on the non-scalar param boundary)"
  (doc
    "The no-false-reject control: a `(bin …)` pattern over a genuine Bytes scrutinee is well-typed — no
           CDZ0203. Here `b : Bytes` is an EXPORTED parameter, and a non-scalar entry parameter has no scalar
           boundary representation yet, so the program DECLINES at emit (not a type reject) — confirming the
           bin/Bytes match itself is accepted, distinct from the wrong-kind rejects above.")
  (input (do (def (f (: b Bytes)) (match b ((bin (u8 x)) x) (_ 0))) (export f)))
  (call f (: b"\x2a" Bytes))
  (output (: 42 Int64)))

(case
  "a le pattern segment reads a fixed-width integer little-endian"
  (doc
    "The construction `(bin (u16 258 le))` emitted `(list 2 1)`; matching those same bytes against
           `(bin (u16 n le))` reads them back least-significant byte first, recovering n = 258. Pins that
           the `le` modifier is honored in pattern position exactly as in expression position — matching is
           the inverse of construction over the same modifier, not big-endian-only on the way in.")
  (input (match (bin (u16 258 le)) ((bin (u16 n le)) n) (_ 0)))
  (output (: 258 Int64)))

(case
  "a signed pattern segment reads a two's-complement integer as negative"
  (doc
    "The byte 255 read through a SIGNED `(i8 n)` pattern is -1, not 255 — a signed segment
           interprets its two's-complement bits as a signed integer, the inverse of the `(i8 -1)`
           construction that emitted 255. Pins that signedness governs matching too: the same byte reads
           back as -1 under `i8` and as 255 under `u8` (next case).")
  (input (match (Bytes.of #list(255)) ((bin (i8 n)) n) (_ 0)))
  (output (: -1 Int64)))

(case
  "an unsigned pattern segment reads the same byte as a non-negative integer"
  (doc
    "The companion of the signed-read case: the byte 255 read through an UNSIGNED `(u8 n)` pattern is
           255. The one byte reads back as -1 under `i8` and 255 under `u8`, so signedness is a property of
           the segment, not the bytes. Pins that the two readings of one byte differ precisely by the
           segment's sign, mirroring the construction-side split between `(i8 -1)` and `(u8 -1)`.")
  (input (match (Bytes.of #list(255)) ((bin (u8 n)) n) (_ 0)))
  (output (: 255 Int64)))

(case
  "a signed segment reads the sign-bit-only byte as the minimum, not its magnitude"
  (doc
    "The 0x80 boundary, sharper than the 0xFF cases above (which read the trivial all-ones byte as
           -1): the byte 128 (`0b1000_0000`, only the sign bit set) read through a SIGNED `(i8 n)` pattern
           is -128 — the Int8 MINIMUM, its two's-complement value — NOT 128 (its magnitude) and NOT -0. This
           is where a naive reinterpretation that masks the sign bit and negates the remaining magnitude
           would wrongly give 0 or -0; the correct reading is the two's-complement -128. Pins the
           sign-bit-only extreme of the signed segment read.")
  (input (match (Bytes.of #list(128)) ((bin (i8 n)) n) (_ 0)))
  (output (: -128 Int64)))

(case
  "an unsigned segment reads the same sign-bit-only byte as 128"
  (doc
    "The unsigned companion: the byte 128 read through `(u8 n)` is 128 — the same byte reads back as
           -128 under `i8` and 128 under `u8`, differing by exactly the segment's sign at the sign-bit
           boundary (as the 0xFF pair does at the all-ones byte). Pins that the two readings of 0x80 split
           by segment sign, so signedness is a property of the segment across the whole byte range, not
           just the all-ones case.")
  (input (match (Bytes.of #list(128)) ((bin (u8 n)) n) (_ 0)))
  (output (: 128 Int64)))

(case
  "constructing a signed segment at the minimum emits the sign-bit-only byte"
  (doc
    "The construction inverse of the signed 0x80 read: `(bin (i8 -128))` encodes the Int8 minimum in
           two's complement as the byte 0x80 = 128, so it equals `(Bytes.of (list 128))`. With the read
           case above this pins the round-trip at the signed minimum — `-128` emits 0x80, and 0x80 read as
           `i8` is `-128` — the extreme companion of the `(i8 -1)` ⇆ 255 round-trip.")
  (input (= (bin (i8 -128)) (Bytes.of #list(128))))
  (output (: true Bool)))

; The round-trip cases above use mid-range values (258) and the i8 extremes (-1, -128). These pin the
; MULTI-BYTE-WIDTH extremes — where an off-by-one in the shift/mask byte-assembly or a sign-extension slip
; would surface: u16 at its max (65535 = every bit set across two bytes), u32 at its max (four bytes all
; 0xFF), and i16 at its two's-complement minimum (-32768 = 0x8000). Each constructs then matches back to the
; same value, both backends.
(case
  "a u16 segment round-trips at its maximum value"
  (doc
    "`(bin (u16 65535))` — the u16 maximum, every bit set across two big-endian bytes (0xFF 0xFF) —
           matched by `(bin (u16 n))` reads back 65535. Pins the round-trip at the u16 ceiling: a byte-
           assembly that dropped or mis-shifted the high byte would read a smaller value. The extreme
           companion of the mid-range `(u16 258)` round-trip.")
  (input (match (bin (u16 65535)) ((bin (u16 n)) n) (_ -1)))
  (output (: 65535 Int64)))

(case
  "a u32 segment round-trips at its maximum value"
  (doc
    "`(bin (u32 4294967295))` — the u32 maximum, four big-endian bytes all 0xFF — matched by `(bin
           (u32 n))` reads back 4294967295. Pins the four-byte big-endian assembly is exact at the ceiling
           (a value past u16 range, so all four bytes carry significant bits); an off-by-one in the 8/16/24-
           bit shifts would corrupt it.")
  (input (match (bin (u32 4294967295)) ((bin (u32 n)) n) (_ -1)))
  (output (: 4294967295 Int64)))

(case
  "an i16 segment round-trips at its two's-complement minimum"
  (doc
    "`(bin (i16 -32768))` — the Int16 minimum, 0x8000 (only the sign bit set across two bytes) —
           matched by `(bin (i16 n))` reads back -32768, NOT +32768 or a mis-sign-extended value. Pins the
           signed multi-byte round-trip at the sign-bit extreme, the i16 companion of the `(i8 -128)` ⇆
           0x80 round-trip.")
  (input (match (bin (i16 -32768)) ((bin (i16 n)) n) (_ 0)))
  (output (: -32768 Int64)))

(case
  "a bits pattern segment reads sub-byte fields back into integers"
  (doc
    "The packed byte `0b1010_0101` matched against `(bin (bits a 1) (bits b 2) (bits c 5))` reads the
           three most-significant-field-first sub-byte fields back: a = 1, b = 0b01 = 1, c = 0b00101 = 5,
           whose sum a + b + c is 7. Pins that `bits` segments read in the same most-significant-first order
           they pack in — the inverse of the bit-field construction case — and that a bits-only pattern must
           still close a whole byte to be well-formed.")
  (input
    (match
      (Bytes.of #list(0b10100101))
      ((bin (bits a 1) (bits b 2) (bits c 5)) (+ (+ a b) c))
      (_ 0)))
  (output (: 7 Int64)))

(case
  "a bits pattern with a 3/5 split reads the high and low fields most-significant-first"
  (doc
    "The packed byte `0b1010_0101` (= 165) matched against `(bin (bits a 3) (bits b 5))` reads the
           high 3 bits into a and the low 5 into b most-significant-first: a = 0b101 = 5, b = 0b00101 = 5.
           The composite `100*a + b` = 505 witnesses BOTH fields (a different field split than the 1/2/5
           case above, so a decoder that hardcoded field widths would miss it).")
  (input (match (Bytes.of #list(165)) ((bin (bits a 3) (bits b 5)) (+ (* 100 a) b)) (_ -1)))
  (output (: 505 Int64)))

(case
  "a bit-field run spanning a byte boundary reads as one big-endian int split by field width"
  (doc
    "A `(bits a 4) (bits b 12)` pattern run closes on two bytes: the run is read as one 16-bit
           big-endian integer then split by field width. Over `(Bytes.of (list 171 205))` = 0xABCD, a is
           the high nibble 0xA = 10 and b the low 12 bits 0xBCD = 3021; composite `10000*a + b` = 103021.
           Pins that bit-field PATTERN decoding crosses a byte boundary the same way construction packs it.")
  (input (match (Bytes.of #list(171 205)) ((bin (bits a 4) (bits b 12)) (+ (* 10000 a) b)) (_ -1)))
  (output (: 103021 Int64)))

(case
  "a leading literal bit-field tag that misses falls through to the catch-all"
  (doc
    "A LITERAL bit-field segment gates the arm: `(bin (bits 1 1) (bits x 7))` matches only a byte
           whose top bit is 1. Over `(Bytes.of (list 1))` = 0b0000_0001 the top bit is 0, so the arm does
           NOT fire and the match takes the catch-all → -1. The const-scrutinee companion of the runtime
           top-bit-tag dispatch; pins that a literal bit-field probe is a genuine gate, not always-true.")
  (input (match (Bytes.of #list(1)) ((bin (bits 1 1) (bits x 7)) x) (_ -1)))
  (output (: -1 Int64)))

(case
  "an integer segment after a byte-aligned bit-field run reads at the advanced offset"
  (doc
    "Once a `bits` run closes on a byte boundary, a following `uNN` segment reads at the run's
           advanced byte offset: `(bin (bits a 3) (bits b 5) (u8 c))` over `(Bytes.of (list 165 42))` reads
           a = 5, b = 5 from byte 0 and c = 42 from byte 1; composite `100*a + b + c` = 547. Pins that a
           fixed-width segment composes with a preceding bit-field run in PATTERN position.")
  (input
    (match
      (Bytes.of #list(165 42))
      ((bin (bits a 3) (bits b 5) (u8 c)) (+ (+ (* 100 a) b) c))
      (_ -1)))
  (output (: 547 Int64)))

(case
  "a bit-field sizes a dependent bytes segment"
  (doc
    "A `(bits n 8)` field can name the size of a following dependent `(bytes payload n)`: over
           `(Bytes.of (list 2 65 66))` the 8-bit field reads n = 2, then binds 2 payload bytes → its
           length is 2. The bit-field size read rides the same non-negative length floor a fixed-int size
           does; the truncated companion below overruns and falls through.")
  (input
    (match
      (Bytes.of #list(2 65 66))
      ((bin (bits n 8) (bytes payload n)) (Bytes.len payload))
      (_ -1)))
  (output (: 2 Int64)))

(case
  "a bit-field-sized dependent bytes segment that overruns the input falls through"
  (doc
    "The miss companion: `(bin (bits n 8) (bytes payload n))` over `(Bytes.of (list 2 65))` reads
           n = 2 but only one byte follows the prefix, so the dependent segment overruns and the arm does
           not match → catch-all -1. Pins that a bit-field-named size is bounds-checked like a fixed-int
           size, never a backward or out-of-range read.")
  (input
    (match (Bytes.of #list(2 65)) ((bin (bits n 8) (bytes payload n)) (Bytes.len payload)) (_ -1)))
  (output (: -1 Int64)))

(case
  "a u64 pattern segment reads eight big-endian bytes back into an integer"
  (doc
    "The eight bytes `(list 0 0 0 0 0 0 1 2)` matched against `(bin (u64 n))` read back big-endian as
           n = 258, round-tripping the u64 construction case. Pins the widest fixed-width segment in pattern
           position and that it consumes exactly its eight bytes.")
  (input (match (bin (u64 258)) ((bin (u64 n)) n) (_ 0)))
  (output (: 258 Int64)))

(case
  "a literal segment before a binder dispatches then reads the payload"
  (doc
    "The pattern `(bin (u8 1) (u16 n))` fires only when the first byte equals the tag 1, then reads
           the following big-endian u16 as n = 258. Against a leading tag of 1 the arm matches and yields
           258; a fixed literal and a binder compose in one pattern. Pins tag-then-field dispatch, the shape
           a tagged binary record takes.")
  (input (match (Bytes.of #list(1 1 2)) ((bin (u8 1) (u16 n)) n) (_ 0)))
  (output (: 258 Int64)))

(case
  "a bin pattern must consume the whole scrutinee or it does not match"
  (doc
    "The scrutinee is three bytes but the arm `(bin (u16 n))` describes only two, leaving one byte
           unconsumed, so the arm does NOT match and control falls to the catch-all, yielding 0. Pins that a
           `bin` pattern matches the ENTIRE byte sequence — leftover bytes are a non-match, which is why a
           trailing `(bytes rest)` segment is needed to accept a variable-length remainder (the next case).
           This is the whole-scrutinee accounting a length-framing loop relies on.")
  (input (match (Bytes.of #list(1 2 3)) ((bin (u16 n)) n) (_ 0)))
  (output (: 0 Int64)))

(case
  "a trailing bytes segment accepts the leftover the fixed segments leave"
  (doc
    "The companion of the whole-consumption case: adding a final `(bytes rest)` to the same
           three-byte scrutinee lets the u16 read the first two bytes (n = 258) and `rest` absorb the third,
           so the arm matches and yields n = 258. Pins that a trailing unsized `(bytes …)` is exactly what
           relaxes the whole-scrutinee rule to accept a variable-length tail.")
  (input (match (Bytes.of #list(1 2 3)) ((bin (u16 n) (bytes rest)) n) (_ 0)))
  (output (: 258 Int64)))

(case
  "an empty scrutinee matches an empty bin pattern"
  (doc
    "The zero-length Bytes value matches the empty pattern `(bin)` — no segments to read, nothing
           left over — so the arm fires and yields \"empty\". The inverse of the `(bin)` construction case,
           and the base case a recursive framing parser bottoms out on. Pins that `(bin)` in pattern
           position matches exactly the empty sequence.")
  (input (match (Bytes.of #list()) ((bin) "empty") (_ "nonempty")))
  (output (: "empty" String)))

(case
  "a non-empty scrutinee does not match an empty bin pattern"
  (doc
    "The companion of the empty-matches-empty case: a one-byte scrutinee has a leftover byte the empty
           pattern `(bin)` does not consume, so the arm does not match and control falls to the catch-all.
           Pins that `(bin)` matches ONLY the empty sequence — the whole-consumption rule applied to the
           zero-segment pattern.")
  (input (match (Bytes.of #list(0)) ((bin) "empty") (_ "nonempty")))
  (output (: "nonempty" String)))

(case
  "a dependent-size segment binds exactly the number of bytes an earlier segment named"
  (doc
    "Against `(Bytes.of (list 2 10 20 99))`, the pattern `(bin (u8 n) (bytes body n) (bytes rest))`
           reads n = 2 from the first byte, then binds `body` to exactly the next n = 2 bytes
           `(list 10 20)`, leaving `rest` = `(list 99)`. The crown jewel: a segment's size is a value
           bound earlier in the same pattern, all value-level. This case checks `body`; the next checks
           `rest`.")
  (input
    (match
      (Bytes.of #list(2 10 20 99))
      ((bin (u8 n) (bytes body n) (bytes rest)) (= body (Bytes.of #list(10 20))))
      (_ false)))
  (output (: true Bool)))

(case
  "a final bytes segment binds the remainder after the sized segments"
  (doc
    "The companion of the dependent-size case: the same match returns `rest`, the bytes after the
           n-byte body — `(list 99)`. Pins that a final unsized `(bytes rest)` captures everything left,
           the remainder a framing loop iterates on.")
  (input
    (match
      (Bytes.of #list(2 10 20 99))
      ((bin (u8 n) (bytes body n) (bytes rest)) (= rest (Bytes.of #list(99))))
      (_ false)))
  (output (: true Bool)))

(case
  "a dependent-size utf8 segment decodes a length-prefixed string"
  (doc
    "The string-decoding companion of the dependent-size `(bytes body n)` case: `(bin (u8 n) (utf8 s n))`
           reads a one-byte length `n = 2`, then decodes the next `n` bytes as strict UTF-8, binding `s` to
           the resulting String. Against `(Bytes.of (list 2 104 105))` — length 2 then the bytes 104 105 =
           `hi` — `s` = \"hi\". Pins that a `utf8` segment binds a DECODED STRING (not raw bytes), sized by an
           earlier integer segment, the length-prefixed-string idiom (a UTF-8 field is decoded, validated, and
           bound in one pattern). Ill-formed UTF-8 in those bytes would be a non-match (falls to the catch-all),
           never a trap — decoding-bytes-to-a-string is total.")
  (input (match (Bytes.of #list(2 104 105)) ((bin (u8 n) (utf8 s n)) s) (_ "miss")))
  (output (: "hi" String)))

(case
  "a utf8 segment sized by a CONSTANT LITERAL decodes a fixed-width string"
  (doc
    "A sized `utf8` segment's size MAY be a CONSTANT LITERAL, not only a name bound by an earlier
           segment: `(bin (utf8 s 2))` decodes exactly the first 2 bytes as strict UTF-8. Against
           `(Bytes.of (list 104 105))` = `hi` → `s` = \"hi\". A literal size is the most basic sized-segment
           form (Erlang bit-syntax precedent: a segment size is an integer expression, of which a literal is
           the simplest); it MUST match, exactly like the named/dependent form above. Pins that a constant
           size is accepted — earlier it silently fell through to the catch-all (a miscompile: the size
           resolver looked up an earlier-segment binder BY NAME and a literal returned nothing → non-match),
           fixed 2026-07-21 (ruling (a)) so `bin_decode_dependent_size` accepts a constant-literal size.")
  (input (match (Bytes.of #list(104 105)) ((bin (utf8 s 2)) s) (_ "miss")))
  (output (: "hi" String)))

(case
  "a constant-size utf8 segment over ill-formed UTF-8 is a non-match, not a trap"
  (doc
    "The totality companion of the constant-size utf8 case: a `(bin (utf8 s 2))` segment whose 2 bytes
           are NOT valid UTF-8 must FALL THROUGH to the catch-all, never trap — decoding-bytes-to-a-string is
           total (the ill-formed case is a branch the exhaustiveness rule forces the program to carry).
           Against `(Bytes.of (list 255 255))` — 0xFF 0xFF is not a well-formed UTF-8 sequence (an invalid
           lead byte) — the utf8 segment does not match, so the `_` arm yields \"bad\". Pins that the strict
           `str::from_utf8` validation in the decode (which the constant-literal-size path now reaches, ruling
           (a)) treats ill-formed bytes as a non-match, the soundness guard that pairs with the const-size fix.")
  (input (match (Bytes.of #list(255 255)) ((bin (utf8 s 2)) s) (_ "bad")))
  (output (: "bad" String)))

(case
  "a bytes segment sized by a CONSTANT LITERAL binds a fixed number of bytes then composes"
  (doc
    "The `bytes` companion of the constant-literal-size utf8 case: `(bin (bytes b 2) (u8 last))` binds
           exactly the first 2 bytes to `b`, then a following `u8` segment reads the third byte — a
           constant-size prefix followed by more segments (NOT a final variable-length segment). Against
           `(Bytes.of (list 10 20 30))` → `b` = `[10, 20]` (len 2), `last` = 30, so `Bytes.len b + last` =
           32. Pins that a constant-literal `bytes` size (a) matches and (b) composes with a following
           segment at the now-static offset — the same ruling-(a) fix as the utf8 case, and the bytes
           analogue that a fixed-size segment need not be the final one.")
  (input
    (match (Bytes.of #list(10 20 30)) ((bin (bytes b 2) (u8 last)) (+ (Bytes.len b) last)) (_ -1)))
  (output (: 32 Int64)))

(case
  "a literal segment matches a magic-number header by equality"
  (doc
    "The pattern `(bin (u32 0x89504E47) (bytes rest))` matches the scrutinee only when its first
           four bytes equal the magic number (137 80 78 71) — a literal segment matches by equality, the
           direct analogue of a literal value pattern, and the hex literal names the magic number
           legibly. Pins magic-number dispatch on a binary header.")
  (input
    (match
      (Bytes.of #list(137 80 78 71 1 2))
      ((bin (u32 0x89504e47) (bytes rest)) "match")
      (_ "other")))
  (output (: "match" String)))

(case
  "a bin arm whose fixed-width segment overruns the input falls through"
  (doc
    "The scrutinee `(Bytes.of (list 5))` is one byte; the arm `(bin (u16 n) (bytes rest))` needs
           two bytes for its u16, so it cannot match and control falls to the catch-all, yielding 0.
           Pins that too-short input is a non-match (the arm simply does not fire), not a trap — the same
           total-or-trap discipline the corpus pins for a `bytes` segment that overruns.")
  (input (match (Bytes.of #list(5)) ((bin (u16 n) (bytes rest)) n) (_ 0)))
  (output (: 0 Int64)))

(case
  "a dependent-size segment that overruns the remaining bytes falls through"
  (doc
    "Against `(Bytes.of (list 9 1 2))`, the pattern `(bin (u8 n) (bytes body n))` reads n = 9 but
           only two bytes remain, so `(bytes body 9)` cannot bind nine bytes and the arm falls to the
           catch-all. Pins that a dependent size larger than the remainder is a non-match, not a trap or
           a short read.")
  (input (match (Bytes.of #list(9 1 2)) ((bin (u8 n) (bytes body n)) (Bytes.len body)) (_ -1)))
  (output (: -1 Int64)))

(case
  "a dependent size of zero binds an empty body and leaves the rest untouched"
  (doc
    "Against `(Bytes.of (list 0 42))`, the pattern `(bin (u8 n) (bytes body n) (bytes rest))` reads
           n = 0, so `(bytes body 0)` binds the EMPTY byte sequence and `rest` gets the whole remainder
           `(list 42)`. This case checks `body` is empty. Pins that a zero dependent size is a valid
           non-match-free read (an empty field), not an overrun or a special case — the degenerate framing a
           loop hits on a zero-length record.")
  (input
    (match
      (Bytes.of #list(0 42))
      ((bin (u8 n) (bytes body n) (bytes rest)) (= body (Bytes.of #list())))
      (_ false)))
  (output (: true Bool)))

(case
  "two dependent-size segments each bind the count a preceding segment named"
  (doc
    "Against `(Bytes.of (list 1 2 2 10 20 99))`, the pattern
           `(bin (u8 a) (bytes x a) (u8 b) (bytes y b) (bytes rest))` reads the two length-prefixed fields
           in sequence: a = 1 → `x` = `(list 2)`; then b = 2 → `y` = `(list 10 20)`; the final byte 99 lands
           in `rest`. This case checks `y`. Pins that several dependent sizes chain in one pattern, each
           reading a count bound just before it — the sequential length-prefixed framing a single `bin`
           expresses without a loop.")
  (input
    (match
      (Bytes.of #list(1 2 2 10 20 99))
      ((bin (u8 a) (bytes x a) (u8 b) (bytes y b) (bytes rest)) (= y (Bytes.of #list(10 20))))
      (_ false)))
  (output (: true Bool)))

; The dependent-size cases above cover bind-exactly, remainder, zero, chained, and a clear overrun. These
; pin the size-arithmetic BOUNDARIES a naive `remaining >= n` check can slip on: a size EXACTLY equal to the
; remaining bytes (the last-fits boundary), a size ONE PAST it (off-by-one overrun), and — the soundness
; one — a SIGNED size segment whose value reads NEGATIVE (0xFF over i8 = -1), which must fall through rather
; than read backwards or underflow the remaining-bytes subtraction / wrap the count to a huge unsigned slice.
(case
  "a dependent-size segment binding exactly the remaining bytes leaves an empty rest"
  (doc
    "The last-fits boundary: `(bin (u8 n) (bytes body n))` against `(Bytes.of (list 3 10 20 30))` reads
           n = 3, and exactly 3 bytes remain, so `body` binds all of `(list 10 20 30)` and the pattern
           consumes the whole scrutinee → matches. Pins that size == remaining is a MATCH (not an overrun) —
           the boundary a strict `remaining > n` check would wrongly reject; it must be `remaining >= n`.")
  (input
    (match
      (Bytes.of #list(3 10 20 30))
      ((bin (u8 n) (bytes body n)) (= body (Bytes.of #list(10 20 30))))
      (_ false)))
  (output (: true Bool)))

(case
  "a dependent size one past the remaining bytes overruns and falls through"
  (doc
    "The off-by-one overrun: the SAME pattern against `(Bytes.of (list 4 10 20 30))` reads n = 4, but
           only 3 bytes remain — one short — so `(bytes body 4)` overruns and the arm does NOT match, falling
           to the wildcard → false. The strict complement of the exact-fit case: size = remaining + 1 is the
           first overrun. Pins the bounds check is exact at the one-byte margin.")
  (input (match (Bytes.of #list(4 10 20 30)) ((bin (u8 n) (bytes body n)) true) (_ false)))
  (output (: false Bool)))

(case
  "a signed dependent size that reads negative falls through, not a backward read"
  (doc
    "The soundness case: a SIGNED size segment `(i8 n)` reading the byte 0xFF = 255 interprets it as
           the two's-complement -1. A negative byte count has no valid read — `(bytes body n)` with n = -1
           must FALL THROUGH (→ false), NOT read backwards, wrap the count to a huge unsigned slice length,
           or underflow the `remaining - n` subtraction. `(Bytes.of (list 255 10 20))` matched with `(bin
           (i8 n) (bytes body n))` → false. Pins a negative dependent size is rejected at the bounds check,
           the signed companion of the overrun case — a naive unsigned cast of the count would read 255
           bytes (overrun) or a wrap could pass a huge count to a slice.")
  (input (match (Bytes.of #list(255 10 20)) ((bin (i8 n) (bytes body n)) true) (_ false)))
  (output (: false Bool)))

(case
  "constructing a sized bytes segment whose value length differs from the size is rejected"
  (doc
    "`(bin (bytes (Bytes.of (list 1 2 3)) 2))` splices a three-byte value into a segment declared to
           be two bytes wide; the declared size and the value's length disagree, so there is no defined
           encoding. With CONSTANT operands the mismatch is provable at compile time, so — like every
           compile-provable trap — it FAILS THE BUILD (CDZ0304) rather than shipping a component that traps
           (reference-compiler.md #A Compile-Provable Trap Fails The Build); a runtime `b`/`n` whose lengths
           disagree traps \"binary value does not fit segment\" at that point. Pins that a SIZED `(bytes b
           n)` build is length-checked against n — the whole-value analogue of the u8 out-of-range check,
           and the construction-side counterpart of a matching `(bytes body n)` overrun being a non-match.")
  (input (bin (bytes (Bytes.of #list(1 2 3)) 2)))
  (error CDZ0304))

; --- A bin pattern applies only to a Bytes scrutinee -----------------------------------------
; A `(bin …)` pattern DECODES a Bytes value, so it is well-formed only over a Bytes scrutinee. Matching
; it against a definite NON-Bytes scrutinee — an Int64, a String, a List — is a type error (CDZ0203, 'a
; `(bin …)` pattern decodes a Bytes value, but this scrutinee is <T>'), the bin twin of the map-key /
; list-element pattern-type checks. It is caught at the offending arm, not silently accepted (which left
; the arm to fall through to a misleading generic 'pattern not yet supported'). An unsolved (`Any`/`Var`)
; scrutinee is skipped — a runtime Bytes may still flow in — so the reject fires only on a DEFINITE
; non-Bytes type, whether a constant or a runtime parameter.
(case
  "a bin pattern over an Int64 scrutinee is a type error"
  (doc
    "`(match 5 ((bin (u8 x)) x) (_ 0))` matches a `(bin …)` pattern against the Int64 `5`. A bin
           pattern decodes a Bytes value, and an Int64 is not Bytes, so it is rejected (CDZ0203, naming the
           scrutinee's type). Pins that the bin pattern's scrutinee-type check fires on a definite non-Bytes
           scalar — the binary-matching companion of a list pattern over a non-list scrutinee.")
  (input (do (def (main) (match 5 ((bin (u8 x)) x) (_ 0))) (export main)))
  (error CDZ0203))

(case
  "a bin pattern over a String scrutinee is a type error"
  (doc
    "`(match \"hi\" ((bin (u8 x)) x) (_ 0))` — a String is not Bytes (text vs a byte sequence are
           distinct types; the bridge is `String.to-bytes`/`from-bytes`), so a bin pattern over it is
           rejected (CDZ0203). Pins that a String scrutinee does not silently decode as bytes — the author
           must encode it first.")
  (input (do (def (main) (match "hi" ((bin (u8 x)) x) (_ 0))) (export main)))
  (error CDZ0203))

(case
  "a bin pattern over a List scrutinee is a type error"
  (doc
    "`(match (list 1 2) ((bin (u8 x)) x) (_ 0))` — a `(List Int64)` is not Bytes (a list of integers
           is not a byte sequence; `Bytes.of` is the explicit bridge), rejected CDZ0203. Pins that the
           scrutinee-type check covers a compound collection, not only a scalar.")
  (input (do (def (main) (match #list(1 2) ((bin (u8 x)) x) (_ 0))) (export main)))
  (error CDZ0203))

(case
  "a bin pattern over a runtime non-Bytes scrutinee is a type error"
  (doc
    "`(match n ((bin (u8 x)) x) (_ 0))` with `n` a runtime Int64 parameter — the scrutinee is a
           definite non-Bytes type known statically even though its value arrives at run time, so the reject
           still fires (CDZ0203). Pins that the check is on the scrutinee's static TYPE, not whether it is a
           constant; a runtime Int64 is rejected exactly as the constant `5` is (distinct from an unsolved
           `Any`/`Var` scrutinee, which is skipped because a runtime Bytes may flow in).")
  (input (do (def (main (: n Int64)) (match n ((bin (u8 x)) x) (_ 0))) (export main)))
  (error CDZ0203))

; ============================================================================================
; Protocol round-trips — construct and match are inverse over a whole realistic layout
; ============================================================================================
(case
  "a tag-length-value record round-trips through construct then match"
  (doc
    "A TLV record is built `(bin (u8 7) (u16 3) (bytes payload))` — a one-byte tag, a big-endian u16
           length, then the payload — and matched back with `(bin (u8 7) (u16 n) (bytes body n))`: the
           literal tag 7 dispatches, the length n = 3 sizes the dependent `body`, which recovers the
           original payload. Pins the canonical tag-length-value shape in one expression, showing the
           literal, fixed-width, and dependent-size segments compose into a real record grammar.")
  (input
    (match
      (bin (u8 7) (u16 3) (bytes (Bytes.of #list(100 101 102))))
      ((bin (u8 7) (u16 n) (bytes body n)) (= body (Bytes.of #list(100 101 102))))
      (_ false)))
  (output (: true Bool)))

(case
  "a magic header and a length-prefixed chunk parse together"
  (doc
    "A PNG-style layout — the u32 magic `0x89504E47`, a u32 chunk length, then that many data bytes —
           is built `(bin (u32 0x89504E47) (u32 2) (bytes data))` and parsed with
           `(bin (u32 0x89504E47) (u32 len) (bytes body len))`: the literal magic segment gates the parse
           and the u32 length sizes the chunk body. Pins that a magic-number guard and a dependent-size
           chunk chain in one pattern — the shape a chunked container format (PNG, RIFF) is read with.")
  (input
    (match
      (bin (u32 0x89504e47) (u32 2) (bytes (Bytes.of #list(65 66))))
      ((bin (u32 0x89504e47) (u32 len) (bytes body len)) (= body (Bytes.of #list(65 66))))
      (_ false)))
  (output (: true Bool)))

(case
  "parsing a length-framed message and rebuilding it yields the original bytes"
  (doc
    "The strongest inverse statement: a length-framed message is matched with
           `(bin (u16 n) (bytes body n))`, then rebuilt from the bound `n` and `body` with
           `(bin (u16 n) (bytes body))`, and the rebuilt bytes equal the original frame. Pins that
           construction and matching are genuinely inverse — parse-then-serialize is the identity on a
           well-formed frame — not merely that each direction works in isolation.")
  (input
    (let
      ((frame (Bytes.of #list(0 3 10 20 30))))
      (match frame ((bin (u16 n) (bytes body n)) (= (bin (u16 n) (bytes body)) frame)) (_ false))))
  (output (: true Bool)))

(case
  "a header of packed nibbles and a length-prefixed body round-trips"
  (doc
    "A header packs a 4-bit version and 4-bit flags into one byte, followed by a u16 length and that
           many payload bytes: built `(bin (bits 1 4) (bits 2 4) (u16 2) (bytes payload))`, matched with
           `(bin (bits ver 4) (bits flags 4) (u16 n) (bytes body n))`, then rebuilt and compared to the
           original. Pins that sub-byte bit-fields participate in a full round-trip alongside byte-aligned
           segments — the mixed bit-and-byte header a wire protocol actually uses.")
  (input
    (let
      ((msg (bin (bits 1 4) (bits 2 4) (u16 2) (bytes (Bytes.of #list(9 9))))))
      (match
        msg
        ((bin (bits ver 4) (bits flags 4) (u16 n) (bytes body n))
          (= (bin (bits ver 4) (bits flags 4) (u16 n) (bytes body)) msg))
        (_ false))))
  (output (: true Bool)))

; ============================================================================================
; Exhaustiveness — a match over Bytes needs a catch-all (existing CDZ0210 rule, no special case)
; ============================================================================================
(case
  "a match over bytes with only a bin arm and no catch-all is non-exhaustive"
  (doc
    "A `bin` pattern never covers every byte sequence — the empty sequence, a shorter sequence, or
           one whose literal segments differ all fail to match — so a match whose only arm is a `(bin …)`
           pattern does not cover the scrutinee's type and is rejected CDZ0210, exactly as a sum match
           missing a variant is. Pins that binary matching reuses the exhaustiveness rule rather than a
           special case. A generation that does not yet cover the rule declines (todo), not miscompiles.")
  (input (match (Bytes.of #list(1 2)) ((bin (u16 n)) n)))
  (error CDZ0210))

; ============================================================================================
; Ill-formed binary forms — static rejection CDZ0220 (byte-alignment and well-formedness)
; ============================================================================================
(case
  "a binary form whose bit-fields do not close a byte is ill-formed"
  (doc
    "`(bin (bits 1 1) (bits 0 3))` has bit-field widths summing to 4, so the form is not
           byte-aligned and no whole number of bytes is emitted. Because the widths are compile-time
           constants the misalignment is caught statically: an ill-formed binary form, rejected CDZ0220.
           Pins the byte-alignment discipline as a compile-time check, not a runtime surprise.")
  (input (bin (bits 1 1) (bits 0 3)))
  (error CDZ0220))

(case
  "a non-final unsized bytes segment is ill-formed"
  (doc
    "`(bin (bytes a) (u8 1))` places an unsized `(bytes a)` before another segment: an unsized
           bytes segment consumes all remaining bytes, so anything after it can never be reached, an
           ill-formed binary form rejected CDZ0220. Pins that an unsized `bytes` is legal only as the
           final segment (a sized `(bytes a n)` may appear anywhere).")
  (input (bin (bytes (Bytes.of #list(1 2))) (u8 1)))
  (error CDZ0220))

(case
  "a bit-field width that is not a compile-time constant is ill-formed"
  (doc
    "`(bin (bits 1 k))` uses a run-time value k as a bit-field width; a `bits` width must be a
           compile-time constant so the form's byte-alignment is statically checkable. A non-constant
           width is an ill-formed binary form rejected CDZ0220. Pins that widths are static even though
           the values filling them are dynamic.")
  (input (let ((k 3)) (bin (bits 1 k))))
  (error CDZ0220))

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
(case
  "a misspelled segment kind is rejected with a rename suggestion"
  (doc
    "`(bin (byte 5))` uses `byte` where the kind is `bytes` — a confident typo of a known segment
           kind. The kind head names no segment, so it is rejected (CDZ0201, 'unrecognized bin segment kind
           `byte` — did you mean `bytes`?'), the bin-segment did-you-mean. Pins the misspelled-kind path
           (the rename fix), distinct from a valid kind with a bad layout (CDZ0220 above). (fix migrated
           from rcdzc a_misspelled_bin_segment_kind_offers_the_rename_fix.)")
  (input (do (def (main) (bin (byte 5))) (export main)))
  (error CDZ0201 (message "did you mean `bytes`?") (fix (kind replace) (replacement "bytes"))))

(case
  "a misspelled utf8 segment kind is rejected"
  (doc
    "`(bin (utf 65))` uses `utf` for `utf8` — another confident kind typo, rejected CDZ0201 with the
           `utf8` rename suggestion. Pins that the did-you-mean covers the text-segment kind as well as
           `bytes`, so a near-miss on any closed-vocabulary kind is named.")
  (input (do (def (main) (bin (utf 65))) (export main)))
  (error CDZ0201 (message "did you mean `utf8`?") (fix (kind replace) (replacement "utf8"))))

(case
  "a misspelled bits segment kind is rejected with the bits rename"
  (doc
    "`(bin (bit b))` uses `bit` for `bits` — the bit-width-segment kind typo, rejected CDZ0201 with the
           `bits` rename. Completes the did-you-mean coverage across the closed kind vocabulary. From rcdzc
           a_misspelled_bin_segment_kind_offers_the_rename_fix.")
  (input (do (def (f (: b Bytes)) (bin (bit b))) (export f)))
  (error CDZ0201 (message "did you mean `bits`?") (fix (kind replace) (replacement "bits"))))

(case
  "a fixed-width integer segment of a non-byte-aligned width is rejected"
  (doc
    "`(bin (u9 5))` names a 9-bit unsigned integer segment — `uNN` segments are the byte-aligned
           widths u8/u16/u32/u64 only, so `u9` is not a segment kind (CDZ0201, with the dedicated message
           pointing at `(bits v k)` for an arbitrary bit width). Distinct from a plain typo: the head is a
           recognizable `u`-integer shape at a width the byte-aligned segments do not offer, so it keeps its
           own message rather than a rename. Pins the wrong-integer-width path.")
  (input (do (def (main) (bin (u9 5))) (export main)))
  (error CDZ0201))

(case
  "a far-miss segment kind keeps the plain unrecognized message"
  (doc
    "`(bin (zzz 5))` uses `zzz` — a head that is not close to any known kind. It is rejected
           (CDZ0201) with the plain 'unrecognized bin segment kind (expected uNN/iNN/bits/bytes/utf8)'
           message, no rename fix (there is no confident correction). Pins the far-miss path, the
           complement of the misspelled-kind case: the vocabulary is closed and a head outside it — near or
           far — is rejected.")
  (input (do (def (main) (bin (zzz 5))) (export main)))
  (error CDZ0201 (message "unrecognized bin segment kind") (no-fix)))

; ============================================================================================
; Fit — a segment REQUIRES its width-typed value. A CONSTANT literal grounds to that width and a
; provable overflow FAILS THE BUILD (CDZ0304 / CDZ0220); a NON-CONSTANT value of a different integer
; type is a COMPILE-TIME TYPE ERROR (CDZ0203). A value that fits its type has no out-of-range case, so
; construction NEVER traps — an out-of-range value is a type failure, not a runtime failure.
; ============================================================================================
(case
  "constructing a u8 segment from a value above its range is rejected"
  (doc
    "`(bin (u8 256))` asks an 8-bit unsigned segment to hold 256, which needs nine bits and has no
           8-bit encoding — it does NOT truncate to 0. The literal grounds to the segment's `UInt8` and the
           overflow is provable, so it FAILS THE BUILD (CDZ0304) — the compile-provable-trap rule
           (reference-compiler.md #A Compile-Provable Trap Fails The Build). The companion of the Bytes
           out-of-range check, at the segment boundary. (A non-constant value of the wrong type is a
           CDZ0203 type error — see the runtime section.)")
  (input (bin (u8 256)))
  (error CDZ0304))

(case
  "constructing an unsigned segment from a negative value is rejected"
  (doc
    "`(bin (u8 -1))` gives a negative value to an UNSIGNED segment, which has no negative encoding —
           it does NOT wrap to 255 (that is the meaning of the SIGNED `(i8 -1)` case above). The literal
           grounds to the segment's `UInt8` and the negative is out of range, so it FAILS THE BUILD
           (CDZ0304). Pins that unsigned and signed segments differ on a negative value: the signed one
           encodes it in two's complement, the unsigned one has no encoding.")
  (input (bin (u8 -1)))
  (error CDZ0304))

(case
  "constructing a bit-field from a value wider than its width is rejected"
  (doc
    "`(bin (bits 2 1))` gives the value 2 (which needs two bits) to a 1-bit field, so it does not fit.
           With a CONSTANT operand the misfit is provable at compile time, so the ill-formed bit-field is
           rejected (CDZ0220 — the binary well-formedness code). Pins that a bit-field's value is
           range-checked against its width, the sub-byte companion of the u8-overflow check.")
  (input (bin (bits 2 1)))
  (error CDZ0220))

(case
  "a narrower-typed runtime value is not silently widened into a wider segment"
  (doc
    "The segment's required type is EXACT in both width and signedness — it is a TYPE match, not a
           value-range fit. A runtime `UInt8` fed to a `u16` segment is a COMPILE-TIME type error (CDZ0203),
           even though every `UInt8` value trivially fits 16 bits: widening is as explicit as narrowing, never
           implicit (the caller writes `UInt16.of` / `UInt16.wrap`). Pins that `(u16 v)` requires a `UInt16`,
           not merely an integer that fits — so a future change cannot quietly accept any narrower unsigned
           value. The widening companion of the wider-value rejection (an `Int64` into `u8`).")
  (input (do (def (main (: n UInt8)) (Bytes.len (bin (u16 n)))) (export main)))
  (error CDZ0203))

(case
  "an unsigned runtime value is not accepted by a signed segment of the same width"
  (doc
    "Signedness is strict too: a runtime `UInt8` fed to a SIGNED `i8` segment is a COMPILE-TIME type
           error (CDZ0203) — `(i8 v)` requires an `Int8`, not a same-width unsigned value (their encodings of
           a high value differ: a `UInt8` 200 is not the `Int8` −56). Pins that the segment match is on BOTH
           axes, the signedness companion of the no-silent-widening case.")
  (input (do (def (main (: n UInt8)) (Bytes.len (bin (i8 n)))) (export main)))
  (error CDZ0203))

; ============================================================================================
; Runtime segments — a `bin` whose segment value / scrutinee is a RUNTIME value (a def parameter,
; not a compile-time constant). A fixed-width integer segment REQUIRES the width-matching typed value —
; `(u16 v)` takes a `UInt16`, `(bits v k)` takes a `(UInt k)` — so the value provably fits the segment and
; construction NEVER traps: an out-of-range value is a COMPILE-TIME TYPE ERROR (CDZ0203), and narrowing
; (`UInt8.wrap` to truncate, `UInt8.of` to check) is the caller's job. Matching decodes the scrutinee's
; bytes at run time, binding a general integer. Each threads its value through `main`'s parameter (via
; `(call …)`) so the `bin` cannot fold to a constant.
; ============================================================================================
(case
  "a runtime value is constructed into a fixed-width segment and its length read"
  (doc
    "`(bin (u16 n))` with `n` a RUNTIME `UInt16` parameter (not a constant) builds a two-byte sequence
           on the byte heap at run time — the construction does not fold. Reading its length back yields 2,
           the segment's width. Pins that a `bin` construction whose value is only known at run time still
           produces a well-formed Bytes, and that a `u16` segment takes a `UInt16` value.")
  (input (do (def (main (: n UInt16)) (Bytes.len (bin (u16 n)))) (export main)))
  (call main (: 258 UInt16))
  (output (: 2 Int64)))

(case
  "a runtime fixed-width segment is emitted big-endian and read back by index"
  (doc
    "`(bin (u16 258))` built at run time encodes 258 = 0x0102 BIG-ENDIAN, so byte 0 is the
           most-significant `0x01` = 1. Reads it back with `Bytes.at`. Pins that a runtime construction
           lays its bytes most-significant-first, the same order the constant fold and the pattern decode
           agree on.")
  (input
    (do
      (def (main (: n UInt16)) (match (Bytes.at (bin (u16 n)) 0) ((Some b) b) ((None _) -1)))
      (export main)))
  (call main (: 258 UInt16))
  (output (: 1 Int64)))

(case
  "a wider runtime value in a fixed-width segment is a compile-time type error"
  (doc
    "`(bin (u8 n))` with `n` a runtime `Int64` is a COMPILE-TIME TYPE ERROR (CDZ0203): a `u8` segment
           takes a `UInt8`, and an `Int64` may exceed the 8-bit range, so it is NOT silently accepted and
           range-checked-then-trapped at run time. The value that does not fit becomes a type failure, not a
           runtime failure — the caller must convert explicitly (`UInt8.wrap` to truncate to the low 8 bits,
           `UInt8.of` for a checked narrow). Replaces the former runtime out-of-range TRAP: a width-typed
           segment provably fits its value, so construction never traps.")
  (input (do (def (main (: n Int64)) (Bytes.len (bin (u8 n)))) (export main)))
  (error CDZ0203))

(case
  "a runtime value narrowed to the segment width constructs without trapping"
  (doc
    "The idiom the type error above directs the caller to: `(bin (u8 (UInt8.wrap n)))` with a runtime
           `Int64 n` narrows `n` to `UInt8` (truncating to the low 8 bits, numeric-model.md #wrap Never
           Traps) BEFORE placing it in the segment, so the `u8` segment gets a value it provably fits and
           construction is total. `n = 258` wraps to 2, so the byte sequence is one byte and its length is 1.
           Pins that the caller's explicit narrowing makes the runtime construction total — no trap, the
           point of requiring the width type.")
  (input (do (def (main (: n Int64)) (Bytes.len (bin (u8 (UInt8.wrap n))))) (export main)))
  (call main (: 258 Int64))
  (output (: 1 Int64)))

(case
  "a wrap-narrowed MULTI-BYTE segment decodes the truncated value back"
  (doc
    "The u8 wrap case above observes only the LENGTH of the construction; here the wrapped VALUE is
           observed through a decode, at the multi-byte width: `(bin (u16 (UInt16.wrap v)))` matched by
           `((bin (u16 x)) x)`. In-range values round-trip identically (300 → 300, 65535 → 65535 — the
           u16 ceiling, every bit set across both big-endian bytes), and out-of-range values decode to
           their low 16 bits (65536 = 0x10000 → 0, 65537 → 1) — the truncation happens at the WRAP, before
           the segment, so what the bytes hold IS the wrapped value. A pipeline that range-checked instead
           of truncating, or that wrapped at 8 bits for a 16-bit segment, diverges at the last two calls.")
  (input
    (do
      (def (main (: v Int64)) (match (bin (u16 (UInt16.wrap v))) ((bin (u16 x)) x) (_ -1)))
      (export main)))
  (call main (: 300 Int64))
  (output (: 300 Int64))
  (call main (: 65535 Int64))
  (output (: 65535 Int64))
  (call main (: 65536 Int64))
  (output (: 0 Int64))
  (call main (: 65537 Int64))
  (output (: 1 Int64)))

(case
  "wrap-narrowed values fill BOTH segments of a dispatching frame — the wrapped tag selects the arm"
  (doc
    "The wrap idiom composed with multi-arm dispatch: `(bin (u8 (UInt8.wrap t)) (u16 (UInt16.wrap
           v)))` — a mixed-width frame where BOTH segments take wrap-narrowed runtime `Int64`s, matched by
           a literal-tag arm then a binder arm. t=1 hits the literal arm and returns the u16 field (65535,
           its ceiling); t=258 wraps to 2 BEFORE the bytes exist, so the literal-1 arm MISSES and the
           binder arm decodes `other` = 2 → 2·100000 + 300 = 200300. Pins that dispatch happens on the
           WRAPPED byte (the arm predicate reads what the wrap stored, not the pre-wrap value — a fold
           that tested `t` against the literal instead of `t`'s low byte would take the wrong arm at the
           second call), and that two wrap-narrowed segments of different widths compose in one frame.")
  (input
    (do
      (def
        (main (: t Int64) (: v Int64))
        (match
          (bin (u8 (UInt8.wrap t)) (u16 (UInt16.wrap v)))
          ((bin (u8 1) (u16 x)) x)
          ((bin (u8 other) (u16 y)) (+ (* other 100000) y))
          (_ -1)))
      (export main)))
  (call main (: 1 Int64) (: 65535 Int64))
  (output (: 65535 Int64))
  (call main (: 258 Int64) (: 300 Int64))
  (output (: 200300 Int64)))

; A runtime `(bin …)` construction result IS a Bytes value (this file's opening: "expression position
; `(bin …)` CONSTRUCTS a Bytes value"), so it must be `=`-comparable like any Bytes. It builds a FRESH
; owned Bytes on the rope heap — exactly as `Bytes.of` does — so as an operand of the borrowing `=` it is
; owned and reclaimed after the compare. A runtime bin result compared against the Bytes it builds is true;
; against different content, false. The runtime `Bytes.of` control (same content, runtime) and the CONSTANT
; bin control (folds to a comparable Bytes) both already compare — these pin the RUNTIME bin result joins
; them. (The wasm backend runs these; the RUST backend does not yet render a runtime `(bin …)` value at all
; — a broader rust gap — so on rust they decline, exactly as the runtime bin-MATCH cases below do.)
(case
  "a runtime bin construction result compares equal to the Bytes it builds"
  (doc
    "`(= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))` with v=5: the runtime bin builds the one-byte
           Bytes 0x05, equal to `(Bytes.of (list 5))` → true. A runtime `(bin …)` result is a Bytes value
           (this file's opening), so it is `=`-comparable like any Bytes — it builds a fresh owned Bytes on
           the rope heap exactly as `Bytes.of` does. Before, the runtime bin result as a `value-eq` operand
           was not recognized as an owned heap producer, so `=` DECLINED (an aliasing-can't-prove reject,
           not a miscompile) though a runtime `Bytes.of` of the same content compared fine. Pins the runtime
           bin result compares by content.")
  (input
    (do (def (main (: v Int64)) (= (bin (u8 (UInt8.wrap v))) (Bytes.of #list(5)))) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case
  "a runtime bin construction result unequal to different content is false"
  (doc
    "The discriminator: the same `(= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))` with v=9 builds
           the byte 0x09 ≠ 0x05 → false. Pins the runtime bin `=` is a genuine content compare, not a
           blanket true — the companion of the equal case.")
  (input
    (do (def (main (: v Int64)) (= (bin (u8 (UInt8.wrap v))) (Bytes.of #list(5)))) (export main)))
  (call main (: 9 Int64))
  (output (: false Bool)))

(case
  "an explicit Bytes annotation on a runtime bin result compares the same"
  (doc
    "The annotated form: `(= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of (list 5)))` with v=5 → true.
           The annotation asserts the runtime bin's Bytes type explicitly; it compares exactly as the
           unannotated case. Pins the gap was never type inference failing to see it as Bytes (the annotation
           confirms Bytes) — the `=` lowering simply had to recognize the runtime bin result as an owned
           Bytes producer, which it now does.")
  (input
    (do
      (def (main (: v Int64)) (= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of #list(5))))
      (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case
  "a runtime Bytes.of value is equality-comparable (the runtime-Bytes control)"
  (doc
    "The control that always worked: `(= (Bytes.of (list (UInt8.wrap v))) (Bytes.of (list 5)))` with
           v=5 → true. Runtime `Bytes.of` `=` is fine; pins the runtime bin case joins it (the gap was
           specific to the runtime `bin` construction result, not runtime Bytes equality in general).")
  (input
    (do
      (def (main (: v Int64)) (= (Bytes.of #list((UInt8.wrap v))) (Bytes.of #list(5))))
      (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case
  "a runtime bin pattern decodes a fixed-width segment from a runtime scrutinee"
  (doc
    "A `(bin …)` pattern matches a RUNTIME Bytes scrutinee: `(bin (u16 n))` is built from a runtime
           `UInt16` parameter, then matched back with `(bin (u16 m))`, binding `m = n = 258`. The decode
           reads the scrutinee's bytes at run time (a length probe + a big-endian assemble), round-tripping
           the construction. Pins that construction (which takes the width type) and matching (which binds a
           general integer) are inverse over a runtime value, not just a constant.")
  (input
    (do (def (main (: n UInt16)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -1))) (export main)))
  (call main (: 258 UInt16))
  (output (: 258 Int64)))

(case
  "a guarded bin-match arm reads its decoded binder and falls through when the guard fails"
  (doc
    "A `(bin …)` pattern under a GUARD, over a RUNTIME scrutinee — the composition of runtime
           bin-decode and match-arm guards. The arm `(guard (bin (u8 n)) (> n 5))` decodes the one-byte
           field `n` via a runtime `BinIntRead`, then gates the arm on `(> n 5)` reading that decoded
           binder; a second guarded arm `(guard (bin (u8 n)) (> n 0))` catches the 1..5 range, and the
           wildcard catches 0. The scrutinee is `(bin (u8 h))` built from a runtime `h` so it cannot fold.
           Pins that a guard sees the runtime-decoded segment binder in scope AND that a failing guard on a
           bin arm FALLS THROUGH to the next arm (which re-probes the same materialized scrutinee), not
           traps — the bin analogue of the scalar `guarded arm falls through` case. h=9 → first guard
           `9 > 5` holds → 100; h=3 → first guard fails, second `3 > 0` holds → 200; h=0 → both guards fail
           → wildcard 300.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (bin (u8 (UInt8.wrap h)))
          ((guard (bin (u8 n)) (> n 5)) 100)
          ((guard (bin (u8 n)) (> n 0)) 200)
          (_ 300)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 100 Int64))
  (call main (: 3 Int64))
  (output (: 200 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "a guarded bin-match arm over a CONSTANT scrutinee folds the guard and selects the arm"
  (doc
    "The CONST-scrutinee companion of the guarded bin-match case: the scrutinee `(Bytes.of (list 7))`
           is a compile-time constant, so the bin matcher decodes each arm's segments at compile time AND
           folds the guard (the guard cond reads the decoded binder `n` via the same `BinField` the body
           sees — Case 6bg — which over a const scrutinee folds to a `ConstBool`). n = 7: the first guard
           `(> n 5)` folds TRUE → arm 1 (100). Pins that a guarded bin arm's guard is EVALUATED (not ignored)
           on the const path and that a TRUE fold selects the arm — the const analogue of the runtime
           fall-through case, exercising the const-path guard fold rather than the runtime `pred AND guard`.")
  (input
    (match
      (Bytes.of #list(7))
      ((guard (bin (u8 n)) (> n 5)) 100)
      ((guard (bin (u8 n)) (> n 0)) 200)
      (_ 300)))
  (output (: 100 Int64)))

(case
  "a guarded bin-match arm over a CONSTANT scrutinee whose guard fails falls to the next arm"
  (doc
    "The const-path guard FALL-THROUGH companion: over the constant `(Bytes.of (list 3))`, n = 3, so
           arm 1's guard `(> n 5)` folds FALSE and the matcher continues to arm 2, whose guard `(> n 0)`
           folds TRUE → 200. Pins that a FALSE guard fold on the const path advances to the next arm (not a
           trap, not a wrong-arm selection) — the const twin of the runtime fall-through, closing the
           const-path guard fold's false branch.")
  (input
    (match
      (Bytes.of #list(3))
      ((guard (bin (u8 n)) (> n 5)) 100)
      ((guard (bin (u8 n)) (> n 0)) 200)
      (_ 300)))
  (output (: 200 Int64)))

(case
  "a guarded bin arm with a CONST leading segment reads a binder from a LATER segment"
  (doc
    "The existing guarded-bin cases all read a binder from a SINGLE-segment `(bin (u8 n))`. This pins
           the LITERAL-TAG-THEN-BINDER shape: `(guard (bin (u8 5) (u8 n)) (> n 3))` — a CONST leading
           segment `(u8 5)` (a literal probe, not a binder) followed by a binder segment `(u8 n)`, over a
           RUNTIME bin whose first byte is 5. The literal segment must PROBE (match only when byte 0 == 5)
           while `n` decodes from the SECOND segment and is in scope for BOTH the guard cond `(> n 3)` AND
           the body — the bin analogue of the partial-const guarded-map arm (a const-vs-runtime mix must not
           disturb the later segment's binder resolution). `main 9` calls `f` on `bin[5, 9]`: byte 0 == 5
           probes true, n=9, guard `9 > 3` holds → body reads n = 9; `main 0` calls `f` on the `(bin (u8 9)
           (u8 9))` witness whose first byte != 5, so the literal probe fails → falls to the wildcard → -1.")
  (input
    (do
      (def (f (: b Bytes)) (match b ((guard (bin (u8 5) (u8 n)) (> n 3)) n) (_ -1)))
      (def
        (main (: v Int64))
        (if (> v 0) (f (bin (u8 5) (u8 (UInt8.wrap v)))) (f (bin (u8 9) (u8 9)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

(case
  "a bin-arm guard cond that traps at the decoded binder TRAPS the match — it does not fall through"
  (doc
    "The TRAP face of bin-guard evaluation, closing the guard-outcome triple (true → select, false →
           fall through, TRAP → trap): the guard `(> n (/ 12 (- n 9)))` divides by `(- n 9)`, which is 0
           exactly when the decoded second byte n = 9. A guard is an EVALUATED expression, not a refutable
           probe — its trap is observed, so the match must TRAP at n = 9, not treat the failure as a
           guard-miss and fall to the `(* 100 m)` arm (which would yield 900) or the wildcard. k = 0 →
           bytes [5, 9] → n = 9 → the guard divides by zero → trap; k = 3 → bytes [5, 12] → guard
           `12 > 12/3 = 4` holds → the arm body reads n = 12. The bin analogue of the scalar
           observed-trap rule (a demanded trapping computation fires; only a DISCARDED one is elided).")
  (input
    (do
      (def
        (main (: k UInt8))
        (match
          (Bytes.of #list((UInt8.wrap 5) (UInt8.wrap (+ 9 k))))
          ((guard (bin (u8 5) (u8 n)) (> n (/ 12 (- n 9)))) n)
          ((bin (u8 5) (u8 m)) (* 100 m))
          (_ -1)))
      (export main)))
  (call main (: 0 UInt8))
  (trap "division by zero")
  (call main (: 3 UInt8))
  (output (: 12 Int64)))

(case
  "a failed multi-segment bin guard falls through and the NEXT arm re-decodes its own binders"
  (doc
    "The MULTI-SEGMENT fall-through companion: three arms probe the same tag `(u8 5)` and decode TWO
           binder segments each under different guards. A failed guard must fall through with the next
           arm's binders decoding cleanly from the same materialized scrutinee — distinct binder NAMES per
           arm (n/p, a/b, x/y) pin that each arm's decode is its own scope, not a reuse of the failed
           arm's slots. k = 60 → bytes [5, 60, 61]: arm 1's guard `60 > 50` holds → n + p = 121. k = 7 →
           bytes [5, 7, 8]: arm 1 fails (7 ≤ 50), arm 2's guard `8 > 7` holds → 10·(7+8) = 150 (the
           unguarded arm 3, 100·15 = 1500, must NOT be reached).")
  (input
    (do
      (def
        (main (: k UInt8))
        (match
          (Bytes.of #list((UInt8.wrap 5) (UInt8.wrap k) (UInt8.wrap (+ k 1))))
          ((guard (bin (u8 5) (u8 n) (u8 p)) (> n 50)) (+ n p))
          ((guard (bin (u8 5) (u8 a) (u8 b)) (> b a)) (* 10 (+ a b)))
          ((bin (u8 5) (u8 x) (u8 y)) (* 100 (+ x y)))
          (_ -1)))
      (export main)))
  (call main (: 60 UInt8))
  (output (: 121 Int64))
  (call main (: 7 UInt8))
  (output (: 150 Int64)))

(case
  "a runtime bin match dispatches on a literal tag across arms"
  (doc
    "A multi-arm `bin` match over a RUNTIME scrutinee: a leading LITERAL tag segment selects the arm
           (tag 1 vs tag 2), and a runtime `u16` field fills the payload. The construction takes a `UInt8`
           tag and a `UInt16` field (the segments' width types). Built with tag 2, so the second arm fires:
           `y = 300`, `y + 1000 = 1300`. Pins tag-then-field dispatch across arms at run time — the shape a
           tagged binary format's parser takes.")
  (input
    (do
      (def
        (main (: t UInt8) (: v UInt16))
        (match
          (bin (u8 t) (u16 v))
          ((bin (u8 1) (u16 x)) x)
          ((bin (u8 2) (u16 y)) (+ y 1000))
          (_ -1)))
      (export main)))
  (call main (: 2 UInt8) (: 300 UInt16))
  (output (: 1300 Int64))
  ; The FIRST-arm hit: tag 1 selects the `x` arm, returning the raw field (300).
  (call main (: 1 UInt8) (: 300 UInt16))
  (output (: 300 Int64))
  ; The MISS→DEFAULT fallthrough: tag 9 matches neither literal-tag arm, so the runtime if-chain falls
  ; through every arm predicate to the `_` catch-all (-1) — the tail of the per-arm nested `if`, NOT a trap.
  (call main (: 9 UInt8) (: 300 UInt16))
  (output (: -1 Int64)))

(case
  "a runtime bin match whose catch-all BINDS and reads the whole Bytes scrutinee"
  (doc
    "The dispatch-or-handle-the-raw-bytes idiom: a runtime `bin` match whose fall-through arm is not a
           `_` discard nor a scalar binder but a NAME that binds the WHOLE Bytes scrutinee and reads it. `(bin
           (u8 1) (u8 x) (u8 y)) => x+y` handles the tag-1 shape; the catch-all `whole => Bytes.len whole`
           binds the entire scrutinee and returns its length. Over `[h, 7, 8]` from a runtime `h`: h=1 → the
           bin arm fires → 7+8 = 15; h=9 → no bin arm matches, so `whole` binds the full 3-byte scrutinee →
           `Bytes.len whole` = 3. Pins that a bin-match catch-all may BIND the whole scrutinee (a Bytes value
           read in the arm body), not only discard it — the materialized scrutinee flows to the binder, the
           real fallback shape a parser uses to keep the unrecognized bytes.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
          ((bin (u8 1) (u8 x) (u8 y)) (+ x y))
          (whole (Bytes.len whole))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 15 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64)))

(case
  "a runtime bin match decodes a Bytes.slice sub-range of a larger buffer"
  (doc
    "The parse-a-slice-of-a-buffer idiom: the bin-match scrutinee is itself a runtime `Bytes.slice`
           sub-range of a bigger buffer (an `Option`, so `Some sub` unwraps it), then `(bin (u8 a) (u8 b))`
           decodes the two bytes of the slice. `Bytes.slice([99, h, 7], 1, 2)` = the 2-byte window `[h, 7]`
           (skipping the leading 99), and the bin match reads `a=h`, `b=7`. Over a runtime `h`: h=5 → the slice
           is `[5, 7]` → a+b = 12; h=0 → `[0, 7]` → 7. Pins that a bin decode composes over a RUNTIME
           `Bytes.slice` result (a fresh Bytes handle from the O(1) slice op), not only over a top-level
           scrutinee — the shape a framed-buffer parser takes when it decodes a windowed region.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.slice (Bytes.of #list((UInt8.wrap 99) (UInt8.wrap h) (UInt8.wrap 7))) 1 2)
          ((Some sub) (match sub ((bin (u8 a) (u8 b)) (+ a b)) (_ -1)))
          ((None) -2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a bin tag-dispatch over a RUNTIME-START slice reads the re-based window per call"
  (doc
    "The slice-decode case above windows at a CONSTANT start (runtime content); here the START is
           the boundary parameter, so one compiled `(bin (u8 1) (u8 v))` dispatch reads a differently-
           BASED window per call: a=1 slices `[1, 42]` out of `[9, 1, 42, 7]` — the literal tag 1 matches
           and v=42; a=0 slices `[9, 1]` — tag 9 misses the literal arm → -1. A view that baked the
           offset (or always decoded from the parent's byte 0) would answer both calls identically. The
           bin-dispatch composition of the runtime-start re-basing pin in 10-bytes.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 1 42 7)) a 2)
          ((Some s) (match s ((bin (u8 1) (u8 v)) v) (_ -1)))
          ((None u) -2)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a rest-binder over a runtime-start slice measures the SLICE length, not the parent's"
  (doc
    "The rest-arm bounds face of the sliced scrutinee: `(bin (u8 1) (bytes rest))` over a 3-byte
           window of a 6-byte buffer — at a=0 the window is `[1,2,3]`, the tag matches, and `rest` is the
           2 remaining SLICE bytes (not the parent's 5 remaining). At a=2 the window `[3,4,5]` misses the
           tag → -1. Pins that the unsized final segment is bounded by the VIEW's extent — a rest that
           ran to the parent buffer's end would measure 5 and over-read past the window.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(1 2 3 4 5 6)) a 3)
          ((Some s) (match s ((bin (u8 1) (bytes rest)) (Bytes.len rest)) (_ -1)))
          ((None u) -2)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a dependent-size body over a runtime-start slice re-bases its size arithmetic per call"
  (doc
    "The size-arithmetic face of the runtime-start slice, completing the axis beside tag-dispatch and
           the rest-binder above: `(bin (u8 n) (bytes body n))` reads a leading length then binds `body` to
           the next n SLICE bytes. At a=1 the window is `[2,55,66]`, n=2, so `body` is the 2 bytes `[55,66]`
           → len 2. At a=0 the window is `[9,2,55]`, n=9 demands 9 bytes but only 2 remain IN THE VIEW → the
           dependent segment fails its bounds check → non-match → -1. Pins that the `off = slice_start +
           prior + read(n)` offset is computed against the VIEW's base and length, not the parent buffer's —
           a size arm that read `n` from the parent's byte 0 (or bounded n against the parent's remaining 4)
           would decode a different window or spuriously match at a=0.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 2 55 66 7)) a 3)
          ((Some s) (match s ((bin (u8 n) (bytes body n)) (Bytes.len body)) (_ -1)))
          ((None u) -2)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a NESTED runtime bin match re-parses a bin-decoded payload"
  (doc
    "The recursive / chunked-parser shape: an outer `(bin (u8 n) (bytes body n))` decodes a
           length-prefixed payload, binding `body` to the `n`-byte sub-Bytes, and the arm body then runs a
           SECOND bin match over that bound `body`. Against `[2, h, 7]` from a runtime `h`: the outer reads
           n=2 and binds `body = [h, 7]`; the inner `(bin (u8 x) (u8 y))` decodes it → `x=h`, `y=7`, returning
           `x*10 + y`. h=5 → body `[5,7]` → 57; h=0 → `[0,7]` → 7. Pins that a bin-decoded `bytes` binder is a
           first-class Bytes value that can itself be the scrutinee of a nested bin match (the outer decode's
           `BinSizedRead` slice flows into the inner match's materialized scrutinee) — the layered-frame parser
           idiom, decode a container then decode its contents.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap 2) (UInt8.wrap h) (UInt8.wrap 7)))
          ((bin (u8 n) (bytes body n)) (match body ((bin (u8 x) (u8 y)) (+ (* x 10) y)) (_ -1)))
          (_ -2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 57 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a recursive parser drains a STREAM of length-prefixed frames to the empty base case"
  (doc
    "The chunked-parser pin above decodes ONE nested frame; this pins the STREAM idiom — a
           dependent-size segment AND a rest-binder in one pattern, `(bin (u8 n) (bytes body n)
           (bytes tail))`, with the parser recursing on `tail` until the `(bin)` empty base fires.
           The accumulator weights each frame's byte-sum by its 1-based INDEX, so frame BOUNDARIES
           matter: a mis-split that moves a byte between frames changes the weighted sum even when
           the plain sum is preserved. (A dependent size field must be an earlier segment of the
           SAME bin — splitting the decode across nested matches declines.) mode 1: frames
           [10],[20 30],[1·40] → 10·1+50·2+40·3 = 230; mode 2: one 3-byte frame → 18·1 = 18;
           mode 3: EMPTY stream → base case → 0.")
  (input
    (do
      (def
        (sum-bytes (: b Bytes) (: i Int64) (: acc Int64))
        (if
          (>= i (Bytes.len b))
          acc
          (match (Bytes.at b i) ((Some v) (sum-bytes b (+ i 1) (+ acc v))) ((None _u) -1))))
      (def
        (drain (: s Bytes) (: idx Int64) (: acc Int64))
        (match
          s
          ((bin) acc)
          ((bin (u8 n) (bytes body n) (bytes tail))
            (drain tail (+ idx 1) (+ acc (* (sum-bytes body 0 0) idx))))
          (_ -3)))
      (def
        (main (: mode Int64))
        (do
          (def
            s
            (if
              (= mode 1)
              (Bytes.of #list(1 10 2 20 30 1 40))
              (if (= mode 2) (Bytes.of #list(3 5 6 7)) (Bytes.of #list()))))
          (drain s 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 230 Int64))
  (call main (: 2 Int64))
  (output (: 18 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a reframed packet with a TRANSFORMED header compares byte-equal to its independent twin"
  (doc
    "The transcoder pin further down re-encodes the SAME decoded n and checks only Bytes.len;
           this pins the TRANSFORM: `(bin (u8 (UInt8.wrap (+ n 1))) (u8 255) (bytes body))` —
           arithmetic on a decoded field re-entering a width-typed segment through an EXPLICIT wrap
           (the +1 widens past UInt8, so the narrow is the caller's job per the fit rule), a literal
           sentinel byte, and the body splice. The output is compared BYTE-EQUAL to an independently
           constructed twin — and the mode-2 NEGATIVE face (twin differs in the last byte → 0)
           witnesses that the equality discriminates, so a wrong-content reframe with the right
           length cannot pass. mode 1 → 4·10+1 = 41; mode 2 → 40.")
  (input
    (do
      (def
        (reframe (: b Bytes))
        (match
          b
          ((bin (u8 n) (bytes body n)) (bin (u8 (UInt8.wrap (+ n 1))) (u8 255) (bytes body)))
          (_ (bin))))
      (def
        (main (: mode Int64))
        (do
          (def out (reframe (Bytes.of #list(2 20 30))))
          (def expected (if (= mode 1) (Bytes.of #list(3 255 20 30)) (Bytes.of #list(3 255 20 31))))
          (+ (* (Bytes.len out) 10) (if (= out expected) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 41 Int64))
  (call main (: 2 Int64))
  (output (: 40 Int64)))

(case
  "a three-arm runtime literal-tag dispatch hits the middle and last arms and misses past all three"
  (doc
    "The two-arm dispatch above can't distinguish a genuine PER-ARM fall-through chain from a
           two-way branch; three literal-tag arms witness the chain at depth: tag 2 falls past arm 1's
           predicate and hits the MIDDLE arm (y + 1000 = 1300), tag 3 falls past two predicates to the
           LAST literal arm (z + 2000 = 2300), and tag 9 falls past ALL THREE to the catch-all (-1). A
           dispatch that compiled the arms to a two-way test (or a jump table missing the fall-through
           tail) diverges at one of the three calls. Expected: 1300, 2300, -1.")
  (input
    (do
      (def
        (main (: t UInt8) (: v UInt16))
        (match
          (bin (u8 t) (u16 v))
          ((bin (u8 1) (u16 x)) x)
          ((bin (u8 2) (u16 y)) (+ y 1000))
          ((bin (u8 3) (u16 z)) (+ z 2000))
          (_ -1)))
      (export main)))
  (call main (: 2 UInt8) (: 300 UInt16))
  (output (: 1300 Int64))
  (call main (: 3 UInt8) (: 300 UInt16))
  (output (: 2300 Int64))
  (call main (: 9 UInt8) (: 300 UInt16))
  (output (: -1 Int64)))

(case
  "a missed literal tag falls to a BINDER bin arm that decodes the scrutinee"
  (doc
    "The default arm is a BINDING bin pattern, not `_`: `((bin (u8 other) (u8 b)) (+ other b))` —
           a missed tag (5) falls through the literal arm and the binder arm DECODES the whole scrutinee
           (5 + 7 = 12), while a hit (1) takes the literal arm (100). Pins that the fall-through target
           can itself be a decoding bin pattern (the parser idiom: known opcodes special-cased, everything
           else decoded generically) — the catch-all need not discard the bytes. Expected: 100, 12.")
  (input
    (do
      (def
        (main (: t UInt8))
        (match
          (bin (u8 t) (u8 7))
          ((bin (u8 1) (u8 a)) 100)
          ((bin (u8 other) (u8 b)) (+ other b))
          (_ -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100 Int64))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "a VARIABLE catch-all arm binds the whole runtime scrutinee as a Bytes value"
  (doc
    "The third fall-through shape, completing the family: not a discarding `_` and not a decoding
           bin pattern, but a plain VARIABLE arm — `(rest (Bytes.len rest))` — which binds the entire
           unmatched scrutinee as an ordinary Bytes value. tag=1 hits the literal arm (42); tag=9 misses
           and the variable arm receives the whole two-byte frame → `Bytes.len` = 2. Pins that the
           scrutinee flows into the fall-through arm as a first-class Bytes (usable by any Bytes op, not
           only re-matched or discarded) — this shape used to reject CDZ0203 (the variable arm's binder
           didn't unify with the Bytes scrutinee) while its `_` and decoding siblings compiled.")
  (input
    (do
      (def
        (main (: tag Int64))
        (match (bin (u8 (UInt8.wrap tag)) (u8 42)) ((bin (u8 1) (u8 v)) v) (rest (Bytes.len rest))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 9 Int64))
  (output (: 2 Int64)))

(case
  "a runtime bytes value is spliced into a length-prefixed frame"
  (doc
    "`(bin (u16 3) (bytes b))` with `b` a RUNTIME `Bytes` value splices `b` after a two-byte header,
           building the frame at run time (a `bytes-concat` of the emitted header and the runtime body).
           `mk` builds a three-byte body from a runtime `n`; the frame's length is 2 (header) + 3 (body) =
           5. Pins the length-prefixed-frame builder — a fixed header composed with a runtime-length body,
           the construction companion of the dependent-size MATCH.")
  (input
    (do
      (def (frame (: b Bytes)) (Bytes.len (bin (u16 3) (bytes b))))
      (def (mk (: n Int64)) (Bytes.of #list((UInt8.wrap n) 20 30)))
      (def (main (: n Int64)) (frame (mk n)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 5 Int64)))

(case
  "a frame body ACCUMULATED by a fold of per-element bin constructions round-trips"
  (doc
    "The serializer-loop idiom: `build` folds n per-element `(bin (u8 (UInt8.wrap i)))` one-byte
           constructions into one body via Bytes.concat, then a header frame wraps it and the pattern
           decodes both back — length byte 5, rest 5 bytes (505). Each loop iteration runs a FRESH bin
           construction whose result concatenates onto the accumulator (the splice pin above splices ONE
           pre-built value); a construction that reused a buffer across iterations, or a concat that
           dropped an iteration's byte, drifts the rest length.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (>= i n) acc (build (+ i 1) n (Bytes.concat acc (bin (u8 (UInt8.wrap i)))))))
      (def
        (main (: n Int64))
        (let
          ((body (build 0 n (Bytes.of #list()))))
          (match
            (bin (u8 (UInt8.wrap n)) (bytes body))
            ((bin (u8 len) (bytes rest)) (+ (* 100 len) (Bytes.len rest)))
            (_ -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 505 Int64)))

(case
  "a runtime bin match binds the tail after a fixed header via a final rest segment"
  (doc
    "A `(bin …)` pattern ending in a FINAL UNSIZED `(bytes rest)` over a RUNTIME scrutinee: a fixed
           one-byte header then a variable-length tail. The length probe accepts any length `>= 1` (the
           header, not an exact width), the header binds via a fixed-offset read, and the tail binds as
           `bytes-slice(scrutinee, 1, len - 1)`. Built from a runtime tag with a three-byte payload, so
           `rest` is those three bytes and `Bytes.len rest = 3`. Pins the header-plus-rest parser shape —
           a tag followed by an opaque remainder — over a runtime value.")
  (input
    (do
      (def
        (main (: n UInt8))
        (let
          ((payload (Bytes.of #list(1 2 3))))
          (match (bin (u8 n) (bytes payload)) ((bin (u8 t) (bytes rest)) (Bytes.len rest)) (_ -9))))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 3 Int64)))

(case
  "a runtime final rest segment binds the empty tail when the scrutinee is only the header"
  (doc
    "The final `(bytes rest)` binds an EMPTY tail when the runtime scrutinee is exactly the fixed
           header: `bytes-len >= 1` still holds (the header is present), and the tail slice is `[1, 0)` —
           an empty Bytes, so `Bytes.len rest = 0`. Pins that a rest segment absorbs zero remaining bytes
           without trapping (the degenerate case of the header-plus-rest parser).")
  (input
    (do
      (def
        (main (: n UInt8))
        (let
          ((payload (Bytes.of #list())))
          (match (bin (u8 n) (bytes payload)) ((bin (u8 t) (bytes rest)) (Bytes.len rest)) (_ -9))))
      (export main)))
  (call main (: 7 UInt8))
  (output (: 0 Int64)))

(case
  "a runtime bin match feeds its decoded segment binders into a sum+tuple constructor, then projects back"
  (doc
    "The decode-then-BUILD idiom: a `(bin (u8 a) (u8 b))` match over a RUNTIME scrutinee binds two
           integer segments, then the arm body uses those binders as CONSTRUCTOR ARGUMENTS to assemble a
           structured value `(Some (P a b))` (an `(Option Pair)`) and projects a field back out — the shape a
           real parser takes: read fields, package them into a domain type, then read the packaged value. Against
           a two-byte frame `[h, 7]` from a runtime `h` (so the bin cannot fold): h=9 builds `Some (P 9 7)`,
           then the outer match unwraps `P a2 b2` and returns `a2*256 + b2` = 9*256+7 = 2311. Pins that a
           bin-decode binder composes as a constructor argument into a sum-of-tuple built at run time (the
           decoded `a`/`b` flow through `BinField` reads into the `SumNew`/tuple payload) AND is read back
           out — a full decode→construct→destructure round-trip, projected to a scalar so both backends agree.")
  (input
    (do
      (type Pair (P Int64 Int64))
      (def
        (main (: h Int64))
        (match
          (match
            (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7)))
            ((bin (u8 a) (u8 b)) (Some (P a b)))
            (_ (None)))
          ((Some (P a2 b2)) (+ (* a2 256) b2))
          ((None) -1)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 2311 Int64))
  (live-objects 0))

(case
  "a runtime bin match binds a dependent-size bytes segment over a runtime scrutinee"
  (doc
    "The length-prefixed-frame parse (the dependent-size crown jewel) over a RUNTIME scrutinee: a
           `(bin (u8 n) (bytes payload n))` pattern reads a one-byte header `n` then binds EXACTLY `n`
           payload bytes. The arm's length probe is `bytes-len == fixed_prefix(1) + n` — `n` read at runtime
           via a fixed-offset int read — and the payload binds as `bytes-slice(scrutinee, 1, n)`. The
           scrutinee is a three-byte frame `[h, 7, 8]` built from a RUNTIME header `h` (so the bin cannot
           fold), and the arm returns the payload length. h=2 → the frame is exactly prefix(1)+2 = 3 bytes,
           so the two payload bytes bind → `Bytes.len payload = 2`. Pins runtime dependent-size decoding —
           read a size, then that many bytes — the wasm companion of the constant dependent-size cases
           above (was declined on wasm, computed on rust; now lowered via a sized bytes-slice).")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
          ((bin (u8 n) (bytes payload n)) (Bytes.len payload))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64)))

(case
  "a runtime dependent-size match reads a MULTI-BYTE (u16) length prefix"
  (doc
    "The multi-byte-prefix companion of the runtime dependent-size case above: the length is a `u16`
           (two bytes, big-endian), not a `u8`, so the dependent `(bytes body n)` starts at the STATIC offset
           2 and its size `n` is a two-byte read. `(bin (u16 n) (bytes body n))` over a runtime four-byte frame
           `[0, h, 7, 8]` — the u16 reads `n = h` (high byte 0), then binds `n` body bytes. h=2 → frame is
           exactly prefix(2)+2 = 4 bytes, so the two body bytes bind → `Bytes.len body = 2`; h=1 → the u16
           reads 1 but two bytes remain after the prefix, so the arm does not consume the whole scrutinee and
           falls through → -1. Pins that the length prefix feeding a dependent size may be WIDER than one byte
           (the u16 decode at offset 0 feeds the slice at the static offset 2) — the u16-framed-message shape,
           the runtime companion of the constant u16-length cases.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap 0) (UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
          ((bin (u16 n) (bytes body n)) (Bytes.len body))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64)))

(case
  "a runtime literal-tag probe gates a dependent-size body in one arm"
  (doc
    "The chunked-format shape at RUNTIME: a LITERAL magic/tag segment must MATCH before a dependent-size
           body binds — `(bin (u8 137) (u8 n) (bytes body n))`. The arm's predicate ANDs the literal-equality
           probe (byte 0 == 137) with the dependent length probe (`bytes-len == prefix(2) + n`), all at run
           time. Over `(Bytes.of (list 137 h 7 8))`: h=2 → tag 137 matches AND n=2 sizes the two body bytes →
           `Bytes.len body = 2`; h=1 → tag matches but n=1 leaves a trailing byte unconsumed → the arm does not
           match the whole scrutinee → falls through → -1. Pins that a runtime literal-segment probe COMPOSES
           with a runtime dependent-size read in a single arm (the const `magic + length-prefixed chunk` case
           above, but with the magic gate and the size read both evaluated at run time), the tag-length-value
           parser shape.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap 137) (UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
          ((bin (u8 137) (u8 n) (bytes body n)) (Bytes.len body))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64)))

(case
  "a runtime dependent-size match falls through when the scrutinee is too short for the size prefix"
  (doc
    "The truncated-frame boundary the dependent-size length probe must GUARD: a RUNTIME scrutinee too
           short to even READ the size prefix must FALL THROUGH, not trap. The arm `(bin (u8 a) (u8 n) (bytes
           payload n))` needs a TWO-byte fixed prefix (a header `a` then the size `n`), but the scrutinee is
           ONE byte. The exact length probe is `bytes-len == prefix(2) + n`, and reading `n` is a fixed-offset
           read at byte 1 — which OVERRUNS a one-byte scrutinee. The probe must be FLOORED (`bytes-len >=
           prefix` short-circuits BEFORE the `n`-read) so a too-short scrutinee is a non-match, matching the
           const-fold reference (`bin_match_decode` returns None at every overrun). The scrutinee is built
           from a runtime `h` so the match cannot fold. h=9 → a one-byte frame `[9]` → too short for the
           2-byte prefix → falls through to -1. Earlier this TRAPPED (wasm `unreachable`): the length probe's
           `n`-read was an unconditional outermost compare operand with no length floor (reviewer finding
           2026-07-18, corpus-bugfix — the literal-segment probes were short-circuited but the dependent-size
           read was not).")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h)))
          ((bin (u8 a) (u8 n) (bytes payload n)) (+ a (Bytes.len payload)))
          (_ -1)))
      (export main)))
  (call main (: 9 Int64))
  (output (: -1 Int64)))

(case
  "a runtime dependent-size match with a signed negative size falls through"
  (doc
    "The signed-size soundness guard: a SIGNED size segment `(i8 n)` whose byte reads NEGATIVE must
           FALL THROUGH, not spuriously match or trap. The const path filters `n >= 0`; the runtime path adds
           the same `n >= 0` guard in the length probe. Here `(bin (i8 n) (bytes payload n))` over a runtime
           two-byte scrutinee whose size byte is 0xFF = -1 (as i8): `prefix(1) + (-1) = 0`, which could
           spuriously satisfy `bytes-len == 0` on an empty tail, or drive a negative slice. The `n >= 0`
           guard makes it a clean non-match → -1. Built from a runtime `h` so it cannot fold. h=255 → size
           byte 0xFF → n = -1 → falls through. Pins the runtime negative-size guard mirrors the const path's
           `filter(|v| *v >= 0)` (reviewer finding 2026-07-18).")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 20)))
          ((bin (i8 n) (bytes payload n)) (Bytes.len payload))
          (_ -1)))
      (export main)))
  (call main (: 255 Int64))
  (output (: -1 Int64)))

(case
  "a runtime bin match with a NON-FINAL dependent-size segment binds the body then the rest"
  (doc
    "The non-final dependent-size shape over a RUNTIME scrutinee — a `(bytes body n)` FOLLOWED by more
           segments, so the cursor offset goes dynamic (`static_base + n`) for everything after it. The
           pattern `(bin (u8 n) (bytes body n) (bytes rest))` reads a one-byte size `n`, binds EXACTLY `n`
           payload bytes to `body`, then binds the remainder to `rest`. Against a four-byte runtime frame
           `[h, 7, 8, 9]` with h=2: `n = 2` → `body = [7, 8]` (len 2), `rest = [9]` (len 1), so the arm
           returns `Bytes.len body + Bytes.len rest = 3`. The CONSTANT-scrutinee twin of this shape already
           passes (the dependent-size crown-jewel cases above use a `Bytes.of` const, so the const evaluator
           does the slicing); this is its RUNTIME companion. The runtime lowering today admits only a SINGLE
           final variable-length segment (a `(bytes rest)` OR a `(bytes body n)`) and DECLINES a non-final
           dependent-size segment cleanly (lower.rs 'non-final variable-length segment is not yet lowered'),
           so the gate scores this TODO — it is a well-formed form (distinct from the permanent CDZ0220
           non-final UNSIZED `(bytes b)`), pinned here so the §4a dynamic-offset lowering flips it to PASS.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8) (UInt8.wrap 9)))
          ((bin (u8 n) (bytes body n) (bytes rest)) (+ (Bytes.len body) (Bytes.len rest)))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 3 Int64)))

(case
  "a runtime non-final dependent-size body of zero binds empty and rest absorbs everything"
  (doc
    "The §4a non-final shape with a ZERO runtime size: `(bin (u8 n) (bytes body n) (bytes rest))`
           against `[0, 7, 8, 9]` (h=0) reads `n = 0` → `body = []` (len 0) and `rest = [7, 8, 9]` (len 3),
           so `Bytes.len body + Bytes.len rest = 3`. Pins that a zero dependent size at a non-final position
           binds an empty body and the following segment reads at the SAME dynamic offset (`total + 0`) — the
           runtime companion of the constant zero-size case, exercising the `off_plus`-is-zero dynamic path.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8) (UInt8.wrap 9)))
          ((bin (u8 n) (bytes body n) (bytes rest)) (+ (Bytes.len body) (Bytes.len rest)))
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

(case
  "a runtime non-final dependent-size body that overruns the frame falls through"
  (doc
    "The §4a non-final shape where the runtime size OVERRUNS: `(bin (u8 n) (bytes body n) (bytes rest))`
           against the four-byte `[5, 7, 8, 9]` (h=5) reads `n = 5`, but only three bytes follow the size
           byte, so `total(1) + n(5) = 6 > bytes-len(4)` → the arm does NOT match and control falls to the
           catch-all, yielding -1. Pins that the FLOOR + length predicate (`bytes-len >= total + n`) guards
           the dynamic-offset payload/rest reads: a size larger than the remainder is a non-match, never a
           trap or an out-of-bounds read (the runtime companion of the const-path overrun case above).")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8) (UInt8.wrap 9)))
          ((bin (u8 n) (bytes body n) (bytes rest)) (+ (Bytes.len body) (Bytes.len rest)))
          (_ -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: -1 Int64)))

(case
  "a runtime literal int probe reads at a dynamic offset after a dependent-size body"
  (doc
    "A LITERAL fixed-width segment AFTER a non-final dependent-size body — its byte offset is dynamic
           (`static_base + n`). `(bin (u8 n) (bytes body n) (u8 99))` against `[2, 7, 8, 99]` (h=2) reads
           `n = 2` → `body = [7, 8]`, then probes the byte at offset `1 + n = 3` against the literal 99 →
           equal, so the arm matches and returns `Bytes.len body = 2`. Pins that a literal-int PROBE (not
           just a binder) reads at the §4a dynamic offset, and that the length predicate (`bytes-len == 1 + n
           + 1`, i.e. exact 4) is ANDed before the probe so the read stays in bounds. A frame whose tag byte
           is not 99 would fall through to -1.")
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8) (UInt8.wrap 99)))
          ((bin (u8 n) (bytes body n) (u8 99)) (Bytes.len body))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64)))

(case
  "a runtime bit-field packs a runtime value into a nibble"
  (doc
    "`(bin (bits ((UInt 4).wrap n) 4) (bits 5 4))` with a RUNTIME `n` packs the low nibble of `n`
           into the HIGH nibble and the constant 5 into the low nibble of one byte (most-significant field
           first). A `(bits v 4)` field takes a `(UInt 4)`, so the caller narrows `n` with `(UInt 4).wrap`
           (truncating to the low four bits) BEFORE the segment — the segment then provably fits its value
           and never traps. n=10 → (10<<4)|5 = 0xA5 = 165. Reads byte 0 back. Pins runtime bit-field
           packing — the companion of the constant `(bits 1 1)(bits 2 3)(bits 5 4)` case over a runtime
           value, with the caller responsible for narrowing to the field width.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Bytes.at (bin (bits ((. (UInt 4) wrap) n) 4) (bits 5 4)) 0)
          ((Some b) b)
          ((None _) -1)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 165 Int64)))

(case
  "runtime-packed nibbles MATCH back out through bits patterns - the pack-unpack identity"
  (doc
    "The runtime bit-field pins construct and read a raw byte; this round-trips through bits
           PATTERNS — two runtime nibbles packed then matched back out: construction and destructuring
           must agree on field order/offsets (a mismatch swaps a and b). Boundary nibbles 0/15.")
  (input
    (do
      (def
        (main (: v Int64) (: w Int64))
        (match
          (bin (bits ((. (UInt 4) wrap) v) 4) (bits ((. (UInt 4) wrap) w) 4))
          ((bin (bits a 4) (bits b 4)) (+ (* a 100) b))
          (_ -1)))
      (export main)))
  (call main (: 10 Int64) (: 5 Int64))
  (output (: 1005 Int64))
  (call main (: 0 Int64) (: 15 Int64))
  (output (: 15 Int64)))

(case
  "a nibble OPCODE dispatches arms of different shapes with a byte-aligned dependent size"
  (doc
    "The single-tag probe and the bit-field-sized bytes exist separately; this dispatches a
           nibble OPCODE across arms of DIFFERENT residual shapes — arm 1 with a byte-aligned
           (bits n 8)(bytes body n) dependent read, arm 2 a single-byte flags frame, + miss. (A
           dependent size sourced from a NON-byte-aligned bit-field over a runtime scrutinee is the
           documented dynamically-offset not-yet.)")
  (input
    (do
      (def
        (sum-bytes (: b Bytes) (: i Int64) (: acc Int64))
        (if
          (>= i (Bytes.len b))
          acc
          (match (Bytes.at b i) ((Some v) (sum-bytes b (+ i 1) (+ acc v))) ((None _u) -1))))
      (def
        (parse (: b Bytes))
        (match
          b
          ((bin (bits 1 4) (bits _f 4) (bits n 8) (bytes body n)) (sum-bytes body 0 0))
          ((bin (bits 2 4) (bits f 4)) (* f 10))
          (_ -1)))
      (def
        (main (: mode Int64))
        (parse
          (if
            (= mode 1)
            (Bytes.of #list(16 2 5 6))
            (if (= mode 2) (Bytes.of #list(39)) (Bytes.of #list(153))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 2 Int64))
  (output (: 70 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a length-prefixed frame built FRESH from a runtime payload round-trips through its own parser"
  (doc
    "The reframe pins re-encode a DECODED n; this COMPUTES the header from the payload at
           build time ((u8 (wrap (Bytes.len body))) (bytes body)) then round-trips through its own
           parser with a rest-exhaustion check. EMPTY-payload face → the one-byte n=0 frame.")
  (input
    (do
      (def
        (sum-bytes (: b Bytes) (: i Int64) (: acc Int64))
        (if
          (>= i (Bytes.len b))
          acc
          (match (Bytes.at b i) ((Some v) (sum-bytes b (+ i 1) (+ acc v))) ((None _u) -1))))
      (def (frame (: body Bytes)) (bin (u8 (UInt8.wrap (Bytes.len body))) (bytes body)))
      (def
        (main (: mode Int64))
        (do
          (def body (if (= mode 1) (Bytes.of #list(5 6 7)) (Bytes.of #list())))
          (match
            (frame body)
            ((bin (u8 n) (bytes got n) (bytes rest))
              (if (= (Bytes.len rest) 0) (+ (* n 100) (sum-bytes got 0 0)) -2))
            (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 318 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a MIXED-endian frame packs big and little fields side by side and round-trips both"
  (doc
    "Per-SEGMENT byte-order independence: a BIG-endian u16 beside a LITTLE-endian u16 in one
           bin over RUNTIME values, both matched back — an endianness applied per-frame rather than
           per-segment corrupts one field. Boundary face 0xFFFF.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (match
          (bin (u16 (UInt16.wrap a)) (u16 (UInt16.wrap b) le))
          ((bin (u16 x) (u16 y le)) (+ x y))
          (_ -1)))
      (export main)))
  (call main (: 258 Int64) (: 772 Int64))
  (output (: 1030 Int64))
  (call main (: 0 Int64) (: 65535 Int64))
  (output (: 65535 Int64)))

(case
  "a RUNTIME negative signed field beside an unsigned one round-trips with independent sign handling"
  (doc
    "The SIGN twin of the mixed-endian pin: a runtime NEGATIVE (i8 (Int8.wrap v)) beside
           (u8 (wrap w)) — the i8 read must SIGN-EXTEND (-5 from byte 251) while the adjacent u8
           stays zero-extended. Boundary faces 127/255; the negative face also exercises a negative
           RESULT through the harness.")
  (input
    (do
      (def
        (main (: v Int64) (: w Int64))
        (match
          (bin (i8 (Int8.wrap v)) (u8 (UInt8.wrap w)))
          ((bin (i8 x) (u8 y)) (+ (* x 1000) y))
          (_ 1)))
      (export main)))
  (call main (: -5 Int64) (: 7 Int64))
  (output (: -4993 Int64))
  (call main (: 127 Int64) (: 255 Int64))
  (output (: 127255 Int64)))

(case
  "a runtime bit-field run spans two bytes and composes with an int segment"
  (doc
    "A runtime bit-field RUN that spans a byte boundary and is followed by a byte-aligned int
           segment: `(bits ((UInt 4).wrap n) 4) (bits 1 4) (u8 42)` packs the low nibble of `n` and the
           constant 1 into byte 0 = (n<<4)|1, then writes 42 as byte 1. The 4-bit field takes a `(UInt 4)`
           (the caller narrows `n` with `(UInt 4).wrap`); the trailing `(u8 42)` takes a `UInt8` (a bare
           literal grounds to it). n=3 → byte 1 = 42. Pins that a runtime bit-field run closes to a whole
           byte before the int segment (CDZ0220 byte-alignment) and the int byte follows immediately.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Bytes.at (bin (bits ((. (UInt 4) wrap) n) 4) (bits 1 4) (u8 42)) 1)
          ((Some b) b)
          ((None _) -1)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 42 Int64)))

(case
  "a runtime bin match decodes a byte-aligned bit-field run"
  (doc
    "A `(bin (bits a 3) (bits b 5))` pattern over a RUNTIME one-byte scrutinee decodes two sub-byte
           fields MSB-first: `a` is the high 3 bits, `b` the low 5. The runtime matcher reads the byte-
           aligned run as one big-endian integer then shifts+masks each field (`a = (byte >> 5) & 0x7`,
           `b = byte & 0x1F`), mirroring the const-fold `bin_match_decode`. The scrutinee is built from a
           RUNTIME header `h` so the match cannot fold. h=165 (0b1010_0101) → a=0b101=5, b=0b00101=5 →
           100*5+5 = 505. Pins runtime bit-field DECODING — the match companion of the runtime bit-field
           construction cases above (was declined on wasm; now lowered via a run-read + shift/mask).")
  (input
    (do
      (def
        (run (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h)))
          ((bin (bits a 3) (bits b 5)) (+ (* 100 a) b))
          (_ -1)))
      (export run)))
  (call run (: 165 Int64))
  (output (: 505 Int64)))

(case
  "a runtime bin match reads an int segment after a byte-aligned bit-field run"
  (doc
    "A byte-aligned bit-field run CLOSES to a whole byte, so a FOLLOWING fixed-width int segment
           reads at a STATIC byte offset — `(bin (bits a 3) (bits b 5) (u8 c))` decodes the run's two
           sub-byte fields from byte 0 then `c` from byte 1. Over a RUNTIME scrutinee `[h, 42]`: h=165 →
           a=5, b=5 (byte 0), c=42 (byte 1) → 100*5+5+42 = 547. Pins that the bit-field run's byte width
           advances the static offset of a trailing byte-aligned segment (a bitfield header + a byte
           payload — a common wire shape); earlier a segment after ANY bit-field declined outright.")
  (input
    (do
      (def
        (run (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 42)))
          ((bin (bits a 3) (bits b 5) (u8 c)) (+ (+ (* 100 a) b) c))
          (_ -1)))
      (export run)))
  (call run (: 165 Int64))
  (output (: 547 Int64)))

(case
  "a runtime dependent-size bytes segment sized by a bit-field field"
  (doc
    "The dependent-size `(bytes payload n)` may take its size `n` from a BIT-FIELD segment, not only a
           fixed int: `(bin (bits n 8) (bytes payload n))` reads `n` out of its byte-aligned bit-field run
           (run-read + shift/mask) then binds exactly `n` payload bytes. Over a RUNTIME `[h, 65, 66]`: h=2 →
           n=2 → payload is the two bytes → `Bytes.len payload` = 2. And the length floor still guards a
           truncated frame: `[h, 65]` with h=2 needs prefix(1)+2 = 3 bytes but has 2 → falls through to -1
           (the bit-field size read rides the same `bytes-len >= prefix` short-circuit + `n >= 0` guard as a
           fixed-int size). Pins that a bit-field is a valid dependent-size source; earlier only a fixed-int
           size segment was accepted.")
  (input
    (do
      (def
        (run (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 65) (UInt8.wrap 66)))
          ((bin (bits n 8) (bytes payload n)) (Bytes.len payload))
          (_ -1)))
      (export run)))
  (call run (: 2 Int64))
  (output (: 2 Int64)))

(case
  "a runtime bin match dispatches on a leading literal bit-field tag"
  (doc
    "A LITERAL bit-field segment is a probe: `(bin (bits 1 1) (bits x 7))` matches only a byte whose
           TOP bit is 1, binding `x` to the low 7 bits. The runtime predicate reads the run and shift/masks
           the tag field, ANDing `((byte >> 7) & 1) == 1` into the arm's length probe (short-circuited).
           Built from a runtime `h`: h=129 (0b1000_0001) → top bit 1 (match), x=0b0000001=1; h=1
           (0b0000_0001) → top bit 0 → falls through to -1. This case checks the MATCH; the next the miss.
           Pins sub-byte tag dispatch — a bitfield-tagged binary format's discriminator.")
  (input
    (do
      (def
        (run (: h Int64))
        (match (Bytes.of #list((UInt8.wrap h))) ((bin (bits 1 1) (bits x 7)) x) (_ -1)))
      (export run)))
  (call run (: 129 Int64))
  (output (: 1 Int64)))

(case
  "a runtime bit-field literal tag that does not match falls through"
  (doc
    "The miss companion: the same `(bin (bits 1 1) (bits x 7))` over a byte whose top bit is 0
           (h=1, 0b0000_0001) does NOT match the `(bits 1 1)` tag probe, so control falls to the catch-all
           → -1. Pins that a literal bit-field probe is a genuine equality gate (a non-matching tag is a
           non-match, not a bind), the sub-byte analogue of a literal int-segment tag miss.")
  (input
    (do
      (def
        (run (: h Int64))
        (match (Bytes.of #list((UInt8.wrap h))) ((bin (bits 1 1) (bits x 7)) x) (_ -1)))
      (export run)))
  (call run (: 1 Int64))
  (output (: -1 Int64)))

(case
  "a decoded field re-encodes into the same-width segment without an explicit narrow"
  (doc
    "The decode/encode dual is SYMMETRIC: a `(u16 m)` PATTERN binder decodes a field, and that same
           binder feeds a `(u16 m)` CONSTRUCTION directly — no `UInt16.wrap`/`UInt16.of` needed. The binder
           types as the segment's own width (`UInt16`), which is exactly what the re-encoding segment
           requires, so a parse-then-rebuild round-trip type-checks with no conversion. Here `main` decodes a
           runtime `(bin (u16 n))`, re-encodes the bound field, and reads byte 0 back — 258 = 0x0102 big-
           endian, byte 0 = 1. Pins that the width-typed construction contract does NOT break the natural
           decode→re-encode round-trip a binary transcoder is built from (a decoded field is already the
           right type for its own segment).")
  (input
    (do
      (def
        (main (: n UInt16))
        (match
          (bin (u16 n))
          ((bin (u16 m)) (match (Bytes.at (bin (u16 m)) 0) ((Some b) b) ((None _) -1)))
          (_ -9)))
      (export main)))
  (call main (: 258 UInt16))
  (output (: 1 Int64)))

(case
  "a multi-byte runtime bit-field takes the matching wide unsigned type"
  (doc
    "A `(bits v k)` field with `k` wider than a byte requires `v : (UInt k)` — the width type follows
           the field width, not a fixed byte. `(bin (bits n 16))` closes exactly two bytes from a runtime
           `(UInt 16)`; n=258 = 0x0102 big-endian, so byte 0 = 1. Pins that the width-typed contract extends
           to MULTI-BYTE bit-fields (the arbitrary-width `(UInt k)` requirement), not just sub-byte ones.")
  (input
    (do
      (def (main (: n UInt16)) (match (Bytes.at (bin (bits n 16)) 0) ((Some b) b) ((None _) -1)))
      (export main)))
  (call main (: 258 UInt16))
  (output (: 1 Int64)))

(case
  "a wrong-width value in a multi-byte bit-field is a compile-time type error"
  (doc
    "The width match is exact for multi-byte bit-fields too: a runtime `Int64` fed to a 16-bit field is
           a COMPILE-TIME type error (CDZ0203, the field wants `UInt16`), the multi-byte companion of the
           sub-byte `bits` case. Pins that a wide bit-field is not a loophole around the width-typed rule.")
  (input (do (def (main (: n Int64)) (Bytes.len (bin (bits n 16)))) (export main)))
  (error CDZ0203))

(case
  "a non-byte-aligned bit-field width takes its exact arbitrary-width unsigned type"
  (doc
    "The `(UInt k)` requirement holds for a NON-power-of-two, non-byte-aligned width too: a 12-bit
           field takes a `(UInt 12)`, closed to a whole byte by a trailing 4-bit constant (12+4 = 16 = two
           bytes, CDZ0220 byte-aligned). The caller narrows a runtime `Int64` with `(UInt 12).wrap`
           (truncating to the low 12 bits) — the arbitrary-width analogue of the `(UInt 4).wrap` nibble
           idiom. n=0x123 packs 0x123 into the high 12 bits then a zero nibble → bytes 0x12 0x30; byte 0 =
           0x12 = 18. Pins that an arbitrary bit width has a matching arbitrary-width unsigned type, and that
           `(UInt k).wrap` narrows to it, not a rounding to the nearest byte type.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Bytes.at (bin (bits ((. (UInt 12) wrap) n) 12) (bits 0 4)) 0)
          ((Some b) b)
          ((None _) -1)))
      (export main)))
  (call main (: 291 Int64))
  (output (: 18 Int64)))

(case
  "a decoded length field re-encodes a length-prefixed frame end to end"
  (doc
    "The full length-prefixed-frame TRANSCODER round-trip under the width-typed rule: a `(bin (u8 n)
           (bytes body n) (bytes rest))` pattern decodes a `u8` length `n`, binds exactly `n` body bytes
           (the dependent size), and the tail; then it RE-ENCODES `(bin (u8 n) (bytes body))` — the SAME
           decoded `n` serves as both the `(bytes body n)` dependent SIZE and the `(u8 n)` re-encode value,
           with NO explicit narrow (the decoded field already types as the header's `UInt8`). Built at run
           time from a `(list 2 10 20 99)` scrutinee (length 2, body [10 20], rest [99]); the re-encoded
           frame is `[2, 10, 20]` and its length is 3. Pins that a decoded length round-trips through both a
           dependent-size bind and a same-width re-encode — the core move a binary reframer/transcoder is
           built from — without breaking under the width-typed contract.")
  (input
    (do
      (def
        (reframe (: b Bytes))
        (match b ((bin (u8 n) (bytes body n) (bytes rest)) (bin (u8 n) (bytes body))) (_ (bin))))
      (def (main (: k Int64)) (Bytes.len (reframe (Bytes.of #list(2 10 20 99)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

(case
  "a big-endian runtime construction read back through an le pattern crosses the byte order"
  (doc
    "The CROSS-order witness the le ROUND-TRIP cases cannot provide: every runtime `le` case above
           uses `le` on BOTH sides, so an implementation that ignored `le` symmetrically (construct AND
           match both big-endian) would still round-trip correctly. Here the construction `(bin (u16
           (UInt16.wrap v)))` is DEFAULT big-endian — v = 258 = 0x0102 lays [0x01, 0x02] — and the pattern
           `(bin (u16 n le))` reads those same two bytes LEAST-significant-first, assembling 0x0201 = 513.
           The VALUE CHANGES across the order boundary, so either side dropping its byte order is caught:
           construct-le-ignored reads 258 (wrong), match-le-ignored reads 258 (wrong), both-ignored reads
           258 (wrong) — only genuine BE-construct + LE-read yields 513. Over a runtime operand so nothing
           folds. Expected: 513.")
  (input
    (do
      (def (main (: v Int64)) (match (bin (u16 (UInt16.wrap v))) ((bin (u16 n le)) n) (_ 0)))
      (export main)))
  (call main (: 258 Int64))
  (output (: 513 Int64)))

(case
  "a runtime signed little-endian segment round-trips a negative value"
  (doc
    "The three orthogonal axes combined over a RUNTIME value: SIGNED (two's complement) + LITTLE-ENDIAN
           (byte reversal) + the width-typed contract. `(i16 n le)` takes a runtime `Int16` n = -2 = 0xFFFE;
           little-endian lays it LSB-first as bytes [0xFE, 0xFF], and matching `(i16 m le)` reassembles the
           two's-complement value back to -2. Pins that sign-extension and endianness compose correctly over
           a runtime construct→match round-trip (a constant `(i16 -1)`/`le u16` cover each axis singly; this
           is the runtime intersection with a NEGATIVE multi-byte value).")
  (input
    (do (def (main (: n Int16)) (match (bin (i16 n le)) ((bin (i16 m le)) m) (_ 0))) (export main)))
  (call main (: -2 Int16))
  (output (: -2 Int64)))

(case
  "a runtime signed little-endian segment lays the low byte first"
  (doc
    "The byte-order half of the signed-le round-trip: `(bin (i16 n le))` with a runtime `Int16` n = -2
           = 0xFFFE emits the LEAST-significant byte first, so byte 0 = 0xFE = 254 (not 0xFF). Reads it back
           with `Bytes.at`. Pins that `le` reverses a SIGNED segment's bytes the same way it does an unsigned
           one — the two's-complement bit pattern is laid low-byte-first, so a reader that ignored `le` on a
           signed segment (emitting big-endian 0xFF) would differ here.")
  (input
    (do
      (def (main (: n Int16)) (match (Bytes.at (bin (i16 n le)) 0) ((Some b) b) ((None _) -1)))
      (export main)))
  (call main (: -2 Int16))
  (output (: 254 Int64)))

(case
  "a u16 bin field straddles a byte-rope seam through a runtime-start slice"
  (doc
    "The bin-pattern consumer of the seam-crossing window: the rope [1,18] ++ [52,5] holds a
           big-endian u16 whose TWO BYTES LIVE IN DIFFERENT LEAVES once the runtime-start slice picks
           window [k, k+2). k=1: the field stitches 18,52 across the seam -> 0x1234 = 4660 — a per-leaf
           field read (or a seam-clamped window) truncates or zero-fills. k=0 ([1,18] -> 274) and k=2
           ([52,5] -> 13317) are the single-leaf controls on either side.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(1 18)) (Bytes.of #list(52 5))))
          (def w (Option.expect (Bytes.slice b k 2) "in"))
          (match w ((bin (u16 x)) x) (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4660 Int64))
  (call main (: 0 Int64))
  (output (: 274 Int64))
  (call main (: 2 Int64))
  (output (: 13317 Int64))
  (live-objects known-leak))

(case
  "a dependent-size framing loop walks frames that straddle every rope seam"
  (doc
    "The framing-LOOP composition of the dependent-size crown jewel over a ROPE: frames
           [2|10,20] [1|30] [3|40,41,x] laid out as `[2,10] ++ [20,1,30,3] ++ [40,41,x]` so EVERY
           frame's length-prefix and body land in different leaves. The recursive
           `(bin (u8 n) (bytes body n) (bytes rest))` walk must re-base `rest` across seams and the
           dependent read must stitch bodies through them: 3 frames, body sum 183 at x=42 (3183) and
           191 at x=50 (3191 — the runtime tail byte defeats folding). A framing loop that clamped a
           frame at a leaf boundary or re-read from the parent's origin drops a frame or double-counts.")
  (input
    (do
      (def
        (frames (: b Bytes) (: cnt Int64) (: acc Int64))
        (match
          b
          ((bin (u8 n) (bytes body n) (bytes rest)) (frames rest (+ cnt 1) (+ acc (bsum body 0 0))))
          (_ (+ (* 1000 cnt) acc))))
      (def
        (bsum (: b Bytes) (: i Int64) (: acc Int64))
        (match (Bytes.at b i) ((Some v) (bsum b (+ i 1) (+ acc v))) ((None _u) acc)))
      (def
        (main (: x UInt8))
        (do
          (def
            b
            (Bytes.concat
              (Bytes.concat (Bytes.of #list(2 10)) (Bytes.of #list(20 1 30 3)))
              (Bytes.of #list(40 41 x))))
          (frames b 0 0)))
      (export main)))
  (call main (: 42 UInt8))
  (output (: 3183 Int64))
  (call main (: 50 UInt8))
  (output (: 3191 Int64))
  (live-objects known-leak))

(case
  "a u16 bin field reads correctly at every runtime slice offset, odd or even"
  (doc
    "The ALIGNMENT face of the runtime-start slice family (:987 dispatches on a u8 tag; this reads
           a MULTI-BYTE field): over [0,1,18,52,86], the window `(Bytes.slice b k 2)` puts the big-endian
           u16 at a runtime offset with either parity — k=1 (odd) -> 0x0112 = 274, k=2 (even) -> 0x1234
           = 4660, k=3 (odd) -> 0x3456 = 13398. A lowering that assumed an aligned base for multi-byte
           loads (or fetched the field with an even-address shortcut) flips the odd-offset rows.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.of #list(0 1 18 52 86)))
          (def w (Option.expect (Bytes.slice b k 2) "in"))
          (match w ((bin (u16 x)) x) (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 274 Int64))
  (call main (: 2 Int64))
  (output (: 4660 Int64))
  (call main (: 3 Int64))
  (output (: 13398 Int64))
  (live-objects known-leak))

(case
  "a u32 bin binding with the top bit set does unsigned arithmetic"
  (doc
    "The 32-bit UNSIGNED-arithmetic face (and the clean control of the open u64 signed-division
           finding): `(bin (u32 n))` over [x,0,0,1] at x=128 binds 0x80000001 = 2147483649 — above
           Int32.max but comfortably inside Int64 — and `(% n 1000)` computes 649 unsigned; x=255
           binds 0xFF000001 = 4278190081 (81). A u32 binding that sign-extended its 32nd bit into the
           carrier would answer -647/-919. x=0 control (1).")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(x 0 0 1)))
          (match b ((bin (u32 n)) (Int64.of (% n 1000))) (_ -1))))
      (export main)))
  (call main (: 128 UInt8))
  (output (: 649 Int64))
  (call main (: 255 UInt8))
  (output (: 81 Int64))
  (call main (: 0 UInt8))
  (output (: 1 Int64)))

(case
  "a runtime bin match decodes a FINAL constant-size utf8 segment"
  (doc
    "The RUNTIME companion of the constant-scrutinee utf8 segment pins (:356/:368): `(bin (utf8 s 2))`
           over a runtime-built two-byte value decodes strict UTF-8 (x=105 -> \"hi\", 2 scalars) and treats
           ill-formed bytes as a NON-MATCH falling to the catch-all (x=255 -> -1), exactly as the const
           path does. This FINAL constant-size utf8 segment needs no dynamic cursor — its byte range is the
           tail after the (here empty) fixed prefix — so the runtime lowering reads the range, folds its
           bytes into the whole-scrutinee length, and ANDs strict UTF-8 well-formedness into the arm
           predicate (ill-formed -> the arm does not match). The two rows are the decode + totality faces
           in one. A non-zero static offset is the companion pin below; a non-final / dependent-size utf8
           whose byte length would make a following segment's offset dynamic is a later slice.")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(104 x)))
          (match b ((bin (utf8 s 2)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a runtime bin match decodes a FINAL utf8 segment at a NON-ZERO static offset"
  (doc
    "The static-offset companion of the final constant-size utf8 pin (:1878, which reads utf8 at
           offset 0): a `(u8 tag)` prefix puts the `(utf8 s 2)` segment at byte offset 1, so the decode
           must read the LAST two bytes, not the first. Over `(Bytes.of (list 1 104 x))`: tag=1, then `s`
           decodes bytes [104, x] as strict UTF-8 — x=105 -> \"hi\" (2 scalars) -> 10*1 + 2 = 12; x=255 is
           an invalid continuation byte, so the utf8 segment does NOT match and the `_` arm yields -1.
           Pins (a) the utf8 read honours the preceding int prefix's static byte offset, and (b) the utf8
           byte width is folded into the whole-scrutinee length (`bytes-len == 1 + 2`) — a length test
           that dropped the two utf8 bytes would make this 3-byte input fall through to -1 at x=105.")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(1 104 x)))
          (match b ((bin (u8 tag) (utf8 s 2)) (+ (* 10 tag) (String.scalar-len s))) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 12 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a runtime bin match decodes a FINAL DEPENDENT-size utf8 segment"
  (doc
    "The dependent-size companion of the constant-size utf8 pins (:1878/:1899): a `(u8 n)` length
           prefix SIZES the following `(utf8 s n)` — the string-decoding analogue of the dependent-size
           `(bytes body n)` case (cu-control below), which already passes, so the runtime length/offset
           plumbing works and the only new work is the strict-UTF-8 decode at a RUNTIME-computed size.
           Over `(Bytes.of (list 2 104 x))`: n=2 sizes the two body bytes [104, x] as the utf8 segment,
           the whole-scrutinee length pins `bytes-len == 1 + n`; x=105 -> \"hi\" (2 scalars); x=255 is an
           invalid continuation byte -> the utf8 segment does not match -> the `_` arm -> -1. Pins that a
           dependent-size utf8 reads `n` at run time (a `BinSizedRead` of a `BinIntRead` length) and still
           validates strict UTF-8 as a totality non-match.")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(2 104 x)))
          (match b ((bin (u8 n) (utf8 s n)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a runtime bin match decodes a NON-FINAL dependent-size utf8 segment before a trailing field"
  (doc
    "The non-final face of the dependent-size utf8 decode: `(bin (u8 n) (utf8 s n) (u8 7))` — the
           utf8 segment is NOT last, so the trailing `(u8 7)` literal probe reads at the DYNAMIC offset
           `1 + n` (the utf8's runtime byte length threads into the following segment's `off_plus`, exactly
           as a dependent `(bytes body n)` does). Over `(Bytes.of (list 2 104 x 7))`: n=2 sizes s = [104, x],
           the trailing byte 7 sits at offset 1+2=3 and must equal the literal 7; x=105 -> \"hi\" (2 scalars);
           x=255 is ill-formed utf8 -> non-match -> -1. Pins that a dependent-size utf8 composes with a
           following segment at a runtime-computed offset (a byte-reversed / stale-offset read would probe
           the wrong byte for the `7` literal, or the utf8 would decode the wrong window).")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(2 104 x 7)))
          (match b ((bin (u8 n) (utf8 s n) (u8 7)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a dependent-size utf8 whose length prefix OVERRUNS the frame is a non-match, not a trap"
  (doc
    "Trap-safety of the dependent-size utf8 read (the totality companion of the ill-formed-bytes
           pins): a `(bin (u8 n) (utf8 s n))` over `(Bytes.of (list x 104))` where the length prefix `x`
           feeds `n`. The frame is exactly 2 bytes (prefix + one body byte), so the arm matches ONLY when
           `bytes-len == 1 + n`, i.e. n=1: x=1 -> s decodes the single body byte [104] = \"h\" (1 scalar).
           An OVERRUNNING prefix x=5 (n=5 wants five body bytes but one is present) and a SHORT prefix x=0
           (n=0 leaves the body byte unconsumed) each FALL THROUGH to the `_` arm -> -1. Pins that the
           runtime utf8 `BinSizedRead` never reads out of bounds — the length floor short-circuits the
           decode, so an overrunning frame is a NON-MATCH, never a trap (the dependent-`bytes` reviewer
           finding 2026-07-18, now exercised for the utf8 decode path).")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(x 104)))
          (match b ((bin (u8 n) (utf8 s n)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 1 UInt8))
  (output (: 1 Int64))
  (call main (: 5 UInt8))
  (output (: -1 Int64))
  (call main (: 0 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "le multi-byte fields read little-endian at runtime offsets, unsigned and signed"
  (doc
    "The le pin (:159) round-trips one const u16; this reads le fields from RUNTIME slice offsets
           over [0,1,2,3,4,254,255]: `(u32 n le)` at k=1 assembles [1,2,3,4] least-significant-first ->
           0x04030201 = 67305985; at k=3, [3,4,254,255] -> 0xFFFE0403 = 4294837251 (the high bytes land
           in the TOP of the word — a byte-reversal that only swapped a u16 lane, or a be fallback,
           flips these); k=5 reads [254,255] through SIGNED `(i16 n le)` -> 0xFFFE = -2 (le byte
           assembly THEN two's-complement, order matters).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.of #list(0 1 2 3 4 254 255)))
          (if
            (< k 5)
            (match (Option.expect (Bytes.slice b k 4) "in") ((bin (u32 n le)) (Int64.of n)) (_ -1))
            (match (Option.expect (Bytes.slice b 5 2) "in") ((bin (i16 n le)) n) (_ -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 67305985 Int64))
  (call main (: 3 Int64))
  (output (: 4294837251 Int64))
  (call main (: 5 Int64))
  (output (: -2 Int64))
  (live-objects known-leak))

(case
  "a runtime bin ENCODE lays out le and signed fields byte-exactly"
  (doc
    "The ENCODE twin of the le decode pins — expression-position `(bin (u32 x le) (i16 -2))` over a
           RUNTIME x (nothing folds): x=0x04030201 emits le bytes [1,2,3,4] then the signed -2 as
           big-endian [255,254] — six bytes, spot-read by Bytes.at (len 6, at0=1, at3=4, at4=255 ->
           610655). x=1 puts the value in the FIRST byte only ([1,0,0,0] -> 610255); x=4294967295 is
           all-ones (3175755). A construction that emitted be (or dropped the sign bytes) flips every row.")
  (input
    (do
      (def
        (main (: x Int64))
        (do
          (def b (bin (u32 (UInt32.wrap x) le) (i16 (Int16.wrap -2))))
          (+
            (* 100000 (Bytes.len b))
            (+
              (* 10000 (match (Bytes.at b 0) ((Some v) v) ((None _u) 99)))
              (+
                (* 100 (match (Bytes.at b 3) ((Some v) v) ((None _u) 99)))
                (match (Bytes.at b 4) ((Some v) v) ((None _u) 99)))))))
      (export main)))
  (call main (: 67305985 Int64))
  (output (: 610655 Int64))
  (call main (: 1 Int64))
  (output (: 610255 Int64))
  (call main (: 4294967295 Int64))
  (output (: 3175755 Int64)))

(case
  "a dependent-size body re-matches under a second bin pattern"
  (doc
    "TWO-LEVEL decode — the shape every framed protocol needs: the outer runtime match
           `(bin (u8 tag) (u8 n) (bytes body n) (bytes rest))` binds a 2-byte body, and the INNER
           `(match body ((bin (u16 v)) …))` decodes that bound sub-window as its own scrutinee.
           Over [x,2,18,52,9]: tag=x, v=0x1234=4660, rest=[9] -> 10000x + 4661 (74661 at x=7, 4661
           at x=0). Pins that a `(bytes body n)` binder is a first-class Bytes value a second bin
           pattern can re-anchor on — an inner decode that read from the OUTER frame's origin (or a
           body binder carrying stale offsets) reads tag/len bytes as the u16 (0x0702 = 1794).")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(x 2 18 52 9)))
          (match
            b
            ((bin (u8 tag) (u8 n) (bytes body n) (bytes rest))
              (match body ((bin (u16 v)) (+ (* 10000 tag) (+ v (Bytes.len rest)))) (_ -2)))
            (_ -1))))
      (export main)))
  (call main (: 7 UInt8))
  (output (: 74661 Int64))
  (call main (: 0 UInt8))
  (output (: 4661 Int64)))

(case
  "an i64 le field at the sign boundary decodes negative from its high LAST byte"
  (doc
    "Composes le byte order + signedness + full width: [9,0,...,x] read `(i64 n le)` puts x in
           the MOST-significant position, so x=128 lands the sign bit from the LAST byte —
           n = -(2^63)+9 and the truncating `% 1000` answers -799 (dividend sign). x=0 reads +9 (9).
           A decoder that sign-extended from the FIRST byte (be habit under an le flag) or assembled
           le then dropped the sign reads +...809 % 1000 = 809 or +9 at x=128.")
  (input
    (do
      (def
        (main (: x UInt8))
        (match (Bytes.of #list(9 0 0 0 0 0 0 x)) ((bin (i64 n le)) (% n 1000)) (_ -2)))
      (export main)))
  (call main (: 128 UInt8))
  (output (: -799 Int64))
  (call main (: 0 UInt8))
  (output (: 9 Int64)))

(case
  "an i64 bin binding with the sign bit set is negative in both const and runtime folds"
  (doc
    "The SIGNED-segment perimeter of the u64 const-eval finding: `(bin (i64 n))` over
           [128,0,...,9] IS negative by design (two's-complement -9223372036854775799), and the
           CONST fold and RUNTIME path must agree — 10·const + runtime: x=128 gives 11 (both
           negative), x=0 gives 10 (const still negative, the runtime zero-lead value positive).
           Guards the u64 fix from overcorrecting: an unsigned-everywhere sweep that hit the i64
           arm flips the const digit to 0.")
  (input
    (do
      (def
        (main (: x UInt8))
        (+
          (* 10 (match (Bytes.of #list(128 0 0 0 0 0 0 9)) ((bin (i64 n)) (if (< n 0) 1 0)) (_ -2)))
          (match (Bytes.of #list(x 0 0 0 0 0 0 9)) ((bin (i64 n)) (if (< n 0) 1 0)) (_ -2))))
      (export main)))
  (call main (: 128 UInt8))
  (output (: 11 Int64))
  (call main (: 0 UInt8))
  (output (: 10 Int64)))

(case
  "const u32 and u16 bin bindings with their top bits set fold unsigned"
  (doc
    "The WIDTH perimeter of the u64 const-eval finding: constant-folded `(bin (u32 m))` over
           [128,0,0,1] computes `(> m 5)` = 1 and `((m % 1000) % 10)` = 9 (0x80000001 = 2147483649,
           unsigned), and `(bin (u16 m))` over [128,1] compares 1 — encoded 191. The sub-64 widths
           fit the i64 carrier with headroom, so they fold correctly TODAY; this pins them so the
           pending u64-const fix (and any carrier rework) keeps them right. The runtime u32 twin is
           pinned separately; this is the CONST-fold face.")
  (input
    (do
      (def
        (main (: x UInt8))
        (+
          (*
            100
            (match (Bytes.of #list(128 0 0 1)) ((bin (u32 m)) (if (> m (: 5 UInt32)) 1 0)) (_ -2)))
          (+
            (*
              10
              (match
                (Bytes.of #list(128 0 0 1))
                ((bin (u32 m)) (Int64.of (% (% m (: 1000 UInt32)) 10)))
                (_ -2)))
            (match (Bytes.of #list(128 1)) ((bin (u16 m)) (if (> m (: 5 UInt16)) 1 0)) (_ -2)))))
      (export main)))
  (call main (: 0 UInt8))
  (output (: 191 Int64)))

(case
  "a const top-bit-set u64 bin binding folds unsigned"
  (doc
    "The u64 face of the const-eval binding finding (the width 2114 above is the perimeter): a
           constant-folded `(bin (u64 m))` over [128,0,0,0,0,0,0,9] = 2^63+9 must decode UNSIGNED, not
           through a signed i64 carrier. `(> m 5)` is 1 (a signed read of the top-bit-set value would be
           negative → 0), and `(m % 1000)` is 817 (2^63+9 mod 1000, unsigned; a signed read would compute
           the dividend-signed -799 or reject a bogus overflow). Composite `1000*(>5) + (m % 1000)` = 1817.")
  (input
    (do
      (def
        (main)
        (match
          (Bytes.of #list(128 0 0 0 0 0 0 9))
          ((bin (u64 m)) (+ (* 1000 (if (> m (: 5 UInt64)) 1 0)) (Int64.of (% m (: 1000 UInt64)))))
          (_ -2)))
      (export main)))
  (output (: 1817 Int64)))

(case
  "two const top-bit-set u64 bin bindings in one fn both fold unsigned"
  (doc
    "The two-match perimeter of the u64 const fold: TWO `(bin (u64 m))` matches over the same
           [128,0,0,0,0,0,0,9] in one function must BOTH fold unsigned without the second
           re-materializing the folded 2^63+9 at a signed width. Their unsigned `(m % 1000)` values sum
           to 817 + 817 = 1634 (a signed re-materialization would flip a sign or reject).")
  (input
    (do
      (def
        (main)
        (+
          (match
            (Bytes.of #list(128 0 0 0 0 0 0 9))
            ((bin (u64 m)) (Int64.of (% m (: 1000 UInt64))))
            (_ -2))
          (match
            (Bytes.of #list(128 0 0 0 0 0 0 9))
            ((bin (u64 m)) (Int64.of (% m (: 1000 UInt64))))
            (_ -2))))
      (export main)))
  (output (: 1634 Int64)))

(case
  "a top-bit u64 binding re-encodes to its exact source bytes"
  (doc
    "The ENCODE round-trip of the u64-binding family: `(bin (u64 n))` re-encoding the binding
           read from [x,0,...,9] must reproduce the SOURCE bytes exactly — whole-value equality
           against the original AND a spot-read of byte 0 through a second decode (11 at x=128, the
           top-bit face; 11 at x=0). An encode that pushed the binding through a signed carrier
           conversion (the widening-divergence hazard) or truncated the high byte flips a digit.
           Two clean rejects recorded alongside: Float64.of-int and Float64.of both CDZ0301 a UInt64
           operand — no silent u64→float route exists to mis-convert.")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def src (Bytes.of #list(x 0 0 0 0 0 0 9)))
          (match
            src
            ((bin (u64 n))
              (match
                (bin (u64 n))
                ((bin (u8 b0) (bytes _rest))
                  (+ (* 10 (if (= (bin (u64 n)) src) 1 0)) (if (= b0 x) 1 0)))
                (_ -2)))
            (_ -3))))
      (export main)))
  (call main (: 128 UInt8))
  (output (: 11 Int64))
  (call main (: 0 UInt8))
  (output (: 11 Int64)))

; --- The seam-crossing segment-kind matrix: the :1740 seam pin reads ONE kind (big-endian u16).
; These complete the kinds over the same two-leaf rope + runtime-slice shape: little-endian order,
; signed extension, 8-byte top-bit width, sub-byte bit splits, and the dependent-size body (with
; its oversize-count decline). Each stitches THROUGH the physical leaf boundary; per-leaf reads,
; seam clamps, or narrow carriers diverge on the starred rows.
(case
  "a little-endian u16 bin field stitches its bytes across a rope seam in le order"
  (doc
    "The LE face of the seam-crossing family (the existing seam pin :1740 is big-endian only): the 2-byte window at k=1 straddles the leaves, and the field must assemble LSB-first from the STITCHED pair — [18,52] le = 52*256+18 = 13330. An impl that stitched physically then swapped, vs placing per-byte in le order, diverges exactly here; k=0/k=2 are the single-leaf controls.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(1 18)) (Bytes.of #list(52 5))))
          (def w (Option.expect (Bytes.slice b k 2) "in"))
          (match w ((bin (u16 x le)) x) (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 13330 Int64))
  (call main (: 0 Int64))
  (output (: 4609 Int64))
  (call main (: 2 Int64))
  (output (: 1332 Int64))
  (live-objects known-leak))

(case
  "a signed i16 bin field spanning a rope seam sign-extends from the stitched bytes"
  (doc
    "The SIGN face: the seam window [255,254] reads -2 — sign-extension must run on the full stitched 16 bits, not a leaf's first byte (a per-leaf extend gives a positive splice). k=0 (511) and k=2 (-507) bracket the seam from both sides.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(1 255)) (Bytes.of #list(254 5))))
          (def w (Option.expect (Bytes.slice b k 2) "in"))
          (match w ((bin (i16 x)) x) (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: -2 Int64))
  (call main (: 0 Int64))
  (output (: 511 Int64))
  (call main (: 2 Int64))
  (output (: -507 Int64))
  (live-objects known-leak))

(case
  "a u64 bin field with the top bit set reads unsigned across a rope seam"
  (doc
    "The WIDTH+TOP-BIT face: an 8-byte field whose top bit is set crosses the seam (k=1: bytes ff 02..08 -> 18375252745424078600, observed via BigInt.of). An i64-signed carrier or a per-leaf clamp corrupts the high byte; k=2 (top bit clear) is the in-range control.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(0 255 2 3 4 5 6 7)) (Bytes.of #list(8 9))))
          (def w (Option.expect (Bytes.slice b k 8) "in"))
          (match w ((bin (u64 x)) (BigInt.of x)) (_ (BigInt.of -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 18375252745424078600 BigInt))
  (call main (: 2 Int64))
  (output (: 144964032628459529 BigInt))
  (live-objects known-leak))

(case
  "bit-field segments spanning a rope seam split the stitched 16 bits, not a leaf's"
  (doc
    "The BITS face of the seam family: (bits a 4)(bits b2 12) over a 2-byte window whose 16 bits straddle two leaves — the 4/12 split must run on the stitched pair (k=1: [18,52] -> a=1,b2=564 -> 10564). k=0/k=2 are the single-leaf controls (101298 / 31029).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(165 18)) (Bytes.of #list(52 5))))
          (def w (Option.expect (Bytes.slice b k 2) "in"))
          (match w ((bin (bits a 4) (bits b2 12)) (+ (* a 10000) b2)) (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10564 Int64))
  (call main (: 0 Int64))
  (output (: 101298 Int64))
  (call main (: 2 Int64))
  (output (: 31029 Int64))
  (live-objects known-leak))

(case
  "a dependent-size body straddling a rope seam stitches, and an oversize count fails the match"
  (doc
    "The DEPENDENT-SIZE face: (u8 n)(bytes body n)(bytes rest) from a runtime-sliced start whose n-byte body crosses the seam (k=1: n=3, body [10,20,30] sums 60 -> 60015 with rest [7,8]); at k=0 the count byte is 9 with only 6 remaining, so the match FAILS to the catch-all (-1) — the oversize face doubles as the honest-decline row.")
  (input
    (do
      (def
        (bsum (: b Bytes) (: i Int64) (: acc Int64))
        (match (Bytes.at b i) ((Some v) (bsum b (+ i 1) (+ acc v))) ((None _u) acc)))
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(9 3 10)) (Bytes.of #list(20 30 7 8))))
          (def w (Option.expect (Bytes.slice b k (- 7 k)) "in"))
          (match
            w
            ((bin (u8 n) (bytes body n) (bytes rest)) (+ (* 1000 (bsum body 0 0)) (bsum rest 0 0)))
            (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 60015 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

; --- bin construction/decode order + the frame-body perform walk. ---
(case
  "bin-encode segment ORDER holds for runtime-swapped operand values"
  (doc
    "The bin-construction twin of the splice-order pin: two runtime-selected UInt8 values
           (swapping by k) encode into `(bin (u8 a) (u8 b))` and the byte positions prove segment
           fidelity (1020 at k=1, 2010 at k=2 — the spot-reads distinguish position from value).
           A segment emitter that ordered by operand evaluation completion (or shared one wrap's
           result across both slots) crosses the bytes. With the splice pin this closes the
           ordered-operand discipline across both construction meta-levels (Ast splice + byte
           encode).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def a (UInt8.wrap (if (= k 1) 10 20)))
          (def b (UInt8.wrap (if (= k 1) 20 10)))
          (def f (bin (u8 a) (u8 b)))
          (+
            (* 100 (match (Bytes.at f 0) ((Some v) v) ((None _u) -1)))
            (match (Bytes.at f 1) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1020 Int64))
  (call main (: 2 Int64))
  (output (: 2010 Int64)))

(case
  "a recursive TLV frame walk over sliced runtime bytes decodes LE payloads per frame"
  (doc
    "The protocol-parser composition: a recursive walk `Bytes.slice`s a 3-byte frame per step from
           one runtime byte sequence, bin-matches each frame `(bin (u8 tag) (u16 val le))`, accumulates
           the little-endian payloads, and stops at the tag-0 sentinel frame. Composes three pinned
           surfaces — slice-window re-basing per call, the LE u16 segment read over a SLICE-backed
           frame (not a whole materialized bin), and recursion driving fresh decodes — none of whose
           existing pins exercise slice-per-frame recursion. k = 5: frames (1, 5+256=261) then (1, 2)
           then stop → 263; k = 200: (1, 456) then (1, 2) → 458.")
  (input
    (do
      (def
        (walk (: b Bytes) (: off Int64) (: acc Int64))
        (match
          (Bytes.slice b off 3)
          ((Some frame)
            (match
              frame
              ((bin (u8 tag) (u16 val le))
                (if (= (Int64.of tag) 0) acc (walk b (+ off 3) (+ acc (Int64.of val)))))
              (_ (- 0 acc))))
          ((None _u) acc)))
      (def
        (main (: k UInt8))
        (walk
          (Bytes.of
            #list((UInt8.wrap 1)
              k
              (UInt8.wrap 1)
              (UInt8.wrap 1)
              (UInt8.wrap 2)
              (UInt8.wrap 0)
              (UInt8.wrap 0)
              (UInt8.wrap 0)
              (UInt8.wrap 0)))
          0
          0))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 263 Int64))
  (call main (: 200 UInt8))
  (output (: 458 Int64))
  (live-objects known-leak))

(case
  "a bin-decoded frame body drives per-byte performs through a recursive walk"
  (doc
    "Binary matching × effects: the dependent-size `(bytes body n)` binder feeds a RECURSIVE
           walk whose every step PERFORMS — the frame parser emitting events. Each `ev` scales its
           byte by the stepped state (v·(s+1)): body [x,20] → x·1 + 20·2 = x+40 (50 at x=10, 40 at
           x=0). The bin binder must stay a live Bytes value across N perform/resume suspensions
           of the walk — a body binder holding a frame-relative view invalidated by the handler
           round-trip (or a walk whose accumulator re-read a stale state) breaks the sum.")
  (input
    (do
      (effect Emit (op ev (-> Int64 Int64)))
      (def
        (walk (: b Bytes) (: i Int64) (: acc Int64))
        (match (Bytes.at b i) ((Some v) (walk b (+ i 1) (+ acc (Emit.ev v)))) ((None _u) acc)))
      (def
        (main (: x UInt8))
        (handle
          Emit
          0
          ((ev (v) s (resume (* v (+ s 1)) (+ s 1))))
          (match
            (Bytes.of #list(2 x 20 99))
            ((bin (u8 n) (bytes body n) (bytes _rest)) (walk body 0 0))
            (_ -1))))
      (export main)))
  (call main (: 10 UInt8))
  (output (: 50 Int64))
  (call main (: 0 UInt8))
  (output (: 40 Int64))
  (live-objects known-leak))

(case
  "a bits run spanning TWO bytes decodes MSB-first over a runtime scrutinee"
  (doc
    "The cross-byte DECODE face (the sub-byte 3+5 run and the byte-aligned-run-then-int cases pin
           single-byte shapes; construction pins cross-byte PACKING): `(bin (bits a 3) (bits b 13))` over
           runtime bytes [h, 90] — the 13-bit `b` STRADDLES the byte boundary, so the matcher must read
           the 16-bit run big-endian and shift/mask across bytes. h=182 (0b10110110): a = top 3 = 5,
           b = low 13 of 0b1011011001011010 = 5722 → 505722. A per-byte decoder (or one that read the
           run little-endian) misreads b.")
  (input
    (do
      (def
        (run (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 90)))
          ((bin (bits a 3) (bits b 13)) (+ (* 100000 a) b))
          (_other -1)))
      (export run)))
  (call run (: 182 Int64))
  (output (: 505722 Int64)))

(case
  "FOUR bit-fields with widths 5+7+9+3 decode across three runtime bytes"
  (doc
    "The dense-packing decode face: widths 5+7+9+3 = 24 bits over three runtime-headed bytes, where
           `b` (7) crosses the first boundary and `c` (9) crosses the second — no field is byte-aligned
           after the first. h=202, bytes [202,53,227] = 0b110010100011010111100011: a=25, b=35, c=188,
           d=3 → 25351883. Any off-by-one in the running bit cursor (or a mask clipped at a byte edge)
           shifts every later field.")
  (input
    (do
      (def
        (run (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 53) (UInt8.wrap 227)))
          ((bin (bits a 5) (bits b 7) (bits c 9) (bits d 3))
            (+ (* 1000000 a) (+ (* 10000 b) (+ (* 10 c) d))))
          (_other -1)))
      (export run)))
  (call run (: 202 Int64))
  (output (: 25351883 Int64)))

(case
  "a dependent-size bin-match payload read reclaims its slice across a loop (no live objects)"
  (doc
    "Each iteration builds a fresh runtime 3-byte frame [h,7,8] and matches the dependent-size
           `(bytes payload k)` (k = the runtime u8 header), reading `Bytes.len payload`. h=2 binds 2 payload
           bytes per iter -> 500 x 2 = 1000. The BinSizedRead dups the borrowed scrutinee and bytes-slice
           consumes the dup, returning a fresh owned slice; the dup/consume pairing must net the heap to 0
           (a leaked slice or unbalanced dup would scale to ~N live).")
  (input
    (do
      (def
        (loop (: j Int64) (: n Int64) (: h Int64) (: tot Int64))
        (if
          (< j n)
          (loop
            (+ j 1)
            n
            h
            (+
              tot
              (match
                (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
                ((bin (u8 k) (bytes payload k)) (Bytes.len payload))
                (_ -1))))
          tot))
      (def (f (: h Int64)) (loop 0 500 h 0))
      (export f)))
  (call f (: 2 Int64))
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "a NON-FINAL dependent-size bin-match with a dynamic-offset rest read reclaims every slice (no live objects)"
  (doc
    "Each iteration builds a fresh runtime 4-byte frame [h,7,8,9] and matches `(bytes body k)` (k = the
           runtime u8 header) followed by `(bytes rest)` -- rest reads at the DYNAMIC offset 1+k. h=2 ->
           body=[7,8] (2) + rest=[9] (1) = 3 per iter -> 500 x 3 = 1500. TWO owned slices per match plus the
           dynamic off_plus read, each dup/consume pair must net to 0 (a leaked slice or an off_plus dup
           without consume would scale to ~N live).")
  (input
    (do
      (def
        (loop (: j Int64) (: n Int64) (: h Int64) (: tot Int64))
        (if
          (< j n)
          (loop
            (+ j 1)
            n
            h
            (+
              tot
              (match
                (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8) (UInt8.wrap 9)))
                ((bin (u8 k) (bytes body k) (bytes rest)) (+ (Bytes.len body) (Bytes.len rest)))
                (_ -1))))
          tot))
      (def (f (: h Int64)) (loop 0 500 h 0))
      (export f)))
  (call f (: 2 Int64))
  (output (: 1500 Int64))
  (live-objects 0))

; -- breaker batch 404 (2026-08-26): RUNTIME dependent-size utf8 segment decodes — non-final with
; trailing field, final position, and the bytes-segment control (cu01-cu03).
(case
  "cu01 runtime NON-FINAL dependent-size utf8 segment decodes with a trailing field"
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(2 104 x 7)))
          (match b ((bin (u8 n) (utf8 s n) (u8 7)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "cu02 runtime FINAL dependent-size utf8 segment decodes"
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(2 104 x)))
          (match b ((bin (u8 n) (utf8 s n)) (String.scalar-len s)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 255 UInt8))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "cu03 CONTROL runtime non-final dependent-size BYTES segment"
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(2 104 x 7)))
          (match b ((bin (u8 n) (bytes body n) (u8 7)) (Bytes.len body)) (_ -1))))
      (export main)))
  (call main (: 105 UInt8))
  (output (: 2 Int64))
  (call main (: 99 UInt8))
  (output (: 2 Int64)))

; -- runtime bin-match (param-built scrutinee, defeats the const fold) rest + dependent-size segments
; (migration from rcdzc bin-match cdz-run tests, 2026-08-27): exercises the runtime BinRestRead / BinSizedRead emit.
(case
  "a runtime bin-match binds a final rest bytes segment (tail length)"
  (input
    (do
      (def
        (main (: n UInt8))
        (let
          ((payload (Bytes.of #list(1 2 3))))
          (match (bin (u8 n) (bytes payload)) ((bin (u8 t) (bytes rest)) (Bytes.len rest)) (_ -9))))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 3 Int64)))

(case
  "a runtime bin-match final rest is empty when the scrutinee is exactly the header"
  (input
    (do
      (def
        (main (: n UInt8))
        (let
          ((payload (Bytes.of #list())))
          (match (bin (u8 n) (bytes payload)) ((bin (u8 t) (bytes rest)) (Bytes.len rest)) (_ -9))))
      (export main)))
  (call main (: 7 UInt8))
  (output (: 0 Int64)))

(case
  "a runtime bin-match final rest after a 2-byte header binds the tail"
  (input
    (do
      (def
        (main (: n UInt16))
        (let
          ((payload (Bytes.of #list(9 8 7 6 5))))
          (match (bin (u16 n) (bytes payload)) ((bin (u16 t) (bytes rest)) (Bytes.len rest)) (_ -9))))
      (export main)))
  (call main (: 300 UInt16))
  (output (: 5 Int64)))

(case
  "a runtime bin-match too short for the fixed prefix falls through"
  (input
    (do
      (def
        (main (: n UInt8))
        (match (bin (u8 n)) ((bin (u16 t) (bytes rest)) (Bytes.len rest)) (_ -9)))
      (export main)))
  (call main (: 5 UInt8))
  (output (: -9 Int64)))

(case
  "a runtime bin-match binds a dependent-size bytes segment named by an earlier binder"
  (input
    (do
      (def
        (main (: h Int64))
        (match
          (Bytes.of #list((UInt8.wrap h) (UInt8.wrap 7) (UInt8.wrap 8)))
          ((bin (u8 n) (bytes payload n)) (Bytes.len payload))
          (_ -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64)))

; -- runtime bin-match decode over a param-built scrutinee (defeats the const fold; exercises the runtime
; BinIntRead + literal-tag dispatch emit; migration from rcdzc a_runtime_bin_match_decodes_..., 2026-08-27).
(case
  "a runtime bin-match u16 round-trips the built value"
  (input
    (do (def (main (: n UInt16)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -9))) (export main)))
  (call main (: 258 UInt16))
  (output (: 258 Int64)))

(case
  "a runtime bin-match signed i8 decodes two's complement"
  (input (do (def (main (: n Int8)) (match (bin (i8 n)) ((bin (i8 m)) m) (_ -9))) (export main)))
  (call main (: -1 Int8))
  (output (: -1 Int64)))

(case
  "a runtime bin-match little-endian read matches the little-endian build"
  (input
    (do
      (def (main (: n UInt16)) (match (bin (u16 n le)) ((bin (u16 m le)) m) (_ -9)))
      (export main)))
  (call main (: 258 UInt16))
  (output (: 258 Int64)))

(case
  "a runtime bin-match leading literal tag dispatches to the decode"
  (input
    (do
      (def (main (: n UInt16)) (match (bin (u8 1) (u16 n)) ((bin (u8 1) (u16 m)) m) (_ -9)))
      (export main)))
  (call main (: 300 UInt16))
  (output (: 300 Int64)))

(case
  "a runtime bin-match mismatched literal tag falls to the catch-all"
  (input
    (do
      (def (main (: n UInt16)) (match (bin (u8 2) (u16 n)) ((bin (u8 1) (u16 m)) m) (_ -9)))
      (export main)))
  (call main (: 300 UInt16))
  (output (: -9 Int64)))

(case
  "a runtime bin-match whole-scrutinee length mismatch falls to the catch-all"
  (input (do (def (main (: n UInt8)) (match (bin (u8 n)) ((bin (u16 m)) m) (_ -9))) (export main)))
  (call main (: 5 UInt8))
  (output (: -9 Int64)))

(case
  "a runtime bin-match multi-arm dispatch selects by the leading literal tag"
  (input
    (do
      (def
        (main (: t UInt8) (: v UInt16))
        (match
          (bin (u8 t) (u16 v))
          ((bin (u8 1) (u16 x)) x)
          ((bin (u8 2) (u16 y)) (+ y 1000))
          (_ -1)))
      (export main)))
  (call main (: 1 UInt8) (: 42 UInt16))
  (output (: 42 Int64))
  (call main (: 2 UInt8) (: 42 UInt16))
  (output (: 1042 Int64))
  (call main (: 9 UInt8) (: 42 UInt16))
  (output (: -1 Int64)))

; -- a top-bit-set u64 bin binding reads UNSIGNED (migration from rcdzc a_u64_bin_binding_binds_unsigned_
; not_wrapped_signed, 2026-08-27): the existing u64 pattern case reads a small value (top bit clear); this
; pins the top-bit-set path where a signed rem_s would give the wrong answer.
(case
  "a top-bit-set u64 bin binding reads UNSIGNED not signed-wrapped"
  (doc
    "`(bin (u64 n))` over a byte sequence whose top bit is set must read n as UNSIGNED (>= 2^63):
           x=128 -> n = 2^63 + 1, (% n 1000) = 809 unsigned (a signed rem_s would give -807). x=64 ->
           n = 2^62 + 1 -> 905, the top-bit-clear control where signed and unsigned agree.")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def b (Bytes.of #list(x 0 0 0 0 0 0 1)))
          (match b ((bin (u64 n)) (Int64.of (% n 1000))) (_ -1))))
      (export main)))
  (call main (: 128 UInt8))
  (output (: 809 Int64))
  (call main (: 64 UInt8))
  (output (: 905 Int64)))

; ── breaker batch 574: bin patterns × DEEP ropes (the 50-concat bdr shape destructured). bmr1 =
; segments read exactly ACROSS rope seams (the u16 spans the 2-byte base, the u8 crosses into the
; concat chain, the rest binds the remainder) and everything reclaims. bmr2 = fifty matches leak
; ZERO per-match (census 1 = the borrowed-param rope, the fixed mts1 class) — the (bytes rest)
; binding reclaims clean where Bytes.slice leaks its dup (the slc family): the CONTRAST between
; the two remainder-taking mechanisms.
(case
  "bmr1 bin segments read exactly across the seams of a 50-concat rope and reclaim"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(7))) (- k 1))))
      (def
        (main (: n Int64))
        (let
          ((r (grow (Bytes.of #list(1 2)) 50)))
          (match
            r
            ((bin (u16 head) (u8 first7) (bytes rest)) (+ (* 100 head) (+ first7 (Bytes.len rest))))
            (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 25856 Int64))
  (live-objects 0))

(case
  "bmr2 fifty bin matches over a deep rope leak zero per-match (the rest binding reclaims where slicing leaks)"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(7))) (- k 1))))
      (def
        (frames (: r Bytes) (: k Int64))
        (if
          (= k 0)
          0
          (+
            (match r ((bin (u16 head) (bytes rest)) (+ head (Bytes.len rest))) (_ -1))
            (frames r (- k 1)))))
      (def (main (: n Int64)) (frames (grow (Bytes.of #list(1 2)) 50) n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 15400 Int64))
  (live-objects known-leak))

; ── breaker batch 575: RUNTIME bin construction cells (the corpus round-trips are constant; these
; take the widths from the ARG). Segment typing is exact-width (an Int64 in a u8 slot is a
; teaching CDZ0203: wrap-to-truncate or of-to-check) — bcr1 pins the explicit-wrap round-trip
; including the truncation semantics at an over-width value (300 → u8 44); bcr2 = fifty
; construct+match cycles reclaim to zero.
(case
  "bcr1 a runtime bin construction round-trips through its own pattern with explicit wrap semantics (300 → u16 300 / u8 44)"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (bin (u16 (UInt16.wrap n)) (u8 (UInt8.wrap n)))
          ((bin (u16 a) (u8 b)) (+ (* 1000 a) b))
          (_ -1)))
      (export main)))
  (call main (: 300 Int64))
  (output (: 300044 Int64))
  (live-objects 0))

(case
  "bcr2 fifty runtime bin construct+match cycles reclaim to zero"
  (input
    (do
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (match
              (bin (u16 (UInt16.wrap k)) (u8 (UInt8.wrap k)))
              ((bin (u16 a) (u8 b)) (+ a b))
              (_ -1))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 2550 Int64))
  (live-objects 0))

; ── breaker batch 576: dependent-size utf8 segments × DEEP ropes. ubr1 = the length prefix sits
; in the rope BASE and the decoded string SPANS the first seam — exact decode + content check +
; rest accounting (the 4-cell decode residue is the utf8 face of the view-residue family). ubr2 =
; fifty decodes leak ~2/frame linearly (+1 borrowed rope) — calibration with slc/rp2.
(case
  "ubr1 a dependent-size utf8 segment decodes across a rope seam (prefix in the base, string spanning the concat)"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(104 105))) (- k 1))))
      (def
        (main (: n Int64))
        (let
          ((r (grow (bin (u8 (UInt8.wrap (* n 2)))) 50)))
          (match
            r
            ((bin (u8 len) (utf8 s len) (bytes rest))
              (+ (* 1000 (String.byte-len s)) (+ (if (= s "hi") 100 0) (Bytes.len rest))))
            (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2198 Int64))
  (live-objects known-leak))

(case
  "ubr2 fifty dependent-size utf8 decodes over a deep rope leak linearly (~2 per decode)"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(104 105))) (- k 1))))
      (def
        (frames (: r Bytes) (: k Int64))
        (if
          (= k 0)
          0
          (+
            (match r ((bin (u8 len) (utf8 s len) (bytes rest)) (String.byte-len s)) (_ -1))
            (frames r (- k 1)))))
      (def (main (: n Int64)) (frames (grow (bin (u8 (UInt8.wrap (+ n 1)))) 50) n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 2550 Int64))
  (live-objects known-leak))

; ── breaker batch 577: signed segments + the utf8-construction gap. sbn1 = a runtime NEGATIVE
; round-trips through i16/i8 with exact wrap semantics (-300 → i16 -300 / i8 -44). sbn2 = utf8
; construction is PATTERN-ONLY today ("constructing a utf8 bin segment is not yet built") — an
; honest decline naming the increment; todo auto-flip witness with a twice-traced oracle (the
; grammar requires the explicit-size (utf8 s n) form in BOTH positions; a malformed one-arg
; segment cascades to a misleading unbound-bin secondary error — the CDZ0201 primary is correct).
(case
  "sbn1 a runtime negative round-trips through signed i16/i8 segments with exact wrap semantics"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (bin (i16 (Int16.wrap n)) (i8 (Int8.wrap n)))
          ((bin (i16 a) (i8 b)) (+ (* 1000 a) b))
          (_ 9)))
      (export main)))
  (call main (: -300 Int64))
  (output (: -300044 Int64))
  (live-objects 0))

(case
  "sbn2 a runtime utf8 CONSTRUCTION segment declines pending the encode increment (utf8 is pattern-only)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s (String.concat "h" (if (> n 0) "i" "o"))))
          (match
            (bin (u8 (UInt8.wrap (String.byte-len s))) (utf8 s 2))
            ((bin (u8 len) (utf8 out len)) (if (= out "hi") 100 0))
            (_ -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100 Int64)))

; A `(bin …)` pattern never covers every byte sequence — empty input, a wrong length, an unequal literal, or
; (for a `utf8` segment) ill-formed bytes all fail to match — so a `bin` match whose only arm is a bin pattern
; is non-exhaustive → CDZ0210, exactly as a sum match missing a variant. (Migrated from rcdzc
; a_bytes_match_with_only_a_bin_arm_and_no_catch_all_is_non_exhaustive + a_utf8_bin_match_with_no_catch_all_is_non_exhaustive.)
(case
  "a bytes match with only a bin arm and no catch-all is non-exhaustive"
  (input (do (def (main) (match (Bytes.of #list(1 2)) ((bin (u16 n)) n))) (export main)))
  (error CDZ0210))

(case
  "a utf8 bin match with no catch-all is non-exhaustive"
  (input
    (do
      (def (main) (match (Bytes.of #list(3 102 111 111)) ((bin (u8 n) (utf8 name n)) name)))
      (export main)))
  (error CDZ0210))

; bpx1: a runtime bin-match over a locally-BUILT Bytes (arg-dependent first byte) — three fixed-width
; u8 segments destructured and recombined positionally. Round-trips on the cadenza hop since #7934
; (bin-match sub-slice 1); previously hop-1 declined the whole match. (breaker bp1 probe, promoted
; at v-cadenza-backend's ask.)
(case
  "a runtime bin match over a built Bytes destructures three fixed-width segments"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((b (Bytes.of #list((UInt8.of n) 20 30))))
          (match
            b
            ((bin (u8 x) (u8 y) (u8 z))
              (+ (* 10000 (Int64.of x)) (+ (* 100 (Int64.of y)) (Int64.of z))))
            (_ -1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 72030 Int64))
  (call main (: 9 Int64))
  (output (: 92030 Int64)))

; blx1: MULTI-ARM bin match with OVERLAPPING literal prefixes (magic-number discrimination) — the
; arm-fused decision tree shares segment reads across arms. Round-trips on the cadenza hop since
; #7958 (wildcard/unread segments across overlapping literal arms); previously the fused reads
; escaped the bin-match recognizer and hop-1 declined. All three arms exercised across three calls,
; including the runtime first byte colliding with the 137 magic. (breaker bl1 probe, promoted.)
(case
  "a multi-arm bin match discriminates overlapping literal prefixes"
  (input
    (do
      (def
        (classify (: b Bytes))
        (match
          b
          ((bin (u8 137) (u8 80) (u8 x)) (+ 1000 (Int64.of x)))
          ((bin (u8 137) (u8 y) (u8 _z)) (+ 2000 (Int64.of y)))
          ((bin (u8 a) (u8 _b) (u8 _c)) (+ 3000 (Int64.of a)))
          (_ -1)))
      (def
        (main (: n Int64))
        (+
          (classify (Bytes.of #list(137 80 (UInt8.of n))))
          (+
            (* 10000 (classify (Bytes.of #list(137 66 1))))
            (* 100000000 (classify (Bytes.of #list((UInt8.of n) 80 1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 300520661005 Int64))
  (call main (: 137 Int64))
  (output (: 100120661137 Int64))
  ; TODO(v-memory-safety): the arm-fused multi-arm bin match over a Bytes value currently leaks the
  ; discriminated segment reads; value-correct (no UAF) but not reclaimed to 0 — tracked known-leak,
  ; tighten to 0 once the bin-match segment-read interior reclaim lands. (breaker blx1 #7958, added unpinned.)
  (live-objects known-leak))

; dzx1: the length-prefixed FRAME — a fixed tail segment AFTER a dependent-size payload (a trailing
; read at a DYNAMIC offset). Round-trips on the cadenza hop since #7977 (dependent-final landed in
; #7972; the trailing fixed read was the last framing rung). Pins the full mismatch matrix too: a
; length shorter than the remaining bytes AND one past the end both fall to the wildcard.
; (breaker dz1 probe, promoted; live-objects 0 census-checked.)
(case
  "a length-prefixed frame parses payload and trailing tag, rejecting bad lengths"
  (input
    (do
      (def
        (parse (: b Bytes))
        (match
          b
          ((bin (u8 len) (bytes payload len) (u8 tail))
            (+ (* 1000 (Bytes.len payload)) (Int64.of tail)))
          (_ -1)))
      (def
        (main (: n Int64))
        (+
          (parse (Bytes.of #list(2 10 20 7)))
          (* 100000 (parse (Bytes.of #list((UInt8.of n) 1 2 3 9))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 300902007 Int64))
  (call main (: 0 Int64))
  (output (: -97993 Int64))
  (call main (: 200 Int64))
  (output (: -97993 Int64)))

; bfx1/bfx2: BIT-FIELD bin-matches ride the byte-read + shift/mask fallback on the cadenza target —
; no dedicated re-emission slice needed (v-cadenza-backend coverage finding, breaker-verified:
; tri-target exact + byte-idempotent + live-objects 0). bfx1 pins nibble BINDERS reconstructing the
; byte; bfx2 pins a LITERAL bit tag (the high bit) discriminating two arms with a 7-bit payload.
(case
  "nibble bit-field binders reconstruct the byte"
  (input
    (do
      (def
        (parse (: b Bytes))
        (match b ((bin (bits hi 4) (bits lo 4)) (+ (* 16 (Int64.of hi)) (Int64.of lo))) (_ -1)))
      (def (main (: n Int64)) (parse (Bytes.of #list((UInt8.of n)))))
      (export main)))
  (call main (: 171 Int64))
  (output (: 171 Int64))
  (call main (: 15 Int64))
  (output (: 15 Int64)))

(case
  "a literal high-bit tag discriminates two arms over a seven-bit payload"
  (input
    (do
      (def
        (parse (: b Bytes))
        (match
          b
          ((bin (bits 1 1) (bits v 7)) (+ 100 (Int64.of v)))
          ((bin (bits 0 1) (bits v 7)) (+ 200 (Int64.of v)))
          (_ -1)))
      (def (main (: n Int64)) (parse (Bytes.of #list((UInt8.of n)))))
      (export main)))
  (call main (: 171 Int64))
  (output (: 143 Int64))
  (call main (: 15 Int64))
  (output (: 215 Int64)))
