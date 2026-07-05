; Bytes — the byte-sequence value form the seed realizes so the Cadenza-authored
; compiler can construct a component's wasm bytes as an ordinary value
; (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed";
; self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous";
; options/realized-capability-set/seed-ignition-set.md). Tagged (needs bytes): the
; seed realizes this capability, so it runs these cases; a generation that does not
; realize Bytes skips them. Results are (: <value> <Type>); an out-of-range byte or
; an out-of-bounds index is a runtime trap (total-or-trap) that survives type-checking,
; not a static rejection — the compiler emits a component that traps at that point.

(case "a byte sequence is constructed from a list of integers in range"
  (doc    "Witnesses that the seed realizes a Bytes value form: Bytes.of maps a list of
           Int64 in 0..=255 to an immutable byte sequence. This is the value the
           Cadenza-authored compiler builds a component's wasm bytes up as.")
  (needs  bytes)
  (input  (Bytes.of (list 1 2 3)))
  (output (: (Bytes.of (list 1 2 3)) Bytes)))

(case "byte sequences are equal by their bytes in order"
  (doc    "Witnesses Bytes structural equality: two byte sequences are equal exactly
           when they carry the same bytes in the same order (core-semantics.md
           #Equality Is Structural, at the Bytes value form).")
  (needs  bytes)
  (input  (= (Bytes.of (list 10 20 30)) (Bytes.of (list 10 20 30))))
  (output (: true Bool)))

(case "the length of a byte sequence is its byte count"
  (doc    "Witnesses Bytes.len — the compiler needs a byte count to lay out a wasm
           section's size prefix.")
  (needs  bytes)
  (input  (Bytes.len (Bytes.of (list 0 255 128))))
  (output (: 3 Int64)))

(case "concatenating two byte sequences appends their bytes in order"
  (doc    "Witnesses Bytes.concat — the compiler assembles a wasm module by
           concatenating encoded sections.")
  (needs  bytes)
  (input  (= (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4)))
             (Bytes.of (list 1 2 3 4))))
  (output (: true Bool)))

(case "indexing a byte sequence returns the byte at that position"
  (doc    "Witnesses total-or-trap Bytes indexing: an in-bounds index yields the byte
           as an Int64 in 0..=255.")
  (needs  bytes)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 1))
  (output (: 8 Int64)))

(case "constructing a byte sequence with a value out of range traps"
  (doc    "Witnesses that a byte outside 0..=255 has no defined result, so the program
           traps rather than producing an unspecified value (core-semantics.md #Partial
           Operations Have A Defined Outcome). 256 is out of range.")
  (needs  bytes)
  (input  (Bytes.of (list 0 256)))
  (trap   "byte value out of range"))

; The out-of-range case above tests the HIGH end (256 > 255); a byte value is in 0..=255, so the LOW
; end matters too — a NEGATIVE value has no byte representation and MUST trap, not wrap. A construction
; that checks only `> 255` and then narrows with a truncating `as u8` (or masks the low 8 bits) would
; silently turn -1 into 255 — producing an unspecified value the #Partial Operations Have A Defined
; Outcome requirement forbids. This pins the lower bound of the byte range, the companion of the 256 case.

(case "constructing a byte sequence with a negative value traps"
  (doc    "`(Bytes.of (list -1))` gives a byte value below 0 — outside 0..=255, with no byte
           representation — so it MUST trap (core-semantics.md #Partial Operations Have A Defined
           Outcome), NOT wrap to 255 via a truncating `as u8`. The low-end companion of the `256`
           out-of-range case: a byte range is bounded on BOTH sides, so both a too-large and a negative
           value are rejected, never masked into range.")
  (needs  bytes)
  (input  (Bytes.of (list -1)))
  (trap   "byte value out of range"))

(case "indexing a byte sequence out of bounds traps"
  (doc    "Witnesses total-or-trap Bytes indexing on the failing side, mirroring
           List.at (collections-and-text.md #List Operations Are Total Or Trap).")
  (needs  bytes)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 5))
  (trap   "bytes index out of bounds"))

(case "indexing a byte sequence with a negative index traps"
  (doc    "`(Bytes.at (Bytes.of (list 7 8 9)) -1)` uses a negative index — no byte at position -1 — so
           it MUST trap (total-or-trap Bytes indexing), NOT cast the negative index to a large unsigned
           offset and read an unspecified byte. The negative-index companion of the out-of-bounds `5`
           case above, mirroring the List.at negative-index case (05-compound-types).")
  (needs  bytes)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) -1))
  (trap   "bytes index out of bounds"))

; --- The empty byte sequence is an ordinary Bytes value ----------------------------------
; `(Bytes.of (list))` builds the zero-length byte sequence — a first-class Bytes value the compiler
; needs (an empty wasm section body, a zero-length name). Its length is 0, it is equal only to another
; empty byte sequence, and it is the identity element of concatenation on both sides. The empty case is
; where a length-prefix off-by-one or a concat that assumes a non-empty operand shows up, so these pin
; the degenerate boundary the non-empty cases above cannot witness.

(case "the empty byte sequence has length zero"
  (doc    "`(Bytes.of (list))` is the zero-length byte sequence; its length is 0. Pins that Bytes.len
           handles the empty sequence (a length-prefix computation must yield 0, not underflow or read a
           phantom byte).")
  (needs  bytes)
  (input  (Bytes.len (Bytes.of (list))))
  (output (: 0 Int64)))

(case "two empty byte sequences are equal"
  (doc    "`(= (Bytes.of (list)) (Bytes.of (list)))` is true: two zero-length byte sequences carry the
           same (empty) bytes in order, so they are structurally equal (core-semantics.md #Equality Is
           Structural, at the Bytes value form). Pins that Bytes equality treats the empty sequence as a
           genuine value equal to itself, not a special-cased nothing.")
  (needs  bytes)
  (input  (= (Bytes.of (list)) (Bytes.of (list))))
  (output (: true Bool)))

(case "concatenating an empty byte sequence on the right is the identity"
  (doc    "`(Bytes.concat b (Bytes.of (list)))` = b: appending zero bytes changes nothing. Pins the
           right identity of Bytes.concat — a concat that mishandles a zero-length operand (e.g. writes a
           stray length prefix) would break the compiler's section assembly.")
  (needs  bytes)
  (input  (= (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list))) (Bytes.of (list 1 2))))
  (output (: true Bool)))

(case "concatenating an empty byte sequence on the left is the identity"
  (doc    "The left-identity companion: `(Bytes.concat (Bytes.of (list)) b)` = b. Pins that
           concatenation handles a zero-length LEFT operand too, not only a zero-length right operand —
           both sides of concat treat the empty sequence as the identity.")
  (needs  bytes)
  (input  (= (Bytes.concat (Bytes.of (list)) (Bytes.of (list 3 4))) (Bytes.of (list 3 4))))
  (output (: true Bool)))
