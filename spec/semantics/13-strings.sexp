; String operations — witnesses collections-and-text.md. The compiler needs string operations
; for error messages, name encoding (export names → bytes in wasm), and instruction tag dispatch.
; String equality is already witnessed in 01-literals; these cover the remaining operations
; the compiler needs.

(case "string concatenation"
  (doc    "The compiler builds error messages and export names via string concatenation.")
  (input  (String.concat "hello" " world"))
  (output (: "hello world" String)))

(case "string length"
  (input  (String.scalar-len "hello"))
  (output (: 5 Int64)))

(case "scalar length counts Unicode scalar values, not bytes"
  (doc    "Witnesses collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length.
           \"café\" is four scalar values (c, a, f, é) but FIVE UTF-8 bytes — é encodes as
           two bytes. String.scalar-len is the scalar count (4), NOT the byte count (5). The byte count is
           what String.byte-len yields (the byte-len case below), and the two differ here
           precisely because the string is multi-byte. `String.scalar-len \"hello\"` above cannot witness
           this — ASCII makes the two counts coincide.")
  (input  (String.scalar-len "café"))
  (output (: 4 Int64)))

(case "scalar length of a supplementary-plane character is one scalar value"
  (doc    "Witnesses collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length
           at the boundary that most tempts a byte- or UTF-16-based miscount: \"😀\" (U+1F600)
           is a single Unicode scalar value — scalar length 1 — even though it is four UTF-8 bytes (and
           two UTF-16 code units). A length implementation counting bytes would report 4, UTF-16 units 2;
           the scalar count is 1.")
  (input  (String.scalar-len "😀"))
  (output (: 1 Int64)))

; --- Byte length is the UTF-8 byte count, obtained directly ------------------------------
; collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length: alongside the scalar
; length, a string offers its length in the bytes of its UTF-8 encoding as a SEPARATELY-NAMED op —
; String.byte-len — obtainable WITHOUT first materializing the bytes (it need not go through
; `String.to-bytes → Bytes.len`, though it MUST agree with that composition). There is no unqualified
; `String.len`: every length query names whether it counts scalars or bytes, so the café case that
; tempts the "which length?" confusion is a compile-time-explicit choice, not a silent default.

(case "byte length is the UTF-8 byte count"
  (doc    "`(String.byte-len \"café\")` is 5 — the number of bytes in the UTF-8 encoding (é is two
           bytes), NOT the scalar count 4 (String.scalar-len \"café\" = 4, above). Pins the byte length
           as a first-class, directly-obtained op distinct from the scalar length
           (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).")
  (input  (String.byte-len "café"))
  (output (: 5 Int64)))

(case "byte length agrees with the length of the encoded bytes"
  (doc    "`(String.byte-len s)` MUST equal `(Bytes.len (String.to-bytes s))` — the direct byte length
           agrees with materializing the UTF-8 bytes and counting them; only the cost differs. Pins the
           two paths as the same number, so byte-len is a cheap shortcut, not a second answer.")
  (input  (= (String.byte-len "café") (Bytes.len (String.to-bytes "café"))))
  (output (: true Bool)))

(case "byte length counts the normalized form, not the source spelling"
  (doc    "The decomposed \"café\" (c, a, f, e + U+0301 combining acute — SIX UTF-8 bytes as written)
           normalizes to NFC (c, a, f, é — FIVE bytes) before it is a String value, so its byte length
           is 5, the byte count of the NORMALIZED contents (collections-and-text.md #A String Offers
           Both A Scalar Length And A Byte Length, 2nd sentence). Pins that byte-len is a function of the
           string's value, not of the incidental byte spelling normalization removes — the byte-length
           companion of the scalar-length-after-normalization case below.")
  (input  (String.byte-len "café"))
  (output (: 5 Int64)))

(case "string to bytes (UTF-8)"
  (doc    "The compiler encodes export names as UTF-8 bytes for wasm sections. String.to-bytes
           produces the UTF-8 byte sequence of the string.")
  (input  (Bytes.len (String.to-bytes "run")))
  (output (: 3 Int64)))

(case "string to bytes encodes multi-byte characters"
  (doc    "UTF-8 encodes non-ASCII characters as multiple bytes.")
  (input  (Bytes.len (String.to-bytes "café")))
  (output (: 5 Int64)))

(case "string equality"
  (input  (= "hello" "hello"))
  (output (: true Bool)))

(case "string inequality"
  (input  (= "hello" "world"))
  (output (: false Bool)))

; --- The empty string is an ordinary String value ---------------------------------------
; `""` is the zero-length string — a first-class String the compiler needs (an empty error message, an
; empty name). Its length is 0 (counted in Unicode scalar values, of which it has none), it is equal
; only to another empty string, and it is the identity element of concatenation on both sides. The
; non-empty string cases above cannot witness the degenerate boundary where a length computation
; underflows or a concat assumes a non-empty operand; these pin it, the String companion of the
; empty-byte-sequence cluster (10-bytes.sexp) and the empty-map/empty-list cases.

(case "the empty string has length zero"
  (doc    "`(String.scalar-len \"\")` is 0 — the empty string has no Unicode scalar values
           (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length). Pins
           that length handles the zero-length string, not underflowing or reading a phantom scalar.")
  (input  (String.scalar-len ""))
  (output (: 0 Int64)))

(case "two empty strings are equal"
  (doc    "`(= \"\" \"\")` is true: two empty strings have identical (empty) normalized contents, so
           they are equal (collections-and-text.md #String Equality Follows Normalized Contents). Pins
           that string equality treats the empty string as a genuine value equal to itself.")
  (input  (= "" ""))
  (output (: true Bool)))

(case "an empty string is unequal to a non-empty string"
  (doc    "`(= \"\" \"x\")` is false — the empty string and a one-character string have different
           contents. Pins that emptiness on one side is an ordinary inequality, not a special case.")
  (input  (= "" "x"))
  (output (: false Bool)))

(case "concatenating an empty string on the right is the identity"
  (doc    "`(String.concat \"hi\" \"\")` = \"hi\": appending the empty string changes nothing. Pins the
           right identity of String.concat — a concat that mishandles a zero-length operand would break
           the compiler's error-message and name assembly.")
  (input  (= (String.concat "hi" "") "hi"))
  (output (: true Bool)))

(case "concatenating an empty string on the left is the identity"
  (doc    "The left-identity companion: `(String.concat \"\" \"hi\")` = \"hi\". Pins that concatenation
           handles a zero-length LEFT operand too, mirroring the empty-byte-sequence concat cases.")
  (input  (= (String.concat "" "hi") "hi"))
  (output (: true Bool)))

(case "an empty-range slice of a non-empty string is Some of the empty string"
  (doc    "`(String.slice \"hello\" 2 2)` has start = end, so it selects no scalar values — Some of the
           empty string (the in-bounds degenerate companion of the empty-range slice at index 0 already
           witnessed, here at an interior index). A slice whose start equals its end is present and
           empty, not None: the range [2,2) is valid and empty. The unwrapped slice MUST equal \"\".")
  (needs  fallible-access)
  (input  (= (Option.expect (String.slice "hello" 2 2) "slice is in bounds") ""))
  (output (: true Bool)))

; --- String equality and length follow Unicode normalization -------------------------------
; collections-and-text.md #String Equality Follows Normalized Contents: "Two strings MUST be equal
; exactly when their NORMALIZED contents are identical, under the text normalization the hashing-
; and-encoding choice pins." And a string literal is stored in that canonical normal form
; (01-literals "a string literal is normalized to the canonical text form"). So two strings that
; differ ONLY in Unicode normalization — "café" with a composed é (U+00E9) vs "café" with a
; decomposed e + combining acute (U+0301) — denote ONE value: they are equal, and both have length 4
; (four scalar values after normalization). The seed stores the raw scalars without normalizing, so
; it compares them unequal and counts the decomposed form as 5 scalar values.

(case "strings differing only in Unicode normalization are equal"
  (doc    "The composed \"café\" (…, U+00E9) and the decomposed \"café\" (…, e + U+0301
           combining acute) are the same text under the pinned normalization, so they MUST be equal
           (collections-and-text.md #String Equality Follows Normalized Contents). The seed compares
           the un-normalized scalar sequences and wrongly answers false.")
  (input  (= "café" "café"))
  (output (: true Bool)))

(case "string length counts scalar values after normalization"
  (doc    "The length of the decomposed \"café\" MUST be 4 — after normalization it is the four
           scalar values c, a, f, é, the same as the composed form (String.scalar-len \"café\" = 4,
           witnessed above). The seed counts the un-normalized e + combining acute as 5 scalar values.")
  (input  (String.scalar-len "café"))
  (output (: 4 Int64)))

(case "string indexing returns Some of the character"
  (doc    "Witnesses fallible String indexing (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping): an in-bounds scalar index yields the one-scalar string wrapped in Some.")
  (needs  fallible-access)
  (input  (String.at "hello" 1))
  (output (: (Some "e") (Option String))))

; --- String indexing is by Unicode scalar value, not by byte -----------------------------
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values ("its contents are
; independent of any byte encoding") + #A String Offers Both A Scalar Length And A Byte Length:
; String.at addresses the string by SCALAR position, not byte offset. The ASCII `(String.at "hello"
; 1)` above cannot witness this — for ASCII the scalar index and byte offset coincide. A multi-byte
; string distinguishes them: in "café" (scalars c,a,f,é; é is 2 UTF-8 bytes) scalar index 3 is "é",
; whereas byte offset 3 lands in the MIDDLE of é's two-byte encoding (an invalid scalar boundary). A
; byte-indexing lowering would slice a partial code unit or trap; the scalar semantics must return "é".

(case "string indexing addresses Unicode scalar values, not bytes"
  (doc    "`(String.at \"café\" 3)` = \"é\": the string is four scalar values (c, a, f, é) and
           index 3 is the last, é — even though é occupies bytes 3–4 of the five-byte UTF-8 encoding,
           so byte offset 3 would be a partial code unit. Pins that String.at indexes by scalar value
           (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), the companion
           of String.scalar-len counting scalars; the ASCII `(String.at \"hello\" 1)` cannot distinguish
           scalar index from byte offset.")
  (needs  fallible-access)
  (input  (String.at "café" 3))
  (output (: (Some "é") (Option String))))

(case "string indexing past a supplementary-plane scalar lands on the next scalar"
  (doc    "`(String.at \"😀b\" 1)` = \"b\": 😀 (U+1F600) is ONE scalar value occupying four UTF-8
           bytes (and two UTF-16 code units), so scalar index 1 is the character AFTER it, \"b\". A
           byte- or UTF-16-based index would land inside 😀's encoding. Pins scalar-value addressing
           at the boundary that most tempts a byte/UTF-16 miscount (the indexing companion of the
           supplementary-plane length case).")
  (needs  fallible-access)
  (input  (String.at "😀b" 1))
  (output (: (Some "b") (Option String))))

; --- String.at and String.slice are fallible: an out-of-range index yields None -------------
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values gives a string a defined
; scalar length, and #Indexing And Lookup Are Fallible, Not Trapping requires an out-of-range read to
; yield None rather than trap or produce an unspecified value. So String.at at a scalar index at or
; beyond the length, or at a NEGATIVE index, has no character to return and MUST yield None — exactly
; as List.at / Bytes.at do out of bounds (05-compound-types, 10-bytes). A negative index is the classic
; miscompile: a lowering that casts the index to an unsigned width turns -1 into a huge in-range-looking
; offset; the scalar-index bounds check must catch it as out of range and yield None.

(case "string indexing at or beyond the length yields None"
  (doc    "`(String.at \"hi\" 5)` indexes scalar position 5 of a two-scalar string — out of range, no
           character to return — so it MUST yield None (collections-and-text.md #Indexing And Lookup Are
           Fallible, Not Trapping), the String companion of the List.at / Bytes.at out-of-bounds Nones.")
  (needs  fallible-access)
  (input  (String.at "hi" 5))
  (output (: (None unit) (Option String))))

(case "a negative string index yields None rather than wrapping to a large offset"
  (doc    "`(String.at \"hi\" -1)` uses a negative scalar index — no defined character — so it MUST
           yield None, NOT wrap. A lowering that casts the index to an unsigned integer would turn -1
           into a huge positive offset (either reading out of bounds or, worse, an unspecified in-range
           byte); fallible indexing requires None (collections-and-text.md #Indexing And Lookup Are
           Fallible, Not Trapping). The negative-index companion of the out-of-range case above.")
  (needs  fallible-access)
  (input  (String.at "hi" -1))
  (output (: (None unit) (Option String))))

(case "string slicing yields Some of the substring"
  (doc    "Witnesses fallible String slicing: an in-bounds range yields the substring wrapped in Some
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping). This case reads
           the Option directly, without unwrapping, to pin the Some.")
  (needs  fallible-access)
  (input  (String.slice "hello world" 0 5))
  (output (: (Some "hello") (Option String))))

; --- String.slice bounds are checked: reversed, out-of-range, and negative bounds yield None; ----
; --- an empty in-range slice is Some of the empty string ------------------------------------
; String.slice takes a start and an end scalar index. A well-defined slice needs 0 ≤ start ≤ end ≤
; length; any bounds outside that have no defined substring and MUST yield None (collections-and-text.md
; #Indexing And Lookup Are Fallible, Not Trapping), while a degenerate but in-range slice where start =
; end is Some of the empty string — present, not None. These pin the boundary the encoder relies on when
; it slices instruction/name substrings: a reversed or over-long range is None, an empty in-range slice
; is Some "".

(case "a slice whose end is beyond the string length yields None"
  (doc    "`(String.slice \"hi\" 0 5)` asks for scalars 0..5 of a two-scalar string — the end 5 is
           beyond the length — so the slice has no defined substring and MUST yield None
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping).")
  (needs  fallible-access)
  (input  (String.slice "hi" 0 5))
  (output (: (None unit) (Option String))))

(case "a slice whose end precedes its start yields None"
  (doc    "`(String.slice \"hello\" 3 1)` has end 1 before start 3 — a reversed range with no defined
           substring — so it MUST yield None rather than return an empty or reversed string. Pins that
           the start ≤ end constraint is checked, not silently normalized.")
  (needs  fallible-access)
  (input  (String.slice "hello" 3 1))
  (output (: (None unit) (Option String))))

(case "a slice with a negative start yields None"
  (doc    "`(String.slice \"hello\" -1 3)` has a negative start index — outside 0..length — so it MUST
           yield None, not wrap a negative bound to a large unsigned offset (the same negative-index
           miscompile the String.at case guards). Pins that both slice bounds are range-checked as
           signed values.")
  (needs  fallible-access)
  (input  (String.slice "hello" -1 3))
  (output (: (None unit) (Option String))))

(case "a slice whose start equals its end is Some of the empty string"
  (doc    "`(String.slice \"hello\" 2 2)` is a degenerate but in-range slice (0 ≤ 2 ≤ 2 ≤ 5): it
           selects zero scalars, so it is Some of the empty string \"\" — present, NOT None. Pins that
           the bounds check admits start = end (an empty result) rather than rejecting it, the boundary
           just inside the reversed-range None above.")
  (needs  fallible-access)
  (input  (String.slice "hello" 2 2))
  (output (: (Some "") (Option String))))

; --- String.slice over a RUNTIME string (a parameter, not a literal) ------------------------
; The slice cases above feed a string LITERAL, so they const-fold before the runtime emitter is
; reached. A string operation's argument may be a runtime value — a parameter, a match/if-selected
; string — which does NOT fold: the seed must emit a runtime slice that walks the Bytes-backed UTF-8
; leaf to map SCALAR offsets to byte offsets (String offsets are scalar positions, not bytes, unlike
; Bytes.slice's byte length). These pin that the runtime slice agrees with the folded one on value,
; boundary handling, AND scalar-vs-byte indexing — the reader idiom a self-hosting compiler needs.

(case "a runtime string is sliced by scalar offsets"
  (doc    "`(String.slice s 1 4)` on a string PARAMETER `s = \"hello\"` yields Some \"ell\" — scalars
           1..4. Feeding `s` as a parameter defeats const-folding, so this exercises the runtime UTF-8
           slice walk, which must agree with the folded literal cases above.")
  (needs  fallible-access)
  (input  (module m
            (def (f s) (Option.expect (String.slice s 1 4) "in range"))
            (def (main) (f "hello"))))
  (output (: "ell" String)))

(case "a runtime string slice addresses scalar values, not bytes"
  (doc    "`(String.slice s 1 3)` on `s = \"aébc\"` yields Some \"éb\" — scalars 1 and 2 (é is one
           scalar, TWO UTF-8 bytes). A runtime slice that indexed by BYTE offset would split é or read
           the wrong range; pins that the runtime walk maps scalar offsets to byte offsets, exactly as
           String.at does (13-strings §reading a string's scalar addresses scalar values, not bytes).")
  (needs  fallible-access)
  (input  (module m
            (def (f s) (Option.expect (String.slice s 1 3) "in range"))
            (def (main) (f "aébc"))))
  (output (: "éb" String)))

(case "a runtime string slice out of range yields None"
  (doc    "`(String.slice s 0 5)` on the two-scalar `s = \"hi\"` has end 5 past the length, so it yields
           None — the runtime bounds check agrees with the folded out-of-range case. The match takes the
           None arm (-1), witnessing the absent result rather than a trap or a short string.")
  (needs  fallible-access)
  (input  (module m
            (def (f s) (match (String.slice s 0 5) ((Some x) (String.byte-len x)) ((None _) -1)))
            (def (main) (f "hi"))))
  (output (: -1 Int64)))

(case "a runtime string slice with an empty in-range span is Some of the empty string"
  (doc    "`(String.slice s 2 2)` on a runtime `s = \"hello\"` selects zero scalars — Some \"\", present
           not None (the empty-span boundary, on the runtime path). `String.byte-len` of the result is
           0, distinguishing Some \"\" (0) from None (which the match would send elsewhere).")
  (needs  fallible-access)
  (input  (module m
            (def (f s) (match (String.slice s 2 2) ((Some x) (String.byte-len x)) ((None _) -1)))
            (def (main) (f "hello"))))
  (output (: 0 Int64)))

; --- A string operation consumes a string SELECTED by runtime control flow -----------------
; A string operation (String.scalar-len/at/slice/concat) takes a string ARGUMENT; that argument may be a
; string SELECTED at run time by an `if` or a `match`, not only a literal. The seed resolves a string
; operation's argument through a runtime `if` — `(String.scalar-len (if b "hello" "hi"))` computes the
; selected string's length — but NOT through a runtime `match`: `(String.scalar-len (match n (0 "zero") (_
; "other")))` declines. Both `if` and `match` select a string value the same way, so a string
; operation must consume either. (This is about CONSUMING a runtime-selected string in an operation,
; distinct from RETURNING one across the run boundary — the latter needs the compound-value output ABI.)

(case "String.scalar-len of a string selected by a runtime match"
  (doc    "`(match n (0 \"zero\") (_ \"other\"))` selects a string by the runtime value n; `String.scalar-len`
           of that selected string is its scalar length — with n=5 the wildcard picks \"other\", length
           5. A string operation must consume a match-selected string exactly as it consumes an
           if-selected one (the control below, which the seed runs). The seed declines the match case
           (\"unsupported dotted-application\") — its string-op argument resolution follows a runtime
           `if` but not a runtime `match`.")
  (input   (module m
             (def (f n) (String.scalar-len (match n (0 "zero") (_ "other"))))
             (def (main) (f 5))))
  (output  (: 5 Int64)))

(case "String.scalar-len of a string selected by a runtime if"
  (doc    "The control the case above must match: `String.scalar-len` of a string chosen by a runtime `if`
           computes the selected string's length — `(if b \"hello\" \"hi\")` with b=true is \"hello\",
           length 5. The seed runs this; the match companion must behave identically.")
  (input   (module m
             (def (f b) (String.scalar-len (if b "hello" "hi")))
             (def (main) (f true))))
  (output  (: 5 Int64)))

; --- A string flows as a genuine RUNTIME value: a fn parameter, a return, a sum payload -----------
; The cases above CONSUME a string in an operation whose result is a scalar (a length), so the string
; itself never crosses a function boundary as a value. These pin the string flowing as a first-class
; runtime value — passed to a function, returned from one, carried in a sum variant, compared for
; equality, concatenated — the operations a program that dispatches on names and threads a symbol
; table requires (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values; the string
; analogue of the "Bytes as a RUNTIME value" cases in 10-bytes.sexp). A string literal reaching one of
; these positions is a runtime value (a UTF-8 leaf), not only a compile-time constant; the compiler
; materializes it on the value heap. `main` here yields a scalar so the observable stays a plain value.

(case "a string passed to a function as a runtime argument is measured by its byte length"
  (doc    "`(len2 \"hello\")` passes the string literal as a genuine runtime argument to `len2`, whose
           body takes its byte length. `String.byte-len` of a runtime string parameter is 5 — the UTF-8
           byte count. Pins that a string flows across a function boundary as a first-class value, not
           only as a folded constant (the front end passes a form's head string to a classifier this way).")
  (input   (module m
             (def (len2 s) (String.byte-len s))
             (def (main)   (len2 "hello"))))
  (output  (: 5 Int64)))

(case "a runtime string equality selects a branch by comparing a parameter to a literal"
  (doc    "`(if (= s \"def\") 1 0)` compares a runtime string parameter `s` against a literal — the
           name-dispatch primitive a compiler uses to recognize a form's head. Equality is structural
           over the UTF-8 bytes: `(pick \"def\")` is 1 (bytes match), `(pick \"x\")` is 0 (length
           differs). Their sum is 1. Pins runtime string `=` as a byte comparison, not a handle identity.")
  (input   (module m
             (def (pick s) (if (= s "def") 1 0))
             (def (main)   (+ (pick "def") (pick "x")))))
  (output  (: 1 Int64)))

(case "a multi-way string-head dispatch resolves an operator name to its operation"
  (doc    "The compiler's front-rung idiom end-to-end: a form's head is a STRING, and the resolver maps
           it to an operation by a chain of runtime string comparisons — `(= h \"+\")`, `(= h \"-\")`,
           `(= h \"*\")` — selecting the arithmetic to perform on the operands. This is the concrete
           `head-prim`/`resolve` shape a name-based front end takes: the reader hands the compiler a head
           NAME, and resolution turns that name into a code (here, directly into the operation) before
           anything downstream runs — the 'resolve names to codes before selecting instructions' step
           (compiler-pipeline.md §Representation) made concrete over runtime strings. `(eval-head \"+\"
           20 22)` resolves `\"+\"` and computes 42. Unlike the single-comparison case above, this pins a
           MULTI-way dispatch — several head names, each selecting a distinct operation — which is what a
           real head resolver is; the falls-through default (an unrecognized head) yields 0 here, the
           value-level stand-in for the front end's decline on an unknown head.")
  (input   (module m
             (def (eval-head h a b)
               (if (= h "+") (+ a b)
               (if (= h "-") (- a b)
               (if (= h "*") (* a b) 0))))
             (def (main) (eval-head "+" 20 22))))
  (output  (: 42 Int64)))

(case "a string carried as a sum-variant payload is bound and measured at run time"
  (doc    "A `Node` variant carries a String payload built at run time; a `match` binds it and takes its
           byte length. `(weigh (Node.NSym \"hello\"))` is 5 (the bound string's byte length) and
           `(weigh (Node.NInt 3))` is 3, summing to 8. Pins a string as a runtime sum payload — the
           shape of a symbol-carrying AST node the compiler walks — bound by a match arm and consumed.")
  (input   (module m
             (type Node (NInt Int64 | NSym String))
             (def (weigh n) (match n ((Node.NInt i) i) ((Node.NSym s) (String.byte-len s))))
             (def (main)    (+ (weigh (Node.NSym "hello")) (weigh (Node.NInt 3))))))
  (output  (: 8 Int64)))

(case "concatenating two runtime strings and measuring the result"
  (doc    "`(String.concat a b)` of two runtime string parameters yields a runtime string whose byte
           length is the sum of the operands' — `(join \"foo\" \"bar\")` is \"foobar\", byte length 6.
           Pins runtime string concatenation (how a compiler assembles a name or a diagnostic from
           fragments), agreeing with `(+ (byte-len a) (byte-len b))` when neither operand is empty.")
  (input   (module m
             (def (join a b) (String.byte-len (String.concat a b)))
             (def (main)     (join "foo" "bar"))))
  (output  (: 6 Int64)))

(case "a tail-recursive string accumulator builds a runtime string and its length is measured"
  (doc    "A self-recursive function threads a runtime STRING accumulator — `(rep s n)` returns the
           accumulator `s` in its base arm and recurses with `(String.concat s \"x\")` — the string
           analogue of a threaded compound accumulator (a compiler builds a name or a rendered form
           this way, appending fragment by fragment). `(String.byte-len (rep \"\" 3))` is 3: three
           appends of a one-byte \"x\". Pins that the accumulator PARAMETER and the function's RETURN
           both converge to the runtime-string (heap) kind even though the base arm returns the
           parameter bare and the recursive arm is a bare self-call — neither branch of the `if`
           independently reports a heap kind on the first inference pass. Without unifying the two
           branches to the heap kind, the recursive `if` is rejected `if branches differ in kind`
           (the then-branch `s` is heap, the else self-call defaulted to Int64); the same
           accumulator-return-kind convergence a heap-list or Bytes accumulator needs.")
  (input   (module m
             (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
             (def (main)    (String.byte-len (rep "" 3)))))
  (output  (: 3 Int64)))

(case "the byte length of a runtime string equals the length of its encoded bytes"
  (doc    "The runtime companion of the const `byte-len`/`to-bytes` agreement: for a runtime string `s`,
           `(String.byte-len s)` MUST equal `(Bytes.len (String.to-bytes s))` — the direct byte count and
           the encode-then-measure path are one number. `\"café\"` has byte length 5 (é is two bytes), so
           this is true. Pins that a runtime String IS its UTF-8 bytes (String.to-bytes is the identity on
           the underlying representation), the invariant the Bytes-backed String realization rests on.")
  (input   (module m
             (def (agree s) (= (String.byte-len s) (Bytes.len (String.to-bytes s))))
             (def (main)    (agree "café"))))
  (output  (: true Bool)))

(case "the scalar length of a runtime multi-byte string counts scalars, not bytes"
  (doc    "`String.scalar-len` of a runtime string counts Unicode scalar values, not UTF-8 bytes:
           `(slen \"café\")` is 4 (c, a, f, é) even though the byte length is 5. Pins that scalar length
           on a runtime value agrees with the const `chars().count()` — the runtime counts the UTF-8
           leading bytes (those not of the form 10xxxxxx), which for well-formed UTF-8 is the scalar
           count (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).")
  (input   (module m
             (def (slen s) (String.scalar-len s))
             (def (main)   (slen "café"))))
  (output  (: 4 Int64)))

(case "a runtime string is indexed by scalar and the extracted scalar is returned"
  (doc    "`String.at` on a RUNTIME string — the reader's scalar cursor — reads the i-th Unicode scalar
           as a one-scalar string, fallibly. `(at (concat s \"\") 3)` on \"café\" reads scalar 3 (é,
           which occupies UTF-8 bytes 3–4), returning `(Some \"é\")` — indexed by SCALAR, not byte, so
           the multi-byte é comes back whole. Pins runtime `String.at`: the seed walks the UTF-8 buffer
           to the scalar's byte span and slices it (a String is a Bytes-backed leaf), matching the const
           `chars().nth`. The concat forces a runtime value (a bare literal would const-fold).")
  (needs   fallible-access)
  (input   (module m
             (def (at s i) (String.at (String.concat s "") i))
             (def (main)   (at "café" 3))))
  (output  (: (Some "é") (Option String))))

(case "indexing a runtime string past its last scalar yields None"
  (doc    "The fallible companion: `String.at` at a scalar index at or beyond the string's scalar length
           yields `(None unit)`, never a trap (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping). `(at \"hi\" 5)` on the two-scalar \"hi\" is out of range → None. Pins that a
           runtime `String.at` out-of-bounds is a handled absence — the branch a reader takes at
           end-of-input.")
  (needs   fallible-access)
  (input   (module m
             (def (at s i) (String.at (String.concat s "") i))
             (def (main)   (match (at "hi" 5) ((Some c) (String.byte-len c)) ((None _) -1)))))
  (output  (: -1 Int64)))

(case "a runtime string returned across the run boundary renders as its quoted text"
  (doc    "A string BUILT at run time and returned as `main`'s value crosses the boundary as its proper
           String type and renders as the quoted canonical text — `(join \"hel\" \"lo\")` is
           \"hello\". This exercises the compiler-emitted, type-directed string renderer (the analogue
           of the `b\"…\"` Bytes renderer): a runtime String is walked byte-by-byte and quoted/escaped,
           byte-identical to the const `\"…\"` form. Pins RETURNING a runtime string (distinct from the
           cases above, which consume one to a scalar) — the compound-value output ABI for strings.")
  (input   (module m
             (def (join a b) (String.concat a b))
             (def (main)     (join "hel" "lo"))))
  (output  (: "hello" String)))

(case "a returned runtime string with a multi-byte scalar renders the scalar verbatim"
  (doc    "The rendered form of a runtime string passes a printable Unicode scalar through verbatim, not
           as an escape — `(id \"café\")` renders \"café\" (the é is its raw two-byte UTF-8, matching the
           const path's `{:?}`, which prints printable Unicode literally). Pins that the emitted string
           renderer's escaping agrees with the const renderer on multi-byte scalars, so a rendered string
           reads back to the same value.")
  (input   (module m
             (def (id s) (if true s s))
             (def (main) (id "café"))))
  (output  (: "café" String)))

(case "a returned runtime string with a non-printable scalar renders it verbatim, not a u-escape"
  (doc    "A non-printable Unicode scalar (U+0007 BEL) renders VERBATIM as its raw byte, NOT as a
           u-escape: the reader recognizes exactly the closed escape set \\n \\t \\r \\\\ \\\" 
           (collections-and-text.md #A String Literal's Escapes Are A Closed Set), which has NO numeric
           escape, so a u-escape would read back as its literal characters rather than the BEL — the
           rendered string would NOT read back to the same value. (String.from-bytes (Bytes.of (list 97
           7 98))) is the 3-scalar string a-BEL-b; it renders with the BEL byte raw. Pins the round-trip
           the value-oracle gate now checks independently (re-reading the rendered text): a rendered
           string MUST read back to the same value, so the renderer emits ONLY the closed escapes.")
  (input   (module m
             (def (main) (Option.expect (String.from-bytes (Bytes.of (list 97 7 98))) "well-formed"))))
  (output  (: "ab" String)))

; --- Decoding bytes to a string is TOTAL, never trapping ---------------------------------------
; collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping. Cadenza's String is
; guaranteed-well-formed (a sequence of Unicode scalar values, never invalid UTF-8), so turning a Bytes
; into a String is a CHECKED, PARTIAL operation — some byte sequences are not well-formed UTF-8. The
; language's rule is that this partiality is HANDLED, not trapped: `String.from-bytes` yields an
; `Option<String>` (`(Some s)` for well-formed input, `None` for ill-formed), and the `bin`-pattern
; `(utf8 s)` segment (options/binary-syntax/) is a NON-MATCH on ill-formed bytes so a match's
; exhaustiveness obligation (CDZ0210) forces a branch that handles the bad case. This is the principled
; alternative to a trapping decode: where a type can make the partiality explicit and force it to be
; handled, the language prefers that over a trap (it REFINES total-or-trap — a trap is what remains for
; partialities a type cannot surface, not the first resort). Tagged `(needs binary-matching)`: the
; `from-bytes`/`utf8`-segment decode is realized with the `bin` form (options/realized-capability-set/).

(case "decoding well-formed UTF-8 bytes yields the string"
  (doc    "`(String.from-bytes (Bytes.of (list 99 97 102 195 169)))` decodes the UTF-8 bytes of \"café\"
           (c a f, then é as the two bytes 0xC3 0xA9 = 195 169) to `(Some \"café\")`. Pins that a
           well-formed byte sequence decodes to `(Some s)` — the success arm of the total decode
           (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping).")
  (needs  binary-matching)
  (input  (= (String.from-bytes (Bytes.of (list 99 97 102 195 169))) (Some "café")))
  (output (: true Bool)))

(case "decoding ill-formed UTF-8 bytes yields none, not a trap"
  (doc    "`(String.from-bytes (Bytes.of (list 255)))` is given 0xFF, which is not a well-formed UTF-8
           sequence (0xFF never appears in valid UTF-8), so the decode yields `None` — NOT a trap and NOT
           an unspecified string with a replacement character. Pins the failure arm as an ordinary value
           the program handles (collections-and-text.md #Decoding Bytes To A String Is Total, Not
           Trapping). This is the whole point of the total decode: ill-formed input is data, not a halt.")
  (needs  binary-matching)
  (input  (= (String.from-bytes (Bytes.of (list 255))) None))
  (output (: true Bool)))

(case "decoding an overlong UTF-8 encoding yields none"
  (doc    "`(Bytes.of (list 192 128))` is `C0 80` — the OVERLONG two-byte encoding of U+0000, which
           well-formed UTF-8 forbids (a code point must use its shortest encoding; NUL is the one-byte
           `00`). A decoder that only checked the leading/continuation byte STRUCTURE (a lead byte
           `110xxxxx` then a `10xxxxxx` continuation) would wrongly accept `C0 80`; strict UTF-8 (the
           Unicode definition, matching `str::from_utf8`) rejects it, so `String.from-bytes` yields
           `None`. Pins that the decode enforces shortest-form, not just byte shape — a security-relevant
           distinction (overlong encodings have been used to smuggle forbidden bytes past naive
           validators). This is a requirement on the runtime's UTF-8 validator the reader relies on:
           the byte sequence, not the code point, must be canonical.")
  (needs  binary-matching)
  (input  (= (String.from-bytes (Bytes.of (list 192 128))) None))
  (output (: true Bool)))

(case "decoding a surrogate code point encoded as UTF-8 yields none"
  (doc    "`(Bytes.of (list 237 160 128))` is `ED A0 80` — the UTF-8-shaped encoding of U+D800, a HIGH
           SURROGATE. Surrogates are not Unicode scalar values (they exist only for UTF-16 pairing), so
           well-formed UTF-8 excludes them even though the three-byte structure `1110xxxx 10xxxxxx
           10xxxxxx` is superficially valid. Strict UTF-8 rejects the surrogate range U+D800..=U+DFFF, so
           `String.from-bytes` yields `None`. The decode companion of the `Char.from-int` surrogate case
           (which rejects U+D800 as data): a String is a sequence of scalar values, so a byte sequence
           encoding a surrogate is not a well-formed String. Pins that the runtime validator rejects
           surrogate encodings, not only structurally-broken bytes — the same Unicode-scalar boundary
           the char surface enforces, now on the byte-decode path the reader uses.")
  (needs  binary-matching)
  (input  (= (String.from-bytes (Bytes.of (list 237 160 128))) None))
  (output (: true Bool)))

(case "encoding a decoded string round-trips to the same bytes"
  (doc    "For well-formed bytes `b`, decoding then re-encoding yields `b`: matching the `(Some s)` arm of
           `(String.from-bytes b)` and taking `(String.to-bytes s)` gives back the original UTF-8 bytes.
           Pins encode as the inverse of decode-of-well-formed (collections-and-text.md #Decoding Bytes To
           A String Is Total, Not Trapping, 3rd sentence).")
  (needs  binary-matching)
  (input  (match (String.from-bytes (Bytes.of (list 99 97 102 195 169)))
            ((Some s) (= (String.to-bytes s) (Bytes.of (list 99 97 102 195 169))))
            ((None _) false)))
  (output (: true Bool)))

(case "a helper decodes bytes to a string and consumes the fallible result"
  (doc    "The reader's symbol-table idiom: a helper takes raw `Bytes` (a slice of the input), decodes
           them with `String.from-bytes`, and `match`es the fallible result — binding the string in the
           `Some` arm to measure/intern it, and handling malformed bytes in the `None` arm. `(dec (Bytes.of
           (list 104 105)))` decodes \"hi\" and returns its byte length 2. Pins `String.from-bytes`
           consumed THROUGH A FUNCTION BOUNDARY (the shape a self-hosted reader materializes its symbol
           table with), not only at the entrypoint: decoding at `main` directly and matching there works,
           but the same decode-and-match inside a called helper does not yet — the fallible decode's result
           must survive the boundary the way `Bytes.at`/`List.at` results now do. Companion of the
           round-trip case above, which matches `from-bytes` at `main`; this one crosses a call.")
  (needs  fallible-access)
  (input  (module m
            (def (dec b) (match (String.from-bytes b)
                           ((Some s) (String.byte-len s))
                           ((None _) -1)))
            (def (main) (dec (Bytes.of (list 104 105))))))
  (output (: 2 Int64)))

(case "decoding ill-formed bytes through a helper takes the None arm"
  (doc    "The ill-formed companion: `String.from-bytes` of a RUNTIME Bytes that is not well-formed
           UTF-8 yields `(None unit)`, so the helper's `None` arm returns -1 — a TOTAL decode, never a
           trap (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). `(list
           255)` is a lone `0xFF`, an invalid lead byte. Pins that the runtime UTF-8 validator (emitted
           inline, matching `std::str::from_utf8` — rejecting invalid leads, overlong forms, surrogates,
           and code points > U+10FFFF) drives the fallible decode's `None`, so a reader handles
           malformed input rather than trapping on it. Companion of the well-formed case above.")
  (needs  fallible-access)
  (input  (module m
            (def (dec b) (match (String.from-bytes b)
                           ((Some s) (String.byte-len s))
                           ((None _) -1)))
            (def (main) (dec (Bytes.of (list 255))))))
  (output (: -1 Int64)))

(case "a utf8 bin segment binds a decoded string when the bytes are well-formed"
  (doc    "The `bin` pattern `(bin (u8 n) (utf8 name n))` reads a length byte n, then decodes exactly the
           next n bytes as UTF-8 into `name : String`. Against `(list 3 102 111 111)` — n=3, then the
           ASCII bytes of \"foo\" — the `utf8` segment matches and binds name = \"foo\". Pins the
           string-typed binary segment (options/binary-syntax/), the decode built into pattern matching.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 3 102 111 111))
            ((bin (u8 n) (utf8 name n)) name)
            (_ "invalid")))
  (output (: "foo" String)))

(case "a utf8 bin segment is a non-match on ill-formed bytes, forcing the catch-all"
  (doc    "The same pattern `(bin (u8 n) (utf8 name n))` against `(list 1 255)` — n=1, then the byte 0xFF,
           which is not well-formed UTF-8 — does NOT match the `utf8` segment, so control falls to the
           catch-all and yields \"invalid\". The decode failure is a NON-MATCH, never a trap; because the
           match must be exhaustive (CDZ0210), a catch-all is required, so the ill-formed case is
           necessarily handled (collections-and-text.md #Decoding Bytes To A String Is Total, Not
           Trapping — the exhaustiveness clause). This is how binary matching absorbs invalid UTF-8.")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 1 255))
            ((bin (u8 n) (utf8 name n)) name)
            (_ "invalid")))
  (output (: "invalid" String)))

(case "a utf8-decoding bin match with no catch-all is non-exhaustive"
  (doc    "A `bin` pattern with a `(utf8 …)` segment can fail to match on ill-formed bytes, so a match
           whose only arm is such a pattern does not cover every byte sequence and is rejected CDZ0210 —
           the same exhaustiveness rule every `bin` match obeys, made pointed by the fact that the decode
           itself is a source of non-match. Pins that the ill-formed-UTF-8 case cannot be silently
           dropped: the compiler forces a branch for it (collections-and-text.md #Decoding Bytes To A
           String Is Total, Not Trapping).")
  (needs  binary-matching)
  (input  (match (Bytes.of (list 3 102 111 111))
            ((bin (u8 n) (utf8 name n)) name)))
  (error  CDZ0210))

; --- Char — a single validated Unicode scalar, the element type of a string's scalar sequence ----
; collections-and-text.md #A Char Is A Single Unicode Scalar Value: a `Char` is one Unicode scalar
; (a code point in U+0000..=U+10FFFF EXCLUDING the surrogate range U+D800..=U+DFFF), the element type
; of the sequence a String already is (#A String Is A Sequence Of Unicode Scalar Values). The
; type-mapping table already carried a `char` boundary row with no surface producer; `Char` is that
; producer (spec/learnings/2026-07-05-char-is-a-validated-unicode-scalar-the-boundary-already-
; promises.md). A char literal is written `#\<scalar>` (options/char-literal-syntax/hash-scalar-
; literal.md); `Char` is a prelude record, so `Char.to-int` is `(. Char to-int)` and `Char.from-int`
; is `(. Char from-int)`, and `String.scalar-at` reads one scalar of a string.
;
; Tagged `(needs chars)` — a FRESH capability the seed does not realize (NOT the realized
; `collections`, which would make the seed RUN these and reject the unbound `Char`/`String.scalar-at`
; names with a coded diagnostic — a gate FAIL — rather than skip; the same reason `symbols` uses its
; own tag). A later generation realizes scalar access and the `Char` value form; until then the seed's
; behavior gate SKIPS these — they pin the contract the realization must meet, not seed declines.

(case "reading a string's scalar in bounds yields Some of the char"
  (doc    "Witnesses collections-and-text.md #A String's Scalars Are Addressable: `(String.scalar-at
           \"hello\" 1)` reads the scalar at scalar-position 1 — the char `#\\e` — wrapped in Some (an
           Option<Char>, the fallible read analogous to List.at and String.at). This is the operation
           that was missing: String.scalar-len counted scalars but nothing returned one.")
  (needs  chars)
  (input  (String.scalar-at "hello" 1))
  (output (: (Some #\e) (Option Char))))

(case "reading a string's scalar out of bounds yields None"
  (doc    "The out-of-bounds companion: `(String.scalar-at \"hi\" 5)` reads past the end of a two-scalar
           string, so it yields None rather than trapping (collections-and-text.md #A String's Scalars
           Are Addressable — reading is total, and #Indexing And Lookup Are Fallible, Not Trapping). The
           Char analogue of the out-of-range String.at / List.at Nones.")
  (needs  chars)
  (input  (String.scalar-at "hi" 5))
  (output (: (None unit) (Option Char))))

(case "reading a string's scalar addresses scalar values, not bytes"
  (doc    "`(String.scalar-at \"café\" 3)` is `(Some #\\é)`: the string is four scalar values (c, a, f,
           é) and scalar-position 3 is é — even though é occupies bytes 3–4 of the five-byte UTF-8
           encoding, so a byte offset would land mid-scalar. Pins that scalar access addresses by scalar
           value (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), returning a
           Char — the Char companion of the scalar-indexed String.at case above.")
  (needs  chars)
  (input  (String.scalar-at "café" 3))
  (output (: (Some #\é) (Option Char))))

(case "converting a char to its integer scalar value is total"
  (doc    "Witnesses collections-and-text.md #A Char Converts To And From An Integer Totally:
           `(Char.to-int #\\a)` is 97 — the Unicode scalar value (code point) of the char `a`. Total:
           every char is a scalar value that has an integer code point, so to-int never fails.")
  (needs  chars)
  (input  (Char.to-int #\a))
  (output (: 97 Int64)))

(case "converting a scalar-valued integer to a char yields Some"
  (doc    "`(Char.from-int 97)` is `(Some #\\a)` — 97 is the scalar value of `a`, a valid Unicode scalar,
           so the conversion succeeds (collections-and-text.md #A Char Converts To And From An Integer
           Totally). from-int is FALLIBLE (returns an Option) because not every integer is a scalar; this
           is the success arm.")
  (needs  chars)
  (input  (Char.from-int 97))
  (output (: (Some #\a) (Option Char))))

(case "converting a surrogate code point to a char yields None"
  (doc    "`(Char.from-int 55296)` — 55296 is U+D800, a HIGH SURROGATE, which is NOT a Unicode scalar
           value — so from-int yields None rather than producing an invalid char (collections-and-text.md
           #A Char Converts To And From An Integer Totally, and #A Char Is A Single Unicode Scalar Value:
           surrogates are excluded). Pins that the surrogate range is rejected as data (None), never a
           trap and never an ill-formed Char. This is why from-int must be fallible.")
  (needs  chars)
  (input  (Char.from-int 55296))
  (output (: (None unit) (Option Char))))

(case "converting an out-of-range integer to a char yields None"
  (doc    "`(Char.from-int 1114112)` — 1114112 is U+110000, one past the maximum scalar U+10FFFF — so it
           is not a scalar value and from-int yields None (collections-and-text.md #A Char Converts To
           And From An Integer Totally). The high-end companion of the surrogate case; both are handled
           as data, not traps.")
  (needs  chars)
  (input  (Char.from-int 1114112))
  (output (: (None unit) (Option Char))))

(case "char to-int and from-int round-trip through the scalar value"
  (doc    "For a scalar value v, `(Char.from-int v)` is `(Some c)` and `(Char.to-int c)` is v again:
           `(Char.to-int #\\a)` = 97 and `(Char.from-int 97)` = `(Some #\\a)`, so matching the Some arm
           and taking to-int returns 97. Pins from-int as the inverse of to-int on a valid scalar
           (collections-and-text.md #A Char Converts To And From An Integer Totally). MUST be true.")
  (needs  chars)
  (input  (match (Char.from-int 97)
            ((Some c) (= (Char.to-int c) 97))
            ((None _) false)))
  (output (: true Bool)))

(case "a char literal naming a surrogate is a reader error"
  (doc    "`#\\u+D800` names U+D800, a high surrogate — NOT a Unicode scalar value — so the char literal
           denotes no valid scalar and the reader rejects it (CDZ0002, collections-and-text.md #A Char Is
           A Single Unicode Scalar Value; options/char-literal-syntax/). The static companion of the
           dynamic `(Char.from-int 55296)` → None: a literal cannot spell a non-scalar, so the surrogate
           case is caught at read time rather than producing an invalid Char.")
  (needs  chars)
  (input  #\u+D800)
  (error  CDZ0002))

(case "a char order agrees with its scalar value"
  (doc    "Witnesses collections-and-text.md #A Char Is A Single Unicode Scalar Value (2nd sentence: a
           char's ordering is the numeric order of its scalar value): `(< #\\a #\\b)` is true because
           the scalar value of `a` (97) is less than that of `b` (98). Pins that a Char order and the
           string order defined on scalar values agree by construction — a Char is comparable and its
           order is its scalar order.")
  (needs  chars)
  (input  (< #\a #\b))
  (output (: true Bool)))

(case "two equal chars compare equal"
  (doc    "`(= #\\a #\\a)` is true — a char's value is its scalar, and two chars are equal exactly when
           their scalar values are equal. Pins Char equality as scalar equality, the equality companion
           of the Char order case.")
  (needs  chars)
  (input  (= #\a #\a))
  (output (: true Bool)))
