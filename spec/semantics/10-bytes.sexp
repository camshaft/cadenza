; Bytes — the byte-sequence value form the seed realizes so the Cadenza-authored
; compiler can construct a component's wasm bytes as an ordinary value
; (bootstrap.md §"The Self-Hosted Compiler Is Authored In Cadenza";
; self-hosting-and-bootstrap.md §"Each Generation Is Derived By The Previous";
; options/realized-capability-set/seed-ignition-set.md). The
; seed realizes the Bytes capability, so it runs these cases; a generation that does not
; realize Bytes declines them. Results are (: <value> <Type>); an out-of-range byte is a
; runtime trap that survives type-checking, not a static rejection. An out-of-bounds
; index or slice is instead FALLIBLE — it yields None, not a trap (collections-and-text.md
; #Indexing And Lookup Are Fallible, Not Trapping) — so a generation that does not yet
; realize fallible access declines those cases (until it returns an Option from
; Bytes.at / Bytes.slice).
;
; THE OBSERVABLE FORM — `b"…"`. A byte sequence's canonical display is the byte-string literal
; `b"…"` (options/binary-syntax), the SAME shape the `bytes` crate's `Debug` prints and this
; specification's model of a legible byte dump: a printable ASCII byte (0x20..=0x7e) stands for
; itself, and any other byte is an escape — `\n \r \t \\ \" \0` for the named ones, `\xNN` (two
; lowercase hex digits) for the rest. `b"…"` is ALSO reader sugar: it reads to `(Bytes.of (list …))`,
; the way `a.b` reads to `(. a b)`, so the two spellings are one program and the canonical tree
; carries only `Bytes.of` — there is no new node kind. This is why the input `(Bytes.of (list …))`
; and the output `b"…"` name the same value: rendering a byte sequence and reading it back
; round-trips (the cases at the end of this file pin the equivalence). It composes with the `(bin …)`
; binary form (16-binary-matching.sexp): `b"…"` is a whole-value literal (matches by equality, splices
; into `(bytes …)`), where `(bin …)` is a structured segment application — orthogonal surfaces that
; both denote an ordinary Bytes value.
(diagnostic-quality)

(case
  "a byte sequence is constructed from a list of integers in range"
  (doc
    "Witnesses that the seed realizes a Bytes value form: Bytes.of maps a list of
           Int64 in 0..=255 to an immutable byte sequence. This is the value the
           Cadenza-authored compiler builds a component's wasm bytes up as. Its canonical
           OBSERVABLE form is the byte-string display `b\"…\"` (options/binary-syntax): a printable
           ASCII byte stands for itself and any other byte is a `\\xNN` escape, so bytes 1, 2, 3 —
           all non-printable — render `b\"\\x01\\x02\\x03\"`.")
  (input (Bytes.of #list(1 2 3)))
  (output (: b"\x01\x02\x03" Bytes)))

(case
  "byte sequences are equal by their bytes in order"
  (doc
    "Witnesses Bytes structural equality: two byte sequences are equal exactly
           when they carry the same bytes in the same order (core-semantics.md
           #Equality Is Structural, at the Bytes value form).")
  (input (= (Bytes.of #list(10 20 30)) (Bytes.of #list(10 20 30))))
  (output (: true Bool)))

(case
  "constant Bytes structural equality folds over unequal, length-differing, concat, and compact forms"
  (doc
    "The negative + composed companions of the byte-order equality above, all folding to a boolean at
           compile time (a Bytes.of/concat/compact of constants folds to a constant Core::BytesOf, so its `=`
           folds too). Weighted so one result pins five facts: same bytes → T→1; a differing byte → F→2; a
           length difference → F→4; a concat equals the flat build → T→8; a compact equals the plain build →
           T→16, summing to 31. Relocated from rcdzc
           constant_compound_equality_folds_and_a_runtime_one_emits_a_heap_walk (the Bytes arms).")
  (input
    (do
      (def
        (main)
        (+
          (if (= (Bytes.of #list(10 20 30)) (Bytes.of #list(10 20 30))) 1 0)
          (+
            (if (= (Bytes.of #list(10 20 30)) (Bytes.of #list(10 20 99))) 0 2)
            (+
              (if (= (Bytes.of #list(1 2)) (Bytes.of #list(1 2 3))) 0 4)
              (+
                (if
                  (=
                    (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4)))
                    (Bytes.of #list(1 2 3 4)))
                  8
                  0)
                (if (= (Bytes.compact (Bytes.of #list(1 2 3))) (Bytes.of #list(1 2 3))) 16 0))))))
      (export main)))
  (call main)
  (output (: 31 Int64)))

(case
  "the length of a byte sequence is its byte count"
  (doc
    "Witnesses Bytes.len — the compiler needs a byte count to lay out a wasm
           section's size prefix.")
  (input (Bytes.len (Bytes.of #list(0 255 128))))
  (output (: 3 Int64)))

(case
  "concatenating two byte sequences appends their bytes in order"
  (doc
    "Witnesses Bytes.concat — the compiler assembles a wasm module by
           concatenating encoded sections.")
  (input (= (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4))) (Bytes.of #list(1 2 3 4))))
  (output (: true Bool)))

(case
  "indexing a byte sequence returns Some of the byte at that position"
  (doc
    "Witnesses fallible Bytes indexing (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping): an in-bounds index yields the byte as an Int64 in 0..=255 wrapped in Some.")
  (input (Bytes.at (Bytes.of #list(7 8 9)) 1))
  (output (: (Some 8) (Option Int64))))

(case
  "constructing a byte sequence with a value out of range is a type error"
  (doc
    "A byte IS a UInt8 (collections-and-text.md #A Byte Is A UInt8), so `Bytes.of` takes a `(List
           UInt8)`: a byte outside 0..=255 has NO UInt8 value, so `256` is rejected at COMPILE TIME as an
           out-of-range width literal (CDZ0302) rather than trapping at run time. This is stronger than a
           runtime trap — the ill-formed byte cannot even be constructed. To turn a wider integer into a
           byte, TRUNCATE deliberately with `(UInt8.wrap n)` (total, never traps); a bare `256` is not a
           truncation request, it is an ill-typed literal.")
  (input (Bytes.of #list(0 256)))
  (error CDZ0302 (fix (kind wrap) (replacement-contains "UInt8.wrap"))))

; The out-of-range case above tests the HIGH end (256 > 255); a byte value is a UInt8, bounded on BOTH
; sides, so the LOW end matters too — a NEGATIVE literal is not a UInt8 either and is rejected the same
; way at compile time. Neither is silently masked into range: truncation into a byte is the explicit
; `UInt8.wrap`, never an implicit narrowing of an out-of-range literal.
(case
  "constructing a byte sequence with a negative value is a type error"
  (doc
    "`(Bytes.of (list -1))` gives a byte value below 0 — no UInt8 has value -1 — so it is rejected
           at COMPILE TIME (CDZ0302), the low-end companion of the `256` case. A byte is a UInt8; a UInt8
           literal is bounded on BOTH sides. NOT wrapped to 255 via a truncating `as u8`: to truncate a
           wider value into a byte you write `(UInt8.wrap -1)` = 255 explicitly.")
  (input (Bytes.of #list(-1)))
  (error CDZ0302))

(case
  "indexing a byte sequence out of bounds yields None"
  (doc
    "Witnesses fallible Bytes indexing on the absent side, mirroring List.at
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping): an out-of-bounds
           index yields None rather than trapping.")
  (input (Bytes.at (Bytes.of #list(7 8 9)) 5))
  (output (: (None unit) (Option Int64))))

(case
  "indexing a byte sequence with a negative index yields None"
  (doc
    "`(Bytes.at (Bytes.of (list 7 8 9)) -1)` uses a negative index — no byte at position -1 — so
           it MUST yield None (fallible Bytes indexing), NOT cast the negative index to a large unsigned
           offset and read an unspecified byte. The negative-index companion of the out-of-bounds `5`
           case above, mirroring the List.at negative-index case (05-compound-types).")
  (input (Bytes.at (Bytes.of #list(7 8 9)) -1))
  (output (: (None unit) (Option Int64))))

; --- The empty byte sequence is an ordinary Bytes value ----------------------------------
; `(Bytes.of (list))` builds the zero-length byte sequence — a first-class Bytes value the compiler
; needs (an empty wasm section body, a zero-length name). Its length is 0, it is equal only to another
; empty byte sequence, and it is the identity element of concatenation on both sides. The empty case is
; where a length-prefix off-by-one or a concat that assumes a non-empty operand shows up, so these pin
; the degenerate boundary the non-empty cases above cannot witness.
(case
  "the empty byte sequence has length zero"
  (doc
    "`(Bytes.of (list))` is the zero-length byte sequence; its length is 0. Pins that Bytes.len
           handles the empty sequence (a length-prefix computation must yield 0, not underflow or read a
           phantom byte).")
  (input (Bytes.len (Bytes.of #list())))
  (output (: 0 Int64)))

; A `String` supplied where `Bytes` is required has a TOTAL prelude conversion — the UTF-8 encode
; `String.to-bytes` — so the CDZ0203 mismatch carries a WRAP fix wrapping the string in it (the `…` hole
; marks where the operand goes). The REVERSE (`Bytes` where a `String` is required) is FALLIBLE
; (`from-bytes : Bytes → Option String`), so there is no one-shot wrap that type-checks → NO fix (honest,
; not a cascade). (Migrated from rcdzc a_string_where_bytes_is_expected_offers_a_to_bytes_conversion_fix.)
(case
  "a String where Bytes is expected offers a to-bytes conversion wrap (operator-arg position)"
  (input (do (def (f) (Bytes.len "hi")) (export f)))
  (error CDZ0203 (fix (kind wrap) (replacement "(String.to-bytes …)") (unverified))))

(case
  "a String where Bytes is expected offers a to-bytes conversion wrap (annotated-param call site)"
  (input (do (def (f (: b Bytes)) b) (def (main) (f "hi")) (export main)))
  (error CDZ0203 (fix (kind wrap) (replacement "(String.to-bytes …)") (unverified))))

(case
  "a Bytes where a String is expected offers no fix (the decode is fallible)"
  (input (do (def (f (: s String)) s) (def (main (: b Bytes)) (f b)) (export main)))
  (error CDZ0203 (no-fix)))

(case
  "two empty byte sequences are equal"
  (doc
    "`(= (Bytes.of (list)) (Bytes.of (list)))` is true: two zero-length byte sequences carry the
           same (empty) bytes in order, so they are structurally equal (core-semantics.md #Equality Is
           Structural, at the Bytes value form). Pins that Bytes equality treats the empty sequence as a
           genuine value equal to itself, not a special-cased nothing.")
  (input (= (Bytes.of #list()) (Bytes.of #list())))
  (output (: true Bool)))

(case
  "concatenating an empty byte sequence on the right is the identity"
  (doc
    "`(Bytes.concat b (Bytes.of (list)))` = b: appending zero bytes changes nothing. Pins the
           right identity of Bytes.concat — a concat that mishandles a zero-length operand (e.g. writes a
           stray length prefix) would break the compiler's section assembly.")
  (input (= (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list())) (Bytes.of #list(1 2))))
  (output (: true Bool)))

(case
  "concatenating an empty byte sequence on the left is the identity"
  (doc
    "The left-identity companion: `(Bytes.concat (Bytes.of (list)) b)` = b. Pins that
           concatenation handles a zero-length LEFT operand too, not only a zero-length right operand —
           both sides of concat treat the empty sequence as the identity.")
  (input (= (Bytes.concat (Bytes.of #list()) (Bytes.of #list(3 4))) (Bytes.of #list(3 4))))
  (output (: true Bool)))

; --- Concatenation is associative by content --------------------------------------------
; `Bytes.concat` groups two operands, but the byte sequence it denotes depends only on the bytes in
; order, not on how the concatenations were grouped. Pinning associativity BY CONTENT (not by identity)
; is what lets a representation defer or re-group the concatenation work — a deferred-concatenation
; representation may hold `(concat (concat a b) c)` as a tree grouped either way and MUST denote the same
; value (memory-and-resource-model.md #Sharing Is Not Observable, the deferral clause). This is the
; observable law the optimization is measured against; it holds today under eager copy semantics too.
(case
  "concatenation is associative by content"
  (doc
    "`(concat (concat a b) c)` and `(concat a (concat b c))` denote the same byte sequence: concat
           depends only on the bytes in order, not on grouping. Pins the associativity law a
           deferred-concatenation representation must preserve — re-grouping the concatenation tree is
           unobservable (memory-and-resource-model.md #Sharing Is Not Observable).")
  (input
    (=
      (Bytes.concat
        (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4)))
        (Bytes.of #list(5 6)))
      (Bytes.concat
        (Bytes.of #list(1 2))
        (Bytes.concat (Bytes.of #list(3 4)) (Bytes.of #list(5 6))))))
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
(case
  "an in-bounds slice yields Some of the bytes at that range"
  (doc
    "`(Bytes.slice b 1 2)` yields Some of the 2-byte sequence starting at index 1; the unwrapped
           sub-sequence is equal by its bytes in order to the freshly-constructed `(Bytes.of (list 20
           30))`. A slice is an ordinary Bytes value; a representation that shares the parent's storage
           to realize it MUST be indistinguishable from this copy (memory-and-resource-model.md #Sharing
           Is Not Observable). `expect` unwraps the in-bounds slice.")
  (input
    (=
      (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 1 2) "slice is in bounds")
      (Bytes.of #list(20 30))))
  (output (: true Bool)))

(case
  "the length of a slice is the slice's byte count"
  (doc
    "`(Bytes.len (Option.expect (Bytes.slice b 1 2) …))` = 2: length reads the slice's OWN byte count, not
           the parent's. A view representation that stored a length must report the slice's length, never
           the backing sequence's — the sharing is not observable through length.")
  (input
    (Bytes.len (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: 2 Int64)))

(case
  "indexing a slice is relative to the slice's start"
  (doc
    "`(Bytes.at (Option.expect (Bytes.slice b 1 2) …) 0)` = Some 20: index 0 of the slice is the byte at
           the slice's start, not the parent's start. Pins that a view representation adds its offset —
           indexing is relative to the slice, so sharing the parent's storage is not observable through
           indexing.")
  (input
    (Bytes.at
      (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 1 2) "slice is in bounds")
      0))
  (output (: (Some 20) (Option Int64))))

(case
  "a slice at a RUNTIME start re-bases indexing per call"
  (doc
    "The runtime companion of the re-based-indexing pin above (whose start is the constant 1 and
           folds): the slice start is a boundary parameter, so ONE compiled read `(Bytes.at s 0)` must
           re-base against a PER-CALL offset — at `a = 2` slice[0] is the parent's byte 2 (30), at `a = 0`
           it is byte 0 (10). A view carrying a baked offset (or reading the parent's index 0 regardless
           of the slice start) would return 10 for both calls.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(10 20 30 40)) a 2)
          ((Some s) (match (Bytes.at s 0) ((Some v) v) ((None u) -2)))
          ((None u) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 30 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (live-objects 0))

(case
  "a slice of a runtime-start SLICE composes both offsets"
  (doc
    "The view-of-a-view face: an inner `(Bytes.slice outer 1 2)` over an outer runtime-start slice
           must re-base against the OUTER VIEW's coordinates, composing both offsets down to the parent —
           a=1: outer windows [2..5]=(2,3,4,5), inner [1..2] of that is (3,4), byte 0 = 3; a=0: outer
           (1,2,3,4), inner (2,3), byte 0 = 2. A representation composing only ONE level (inner offset
           against the parent, or dropping the outer offset) answers 2 at a=1. Two calls witness the
           outer offset participating.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(1 2 3 4 5 6)) a 4)
          ((Some outer)
            (match
              (Bytes.slice outer 1 2)
              ((Some inner) (match (Bytes.at inner 0) ((Some v) v) ((None u) -3)))
              ((None u) -2)))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a slice OF a slice over a CONCAT rope composes offsets across the seam"
  (doc
    "The view-of-a-view composition case above runs over a FLAT parent; here the parent is a
           two-segment ROPE `(concat (10,20,30) (40,50,60,70))` and the OUTER slice (1,5) CROSSES the
           seam — so the inner slice (1,3) must compose its offset through a view whose backing storage
           changes segment mid-window. inner = (30,40,50): len 3, ends 30+50 → 100·3 + 80 = 380. An
           offset composition that resolved against only one segment (or re-based the inner window at
           the seam) would read the wrong bytes. The rope face of view-of-view offset composition.
           The chunks are RUNTIME-SELECTED (the pick idiom of the runtime-assembled-rope case below,
           per this file's own deferral note) so the concat cannot fold to a flat leaf — the seam is
           genuinely deferred, not pre-joined.")
  (input
    (do
      (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
      (def
        (main (: s Int64))
        (do
          (def
            rope
            (Bytes.concat
              (pick s (Bytes.of #list(10 20 30)) (Bytes.of #list(99)))
              (pick s (Bytes.of #list(40 50 60 70)) (Bytes.of #list(99)))))
          (match
            (Bytes.slice rope 1 5)
            ((Some outer)
              (match
                (Bytes.slice outer 1 3)
                ((Some inner)
                  (+
                    (* 100 (Bytes.len inner))
                    (+
                      (match (Bytes.at inner 0) ((Some v) v) ((None _u) -1))
                      (match (Bytes.at inner 2) ((Some v) v) ((None _u) -1)))))
                ((None _u) -2)))
            ((None _u) -3))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 380 Int64))
  (live-objects known-leak))

(case
  "the composed slice view EQUALS its flat twin and keys a Map by canonical content"
  (doc
    "The identity witness of the rope view-of-view case above: the doubly-sliced seam-crossing
           window must EQUAL `(Bytes.of (list 30 40 50))` by canonical `=` (10) AND find a value stored
           under that flat literal as a Map KEY (+7 → 17). The key path hashes the canonical byte form —
           a view that retained rope-offset residue (segment boundary, parent offsets) in its canonical
           form would hash differently and miss even while element reads agree. Completes the
           slice-composition family: offsets compose (above), and the RESULT is an ordinary value with
           full canonical identity. Chunks runtime-selected (pick idiom) so the rope cannot pre-join.")
  (input
    (do
      (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
      (def
        (main (: s Int64))
        (do
          (def
            rope
            (Bytes.concat
              (pick s (Bytes.of #list(10 20 30)) (Bytes.of #list(99)))
              (pick s (Bytes.of #list(40 50 60 70)) (Bytes.of #list(99)))))
          (def
            inner
            (match
              (Bytes.slice rope 1 5)
              ((Some outer)
                (match (Bytes.slice outer 1 3) ((Some i) i) ((None _u) (Bytes.of #list()))))
              ((None _u) (Bytes.of #list()))))
          (+
            (* 10 (if (= inner (Bytes.of #list(30 40 50))) 1 0))
            (match
              (Map.lookup (Map.insert Map.empty (Bytes.of #list(30 40 50)) 7) inner)
              ((Some v) v)
              ((None _u) -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 17 Int64))
  (live-objects known-leak))

(case
  "Bytes.concat of two runtime SLICES splices window content in order"
  (doc
    "The concat-of-views face (the seam case below slices a CONCAT; this concatenates two SLICES):
           s1 = window [a..a+2] of (1,2,3,4), s2 = the (7,8) window of (5,6,7,8) — `(concat s1 s2)` at
           a=0 is (1,2,7,8), so index 2 reads 7. The concat must copy/point to each VIEW's content (not
           either parent's full buffer); a concat wiring parents in would put 3 at index 2 (s1's parent
           byte) or shift the s2 window.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(1 2 3 4)) a 2)
          ((Some s1)
            (match
              (Bytes.slice (Bytes.of #list(5 6 7 8)) 2 2)
              ((Some s2) (match (Bytes.at (Bytes.concat s1 s2) 2) ((Some v) v) ((None u) -3)))
              ((None u) -2)))
          ((None u) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a runtime slice VIEW crosses a helper-function boundary intact"
  (doc
    "The escape face: the slice is passed to a helper `(first-byte b)` whose body indexes its Bytes
           PARAMETER — the view's re-based coordinates must survive the call ABI (a=2 → parent byte 2 =
           30, a=0 → 10). A lowering that passed the parent handle + lost the offset at the boundary
           would answer 10 for both.")
  (input
    (do
      (def (first-byte (: b Bytes)) (match (Bytes.at b 0) ((Some v) v) ((None u) -1)))
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(10 20 30 40)) a 2)
          ((Some s) (first-byte s))
          ((None u) -2)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 30 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (live-objects 0))

; --- Slice-view LIVENESS: the view keeps its parent's bytes alive (the Perceus face) ---------------
; A slice is a view over its parent's storage, so the memory manager must keep the parent's bytes
; reachable for as long as any view lives — including past the parent BINDING's last direct use, and
; past the parent's whole SCOPE. A drop that reclaimed the parent at its last direct use would leave
; the view dangling (a use-after-free reading freed or reused bytes). These pin the no-stale-read
; contract at the three sharpest liveness shapes.
(case
  "a slice reads correctly after its parent binding's last direct use"
  (doc
    "The reclaim-window shape: `parent` (a CONCAT rope, so its storage is runtime-built) is last
           directly used at the `Bytes.slice` call; the view is read AFTER that point. An eager drop of
           `parent` at its last direct use would free the rope under the view — the read must still see
           byte 30 (a=1 windows (20,30), index 1). The interleaved variant reads `Bytes.len parent`
           BETWEEN slicing and the view read (34 = 30+4), forcing the overlap the borrow analysis must
           respect.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((parent (Bytes.concat (Bytes.of #list(10 20)) (Bytes.of #list(30 40)))))
          (match
            (Bytes.slice parent a 2)
            ((Some s) (match (Bytes.at s 1) ((Some v) v) ((None u) -3)))
            ((None u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 30 Int64))
  (live-objects 0))

(case
  "two slices of one parent both read after the parent is dead"
  (doc
    "The shared-storage face: two views of ONE parent, both read after the parent binding's last
           use — each must see its own window (a=0: s1=(1,2), s2=(3,4) → 13; a=2: (3,4)/(5,6) → 35). The
           parent's storage is kept alive by EITHER view; a refcount that credited only the first slice
           (or double-freed on the second view's drop) would corrupt one read.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((parent (Bytes.of #list(1 2 3 4 5 6))))
          (match
            (Bytes.slice parent a 2)
            ((Some s1)
              (match
                (Bytes.slice parent (+ a 2) 2)
                ((Some s2)
                  (match
                    (Bytes.at s1 0)
                    ((Some v1) (match (Bytes.at s2 0) ((Some v2) (+ (* v1 10) v2)) ((None u) -4)))
                    ((None u) -3)))
                ((None u) -2)))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 13 Int64))
  (call main (: 2 Int64))
  (output (: 35 Int64))
  (live-objects 0))

(case
  "brv1 a sibling view's arm-end reclaim leaves the surviving view's window intact"
  (doc
    "The post-reclaim ALIAS-READ-ORDERING face of the shared-storage pin above (there both views
           are read with both matches still OPEN; here the sibling's match CLOSES first): v2's arm only
           scalar-extracts (`Bytes.at`), so under the arm-borrow relax its Option shell AND the view
           deep-drop at arm end — and THEN the surviving sibling v1 (a view of the SAME parent) is read.
           A deep-drop that recursed into the shared parent storage (instead of just releasing v2's
           reference) would corrupt or dangle v1's read. n=0: 30+10=40; n=2: 30+30=60 — and the heap
           fully balances (0), unlike the both-open shape's residual 2.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((b (Bytes.of #list(10 20 30 40 50 60))))
          (match
            (Bytes.slice b 0 3)
            ((Some v1)
              (+
                (match
                  (Bytes.slice b 2 4)
                  ((Some v2) (match (Bytes.at v2 0) ((Some v) v) ((None u) -4)))
                  ((None u) -2))
                (match (Bytes.at v1 n) ((Some v) v) ((None u) -3))))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 40 Int64))
  (call main (: 2 Int64))
  (output (: 60 Int64))
  (live-objects 0))

(case
  "brv2 an inner slice-of-a-slice's reclaim leaves the OUTER view readable"
  (doc
    "The parent-chain twin of brv1: the reclaimed view w2 is a slice OF the outer view w (not a
           sibling), so its arm-end deep-drop releases a reference INTO w's chain — and then w itself is
           read. An over-release up the view chain (freeing w's window or its transitive parent storage
           when w2's shell drops) would corrupt the later `(Bytes.at w n)`. w=(10,20,30,40), w2=(20,30,40),
           inner read 20; n=0: 20+10=30; n=3: 20+40=60; heap fully balances.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((b (Bytes.of #list(10 20 30 40 50 60))))
          (match
            (Bytes.slice b 0 4)
            ((Some w)
              (+
                (match
                  (Bytes.slice w 1 3)
                  ((Some w2) (match (Bytes.at w2 0) ((Some v) v) ((None u) -4)))
                  ((None u) -2))
                (match (Bytes.at w n) ((Some v) v) ((None u) -3))))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 30 Int64))
  (call main (: 3 Int64))
  (output (: 60 Int64))
  (live-objects 0))

(case
  "bse1 the COMBINED reclaim levers — a SumExpect view-reclaim fires INSIDE an arm-borrow arm, and the outer view is read AFTER"
  (doc
    "The two slice-view reclaim levers composed in ONE arm: the outer match's arm holds w (the
           arm-borrow lever's shell candidate), and inside it a `(Bytes.at (Option.expect (Bytes.slice w
           1 2) \"m\") 0)` fires the SumExpect dup-at-extract + shell-drop + view-drop lever against a
           slice OF w — then w itself is read. Either lever over-releasing into w's chain (or the two
           levers double-marking one node — the b2 double-mark class) would corrupt or dangle the later
           `(Bytes.at w n)`. w=(10,20,30,40), inner view (20,30) reads 20; n=0: 20+10=30; n=3: 20+40=60;
           the heap fully balances.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((b (Bytes.of #list(10 20 30 40 50 60))))
          (match
            (Bytes.slice b 0 4)
            ((Some w)
              (+
                (match
                  (Bytes.at (Option.expect (Bytes.slice w 1 2) "m") 0)
                  ((Some v) v)
                  ((None u) -4))
                (match (Bytes.at w n) ((Some v) v) ((None u) -3))))
            ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 30 Int64))
  (call main (: 3 Int64))
  (output (: 60 Int64))
  (live-objects 0))

(case
  "bse2 a THOUSAND SumExpect view-reclaims into one shared parent, and the parent is read AFTER the loop"
  (doc
    "The loop-scale face of the SumExpect view-reclaim: each of 1000 recursion steps expect-extracts
           a fresh slice view of the SAME parent p, scalar-reads it, and reclaims shell+view at the step —
           1000 reclaim cycles releasing into p's chain — and then p itself is read. A per-cycle
           over-release would corrupt the parent long before the loop ends (and the final `(Bytes.at p 0)`
           read); a per-cycle leak would balloon the balance. 1000×2 + 1 = 2001, heap fully balances.")
  (input
    (do
      (def
        (go (: b Bytes) (: i Int64))
        (if
          (= i 0)
          0
          (+
            (match (Bytes.at (Option.expect (Bytes.slice b 1 2) "m") 0) ((Some v) v) ((None u) -4))
            (go b (- i 1)))))
      (def
        (main (: n Int64))
        (let
          ((p (Bytes.of #list(1 2 3))))
          (+ (go p n) (match (Bytes.at p 0) ((Some v) v) ((None u) -9)))))
      (export main)))
  (call main (: 1000 Int64))
  (output (: 2001 Int64))
  (live-objects 0))

(case
  "a slice returned from a helper OUTLIVES the helper's local parent"
  (doc
    "The strongest escape: `mk-slice` builds `parent` as a LOCAL and returns a view of it — the
           parent binding dies at the helper's return while the view crosses the boundary. The caller
           reads both the length (2) and, per call, the CONTENT (a=2 → 30, a=0 → 10): the parent's bytes
           must survive the helper's frame teardown because the escaping view holds them. A reclaim at
           scope exit (rather than at last-reference) would hand the caller a dangling view.")
  (input
    (do
      (def
        (mk-slice (: a Int64))
        (let
          ((parent (Bytes.of #list(10 20 30 40))))
          (match (Bytes.slice parent a 2) ((Some s) s) ((None u) (Bytes.of #list())))))
      (def
        (main (: a Int64))
        (+
          (* 100 (Bytes.len (mk-slice a)))
          (match (Bytes.at (mk-slice a) 0) ((Some v) v) ((None u) -1))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 230 Int64))
  (call main (: 0 Int64))
  (output (: 210 Int64))
  (live-objects known-leak))

(case
  "a SumExpect-unwrapped slice view BOUND and read TWICE (count>1) is NOT reclaimed (SumExpect single-consumer/escape must-hold)"
  (doc
    "The MUST-HOLD guard for the SumExpect view-reclaim (#4939, v-memory-safety): the reclaim marks a
           SumExpect-extracted Bytes slice-view a dup-site ONLY when it is the operand of exactly ONE `Bytes.at`
           (`count_node_refs == 1` — single-consumer, scalar-extracted, not-escaped), then reclaims it via the
           per-op `reclaim_bytes` drop. This case fences the count>1 EXCLUSION: `mk-slice` returns an
           `(Option.expect (Bytes.slice parent a 2) …)` view of a LOCAL parent, and the caller BINDS it (`v`)
           and reads it TWICE — `(Bytes.len v)` AND `(Bytes.at v 0)` — so `count_node_refs(v) > 1`. The reclaim
           MUST NOT fire (a per-op `reclaim_bytes` drop on a multi-read view would DOUBLE-DROP; and the view
           escapes the helper holding its local parent, so freeing it under a live use = UAF). So `v` stays
           leaking, both reads see the live view: `100·2 + 30` = 230 / `100·2 + 10` = 210. A regression that
           reclaimed a count>1 view would move the value off 230/210 or trip an `assert_node_live` UAF trap.
           The single-consumer scalar-extracted twin (`Bytes.at` over a directly-consumed slice) is bar3/bar4,
           which #4939 correctly drops to 0 — this count>1 case is the complementary must-hold that stays leaking.
           KNOWN-LEAK 1 (was 2): the Option-A SumExpect SHELL-reclaim (#4956) net-0-reclaims the orphaned Some-shell
           here too — the SumExpect node is referenced ONCE by the `let` (its parent is the Let, not a scalar-read),
           so it lands in the SHELL-set and its shell drops soundly (view untouched). The residual 1 IS the
           un-reclaimed count>1 VIEW — the must-hold witness: a regression that wrongly reclaimed the multi-read
           view would drop this to 0 (or trip `assert_node_live`), so known-leak-1 still guards the count>1 view.")
  (input
    (do
      (def
        (mk-slice (: a Int64))
        (let
          ((parent (Bytes.of #list(10 20 30 40))))
          (Option.expect (Bytes.slice parent a 2) "in bounds")))
      (def
        (main (: a Int64))
        (let ((v (mk-slice a))) (+ (* 100 (Bytes.len v)) (Option.expect (Bytes.at v 0) "v"))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 230 Int64))
  (call main (: 0 Int64))
  (output (: 210 Int64))
  (live-objects known-leak))

; -- Bytes.at over an OWNED-TEMPORARY rope-producer reclaims it (migrated from rcdzc bytes_at_over_an_owned_
; temporary_* reclaim tests). Every rope-producer — Bytes.concat / Bytes.slice / Bytes.compact / String.to-bytes
; — CONSUMES its operand and returns a FRESH owned Bytes leaf, so a borrowing `Bytes.at` (bytes-len + bytes-get,
; the read byte is a COPIED i32) over one is an owned temporary that must be dropped after the borrow or it
; leaks a leaf per read. A constant `Bytes.of`/`b"…"` folds away (no runtime handle), so `build` threads the
; bytes through recursion to make a genuine runtime rope. The value is unchanged by the reclaim; the stress
; loops detect a leak (drift/OOM) or double-free (trap). (The rcdzc tests asserted the reclaim via a
; component_imports_op(...,'drop') module-shape check — subsumed here by the live-objects reclaim witness.)
(case
  "bar1 Bytes.at over an owned-temporary Bytes.concat result reclaims it"
  (doc
    "`build` concats a fresh single byte 10 three times -> [10,10,10]; `(Bytes.at (build …) 1)` = Some 10,
           the owned rope dropped after the borrowing at. Value 10; a leaked leaf would show live cells.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10)))) acc))
      (def (main) (Option.expect (Bytes.at (build 0 3 (Bytes.of #list())) 1) "v"))
      (export main)))
  (call main)
  (output (: 10 Int64))
  (live-objects 0))

(case
  "bar2 a borrowed bytes read by Bytes.at AND Bytes.len is not freed early"
  (doc
    "`(let ((bs (build 0 3))) (+ (Option.expect (Bytes.at bs 1)) (Bytes.len bs)))` reads the borrowed
           `bs` twice — the at must not free it under the still-live binding (else double-free). 10 + 3 = 13.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10)))) acc))
      (def
        (main)
        (let
          ((bs (build 0 3 (Bytes.of #list()))))
          (+ (Option.expect (Bytes.at bs 1) "v") (Bytes.len bs))))
      (export main)))
  (call main)
  (output (: 13 Int64))
  (live-objects 0))

(case
  "bar3 Bytes.at over an owned-temporary Bytes.slice result reclaims the slice"
  (doc
    "The slice rope-producer face: `build` -> [10,20,30]x3 (9 bytes); `(Bytes.slice … 1 3)` = window
           [20,30,10]; `(Bytes.at that 1)` = 30, the owned slice dropped after the borrow. Value 30.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10 20 30)))) acc))
      (def
        (main)
        (Option.expect
          (Bytes.at (Option.expect (Bytes.slice (build 0 3 (Bytes.of #list())) 1 3) "s") 1)
          "v"))
      (export main)))
  (call main)
  (output (: 30 Int64))
  (live-objects 0))

(case
  "bar4 5000x Bytes.at over an owned-temporary Bytes.slice each reclaims (no leak drift)"
  (doc
    "Leak/UAF stress: 5000x build+slice a fresh owned bytes and read byte 1 of the window (30). A leaked
           slice leaf would OOM/drift; a double-free would trap. Sum = 150000.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10 20 30)))) acc))
      (def
        (drive (: j Int64) (: m Int64) (: tot Int64))
        (if
          (< j m)
          (drive
            (+ j 1)
            m
            (+
              tot
              (Option.expect
                (Bytes.at (Option.expect (Bytes.slice (build 0 3 (Bytes.of #list())) 1 3) "s") 1)
                "v")))
          tot))
      (def (main) (drive 0 5000 0))
      (export main)))
  (call main)
  (output (: 150000 Int64))
  (live-objects 0))

(case
  "bar5 Bytes.at over an owned-temporary Bytes.compact result reclaims it"
  (doc
    "The compact rope-producer face: `build` -> [10,20]x3 (6 bytes); `(Bytes.compact …)` flattens to a
           fresh leaf; `(Bytes.at that 1)` = 20, the owned compact dropped after the borrow. Value 20.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10 20)))) acc))
      (def (main) (Option.expect (Bytes.at (Bytes.compact (build 0 3 (Bytes.of #list()))) 1) "v"))
      (export main)))
  (call main)
  (output (: 20 Int64))
  (live-objects 0))

(case
  "bar6 Bytes.at over an owned-temporary String.to-bytes result reclaims it"
  (doc
    "The String.to-bytes rope-producer face: `build` -> \"ababab\"; `(String.to-bytes …)` re-tags the
           byte-rope out as a fresh flat Bytes; `(Bytes.at that 1)` = 98 ('b'), the owned bytes dropped after
           the borrow. Value 98.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc String))
        (if (< i n) (build (+ i 1) n (String.concat acc "ab")) acc))
      (def (main) (Option.expect (Bytes.at (String.to-bytes (build 0 3 "")) 1) "v"))
      (export main)))
  (call main)
  (output (: 98 Int64))
  (live-objects 0))

(case
  "bar7 5000x Bytes.at over an owned-temporary Bytes.compact each reclaims (no leak drift)"
  (doc
    "Leak stress for the compact face: 5000x build+compact a fresh owned bytes and read byte 1 (20). A
           leaked leaf per call would OOM/drift. Sum = 100000.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (build (+ i 1) n (Bytes.concat acc (Bytes.of #list(10 20)))) acc))
      (def
        (drive (: j Int64) (: m Int64) (: tot Int64))
        (if
          (< j m)
          (drive
            (+ j 1)
            m
            (+ tot (Option.expect (Bytes.at (Bytes.compact (build 0 3 (Bytes.of #list()))) 1) "v")))
          tot))
      (def (main) (drive 0 5000 0))
      (export main)))
  (call main)
  (output (: 100000 Int64))
  (live-objects 0))

(case
  "String.from-bytes of a runtime slice decodes the WINDOW only"
  (doc
    "The decode face: `String.from-bytes` over a runtime-start slice of (x,a,b,y) at a=1 must
           decode exactly the 2-byte window \"ab\" (byte-len 2) — not the parent's 4 bytes and not a
           mis-based window. Composes the slice view with the total UTF-8 decode (both pinned separately;
           the view must present its window as the decoder's whole input).")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(120 97 98 121)) a 2)
          ((Some s) (match (String.from-bytes s) ((Some str) (String.byte-len str)) ((None u) -3)))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a slice spanning a concatenation sees the logical bytes"
  (doc
    "Slicing across the seam of `(concat a b)` — `(Bytes.slice (concat (list 1 2) (list 3 4)) 1 2)`
           = Some `(Bytes.of (list 2 3))` — reads the LOGICAL bytes in order, independent of how the
           sequence was assembled. Pins that a slice over a deferred-concatenation representation crosses
           leaf boundaries correctly, seeing bytes not physical layout (#Sharing Is Not Observable).")
  (input
    (=
      (Option.expect
        (Bytes.slice (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4))) 1 2)
        "slice is in bounds")
      (Bytes.of #list(2 3))))
  (output (: true Bool)))

(case
  "a zero-length slice is the empty byte sequence"
  (doc
    "`(Bytes.slice b 2 0)` yields Some of the empty byte sequence — equal to `(Bytes.of (list))`.
           Pins the degenerate slice: taking zero bytes at an in-bounds start yields the identity of
           concatenation, present as Some, not None.")
  (input
    (=
      (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 2 0) "slice is in bounds")
      (Bytes.of #list())))
  (output (: true Bool)))

(case
  "slicing past the end of a byte sequence yields None"
  (doc
    "`(Bytes.slice b 2 3)` on a 4-byte sequence asks for 3 bytes starting at index 2 — running one
           byte past the end — so it MUST yield None rather than read beyond the sequence or return a
           short result (fallible, on the same footing as Bytes.at out-of-bounds).")
  (input (Bytes.slice (Bytes.of #list(10 20 30 40)) 2 3))
  (output (: (None unit) (Option Bytes))))

(case
  "slicing with a negative start yields None"
  (doc
    "`(Bytes.slice b -1 2)` uses a start below 0 — no byte at position -1 — so it MUST yield None,
           NOT cast the negative start to a large unsigned offset. The negative-index companion of the
           past-the-end case, mirroring the Bytes.at negative-index None.")
  (input (Bytes.slice (Bytes.of #list(10 20 30 40)) -1 2))
  (output (: (None unit) (Option Bytes))))

; The slice bounds above are compile-time constants (the fold decides in/out of bounds statically). A
; slice whose START and LENGTH are RUNTIME parameters cannot fold: the bounds check `0 <= start` and
; `start + len <= length` runs as emitted instructions over the erased magnitudes, returning Some/None.
; This pins the runtime boundary arithmetic — the same-footing fallibility of the const cases, now
; decided at run time — including the two faces a signed→unsigned confusion would corrupt: a NEGATIVE
; length and an i64-MAX length must both yield None (the addition `start + len` is a checked signed
; compare, never a wrap into a huge unsigned window that spuriously "fits").
(case
  "a runtime-parameter start and length are bounds-checked at run time, yielding Some or None"
  (doc
    "`(Bytes.slice b start len)` with `start` and `len` runtime Int64 parameters over a 5-byte
           sequence: the fold cannot decide, so the bounds check runs as emitted code and returns Some
           (whose `Bytes.len` is the slice's length) or None. The grid pins each boundary — whole
           (0,5)=5, an empty slice AT the end (5,0)=0 present as Some, one byte too long (0,6)=None, the
           last single byte (4,1)=1, one past (4,2)=None — and the two SIGNEDNESS faces a `start+len`
           computed in unsigned space would miscompile: a NEGATIVE length (2,-1) and the i64-MAX length
           (0, 9223372036854775807) must BOTH be None, not a wrap that appears to fit. A start at the
           length with any positive len (5,1)=None. Companion of the const past-end/negative-start cases,
           decided at run time; the erased bounds arithmetic is a checked signed compare.")
  (input
    (do
      (def
        (sl (: start Int64) (: len Int64))
        (match
          (Bytes.slice (Bytes.of #list(10 20 30 40 50)) start len)
          ((Option.Some s) (Bytes.len s))
          ((Option.None) -1)))
      (def (main (: start Int64) (: len Int64)) (sl start len))
      (export main)))
  (call main (: 0 Int64) (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 5 Int64) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64) (: 6 Int64))
  (output (: -1 Int64))
  (call main (: 4 Int64) (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64) (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 2 Int64) (: -1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64) (: 9223372036854775807 Int64))
  (output (: -1 Int64))
  (call main (: 5 Int64) (: 1 Int64))
  (output (: -1 Int64))
  (live-objects 0))

; The seam case at "a slice spanning a concatenation sees the logical bytes" slices across the seam of a
; concat of CONSTANT chunks, which the fold may materialize before slicing. A GENUINELY-runtime byte
; rope — a `Bytes.concat` of chunks selected at run time (an `if` the fold cannot decide) — assembles a
; multi-chunk rope that survives to the emitted `bytes-slice`, so the slice must walk leaf boundaries of
; a real deferred concatenation, not a flat leaf the fold pre-joined. This pins the seam-crossing walk on
; the runtime representation the const case cannot exercise: a slice that begins in the left chunk and
; ends in the right must read the logical bytes in order across the physical leaf boundary.
(case
  "a slice crosses the seam of a runtime-assembled byte rope"
  (doc
    "`(Bytes.slice rope 1 2)` over a rope built at run time by `(Bytes.concat left right)` whose
           chunks are chosen by a run-time `if` — spanning index 1 (last byte of the left chunk) through
           index 2 (first byte of the right chunk) — yields Some `[20, 30]`: the slice reads the logical
           bytes across the leaf boundary of a genuine deferred concatenation, not a flat leaf a fold
           pre-joined (#Sharing Is Not Observable, the deferral clause). The grid reads the slice's length
           (2), its byte at index 0 (20, from the left chunk) and index 1 (30, from the right chunk), and
           an out-of-bounds slice `(Bytes.slice rope 3 2)` past the 4-byte rope yielding None (0). Because
           the chunks are runtime-selected the concatenation cannot fold to a constant, so the seam
           crossing is decided by the emitted `bytes-slice` over the assembled rope.")
  (input
    (do
      (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
      (def
        (rope (: s Int64))
        (Bytes.concat
          (pick s (Bytes.of #list(10 20)) (Bytes.of #list(99 99)))
          (pick s (Bytes.of #list(30 40)) (Bytes.of #list(99 99)))))
      (def
        (main (: s Int64))
        #tuple((match (Bytes.slice (rope s) 1 2) ((Option.Some x) (Bytes.len x)) ((Option.None) -1))
          (match
            (Bytes.slice (rope s) 1 2)
            ((Option.Some x) (Option.expect (Bytes.at x 0) "0"))
            ((Option.None) -1))
          (match
            (Bytes.slice (rope s) 1 2)
            ((Option.Some x) (Option.expect (Bytes.at x 1) "1"))
            ((Option.None) -1))
          (match (Bytes.slice (rope s) 3 2) ((Option.Some _) 1) ((Option.None) 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (tuple 2 20 30 0) (Tuple Int64 Int64 Int64 Int64)))
  (live-objects known-leak))

; --- Compacting a slice preserves its value while releasing shared storage ---------------
; A slice MAY retain its parent's whole storage to represent a small range of it (a view holds the
; parent alive). `(Bytes.compact b)` derives a value equal to `b` whose storage is independent of what
; `b` was derived from — the value-preserving materialization memory-and-resource-model.md #Retained
; Storage Is What A Value's Representation Holds Live requires, letting a program drop a large parent
; while keeping a small slice. Compacting changes STORAGE USE, never the VALUE: the compacted slice is
; equal to the slice by its bytes in order (#Equality Is Structural), so `compact` is not observable
; through any value operation.
(case
  "compacting a slice preserves its bytes"
  (doc
    "`(Bytes.compact (Option.expect (Bytes.slice b 1 2) …))` = the same in-bounds slice: compacting
           materializes the slice into independent storage, changing resource use but not the value.
           Pins that compact is value-preserving — equal by bytes in order to the un-compacted slice
           (memory-and-resource-model.md #Retained Storage Is What A Value's Representation Holds Live).")
  (input
    (=
      (Bytes.compact
        (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 1 2) "slice is in bounds"))
      (Option.expect (Bytes.slice (Bytes.of #list(10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: true Bool)))

(case
  "compacting is the identity on value for a whole byte sequence"
  (doc
    "`(Bytes.compact b)` = `b`: compacting a sequence that already owns its storage changes
           nothing observable. Pins that compact is always value-preserving, whether or not the operand
           shares storage — it never alters the bytes, only (possibly) the storage backing them.")
  (input (= (Bytes.compact (Bytes.of #list(1 2 3))) (Bytes.of #list(1 2 3))))
  (output (: true Bool)))

; `Bytes.compact` returns the SAME handle it's given (it flattens the operand rope in place), so a
; let-bound compact result is an ALIAS of its operand — the dup/drop accounting must treat compact as
; CONSUMING, not borrowing (adv-66). When it was mis-classified as a borrow, a let-bound compact read
; TWICE — once by a value-`=` against the rope (borrow), then by an order-compare whose LEFT operand
; re-walks the rope while `Bytes.concat` consumes the alias — FBIP-freed the shared handle before the
; rope's second deep-walk → wasm OOB (rust computed correctly). This pins the runtime double-read at
; two lengths so the aliasing consume-classification can't regress; the value is unchanged (compact is
; value-preserving, above) — the pin guards the OWNERSHIP, not the bytes.
(case
  "a let-bound Bytes.compact result read twice (eq then order-compare) computes without an OOB fault"
  (doc
    "adv-66: `rope` is built by recursive `Bytes.concat`; `(let ((flat (Bytes.compact rope))) …)`
           reads `flat`/`rope` TWICE — `(= rope flat)` (both borrow) then `(< rope (Bytes.concat flat …))`
           (the concat consumes the alias, the compare re-walks rope). Since compact ALIASES rope, a
           borrow-misclassification freed the shared handle before rope's second deep-walk → OOB on wasm.
           Fixed by classifying `Core::BytesCompact` as consuming. Result 11 (eq true=1 + 10·(rope < flat+B
           true)=10) at BOTH n=2 and n=10; the two lengths exercise the small-rope and larger-rope paths.")
  (input
    (do
      (def
        (build-rope (: n Int64) (: acc Bytes))
        (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def
        (main (: n Int64))
        (let
          ((rope (build-rope n (Bytes.of #list()))))
          (let
            ((flat (Bytes.compact rope)))
            (+
              (if (= rope flat) 1 0)
              (* 10 (if (< rope (Bytes.concat flat (Bytes.of #list((UInt8.wrap 66))))) 1 0))))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 11 Int64))
  (call main (: 2 Int64))
  (output (: 11 Int64)))

; OVER-ROTATION perimeter for the compact-aliasing consume fix (adv-66): these near-neighbor shapes
; ALREADY compute correctly (they don't hit the rope-2nd-deep-walk-after-eq trigger), but they must
; STAY correct. The fix widened the consume set for Core::BytesCompact; a future dup-pass change that
; OVER-consumes a borrow (or mis-widens the set) would double-free one of these and flip it. Pinning
; the passing edge (requested by the seam owner) catches an over-rotation the failing base case can't.
(case
  "compacting then reading the flat's LEN after an equality against the rope is safe (adv-66 perimeter)"
  (doc
    "eq-then-LEN: `(= rope flat)` borrows both, then `(Bytes.len flat)` reads the (aliased) flat
           WITHOUT a second rope deep-walk — no double-consume, so no fault even pre-fix. Result at n=1:
           eq true (1) + 100·len(1) = 101. Guards that the consume-widening doesn't over-free flat's
           borrow-then-len read.")
  (input
    (do
      (def
        (build-rope (: n Int64) (: acc Bytes))
        (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def
        (main (: n Int64))
        (let
          ((rope (build-rope n (Bytes.of #list()))))
          (let ((flat (Bytes.compact rope))) (+ (if (= rope flat) 1 0) (* 100 (Bytes.len flat))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101 Int64)))

(case
  "compacting then a concat+len of the flat after an equality (no rope-left compare) is safe (adv-66 perimeter)"
  (doc
    "eq-then-CONCAT+len, but the second read does NOT re-walk the rope as a compare's left operand
           (it only consumes flat via concat) — so no rope-2nd-deep-walk, no fault even pre-fix. n=1: eq
           true (1) + 10·len(concat flat 'B')=10·2 = 21. The near-miss of the base trigger: same
           consuming concat of the alias, minus the rope-on-left compare that forced the freed re-walk.")
  (input
    (do
      (def
        (build-rope (: n Int64) (: acc Bytes))
        (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def
        (main (: n Int64))
        (let
          ((rope (build-rope n (Bytes.of #list()))))
          (let
            ((flat (Bytes.compact rope)))
            (+
              (if (= rope flat) 1 0)
              (* 10 (Bytes.len (Bytes.concat flat (Bytes.of #list((UInt8.wrap 66))))))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 21 Int64)))

(case
  "compacting then TWO order-compares (no equality) is safe (adv-66 perimeter)"
  (doc
    "Two order-compares, NO eq: each compare deep-walks rope/flat once but there is no borrow-then-
           consume of the SAME binding pair in sequence, so no double-consume — correct even pre-fix.
           n=1: (rope < flat+'B') true (1) + 10·(flat < rope+'B') true (10) = 11. Guards that the
           consume-classification doesn't over-free a binding used across two independent compares.")
  (input
    (do
      (def
        (build-rope (: n Int64) (: acc Bytes))
        (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def
        (main (: n Int64))
        (let
          ((rope (build-rope n (Bytes.of #list()))))
          (let
            ((flat (Bytes.compact rope)))
            (+
              (if (< rope (Bytes.concat flat (Bytes.of #list((UInt8.wrap 66))))) 1 0)
              (* 10 (if (< flat (Bytes.concat rope (Bytes.of #list((UInt8.wrap 66))))) 1 0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64)))

; --- Bytes as a RUNTIME value: construct, measure, and concatenate at run time ------------
; Every case above builds Bytes from literal integers, so the whole value is compile-time-known and
; folds to one baked constant. But the compiler's OWN interface is `compile: list<u8> -> result<list<u8>>`
; — it READS input bytes and BUILDS output bytes whose contents depend on runtime data. These cases pin
; that a byte sequence carrying a genuine runtime byte, or built by a runtime-recursive computation, is
; a first-class value: it lives on the value heap (packed, via the runtime's bytes-alloc/set/get/len),
; renders `(Bytes.of (list …))` identically to the const form, and supports len/concat at run time. This
; is the byte-level substrate a self-hosted compiler assembles a component's wasm bytes with.
(case
  "a byte sequence carrying a runtime byte value is a first-class value"
  (doc
    "`(Bytes.of (list n 66 67))` with `n` a runtime parameter cannot fold to a constant — the
           first byte is decided at run time. The seed builds it on the value heap (bytes-alloc then
           bytes-set per byte, range-checking each to 0..=255) and the type-directed renderer walks it
           back to `b\"ABC\"`, byte-identical to a const byte sequence. Bytes 65 66 67 are the printable
           ASCII `A B C`, so the byte-string display shows them literally. Pins that Bytes is a runtime
           value, not only a compile-time literal — the compiler's output type flowing at run time.")
  (input (do (def (mk n) (Bytes.of #list(n 66 67))) (def (main) (mk 65)) (export main)))
  (output (: b"ABC" Bytes)))

(case
  "a runtime wider integer is truncated into a byte by wrap"
  (doc
    "The runtime companion of the byte-construction cases: `Bytes.of` takes a `(List UInt8)`, so a
           byte built from a WIDER runtime integer is truncated with `(UInt8.wrap n)` — total, keeping the
           low 8 bits (numeric-model.md #wrap Never Traps). `(UInt8.wrap 258)` = 2 (258 mod 256), so
           `(Bytes.of (list (UInt8.wrap 258)))` is the one-byte `b\"\\x02\"`, whose length is 1. Pins that
           the byte bound is carried by the UInt8 TYPE and that crossing into it from a wider value is the
           explicit total `wrap`, not a runtime range trap — there is no out-of-range byte to trap on,
           because a UInt8 is in range by construction. `Bytes.len` reads the result to a scalar (1).")
  (input
    (do
      (def (mk n) (Bytes.len (Bytes.of #list((UInt8.wrap n)))))
      (def (main) (mk 258))
      (export main)))
  (output (: 1 Int64)))

(case
  "the length of a runtime byte sequence is its byte count"
  (doc
    "`Bytes.len` of a byte sequence carrying a runtime byte: the seed folds a runtime Bytes value
           to a SCALAR count via the runtime's bytes-len, the fold-to-scalar half of the idiom (like a
           recursive list sum). `(Bytes.len (Bytes.of (list n 2 3)))` = 3 for any `n`.")
  (input (do (def (sz n) (Bytes.len (Bytes.of #list(n 2 3)))) (def (main) (sz 9)) (export main)))
  (output (: 3 Int64)))

(case
  "concatenating byte sequences built at run time appends their bytes in order"
  (doc
    "`Bytes.concat` of two runtime byte sequences yields a genuine runtime value with the appended
           bytes. The representation MAY defer the concatenation — sharing the operands' storage under a
           concatenation node rather than copying their bytes into a fresh buffer — as an unobservable
           optimization (memory-and-resource-model.md #Sharing Is Not Observable), which keeps this case
           green either way. `(Bytes.concat (Bytes.of (list a)) (Bytes.of (list b 9)))` = `b\"\\x07\\x08\\t\"`
           for `a=7 b=8` — bytes 7 (BEL), 8 (backspace), 9 (tab) render as escapes (9 is the `\\t` special
           escape). Pins runtime concatenation — how a compiler joins the byte fragments of its output.")
  (input
    (do
      (def (join a b) (Bytes.concat (Bytes.of #list(a)) (Bytes.of #list(b 9))))
      (def (main) (join 7 8))
      (export main)))
  (output (: b"\x07\x08\t" Bytes)))

; Bytes.of over a RUNTIME (non-literal) list — a byte sequence built from a `(List UInt8)` the compiler
; cannot see the elements of (a `List.concat`, a param/recursively-built list). `Bytes.of` semantically IS
; a left fold of "append one byte" from the empty sequence, so the compiler synthesizes that fold and runs
; it at runtime (a constant `(list …)` literal still folds to a `Bytes` at compile time). `Bytes.of` is
; monomorphic (`(List UInt8) → Bytes`), so there is no multi-element-type restriction.
(case
  "Bytes.of of a computed (concatenated) runtime list builds a byte sequence of that length"
  (doc
    "`Bytes.of` over a `List.concat` result — a runtime `(List UInt8)` the compiler has no element
           list to fold at compile time. `(List.concat (list a b) (list a))` = [a, b, a]; `Bytes.of` of it
           builds a 3-byte sequence → `Bytes.len` 3. Pins runtime-list byte CONSTRUCTION (the synthesized
           `Bytes.concat` fold). A `Bytes.of` that only accepted a compile-time list literal would DECLINE
           this. MUST be 3.")
  (input
    (do
      (def (main (: a UInt8) (: b UInt8)) (Bytes.len (Bytes.of (List.concat #list(a b) #list(a)))))
      (export main)))
  (call main (: 7 UInt8) (: 9 UInt8))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a byte read back from a runtime-list-built Bytes has the right value"
  (doc
    "Value-correctness of runtime-list `Bytes.of` (not just its length): build `Bytes.of (List.concat
           (list a b) (list a))` = [a, b, a] and read index 1 — it is `b` (9). Encodes 100·len + byte[1] =
           100·3 + 9 = 309. Pins that the synthesized append fold places each byte at its position (a fold
           that dropped/reordered a byte would misread here while the length case still passed). MUST be
           309. (`Bytes.at` is fallible → `(Option Int64)`; the absent arm cannot occur for an in-bounds
           read and yields -1.)")
  (input
    (do
      (def
        (main (: a UInt8) (: b UInt8))
        (+
          (* 100 (Bytes.len (Bytes.of (List.concat #list(a b) #list(a)))))
          (match
            (Bytes.at (Bytes.of (List.concat #list(a b) #list(a))) 1)
            ((Some v) v)
            ((None _u) -1))))
      (export main)))
  (call main (: 7 UInt8) (: 9 UInt8))
  (output (: 309 Int64))
  (live-objects known-leak))

(case
  "a recursively-built byte sequence assembles its bytes at run time"
  (doc
    "The genuine self-hosting idiom for output: a byte sequence whose LENGTH is decided at run
           time, built by recursion + concatenation, not a fixed literal spine. `rep` prepends the byte
           88 `n` times onto the empty sequence, so how many bytes exist is known only at run time.
           `(rep 4)` = `b\"XXXX\"` (byte 88 is the printable ASCII `X`). This is exactly the shape a
           self-hosted compiler uses to emit a component's wasm bytes — concatenating byte fragments in
           a recursion whose depth is driven by the program being compiled.")
  (input
    (do
      (def
        (rep n)
        (if (< n 1) (Bytes.of #list()) (Bytes.concat (Bytes.of #list(88)) (rep (- n 1)))))
      (def (main) (rep 4))
      (export main)))
  (output (: b"XXXX" Bytes))
  (live-objects known-leak))

(case
  "a 2000-deep runtime Bytes.concat rope flattens iteratively and reads its content stack-safe"
  (doc
    "The depth companion of the recursively-built-bytes case above (depth 4); this drives the concat
           ROPE to depth 2000 so the runtime's ITERATIVE `bytes_flatten` (a rope-tree walk) is exercised at a
           depth that would overflow a naive RECURSIVE flatten's native stack. `rep` builds a left-growing rope
           by prepending a 1-byte leaf `[i%256]` at each of n steps (each `Bytes.concat` is a rope node over
           the running rope), so after n steps the value is a 2000-node-deep rope of length 2000 whose byte at
           index i is `(2000-1-i)%256`. Reading forces the flatten: `sum` reads EVERY index with `Bytes.at` and
           totals the bytes. Bytes cycle 0..255 (Sigma over one cycle = 32640); 2000 = 7*256 + 208, so total =
           7*32640 + Sigma 0..207 = 228480 + 21528 = 250008. A flatten that overflowed, or a rope walk that
           mis-ordered/lost a node, changes the sum (a None on any read poisons by -1000000). Runtime n keeps
           it out of the const-fold, exercising the real heap rope + iterative flatten.")
  (input
    (do
      (def
        (rep (: i Int64) (: n Int64) (: acc Bytes))
        (if (< i n) (rep (+ i 1) n (Bytes.concat (Bytes.of #list((UInt8.wrap (% i 256)))) acc)) acc))
      (def
        (sum (: i Int64) (: b Bytes) (: acc Int64))
        (if
          (< i 0)
          acc
          (sum (- i 1) b (+ acc (match (Bytes.at b i) ((Some v) v) ((None u) -1000000))))))
      (def
        (main (: n Int64))
        (let ((r (rep 0 n (Bytes.of #list())))) (sum (- (Bytes.len r) 1) r 0)))
      (export main)))
  (call main (: 2000 Int64))
  (output (: 250008 Int64))
  (live-objects known-leak))

(case
  "an unsigned LEB128 encoder emits the known-answer multibyte encoding"
  (doc
    "The compiler's byte-emitting SPINE as one known-answer case: the recursive unsigned-LEB128
           encoder that produces every section length, vector count, and u32 operand in a wasm module.
           It composes the primitives the numeric cases pin individually — `(< n 128)` (terminator
           test), `(& n 127)` (low 7 bits), `(| … 128)` (continuation bit), `(>> n 7)` (next group),
           `UInt8.wrap` (truncate the composed 7-bit-plus-continuation value to a byte), and `Bytes.concat`
           — into a recursion whose depth is the number of output
           bytes. `(uleb 624485)` is the canonical multibyte value from the LEB128 spec: 624485 =
           0b10011_0001110_1100101, so the little-endian 7-bit groups are 0x65, 0x0E, 0x26, and with
           the continuation bit set on all but the last the bytes are `E5 8E 26` = `b\"\\xe5\\x8e&\"`
           (byte 0x26 is `&`). Pins that the whole encoder composes to the exact bytes wasm requires —
           a single-primitive slip (wrong mask, wrong shift, dropped continuation bit) changes the
           output, so this is a tighter check on the emit path than any primitive alone. The companion
           `(uleb 100)` (100 < 128) exits in one byte to `b\"d\"`, exercising the base case.")
  (input
    (do
      (def
        (uleb n)
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (output (: b"\xe5\x8e&" Bytes))
  (live-objects known-leak))

(case
  "an unsigned LEB128 encoder emits a single byte below the continuation threshold"
  (doc
    "The base case of the LEB128 encoder above: a value under 128 needs no continuation byte, so
           the encoder's `(< n 128)` arm emits exactly one byte and does not recurse. `(uleb 100)` =
           `b\"d\"` (byte 100 is ASCII `d`). Pins the terminator arm in isolation from the recursive
           multibyte path, so a regression in either arm is localized.")
  (input
    (do
      (def
        (uleb n)
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 100))
      (export main)))
  (output (: b"d" Bytes))
  (live-objects known-leak))

(case
  "a recursive emitter dispatches on a sum's variants to build bytes per node"
  (doc
    "The compiler's emit spine as a type-driven tree walk: a recursive `emit : Expr → Bytes`
           `match`es a three-variant AST and returns a DIFFERENT freshly-built byte fragment per variant,
           composing its sub-emissions with `Bytes.concat`. `Expr.Lit` emits one opcode byte (0x42, the
           `i64.const` opcode), `Expr.Neg` emits its operand's bytes then a negate opcode (0x7C), and
           `Expr.Add` emits both operands then an add opcode (0x6A) — post-order, exactly the wasm stack
           discipline a real backend follows. `emit (Add (Lit 1) (Neg (Lit 2)))` yields
           `[42] ++ ([42] ++ [7C]) ++ [6A]` = `b\"BB|j\"`. Distinct from the LEB128 encoder cases above,
           which recurse on an INTEGER's bits: this recurses on a SUM's structure, and each arm builds a
           fresh compound whose shape the compiler must infer directly from the arm bodies (their unified
           `Bytes` shape), not via an `if`-on-discriminant detour. This is the `lower`/`serialize` shape a
           self-hosted compiler is written in — the exhaustive per-variant byte map that turns a typed IR
           node into its instruction bytes.")
  (input
    (do
      (type Expr (Lit Int64) (Neg Expr) (Add (Tuple Expr Expr)))
      (def
        (emit e)
        (match
          e
          ((Expr.Lit n) (Bytes.of #list(0x42)))
          ((Expr.Neg x) (Bytes.concat (emit x) (Bytes.of #list(0x7c))))
          ((Expr.Add #tuple(a b))
            (Bytes.concat (emit a) (Bytes.concat (emit b) (Bytes.of #list(0x6a)))))))
      (def (main) (emit (Expr.Add #tuple((Expr.Lit 1) (Expr.Neg (Expr.Lit 2))))))
      (export main)))
  (output (: b"BB|j" Bytes))
  (live-objects known-leak))

(case
  "a recursive fold of a cons-list to bytes is the whole program result"
  (doc
    "The compiler's SERIALIZE spine: fold a linked list of byte fragments into one byte vector by
           recursive `Bytes.concat`, as the program's DIRECT result. `cat-all` walks a `BL` cons-list
           whose elements are `Bytes`, concatenating each head onto the fold of the tail; `build n`
           constructs the list at run time (so its length — hence the fold depth — is a runtime value),
           making fragment `64+k` for k = n..1. `cat-all (build 3)` folds `[b\"C\" b\"B\" b\"A\"]`
           (bytes 67, 66, 65) to `b\"CBA\"`. Pins that a recursive cons-list→Bytes fold infers its Bytes
           result shape even when the recursive call is the fold's whole value (`(Bytes.concat h (cat-all
           t))` with `cat-all t` as an operand) AND the fold is `main`'s direct result — earlier this
           declined \"cannot infer runtime compound result shape\" unless anchored by a literal concat
           operand. This is how a self-hosted compiler assembles its output: `serialize` folds a code
           stream (a list of encoded instruction/section fragments) into the component's byte vector,
           the list-fold companion of the per-node tree-walk emitter above.")
  (input
    (do
      (type BL BNil (BCons (Tuple Bytes BL)))
      (def
        (build n)
        (if
          (< n 1)
          (BL.BNil ())
          (BL.BCons #tuple((Bytes.of #list((UInt8.wrap (+ 64 n)))) (build (- n 1))))))
      (def
        (cat-all xs)
        (match
          xs
          ((BL.BNil _) (Bytes.of #list()))
          ((BL.BCons #tuple(h t)) (Bytes.concat h (cat-all t)))))
      (def (main) (cat-all (build 3)))
      (export main)))
  (output (: b"CBA" Bytes))
  (live-objects known-leak))

; The recursive fold above renders a SMALL rope (3 fragments) as its whole result, but does not read back a
; DEEP rope's content by position. A many-chunk byte rope (repeated Bytes.concat) is a deep byte-rope;
; Bytes.at / Bytes.len / `=` (and Bytes.compact) must traverse it correctly at every position, not just at
; the 2-chunk depth the slice-across-seam cases use. This pins that: a 20-chunk [10,20] rope (40 bytes)
; indexes right at the start, deep interior, and last byte, equals its compacted flat form, and is None past
; the end — the content-through-depth companion of the small recursive-fold and the 2-chunk rope cases.
(case
  "a deep many-chunk runtime byte rope indexes and measures correctly through its depth"
  (doc
    "A 20-chunk rope built by repeated `Bytes.concat` of `[10,20]` (a deep runtime byte-rope, 40
           bytes). `Bytes.len` = 40; `Bytes.at` reads the right byte at index 0 (10), 1 (20), the deep
           interior 38 (10) and last 39 (20); the rope `=` its `Bytes.compact` flat form (a rope equals its
           compacted twin — compaction materializes the same logical bytes); and `Bytes.at 40` is None (past
           the end → -1). Result `(40, 10, 20, 10, 20, 1, -1)`. Pins that a MANY-chunk byte rope's
           addressing/length/equality/compaction traverse the full depth correctly, not just the 2-chunk
           slice-across-seam ropes and the small recursive-fold — the byte-rope companion of the deep
           string-rope content case (13-strings).")
  (input
    (do
      (def
        (build (: n Int64) (: acc Bytes))
        (if (= n 0) acc (build (- n 1) (Bytes.concat acc (Bytes.of #list(10 20))))))
      (def (at (: b Bytes) (: i Int64)) (match (Bytes.at b i) ((Some v) v) ((None _u) -1)))
      (def
        (main (: n Int64))
        (let
          ((r (build 20 (Bytes.of #list()))))
          #tuple((Bytes.len r)
            (at r 0)
            (at r 1)
            (at r 38)
            (at r 39)
            (if (= r (Bytes.compact r)) 1 0)
            (at r 40))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (tuple 40 10 20 10 20 1 -1) (Tuple Int64 Int64 Int64 Int64 Int64 Int64 Int64)))
  (live-objects known-leak))

; --- Slice and compact at RUNTIME: reading and re-basing byte fragments ---------------------
; Slicing and compacting a byte sequence carrying a runtime value are the input-side companions of the
; concat cases above: a compiler reading its input bytes takes sub-ranges (`Bytes.slice`) and, having
; kept a small piece of a large buffer, re-bases it to release the parent (`Bytes.compact`). These pin
; the fallible slice and the value-preserving compact on GENUINE runtime values (not compile-time
; literals), exercising the shared-storage representation directly: slice is fallible exactly as at
; compile time (Some in bounds, None past the end or below zero), and compact is the identity on value.
(case
  "slicing a byte sequence built at run time yields Some of the sub-range"
  (doc
    "`(Bytes.slice b 1 2)` on a runtime-built `b` yields `(Some b\"\\x14\\x1e\")`: the runtime
           realizes the slice by sharing `b`'s storage (a view node over the parent leaf), which is
           indistinguishable from a fresh copy (memory-and-resource-model.md #Sharing Is Not Observable).
           Bytes 20, 30 are non-printable, so the byte-string display escapes them. Pins the fallible
           slice on a runtime value — how a compiler reads a sub-range of its input bytes without copying.")
  (input
    (do
      (def (sl b s n) (Bytes.slice (Bytes.of #list(b 20 30 40)) s n))
      (def (main) (sl 10 1 2))
      (export main)))
  (output (: (Some b"\x14\x1e") (Option Bytes))))

(case
  "slicing a runtime byte sequence past the end yields None"
  (doc
    "`(Bytes.slice b 2 3)` on a runtime-built 4-byte sequence asks for 3 bytes from index 2 —
           running one byte past the end — so it yields None, never reading beyond the sequence or
           returning a short result. The runtime companion of the const past-the-end case, pinning that
           the bound is checked on the value at run time.")
  (input
    (do
      (def (sl b s n) (Bytes.slice (Bytes.of #list(b 20 30 40)) s n))
      (def (main) (sl 10 2 3))
      (export main)))
  (output (: (None unit) (Option Bytes))))

(case
  "slicing a runtime byte sequence with a negative start yields None"
  (doc
    "`(Bytes.slice b -1 2)` uses a start below 0 at run time — no byte at position -1 — so it
           yields None, NOT a large unsigned offset from casting the negative start. The runtime
           companion of the const negative-start case: the check is on the signed value, so a runtime
           negative start is caught before it can wrap.")
  (input
    (do
      (def (sl b s n) (Bytes.slice (Bytes.of #list(b 20 30 40)) s n))
      (def (main) (sl 10 -1 2))
      (export main)))
  (output (: (None unit) (Option Bytes))))

; The runtime bounds check must be OVERFLOW-SAFE. `Bytes.slice` is the FALLIBLE sub-range read — out of
; range yields None, and it MUST NEVER trap (it is the guarded read). A naive predicate `start + len <=
; byte-count` computed in wrapping i64 overflows for attacker-chosen indices near i64::MAX: the sum wraps
; to a negative value that trivially passes the signed `<=`, wrongly taking the in-range path — then the
; i32-wrap of the huge index either returns a WRONG empty `Some` slice or drives the runtime `bytes-slice`
; out of its u32 range and TRAPS. Both are soundness violations (a wrong value / an uncontrolled trap on a
; trap-free op). The bound must be tested without an overflowing add — e.g. `start <= byte-count && len <=
; byte-count - start` (the difference cannot underflow once `start >= 0 && start <= byte-count`), matching
; the const-fold path's i128 check. These pin the two overflow shapes decline to None. The indices are
; passed via runtime params (the constant fold, which already computes in i128, does not apply).
(case
  "slicing with start+len overflowing i64 is out of range, not a wrong slice"
  (doc
    "`(Bytes.slice b start len)` with `start = 2^62` and `len = 2^62` on a 3-byte sequence: the range
           is astronomically out of bounds → None. A bounds check that computes `start + len` in wrapping
           i64 overflows to i64::MIN (negative), passes a signed `<= byte-count` test, and wrongly takes
           the in-range path — then i32-wraps 2^62 to 0 and returns an empty `Some` (a WRONG value). The
           predicate must be overflow-safe: a sum that would overflow is out of range. Expected None (-1).")
  (input
    (do
      (def
        (main (: s Int64) (: l Int64))
        (match (Bytes.slice (Bytes.of #list(10 20 30)) s l) ((Some b) (Bytes.len b)) ((None _) -1)))
      (export main)))
  (call main (: 4611686018427387904 Int64) (: 4611686018427387904 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "slicing with a start near i64::MAX is out of range, not a trap"
  (doc
    "The trap sibling: `start = i64::MAX`, `len = 1` on a 3-byte sequence — out of bounds → None. A
           wrapping-i64 bounds check computes `start + len = i64::MIN` (overflow), passes the signed `<=`,
           takes the in-range path, and i32-wraps i64::MAX to 0xFFFFFFFF — which the runtime `bytes-slice`
           reads as a 4-billion start and TRAPS. `Bytes.slice` PROMISES it never traps, so this is a
           soundness violation. Pins that an out-of-range start, however large, declines to None (-1).")
  (input
    (do
      (def
        (main (: s Int64) (: l Int64))
        (match (Bytes.slice (Bytes.of #list(10 20 30)) s l) ((Some b) (Bytes.len b)) ((None _) -1)))
      (export main)))
  (call main (: 9223372036854775807 Int64) (: 1 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "the slice overflow guard holds on a chained slice-of-a-slice"
  (doc
    "The overflow-safe bounds check lives in the ONE shared `Core::BytesSlice` emit, so it holds at
           EVERY call site — not only over a fresh `Bytes.of`. The outer `(Bytes.slice b 1 3)` yields a
           3-byte view `[20 30 40]`; slicing THAT view with `start = len = 2^62` must decline to None (a
           wrapping-i64 `start + len` would overflow the inner view's own length check identically and
           return an empty `Some`). Pins that the shared-emit fix covers a view's length feeding the same
           predicate — a slice-of-a-slice is guarded exactly as a slice-of-a-fresh-sequence. Expected None
           (-1); the outer slice is in range so the -2 arm is not taken.")
  (input
    (do
      (def
        (main (: ss Int64) (: sl Int64))
        (match
          (Bytes.slice (Bytes.of #list(10 20 30 40 50)) 1 3)
          ((Some s1) (match (Bytes.slice s1 ss sl) ((Some s2) (Bytes.len s2)) ((None _) -1)))
          ((None _) -2)))
      (export main)))
  (call main (: 4611686018427387904 Int64) (: 4611686018427387904 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "compacting a byte sequence built at run time preserves its bytes"
  (doc
    "`(Bytes.compact b)` on a runtime-built `b` = `b`: compact re-bases the value into storage
           independent of any larger buffer it was sliced from (memory-and-resource-model.md #Retained
           Storage Is What A Value's Representation Holds Live), changing storage use but never the value. Pins
           that compact is value-preserving on a runtime value — how a compiler keeps a small slice of a
           large input while letting the input be reclaimed. `(mk 1)` = `b\"\\x01\\x02\\x03\"`.")
  (input
    (do (def (mk n) (Bytes.compact (Bytes.of #list(n 2 3)))) (def (main) (mk 1)) (export main)))
  (output (: b"\x01\x02\x03" Bytes)))

(case
  "compacting a runtime CONCAT ROPE materializes it, preserving the value and staying usable"
  (doc
    "The rope-MATERIALIZATION face of compact (distinct from compacting a slice-view :299 or an already-
           flat single leaf :567): a genuinely-runtime multi-chunk `Bytes.concat` rope — its chunks chosen by a
           run-time `if` so the concat cannot fold — is `Bytes.compact`ed into one contiguous buffer. Compact is
           value-preserving AND the compacted result is fully usable: over the rope `[10,20]++[30,40]` = `[10,
           20,30,40]`, `c = Bytes.compact rope` satisfies `(= c rope)` (value-preserving — the flattened buffer
           equals the un-compacted rope by bytes-in-order, #Sharing Is Not Observable), `(Bytes.len c)` = 4 (the
           materialized result carries its length), and `(Bytes.at c 2)` = Some 30 (indexing the flat buffer
           works). Encoded as a tuple (1, 4, 30). Pins that flattening a deferred concatenation into contiguous
           storage neither changes the value nor breaks len/at — both backends.")
  (input
    (do
      (def (pickb (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
      (def
        (main (: s Int64))
        (let
          ((rope
              (Bytes.concat
                (pickb s (Bytes.of #list(10 20)) (Bytes.of #list(99 99)))
                (pickb s (Bytes.of #list(30 40)) (Bytes.of #list(99 99))))))
          (let
            ((c (Bytes.compact rope)))
            #tuple((if (= c rope) 1 0)
              (Bytes.len c)
              (match (Bytes.at c 2) ((Some x) x) ((None _u) -1))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (tuple 1 4 30) (Tuple Int64 Int64 Int64)))
  (live-objects known-leak))

(case
  "a HEX ENCODER splits each byte into nibbles and indexes a digit alphabet string"
  (doc
    "The bytes→text rendering composite: each `Bytes.at` byte splits into high/low NIBBLES by
           `/ 16` and `% 16`, each nibble indexes the alphabet STRING `\"0123456789abcdef\"` via
           `String.at` (the alphabet-lookup idiom — a table read, not arithmetic on scalar values),
           and the two hex digits append to a growing rope. Bytes (0, 15, n, 171, 255) cover the
           nibble corners: 0x00 (BOTH nibbles zero — leading-zero digits must render), 0x0f (zero
           HIGH nibble), 0xab (both mid-range), 0xff (both maxed), plus the runtime n as a UInt8
           boundary parameter (Bytes.of takes (List UInt8), so the runtime element must arrive
           ALREADY byte-typed — an Int64 there is CDZ0203). Verified by full-string `=` against the
           expected rope plus byte-length 10 (5 bytes → 10 digits, the 2-digits-per-byte invariant).
           n=16 → `000f10abff`; n=60 → `000f3cabff`; both encode 1001.")
  (input
    (do
      (def (digit (: v Int64)) (Option.expect (String.at "0123456789abcdef" v) "nibble in range"))
      (def
        (hex-go (: bs Bytes) (: i Int64) (: len Int64) (: acc String))
        (if
          (>= i len)
          acc
          (match
            (Bytes.at bs i)
            ((Some b)
              (hex-go
                bs
                (+ i 1)
                len
                (String.concat acc (String.concat (digit (/ b 16)) (digit (% b 16))))))
            ((None _u) acc))))
      (def (hex (: bs Bytes)) (hex-go bs 0 (Bytes.len bs) ""))
      (def
        (main (: n UInt8))
        (do
          (def bs (Bytes.of #list(0 15 n 171 255)))
          (def s (hex bs))
          (+
            (* (String.byte-len s) 100)
            (if
              (=
                s
                (String.concat
                  "000f"
                  (String.concat (if (= (Bytes.at bs 2) (Some 16)) "10" "3c") "abff")))
              1
              0))))
      (export main)))
  (call main (: 16 UInt8))
  (output (: 1001 Int64))
  (call main (: 60 UInt8))
  (output (: 1001 Int64))
  (live-objects known-leak))

(case
  "a HEX DECODER finds each digit's value by alphabet scan and rejects a bad digit"
  (doc
    "The encoder's inverse (the pin above reads the alphabet POSITIONALLY; the decoder must
           SEARCH it): each input scalar's value is found by `find-at` — an index-of scan over the
           same table, a nested search loop inside the outer walk — then accumulated base-16. Two
           faces the encoder direction lacks: BAD-DIGIT rejection (at n=0 the runtime scalar becomes
           `y`, the scan misses, and the whole decode of `abyd` short-circuits to -1 — abandoning,
           not skipping) and LEADING zeros (`\"0010\"` → 16, high-order zeros accumulate silently —
           certificate bit ·5). n=1 → `abcd` = 43981 (439816 with both cert bits); n=0 → the -1
           folds through the encoding to -4.")
  (input
    (do
      (def
        (find-at (: alpha String) (: c String) (: i Int64) (: len Int64))
        (if
          (>= i len)
          -1
          (match
            (String.at alpha i)
            ((Some d) (if (= d c) i (find-at alpha c (+ i 1) len)))
            ((None _u) -1))))
      (def
        (dec-go (: s String) (: i Int64) (: len Int64) (: acc Int64))
        (if
          (>= i len)
          acc
          (match
            (String.at s i)
            ((Some c)
              (do
                (def v (find-at "0123456789abcdef" c 0 16))
                (if (< v 0) -1 (dec-go s (+ i 1) len (+ (* acc 16) v)))))
            ((None _u) acc))))
      (def (hexdec (: s String)) (dec-go s 0 (String.scalar-len s) 0))
      (def
        (main (: n Int64))
        (do
          (def mid (if (> n 0) "c" "y"))
          (+
            (* (hexdec (String.concat "ab" (String.concat mid "d"))) 10)
            (+ (* (if (= (hexdec "0010") 16) 1 0) 5) (if (= (hexdec "ff") 255) 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 439816 Int64))
  (call main (: 0 Int64))
  (output (: -4 Int64))
  (live-objects known-leak))

; --- A runtime `Bytes.at` Option is MATCHED — the reader's core idiom -------------------------------
; The reader walks the input bytes with `(match (Bytes.at input i) ((Some b) …) (None …))` on every
; byte, so this must compile: matching a runtime `Bytes.at` result (an `Option<Int64>` — the byte
; boxed) and returning a scalar from each arm. The `Some` binder is the Int64 BYTE (not an opaque
; handle), so it unifies with a scalar `None` arm. These pin that consuming a runtime `Bytes.at`
; Option by `match` works exactly as consuming any other `Option<Int64>` — the last gate before a
; byte-walking reader (hence true `bytes → bytes` self-hosting).
(case
  "matching a runtime Bytes.at Option binds the byte in the Some arm"
  (doc
    "`(match (Bytes.at b i) ((Some x) x) (None -1))` on a runtime byte sequence `b` at an
           in-bounds index returns the byte: `(at (Bytes.of (list 10 20 30)) 1)` is 20. The `Some`
           binder `x` is the Int64 byte (Bytes.at boxes a byte, and the match unboxes it to the scalar),
           so it unifies with the scalar `None` arm — the reader's per-byte dispatch. Pins that a runtime
           `Bytes.at` Option matches like any `Option<Int64>`.")
  (input
    (do
      (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
      (def (main) (at (Bytes.of #list(10 20 30)) 1))
      (export main)))
  (output (: 20 Int64)))

(case
  "matching a runtime Bytes.at Option takes the None arm past the end"
  (doc
    "The out-of-bounds companion: `(at (Bytes.of (list 10 20 30)) 9)` reads past the end, so the
           match takes the `None` arm and returns -1. Pins that both arms of a runtime `Bytes.at` match
           are reachable and unify — the terminating branch of the byte-walk.")
  (input
    (do
      (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
      (def (main) (at (Bytes.of #list(10 20 30)) 9))
      (export main)))
  (output (: -1 Int64)))

(case
  "a runtime-index Bytes.at at a NEGATIVE index is None, not an unsigned wrap"
  (doc
    "The signedness boundary for `Bytes.at`, the byte twin of the `List.at` negative-index case: at a
           NEGATIVE RUNTIME index `Bytes.at` is `None`, because the bounds check is a SIGNED `0 <= i < len`
           compare — a lowering that compared the index UNSIGNED would turn -1 into a huge offset and read
           out of range. The index is a `main` parameter (not a constant, so nothing folds): `i`=-1 → None
           (→ -1), and the extreme `i`=Int64.min (where a naive negate would overflow) → None too; the
           in-bounds control `i`=1 → 20. Companion of the const-negative `Bytes.at … -1` fold and the
           runtime positive-past-end case — this pins the RUNTIME negative index, the byte analogue of the
           Bytes.slice negative-start signedness pin.")
  (input
    (do
      (def
        (main (: i Int64))
        (match (Bytes.at (Bytes.of #list(10 20 30)) i) ((Some x) x) (None -1)))
      (export main)))
  (call main (: -1 Int64))
  (output (: -1 Int64))
  (call main (: -9223372036854775808 Int64))
  (output (: -1 Int64))
  (call main (: 1 Int64))
  (output (: 20 Int64)))

(case
  "reading a byte from a sequence built with a RUNTIME element widens it to the Option payload"
  (doc
    "`(Bytes.of (list n))` with `n : UInt8` a parameter builds a one-byte sequence from a RUNTIME
           value; `(Bytes.at … 0)` reads that byte back as `Some x`, `x = 5` for n = 5. The read must
           reconcile the STORED byte's width (UInt8 / i32) with the `Some` payload's width (Int64 / i64):
           the stored byte is zero-extended to the payload. Was INVALID WASM ('expected i64, found i32') —
           the `Bytes.at` fold used the raw UInt8 element occurrence as the Some(Int64) payload without
           widening (correct only for a CONSTANT element, whose core folds through the width; a runtime
           element must take the runtime read). A constant-element read (`(Bytes.at (Bytes.of (list 5))
           0)`) folds and was always fine; this pins the runtime-stored byte is widened on read.")
  (input
    (do
      (def (main (: n UInt8)) (match (Bytes.at (Bytes.of #list(n)) 0) ((Some x) x) ((None _) -1)))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 5 Int64)))

(case
  "a recursive byte walk sums a runtime sequence via Bytes.at and match"
  (doc
    "The reader's shape: walk a runtime byte sequence from index 0, matching `(Bytes.at b i)` on
           each step — `Some` binds the byte and recurses with `i+1`, `None` (past the end) terminates
           with the accumulator. `(go (Bytes.of (list 10 20 30)) 0 0)` sums to 60. Pins that a recursive
           function driving over the input bytes by matching `Bytes.at` compiles and runs — the core
           `bytes → AST` loop a self-hosted front end is built on.")
  (input
    (do
      (def (go b i acc) (match (Bytes.at b i) ((Some x) (go b (+ i 1) (+ acc x))) (None acc)))
      (def (main) (go (Bytes.of #list(10 20 30)) 0 0))
      (export main)))
  (output (: 60 Int64))
  (live-objects known-leak))

(case
  "a recursive byte fold calling two helpers emits valid wasm (disjoint scratch slots)"
  (doc
    "A recursive `be` whose body composes a heap-`match` result (the inlined `byte-at`, which
           materializes an i32 Option handle in a scratch slot) with checked ARITHMETIC over another
           helper's result (`(* (byte-at b i) (place …))`, whose overflow guards use i64 scratch slots).
           The two must occupy DISJOINT scratch slots: reusing one wasm local at both an i32 handle and an
           i64 arith temp re-types it to two widths → an invalid module ('expected i64, found i32'). The
           annotated form pins the SCRATCH-SLOT discipline directly (the unannotated form additionally
           needs argument-position inference — a separate increment). `be(b\"\\x01\\x02\", 0, 2)` =
           byte[0]*place(1) + byte[1]*place(0) = 1*256 + 2*1 = 258.")
  (input
    (do
      (def (byte-at (: b Bytes) i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (place k) (if (< k 1) 1 (* 256 (place (- k 1)))))
      (def
        (be (: b Bytes) i n)
        (if (< n 1) 0 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1)))))
      (def (main) (be (Bytes.of #list(1 2)) 0 2))
      (export main)))
  (output (: 258 Int64))
  (live-objects 0))

(case
  "a CBOR head decodes its major type and big-endian argument from the input bytes"
  (doc
    "The compiler's INPUT-side decode spine — the dual of the LEB128 output encoder: reading a
           canonical-binary-AST head from the input bytes. The head's initial byte splits into a major
           type (top 3 bits, `(>> byte 5)`) and additional-info (low 5 bits, `(& byte 31)`); an info of
           24/25/26/27 means a 1/2/4/8-byte BIG-ENDIAN argument follows, assembled most-significant-byte
           first. Against the real bytes of `(quote 300)` encoded as a CBOR uint — `19 01 2C` (info 25 = a
           2-byte argument, then 0x01 0x2C) — `major` is 0 (unsigned int) and `arg` is 0x012C = 300. The
           result `(tuple 0 300)` pins BOTH halves of the head decode at once: the major-type shift and
           the big-endian multi-byte argument assembly (`byte[i]*256 + byte[i+1]`). This composes the byte
           primitives (`Bytes.at`+match, `>>`, `&`, `*`, `+`) into the reader's head-decode step, exactly
           as the LEB128 case composes them into the writer's — a single-primitive slip (wrong shift,
           wrong mask, wrong place value) changes the decoded number, so this is a tighter check on the
           input path than any primitive alone.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (major b i) (>> (byte-at b i) 5))
      (def (info b i) (& (byte-at b i) 31))
      (def (be b i n) (if (< n 1) 0 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1)))))
      (def (place k) (if (< k 1) 1 (* 256 (place (- k 1)))))
      (def (arg b i) (if (< (info b i) 24) (info b i) (be b (+ i 1) 2)))
      (def
        (main)
        #tuple((major (Bytes.of #list(0x19 0x1 0x2c)) 0) (arg (Bytes.of #list(0x19 0x1 0x2c)) 0)))
      (export main)))
  (output (: (tuple 0 300) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "a CBOR atom decodes each scalar major type to its value"
  (doc
    "The reader's LEAF-atom decode, the third leg beside head-index dispatch and length-driven
           iteration: interpreting a CBOR scalar by its major type into the value it denotes. A reader
           decoding a canonical AST's atoms must handle each scalar major: 0 (unsigned int) is its
           argument directly; 1 (negative int) is `-1 - arg` (CBOR's negint convention, so arg 9 encodes
           -10); 7 (simple) carries the booleans (`0xF5` = arg 21 = true, `0xF4` = arg 20 = false). `dec`
           dispatches on major and returns the decoded Int64 (booleans as 1/0). Summing the decodes of
           `29` (negint → -10), `F5` (true → 1), and `0A` (uint 10 → 10) gives -10 + 1 + 10 = 1. Pins
           that a reader interprets each scalar atom form correctly — a negint read as a plain uint (9
           instead of -10), or a boolean's arg mistaken for a small int, would corrupt every literal a
           self-hosted front end reads. Completes the reader's decode surface: head dispatch (which
           operation), length iteration (how many children), and atom decode (each leaf's value).")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-major b i) (>> (byte-at b i) 5))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def
        (dec b i)
        (if
          (= (cbor-major b i) 1)
          (- -1 (cbor-arg b i))
          (if (= (cbor-major b i) 7) (if (= (cbor-arg b i) 21) 1 0) (cbor-arg b i))))
      (def
        (main)
        (+
          (dec (Bytes.of #list(0x29)) 0)
          (+ (dec (Bytes.of #list(0xf5)) 0) (dec (Bytes.of #list(0xa)) 0))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a CBOR simple value that is not a known boolean is classified as not-a-boolean"
  (doc
    "CBOR major type 7 (simple/float) holds MORE than the two booleans: `0xF4`=false (arg 20),
           `0xF5`=true (arg 21), `0xF6`=null (arg 22), and the float heads (`0xF9`/`0xFA`/`0xFB`, arg
           25/26/27). A reader that decodes a major-7 head by checking ONLY `arg == 21` (true) and
           defaulting everything else to false MISCLASSIFIES every other simple value — a float head
           (arg 27) reads as `false`, a silent miscompile. A correct classifier is three-way: arg 20 →
           false, arg 21 → true, anything else → NOT a boolean (to be declined or handled as its real
           kind). `classify` returns 1/0 for the two booleans and -1 for a non-boolean simple value; over
           arg 20 (false→0), 21 (true→1·10), and 27 (a float head→ -1·100) it sums to 0 + 10 + (-100) =
           -90. Pins that the boolean discriminator must check the value IS `0xF4`/`0xF5`, not merely
           `≠ 0xF5`, so a float or null head is not silently read as false — the exact miscompile a
           self-hosted reader's major-7 branch makes when it assumes bool (a CBOR float `3.14` decoding
           to `false`). The reader's fix is to route a non-boolean major-7 value to a decline (KError),
           not default it; this case pins the discrimination the decline depends on.")
  (input
    (do
      (def (classify-simple arg) (if (= arg 21) 1 (if (= arg 20) 0 -1)))
      (def
        (main)
        (+ (classify-simple 20) (+ (* 10 (classify-simple 21)) (* 100 (classify-simple 27)))))
      (export main)))
  (output (: -90 Int64)))

(case
  "resolving a head against a prelude symbol rejects a length-mismatched prefix"
  (doc
    "The reader's NAME-resolution step (ast-encoding.md: a node names its kind by a prelude INDEX;
           the reader byte-compares the indexed prelude symbol against a known operator name — no runtime
           String needed, just Bytes). The comparison must check LENGTH first, then bytes: comparing the
           prelude text symbol `\"++\"` (CBOR text-of-2, `62 2B 2B`) against the operator name `b\"+\"`
           (length 1) must be FALSE — a byte-loop without the length guard would see the first byte match
           (`+` = `+`) and wrongly resolve `\"++\"` to `+`. `name-eq` returns 1 only on an exact
           length-and-bytes match; here the lengths differ (2 vs 1) so it is 0. Pins the length-prefixed
           symbol compare a self-hosted reader uses to turn a head index into an operator identity — a
           prefix must not be mistaken for the whole name, or the reader would mis-resolve every operator
           whose name is a prefix of another. The positive companion (exact match → 1) is the head
           resolution the whole-module compile path already exercises end-to-end.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
      (def (payload-off b i) (+ i (cbor-head-len b i)))
      (def (entry-len b e) (cbor-arg b e))
      (def (entry-byte b e j) (byte-at b (+ (payload-off b e) j)))
      (def (lit-byte lit j) (match (Bytes.at lit j) ((Some x) x) ((None _) 0)))
      (def
        (neq-go b e lit j n)
        (if
          (< j n)
          (if (= (entry-byte b e j) (lit-byte lit j)) (neq-go b e lit (+ j 1) n) false)
          true))
      (def (name-eq b e lit n) (if (= (entry-len b e) n) (neq-go b e lit 0 n) false))
      (def (main) (if (name-eq (Bytes.of #list(0x62 0x2b 0x2b)) 0 b"+" 1) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "a CBOR skip walks past a whole nested item to the next offset"
  (doc
    "The reader's structural NAVIGATION primitive, the companion of the head-decode above: given
           the offset of a CBOR item, `cbor-skip` returns the offset just past that entire item —
           recursively, so a nested array is walked element by element. It dispatches on the major type:
           an array (major 4) skips its head then each of its `arg` elements in turn (the mutually
           recursive `skip-elems`); a byte/text string (major 2/3) skips its head then `arg` payload
           bytes; a scalar (major 0/1/7) is just its head. Against the bytes `82 82 01 02 03` — an array
           of two items whose first is itself the array `[1 2]` (`82 01 02`) and whose second is the
           scalar `3` (`03`) — `cbor-skip` from offset 0 walks the outer array: into the inner two-element
           array, past both scalars, then past the trailing `3`, landing at offset 5 (past all five
           bytes). Pins the recursive item-walk a reader uses to reach the root past `[version, prelude,
           root]` and to advance across an application's argument forms — mutual recursion (`cbor-skip` ↔
           `skip-elems`) over runtime input bytes, the navigation half of `bytes → AST` that the
           head-decode's value-extraction half complements.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-major b i) (>> (byte-at b i) 5))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
      (def (skip-elems b i k) (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
      (def
        (cbor-skip b i)
        (if
          (= (cbor-major b i) 4)
          (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
          (if
            (or (= (cbor-major b i) 3) (= (cbor-major b i) 2))
            (+ (+ i (cbor-head-len b i)) (cbor-arg b i))
            (+ i (cbor-head-len b i)))))
      (def (main) (cbor-skip (Bytes.of #list(0x82 0x82 0x1 0x2 0x3)) 0))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a recursive reader decodes a CBOR application tree and evaluates it by head index"
  (doc
    "The reader's spine assembled end-to-end: `ev` recursively decodes a canonical-AST application
           — a CBOR array `[head-index, operand, operand]` — and dispatches on the decoded head index to
           the operation, recursing into each operand offset located by `child-off` (= `skip-elems` from
           the first element). This composes every input-side primitive the earlier cases pin in
           isolation — head decode (`cbor-major`/`cbor-arg`), navigation (`cbor-skip`/`skip-elems`), and
           child-offset location — into the actual `bytes → value` walk a self-hosted reader performs.
           Against `83 00 01 83 01 02 0B` — the CBOR of the application `[+ 1 [* 2 11]]` (outer array of
           3: head-index 0 = `+`, operand `1`, and a nested array-of-3 head-index 1 = `*` with operands
           `2` and `11`=0x0B) — `ev` reads head 0, recurses into operand 1 (the scalar `1`) and operand 2
           (the nested `[* 2 11]`, which reads head 1 and multiplies 2·11), yielding 1 + (2·11) = 23.
           Pins that the primitives COMPOSE into a recursive tree decode over runtime input bytes — the
           `bytes → AST` reader that, joined to `resolve`, is the front end of a self-hosted compiler.
           A single-primitive slip (wrong child offset, wrong head extraction, a navigation miscount)
           reads the wrong operand and changes the result, so this is a tighter check on the reader than
           any primitive alone — the input dual of the LEB128 known-answer emit case.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-major b i) (>> (byte-at b i) 5))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
      (def (skip-elems b i k) (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
      (def
        (cbor-skip b i)
        (if
          (= (cbor-major b i) 4)
          (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
          (+ i (cbor-head-len b i))))
      (def (elem0 b i) (+ i (cbor-head-len b i)))
      (def (child-off b i k) (skip-elems b (elem0 b i) k))
      (def
        (ev b i)
        (if
          (= (cbor-major b i) 4)
          (if
            (= (cbor-arg b (elem0 b i)) 0)
            (+ (ev b (child-off b i 1)) (ev b (child-off b i 2)))
            (* (ev b (child-off b i 1)) (ev b (child-off b i 2))))
          (cbor-arg b i)))
      (def (main) (ev (Bytes.of #list(0x83 0x0 0x1 0x83 0x1 0x2 0xb)) 0))
      (export main)))
  (output (: 23 Int64))
  (live-objects 0))

(case
  "a CBOR reader walks a variable-length array using its decoded length as the element count"
  (doc
    "The reader's structural-COUNT primitive, distinct from head-index dispatch: a CBOR array's
           additional-info IS its element count, so `cbor-arg` on the array head yields the length, and
           that length drives a walk over the array's elements (`elem k` = `skip-elems` from the first
           element). This is how the whole-module reader finds how many `def`s a module has (the root
           array's length minus the head and name) and how many parameters each `def` takes (its
           signature array's length minus one) — the array length read AS DATA, not a fixed arity.
           Against `84 0A 14 18 1E 18 28` — a CBOR array of 4 whose elements are 10, 20, 30, 40 (the last
           two one-byte-argument encoded, `18 1E` and `18 28`) — `sum-array` reads the length 4 from the
           head, then sums the 4 elements located by `elem`, yielding 10+20+30+40 = 100. Pins that a
           reader drives a loop by an array length decoded from the input (the shape of reading a
           module's def list or a call's argument list), the count half of `bytes → AST` that the
           head-index-dispatch case complements.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-major b i) (>> (byte-at b i) 5))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
      (def (skip-elems b i k) (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
      (def
        (cbor-skip b i)
        (if
          (= (cbor-major b i) 4)
          (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
          (+ i (cbor-head-len b i))))
      (def (elem0 b i) (+ i (cbor-head-len b i)))
      (def (elem b i k) (skip-elems b (elem0 b i) k))
      (def
        (sum-elems b i k n)
        (if (< k n) (+ (cbor-arg b (elem b i k)) (sum-elems b i (+ k 1) n)) 0))
      (def (sum-array b i) (sum-elems b i 0 (cbor-arg b i)))
      (def (main) (sum-array (Bytes.of #list(0x84 0xa 0x14 0x18 0x1e 0x18 0x28)) 0))
      (export main)))
  (output (: 100 Int64))
  (live-objects 0))

(case
  "a CBOR skip steps over a tagged item to the value it wraps"
  (doc
    "The reader's navigation over a CBOR TAG (major 6): a tag is its head followed by exactly one
           tagged data item, so skipping a tag skips its head then recursively skips the one item it
           wraps. This is the `39` bare-name marker a canonical-AST module uses to distinguish a symbol
           reference from a plain integer — encoded `d8 27 <idx>` (tag number 39 = `0x27`, given as a
           one-byte argument after the `0xd8` tag head). Against `d8 27 01` — tag 39 (a two-byte head)
           wrapping the uint `1` (one byte) — `cbor-skip` from offset 0 steps past the tag head and the
           wrapped uint, landing at offset 3. Pins the tag branch of the navigation primitive: without
           it a reader walking a module's def list would miscount offsets the moment it met a tagged
           name, reading the wrong element. Completes the item-kind coverage of `cbor-skip` (array /
           string / tag / scalar) the reader needs to traverse the whole canonical AST.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (cbor-major b i) (>> (byte-at b i) 5))
      (def (cbor-info b i) (& (byte-at b i) 31))
      (def (cbor-arg b i) (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
      (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
      (def (skip-elems b i k) (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
      (def
        (cbor-skip b i)
        (if
          (= (cbor-major b i) 4)
          (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
          (if
            (or (= (cbor-major b i) 3) (= (cbor-major b i) 2))
            (+ (+ i (cbor-head-len b i)) (cbor-arg b i))
            (if
              (= (cbor-major b i) 6)
              (cbor-skip b (+ i (cbor-head-len b i)))
              (+ i (cbor-head-len b i))))))
      (def (main) (cbor-skip (Bytes.of #list(0xd8 0x27 0x1)) 0))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

; --- The `b"…"` literal reads to a byte sequence, and rendering round-trips -----------------------
; `b"…"` is reader sugar for `(Bytes.of (list …))`, so a byte-string literal and the explicit form
; denote ONE value: they are equal. These cases pin the reader equivalence in both directions
; (printable and escaped bytes) and the full round-trip — a byte sequence WRITTEN as `b"…"`,
; constructed, and rendered back yields the same `b"…"` text — so the display form and the input form
; are inverses. A generation that does not yet realize the `b"…"` reader sugar declines (todo), it
; does not miscompile.
(case
  "a byte-string literal equals the explicit byte sequence it desugars to"
  (doc
    "`(= b\"ABC\" (Bytes.of (list 65 66 67)))` is true: `b\"ABC\"` reads to `(Bytes.of (list 65 66
           67))` (bytes 65 66 67 = ASCII `A B C`), so the literal and the explicit form are the same
           value (options/binary-syntax; the `#\"…\"`/`a.b` sugar pattern). Pins that the byte-string
           literal is reader sugar, not a distinct value form.")
  (input (= b"ABC" (Bytes.of #list(65 66 67))))
  (output (: true Bool)))

(case
  "a byte-string literal with escapes equals its explicit byte sequence"
  (doc
    "`(= b\"\\x89PNG\" (Bytes.of (list 137 80 78 71)))` is true: the `\\x89` hex escape is byte
           137 and `PNG` are the printable bytes 80 78 71, so the literal reads to the PNG magic
           prefix. Pins that `\\xNN` and printable-ASCII bytes read to the same values the explicit
           list names — the reader escape set is the inverse of the display escape set.")
  (input (= b"\x89PNG" (Bytes.of #list(137 80 78 71))))
  (output (: true Bool)))

(case
  "an empty byte-string literal is the empty byte sequence"
  (doc
    "`(= b\"\" (Bytes.of (list)))` is true: `b\"\"` reads to the zero-length byte sequence. Pins
           the degenerate literal, the byte-string spelling of `(Bytes.of (list))`.")
  (input (= b"" (Bytes.of #list())))
  (output (: true Bool)))

(case
  "a byte sequence written as a literal renders back to the same literal"
  (doc
    "The full round-trip: a byte sequence built at run time from a `b\"…\"` literal renders back
           to that same `b\"…\"` text. `b\"A\\nB\"` carries the printable `A`, a newline (the `\\n`
           special escape), and `B`; passing it through a runtime function and rendering the result
           yields `b\"A\\nB\"` — reading and displaying a byte sequence are inverses.")
  (input (do (def (id b) b) (def (main) (id b"A\nB")) (export main)))
  (output (: b"A\nB" Bytes)))

; --- A tuple-projected boxed-sum accumulator threaded through a byte-decode tail loop --------------
; A single-scan byte decoder: `one` reads a byte and returns a `(tuple <boxed-sum> <next-pos>)`; the
; driver `loop` threads BOTH projections of that tuple — `(. r 0)` the boxed-sum accumulator and
; `(. r 1)` the advanced position — into its own tail-recursive params. `(. r 0)` extracts a NESTED
; COMPOUND (the boxed sum `W.Atom`) child handle OUT of the tuple; that child escapes into the
; recursive `loop` call as the `last` param. If the enclosing `let`-bound tuple `r` were reclaimed
; after the projections (its scalar `(. r 1)` copies out, so the naive rule sees only borrows), the
; drop would cascade to FREE the escaped boxed-sum child — a use-after-free that read back garbage (the
; accumulator came out 0 instead of 5). Pins that a NESTED-COMPOUND projection ESCAPES its aggregate
; (the aggregate is not reclaimed while its extracted child is still live), so the threaded boxed-sum
; survives the loop step. The `if` inside `one` forces `r` to be a real join-produced heap handle, not
; a folded constant — remove it, or thread a plain-Int64 accumulator, or advance pos any other way, and
; the bug vanishes: all three conditions are jointly required.
(case
  "a tail loop threading a projected boxed-sum accumulator and a projected cursor decodes correctly"
  (doc
    "`one b pos` returns `(tuple (W.Atom <byte>) (+ pos 1))`; `loop` threads `(. r 0)` (the boxed
           sum) and `(. r 1)` (the next position) into its params, so the projected `W.Atom` child
           handle escapes OUT of the tuple into the recursive call. `main 0` scans byte 0 of
           `b\"\\x05\\x07\"` (= 5) once, wraps it in `W.Atom`, threads it through one loop step, and
           unwraps to 5. A projection that let the tuple be reclaimed would free the escaped boxed sum
           and read garbage (it returned 0). Pins nested-compound-projection escape through a tail loop.")
  (input
    (do
      (type W (Atom Int64) (Zero))
      (def
        (one (: b Bytes) (: pos Int64))
        (if
          (= (Option.expect (Bytes.at b pos) "t") 5)
          #tuple((W.Atom (Option.expect (Bytes.at b pos) "v")) (+ pos 1))
          #tuple((W.Atom 99) (+ pos 1))))
      (def
        (loop (: b Bytes) (: n Int64) (: pos Int64) (: last W))
        (if (= n 0) last (let ((r (one b pos))) (loop b (- n 1) (. r 1) (. r 0)))))
      (def (wval (: s W)) (match s ((W.Atom li) li) ((W.Zero _) 0)))
      (def (main (: pos Int64)) (wval (loop b"\x05\x07" 1 pos (W.Atom 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (live-objects 0))

; --- A Bytes ROPE nested in a compound MAP KEY is canonicalized (the Bytes face of nested-rope compaction) --
; A `Bytes.concat` builds a ROPE (a concat node whose raw is a header, not the content) — the same
; rope representation a `String.concat` builds (a String IS a Bytes leaf). The construction-site
; String/Bytes-leaf compaction canonicalizes a Bytes rope nested in a compound too (the
; `elem_needs_rope_compaction` gate covers `Ty::Bytes`), so a compound MAP KEY whose Bytes element is a
; rope hashes into the same CHAMP slot as its flat-twin key. Pins the Bytes face of the nested-rope
; canonicalization (the String faces live in 13-strings). `rep b n` appends the byte 120 (`x`) `n` times.
(case
  "a compound map key whose Bytes element is a rope is found by its flat twin"
  (doc
    "`(Map.insert Map.empty (tuple (rep (Bytes.of (list 104 105)) 3) 1) 42)` keys the map by a TUPLE
           whose Bytes element is a runtime ROPE (three `Bytes.concat`s → bytes [104,105,120,120,120]);
           `(Map.lookup … (tuple (Bytes.of (list 104 105 120 120 120)) 1))` looks up with the flat-twin
           tuple. Equal keys → 42. Before the construction-site compaction the tuple key would hash with its
           nested Bytes rope leaf uncompacted, landing in a different slot → None (-1). The Bytes twin of the
           nested-String map-key case (13-strings). Expected: 42.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def
        (main)
        (match
          (Map.lookup
            (Map.insert Map.empty #tuple((rep (Bytes.of #list(104 105)) 3) 1) 42)
            #tuple((Bytes.of #list(104 105 120 120 120)) 1))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (output (: 42 Int64)))

; --- NESTED / keyed runtime BYTES value-equality (a DIRECT Bytes leaf in a compound / a CHAMP key) -----
; The DIRECT-operand Bytes `=` (`(= bytesA bytesB)`) already compacts each operand before `value-eq`. These
; pin the NESTED and KEYED faces: a Bytes leaf inside a tuple/sum (compared by the `value-eq` heap-walk), and
; a Bytes as a Map/Set KEY (compared/hashed by CHAMP `champ_eq`/`champ_hash`). `ty_heap_walkable` now admits
; a `Bytes` leaf, and `key_needs_compaction` compacts a Bytes key — so a `Bytes.concat` ROPE canonicalizes to
; its flat byte form at BOTH the element-construction site (`elem_needs_rope_compaction`) and the key site,
; and the physical byte-walk is EXACT. Before this a compound/keyed runtime Bytes `=` declined "comparison of
; a compound value needs a heap walk". `rep b n` appends the byte 120 (`x`) `n` times to force a runtime rope.
(case
  "a runtime Bytes rope nested in a tuple compares equal to its flat twin"
  (doc
    "`(= (tuple a 1) (tuple b 1))` where `a` is a runtime `Bytes.concat` rope [104,120] and `b` the
           flat `Bytes.of [104,120]` — the nested Bytes leaf is compacted at the tuple construction, so the
           value-eq heap-walk sees identical bytes → true. Pins the NESTED-Bytes face of compound equality.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def (eq (: a Bytes) (: c Bytes)) (= #tuple(a 1) #tuple(c 1)))
      (def (main) (eq (rep (Bytes.of #list(104)) 1) (Bytes.of #list(104 120))))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "different runtime Bytes nested in a tuple compare unequal"
  (doc
    "The negative companion: distinct Bytes leaves in the tuple → false. Confirms the nested-Bytes
           compound walk is genuinely structural, not always-true.")
  (input
    (do
      (def (eq (: a Bytes) (: c Bytes)) (= #tuple(a 1) #tuple(c 1)))
      (def (main) (eq (Bytes.of #list(104)) (Bytes.of #list(105))))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "a runtime Bytes rope is found as a Set element by its flat twin"
  (doc
    "A `Set Bytes` membership test with a rope query key: `(Set.contains (Set.of (list flat)) rope)` —
           the rope query [104,120] is compacted at the `set-contains` KEY site, so it hashes into the same
           CHAMP slot as the flat element it equals → true. Pins the Bytes CHAMP-KEY face (a rope key must
           hash identically to its flat twin, else it would never be found). `key_needs_compaction` now
           compacts a Bytes key.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def (main) (Set.contains #set((Bytes.of #list(104 120))) (rep (Bytes.of #list(104)) 1)))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "a runtime Bytes rope map key is looked up by its flat twin"
  (doc
    "A `Map Bytes _` lookup with a rope query key: insert under the flat key `[104,120]`→42, look up
           with the rope `(rep [104] 1)`. The rope query is compacted at the `map-lookup` KEY site, hashing
           to the flat key's slot → 42. Pins the Bytes map-KEY face (the direct-Bytes-key analogue of the
           nested-tuple-key case above).")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def
        (main)
        (Option.expect
          (Map.lookup
            (Map.insert Map.empty (Bytes.of #list(104 120)) 42)
            (rep (Bytes.of #list(104)) 1))
          "found"))
      (export main)))
  (call main)
  (output (: 42 Int64)))

; The BORROWED-key face of the rope map-key compaction. The case above passes a FRESH-OWNED rope key
; (`(rep …)` directly as the lookup arg) which the emit's old `Owned`-only gate already compacted. But a
; BORROWED rope key — a kept `let`-local (or a `sum-payload`/`Option.expect` binder) read again after the
; lookup, so `map-lookup` only BORROWS it — was NOT compacted under that old gate: its raw slice-VIEW node
; (`[off,len]`) reached `champ_hash` and hashed differently from the equal-content flat key → a wrong-value
; MISS. The fix compacts a String/Bytes key of ANY ownership at the CHAMP key site (`bytes-compact` is
; refcount-neutral — safe on a borrow). Here `k` is bound and read TWICE (the lookup key AND `Bytes.len k`),
; so it is genuinely borrowed at the lookup: 100*42 + 2 = 4202 proves the lookup found the flat twin (42) AND
; `k` survived for the length read (2). Regression guard for the borrowed-rope-CHAMP-key wrong-value fix.
(case
  "a BORROWED runtime Bytes rope map key is compacted at the key site and found by its flat twin"
  (doc
    "The borrowed-key twin of the rope map-key case above: a rope `k` bound in a `let` and read TWICE —
           once as the `Map.lookup` KEY (borrowed, not consumed) and once by `Bytes.len k` — must be compacted
           at the CHAMP key site so it hashes to its flat twin's slot. The old Owned-only compaction gate
           missed it (a borrowed slice-view key hashed differently → wrong-value miss). `k = (rep [104] 1)` =
           the rope for `[104,120]`; lookup finds 42, and `k` survives for `Bytes.len` = 2 → 100*42 + 2 = 4202.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def
        (main)
        (let
          ((k (rep (Bytes.of #list(104)) 1)))
          (+
            (*
              100
              (Option.expect
                (Map.lookup (Map.insert Map.empty (Bytes.of #list(104 120)) 42) k)
                "found"))
            (Bytes.len k))))
      (export main)))
  (call main)
  (output (: 4202 Int64))
  (live-objects known-leak))

(case
  "a runtime Bytes rope in a SUM payload compares equal to its flat twin"
  (doc
    "The variant-payload face: `(B.Wrap rope)` vs `(B.Wrap flat)` — the Bytes payload is compacted at
           the sum construction, so the value-eq walk compares equal → true. Pins that `ty_heap_walkable`
           admits a Bytes leaf through a sum variant's payload, not only a tuple position.")
  (input
    (do
      (type B (Wrap Bytes))
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def (eq (: a Bytes) (: c Bytes)) (= (B.Wrap a) (B.Wrap c)))
      (def (main) (eq (rep (Bytes.of #list(104)) 1) (Bytes.of #list(104 120))))
      (export main)))
  (call main)
  (output (: true Bool)))

; --- Nested-Bytes equality: the remaining composition faces -----------------------------------------
; ea95d250f admits nested/keyed runtime Bytes into value-eq and CHAMP (its pins: tuple-nested,
; unequal control, set element, direct map key, sum payload). These pin the remaining faces,
; promoted from passing breaker probes — the Bytes twins of the nested-String-rope family.
(case
  "a runtime Bytes rope in a record field compares equal to its flat twin"
  (doc
    "The record-field face of the nested-Bytes walk: `(record (f <rope>) (g 1))` = `(record (f
           <flat>) (g 1))` → true. The field-keyed compound complements the landed tuple/sum faces
           (a walk admitting only positional children misses the field map).")
  (input
    (do
      (def
        (main (: a Int64))
        (if
          (=
            #record((=
                f
                (Bytes.concat (Bytes.of #list((UInt8.wrap a))) (Bytes.of #list((UInt8.wrap 2)))))
              (= g 1))
            #record((= f (Bytes.of #list((UInt8.wrap 1) (UInt8.wrap 2)))) (= g 1)))
          1
          0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a compound map key containing a Bytes rope is found by its flat-twin key"
  (doc
    "The COMPOUND-key face: the map is keyed by a TUPLE whose Bytes element is a rope; the
           lookup key carries the flat twin → 42. Requires champ_hash/champ_eq to canonicalize the
           Bytes leaf INSIDE the compound key (the direct-key face is pinned by the landing; a
           per-leaf compaction that only fires on a top-level Bytes key misses the nested one —
           the exact gap the String family had).")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Map.lookup
            (Map.insert
              Map.empty
              #tuple((Bytes.concat
                  (Bytes.of #list((UInt8.wrap a)))
                  (Bytes.of #list((UInt8.wrap 2))))
                1)
              42)
            #tuple((Bytes.of #list((UInt8.wrap 1) (UInt8.wrap 2))) 1))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64)))

(case
  "a float leaf and a Bytes leaf compare together in one compound"
  (doc
    "One tuple carrying BOTH newly-walkable leaf kinds — a Float (canonical byte form) and a
           Bytes rope (compacted) → true against the flat twin. The two ty_heap_walkable admissions
           landed together; this pins them composing in a single walk (an early-exit on the first
           admitted kind would skip the second leaf's canonicalization).")
  (input
    (do
      (def
        (main (: a Int64))
        (if
          (=
            #tuple(1.5
              (Bytes.concat (Bytes.of #list((UInt8.wrap a))) (Bytes.of #list((UInt8.wrap 2)))))
            #tuple(1.5 (Bytes.of #list((UInt8.wrap 1) (UInt8.wrap 2)))))
          1
          0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

; --- Bytes rope SCALE (the composition pins above stay small; these push seam counts to hundreds) ---
(case
  "a 500-iteration Bytes.concat loop measures exactly and indexes both extremes"
  (doc
    "1001 bytes across ~500 seams: a 1-byte head then 500 (7,8) pairs. `Bytes.len` = 1001 (no seam
           drop/double-count), byte 0 = 1 (the head leaf), byte 1000 = 8 (the deepest right-spine leaf's
           last byte). 1001·10000 + 1·100 + 8 = 10010108. The Bytes twin of the 1000-concat String rope
           pin — the two rope representations lower separately.")
  (input
    (do
      (def
        (build (: n Int64) (: acc Bytes))
        (if (< n 1) acc (build (- n 1) (Bytes.concat acc (Bytes.of #list(7 8))))))
      (def
        (main (: n Int64))
        (let
          ((b (Bytes.concat (Bytes.of #list(1)) (build n (Bytes.of #list())))))
          (+
            (* 10000 (Bytes.len b))
            (+
              (* 100 (match (Bytes.at b 0) ((Some v) v) ((None u) -1)))
              (match (Bytes.at b (* n 2)) ((Some v) v) ((None u) -1))))))
      (export main)))
  (call main (: 500 Int64))
  (output (: 10010108 Int64)))

(case
  "a FLETCHER-16 checksum walks a concat-built byte rope across its seams"
  (doc
    "The checksum-class algorithm face: Fletcher-16's DOUBLE accumulator is position-dependent —
           s2 folds s1 at every byte, so a byte read wrong AT ANY SEAM shifts s2 differently than s1
           (a plain sum is order-insensitive and can't see seam misreads that preserve the multiset).
           Each byte comes through Bytes.at's Option unwrap; r=4 crosses 4 concat seams; r=0 pins the
           empty-rope boundary (checksum of nothing = 0). Modulo 255 per the Fletcher definition.
           r=1 [10,20,30] → s1=60,s2=100·… → 25660; r=4 (12 bytes) → 52720; r=0 → 0.")
  (input
    (do
      (def
        (build (: r Int64) (: acc Bytes))
        (if (= r 0) acc (build (- r 1) (Bytes.concat acc (Bytes.of #list(10 20 30))))))
      (def
        (go (: b Bytes) (: i Int64) (: n Int64) (: s1 Int64) (: s2 Int64))
        (if
          (>= i n)
          (+ (* s2 256) s1)
          (match
            (Bytes.at b i)
            ((Some v) (do (def t1 (% (+ s1 v) 255)) (go b (+ i 1) n t1 (% (+ s2 t1) 255))))
            ((None _u) -1))))
      (def (fletcher (: b Bytes)) (go b 0 (Bytes.len b) 0 0))
      (def (main (: r Int64)) (fletcher (build r (Bytes.of #list()))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 25660 Int64))
  (call main (: 4 Int64))
  (output (: 52720 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a Fletcher-16 over a seam-spanning slice VIEW equals the checksum of its logical bytes"
  (doc
    "Composes the checksum with #Sharing Is Not Observable: slice(2,2) of [10,20,30]⧺[40,50,60]
           SPANS the concat seam — a view realized by storage-sharing must hand the walk the LOGICAL
           byte sequence [30,40]; the position-sensitive s2 catches a view that leaks physical seam
           structure or misaligns the base offset. + the identity slice(0,6) and the EMPTY slice(3,0)
           → 0. slice(2,2)=[30,40] → 25670; whole → 13010; empty → 0.")
  (input
    (do
      (def
        (go (: b Bytes) (: i Int64) (: n Int64) (: s1 Int64) (: s2 Int64))
        (if
          (>= i n)
          (+ (* s2 256) s1)
          (match
            (Bytes.at b i)
            ((Some v) (do (def t1 (% (+ s1 v) 255)) (go b (+ i 1) n t1 (% (+ s2 t1) 255))))
            ((None _u) -1))))
      (def (fletcher (: b Bytes)) (go b 0 (Bytes.len b) 0 0))
      (def
        (main (: st Int64) (: ln Int64))
        (do
          (def rope (Bytes.concat (Bytes.of #list(10 20 30)) (Bytes.of #list(40 50 60))))
          (match (Bytes.slice rope st ln) ((Some s) (fletcher s)) ((None _u) -1))))
      (export main)))
  (call main (: 2 Int64) (: 2 Int64))
  (output (: 25670 Int64))
  (call main (: 0 Int64) (: 6 Int64))
  (output (: 13010 Int64))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a slice OF a seam-spanning slice re-offsets into the logical bytes of the parent view"
  (doc
    "The NESTED-view face: outer slice(1,4) of the seamed rope spans the seam; an inner slice
           then re-offsets INTO the outer view — offset composition against the PARENT's logical
           space is exactly where a sharing implementation drops or double-counts the base offset.
           inner slice(1,2) = logical [30,40], bytes on OPPOSITE sides of the physical seam →
           25670 (equals the direct seam-spanning checksum above — the nesting must be invisible);
           identity inner slice(0,4) = [20..50] → 11660; single-byte tail inner(3,1) = [50] → 12850.")
  (input
    (do
      (def
        (go (: b Bytes) (: i Int64) (: n Int64) (: s1 Int64) (: s2 Int64))
        (if
          (>= i n)
          (+ (* s2 256) s1)
          (match
            (Bytes.at b i)
            ((Some v) (do (def t1 (% (+ s1 v) 255)) (go b (+ i 1) n t1 (% (+ s2 t1) 255))))
            ((None _u) -1))))
      (def (fletcher (: b Bytes)) (go b 0 (Bytes.len b) 0 0))
      (def
        (main (: st Int64) (: ln Int64))
        (do
          (def rope (Bytes.concat (Bytes.of #list(10 20 30)) (Bytes.of #list(40 50 60))))
          (match
            (Bytes.slice rope 1 4)
            ((Some outer)
              (match (Bytes.slice outer st ln) ((Some inner) (fletcher inner)) ((None _u) -1)))
            ((None _u) -2))))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 25670 Int64))
  (call main (: 0 Int64) (: 4 Int64))
  (output (: 11660 Int64))
  (call main (: 3 Int64) (: 1 Int64))
  (output (: 12850 Int64))
  (live-objects known-leak))

(case
  "String.from-bytes rejects a slice that splits a multibyte scalar and accepts aligned cuts"
  (doc
    "The None face of the decode path (the landed from-bytes pins are happy-path): the bytes of
           \"aé\" are [97, 195, 169] — a slice(0,2) cuts the 2-byte é MID-SEQUENCE, so from-bytes
           must reject (None → -1: UTF-8 validity is checked, not assumed); the aligned cuts decode —
           slice(0,3) whole (\"aé\", byte-len 3 → 103) and slice(1,2) the é alone (a multibyte scalar
           at offset 0 of the view → 102). The validity boundary composed with slice views.")
  (input
    (do
      (def
        (main (: st Int64) (: ln Int64))
        (do
          (def b (Bytes.of #list(97 195 169)))
          (match
            (Bytes.slice b st ln)
            ((Some s)
              (match
                (String.from-bytes s)
                ((Some str) (+ 100 (String.byte-len str)))
                ((None _u) -1)))
            ((None _u) -2))))
      (export main)))
  (call main (: 0 Int64) (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64) (: 3 Int64))
  (output (: 103 Int64))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 102 Int64))
  (live-objects known-leak))

(case
  "a slice window spanning MANY seams of a built rope reads the logical bytes"
  (doc
    "The seam-crossing slice at scale (the const seam pin crosses ONE): a 102-byte window starting
           at byte 99 of a 400-byte 200-seam rope spans ~51 leaf boundaries. Its length is 102 and its
           byte 0 is the parent's byte 99 (odd index → 8): 102+8 = 110. A slice walk that miscounted a
           seam lands the window one byte off; a length computed per-leaf would drift.")
  (input
    (do
      (def
        (build (: n Int64) (: acc Bytes))
        (if (< n 1) acc (build (- n 1) (Bytes.concat acc (Bytes.of #list(7 8))))))
      (def
        (main (: n Int64))
        (match
          (Bytes.slice (build n (Bytes.of #list())) 99 102)
          ((Some w) (+ (Bytes.len w) (match (Bytes.at w 0) ((Some v) v) ((None u) -1))))
          ((None u) -2)))
      (export main)))
  (call main (: 200 Int64))
  (output (: 110 Int64))
  (live-objects 0))

(case
  "String.from-bytes decodes a 400-byte 200-seam rope end to end"
  (doc
    "The total UTF-8 decode over a DEEP rope: 200 (97,98) pairs = \"abab…\" — the decoder must
           walk every leaf in order (a well-formedness check that stopped at the first leaf, or a decode
           that flattened only a prefix, mismeasures). byte-len 400.")
  (input
    (do
      (def
        (build (: n Int64) (: acc Bytes))
        (if (< n 1) acc (build (- n 1) (Bytes.concat acc (Bytes.of #list(97 98))))))
      (def
        (main (: n Int64))
        (match
          (String.from-bytes (build n (Bytes.of #list())))
          ((Some s) (String.byte-len s))
          ((None u) -1)))
      (export main)))
  (call main (: 200 Int64))
  (output (: 400 Int64))
  (live-objects 0))

(case
  "a Bytes slice of a slice composes offsets against the VIEW and bounds against its length"
  (doc
    "The BYTES twin of the string slice-of-slice pin (Bytes.slice takes start+LENGTH, not
           start+end): outer = `(Bytes.slice b 1 4)` over [10,20,30,40,50,60] is the 4-byte view
           [20,30,40,50]; the inner `(Bytes.slice outer k 2)` at k=1 reads [30,40] — view-relative
           (3040); resolving against the BASE gives [20,30] (2030). At k=3 the inner needs view
           bytes 3..5 but the view has 4 — None (7), even though the BASE has bytes there; a
           base-resolved bounds check answers Some [40,50]. k=2 in-bounds tail [40,50] (4050).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.of #list(10 20 30 40 50 60)))
          (def outer (Option.expect (Bytes.slice b 1 4) "in"))
          (match
            (Bytes.slice outer k 2)
            ((Some v)
              (+
                (* 100 (match (Bytes.at v 0) ((Some x) x) ((None _u) -1)))
                (match (Bytes.at v 1) ((Some x) x) ((None _u) -1))))
            ((None _u) 7))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3040 Int64))
  (call main (: 2 Int64))
  (output (: 4050 Int64))
  (call main (: 3 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "String.from-bytes over a rope-backed slice accepts aligned windows and rejects a mid-scalar cut"
  (doc
    "Composes three byte-layer features the existing pins cover only pairwise: a Bytes ROPE whose
           seam falls INSIDE a 2-byte scalar ([97,195] ++ [169,240,159,152,128] — the bytes of \"aé😀\"
           split mid-é), a SLICE window over that rope, and UTF-8 validation of the window. mode 1:
           window [0,3) = \"aé\" — the decode must stitch é across the seam (scalar-len 2). mode 2:
           window [0,4) ends after 😀's LEAD byte — a structurally-torn window, from-bytes None (-1).
           mode 3: window [3,7) is exactly the emoji's four bytes — Some, one scalar (1). A decoder
           that validated per-leaf (or clamped the window to the seam) flips mode 1 or mode 3.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(97 195)) (Bytes.of #list(169 240 159 152 128))))
          (def lo (if (= mode 3) 3 0))
          (def ln (if (= mode 1) 3 4))
          (match
            (Bytes.slice b lo ln)
            ((Some w) (match (String.from-bytes w) ((Some s) (String.scalar-len s)) ((None _u) -1)))
            ((None _u) -2))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "Map.swap keyed by a runtime Bytes ROPE replaces the flat-keyed entry"
  (doc
    "The BYTES leg of the value-yielding canonical-key pair (the Rational legs pin the
           normalize-at-construction rep; Bytes ropes are the canonicalize-at-EQ rep): the map holds
           flat [1,2,3] -> 10 and the swap key is the RUNTIME rope `[1,2] ++ [x]`. x=3: the rope
           content-equals the flat key, so swap REPLACES — prior `(Some 10)`, len 1, flat lookup reads
           the new 20 (1120). x=4: `[1,2,4]` is a new key — prior `(None unit)`, ADDED (len 2), flat
           keeps 10 (210). A swap that hashed the rope's leaf structure instead of its content adds a
           phantom entry at x=3 (220).")
  (input
    (do
      (def
        (main (: x UInt8))
        (do
          (def m (Map.insert Map.empty (Bytes.of #list(1 2 3)) 10))
          (def k (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(x))))
          (def r (Map.swap m k 20))
          (def prior (match (. r 0) ((Some v) (if (= v 10) 1 -9)) ((None _u) 0)))
          (def m2 (. r 1))
          (+
            (* 1000 prior)
            (+
              (* 100 (Map.len m2))
              (match (Map.lookup m2 (Bytes.of #list(1 2 3))) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main (: 3 UInt8))
  (output (: 1120 Int64))
  (call main (: 4 UInt8))
  (output (: 210 Int64)))

; A `String.to-bytes` result over a RUNTIME rope, bound once and read across FOUR OR MORE branch
; arms, MUST compile to a VALID module on every backend. Regression: at ≥4 arms the nested if-chain
; lowers to a `br_table`, and the i64 dispatch index used to land in the same scratch slot (`base`)
; that the arm bodies reuse for `String.to-bytes`'s inlined i32 Bytes handle — one local, two widths
; → the module was WRITTEN but failed wasm validation (`func N failed to validate: expected i32,
; found i64`); 3 arms stayed a linear if-chain (no br_table, no collision). Fixed by floating the
; arm scratch to base+1 so the br_table index keeps its own slot. (breaker/corpus-bugfix routed →
; v-wasm-opt 4f9658803.) The 4 modes exercise len / at-0 / at-2 / at-3 of the shared handle.
(case
  "String.to-bytes of a runtime rope reused across four branch arms compiles to a VALID module"
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def s (String.concat "ab" (if (< mode 100) "cd" "zz")))
          (def bs (String.to-bytes s))
          (if
            (= mode 1)
            (Bytes.len bs)
            (if
              (= mode 2)
              (match (Bytes.at bs 0) ((Some b) b) ((None _u) -1))
              (if
                (= mode 3)
                (match (Bytes.at bs 2) ((Some b) b) ((None _u) -1))
                (match (Bytes.at bs 3) ((Some b) b) ((None _u) -1)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4 Int64))
  (call main (: 2 Int64))
  (output (: 97 Int64))
  (call main (: 3 Int64))
  (output (: 99 Int64))
  (call main (: 4 Int64))
  (output (: 100 Int64)))

(case
  "a recursive drop-byte walk rebinding its Bytes param to a slice-concat converges"
  (doc
    "The BYTES twin of the string-shrinker invalid-module finding (and its clean control): the
           helper drops byte i via `Bytes.slice[0,i) ++ Bytes.slice[i+1, len-i-1)`, and the recursive
           walk REBINDS its param to the helper's result with the exit test reading `Bytes.len` of the
           rebound value — the exact shape that emits an invalid module for String (scalar-len on the
           rebound rope), clean here because Bytes.len is a stored length. Greedy walk from [1,2,3,4,5]
           drops indices 0,1,2 then exits (len 2, the bytes [2,4]). Pins the working side of the seam
           so the String fix can be verified against an unchanged Bytes baseline.")
  (input
    (do
      (def
        (d (: b Bytes) (: i Int64))
        (Bytes.concat
          (Option.expect (Bytes.slice b 0 i) "lo")
          (Option.expect (Bytes.slice b (+ i 1) (- (- (Bytes.len b) i) 1)) "hi")))
      (def (walk (: b Bytes) (: i Int64)) (if (>= i (Bytes.len b)) b (walk (d b i) (+ i 1))))
      (def (main (: mode Int64)) (Bytes.len (walk (Bytes.of #list(1 2 3 4 5)) 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

; --- Byte-wise reversal over a seamed rope. ---
(case
  "a byte-wise reversal over a seamed rope is an involution and lands bytes at mirrored offsets"
  (doc
    "10-bytes walks ropes FORWARD everywhere; reversal runs the seam in BOTH directions: per-byte reverse-index read + bin-append rebuild, rev∘rev=b, and mirrored offsets r[0]/r[2] catch an off-by-one at either end.")
  (input
    (do
      (def
        (brev (: b Bytes) (: i Int64) (: acc Bytes))
        (if
          (< i 0)
          acc
          (match
            (Bytes.at b i)
            ((Option.Some v) (brev b (- i 1) (Bytes.concat acc (bin (u8 (UInt8.wrap v))))))
            ((Option.None _u) acc))))
      (def (rev (: b Bytes)) (brev b (- (Bytes.len b) 1) (Bytes.of #list())))
      (def
        (main (: k Int64))
        (do
          (def b (Bytes.concat (Bytes.of #list(1 (UInt8.wrap k))) (Bytes.of #list(3))))
          (def r (rev b))
          (+
            (* 100 (if (= (rev r) b) 1 0))
            (+ (* 10 (Option.expect (Bytes.at r 0) "h")) (Option.expect (Bytes.at r 2) "t")))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 131 Int64))
  (live-objects known-leak))

; --- View-vs-rope equality both directions, and Bytes.compact as a CHAMP key. ---
(case
  "a Bytes slice VIEW and a rope compare equal by content in both directions"
  (doc
    "The BYTES twin of the string view×rope compare pin: a borrowed [off,len] view
           (`slice([9,1,2,3,7],1,3)` = [1,2,3]) against a concat rope `[1,2]++[m]` — content
           equality in BOTH operand orders (11 at m=3; 0 at m=4). Bytes has NO blessed order
           (ruled), so eq is the only cross-rep relation — a compare that walked the view's
           parent bytes (9-prefixed) or the rope's node structure breaks a direction; the
           both-directions read catches an asymmetric fast-path.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def view (Option.expect (Bytes.slice (Bytes.of #list(9 1 2 3 7)) 1 3) "in"))
          (def rope (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list((UInt8.wrap mode)))))
          (+ (* 10 (if (= view rope) 1 0)) (if (= rope view) 1 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 11 Int64))
  (call main (: 4 Int64))
  (output (: 0 Int64)))

(case
  "a Bytes-compact key and a flat rebuild both hit a rope-keyed map entry; a proper-prefix slice misses"
  (doc
    "compact's DERIVED value as a key against its parent-rope entry (the compact pins are value-eq only): compact re-boxes storage into an independent allocation — a hash keyed on storage identity/chunk layout misses the content-equal twin. compact(rope) hits (100), a flat rebuild hits (10), a 2-byte compact-slice PREFIX misses (0). The rust-target rows are TODO, tracked with the Bytes-CHAMP-key family.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def rope (Bytes.concat (Bytes.of #list(10 (UInt8.wrap k))) (Bytes.of #list(30))))
          (def m (Map.insert Map.empty rope 42))
          (+
            (*
              100
              (match (Map.lookup m (Bytes.compact rope)) ((Option.Some v) 1) ((Option.None _u) 0)))
            (+
              (*
                10
                (match
                  (Map.lookup m (Bytes.of #list(10 (UInt8.wrap k) 30)))
                  ((Option.Some v) 1)
                  ((Option.None _u) 0)))
              (match
                (Map.lookup m (Option.expect (Bytes.slice (Bytes.compact rope) 0 2) "lo"))
                ((Option.Some v) 1)
                ((Option.None _u) 0))))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 110 Int64)))

; --- The packet idiom: Bytes fields projected and re-framed. ---
(case
  "TWO Bytes fields project from a record and re-concat into a frame (the packet idiom)"
  (doc
    "Bytes in a record field exists for EQ only (rope-vs-flat); here TWO Bytes fields — flat header, seamed rope body — PROJECT and re-CONCAT into a frame with position reads across the re-joined seams (hdr+body: [1,2]+[10,k,30] -> len 5, [1]=2, [3]=k).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def
            pkt
            #record((= hdr (Bytes.of #list(1 2)))
              (= body (Bytes.concat (Bytes.of #list(10 (UInt8.wrap k))) (Bytes.of #list(30))))))
          (def total (Bytes.concat pkt.hdr pkt.body))
          (+
            (* 100 (Bytes.len total))
            (+
              (* 10 (Option.expect (Bytes.at total 1) "b1"))
              (Option.expect (Bytes.at total 3) "b3")))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 540 Int64)))

; --- Construction-path equality for runtime byte ropes (the string companions live in
; 13-strings; the const concat identities above fold before emit). ---
(case
  "a runtime concat-reached Bytes equals the directly-built Bytes and not a decoy"
  (doc
    "The runtime-operand face of the concat==flat identity (the const pin folds before emit):
           `via` splices a runtime leaf [k] onto [3,4] — a genuine two-leaf rope — and must equal the
           directly-built [k,3,4] (tens digit, both k) but not the decoy [k,3,5] (ones digit) → 10.
           The Bytes companion of the cross-path string pin in 13-strings.")
  (input
    (do
      (def (via (: k Int64)) (Bytes.concat (Bytes.of #list((UInt8.wrap k))) (Bytes.of #list(3 4))))
      (def (direct (: k Int64)) (Bytes.of #list((UInt8.wrap k) 3 4)))
      (def (decoy (: k Int64)) (Bytes.of #list((UInt8.wrap k) 3 5)))
      (def
        (main (: k Int64))
        (+ (* 10 (if (= (via k) (direct k)) 1 0)) (if (= (via k) (decoy k)) 1 0)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 10 Int64))
  (call main (: 7 Int64))
  (output (: 10 Int64)))

(case
  "a slice of a runtime Bytes rope equals the directly-built window"
  (doc
    "The slice-of-rope face for Bytes (the string twin lives in 13-strings): rope [k]++[3,4,5];
           the window at start 1 len 2 ([3,4], based entirely in the second leaf after the seam at
           index 1) and the window at start 0 len 1 ([k], the first leaf alone) must both equal their
           directly-built twins → 11. A slice that re-based its window against the wrong leaf (or read
           through the seam without rebasing) flips a leg.")
  (input
    (do
      (def
        (rope (: k Int64))
        (Bytes.concat (Bytes.of #list((UInt8.wrap k))) (Bytes.of #list(3 4 5))))
      (def
        (main (: k Int64))
        (+
          (*
            10
            (if
              (= (Option.expect (Bytes.slice (rope k) 1 2) "in bounds") (Bytes.of #list(3 4)))
              1
              0))
          (if
            (=
              (Option.expect (Bytes.slice (rope k) 0 1) "in bounds")
              (Bytes.of #list((UInt8.wrap k))))
            1
            0)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 11 Int64)))

; --- The kept-binding family for `Bytes.concat` (adv-54b): a let-bound concat result is a
; RUNTIME COMPUTATION — copy-propagating it would recompute per read and consume its source
; rope on each recompute (the adv-54 StrSlice/StrToBytes and adv-66 Bytes.compact family;
; lower.rs is_runtime_computation). These pin the fix's behavior at the corpus tier — the
; landing carried only a Rust unit witness. ---
(case
  "a let-bound Bytes.concat result is read three ways (len, index, order-compare)"
  (doc
    "`rope` is built by recursive concat (n leaves); `joined = (Bytes.concat rope [66])` is then
           read THREE times: `Bytes.len` (n+1), `Bytes.at joined n` (the appended 66), and an
           order-compare against the still-live source `rope` (a strict prefix, so rope < joined).
           n=10 → 11 + 100·66 + 100000·1 = 106611; n=2 → 106603. A copy-propagated concat would
           recompute per read, consuming `rope` — the second read faults or reads freed leaves
           (adv-54b's OOB shape).")
  (input
    (do
      (def
        (build-rope (: n Int64) (: acc Bytes))
        (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def
        (main (: n Int64))
        (let
          ((rope (build-rope n (Bytes.of #list()))))
          (let
            ((joined (Bytes.concat rope (Bytes.of #list((UInt8.wrap 66))))))
            (+
              (Bytes.len joined)
              (+
                (* 100 (match (Bytes.at joined n) ((Some v) (Int64.of v)) ((None _u) -1)))
                (* 100000 (if (< rope joined) 1 0)))))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 106611 Int64))
  (call main (: 2 Int64))
  (output (: 106603 Int64)))

(case
  "a concat-of-concat chain keeps every intermediate binding readable"
  (doc
    "Three let-bound generations — `a` = [k], `ab` = a++[66], `abc` = ab++[67] — with EVERY
           generation read after the chain completes: len(a)=1, len(ab)=2, len(abc)=3, and the
           order-compare ab < abc (strict prefix) → 1 + 2·10 + 3·100 + 1·10000 = 10321. Each
           intermediate is BOTH a consumed concat operand and a later-read binding — the deep-chain
           face of the kept-binding rule (one un-kept generation frees a leaf the next read walks).")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((a (Bytes.of #list((UInt8.wrap k)))))
          (let
            ((ab (Bytes.concat a (Bytes.of #list((UInt8.wrap 66))))))
            (let
              ((abc (Bytes.concat ab (Bytes.of #list((UInt8.wrap 67))))))
              (+
                (Bytes.len a)
                (+ (* 10 (Bytes.len ab)) (+ (* 100 (Bytes.len abc)) (* 10000 (if (< ab abc) 1 0)))))))))
      (export main)))
  (call main (: 65 Int64))
  (output (: 10321 Int64)))

(case
  "Bytes.slice of a map-looked-up Bytes with perform-threaded start and len threads without a scratch collision"
  (doc
    "A `Bytes.slice` whose BYTES operand is looked up from a Map (an Option-returning lookup yielding a
           Bytes) and whose START/LEN are perform results, under a resumptive handler. Pins a wasm-codegen
           miscompile (v-wasm-opt-scouted, breaker-witnessed, 2026-08-05 — the sibling of the #2311
           closure-CallIndirect scratch-alias): the looked-up-Bytes operand is a dup-site `Core::SumPayload`
           whose Perceus retain floats its cell into a scratch slot typed i32; the `bytes-slice` emit ran all
           three operands (bytes i32, start/len i64) at the SAME base (`base + 4`), so a perform-threaded i64
           start/len materialized into that i32 slot → an i32/i64 collision (a wasm local has one type
           function-wide) → `bytes-slice`'s function failed to validate (invalid module). Fix: each operand
           emits at `(*high).max(base + 4)`, disjoint from the retain slot. table[1] = [10,20,30,40,50,60,70,80];
           the `next` handler threads s=1,2 so start=1,len=2 → slice = [20,30], len 2, first byte 20 → 10·2+20 =
           40. Correct on rust throughout (fold sound; the defect was purely wasm scratch allocation).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def table (Map.insert Map.empty 1 (Bytes.of #list(10 20 30 40 50 60 70 80))))
          (handle
            St
            n
            ((next (u) s (resume s (+ s 1))))
            (match
              (Map.lookup table 1)
              ((Some bs)
                (match
                  (Bytes.slice bs (St.next) (St.next))
                  ((Some sl)
                    (+
                      (* 10 (Bytes.len sl))
                      (match (Bytes.at sl 0) ((Some b) (Int64.of b)) ((None _u) -1))))
                  ((None _u) -100)))
              ((None _u) -200)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 40 Int64))
  (live-objects known-leak))

; ============================================================================================
; Byte-string-literal DISPATCH — a runtime Bytes value matched against `b"…"` whole-value literals
; (`match b (b"…" …) … (_ …)`), the Bytes twin of runtime string keyword/opcode dispatch. A Bytes is
; a heap value (not a scalar), so the scalar probe-chain cannot drive it; instead the match desugars to
; a chain of `(= b b"…")` `value-eq` content compares (a direct-Bytes `=` compacts each operand, so a
; rope compares by content). The catch-all `_`/binder arm is required (a Bytes match never exhausts by
; literals — an unequal sequence always falls through), exactly as a scalar/String match needs one.
; ============================================================================================
(case
  "a runtime Bytes value dispatches on byte-string literals"
  (doc
    "The byte-string dispatch idiom: a runtime `Bytes` scrutinee (built from a parameter, so it
           does not fold) matched against `b\"\\x01B\"` / `b\"\\x02B\"` literals with a catch-all. Each arm
           is a content `value-eq` against a freshly built literal leaf, so `[1,66]` selects arm 1,
           `[2,66]` arm 2, and any other sequence the `_` tail. Pins that a whole-value byte-string
           literal pattern dispatches a runtime Bytes by content — the magic-number / opcode-header idiom
           the `bin` binary matcher (16-binary-matching) generalizes.")
  (input
    (do
      (def (classify (: b Bytes)) (match b (b"\x01B" 1) (b"\x02B" 2) (_ 0)))
      (def (main (: n Int64)) (classify (Bytes.of #list((UInt8.wrap n) (UInt8.wrap 66)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64)))

(case
  "a guarded byte-string-literal arm gates a runtime Bytes match on a condition"
  (doc
    "A guarded byte-string arm `(guard b\"\\x01B\" (> n 10))` matches the literal AND the runtime
           guard; on a false guard it falls through to the plain `b\"\\x01B\"` arm — the Bytes twin of a
           guarded scalar/string arm. Same scrutinee `[1,66]` both calls: n=20 takes the guarded arm (1),
           n=5 falls through to the unguarded literal arm (2). Pins that a guard nests inside the matched
           byte-literal branch and threads its else to the next arm.")
  (input
    (do
      (def
        (classify (: b Bytes) (: n Int64))
        (match b ((guard b"\x01B" (> n 10)) 1) (b"\x01B" 2) (_ 0)))
      (def (main (: n Int64)) (classify (Bytes.of #list((UInt8.wrap 1) (UInt8.wrap 66))) n))
      (export main)))
  (call main (: 20 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "a byte-string literal nested in a sum payload refines a runtime Bytes"
  (doc
    "The nested-sum face — `(Some b\"\\x01B\")` refines a runtime `Some` payload by CONTENT (the
           Bytes twin of a nested string-literal payload like `(Some \"…\")`, `core-semantics.md #Pattern
           Matching`: a literal refines the match). The decision tree tests the `Some` discriminant, then
           the payload's bytes against each literal; `[1,66]`→1, `[2,66]`→2, another `Some` payload→the
           `(Some _)` binding arm (3), `None`→0. Pins that a byte-literal payload probe emits a runtime
           `value-eq` leaf compare inside the sum decision tree, not only at a top-level scrutinee.")
  (input
    (do
      (def
        (classify (: o (Option Bytes)))
        (match o ((Some b"\x01B") 1) ((Some b"\x02B") 2) ((Some _) 3) ((None) 0)))
      (def (main (: n Int64)) (classify (Some (Bytes.of #list((UInt8.wrap n) (UInt8.wrap 66))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; Byte-string-literal pattern WELL-FORMEDNESS edges — the same rules a scalar/String match obeys, keyed
; per-arm on the pattern's own kind (a byte-string pattern expects `Bytes`, a text pattern expects
; String/Symbol). These pin the invariants so a future change to the dispatch lowering cannot quietly
; relax them.
(case
  "a byte-string pattern over a String scrutinee is a type error"
  (doc
    "A `b\"AB\"` pattern arm over a definitely-`String` scrutinee crosses the Bytes/String boundary
           → CDZ0201, the byte-string twin of the symbol/string crossing (17-symbols). The pattern's
           expected type comes from its OWN kind (`Bytes`), not the scrutinee, so a text scrutinee does
           not excuse a byte-literal arm.")
  (input
    (do
      (def (classify (: s String)) (match s (b"AB" 1) (_ 0)))
      (def (main (: k Int64)) (classify (if (= k 0) "AB" "x")))
      (export main)))
  (error CDZ0201))

(case
  "a String-literal pattern over a Bytes scrutinee is a type error"
  (doc
    "The mirror: a `\"AB\"` String-literal arm over a definitely-`Bytes` scrutinee crosses the
           boundary → CDZ0201. Pins that the per-arm kind check fires whichever kind the crossing arm
           carries, so a Bytes scrutinee admits only byte-string / wildcard arms.")
  (input
    (do
      (def (classify (: b Bytes)) (match b ("AB" 1) (_ 0)))
      (def (main (: k Int64)) (classify (Bytes.of #list((UInt8.wrap k) (UInt8.wrap 66)))))
      (export main)))
  (error CDZ0201))

(case
  "a runtime Bytes match without a catch-all is non-exhaustive"
  (doc
    "A Bytes match is never exhausted by literals (an unequal byte sequence always falls through),
           so `(match b (b\"AB\" …) (b\"CD\" …))` with NO wildcard is non-exhaustive → CDZ0210 — the same
           rule a scalar/String match obeys (`core-semantics.md #Matching Is Exhaustive Or Rejected`).")
  (input
    (do
      (def (classify (: b Bytes)) (match b (b"AB" 1) (b"CD" 2)))
      (def (main (: k Int64)) (classify (Bytes.of #list((UInt8.wrap k) (UInt8.wrap 66)))))
      (export main)))
  (error CDZ0210))

(case
  "an empty byte-string literal pattern matches the empty Bytes"
  (doc
    "The empty byte-string literal `b\"\"` is a whole-value literal like any other — it matches
           exactly the empty `Bytes` by content, distinct from a one-byte `b\"A\"`. Pins the zero-length
           edge of byte-string dispatch (an empty payload is a real value, not a degenerate no-op).")
  (input
    (do
      (def (classify (: b Bytes)) (match b (b"" 1) (b"A" 2) (_ 0)))
      (def (main (: k Int64)) (classify (if (= k 0) (Bytes.of #list()) (Bytes.of #list(65)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

; A runtime `(bin …)` construction builds a FRESH OWNED Bytes on the rope heap; a borrowing consumer
; (`Bytes.len`) must drop that owned temporary or it leaks. These pin the producer-reclaim balance for the
; `(bin …)`/`(bits …)` builders (BinBuild/BinBitsBuild = Owned) — `(live-objects 0)` on the debug-counters
; runtime. Runtime `n` (via `UInt8.wrap`) so the `(bin …)` can't const-fold; `main` returns the scalar
; length, so the only heap traffic is the built Bytes.
(case
  "a runtime bin construction borrowed by Bytes.len leaves no live heap objects"
  (doc
    "`(Bytes.len (bin (u8 n') (u8 n'+1) (u8 n'+2)))` over a runtime `n` builds a fresh 3-byte owned
           Bytes and reads its length (3); the owned Bytes leaf must be dropped after the borrowing length
           read. n=3 -> length 3, live-objects 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (Bytes.len (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))) (u8 (UInt8.wrap (+ n 2))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a runtime bit-field bin construction borrowed by Bytes.len leaves no live heap objects"
  (doc
    "The `(bits …)` (BinBitsBuild) sibling — two full-byte (width-8) bit-field segments build a 2-byte
           owned Bytes: `(Bytes.len (bin (bits n' 8) (bits n'+1 8)))`. n=1 -> length 2, and the fresh
           bit-packed Bytes must be dropped after the borrowing length read — live-objects 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (Bytes.len (bin (bits (UInt8.wrap n) 8) (bits (UInt8.wrap (+ n 1)) 8))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects 0))

; -- breaker batch 414 (2026-08-26): Bytes.of over a NON-literal (branch-selected) list lowers.
(case
  "ce06 Bytes.of of a NON-literal (branch-selected) list lowers"
  (input
    (do
      (def
        (f (: k Int64))
        (Bytes.len
          (Bytes.of (if (> k 0) #list((UInt8.wrap 65) (UInt8.wrap 66)) #list((UInt8.wrap 67))))))
      (export f)))
  (call f 1)
  (output (: 2 Int64))
  (live-objects known-leak))

; -- breaker batch 445 (2026-08-27): static-data drop-safety for deduplicated constant Bytes
; (#3837 extended constant-Bytes detection to Core::ConstBytes; both occurrences of a byte-identical
; literal dedup onto one const_byte_slice). These pin that the SHARED static survives Perceus drops:
; a program may use the same literal twice with a drop between (no use-after-free on the second use),
; and a 50-frame construct+drop loop over the shared constant reclaims to zero (a per-eval fresh
; allocation would also pass the value check — the live-objects 0 is what pins the balance either way;
; a drop that freed the shared static would trap or misread here).
(case
  "sbd1 two occurrences of one constant Bytes literal — branch-selected use, length reads, and runtime equality across the pair"
  (doc
    "`a` branch-selects (on the runtime arg) between the shared literal and a different one; `b` is a
           second occurrence of the same literal. n=1 takes the shared arm: 100*len(a) + len(b) + 1000 if
           a=b -> 100*20 + 20 + 1000 = 3020. The runtime `=` compares a deduplicated constant against its
           own second occurrence; the intermediate drops of `a`/`b` must not free the static. MUST be 3020,
           live-objects 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (if (= n 1) b"const-shared-payload" b"other"))
            (x (Bytes.len a))
            (b b"const-shared-payload"))
          (+ (* 100 x) (+ (Bytes.len b) (if (= a b) 1000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3020 Int64))
  (live-objects 0))

(case
  "sbd2 a fifty-frame recursion re-evaluating a constant Bytes literal each frame reclaims to zero"
  (doc
    "Per-frame amplification over the shared static: each frame branch-selects (parity of k) between
           the shared literal (len 20) and a second literal (len 15), reads the length, and drops. n=50 ->
           25*20 + 25*15 = 875. A leak of even one object per evaluation reads >=50 here; a drop that freed
           the build-once static would corrupt later frames. MUST be 875, live-objects 0.")
  (input
    (do
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (let
            ((a (if (= (% k 2) 0) b"const-shared-payload" b"odd-frame-bytes")))
            (+ (Bytes.len a) (frames (- k 1))))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 875 Int64))
  (live-objects 0))

; -- breaker batch 447 (2026-08-27): the two ConstBytes REPRESENTATIONS #3837 unifies, probed at
; runtime. A compile-time-constant Bytes reaches the backend two ways — a Core::BytesOf of constants
; (the b"…" reader sugar) and a baked Core::ConstBytes leaf (a const-transform fold such as
; Blake3.of) — and #3837 makes both build-once hoist targets. These pin content-correctness and
; drop-safety across that split; sbd1/sbd2 above pin the single-representation case.
(case
  "mrd1 a byte-string literal and a const-folded Bytes.of with identical content compare equal at runtime"
  (doc
    "`a` branch-selects (runtime arg) between b\"ABC\" and a different literal; `b` is
           `(Bytes.of (list 65 66 67))` — the same three bytes reaching the backend as a folded constant.
           n=1: 100*3 + 3 + 1000 (a=b) = 1303. Whether the two representations dedup onto one static or
           stay separate, content equality and both length reads must hold, and both drop clean. MUST be
           1303, live-objects 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (if (= n 1) b"ABC" b"zz")) (b (Bytes.of #list(65 66 67))))
          (+ (* 100 (Bytes.len a)) (+ (Bytes.len b) (if (= a b) 1000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1303 Int64))
  (live-objects 0))

(case
  "mrd2 a Blake3-baked constant Bytes used twice with drops between — equal hashes, thirty-two bytes each, no live objects"
  (doc
    "`Blake3.of` of a constant folds to a baked ConstBytes leaf (the representation #3837 newly
           recognizes as a hoist target). `h` branch-selects between the hashes of two different constant
           inputs; `h2` re-evaluates the shared one. n=1: both are the 32-byte hash of b\"stable-input\" ->
           100*32 + 32 + 1000 (h=h2) = 4232, and the drops between uses must not free the baked payload.
           MUST be 4232, live-objects 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((h (if (= n 1) (Blake3.of b"stable-input") (Blake3.of b"other-input")))
            (x (Bytes.len h))
            (h2 (Blake3.of b"stable-input")))
          (+ (* 100 x) (+ (Bytes.len h2) (if (= h h2) 1000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4232 Int64))
  (live-objects 0))

; ── the INSERT-direction Bytes-rope map-key face (migrated from rcdzc): insert under a rope key, found by its flat twin, 0-leak ──
(case
  "a map keyed by inserting under a runtime Bytes ROPE is found by its flat twin and adds no leak"
  (doc
    "The INSERT-direction Bytes rope map-key face (the lookup-by-rope and swap-by-rope directions are
           pinned elsewhere): insert an entry UNDER an owned runtime Bytes rope key `(rep [104] 1)` =
           [104,120], then look up with the flat literal [104,120] -> 42. The compiler bytes-compacts a
           String/Bytes key at every champ site (here the INSERT site), so the rope key hashes+compares to
           the flat twin's slot (was None -> -1, a champ_hash/champ_eq physical-byte miss). The owned rope
           key is consumed by the insert and its compaction is refcount-neutral, so the program reclaims
           fully -> live-objects 0.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def
        (main)
        (match
          (Map.lookup
            (Map.insert (Map.empty) (rep (Bytes.of #list(104)) 1) 42)
            (Bytes.of #list(104 120)))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (call main)
  (output (: 42 Int64))
  (live-objects 0))

; -- breaker batch 460 (2026-08-27): the Bytes.slice extract-leak isolated on an IMMORTAL source.
; Post-#3869 a constant Bytes literal is census-excluded, so osx4's baseline is 0 and osx3
; isolates what the slice extraction itself leaks: exactly 2 (the Some-wrapped slice view, never
; released after the unwrap). Same Option-shell-mediated extraction shape as the lar1/mlr family
; (List.at/Map.lookup dup-retain, routed to reclaim-placement increment 2) — the slice variant
; wraps a FRESH view rather than a retained element, so it is the adjacent cell, filed with the
; family rather than confirmed identical. Runtime-source slice deltas read larger (~13) only
; because the Bytes.of fold's own leak class compounds on top.
(case
  "osx3 a Bytes.slice of an immortal constant source matched by Some and borrowed leaks the view"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (if (= n 1) b"const-shared-payload" b"other")))
          (match (Bytes.slice a 2 3) ((Option.Some s) (Bytes.len s)) ((Option.None) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "osx4 the immortal constant source alone is census-excluded (the zero baseline for osx3)"
  (input
    (do
      (def
        (main (: n Int64))
        (let ((a (if (= n 1) b"const-shared-payload" b"other"))) (Bytes.len a)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 20 Int64))
  (live-objects 0))

; -- adv-54/adv-54b runtime slice-view to-bytes read-twice soundness (behavioral migration from rcdzc, 2026-08-27):
; a let-bound to-bytes / Bytes.concat of a RUNTIME String.slice view, read MORE THAN ONCE, must see the right
; bytes on every read (the op must be named once, not copy-propagated + recomputed off a consumed buffer).
(case
  "adv54 a let-bound to-bytes of a runtime string slice read twice sees both bytes"
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((s (String.concat "ab" "cdé")))
          (match
            (String.slice s 3 5)
            ((Some tail)
              (let
                ((b (String.to-bytes tail)))
                (+
                  (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                  (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
            ((None _u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 295 Int64))
  (live-objects known-leak))

(case
  "adv54b a let-bound Bytes.concat of slice-view to-bytes read twice sees the concatenated bytes"
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((s (String.concat "ab" "cdé")))
          (match
            (String.slice s (+ 3 k) (+ 5 k))
            ((Some tail)
              (let
                ((b (Bytes.concat (String.to-bytes tail) (String.to-bytes tail))))
                (+
                  (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                  (Int64.of (Option.expect (Bytes.at b 3) "b3")))))
            ((None _u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 200 Int64))
  (live-objects known-leak))

; -- runtime Bytes.at / concat / slice / compact behavior (migration from rcdzc bytes cdz-run tests, 2026-08-27):
; each threads a byte sequence through a fn param so the op runs (not a fold) and reads a scalar out.
(case
  "a runtime Bytes.at byte-sum reads each byte and terminates on the out-of-bounds None"
  (input
    (do
      (def (sum bs i) (match (Bytes.at bs i) ((Some b) (+ b (sum bs (+ i 1)))) ((None _) 0)))
      (def (main) (sum (Bytes.of #list(10 20 30)) 0))
      (export main)))
  (call main)
  (output (: 60 Int64)))

(case
  "a runtime-element Bytes.at read widens the byte to the Int64 Option payload"
  (input
    (do
      (def (main (: n UInt8)) (match (Bytes.at (Bytes.of #list(n)) 0) ((Some x) x) ((None _) -1)))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 5 Int64)))

(case
  "a runtime Bytes.concat length is the sum of operand lengths"
  (input
    (do
      (def (mk n) (Bytes.of #list(n 20 30)))
      (def (main) (Bytes.len (Bytes.concat (mk 10) (mk 40))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a runtime Bytes.slice in bounds yields Some of the sub-length"
  (input
    (do
      (def (mk n) (Bytes.of #list(n 20 30 40)))
      (def (main) (match (Bytes.slice (mk 10) 1 2) ((Some s) (Bytes.len s)) ((None _) -1)))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a runtime Bytes.slice out of bounds is None"
  (input
    (do
      (def (mk n) (Bytes.of #list(n 20 30 40)))
      (def (main) (match (Bytes.slice (mk 10) 3 5) ((Some s) (Bytes.len s)) ((None _) -1)))
      (export main)))
  (call main)
  (output (: -1 Int64)))

(case
  "a runtime Bytes.compact preserves the content length"
  (input
    (do
      (def (mk n) (Bytes.of #list(n 20 30)))
      (def (main) (Bytes.len (Bytes.compact (mk 10))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a runtime if-produced Bytes is length-measured with valid wasm"
  (input
    (do
      (def
        (main (: b Int64))
        (Bytes.len (if (> b 0) (Bytes.of #list(1 2 3)) (Bytes.of #list(4 5)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (call main (: -1 Int64))
  (output (: 2 Int64)))

; ── breaker batch 557: DEEP-Bytes rope cells (the rp1-3 string-depth analogs; 50-concat trees).
; API note pinned in bdr3: `(Bytes.slice b start LENGTH)` (documented) — the String.slice twin
; takes (start, END-exclusive); the divergence is the documented contract, and bdr3's oracle
; exercises the LENGTH reading (slice(1, 52) of a 53-byte rope = 52 bytes).
(case
  "bdr1 a 50-concat deep Bytes rope's len walks the tree and the rope reclaims clean"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(7))) (- k 1))))
      (def
        (main (: n Int64))
        (Bytes.len (grow (if (> n 0) (Bytes.of #list(1 2 3)) (Bytes.of #list(9))) 50)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 53 Int64))
  (live-objects 0))

(case
  "bdr2 a byte-scan with Bytes.at across every seam of a 50-concat rope counts exactly (scalar reads; fixed 1-cell residue)"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(7))) (- k 1))))
      (def
        (cnt (: b Bytes) (: i Int64) (: acc Int64))
        (if
          (= i (Bytes.len b))
          acc
          (cnt b (+ i 1) (if (= (match (Bytes.at b i) ((Some v) v) ((None u) 0)) 7) (+ acc 1) acc))))
      (def
        (main (: n Int64))
        (cnt (grow (if (> n 0) (Bytes.of #list(1 2 3)) (Bytes.of #list(9))) 50) 0 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 50 Int64))
  (live-objects known-leak))

(case
  "bdr3 a slice SPANNING the seams of a deep Bytes rope reads exact length and content (start+LENGTH contract)"
  (input
    (do
      (def
        (grow (: b Bytes) (: k Int64))
        (if (= k 0) b (grow (Bytes.concat b (Bytes.of #list(7))) (- k 1))))
      (def
        (main (: n Int64))
        (let
          ((r (grow (if (> n 0) (Bytes.of #list(1 2 3)) (Bytes.of #list(9))) 50)))
          (match
            (Bytes.slice r 1 (- (Bytes.len r) 1))
            ((Some sl)
              (+ (* 100 (Bytes.len sl)) (match (Bytes.at sl 10) ((Some v) v) ((None u) -1))))
            ((None u2) -99))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 5207 Int64))
  (live-objects 0))
