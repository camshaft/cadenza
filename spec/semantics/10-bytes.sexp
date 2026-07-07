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
  (needs  bytes)
  (input  (Bytes.of (list 1 2 3)))
  (output (: b"\x01\x02\x03" Bytes)))

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
; Storage Is What A Value's Representation Holds Live requires, letting a program drop a large parent
; while keeping a small slice. Compacting changes STORAGE USE, never the VALUE: the compacted slice is
; equal to the slice by its bytes in order (#Equality Is Structural), so `compact` is not observable
; through any value operation.

(case "compacting a slice preserves its bytes"
  (doc    "`(Bytes.compact (Option.expect (Bytes.slice b 1 2) …))` = the same in-bounds slice: compacting
           materializes the slice into independent storage, changing resource use but not the value.
           Pins that compact is value-preserving — equal by bytes in order to the un-compacted slice
           (memory-and-resource-model.md #Retained Storage Is What A Value's Representation Holds Live).")
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
  (needs  bytes)
  (input  (module m
            (def (mk n) (Bytes.of (list n 66 67)))
            (def (main)  (mk 65))))
  (output (: b"ABC" Bytes)))

(case "constructing a byte sequence with a runtime value out of range traps"
  (doc    "The runtime companion of the const `256`/`-1` out-of-range cases: when the byte value is a
           runtime parameter, the range check `0..=255` must still fire at run time — a byte outside the
           range has no defined result, so the program traps (core-semantics.md #Partial Operations Have
           A Defined Outcome) rather than truncating `256` to `0` via a wrapping `as u8`. Pins that the
           bound is enforced on the value, not only on a compile-time literal.")
  (needs  bytes)
  (input  (module m
            (def (mk n) (Bytes.of (list n)))
            (def (main)  (mk 256))))
  (trap   "byte value out of range"))

(case "the length of a runtime byte sequence is its byte count"
  (doc    "`Bytes.len` of a byte sequence carrying a runtime byte: the seed folds a runtime Bytes value
           to a SCALAR count via the runtime's bytes-len, the fold-to-scalar half of the idiom (like a
           recursive list sum). `(Bytes.len (Bytes.of (list n 2 3)))` = 3 for any `n`.")
  (needs  bytes)
  (input  (module m
            (def (sz n) (Bytes.len (Bytes.of (list n 2 3))))
            (def (main)  (sz 9))))
  (output (: 3 Int64)))

(case "concatenating byte sequences built at run time appends their bytes in order"
  (doc    "`Bytes.concat` of two runtime byte sequences yields a genuine runtime value with the appended
           bytes. The representation MAY defer the concatenation — sharing the operands' storage under a
           concatenation node rather than copying their bytes into a fresh buffer — as an unobservable
           optimization (memory-and-resource-model.md #Sharing Is Not Observable), which keeps this case
           green either way. `(Bytes.concat (Bytes.of (list a)) (Bytes.of (list b 9)))` = `b\"\\x07\\x08\\t\"`
           for `a=7 b=8` — bytes 7 (BEL), 8 (backspace), 9 (tab) render as escapes (9 is the `\\t` special
           escape). Pins runtime concatenation — how a compiler joins the byte fragments of its output.")
  (needs  bytes)
  (input  (module m
            (def (join a b) (Bytes.concat (Bytes.of (list a)) (Bytes.of (list b 9))))
            (def (main)      (join 7 8))))
  (output (: b"\x07\x08\t" Bytes)))

(case "a recursively-built byte sequence assembles its bytes at run time"
  (doc    "The genuine self-hosting idiom for output: a byte sequence whose LENGTH is decided at run
           time, built by recursion + concatenation, not a fixed literal spine. `rep` prepends the byte
           88 `n` times onto the empty sequence, so how many bytes exist is known only at run time.
           `(rep 4)` = `b\"XXXX\"` (byte 88 is the printable ASCII `X`). This is exactly the shape a
           self-hosted compiler uses to emit a component's wasm bytes — concatenating byte fragments in
           a recursion whose depth is driven by the program being compiled.")
  (needs  bytes)
  (input  (module m
            (def (rep n) (if (< n 1)
                            (Bytes.of (list))
                            (Bytes.concat (Bytes.of (list 88)) (rep (- n 1)))))
            (def (main)  (rep 4))))
  (output (: b"XXXX" Bytes)))

(case "an unsigned LEB128 encoder emits the known-answer multibyte encoding"
  (doc    "The compiler's byte-emitting SPINE as one known-answer case: the recursive unsigned-LEB128
           encoder that produces every section length, vector count, and u32 operand in a wasm module.
           It composes the primitives the numeric cases pin individually — `(< n 128)` (terminator
           test), `(& n 127)` (low 7 bits), `(| … 128)` (continuation bit), `(>> n 7)` (next group),
           `Int.to-byte`, and `Bytes.concat` — into a recursion whose depth is the number of output
           bytes. `(uleb 624485)` is the canonical multibyte value from the LEB128 spec: 624485 =
           0b10011_0001110_1100101, so the little-endian 7-bit groups are 0x65, 0x0E, 0x26, and with
           the continuation bit set on all but the last the bytes are `E5 8E 26` = `b\"\\xe5\\x8e&\"`
           (byte 0x26 is `&`). Pins that the whole encoder composes to the exact bytes wasm requires —
           a single-primitive slip (wrong mask, wrong shift, dropped continuation bit) changes the
           output, so this is a tighter check on the emit path than any primitive alone. The companion
           `(uleb 100)` (100 < 128) exits in one byte to `b\"d\"`, exercising the base case.")
  (needs  bytes)
  (input  (module m
            (def (uleb n)
              (if (< n 128)
                  (Bytes.of (list (Int.to-byte n)))
                  (Bytes.concat (Bytes.of (list (Int.to-byte (| (& n 127) 128)))) (uleb (>> n 7)))))
            (def (main) (uleb 624485))))
  (output (: b"\xe5\x8e&" Bytes)))

(case "an unsigned LEB128 encoder emits a single byte below the continuation threshold"
  (doc    "The base case of the LEB128 encoder above: a value under 128 needs no continuation byte, so
           the encoder's `(< n 128)` arm emits exactly one byte and does not recurse. `(uleb 100)` =
           `b\"d\"` (byte 100 is ASCII `d`). Pins the terminator arm in isolation from the recursive
           multibyte path, so a regression in either arm is localized.")
  (needs  bytes)
  (input  (module m
            (def (uleb n)
              (if (< n 128)
                  (Bytes.of (list (Int.to-byte n)))
                  (Bytes.concat (Bytes.of (list (Int.to-byte (| (& n 127) 128)))) (uleb (>> n 7)))))
            (def (main) (uleb 100))))
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
  (needs  bytes)
  (input  (module m
            (type Expr (Lit Int64 | Neg Expr | Add (Tuple Expr Expr)))
            (def (emit e)
              (match e
                ((Expr.Lit n)           (Bytes.of (list 0x42)))
                ((Expr.Neg x)           (Bytes.concat (emit x) (Bytes.of (list 0x7C))))
                ((Expr.Add (tuple a b)) (Bytes.concat (emit a) (Bytes.concat (emit b) (Bytes.of (list 0x6A)))))))
            (def (main) (emit (Expr.Add (tuple (Expr.Lit 1) (Expr.Neg (Expr.Lit 2))))))))
  (output (: b"BB|j" Bytes)))

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
  (needs  fallible-access)
  (input  (module m
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 1 2))))
  (output (: (Some b"\x14\x1e") (Option Bytes))))

(case "slicing a runtime byte sequence past the end yields None"
  (doc    "`(Bytes.slice b 2 3)` on a runtime-built 4-byte sequence asks for 3 bytes from index 2 —
           running one byte past the end — so it yields None, never reading beyond the sequence or
           returning a short result. The runtime companion of the const past-the-end case, pinning that
           the bound is checked on the value at run time.")
  (needs  fallible-access)
  (input  (module m
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 2 3))))
  (output (: (None unit) (Option Bytes))))

(case "slicing a runtime byte sequence with a negative start yields None"
  (doc    "`(Bytes.slice b -1 2)` uses a start below 0 at run time — no byte at position -1 — so it
           yields None, NOT a large unsigned offset from casting the negative start. The runtime
           companion of the const negative-start case: the check is on the signed value, so a runtime
           negative start is caught before it can wrap.")
  (needs  fallible-access)
  (input  (module m
            (def (sl b s n) (Bytes.slice (Bytes.of (list b 20 30 40)) s n))
            (def (main)     (sl 10 -1 2))))
  (output (: (None unit) (Option Bytes))))

(case "compacting a byte sequence built at run time preserves its bytes"
  (doc    "`(Bytes.compact b)` on a runtime-built `b` = `b`: compact re-bases the value into storage
           independent of any larger buffer it was sliced from (memory-and-resource-model.md #Retained
           Storage Is What A Value's Representation Holds Live), changing storage use but never the value. Pins
           that compact is value-preserving on a runtime value — how a compiler keeps a small slice of a
           large input while letting the input be reclaimed. `(mk 1)` = `b\"\\x01\\x02\\x03\"`.")
  (needs  bytes)
  (input  (module m
            (def (mk n) (Bytes.compact (Bytes.of (list n 2 3))))
            (def (main) (mk 1))))
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
  (needs  fallible-access)
  (input  (module m
            (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
            (def (main)   (at (Bytes.of (list 10 20 30)) 1))))
  (output (: 20 Int64)))

(case "matching a runtime Bytes.at Option takes the None arm past the end"
  (doc    "The out-of-bounds companion: `(at (Bytes.of (list 10 20 30)) 9)` reads past the end, so the
           match takes the `None` arm and returns -1. Pins that both arms of a runtime `Bytes.at` match
           are reachable and unify — the terminating branch of the byte-walk.")
  (needs  fallible-access)
  (input  (module m
            (def (at b i) (match (Bytes.at b i) ((Some x) x) (None -1)))
            (def (main)   (at (Bytes.of (list 10 20 30)) 9))))
  (output (: -1 Int64)))

(case "a recursive byte walk sums a runtime sequence via Bytes.at and match"
  (doc    "The reader's shape: walk a runtime byte sequence from index 0, matching `(Bytes.at b i)` on
           each step — `Some` binds the byte and recurses with `i+1`, `None` (past the end) terminates
           with the accumulator. `(go (Bytes.of (list 10 20 30)) 0 0)` sums to 60. Pins that a recursive
           function driving over the input bytes by matching `Bytes.at` compiles and runs — the core
           `bytes → AST` loop a self-hosted front end is built on.")
  (needs  fallible-access)
  (input  (module m
            (def (go b i acc)
              (match (Bytes.at b i)
                ((Some x) (go b (+ i 1) (+ acc x)))
                (None acc)))
            (def (main) (go (Bytes.of (list 10 20 30)) 0 0))))
  (output (: 60 Int64)))

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
  (needs  fallible-access)
  (input  (module m
            (def (byte-at b i)  (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
            (def (major b i)    (>> (byte-at b i) 5))
            (def (info b i)     (& (byte-at b i) 31))
            (def (be b i n)     (if (< n 1) 0
                                 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1)))))
            (def (place k)      (if (< k 1) 1 (* 256 (place (- k 1)))))
            (def (arg b i)      (if (< (info b i) 24) (info b i) (be b (+ i 1) 2)))
            (def (main)         (tuple (major (Bytes.of (list 0x19 0x01 0x2C)) 0)
                                       (arg   (Bytes.of (list 0x19 0x01 0x2C)) 0)))))
  (output (: (tuple 0 300) (Tuple Int64 Int64))))

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
  (needs  bytes)
  (input  (= b"ABC" (Bytes.of (list 65 66 67))))
  (output (: true Bool)))

(case "a byte-string literal with escapes equals its explicit byte sequence"
  (doc    "`(= b\"\\x89PNG\" (Bytes.of (list 137 80 78 71)))` is true: the `\\x89` hex escape is byte
           137 and `PNG` are the printable bytes 80 78 71, so the literal reads to the PNG magic
           prefix. Pins that `\\xNN` and printable-ASCII bytes read to the same values the explicit
           list names — the reader escape set is the inverse of the display escape set.")
  (needs  bytes)
  (input  (= b"\x89PNG" (Bytes.of (list 137 80 78 71))))
  (output (: true Bool)))

(case "an empty byte-string literal is the empty byte sequence"
  (doc    "`(= b\"\" (Bytes.of (list)))` is true: `b\"\"` reads to the zero-length byte sequence. Pins
           the degenerate literal, the byte-string spelling of `(Bytes.of (list))`.")
  (needs  bytes)
  (input  (= b"" (Bytes.of (list))))
  (output (: true Bool)))

(case "a byte sequence written as a literal renders back to the same literal"
  (doc    "The full round-trip: a byte sequence built at run time from a `b\"…\"` literal renders back
           to that same `b\"…\"` text. `b\"A\\nB\"` carries the printable `A`, a newline (the `\\n`
           special escape), and `B`; passing it through a runtime function and rendering the result
           yields `b\"A\\nB\"` — reading and displaying a byte sequence are inverses.")
  (needs  bytes)
  (input  (module m
            (def (id b) b)
            (def (main) (id b"A\nB"))))
  (output (: b"A\nB" Bytes)))
