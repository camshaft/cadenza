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
; Tagged `(needs binary-matching)`: a later generation realizes the `bin` form (it subsumes the seed's
; `bytes` value form; options/realized-capability-set/). The seed does not realize it, so its behavior
; gate SKIPS these cases — they pin the contract the realization must meet, they are not seed declines.

; ============================================================================================
; Construction — `(bin …)` in expression position builds a Bytes value
; ============================================================================================

(case "a u16 segment encodes an integer big-endian by default"
  (doc    "`(bin (u16 258))` encodes 258 (0x0102) as two bytes, most-significant first — big-endian is
           the default byte order, so the result is `(Bytes.of (list 1 2))`. Pins the default-endianness
           construction the wasm/network-order idiom depends on.")
  (needs  binary-matching)
  (input  (= (bin (u16 258)) (Bytes.of (list 1 2))))
  (output (: true Bool)))

(case "the le modifier encodes a u16 little-endian"
  (doc    "`(bin (u16 258 le))` selects little-endian with the `le` modifier, so 0x0102 is emitted
           least-significant byte first: `(Bytes.of (list 2 1))`. Pins that byte order is explicit and
           the modifier reverses the default, never an implicit host-endianness choice.")
  (needs  binary-matching)
  (input  (= (bin (u16 258 le)) (Bytes.of (list 2 1))))
  (output (: true Bool)))

(case "a u32 segment encodes four big-endian bytes"
  (doc    "`(bin (u32 0x89504E47))` encodes the magic number as four big-endian bytes
           `(Bytes.of (list 137 80 78 71))` — the fixed-width, whole-byte encoding a magic-number header
           is built from. Written as a hex literal (01-literals.sexp), the value reads as its bytes at a
           glance. Pins u32 width and byte order together.")
  (needs  binary-matching)
  (input  (= (bin (u32 0x89504E47)) (Bytes.of (list 137 80 78 71))))
  (output (: true Bool)))

(case "a signed i8 segment encodes as two's complement"
  (doc    "`(bin (i8 -1))` encodes -1 in a signed 8-bit segment as the two's-complement byte 255. Pins
           that a SIGNED segment admits a negative value (unlike an unsigned segment, which traps on one —
           see below), encoding it in two's complement.")
  (needs  binary-matching)
  (input  (= (bin (i8 -1)) (Bytes.of (list 255))))
  (output (: true Bool)))

(case "bit-field segments pack sub-byte values into one byte"
  (doc    "`(bin (bits 1 1) (bits 2 3) (bits 5 4))` packs a 1-bit flag (1), a 3-bit tag (2 = 0b010), and a
           4-bit value (5 = 0b0101), most-significant field first: 1·010·0101 = 0b1010_0101 = 165. The
           three widths sum to 8, so the `bin` closes exactly one byte. The expected byte is written as a
           binary literal `0b1010_0101` so the packed bit-fields are legible. Pins sub-byte bit-field
           packing and the most-significant-field-first order.")
  (needs  binary-matching)
  (input  (= (bin (bits 1 1) (bits 2 3) (bits 5 4)) (Bytes.of (list 0b1010_0101))))
  (output (: true Bool)))

(case "a length-prefixed frame is built from a size segment and a bytes segment"
  (doc    "`(bin (u16 (Bytes.len payload)) (bytes payload))` writes payload's length as a big-endian
           u16 prefix, then splices payload — the length-framing idiom that replaces hand-rolled
           `(Bytes.concat (Bytes.of (list (& (>> len 8) 255) (& len 255))) payload)`. Pins that a computed
           value feeds a size segment and an unsized `(bytes …)` splices a whole Bytes value.")
  (needs  binary-matching)
  (input  (= (bin (u16 (Bytes.len (Bytes.of (list 10 20 30)))) (bytes (Bytes.of (list 10 20 30))))
             (Bytes.of (list 0 3 10 20 30))))
  (output (: true Bool)))

(case "an empty binary form is the empty byte sequence"
  (doc    "`(bin)` with no segments is the zero-length Bytes value, equal to `(Bytes.of (list))`. Pins the
           degenerate construction — the identity a fold over segments starts from.")
  (needs  binary-matching)
  (input  (= (bin) (Bytes.of (list))))
  (output (: true Bool)))

(case "the length of a fixed-width construction is the sum of its segment widths"
  (doc    "`(Bytes.len (bin (u32 0)))` = 4: a u32 segment is four bytes wide regardless of the value it
           carries. Pins that fixed-width segments contribute their width, not a value-dependent length.")
  (needs  binary-matching)
  (input  (Bytes.len (bin (u32 0))))
  (output (: 4 Int64)))

; ============================================================================================
; Matching — `(bin …)` in pattern position destructures a Bytes scrutinee
; ============================================================================================

(case "a bin pattern binds an integer read from a fixed-width segment"
  (doc    "Matching `(bin (u16 258))` against the pattern `(bin (u16 n))` reads the big-endian u16 back
           into n, round-tripping the construction: n = 258. Pins that construction and matching are
           inverse over the same segment grammar.")
  (needs  binary-matching)
  (input  (match (bin (u16 258))
            ((bin (u16 n)) n)
            (_ 0)))
  (output (: 258 Int64)))

(case "a dependent-size segment binds exactly the number of bytes an earlier segment named"
  (doc    "Against `(Bytes.of (list 2 10 20 99))`, the pattern `(bin (u8 n) (bytes body n) (bytes rest))`
           reads n = 2 from the first byte, then binds `body` to exactly the next n = 2 bytes
           `(list 10 20)`, leaving `rest` = `(list 99)`. The crown jewel: a segment's size is a value
           bound earlier in the same pattern, all value-level. This case checks `body`; the next checks
           `rest`.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 2 10 20 99))
            ((bin (u8 n) (bytes body n) (bytes rest)) (= body (Bytes.of (list 10 20))))
            (_ false)))
  (output (: true Bool)))

(case "a final bytes segment binds the remainder after the sized segments"
  (doc    "The companion of the dependent-size case: the same match returns `rest`, the bytes after the
           n-byte body — `(list 99)`. Pins that a final unsized `(bytes rest)` captures everything left,
           the remainder a framing loop iterates on.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 2 10 20 99))
            ((bin (u8 n) (bytes body n) (bytes rest)) (= rest (Bytes.of (list 99))))
            (_ false)))
  (output (: true Bool)))

(case "a literal segment matches a magic-number header by equality"
  (doc    "The pattern `(bin (u32 0x89504E47) (bytes rest))` matches the scrutinee only when its first
           four bytes equal the magic number (137 80 78 71) — a literal segment matches by equality, the
           direct analogue of a literal value pattern, and the hex literal names the magic number
           legibly. Pins magic-number dispatch on a binary header.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 137 80 78 71 1 2))
            ((bin (u32 0x89504E47) (bytes rest)) "match")
            (_ "other")))
  (output (: "match" String)))

(case "a bin arm whose fixed-width segment overruns the input falls through"
  (doc    "The scrutinee `(Bytes.of (list 5))` is one byte; the arm `(bin (u16 n) (bytes rest))` needs
           two bytes for its u16, so it cannot match and control falls to the catch-all, yielding 0.
           Pins that too-short input is a non-match (the arm simply does not fire), not a trap — the same
           total-or-trap discipline the corpus pins for a `bytes` segment that overruns.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 5))
            ((bin (u16 n) (bytes rest)) n)
            (_ 0)))
  (output (: 0 Int64)))

(case "a dependent-size segment that overruns the remaining bytes falls through"
  (doc    "Against `(Bytes.of (list 9 1 2))`, the pattern `(bin (u8 n) (bytes body n))` reads n = 9 but
           only two bytes remain, so `(bytes body 9)` cannot bind nine bytes and the arm falls to the
           catch-all. Pins that a dependent size larger than the remainder is a non-match, not a trap or
           a short read.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 9 1 2))
            ((bin (u8 n) (bytes body n)) (Bytes.len body))
            (_ -1)))
  (output (: -1 Int64)))

; ============================================================================================
; Exhaustiveness — a match over Bytes needs a catch-all (existing CDZ0210 rule, no special case)
; ============================================================================================

(case "a match over bytes with only a bin arm and no catch-all is non-exhaustive"
  (doc    "A `bin` pattern never covers every byte sequence — the empty sequence, a shorter sequence, or
           one whose literal segments differ all fail to match — so a match whose only arm is a `(bin …)`
           pattern does not cover the scrutinee's type and is rejected CDZ0210, exactly as a sum match
           missing a variant is. Pins that binary matching reuses the exhaustiveness rule rather than a
           special case. A generation that does not yet cover the rule declines (todo), not miscompiles.")
  (needs  binary-matching)
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
  (needs  binary-matching)
  (input  (bin (bits 1 1) (bits 0 3)))
  (error  CDZ0220))

(case "a non-final unsized bytes segment is ill-formed"
  (doc    "`(bin (bytes a) (u8 1))` places an unsized `(bytes a)` before another segment: an unsized
           bytes segment consumes all remaining bytes, so anything after it can never be reached, an
           ill-formed binary form rejected CDZ0220. Pins that an unsized `bytes` is legal only as the
           final segment (a sized `(bytes a n)` may appear anywhere).")
  (needs  binary-matching)
  (input  (bin (bytes (Bytes.of (list 1 2))) (u8 1)))
  (error  CDZ0220))

(case "a bit-field width that is not a compile-time constant is ill-formed"
  (doc    "`(bin (bits 1 k))` uses a run-time value k as a bit-field width; a `bits` width must be a
           compile-time constant so the form's byte-alignment is statically checkable. A non-constant
           width is an ill-formed binary form rejected CDZ0220. Pins that widths are static even though
           the values filling them are dynamic.")
  (needs  binary-matching)
  (input  (let ((k 3)) (bin (bits 1 k))))
  (error  CDZ0220))

; ============================================================================================
; Runtime fit — a value that does not fit its segment traps (total-or-trap construction)
; ============================================================================================

(case "constructing a u8 segment from a value above its range traps"
  (doc    "`(bin (u8 256))` asks an 8-bit unsigned segment to hold 256, which needs nine bits and has no
           8-bit encoding, so construction traps with reason \"binary value does not fit segment\" rather
           than truncating to 0. The companion of the Bytes out-of-range trap, at the segment boundary.")
  (needs  binary-matching)
  (input  (bin (u8 256)))
  (trap   "binary value does not fit segment"))

(case "constructing an unsigned segment from a negative value traps"
  (doc    "`(bin (u8 -1))` gives a negative value to an UNSIGNED segment, which has no negative encoding,
           so it traps — it does NOT wrap to 255 (that is the meaning of the SIGNED `(i8 -1)` case above).
           Pins that unsigned and signed segments differ on a negative value: the signed one encodes it in
           two's complement, the unsigned one traps.")
  (needs  binary-matching)
  (input  (bin (u8 -1)))
  (trap   "binary value does not fit segment"))

(case "constructing a bit-field from a value wider than its width traps"
  (doc    "`(bin (bits 2 1))` gives the value 2 (which needs two bits) to a 1-bit field, so it does not
           fit and construction traps. Pins that a bit-field's value is range-checked against its width at
           run time, the sub-byte companion of the u8-overflow trap.")
  (needs  binary-matching)
  (input  (bin (bits 2 1)))
  (trap   "binary value does not fit segment"))
