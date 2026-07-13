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

(case "a byte sequence is constructed from a list of integers in range"
  (doc    "Witnesses that the seed realizes a Bytes value form: Bytes.of maps a list of
           Int64 in 0..=255 to an immutable byte sequence. This is the value the
           Cadenza-authored compiler builds a component's wasm bytes up as. Its canonical
           OBSERVABLE form is the byte-string display `b\"…\"` (options/binary-syntax): a printable
           ASCII byte stands for itself and any other byte is a `\\xNN` escape, so bytes 1, 2, 3 —
           all non-printable — render `b\"\\x01\\x02\\x03\"`.")
  (input  (Bytes.of (list 1 2 3)))
  (output (: b"\x01\x02\x03" Bytes)))

(case "byte sequences are equal by their bytes in order"
  (doc    "Witnesses Bytes structural equality: two byte sequences are equal exactly
           when they carry the same bytes in the same order (core-semantics.md
           #Equality Is Structural, at the Bytes value form).")
  (input  (= (Bytes.of (list 10 20 30)) (Bytes.of (list 10 20 30))))
  (output (: true Bool)))

(case "the length of a byte sequence is its byte count"
  (doc    "Witnesses Bytes.len — the compiler needs a byte count to lay out a wasm
           section's size prefix.")
  (input  (Bytes.len (Bytes.of (list 0 255 128))))
  (output (: 3 Int64)))

(case "concatenating two byte sequences appends their bytes in order"
  (doc    "Witnesses Bytes.concat — the compiler assembles a wasm module by
           concatenating encoded sections.")
  (input  (= (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4)))
             (Bytes.of (list 1 2 3 4))))
  (output (: true Bool)))

(case "indexing a byte sequence returns Some of the byte at that position"
  (doc    "Witnesses fallible Bytes indexing (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping): an in-bounds index yields the byte as an Int64 in 0..=255 wrapped in Some.")
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 1))
  (output (: (Some 8) (Option Int64))))

(case "constructing a byte sequence with a value out of range is a type error"
  (doc    "A byte IS a UInt8 (collections-and-text.md #A Byte Is A UInt8), so `Bytes.of` takes a `(List
           UInt8)`: a byte outside 0..=255 has NO UInt8 value, so `256` is rejected at COMPILE TIME as an
           out-of-range width literal (CDZ0302) rather than trapping at run time. This is stronger than a
           runtime trap — the ill-formed byte cannot even be constructed. To turn a wider integer into a
           byte, TRUNCATE deliberately with `(UInt8.wrap n)` (total, never traps); a bare `256` is not a
           truncation request, it is an ill-typed literal.")
  (input  (Bytes.of (list 0 256)))
  (error  CDZ0302))

; The out-of-range case above tests the HIGH end (256 > 255); a byte value is a UInt8, bounded on BOTH
; sides, so the LOW end matters too — a NEGATIVE literal is not a UInt8 either and is rejected the same
; way at compile time. Neither is silently masked into range: truncation into a byte is the explicit
; `UInt8.wrap`, never an implicit narrowing of an out-of-range literal.

(case "constructing a byte sequence with a negative value is a type error"
  (doc    "`(Bytes.of (list -1))` gives a byte value below 0 — no UInt8 has value -1 — so it is rejected
           at COMPILE TIME (CDZ0302), the low-end companion of the `256` case. A byte is a UInt8; a UInt8
           literal is bounded on BOTH sides. NOT wrapped to 255 via a truncating `as u8`: to truncate a
           wider value into a byte you write `(UInt8.wrap -1)` = 255 explicitly.")
  (input  (Bytes.of (list -1)))
  (error  CDZ0302))

(case "indexing a byte sequence out of bounds yields None"
  (doc    "Witnesses fallible Bytes indexing on the absent side, mirroring List.at
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping): an out-of-bounds
           index yields None rather than trapping.")
  (input  (Bytes.at (Bytes.of (list 7 8 9)) 5))
  (output (: (None unit) (Option Int64))))

(case "indexing a byte sequence with a negative index yields None"
  (doc    "`(Bytes.at (Bytes.of (list 7 8 9)) -1)` uses a negative index — no byte at position -1 — so
           it MUST yield None (fallible Bytes indexing), NOT cast the negative index to a large unsigned
           offset and read an unspecified byte. The negative-index companion of the out-of-bounds `5`
           case above, mirroring the List.at negative-index case (05-compound-types).")
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
  (input  (Bytes.len (Bytes.of (list))))
  (output (: 0 Int64)))

(case "two empty byte sequences are equal"
  (doc    "`(= (Bytes.of (list)) (Bytes.of (list)))` is true: two zero-length byte sequences carry the
           same (empty) bytes in order, so they are structurally equal (core-semantics.md #Equality Is
           Structural, at the Bytes value form). Pins that Bytes equality treats the empty sequence as a
           genuine value equal to itself, not a special-cased nothing.")
  (input  (= (Bytes.of (list)) (Bytes.of (list))))
  (output (: true Bool)))

(case "concatenating an empty byte sequence on the right is the identity"
  (doc    "`(Bytes.concat b (Bytes.of (list)))` = b: appending zero bytes changes nothing. Pins the
           right identity of Bytes.concat — a concat that mishandles a zero-length operand (e.g. writes a
           stray length prefix) would break the compiler's section assembly.")
  (input  (= (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list))) (Bytes.of (list 1 2))))
  (output (: true Bool)))

(case "concatenating an empty byte sequence on the left is the identity"
  (doc    "The left-identity companion: `(Bytes.concat (Bytes.of (list)) b)` = b. Pins that
           concatenation handles a zero-length LEFT operand too, not only a zero-length right operand —
           both sides of concat treat the empty sequence as the identity.")
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
  (input  (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")
             (Bytes.of (list 20 30))))
  (output (: true Bool)))

(case "the length of a slice is the slice's byte count"
  (doc    "`(Bytes.len (Option.expect (Bytes.slice b 1 2) …))` = 2: length reads the slice's OWN byte count, not
           the parent's. A view representation that stored a length must report the slice's length, never
           the backing sequence's — the sharing is not observable through length.")
  (input  (Bytes.len (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: 2 Int64)))

(case "indexing a slice is relative to the slice's start"
  (doc    "`(Bytes.at (Option.expect (Bytes.slice b 1 2) …) 0)` = Some 20: index 0 of the slice is the byte at
           the slice's start, not the parent's start. Pins that a view representation adds its offset —
           indexing is relative to the slice, so sharing the parent's storage is not observable through
           indexing.")
  (input  (Bytes.at (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds") 0))
  (output (: (Some 20) (Option Int64))))

(case "a slice spanning a concatenation sees the logical bytes"
  (doc    "Slicing across the seam of `(concat a b)` — `(Bytes.slice (concat (list 1 2) (list 3 4)) 1 2)`
           = Some `(Bytes.of (list 2 3))` — reads the LOGICAL bytes in order, independent of how the
           sequence was assembled. Pins that a slice over a deferred-concatenation representation crosses
           leaf boundaries correctly, seeing bytes not physical layout (#Sharing Is Not Observable).")
  (input  (= (Option.expect (Bytes.slice (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))) 1 2)
                     "slice is in bounds")
             (Bytes.of (list 2 3))))
  (output (: true Bool)))

(case "a zero-length slice is the empty byte sequence"
  (doc    "`(Bytes.slice b 2 0)` yields Some of the empty byte sequence — equal to `(Bytes.of (list))`.
           Pins the degenerate slice: taking zero bytes at an in-bounds start yields the identity of
           concatenation, present as Some, not None.")
  (input  (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 0) "slice is in bounds")
             (Bytes.of (list))))
  (output (: true Bool)))

(case "slicing past the end of a byte sequence yields None"
  (doc    "`(Bytes.slice b 2 3)` on a 4-byte sequence asks for 3 bytes starting at index 2 — running one
           byte past the end — so it MUST yield None rather than read beyond the sequence or return a
           short result (fallible, on the same footing as Bytes.at out-of-bounds).")
  (input  (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 3))
  (output (: (None unit) (Option Bytes))))

(case "slicing with a negative start yields None"
  (doc    "`(Bytes.slice b -1 2)` uses a start below 0 — no byte at position -1 — so it MUST yield None,
           NOT cast the negative start to a large unsigned offset. The negative-index companion of the
           past-the-end case, mirroring the Bytes.at negative-index None.")
  (input  (Bytes.slice (Bytes.of (list 10 20 30 40)) -1 2))
  (output (: (None unit) (Option Bytes))))

; --- Compacting a slice preserves its value while releasing shared storage ---------------
; A slice MAY retain its parent's whole storage to represent a small range of it (a view holds the
; parent alive). `(Bytes.compact b)` derives a value equal to `b` whose storage is independent of what
; `b` was derived from — the value-preserving materialization memory-and-resource-model.md #Retained
; Storage Is What A Value's Representation Holds Live requires, letting a program drop a large parent
; while keeping a small slice. Compacting changes STORAGE USE, never the VALUE: the compacted slice is
; equal to the slice by its bytes in order (#Equality Is Structural), so `compact` is not observable
; through any value operation.

(case "compacting a slice preserves its bytes"
  (doc    "`(Bytes.compact (Option.expect (Bytes.slice b 1 2) …))` = the same in-bounds slice: compacting
           materializes the slice into independent storage, changing resource use but not the value.
           Pins that compact is value-preserving — equal by bytes in order to the un-compacted slice
           (memory-and-resource-model.md #Retained Storage Is What A Value's Representation Holds Live).")
  (input  (= (Bytes.compact (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds"))
             (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) "slice is in bounds")))
  (output (: true Bool)))

(case "compacting is the identity on value for a whole byte sequence"
  (doc    "`(Bytes.compact b)` = `b`: compacting a sequence that already owns its storage changes
           nothing observable. Pins that compact is always value-preserving, whether or not the operand
           shares storage — it never alters the bytes, only (possibly) the storage backing them.")
  (input  (= (Bytes.compact (Bytes.of (list 1 2 3))) (Bytes.of (list 1 2 3))))
  (output (: true Bool)))

; --- Bytes as a RUNTIME value: construct, measure, and concatenate at run time ------------
; Every case above builds Bytes from literal integers, so the whole value is compile-time-known and
; folds to one baked constant. But the compiler's OWN interface is `compile: list<u8> -> result<list<u8>>`
; — it READS input bytes and BUILDS output bytes whose contents depend on runtime data. These cases pin
; that a byte sequence carrying a genuine runtime byte, or built by a runtime-recursive computation, is
; a first-class value: it lives on the value heap (packed, via the runtime's bytes-alloc/set/get/len),
; renders `(Bytes.of (list …))` identically to the const form, and supports len/concat at run time. This
; is the byte-level substrate a self-hosted compiler assembles a component's wasm bytes with.

(case "a byte sequence carrying a runtime byte value is a first-class value"
  (doc    "`(Bytes.of (list n 66 67))` with `n` a runtime parameter cannot fold to a constant — the
           first byte is decided at run time. The seed builds it on the value heap (bytes-alloc then
           bytes-set per byte, range-checking each to 0..=255) and the type-directed renderer walks it
           back to `b\"ABC\"`, byte-identical to a const byte sequence. Bytes 65 66 67 are the printable
           ASCII `A B C`, so the byte-string display shows them literally. Pins that Bytes is a runtime
           value, not only a compile-time literal — the compiler's output type flowing at run time.")
  (input  (do
            (def (mk n) (Bytes.of (list n 66 67)))
            (def (main)  (mk 65)) (export main)))
  (output (: b"ABC" Bytes)))

(case "a runtime wider integer is truncated into a byte by wrap"
  (doc    "The runtime companion of the byte-construction cases: `Bytes.of` takes a `(List UInt8)`, so a
           byte built from a WIDER runtime integer is truncated with `(UInt8.wrap n)` — total, keeping the
           low 8 bits (numeric-model.md #wrap Never Traps). `(UInt8.wrap 258)` = 2 (258 mod 256), so
           `(Bytes.of (list (UInt8.wrap 258)))` is the one-byte `b\"\\x02\"`, whose length is 1. Pins that
           the byte bound is carried by the UInt8 TYPE and that crossing into it from a wider value is the
           explicit total `wrap`, not a runtime range trap — there is no out-of-range byte to trap on,
           because a UInt8 is in range by construction. `Bytes.len` reads the result to a scalar (1).")
  (input  (do
            (def (mk n) (Bytes.len (Bytes.of (list (UInt8.wrap n)))))
            (def (main)  (mk 258)) (export main)))
  (output (: 1 Int64)))

(case "the length of a runtime byte sequence is its byte count"
  (doc    "`Bytes.len` of a byte sequence carrying a runtime byte: the seed folds a runtime Bytes value
           to a SCALAR count via the runtime's bytes-len, the fold-to-scalar half of the idiom (like a
           recursive list sum). `(Bytes.len (Bytes.of (list n 2 3)))` = 3 for any `n`.")
  (input  (do
            (def (sz n) (Bytes.len (Bytes.of (list n 2 3))))
            (def (main)  (sz 9)) (export main)))
  (output (: 3 Int64)))

(case "concatenating byte sequences built at run time appends their bytes in order"
  (doc    "`Bytes.concat` of two runtime byte sequences yields a genuine runtime value with the appended
           bytes. The representation MAY defer the concatenation — sharing the operands' storage under a
           concatenation node rather than copying their bytes into a fresh buffer — as an unobservable
           optimization (memory-and-resource-model.md #Sharing Is Not Observable), which keeps this case
           green either way. `(Bytes.concat (Bytes.of (list a)) (Bytes.of (list b 9)))` = `b\"\\x07\\x08\\t\"`
           for `a=7 b=8` — bytes 7 (BEL), 8 (backspace), 9 (tab) render as escapes (9 is the `\\t` special
           escape). Pins runtime concatenation — how a compiler joins the byte fragments of its output.")
  (input  (do
            (def (join a b) (Bytes.concat (Bytes.of (list a)) (Bytes.of (list b 9))))
            (def (main)      (join 7 8)) (export main)))
  (output (: b"\x07\x08\t" Bytes)))

(case "a recursively-built byte sequence assembles its bytes at run time"
  (doc    "The genuine self-hosting idiom for output: a byte sequence whose LENGTH is decided at run
           time, built by recursion + concatenation, not a fixed literal spine. `rep` prepends the byte
           88 `n` times onto the empty sequence, so how many bytes exist is known only at run time.
           `(rep 4)` = `b\"XXXX\"` (byte 88 is the printable ASCII `X`). This is exactly the shape a
           self-hosted compiler uses to emit a component's wasm bytes — concatenating byte fragments in
           a recursion whose depth is driven by the program being compiled.")
  (input  (do
            (def (rep n) (if (< n 1)
                            (Bytes.of (list))
                            (Bytes.concat (Bytes.of (list 88)) (rep (- n 1)))))
            (def (main)  (rep 4)) (export main)))
  (output (: b"XXXX" Bytes)))

(case "an unsigned LEB128 encoder emits the known-answer multibyte encoding"
  (doc    "The compiler's byte-emitting SPINE as one known-answer case: the recursive unsigned-LEB128
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
  (input  (do
            (def (uleb n)
              (if (< n 128)
                  (Bytes.of (list (UInt8.wrap n)))
                  (Bytes.concat (Bytes.of (list (UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
            (def (main) (uleb 624485)) (export main)))
  (output (: b"\xe5\x8e&" Bytes)))

(case "an unsigned LEB128 encoder emits a single byte below the continuation threshold"
  (doc    "The base case of the LEB128 encoder above: a value under 128 needs no continuation byte, so
           the encoder's `(< n 128)` arm emits exactly one byte and does not recurse. `(uleb 100)` =
           `b\"d\"` (byte 100 is ASCII `d`). Pins the terminator arm in isolation from the recursive
           multibyte path, so a regression in either arm is localized.")
  (input  (do
            (def (uleb n)
              (if (< n 128)
                  (Bytes.of (list (UInt8.wrap n)))
                  (Bytes.concat (Bytes.of (list (UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
            (def (main) (uleb 100)) (export main)))
  (output (: b"d" Bytes)))

(case "a recursive emitter dispatches on a sum's variants to build bytes per node"
  (doc    "The compiler's emit spine as a type-driven tree walk: a recursive `emit : Expr → Bytes`
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
  (input  (do
            (type Expr (Lit Int64) (Neg Expr) (Add (Tuple Expr Expr)))
            (def (emit e)
              (match e
                ((Expr.Lit n)           (Bytes.of (list 0x42)))
                ((Expr.Neg x)           (Bytes.concat (emit x) (Bytes.of (list 0x7C))))
                ((Expr.Add (tuple a b)) (Bytes.concat (emit a) (Bytes.concat (emit b) (Bytes.of (list 0x6A)))))))
            (def (main) (emit (Expr.Add (tuple (Expr.Lit 1) (Expr.Neg (Expr.Lit 2)))))) (export main)))
  (output (: b"BB|j" Bytes)))

(case "a recursive fold of a cons-list to bytes is the whole program result"
  (doc    "The compiler's SERIALIZE spine: fold a linked list of byte fragments into one byte vector by
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
  (input  (do
            (type BL BNil (BCons (Tuple Bytes BL)))
            (def (build n) (if (< n 1)
                               (BL.BNil ())
                               (BL.BCons (tuple (Bytes.of (list (UInt8.wrap (+ 64 n)))) (build (- n 1))))))
            (def (cat-all xs) (match xs
                                ((BL.BNil _)            (Bytes.of (list)))
                                ((BL.BCons (tuple h t)) (Bytes.concat h (cat-all t)))))
            (def (main) (cat-all (build 3))) (export main)))
  (output (: b"CBA" Bytes)))

; --- Slice and compact at RUNTIME: reading and re-basing byte fragments ---------------------
; Slicing and compacting a byte sequence carrying a runtime value are the input-side companions of the
; concat cases above: a compiler reading its input bytes takes sub-ranges (`Bytes.slice`) and, having
; kept a small piece of a large buffer, re-bases it to release the parent (`Bytes.compact`). These pin
; the fallible slice and the value-preserving compact on GENUINE runtime values (not compile-time
; literals), exercising the shared-storage representation directly: slice is fallible exactly as at
; compile time (Some in bounds, None past the end or below zero), and compact is the identity on value.

(case "slicing a byte sequence built at run time yields Some of the sub-range"
  (doc    "`(Bytes.slice b 1 2)` on a runtime-built `b` yields `(Some b\"\\x14\\x1e\")`: the runtime
           realizes the slice by sharing `b`'s storage (a view node over the parent leaf), which is
           indistinguishable from a fresh copy (memory-and-resource-model.md #Sharing Is Not Observable).
           Bytes 20, 30 are non-printable, so the byte-string display escapes them. Pins the fallible
           slice on a runtime value — how a compiler reads a sub-range of its input bytes without copying.")
  (input  (do
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 1 2)) (export main)))
  (output (: (Some b"\x14\x1e") (Option Bytes))))

(case "slicing a runtime byte sequence past the end yields None"
  (doc    "`(Bytes.slice b 2 3)` on a runtime-built 4-byte sequence asks for 3 bytes from index 2 —
           running one byte past the end — so it yields None, never reading beyond the sequence or
           returning a short result. The runtime companion of the const past-the-end case, pinning that
           the bound is checked on the value at run time.")
  (input  (do
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 2 3)) (export main)))
  (output (: (None unit) (Option Bytes))))

(case "slicing a runtime byte sequence with a negative start yields None"
  (doc    "`(Bytes.slice b -1 2)` uses a start below 0 at run time — no byte at position -1 — so it
           yields None, NOT a large unsigned offset from casting the negative start. The runtime
           companion of the const negative-start case: the check is on the signed value, so a runtime
           negative start is caught before it can wrap.")
  (input  (do
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 -1 2)) (export main)))
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

(case "slicing with start+len overflowing i64 is out of range, not a wrong slice"
  (doc    "`(Bytes.slice b start len)` with `start = 2^62` and `len = 2^62` on a 3-byte sequence: the range
           is astronomically out of bounds → None. A bounds check that computes `start + len` in wrapping
           i64 overflows to i64::MIN (negative), passes a signed `<= byte-count` test, and wrongly takes
           the in-range path — then i32-wraps 2^62 to 0 and returns an empty `Some` (a WRONG value). The
           predicate must be overflow-safe: a sum that would overflow is out of range. Expected None (-1).")
  (input  (do
            (def (main (: s Int64) (: l Int64))
              (match (Bytes.slice (Bytes.of (list 10 20 30)) s l)
                ((Some b) (Bytes.len b))
                ((None _) -1)))
            (export main)))
  (call   main (: 4611686018427387904 Int64) (: 4611686018427387904 Int64))
  (output (: -1 Int64)))

(case "slicing with a start near i64::MAX is out of range, not a trap"
  (doc    "The trap sibling: `start = i64::MAX`, `len = 1` on a 3-byte sequence — out of bounds → None. A
           wrapping-i64 bounds check computes `start + len = i64::MIN` (overflow), passes the signed `<=`,
           takes the in-range path, and i32-wraps i64::MAX to 0xFFFFFFFF — which the runtime `bytes-slice`
           reads as a 4-billion start and TRAPS. `Bytes.slice` PROMISES it never traps, so this is a
           soundness violation. Pins that an out-of-range start, however large, declines to None (-1).")
  (input  (do
            (def (main (: s Int64) (: l Int64))
              (match (Bytes.slice (Bytes.of (list 10 20 30)) s l)
                ((Some b) (Bytes.len b))
                ((None _) -1)))
            (export main)))
  (call   main (: 9223372036854775807 Int64) (: 1 Int64))
  (output (: -1 Int64)))

(case "the slice overflow guard holds on a chained slice-of-a-slice"
  (doc    "The overflow-safe bounds check lives in the ONE shared `Core::BytesSlice` emit, so it holds at
           EVERY call site — not only over a fresh `Bytes.of`. The outer `(Bytes.slice b 1 3)` yields a
           3-byte view `[20 30 40]`; slicing THAT view with `start = len = 2^62` must decline to None (a
           wrapping-i64 `start + len` would overflow the inner view's own length check identically and
           return an empty `Some`). Pins that the shared-emit fix covers a view's length feeding the same
           predicate — a slice-of-a-slice is guarded exactly as a slice-of-a-fresh-sequence. Expected None
           (-1); the outer slice is in range so the -2 arm is not taken.")
  (input  (do
            (def (main (: ss Int64) (: sl Int64))
              (match (Bytes.slice (Bytes.of (list 10 20 30 40 50)) 1 3)
                ((Some s1)
                  (match (Bytes.slice s1 ss sl)
                    ((Some s2) (Bytes.len s2))
                    ((None _) -1)))
                ((None _) -2)))
            (export main)))
  (call   main (: 4611686018427387904 Int64) (: 4611686018427387904 Int64))
  (output (: -1 Int64)))

(case "compacting a byte sequence built at run time preserves its bytes"
  (doc    "`(Bytes.compact b)` on a runtime-built `b` = `b`: compact re-bases the value into storage
           independent of any larger buffer it was sliced from (memory-and-resource-model.md #Retained
           Storage Is What A Value's Representation Holds Live), changing storage use but never the value. Pins
           that compact is value-preserving on a runtime value — how a compiler keeps a small slice of a
           large input while letting the input be reclaimed. `(mk 1)` = `b\"\\x01\\x02\\x03\"`.")
  (input  (do
            (def (mk n) (Bytes.compact (Bytes.of (list n 2 3))))
            (def (main) (mk 1)) (export main)))
  (output (: b"\x01\x02\x03" Bytes)))

; --- A runtime `Bytes.at` Option is MATCHED — the reader's core idiom -------------------------------
; The reader walks the input bytes with `(match (Bytes.at input i) ((Some b) …) (None …))` on every
; byte, so this must compile: matching a runtime `Bytes.at` result (an `Option<Int64>` — the byte
; boxed) and returning a scalar from each arm. The `Some` binder is the Int64 BYTE (not an opaque
; handle), so it unifies with a scalar `None` arm. These pin that consuming a runtime `Bytes.at`
; Option by `match` works exactly as consuming any other `Option<Int64>` — the last gate before a
; byte-walking reader (hence true `bytes → bytes` self-hosting).

(case "matching a runtime Bytes.at Option binds the byte in the Some arm"
  (doc    "`(match (Bytes.at b i) ((Some x) x) (None -1))` on a runtime byte sequence `b` at an
           in-bounds index returns the byte: `(at (Bytes.of (list 10 20 30)) 1)` is 20. The `Some`
           binder `x` is the Int64 byte (Bytes.at boxes a byte, and the match unboxes it to the scalar),
           so it unifies with the scalar `None` arm — the reader's per-byte dispatch. Pins that a runtime
           `Bytes.at` Option matches like any `Option<Int64>`.")
  (input  (do
            (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
            (def (main)   (at (Bytes.of (list 10 20 30)) 1)) (export main)))
  (output (: 20 Int64)))

(case "matching a runtime Bytes.at Option takes the None arm past the end"
  (doc    "The out-of-bounds companion: `(at (Bytes.of (list 10 20 30)) 9)` reads past the end, so the
           match takes the `None` arm and returns -1. Pins that both arms of a runtime `Bytes.at` match
           are reachable and unify — the terminating branch of the byte-walk.")
  (input  (do
            (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
            (def (main)   (at (Bytes.of (list 10 20 30)) 9)) (export main)))
  (output (: -1 Int64)))

(case "reading a byte from a sequence built with a RUNTIME element widens it to the Option payload"
  (doc    "`(Bytes.of (list n))` with `n : UInt8` a parameter builds a one-byte sequence from a RUNTIME
           value; `(Bytes.at … 0)` reads that byte back as `Some x`, `x = 5` for n = 5. The read must
           reconcile the STORED byte's width (UInt8 / i32) with the `Some` payload's width (Int64 / i64):
           the stored byte is zero-extended to the payload. Was INVALID WASM ('expected i64, found i32') —
           the `Bytes.at` fold used the raw UInt8 element occurrence as the Some(Int64) payload without
           widening (correct only for a CONSTANT element, whose core folds through the width; a runtime
           element must take the runtime read). A constant-element read (`(Bytes.at (Bytes.of (list 5))
           0)`) folds and was always fine; this pins the runtime-stored byte is widened on read.")
  (input  (do
            (def (main (: n UInt8)) (match (Bytes.at (Bytes.of (list n)) 0) ((Some x) x) ((None _) -1)))
            (export main)))
  (call   main (: 5 UInt8))
  (output (: 5 Int64)))

(case "a recursive byte walk sums a runtime sequence via Bytes.at and match"
  (doc    "The reader's shape: walk a runtime byte sequence from index 0, matching `(Bytes.at b i)` on
           each step — `Some` binds the byte and recurses with `i+1`, `None` (past the end) terminates
           with the accumulator. `(go (Bytes.of (list 10 20 30)) 0 0)` sums to 60. Pins that a recursive
           function driving over the input bytes by matching `Bytes.at` compiles and runs — the core
           `bytes → AST` loop a self-hosted front end is built on.")
  (input  (do
            (def (go b i acc)
              (match (Bytes.at b i)
                ((Some x) (go b (+ i 1) (+ acc x)))
                (None acc)))
            (def (main) (go (Bytes.of (list 10 20 30)) 0 0)) (export main)))
  (output (: 60 Int64)))

(case "a recursive byte fold calling two helpers emits valid wasm (disjoint scratch slots)"
  (doc    "A recursive `be` whose body composes a heap-`match` result (the inlined `byte-at`, which
           materializes an i32 Option handle in a scratch slot) with checked ARITHMETIC over another
           helper's result (`(* (byte-at b i) (place …))`, whose overflow guards use i64 scratch slots).
           The two must occupy DISJOINT scratch slots: reusing one wasm local at both an i32 handle and an
           i64 arith temp re-types it to two widths → an invalid module ('expected i64, found i32'). The
           annotated form pins the SCRATCH-SLOT discipline directly (the unannotated form additionally
           needs argument-position inference — a separate increment). `be(b\"\\x01\\x02\", 0, 2)` =
           byte[0]*place(1) + byte[1]*place(0) = 1*256 + 2*1 = 258.")
  (input  (do
            (def (byte-at (: b Bytes) i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (place k) (if (< k 1) 1 (* 256 (place (- k 1)))))
            (def (be (: b Bytes) i n)
              (if (< n 1) 0 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1)))))
            (def (main) (be (Bytes.of (list 1 2)) 0 2)) (export main)))
  (output (: 258 Int64)))

(case "a CBOR head decodes its major type and big-endian argument from the input bytes"
  (doc    "The compiler's INPUT-side decode spine — the dual of the LEB128 output encoder: reading a
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
  (input  (do
            (def (byte-at b i)  (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (major b i)    (>> (byte-at b i) 5))
            (def (info b i)     (& (byte-at b i) 31))
            (def (be b i n)     (if (< n 1) 0
                                 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1)))))
            (def (place k)      (if (< k 1) 1 (* 256 (place (- k 1)))))
            (def (arg b i)      (if (< (info b i) 24) (info b i) (be b (+ i 1) 2)))
            (def (main)         (tuple (major (Bytes.of (list 0x19 0x01 0x2C)) 0)
                                       (arg   (Bytes.of (list 0x19 0x01 0x2C)) 0))) (export main)))
  (output (: (tuple 0 300) (Tuple Int64 Int64))))

(case "a CBOR atom decodes each scalar major type to its value"
  (doc    "The reader's LEAF-atom decode, the third leg beside head-index dispatch and length-driven
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
  (input  (do
            (def (byte-at b i)    (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-major b i) (>> (byte-at b i) 5))
            (def (cbor-info b i)  (& (byte-at b i) 31))
            (def (cbor-arg b i)   (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (dec b i)
              (if (= (cbor-major b i) 1)
                  (- (- 0 1) (cbor-arg b i))
              (if (= (cbor-major b i) 7)
                  (if (= (cbor-arg b i) 21) 1 0)
                  (cbor-arg b i))))
            (def (main) (+ (dec (Bytes.of (list 0x29)) 0)
                        (+ (dec (Bytes.of (list 0xF5)) 0)
                           (dec (Bytes.of (list 0x0A)) 0)))) (export main)))
  (output (: 1 Int64)))

(case "a CBOR simple value that is not a known boolean is classified as not-a-boolean"
  (doc    "CBOR major type 7 (simple/float) holds MORE than the two booleans: `0xF4`=false (arg 20),
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
  (input  (do
            (def (classify-simple arg)
              (if (= arg 21) 1
              (if (= arg 20) 0
                             (- 0 1))))
            (def (main) (+ (classify-simple 20)
                        (+ (* 10 (classify-simple 21))
                           (* 100 (classify-simple 27))))) (export main)))
  (output (: -90 Int64)))

(case "resolving a head against a prelude symbol rejects a length-mismatched prefix"
  (doc    "The reader's NAME-resolution step (ast-encoding.md: a node names its kind by a prelude INDEX;
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
  (input  (do
            (def (byte-at b i)       (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-info b i)     (& (byte-at b i) 31))
            (def (cbor-arg b i)      (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
            (def (payload-off b i)   (+ i (cbor-head-len b i)))
            (def (entry-len b e)     (cbor-arg b e))
            (def (entry-byte b e j)  (byte-at b (+ (payload-off b e) j)))
            (def (lit-byte lit j)    (match (Bytes.at lit j) ((Some x) x) ((None _) 0)))
            (def (neq-go b e lit j n)
              (if (< j n)
                  (if (= (entry-byte b e j) (lit-byte lit j)) (neq-go b e lit (+ j 1) n) false)
                  true))
            (def (name-eq b e lit n) (if (= (entry-len b e) n) (neq-go b e lit 0 n) false))
            (def (main) (if (name-eq (Bytes.of (list 0x62 0x2B 0x2B)) 0 b"+" 1) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "a CBOR skip walks past a whole nested item to the next offset"
  (doc    "The reader's structural NAVIGATION primitive, the companion of the head-decode above: given
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
  (input  (do
            (def (byte-at b i)       (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-major b i)    (>> (byte-at b i) 5))
            (def (cbor-info b i)     (& (byte-at b i) 31))
            (def (cbor-arg b i)      (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
            (def (skip-elems b i k)  (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
            (def (cbor-skip b i)
              (if (= (cbor-major b i) 4)
                  (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
              (if (or (= (cbor-major b i) 3) (= (cbor-major b i) 2))
                  (+ (+ i (cbor-head-len b i)) (cbor-arg b i))
                  (+ i (cbor-head-len b i)))))
            (def (main) (cbor-skip (Bytes.of (list 0x82 0x82 0x01 0x02 0x03)) 0)) (export main)))
  (output (: 5 Int64)))

(case "a recursive reader decodes a CBOR application tree and evaluates it by head index"
  (doc    "The reader's spine assembled end-to-end: `ev` recursively decodes a canonical-AST application
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
  (input  (do
            (def (byte-at b i)       (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-major b i)    (>> (byte-at b i) 5))
            (def (cbor-info b i)     (& (byte-at b i) 31))
            (def (cbor-arg b i)      (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
            (def (skip-elems b i k)  (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
            (def (cbor-skip b i)
              (if (= (cbor-major b i) 4)
                  (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
                  (+ i (cbor-head-len b i))))
            (def (elem0 b i)      (+ i (cbor-head-len b i)))
            (def (child-off b i k) (skip-elems b (elem0 b i) k))
            (def (ev b i)
              (if (= (cbor-major b i) 4)
                  (if (= (cbor-arg b (elem0 b i)) 0)
                      (+ (ev b (child-off b i 1)) (ev b (child-off b i 2)))
                      (* (ev b (child-off b i 1)) (ev b (child-off b i 2))))
                  (cbor-arg b i)))
            (def (main) (ev (Bytes.of (list 0x83 0x00 0x01 0x83 0x01 0x02 0x0B)) 0)) (export main)))
  (output (: 23 Int64)))

(case "a CBOR reader walks a variable-length array using its decoded length as the element count"
  (doc    "The reader's structural-COUNT primitive, distinct from head-index dispatch: a CBOR array's
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
  (input  (do
            (def (byte-at b i)       (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-major b i)    (>> (byte-at b i) 5))
            (def (cbor-info b i)     (& (byte-at b i) 31))
            (def (cbor-arg b i)      (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
            (def (skip-elems b i k)  (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
            (def (cbor-skip b i)
              (if (= (cbor-major b i) 4)
                  (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
                  (+ i (cbor-head-len b i))))
            (def (elem0 b i)     (+ i (cbor-head-len b i)))
            (def (elem b i k)    (skip-elems b (elem0 b i) k))
            (def (sum-elems b i k n) (if (< k n) (+ (cbor-arg b (elem b i k)) (sum-elems b i (+ k 1) n)) 0))
            (def (sum-array b i) (sum-elems b i 0 (cbor-arg b i)))
            (def (main) (sum-array (Bytes.of (list 0x84 0x0A 0x14 0x18 0x1E 0x18 0x28)) 0)) (export main)))
  (output (: 100 Int64)))

(case "a CBOR skip steps over a tagged item to the value it wraps"
  (doc    "The reader's navigation over a CBOR TAG (major 6): a tag is its head followed by exactly one
           tagged data item, so skipping a tag skips its head then recursively skips the one item it
           wraps. This is the `39` bare-name marker a canonical-AST module uses to distinguish a symbol
           reference from a plain integer — encoded `d8 27 <idx>` (tag number 39 = `0x27`, given as a
           one-byte argument after the `0xd8` tag head). Against `d8 27 01` — tag 39 (a two-byte head)
           wrapping the uint `1` (one byte) — `cbor-skip` from offset 0 steps past the tag head and the
           wrapped uint, landing at offset 3. Pins the tag branch of the navigation primitive: without
           it a reader walking a module's def list would miscount offsets the moment it met a tagged
           name, reading the wrong element. Completes the item-kind coverage of `cbor-skip` (array /
           string / tag / scalar) the reader needs to traverse the whole canonical AST.")
  (input  (do
            (def (byte-at b i)       (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (cbor-major b i)    (>> (byte-at b i) 5))
            (def (cbor-info b i)     (& (byte-at b i) 31))
            (def (cbor-arg b i)      (if (< (cbor-info b i) 24) (cbor-info b i) (byte-at b (+ i 1))))
            (def (cbor-head-len b i) (if (< (cbor-info b i) 24) 1 2))
            (def (skip-elems b i k)  (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
            (def (cbor-skip b i)
              (if (= (cbor-major b i) 4)
                  (skip-elems b (+ i (cbor-head-len b i)) (cbor-arg b i))
              (if (or (= (cbor-major b i) 3) (= (cbor-major b i) 2))
                  (+ (+ i (cbor-head-len b i)) (cbor-arg b i))
              (if (= (cbor-major b i) 6)
                  (cbor-skip b (+ i (cbor-head-len b i)))
                  (+ i (cbor-head-len b i))))))
            (def (main) (cbor-skip (Bytes.of (list 0xD8 0x27 0x01)) 0)) (export main)))
  (output (: 3 Int64)))

; --- The `b"…"` literal reads to a byte sequence, and rendering round-trips -----------------------
; `b"…"` is reader sugar for `(Bytes.of (list …))`, so a byte-string literal and the explicit form
; denote ONE value: they are equal. These cases pin the reader equivalence in both directions
; (printable and escaped bytes) and the full round-trip — a byte sequence WRITTEN as `b"…"`,
; constructed, and rendered back yields the same `b"…"` text — so the display form and the input form
; are inverses. A generation that does not yet realize the `b"…"` reader sugar declines (todo), it
; does not miscompile.

(case "a byte-string literal equals the explicit byte sequence it desugars to"
  (doc    "`(= b\"ABC\" (Bytes.of (list 65 66 67)))` is true: `b\"ABC\"` reads to `(Bytes.of (list 65 66
           67))` (bytes 65 66 67 = ASCII `A B C`), so the literal and the explicit form are the same
           value (options/binary-syntax; the `#\"…\"`/`a.b` sugar pattern). Pins that the byte-string
           literal is reader sugar, not a distinct value form.")
  (input  (= b"ABC" (Bytes.of (list 65 66 67))))
  (output (: true Bool)))

(case "a byte-string literal with escapes equals its explicit byte sequence"
  (doc    "`(= b\"\\x89PNG\" (Bytes.of (list 137 80 78 71)))` is true: the `\\x89` hex escape is byte
           137 and `PNG` are the printable bytes 80 78 71, so the literal reads to the PNG magic
           prefix. Pins that `\\xNN` and printable-ASCII bytes read to the same values the explicit
           list names — the reader escape set is the inverse of the display escape set.")
  (input  (= b"\x89PNG" (Bytes.of (list 137 80 78 71))))
  (output (: true Bool)))

(case "an empty byte-string literal is the empty byte sequence"
  (doc    "`(= b\"\" (Bytes.of (list)))` is true: `b\"\"` reads to the zero-length byte sequence. Pins
           the degenerate literal, the byte-string spelling of `(Bytes.of (list))`.")
  (input  (= b"" (Bytes.of (list))))
  (output (: true Bool)))

(case "a byte sequence written as a literal renders back to the same literal"
  (doc    "The full round-trip: a byte sequence built at run time from a `b\"…\"` literal renders back
           to that same `b\"…\"` text. `b\"A\\nB\"` carries the printable `A`, a newline (the `\\n`
           special escape), and `B`; passing it through a runtime function and rendering the result
           yields `b\"A\\nB\"` — reading and displaying a byte sequence are inverses.")
  (input  (do
            (def (id b) b)
            (def (main) (id b"A\nB")) (export main)))
  (output (: b"A\nB" Bytes)))
