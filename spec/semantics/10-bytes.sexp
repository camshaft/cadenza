; Bytes — the byte-sequence value form the seed realizes so the Cadenza-authored
; compiler can construct a component's wasm bytes as an ordinary value
; (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed";
; self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous";
; options/realized-capability-set/seed-ignition-set.md). Tagged (needs bytes): the
; seed realizes this capability, so it runs these cases; a generation that does not
; realize Bytes skips them. Results are (: <value> <Type>); an out-of-range byte is a
; runtime trap that survives type-checking, not a static rejection. An out-of-bounds
; index or slice is instead FALLIBLE — it yields None, not a trap (collections-and-text.md
; #Indexing And Lookup Are Fallible, Not Trapping) — so those cases are tagged
; (needs fallible-access), a capability the seed does not yet realize (they skip until a
; generation returns an Option from Bytes.at / Bytes.slice).

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

(case "indexing a byte sequence returns Some of the byte at that position"
  (doc    "Witnesses fallible Bytes indexing (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping): an in-bounds index yields the byte as an Int64 in 0..=255 wrapped in Some.")
  (needs  fallible-access)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 1))
  (output (: (Some 8) (Option Int64))))

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

(case "indexing a byte sequence out of bounds yields None"
  (doc    "Witnesses fallible Bytes indexing on the absent side, mirroring List.at
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping): an out-of-bounds
           index yields None rather than trapping.")
  (needs  fallible-access)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 5))
  (output (: (None unit) (Option Int64))))

(case "indexing a byte sequence with a negative index yields None"
  (doc    "`(Bytes.at (Bytes.of (list 7 8 9)) -1)` uses a negative index — no byte at position -1 — so
           it MUST yield None (fallible Bytes indexing), NOT cast the negative index to a large unsigned
           offset and read an unspecified byte. The negative-index companion of the out-of-bounds `5`
           case above, mirroring the List.at negative-index case (05-compound-types).")
  (needs  fallible-access)
  (input  (Bytes.at (Bytes.of (list 7 8 9)) -1))
  (output (: (None unit) (Option Int64))))

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

; --- Concatenation is associative by content --------------------------------------------
; `Bytes.concat` groups two operands, but the byte sequence it denotes depends only on the bytes in
; order, not on how the concatenations were grouped. Pinning associativity BY CONTENT (not by identity)
; is what lets a representation defer or re-group the concatenation work — a deferred-concatenation
; representation may hold `(concat (concat a b) c)` as a tree grouped either way and MUST denote the same
; value (memory-and-resource-model.md #Sharing Is Not Observable, the deferral clause). This is the
; observable law the optimization is measured against; it holds today under eager copy semantics too.

(case "concatenation is associative by content"
  (doc    "`(concat (concat a b) c)` and `(concat a (concat b c))` denote the same byte sequence: concat
           depends only on the bytes in order, not on grouping. Pins the associativity law a
           deferred-concatenation representation must preserve — re-grouping the concatenation tree is
           unobservable (memory-and-resource-model.md #Sharing Is Not Observable).")
  (needs  bytes)
  (input  (= (Bytes.concat (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))) (Bytes.of (list 5 6)))
             (Bytes.concat (Bytes.of (list 1 2)) (Bytes.concat (Bytes.of (list 3 4)) (Bytes.of (list 5 6))))))
  (output (: true Bool)))

; --- A slice is a byte sequence, indistinguishable from a copy ---------------------------
; `(Bytes.slice b start length)` yields the `length` bytes of `b` beginning at `start` as an ordinary
; Bytes value. A representation MAY realize the slice by sharing `b`'s storage (a view into it) rather
; than by copying those bytes — but memory-and-resource-model.md #Sharing Is Not Observable requires the
; shared-storage slice and a freshly-constructed copy of the same bytes to be indistinguishable by EVERY
; operation: equality, length, and indexing. These cases pin that contract under copy semantics, so a
; later view representation lands as a pure optimization that MUST keep them green. Slicing is fallible
; on the same footing as `Bytes.at` (collections-and-text.md #Indexing And Lookup Are Fallible, Not
; Trapping): an in-bounds range yields Some of the sub-sequence, and a start or length that runs past
; the end yields None rather than reading beyond the sequence. A case that operates on the sliced bytes
; unwraps the Option with `expect` (core-semantics.md #Requiring The Value Of An Optional Traps On
; Absence), so it names the in-bounds expectation at the point it requires the sub-sequence.

(case "an in-bounds slice yields Some of the bytes at that range"
  (doc    "`(Bytes.slice b 1 2)` yields Some of the 2-byte sequence starting at index 1; the unwrapped
           sub-sequence is equal by its bytes in order to the freshly-constructed `(Bytes.of (list 20
           30))`. A slice is an ordinary Bytes value; a representation that shares the parent's storage
           to realize it MUST be indistinguishable from this copy (memory-and-resource-model.md #Sharing
           Is Not Observable). `expect` unwraps the in-bounds slice.")
  (needs  fallible-access)
  (input  (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")
             (Bytes.of (list 20 30))))
  (output (: true Bool)))

(case "the length of a slice is the slice's byte count"
  (doc    "`(Bytes.len (Option.expect (Bytes.slice b 1 2) …))` = 2: length reads the slice's OWN byte count, not
           the parent's. A view representation that stored a length must report the slice's length, never
           the backing sequence's — the sharing is not observable through length.")
  (needs  fallible-access)
  (input  (Bytes.len (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: 2 Int64)))

(case "indexing a slice is relative to the slice's start"
  (doc    "`(Bytes.at (Option.expect (Bytes.slice b 1 2) …) 0)` = Some 20: index 0 of the slice is the byte at
           the slice's start, not the parent's start. Pins that a view representation adds its offset —
           indexing is relative to the slice, so sharing the parent's storage is not observable through
           indexing.")
  (needs  fallible-access)
  (input  (Bytes.at (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds") 0))
  (output (: (Some 20) (Option Int64))))

(case "a slice spanning a concatenation sees the logical bytes"
  (doc    "Slicing across the seam of `(concat a b)` — `(Bytes.slice (concat (list 1 2) (list 3 4)) 1 2)`
           = Some `(Bytes.of (list 2 3))` — reads the LOGICAL bytes in order, independent of how the
           sequence was assembled. Pins that a slice over a deferred-concatenation representation crosses
           leaf boundaries correctly, seeing bytes not physical layout (#Sharing Is Not Observable).")
  (needs  fallible-access)
  (input  (= (Option.expect (Bytes.slice (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))) 1 2)
                     "slice is in bounds")
             (Bytes.of (list 2 3))))
  (output (: true Bool)))

(case "a zero-length slice is the empty byte sequence"
  (doc    "`(Bytes.slice b 2 0)` yields Some of the empty byte sequence — equal to `(Bytes.of (list))`.
           Pins the degenerate slice: taking zero bytes at an in-bounds start yields the identity of
           concatenation, present as Some, not None.")
  (needs  fallible-access)
  (input  (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 0) "slice is in bounds")
             (Bytes.of (list))))
  (output (: true Bool)))

(case "slicing past the end of a byte sequence yields None"
  (doc    "`(Bytes.slice b 2 3)` on a 4-byte sequence asks for 3 bytes starting at index 2 — running one
           byte past the end — so it MUST yield None rather than read beyond the sequence or return a
           short result (fallible, on the same footing as Bytes.at out-of-bounds).")
  (needs  fallible-access)
  (input  (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 3))
  (output (: (None unit) (Option Bytes))))

(case "slicing with a negative start yields None"
  (doc    "`(Bytes.slice b -1 2)` uses a start below 0 — no byte at position -1 — so it MUST yield None,
           NOT cast the negative start to a large unsigned offset. The negative-index companion of the
           past-the-end case, mirroring the Bytes.at negative-index None.")
  (needs  fallible-access)
  (input  (Bytes.slice (Bytes.of (list 10 20 30 40)) -1 2))
  (output (: (None unit) (Option Bytes))))

; --- Compacting a slice preserves its value while releasing shared storage ---------------
; A slice MAY retain its parent's whole storage to represent a small range of it (a view holds the
; parent alive). `(Bytes.compact b)` derives a value equal to `b` whose storage is independent of what
; `b` was derived from — the value-preserving materialization memory-and-resource-model.md #Retained
; Storage Is Accounted For What It Holds Live requires, letting a program drop a large parent while
; keeping a small slice. Compacting changes RESOURCE USE, never the VALUE: the compacted slice is equal
; to the slice by its bytes in order (#Equality Is Structural), so `compact` is observable only through
; the resource measure, not through any value operation.

(case "compacting a slice preserves its bytes"
  (doc    "`(Bytes.compact (Option.expect (Bytes.slice b 1 2) …))` = the same in-bounds slice: compacting
           materializes the slice into independent storage, changing resource use but not the value.
           Pins that compact is value-preserving — equal by bytes in order to the un-compacted slice
           (memory-and-resource-model.md #Retained Storage Is Accounted For What It Holds Live).")
  (needs  fallible-access)
  (input  (= (Bytes.compact (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds"))
             (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: true Bool)))

(case "compacting is the identity on value for a whole byte sequence"
  (doc    "`(Bytes.compact b)` = `b`: compacting a sequence that already owns its storage changes
           nothing observable. Pins that compact is always value-preserving, whether or not the operand
           shares storage — it never alters the bytes, only (possibly) the storage backing them.")
  (needs  bytes)
  (input  (= (Bytes.compact (Bytes.of (list 1 2 3))) (Bytes.of (list 1 2 3))))
  (output (: true Bool)))
