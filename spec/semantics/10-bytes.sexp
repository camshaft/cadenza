; Bytes — the byte-sequence value form the seed realizes so the Cadenza-authored
; compiler can construct a component's wasm bytes as an ordinary value
; (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed";
; self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous";
; options/realized-capability-set/seed-ignition-set.md). Tagged (needs bytes): the
; seed realizes this capability, so it runs these cases; a generation that does not
; realize Bytes skips them. Results are (: <value> <Type>); an out-of-range byte or
; an out-of-bounds index is a runtime trap (total-or-trap), not a static rejection —
; consistent with the dynamic seed (constitution §VII bootstrap carve-out).

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
  (doc    "Witnesses that a byte outside 0..=255 has no defined result for the dynamic
           interpreter, so it traps rather than producing an unspecified value
           (core-semantics.md #Partial Operations Have A Defined Outcome). 256 is out
           of range.")
  (needs  bytes)
  (input  (Bytes.of (list 0 256)))
  (trap   "byte value out of range"))

(case "indexing a byte sequence out of bounds traps"
  (doc    "Witnesses total-or-trap Bytes indexing on the failing side, mirroring
           List.at (collections-and-text.md #List Operations Are Total Or Trap).")
  (needs  bytes)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 5))
  (trap   "bytes index out of bounds"))
