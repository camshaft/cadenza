; String operations — witnesses collections-and-text.md. The compiler needs string operations
; for error messages, name encoding (export names → bytes in wasm), and instruction tag dispatch.
; String equality is already witnessed in 01-literals; these cover the remaining operations
; the compiler needs.

(case "string concatenation"
  (doc    "The compiler builds error messages and export names via string concatenation.")
  (input  (String.concat "hello" " world"))
  (output (: "hello world" String)))

(case "string length"
  (input  (String.len "hello"))
  (output (: 5 Int64)))

(case "string length counts Unicode scalar values, not bytes"
  (doc    "Witnesses collections-and-text.md #A String's Length MUST Be Counted In Unicode Scalar
           Values. \"café\" is four scalar values (c, a, f, é) but FIVE UTF-8 bytes — é encodes as
           two bytes. String.len is the scalar count (4), NOT the byte count (5). The byte count is
           what String.to-bytes → Bytes.len yields (the case just below), and the two differ here
           precisely because the string is multi-byte. `String.len \"hello\"` above cannot witness
           this — ASCII makes the two counts coincide.")
  (input  (String.len "café"))
  (output (: 4 Int64)))

(case "string length of a supplementary-plane character is one scalar value"
  (doc    "Witnesses collections-and-text.md #A String's Length MUST Be Counted In Unicode Scalar
           Values at the boundary that most tempts a byte- or UTF-16-based miscount: \"😀\" (U+1F600)
           is a single Unicode scalar value — length 1 — even though it is four UTF-8 bytes (and two
           UTF-16 code units). A length implementation counting bytes would report 4, UTF-16 units 2;
           the scalar count is 1.")
  (input  (String.len "😀"))
  (output (: 1 Int64)))

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
           scalar values c, a, f, é, the same as the composed form (String.len \"café\" = 4,
           witnessed above). The seed counts the un-normalized e + combining acute as 5 scalar values.")
  (input  (String.len "café"))
  (output (: 4 Int64)))

(case "string indexing returns a character"
  (input  (String.at "hello" 1))
  (output (: "e" String)))

; --- String indexing is by Unicode scalar value, not by byte -----------------------------
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values ("its contents are
; independent of any byte encoding") + #A String's Length MUST Be Counted In Unicode Scalar Values:
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
           of String.len counting scalars; the ASCII `(String.at \"hello\" 1)` cannot distinguish
           scalar index from byte offset.")
  (input  (String.at "café" 3))
  (output (: "é" String)))

(case "string indexing past a supplementary-plane scalar lands on the next scalar"
  (doc    "`(String.at \"😀b\" 1)` = \"b\": 😀 (U+1F600) is ONE scalar value occupying four UTF-8
           bytes (and two UTF-16 code units), so scalar index 1 is the character AFTER it, \"b\". A
           byte- or UTF-16-based index would land inside 😀's encoding. Pins scalar-value addressing
           at the boundary that most tempts a byte/UTF-16 miscount (the indexing companion of the
           supplementary-plane length case).")
  (input  (String.at "😀b" 1))
  (output (: "b" String)))

; --- String.at and String.slice are total-or-trap: an out-of-range index has no result ----
; collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values gives a string a defined
; scalar length, and core-semantics.md #Partial Operations Have A Defined Outcome requires an
; operation with no result for some inputs to trap rather than produce an unspecified value. So
; String.at at a scalar index at or beyond the length, or at a NEGATIVE index, has no character to
; return and MUST trap — exactly as List.at / Bytes.at do out of bounds (05-compound-types,
; 10-bytes). A negative index is the classic miscompile: a lowering that casts the index to an
; unsigned width turns -1 into a huge in-range-looking offset; the scalar-index bounds check must
; catch it as out of range and trap.

(case "string indexing at or beyond the length traps"
  (doc    "`(String.at \"hi\" 5)` indexes scalar position 5 of a two-scalar string — out of range, no
           character to return — so it MUST trap (core-semantics.md #Partial Operations Have A Defined
           Outcome), the String companion of the List.at / Bytes.at out-of-bounds traps.")
  (input  (String.at "hi" 5))
  (trap   "string index out of bounds"))

(case "a negative string index traps rather than wrapping to a large offset"
  (doc    "`(String.at \"hi\" -1)` uses a negative scalar index — no defined character — so it MUST
           trap, NOT wrap. A lowering that casts the index to an unsigned integer would turn -1 into a
           huge positive offset (either reading out of bounds or, worse, an unspecified in-range byte);
           the total-or-trap discipline requires a trap (core-semantics.md #Partial Operations Have A
           Defined Outcome). The negative-index companion of the out-of-range case above.")
  (input  (String.at "hi" -1))
  (trap   "string index out of bounds"))

(case "string slicing"
  (input  (String.slice "hello world" 0 5))
  (output (: "hello" String)))

; --- String.slice bounds are checked: reversed, out-of-range, and negative bounds trap; ----
; --- an empty slice is a value ----------------------------------------------------------
; String.slice takes a start and an end scalar index. A well-defined slice needs 0 ≤ start ≤ end ≤
; length; any bounds outside that have no defined substring and MUST trap (core-semantics.md #Partial
; Operations Have A Defined Outcome), while a degenerate but in-range slice where start = end is the
; empty string — a value, not a trap. These pin the boundary the encoder relies on when it slices
; instruction/name substrings: a reversed or over-long range is a trap, an empty range is "".

(case "a slice whose end is beyond the string length traps"
  (doc    "`(String.slice \"hi\" 0 5)` asks for scalars 0..5 of a two-scalar string — the end 5 is
           beyond the length — so the slice has no defined substring and MUST trap (core-semantics.md
           #Partial Operations Have A Defined Outcome).")
  (input  (String.slice "hi" 0 5))
  (trap   "string slice out of bounds"))

(case "a slice whose end precedes its start traps"
  (doc    "`(String.slice \"hello\" 3 1)` has end 1 before start 3 — a reversed range with no defined
           substring — so it MUST trap rather than return an empty or reversed string. Pins that the
           start ≤ end constraint is checked, not silently normalized.")
  (input  (String.slice "hello" 3 1))
  (trap   "string slice out of bounds"))

(case "a slice with a negative start traps"
  (doc    "`(String.slice \"hello\" -1 3)` has a negative start index — outside 0..length — so it MUST
           trap, not wrap a negative bound to a large unsigned offset (the same negative-index
           miscompile the String.at case guards). Pins that both slice bounds are range-checked as
           signed values.")
  (input  (String.slice "hello" -1 3))
  (trap   "string slice out of bounds"))

(case "a slice whose start equals its end is the empty string"
  (doc    "`(String.slice \"hello\" 2 2)` is a degenerate but in-range slice (0 ≤ 2 ≤ 2 ≤ 5): it
           selects zero scalars, so it is the empty string \"\" — a value, NOT a trap. Pins that the
           bounds check admits start = end (an empty result) rather than rejecting it, the boundary
           just inside the reversed-range trap above.")
  (input  (String.slice "hello" 2 2))
  (output (: "" String)))

; --- A string operation consumes a string SELECTED by runtime control flow -----------------
; A string operation (String.len/at/slice/concat) takes a string ARGUMENT; that argument may be a
; string SELECTED at run time by an `if` or a `match`, not only a literal. The seed resolves a string
; operation's argument through a runtime `if` — `(String.len (if b "hello" "hi"))` computes the
; selected string's length — but NOT through a runtime `match`: `(String.len (match n (0 "zero") (_
; "other")))` declines. Both `if` and `match` select a string value the same way, so a string
; operation must consume either. (This is about CONSUMING a runtime-selected string in an operation,
; distinct from RETURNING one across the run boundary — the latter needs the compound-value output ABI.)

(case "String.len of a string selected by a runtime match"
  (doc    "`(match n (0 \"zero\") (_ \"other\"))` selects a string by the runtime value n; `String.len`
           of that selected string is its scalar length — with n=5 the wildcard picks \"other\", length
           5. A string operation must consume a match-selected string exactly as it consumes an
           if-selected one (the control below, which the seed runs). The seed declines the match case
           (\"unsupported dotted-application\") — its string-op argument resolution follows a runtime
           `if` but not a runtime `match`.")
  (input   (module m
             (def (f n) (String.len (match n (0 "zero") (_ "other"))))
             (def (main) (f 5))))
  (output  (: 5 Int64)))

(case "String.len of a string selected by a runtime if"
  (doc    "The control the case above must match: `String.len` of a string chosen by a runtime `if`
           computes the selected string's length — `(if b \"hello\" \"hi\")` with b=true is \"hello\",
           length 5. The seed runs this; the match companion must behave identically.")
  (input   (module m
             (def (f b) (String.len (if b "hello" "hi")))
             (def (main) (f true))))
  (output  (: 5 Int64)))
