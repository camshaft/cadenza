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

(case "string slicing"
  (input  (String.slice "hello world" 0 5))
  (output (: "hello" String)))

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
