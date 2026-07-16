; String operations — witnesses collections-and-text.md. The compiler needs string operations
; for error messages, name encoding (export names → bytes in wasm), and instruction tag dispatch.
; String equality is already witnessed in 01-literals; these cover the remaining operations
; the compiler needs.

(case "string concatenation"
  (doc    "The compiler builds error messages and export names via string concatenation.")
  (input  (String.concat "hello" " world"))
  (output (: "hello world" String)))

(case "a runtime-built string crosses the boundary as its value form"
  (doc    "A String built at RUN TIME (a recursion over a live parameter, not a compile-time constant)
           returns to the host as `(: \"…\" String)`. A runtime String is the same UTF-8 byte-rope heap
           value as Bytes (String.concat is a byte-rope concat), so it escapes through the SAME looping
           encoder that a runtime Bytes does — only the value-form frame differs (a String vs a Bytes
           leaf). `rep` appends \"x\" `n` times to \"hi\"; with n=3 → \"hixxx\". Pins that a genuinely
           runtime String (not a folded constant) has a component-boundary representation — it declined
           before as \"String has no component boundary representation\".")
  (input  (do (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
              (def (main) (rep "hi" 3))
              (export main)))
  (output (: "hixxx" String)))

(case "a runtime string rope compares equal to its flat twin"
  (doc    "The equality companion of the rope-escape case above: `(= (rep \"hi\" 3) \"hixxx\")` is TRUE →
           1. `rep` appends \"x\" three times via `String.concat`, building a genuinely RUNTIME String ROPE
           whose content IS \"hixxx\" (it renders as \"hixxx\", byte-len 5). `=` lowers to `value-eq` =
           `champ_eq`, a PHYSICAL-byte compare — and a `String.concat` rope's bytes differ from a flat
           leaf's of identical content, so the rope compared UNEQUAL to the flat literal (returned 0, a
           silent MISCOMPILE) until the compiler CANONICALIZES an owned String operand with `bytes-compact`
           before the compare. A flat runtime string (n=0, no concat) already compared correctly; only a
           rope operand needed compaction. Also fixes a string `match` against a literal (it desugars to
           the same `value-eq` chain). Pins that a runtime rope and its flat twin are `=`-equal.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (rep "hi" 3) "hixxx") 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a runtime string rope matches a string-literal arm"
  (doc    "The `match` sibling: `(match (rep \"hi\" 3) (\"hixxx\" 1) (_ 0))` takes the \"hixxx\" arm → 1. A
           string `match` desugars to a chain of `(= scrutinee <literal>)` value-eq tests, so it hit the
           same rope-vs-flat physical-byte miscompile (took the `_` arm, returning 0) until the rope operand
           is compacted before the compare. Confirms the fix covers the match-desugar path, not only `=`.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (match (rep "hi" 3) ("hixxx" 1) (_ 0)))
            (export main)))
  (output (: 1 Int64)))

(case "a runtime string rope compared through a borrowed operand"
  (doc    "The BORROWED-operand remainder of the rope-eq fix. The earlier fix compacted only an OWNED
           String operand (a fresh `String.concat` result); a genuine rope reaching `=` through a
           BORROWED operand — a `Map.lookup`-stored value, a `SumPayload`-extracted payload, or a
           runtime-rope param — was compared by its UNFLATTENED header bytes and silently returned the
           WRONG answer. Here `rep \"hi\" 3` = \"hixxx\" (a runtime rope) is stored as a map value, looked
           up, and compared as the BORROWED `Some` payload `s` INSIDE the arm (`(= s \"hixxx\")`). Because
           `bytes-compact` is refcount-NEUTRAL (it flattens the node IN PLACE and returns the same handle,
           unobservable even when shared), the compiler now compacts EVERY String operand — owned OR
           borrowed — and drops only the owned ones, so the borrowed rope payload compares by content.
           Expected: the arm fires and returns 1 (was 0 — a champ_eq physical-byte miss).")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (f (: mp (Map String String)) (: k String))
              (match (Map.lookup mp k)
                ((Some s) (if (= s "hixxx") 1 0))
                ((None) (- 0 1))))
            (def (main) (f (Map.insert (Map.empty) "y" (rep "hi" 3)) "y"))
            (export main)))
  (output (: 1 Int64)))

(case "a string equality on an inlined match operand"
  (doc    "A `String ==` whose operand is an INLINED function returning a `match` — `(= (f …) \"z\")` where
           `f` returns `(match (Map.lookup m k) ((Some s) s) ((None) \"?\"))`, β-reduced into the `=`
           operand (the default for a non-recursive call). The two arms DISAGREE on ownership: the `Some`
           arm returns a BORROWED payload (`s`, a `sum-payload` read off the owned Option), the `None` arm
           returns an OWNED constant (\"?\"). The value-eq operand-ownership analysis had no `match` arm and
           fell through to a DECLINE (`borrowing op operand has an ownership this backend cannot yet
           prove`), blocking any program that compares a returned map/variant payload once the wrapper
           inlines — the shape a compiler-in-Cadenza substitution pass hits. It now classifies a match
           operand by the JOIN of its arm bodies (owned iff every arm is owned, else borrowed — the
           leak-safe value), so the mixed join here is borrowed: no drop follows, and the leak matches the
           standalone-function path. The lookup finds \"z\", so `= \"z\"` is true → 1.")
  (input  (do
            (def (f (: m (Map String String)) (: k String))
              (match (Map.lookup m k) ((Some s) s) ((None) "?")))
            (def (main) (if (= (f (Map.insert (Map.empty) "y" "z") "y") "z") 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a mixed-ownership inlined-match operand leaves the source map intact across repeated compares"
  (doc    "The DROP-CORRUPTION guard for the match-operand ownership JOIN above: the join classifies the
           mixed Some/None operand as BORROWED, so NO post-compare drop is emitted. Had it mis-joined to
           OWNED, the drop after the first compare would free the "z" payload the map still owns — and
           this case would see it: the SAME let-bound map is consulted TWICE through the inlined match
           (each compare true → 1 + 1) and then read structurally (`Map.len` → 1), so 3. A use-after-free
           on the payload flips the second compare (→ 2) or corrupts the size read. Pins the leak-safe
           side of the join with the source value still live.")
  (input  (do
            (def (f (: m (Map String String)) (: k String))
              (match (Map.lookup m k) ((Some s) s) ((None) "?")))
            (def (main)
              (let ((m (Map.insert (Map.empty) "y" "z")))
                (+ (+ (if (= (f m "y") "z") 1 0)
                      (if (= (f m "y") "z") 1 0))
                   (Map.len m))))
            (export main)))
  (output (: 3 Int64)))

(case "an all-owned-arms match operand compares correctly through the ownership join"
  (doc    "The OWNED side of the match-operand join: BOTH arms build a fresh `String.concat` result, so
           the join is Owned (every arm owned) and the post-compare drop is correct — each arm's rope is
           a temporary nothing else holds. k = 0 → \"zx\" = \"zx\" → 1; k = 1 → \"zy\" ≠ \"zx\" → 0 (a
           genuine value test, both directions). With the mixed-arm case above this pins both join
           outcomes: all-owned → owned (dropped), mixed → borrowed (left to the owner).")
  (input  (do
            (def (pick (: k Int64))
              (match k (0 (String.concat "z" "x")) (_ (String.concat "z" "y"))))
            (def (main (: k Int64))
              (if (= (pick k) "zx") 1 0))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64))
  (call   main (: 1 Int64))
  (output (: 0 Int64)))

(case "a runtime string rope map key is found by its flat twin"
  (doc    "The MAP-KEY companion of the rope-eq cases above: a map keyed by a runtime String ROPE
           `(rep \"hi\" 3)` = \"hixxx\" looked up with the flat literal \"hixxx\" MUST find 42. The map-key
           path hashes+compares with `champ_hash`/`champ_eq` — a PHYSICAL-byte compare, the SAME contract
           `value-eq` uses — so a rope key (different bytes than its flat twin) landed in a different slot
           and `Map.lookup` returned `None` (→ -1), a silent MISCOMPILE. The value-eq rope fix compacted
           only the `=`/match operand; this pins the twin KEY path: the compiler now `bytes-compact`s an
           owned String key at every Map/Set champ site (insert/lookup/remove, set-of/insert/contains/
           remove), so a rope key and its flat twin hash+compare equal. Expected: 42.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert (Map.empty) (rep "hi" 3) 42) "hixxx")
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

(case "a flat string map key is found by a runtime rope twin"
  (doc    "The symmetric direction: insert under the FLAT literal key \"hixxx\", look up with the runtime
           ROPE `(rep \"hi\" 3)` = \"hixxx\" → 42. Compaction canonicalizes the LOOKUP key too (not only the
           inserted key), so the rope lookup key hashes into the flat key's slot. Confirms the fix covers
           both champ sites — the inserted key AND the lookup/borrow key — not just one direction.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert (Map.empty) "hixxx" 42) (rep "hi" 3))
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

(case "a runtime string rope inserted into a set is a member"
  (doc    "The SET element-insert companion of the map-key cases: inserting a runtime String ROPE
           `(rep \"hi\" 3)`=\"hixxx\" into an empty set yields a 1-element set (`Set.len` = 1). Two
           earlier faults: the empty set `(Set.of (list))` leaves its element type an unresolved VAR
           (no construction pins it), so the backend defaulted the element box to `box-int` — which
           mis-boxed the i32 String HANDLE as an integer, emitting an INVALID component; and the rope
           element was not compacted before champ_insert. The fix boxes the element by its OWN concrete
           type when the set's declared type is unresolved, and compacts a rope element. A flat string /
           an Int element already inserted (box-int is correct for an Int); only a heap-handle element
           into an empty set hit the box bug. Expected: len 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (Set.len (Set.insert (Set.of (list)) (rep "hi" 3))))
            (export main)))
  (output (: 1 Int64)))

(case "a runtime string rope set element is found by its flat twin"
  (doc    "The membership companion: after inserting the runtime ROPE `(rep \"hi\" 3)`=\"hixxx\" into a
           set, `Set.contains` with the flat literal \"hixxx\" is TRUE (→ 1). The element-insert now
           compacts the rope to its canonical flat leaf before champ_insert (mirroring the Map key and
           the Set QUERY key), so the flat query's champ_hash lands in the same slot. Pins that the Set
           ELEMENT-insert path canonicalizes a rope element, not only the query key.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (Set.contains (Set.insert (Set.of (list)) (rep "hi" 3)) "hixxx") 1 0))
            (export main)))
  (output (: 1 Int64)))

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

(case "string to bytes produces the exact UTF-8 byte values, not just the right count"
  (doc    "String.to-bytes must produce the exact UTF-8 BYTES, not merely the right byte count — a
           length-only check would pass a Latin-1 encoding or a wrong continuation byte. `é` (U+00E9)
           encodes as the two bytes 0xC3 0xA9 = 195 169, and the 4-byte astral `😀` (U+1F600) as 0xF0 0x9F
           0x98 0x80 = 240 159 152 128. `(String.to-bytes \"é😀\")` therefore equals `(Bytes.of (list 195
           169 240 159 152 128))` — the 2-byte then 4-byte sequences concatenated. Pins the byte-level
           correctness of the UTF-8 encoder across the 2-byte and 4-byte forms (the boundaries a naive
           encoder gets wrong), the value companion of the byte-count cases above.")
  (input  (= (String.to-bytes "é😀") (Bytes.of (list 195 169 240 159 152 128))))
  (output (: true Bool)))

(case "string equality"
  (input  (= "hello" "hello"))
  (output (: true Bool)))

(case "string inequality"
  (input  (= "hello" "world"))
  (output (: false Bool)))

(case "a string-literal pattern in a match selects by string equality, distinguishing Unicode"
  (doc    "A `match` may test a String scrutinee against string-LITERAL patterns, selecting the arm whose
           literal equals the scrutinee (by normalized contents, collections-and-text.md #String Equality
           Follows Normalized Contents) — the same equality the `=` operator uses. `(match \"café\" (\"cafe\"
           1) (\"café\" 2) (_ 9))` selects the `\"café\"` arm, not `\"cafe\"`: the é distinguishes them, so
           the result is 2. Pins that string-literal pattern matching is by full Unicode-scalar equality
           (not a byte-prefix or ASCII-fold), and that a non-matching literal falls through to the wildcard
           — the string companion of the integer-literal-pattern match.")
  (input  (match "café" ("cafe" 1) ("café" 2) (_ 9)))
  (output (: 2 Int64)))

(case "a RUNTIME string scrutinee matches string literals by equality"
  (doc    "The RUNTIME companion of the constant string match above — THE compiler head-dispatch idiom: a
           `match` over a String chosen at run time, against string-LITERAL patterns. `op`'s scrutinee `s`
           is a runtime String (selected by a Bool, so it does not fold to a constant), matched `(match s
           (\"add\" 1) (\"sub\" 2) (_ 0))`. A String is a heap value, so this is not a scalar probe chain;
           it lowers to a chain of `(= s literal)` runtime string-equality tests (`value-eq`, the same
           equality `=` uses), the wildcard the tail. `op(if true \"add\" \"sub\")` = `op(\"add\")` selects
           the `\"add\"` arm → 1. Pins that a compiler can dispatch on a runtime keyword/opcode name by
           pattern, not only on a compile-time-constant string.")
  (input  (do
            (def (op (: s String)) (match s ("add" 1) ("sub" 2) (_ 0)))
            (def (main) (op (if true "add" "sub"))) (export main)))
  (output (: 1 Int64)))

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
  (input  (String.at "café" 3))
  (output (: (Some "é") (Option String))))

(case "string indexing past a supplementary-plane scalar lands on the next scalar"
  (doc    "`(String.at \"😀b\" 1)` = \"b\": 😀 (U+1F600) is ONE scalar value occupying four UTF-8
           bytes (and two UTF-16 code units), so scalar index 1 is the character AFTER it, \"b\". A
           byte- or UTF-16-based index would land inside 😀's encoding. Pins scalar-value addressing
           at the boundary that most tempts a byte/UTF-16 miscount (the indexing companion of the
           supplementary-plane length case).")
  (input  (String.at "😀b" 1))
  (output (: (Some "b") (Option String))))

(case "a constant-index String.at result compares equal to a literal by content"
  (doc    "`(= (String.at \"banana\" 1) \"a\")` is true — the scalar at index 1 of \"banana\" is \"a\", and
           the one-scalar String it yields compares equal by CONTENT to the literal \"a\". A constant-index
           `String.at` folds to a `ConstStr`, so its equality is a content compare. Pins that a `String.at`
           result is content-comparable (the character-classifying `(= (String.at s i) …)` a lexer uses).
           MUST be true.")
  (input  (= (Option.expect (String.at "banana" 1) "c") "a"))
  (output (: true Bool)))

(case "a constant-index String.at result compares unequal to a different literal"
  (doc    "The negative companion: index 0 of \"banana\" is \"b\", so `(= (String.at \"banana\" 0) \"a\")`
           is FALSE — the content compare distinguishes \"b\" from \"a\". Together with the case above this
           pins that a folded `String.at` result equals a literal exactly when their content matches. (At a
           CONSTANT index this equality folds and is correct; the same at a RUNTIME index is the
           failures-queue miscompile — these are the working boundary the bug sits against.) MUST be false.")
  (input  (= (Option.expect (String.at "banana" 0) "c") "a"))
  (output (: false Bool)))

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
  (input  (String.at "hi" 5))
  (output (: (None unit) (Option String))))

(case "a negative string index yields None rather than wrapping to a large offset"
  (doc    "`(String.at \"hi\" -1)` uses a negative scalar index — no defined character — so it MUST
           yield None, NOT wrap. A lowering that casts the index to an unsigned integer would turn -1
           into a huge positive offset (either reading out of bounds or, worse, an unspecified in-range
           byte); fallible indexing requires None (collections-and-text.md #Indexing And Lookup Are
           Fallible, Not Trapping). The negative-index companion of the out-of-range case above.")
  (input  (String.at "hi" -1))
  (output (: (None unit) (Option String))))

(case "string slicing yields Some of the substring"
  (doc    "Witnesses fallible String slicing: an in-bounds range yields the substring wrapped in Some
           (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping). This case reads
           the Option directly, without unwrapping, to pin the Some.")
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
  (input  (String.slice "hi" 0 5))
  (output (: (None unit) (Option String))))

(case "a slice whose end precedes its start yields None"
  (doc    "`(String.slice \"hello\" 3 1)` has end 1 before start 3 — a reversed range with no defined
           substring — so it MUST yield None rather than return an empty or reversed string. Pins that
           the start ≤ end constraint is checked, not silently normalized.")
  (input  (String.slice "hello" 3 1))
  (output (: (None unit) (Option String))))

(case "a slice with a negative start yields None"
  (doc    "`(String.slice \"hello\" -1 3)` has a negative start index — outside 0..length — so it MUST
           yield None, not wrap a negative bound to a large unsigned offset (the same negative-index
           miscompile the String.at case guards). Pins that both slice bounds are range-checked as
           signed values.")
  (input  (String.slice "hello" -1 3))
  (output (: (None unit) (Option String))))

(case "a slice whose start equals its end is Some of the empty string"
  (doc    "`(String.slice \"hello\" 2 2)` is a degenerate but in-range slice (0 ≤ 2 ≤ 2 ≤ 5): it
           selects zero scalars, so it is Some of the empty string \"\" — present, NOT None. Pins that
           the bounds check admits start = end (an empty result) rather than rejecting it, the boundary
           just inside the reversed-range None above.")
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
  (input  (do
            (def (f s) (Option.expect (String.slice s 1 4) "in range"))
            (def (main) (f "hello")) (export main)))
  (output (: "ell" String)))

(case "a runtime string slice addresses scalar values, not bytes"
  (doc    "`(String.slice s 1 3)` on `s = \"aébc\"` yields Some \"éb\" — scalars 1 and 2 (é is one
           scalar, TWO UTF-8 bytes). A runtime slice that indexed by BYTE offset would split é or read
           the wrong range; pins that the runtime walk maps scalar offsets to byte offsets, exactly as
           String.at does (13-strings §reading a string's scalar addresses scalar values, not bytes).")
  (input  (do
            (def (f s) (Option.expect (String.slice s 1 3) "in range"))
            (def (main) (f "aébc")) (export main)))
  (output (: "éb" String)))

(case "a runtime string slice out of range yields None"
  (doc    "`(String.slice s 0 5)` on the two-scalar `s = \"hi\"` has end 5 past the length, so it yields
           None — the runtime bounds check agrees with the folded out-of-range case. The match takes the
           None arm (-1), witnessing the absent result rather than a trap or a short string.")
  (input  (do
            (def (f s) (match (String.slice s 0 5) ((Some x) (String.byte-len x)) ((None _) -1)))
            (def (main) (f "hi")) (export main)))
  (output (: -1 Int64)))

(case "a runtime string slice with an empty in-range span is Some of the empty string"
  (doc    "`(String.slice s 2 2)` on a runtime `s = \"hello\"` selects zero scalars — Some \"\", present
           not None (the empty-span boundary, on the runtime path). `String.byte-len` of the result is
           0, distinguishing Some \"\" (0) from None (which the match would send elsewhere).")
  (input  (do
            (def (f s) (match (String.slice s 2 2) ((Some x) (String.byte-len x)) ((None _) -1)))
            (def (main) (f "hello")) (export main)))
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
  (input   (do
             (def (f n) (String.scalar-len (match n (0 "zero") (_ "other"))))
             (def (main) (f 5)) (export main)))
  (output  (: 5 Int64)))

(case "String.scalar-len of a string selected by a runtime if"
  (doc    "The control the case above must match: `String.scalar-len` of a string chosen by a runtime `if`
           computes the selected string's length — `(if b \"hello\" \"hi\")` with b=true is \"hello\",
           length 5. The seed runs this; the match companion must behave identically.")
  (input   (do
             (def (f b) (String.scalar-len (if b "hello" "hi")))
             (def (main) (f true)) (export main)))
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
  (input   (do
             (def (len2 s) (String.byte-len s))
             (def (main)   (len2 "hello")) (export main)))
  (output  (: 5 Int64)))

(case "a runtime string equality selects a branch by comparing a parameter to a literal"
  (doc    "`(if (= s \"def\") 1 0)` compares a runtime string parameter `s` against a literal — the
           name-dispatch primitive a compiler uses to recognize a form's head. Equality is structural
           over the UTF-8 bytes: `(pick \"def\")` is 1 (bytes match), `(pick \"x\")` is 0 (length
           differs). Their sum is 1. Pins runtime string `=` as a byte comparison, not a handle identity.")
  (input   (do
             (def (pick s) (if (= s "def") 1 0))
             (def (main)   (+ (pick "def") (pick "x"))) (export main)))
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
  (input   (do
             (def (eval-head h a b)
               (if (= h "+") (+ a b)
               (if (= h "-") (- a b)
               (if (= h "*") (* a b) 0))))
             (def (main) (eval-head "+" 20 22)) (export main)))
  (output  (: 42 Int64)))

(case "a string carried as a sum-variant payload is bound and measured at run time"
  (doc    "A `Node` variant carries a String payload built at run time; a `match` binds it and takes its
           byte length. `(weigh (Node.NSym \"hello\"))` is 5 (the bound string's byte length) and
           `(weigh (Node.NInt 3))` is 3, summing to 8. Pins a string as a runtime sum payload — the
           shape of a symbol-carrying AST node the compiler walks — bound by a match arm and consumed.")
  (input   (do
             (type Node (NInt Int64) (NSym String))
             (def (weigh n) (match n ((Node.NInt i) i) ((Node.NSym s) (String.byte-len s))))
             (def (main)    (+ (weigh (Node.NSym "hello")) (weigh (Node.NInt 3)))) (export main)))
  (output  (: 8 Int64)))

(case "concatenating two runtime strings and measuring the result"
  (doc    "`(String.concat a b)` of two runtime string parameters yields a runtime string whose byte
           length is the sum of the operands' — `(join \"foo\" \"bar\")` is \"foobar\", byte length 6.
           Pins runtime string concatenation (how a compiler assembles a name or a diagnostic from
           fragments), agreeing with `(+ (byte-len a) (byte-len b))` when neither operand is empty.")
  (input   (do
             (def (join a b) (String.byte-len (String.concat a b)))
             (def (main)     (join "foo" "bar")) (export main)))
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
  (input   (do
             (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
             (def (main)    (String.byte-len (rep "" 3))) (export main)))
  (output  (: 3 Int64)))

(case "the byte length of a runtime string equals the length of its encoded bytes"
  (doc    "The runtime companion of the const `byte-len`/`to-bytes` agreement: for a runtime string `s`,
           `(String.byte-len s)` MUST equal `(Bytes.len (String.to-bytes s))` — the direct byte count and
           the encode-then-measure path are one number. `\"café\"` has byte length 5 (é is two bytes), so
           this is true. Pins that a runtime String IS its UTF-8 bytes (String.to-bytes is the identity on
           the underlying representation), the invariant the Bytes-backed String realization rests on.")
  (input   (do
             (def (agree s) (= (String.byte-len s) (Bytes.len (String.to-bytes s))))
             (def (main)    (agree "café")) (export main)))
  (output  (: true Bool)))

(case "String.to-bytes of a genuinely-runtime string yields its UTF-8 byte length"
  (doc    "`String.to-bytes` on a value that is NOT a compile-time-visible constant string — here forced
           runtime by `(String.concat s \"\")`, the same shape `String.at`'s runtime cases use — must emit
           the runtime encoding, not decline. A String IS a UTF-8 Bytes leaf, so the encoding is total: it
           materializes the string's byte-rope into a canonical flat Bytes leaf (the runtime `bytes-compact`
           op — the exact inverse of the runtime `str-from-bytes` decode). `\"café\"` is 5 UTF-8 bytes (é is
           two). Pins the runtime `String.to-bytes` path the compiler-in-Cadenza codec ENCODER rests on —
           previously declined 'not yet computed (constant strings only)'.")
  (input   (do
             (def (enc s)  (Bytes.len (String.to-bytes (String.concat s ""))))
             (def (main)   (enc "café")) (export main)))
  (output  (: 5 Int64)))

(case "String.to-bytes of a runtime string threaded through recursion round-trips its bytes"
  (doc    "The serializer shape: a String payload threaded through a recursive encoder is genuinely runtime,
           so `String.to-bytes(n)` takes the runtime path. Encodes `Name(\"foo\")` as a tag byte (2) then the
           byte-length prefix then the UTF-8 bytes — `1 + 1 + 3 = 5` bytes total. This is the exact shape the
           compiler-ml codec's encode.cdz takes (a Name's UTF-8 written via String.to-bytes), the round-trip
           blocker this op removes.")
  (input   (do
             (def (b1 x)        (Bytes.of (list (UInt8.wrap x))))
             (def (str-payload s) (Bytes.concat (b1 (String.byte-len s)) (String.to-bytes s)))
             (def (main)        (Bytes.len (Bytes.concat (b1 2) (str-payload (String.concat "foo" ""))))) (export main)))
  (output  (: 5 Int64)))

(case "a byte of a runtime string's encoding is its exact UTF-8 value, not merely the right count"
  (doc    "Content, not just length: index the runtime `String.to-bytes` result to read a specific UTF-8
           byte. `\"café\"` encodes to `99 97 102 195 169` (é = C3 A9); byte 4 is `169` (0xA9, the trailing
           byte of é). Forced runtime by `(String.concat s \"\")`. Pins that the runtime `bytes-compact`
           flatten PRESERVES the byte content — a rope's node `raw` holds header bytes, not content, so a
           flatten that read the header would give the wrong byte. Reads via `Bytes.at` (→ `Option Int64`),
           avoiding runtime-Bytes value-equality (a separate unimplemented compound heap-walk).")
  (input   (do
             (def (enc s)  (String.to-bytes (String.concat s "")))
             (def (main)   (Option.expect (Bytes.at (enc "café") 4) "in range")) (export main)))
  (output  (: 169 Int64)))

(case "two runtime Bytes values of equal content compare equal (rope vs flat)"
  (doc    "Runtime `Bytes` value-equality (`=`): a `Bytes.concat` builds a ROPE whose physical node bytes
           differ from a flat leaf of IDENTICAL content, so the tagless `champ_eq` walk would compare them
           UNEQUAL unless the rope is flattened first. The `value-eq` emit `bytes-compact`s every direct
           Bytes operand before the compare (the byte twin of the String-operand compaction), so a
           `Bytes.concat` rope `[104,105]` compares EQUAL to the flat literal `Bytes.of [104,105]` → true.
           Pins the DIRECT-operand runtime Bytes `=` — was declined 'comparison of a compound value needs a
           heap walk'.")
  (input   (do
             (def (rope)   (Bytes.concat (Bytes.of (list 104)) (Bytes.of (list 105))))
             (def (main)   (= (rope) (Bytes.of (list 104 105)))) (export main)))
  (output  (: true Bool)))

(case "runtime Bytes value-equality distinguishes different content"
  (doc    "The negative companion: two runtime Bytes of DIFFERENT content compare `false` (not merely a
           physical-identity check). A `Bytes.concat` rope `[104,105]` is NOT equal to `Bytes.of [104,106]`.
           Confirms the `value-eq` compare is genuinely structural over the flattened bytes, not a trivial
           always-true / handle-identity compare.")
  (input   (do
             (def (rope)   (Bytes.concat (Bytes.of (list 104)) (Bytes.of (list 105))))
             (def (main)   (= (rope) (Bytes.of (list 104 106)))) (export main)))
  (output  (: false Bool)))

(case "the runtime UTF-8 encoding of a string equals its exact byte literal"
  (doc    "The round-trip the compiler-in-Cadenza codec rests on: `String.to-bytes` of a genuinely-runtime
           string (forced by `String.concat s \"\"`) compares EQUAL to the exact UTF-8 `Bytes.of` literal.
           `\"é😀\"` = `195 169 240 159 152 128` (é = C3 A9, 😀 = F0 9F 98 80). The to-bytes result is itself
           a `bytes-compact`ed flat leaf and the `value-eq` emit compacts the literal operand too, so the
           structural compare is exact → true. This is the case that DECLINED before direct-Bytes `=`.")
  (input   (do
             (def (enc s)  (String.to-bytes (String.concat s "")))
             (def (main)   (= (enc "é😀") (Bytes.of (list 195 169 240 159 152 128)))) (export main)))
  (output  (: true Bool)))

(case "the scalar length of a runtime multi-byte string counts scalars, not bytes"
  (doc    "`String.scalar-len` of a runtime string counts Unicode scalar values, not UTF-8 bytes:
           `(slen \"café\")` is 4 (c, a, f, é) even though the byte length is 5. Pins that scalar length
           on a runtime value agrees with the const `chars().count()` — the runtime counts the UTF-8
           leading bytes (those not of the form 10xxxxxx), which for well-formed UTF-8 is the scalar
           count (collections-and-text.md #A String Offers Both A Scalar Length And A Byte Length).")
  (input   (do
             (def (slen s) (String.scalar-len s))
             (def (main)   (slen "café")) (export main)))
  (output  (: 4 Int64)))

(case "a runtime string is indexed by scalar and the extracted scalar is returned"
  (doc    "`String.at` on a RUNTIME string — the reader's scalar cursor — reads the i-th Unicode scalar
           as a one-scalar string, fallibly. `(at (concat s \"\") 3)` on \"café\" reads scalar 3 (é,
           which occupies UTF-8 bytes 3–4), returning `(Some \"é\")` — indexed by SCALAR, not byte, so
           the multi-byte é comes back whole. Pins runtime `String.at`: the seed walks the UTF-8 buffer
           to the scalar's byte span and slices it (a String is a Bytes-backed leaf), matching the const
           `chars().nth`. The concat forces a runtime value (a bare literal would const-fold).")
  (input   (do
             (def (at s i) (String.at (String.concat s "") i))
             (def (main)   (at "café" 3)) (export main)))
  (output  (: (Some "é") (Option String))))

(case "indexing a runtime string past its last scalar yields None"
  (doc    "The fallible companion: `String.at` at a scalar index at or beyond the string's scalar length
           yields `(None unit)`, never a trap (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping). `(at \"hi\" 5)` on the two-scalar \"hi\" is out of range → None. Pins that a
           runtime `String.at` out-of-bounds is a handled absence — the branch a reader takes at
           end-of-input.")
  (input   (do
             (def (at s i) (String.at (String.concat s "") i))
             (def (main)   (match (at "hi" 5) ((Some c) (String.byte-len c)) ((None _) -1))) (export main)))
  (output  (: -1 Int64)))

(case "a runtime string returned across the run boundary renders as its quoted text"
  (doc    "A string BUILT at run time and returned as `main`'s value crosses the boundary as its proper
           String type and renders as the quoted canonical text — `(join \"hel\" \"lo\")` is
           \"hello\". This exercises the compiler-emitted, type-directed string renderer (the analogue
           of the `b\"…\"` Bytes renderer): a runtime String is walked byte-by-byte and quoted/escaped,
           byte-identical to the const `\"…\"` form. Pins RETURNING a runtime string (distinct from the
           cases above, which consume one to a scalar) — the compound-value output ABI for strings.")
  (input   (do
             (def (join a b) (String.concat a b))
             (def (main)     (join "hel" "lo")) (export main)))
  (output  (: "hello" String)))

(case "a returned runtime string with a multi-byte scalar renders the scalar verbatim"
  (doc    "The rendered form of a runtime string passes a printable Unicode scalar through verbatim, not
           as an escape — `(id \"café\")` renders \"café\" (the é is its raw two-byte UTF-8, matching the
           const path's `{:?}`, which prints printable Unicode literally). Pins that the emitted string
           renderer's escaping agrees with the const renderer on multi-byte scalars, so a rendered string
           reads back to the same value.")
  (input   (do
             (def (id s) (if true s s))
             (def (main) (id "café")) (export main)))
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
  (input   (do
             (def (main) (Option.expect (String.from-bytes (Bytes.of (list 97 7 98))) "well-formed")) (export main)))
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
; partialities a type cannot surface, not the first resort). The
; `from-bytes`/`utf8`-segment decode is realized with the `bin` form (options/realized-capability-set/),
; so a generation without binary matching declines it.

(case "decoding well-formed UTF-8 bytes yields the string"
  (doc    "`(String.from-bytes (Bytes.of (list 99 97 102 195 169)))` decodes the UTF-8 bytes of \"café\"
           (c a f, then é as the two bytes 0xC3 0xA9 = 195 169) to `(Some \"café\")`. Pins that a
           well-formed byte sequence decodes to `(Some s)` — the success arm of the total decode
           (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping).")
  (input  (= (String.from-bytes (Bytes.of (list 99 97 102 195 169))) (Some "café")))
  (output (: true Bool)))

(case "decoding ill-formed UTF-8 bytes yields none, not a trap"
  (doc    "`(String.from-bytes (Bytes.of (list 255)))` is given 0xFF, which is not a well-formed UTF-8
           sequence (0xFF never appears in valid UTF-8), so the decode yields `None` — NOT a trap and NOT
           an unspecified string with a replacement character. Pins the failure arm as an ordinary value
           the program handles (collections-and-text.md #Decoding Bytes To A String Is Total, Not
           Trapping). This is the whole point of the total decode: ill-formed input is data, not a halt.")
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
  (input  (= (String.from-bytes (Bytes.of (list 237 160 128))) None))
  (output (: true Bool)))

(case "decoding a four-byte sequence for a code point above U+10FFFF yields none"
  (doc    "`(Bytes.of (list 244 144 128 128))` is `F4 90 80 80` — a structurally-valid 4-byte UTF-8 shape
           `11110xxx 10xxxxxx 10xxxxxx 10xxxxxx` whose decoded code point is U+110000, ONE PAST the maximum
           Unicode scalar U+10FFFF. Strict UTF-8 rejects it: the highest well-formed 4-byte lead is `F4`
           with a second byte at most `8F` (U+10FFFF), so `90` overflows the range. `String.from-bytes`
           yields `None`. The fourth failure mode of the total decode alongside invalid bytes, overlong
           encodings, and surrogates — a byte sequence whose STRUCTURE is valid but whose CODE POINT is out
           of range (the decode companion of the `Char.from-int 1114112` = U+110000 rejection). Pins that
           the validator checks the decoded scalar's range, not only the byte structure.")
  (input  (= (String.from-bytes (Bytes.of (list 244 144 128 128))) None))
  (output (: true Bool)))

(case "encoding a decoded string round-trips to the same bytes"
  (doc    "For well-formed bytes `b`, decoding then re-encoding yields `b`: matching the `(Some s)` arm of
           `(String.from-bytes b)` and taking `(String.to-bytes s)` gives back the original UTF-8 bytes.
           Pins encode as the inverse of decode-of-well-formed (collections-and-text.md #Decoding Bytes To
           A String Is Total, Not Trapping, 3rd sentence).")
  (input  (match (String.from-bytes (Bytes.of (list 99 97 102 195 169)))
            ((Some s) (= (String.to-bytes s) (Bytes.of (list 99 97 102 195 169))))
            ((None _) false)))
  (output (: true Bool)))

(case "decoding an encoded string round-trips to the same bytes (the inverse direction)"
  (doc    "The INVERSE round-trip of the case above: for a String `s`, ENCODING then DECODING then
           re-encoding yields `s`'s bytes. `(String.to-bytes \"café\")` is the 5-byte UTF-8 `[99 97 102 195
           169]` (é is the 2-byte `C3 A9`); `String.from-bytes` decodes those well-formed bytes back to a
           `Some s'`, and `(String.to-bytes s')` re-encodes to the SAME 5 bytes. Pins that decode is the
           inverse of encode-of-a-string (collections-and-text.md #Decoding Bytes To A String Is Total — the
           bijection in the other direction from the decode-then-encode case above), the shape the compiler
           takes encoding an export name to UTF-8 and reading it back.")
  (input  (match (String.from-bytes (String.to-bytes "café"))
            ((Some s) (= (String.to-bytes s) (Bytes.of (list 99 97 102 195 169))))
            ((None _) false)))
  (output (: true Bool)))

(case "an encode-decode round-trip preserves an ASCII string's byte length"
  (doc    "The length face of the inverse round-trip: `(String.from-bytes (String.to-bytes \"hi\"))` decodes
           the encoded bytes back to a `Some s'` whose `String.byte-len` is 2 — the original length. Pins
           that a to-bytes→from-bytes round-trip yields a usable String of the right size (measured, not
           `=`-compared), the length companion of the byte-preservation case above.")
  (input  (do
            (def (main) (match (String.from-bytes (String.to-bytes "hi")) ((Some s) (String.byte-len s)) (None -1)))
            (export main)))
  (output (: 2 Int64)))

(case "String.from-bytes decodes a RUNTIME byte sequence built by a recursive appender"
  (doc    "The runtime-Bytes decode path: `String.from-bytes` of a byte buffer the compiler CANNOT fold to
           a constant — here `(rep b\"\\x68\" 3)` recursively appends the byte `0x69` ('i') three times to a
           leading `0x68` ('h'), building the rope `\"hiii\"` at run time. A constant `(Bytes.of …)` folds via
           `std::str::from_utf8` in the compiler; a genuinely runtime buffer instead lowers to the runtime
           `str-from-bytes` op (strict UTF-8 validation + a zero-copy re-tag of the validated buffer as a
           String — a String IS a UTF-8 Bytes leaf). Pins that a runtime-computed Bytes decodes to the same
           `Some s` the constant fold would (collections-and-text.md #Decoding Bytes To A String Is Total,
           Not Trapping), the shape a self-hosted reader materializes an interned name with.")
  (input  (do
            (def (rep (: acc Bytes) (: n Int64))
              (if (= n 0) acc (rep (Bytes.concat acc b"\x69") (- n 1))))
            (def (main) (Option.expect (String.from-bytes (rep b"\x68" 3)) "well-formed"))
            (export main)))
  (output (: "hiii" String)))

(case "String.from-bytes of an ill-formed RUNTIME byte sequence yields None (never traps)"
  (doc    "The ill-formed runtime companion: a byte buffer built at run time (a recursive appender of the
           invalid lead byte `0xFF`, which the compiler cannot fold) is NOT well-formed UTF-8, so the runtime
           `str-from-bytes` op returns the NULL sentinel and the compiler builds `(None unit)` — the helper's
           `None` arm returns -1. A TOTAL decode, never a trap, on the RUNTIME path exactly as on the constant
           path (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). Pins that the
           runtime UTF-8 validator (matching `std::str::from_utf8`) drives the fallible decode's `None` for a
           value the compiler could not classify at compile time.")
  (input  (do
            (def (rep (: acc Bytes) (: n Int64))
              (if (= n 0) acc (rep (Bytes.concat acc b"\xff") (- n 1))))
            (def (main) (match (String.from-bytes (rep b"" 2))
                          ((Some s) (String.byte-len s))
                          ((None _) -1)))
            (export main)))
  (output (: -1 Int64)))

(case "String.from-bytes decodes a multibyte RUNTIME Bytes ROPE, flattening before validation"
  (doc    "Exercises `str-from-bytes` on a runtime Bytes ROPE (a `Bytes.concat` tree) whose logical bytes
           span multiple leaves — the op FLATTENS the rope before strict UTF-8 validation, because a rope
           node's raw storage holds header bytes, not content. `(Bytes.concat b\"caf\" b\"\\xc3\\xa9\")` built
           through a recursive appender is the 5-byte UTF-8 of \"café\" (é is the 2-byte `C3 A9`) split across
           rope leaves; decoding it yields `Some s` whose `String.byte-len` is 5. Pins that the runtime
           decode sees the actual content of a shared/spliced buffer, not header bytes (the shape the reader
           hits decoding a name sliced out of a larger input buffer).")
  (input  (do
            (def (build (: acc Bytes) (: n Int64))
              (if (= n 0) acc (build (Bytes.concat acc b"\xc3\xa9") (- n 1))))
            (def (main) (match (String.from-bytes (build b"caf" 1))
                          ((Some s) (String.byte-len s))
                          ((None _) -1)))
            (export main)))
  (output (: 5 Int64)))

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
  (input  (do
            (def (dec b) (match (String.from-bytes b)
                           ((Some s) (String.byte-len s))
                           ((None _) -1)))
            (def (main) (dec (Bytes.of (list 104 105)))) (export main)))
  (output (: 2 Int64)))

(case "decoding ill-formed bytes through a helper takes the None arm"
  (doc    "The ill-formed companion: `String.from-bytes` of a RUNTIME Bytes that is not well-formed
           UTF-8 yields `(None unit)`, so the helper's `None` arm returns -1 — a TOTAL decode, never a
           trap (collections-and-text.md #Decoding Bytes To A String Is Total, Not Trapping). `(list
           255)` is a lone `0xFF`, an invalid lead byte. Pins that the runtime UTF-8 validator (emitted
           inline, matching `std::str::from_utf8` — rejecting invalid leads, overlong forms, surrogates,
           and code points > U+10FFFF) drives the fallible decode's `None`, so a reader handles
           malformed input rather than trapping on it. Companion of the well-formed case above.")
  (input  (do
            (def (dec b) (match (String.from-bytes b)
                           ((Some s) (String.byte-len s))
                           ((None _) -1)))
            (def (main) (dec (Bytes.of (list 255)))) (export main)))
  (output (: -1 Int64)))

; The helper cases above decode a FLAT byte leaf (`Bytes.of (list …)`). A `String.from-bytes` over a
; ROPE input — a `Bytes.concat` tree whose `raw` holds header bytes, NOT content — exercises a distinct
; runtime path: `op_str_from_bytes` must `bytes_flatten` the rope BEFORE the strict UTF-8 validate (else
; it would validate the header bytes as UTF-8, garbage). These pin the rope-input decode (well-formed and
; ill-formed), with a runtime `UInt8.wrap`'d element so the Bytes is genuinely runtime-built (not folded).
(case "decoding a runtime ROPE Bytes as UTF-8 flattens before validating (well-formed)"
  (doc    "`String.from-bytes` of a `Bytes.concat` ROPE ([104,105] ++ [n]) with a runtime UInt8 element n:
           the op flattens the rope to its content bytes THEN validates. n=33 (`!`) → the 3 bytes \"hi!\"
           are well-formed UTF-8 → `(Some s)`, byte-len 3. Pins that a rope input decodes by CONTENT, not by
           its concat-node header bytes (which `bytes_flatten` materializes first).")
  (input  (do
            (def (mk (: n Int64)) (Bytes.concat (Bytes.of (list (UInt8.wrap 104) (UInt8.wrap 105))) (Bytes.of (list (UInt8.wrap n)))))
            (def (main (: n Int64))
              (match (String.from-bytes (mk n)) ((Some s) (String.byte-len s)) ((None _) (- 0 1))))
            (export main)))
  (call   main (: 33 Int64)) (output (: 3 Int64)))

(case "decoding a runtime ROPE Bytes that is ill-formed yields none"
  (doc    "The ill-formed rope companion: a runtime `Bytes.of (list (UInt8.wrap n) 255)` whose second byte
           is `0xFF` (an invalid UTF-8 lead) → `None` → -1. The rope/runtime-element form of the total
           decode's failure arm; the flatten-then-validate path rejects malformed content, never traps.")
  (input  (do
            (def (mk (: n Int64)) (Bytes.of (list (UInt8.wrap n) (UInt8.wrap 255))))
            (def (main (: n Int64))
              (match (String.from-bytes (mk n)) ((Some s) (String.byte-len s)) ((None _) (- 0 1))))
            (export main)))
  (call   main (: 104 Int64)) (output (: -1 Int64)))

(case "a utf8 bin segment binds a decoded string when the bytes are well-formed"
  (doc    "The `bin` pattern `(bin (u8 n) (utf8 name n))` reads a length byte n, then decodes exactly the
           next n bytes as UTF-8 into `name : String`. Against `(list 3 102 111 111)` — n=3, then the
           ASCII bytes of \"foo\" — the `utf8` segment matches and binds name = \"foo\". Pins the
           string-typed binary segment (options/binary-syntax/), the decode built into pattern matching.")
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
; `chars` is a FRESH capability the seed does not realize (distinct from the realized
; `collections`). A later generation realizes scalar access and the `Char` value form; until then the
; seed DECLINES these — they pin the contract the realization must meet.

(case "reading a string's scalar in bounds yields Some of the char"
  (doc    "Witnesses collections-and-text.md #A String's Scalars Are Addressable: `(String.scalar-at
           \"hello\" 1)` reads the scalar at scalar-position 1 — the char `#\\e` — wrapped in Some (an
           Option<Char>, the fallible read analogous to List.at and String.at). This is the operation
           that was missing: String.scalar-len counted scalars but nothing returned one.")
  (input  (String.scalar-at "hello" 1))
  (output (: (Some #\e) (Option Char))))

(case "reading a string's scalar out of bounds yields None"
  (doc    "The out-of-bounds companion: `(String.scalar-at \"hi\" 5)` reads past the end of a two-scalar
           string, so it yields None rather than trapping (collections-and-text.md #A String's Scalars
           Are Addressable — reading is total, and #Indexing And Lookup Are Fallible, Not Trapping). The
           Char analogue of the out-of-range String.at / List.at Nones.")
  (input  (String.scalar-at "hi" 5))
  (output (: (None unit) (Option Char))))

(case "reading a string's scalar addresses scalar values, not bytes"
  (doc    "`(String.scalar-at \"café\" 3)` is `(Some #\\é)`: the string is four scalar values (c, a, f,
           é) and scalar-position 3 is é — even though é occupies bytes 3–4 of the five-byte UTF-8
           encoding, so a byte offset would land mid-scalar. Pins that scalar access addresses by scalar
           value (collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), returning a
           Char — the Char companion of the scalar-indexed String.at case above.")
  (input  (String.scalar-at "café" 3))
  (output (: (Some #\é) (Option Char))))

(case "converting a char to its integer scalar value is total"
  (doc    "Witnesses collections-and-text.md #A Char Converts To And From An Integer Totally:
           `(Char.to-int #\\a)` is 97 — the Unicode scalar value (code point) of the char `a`. Total:
           every char is a scalar value that has an integer code point, so to-int never fails.")
  (input  (Char.to-int #\a))
  (output (: 97 Int64)))

(case "converting a scalar-valued integer to a char yields Some"
  (doc    "`(Char.from-int 97)` is `(Some #\\a)` — 97 is the scalar value of `a`, a valid Unicode scalar,
           so the conversion succeeds (collections-and-text.md #A Char Converts To And From An Integer
           Totally). from-int is FALLIBLE (returns an Option) because not every integer is a scalar; this
           is the success arm.")
  (input  (Char.from-int 97))
  (output (: (Some #\a) (Option Char))))

(case "converting a surrogate code point to a char yields None"
  (doc    "`(Char.from-int 55296)` — 55296 is U+D800, a HIGH SURROGATE, which is NOT a Unicode scalar
           value — so from-int yields None rather than producing an invalid char (collections-and-text.md
           #A Char Converts To And From An Integer Totally, and #A Char Is A Single Unicode Scalar Value:
           surrogates are excluded). Pins that the surrogate range is rejected as data (None), never a
           trap and never an ill-formed Char. This is why from-int must be fallible.")
  (input  (Char.from-int 55296))
  (output (: (None unit) (Option Char))))

(case "converting an out-of-range integer to a char yields None"
  (doc    "`(Char.from-int 1114112)` — 1114112 is U+110000, one past the maximum scalar U+10FFFF — so it
           is not a scalar value and from-int yields None (collections-and-text.md #A Char Converts To
           And From An Integer Totally). The high-end companion of the surrogate case; both are handled
           as data, not traps.")
  (input  (Char.from-int 1114112))
  (output (: (None unit) (Option Char))))

(case "char to-int and from-int round-trip through the scalar value"
  (doc    "For a scalar value v, `(Char.from-int v)` is `(Some c)` and `(Char.to-int c)` is v again:
           `(Char.to-int #\\a)` = 97 and `(Char.from-int 97)` = `(Some #\\a)`, so matching the Some arm
           and taking to-int returns 97. Pins from-int as the inverse of to-int on a valid scalar
           (collections-and-text.md #A Char Converts To And From An Integer Totally). MUST be true.")
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
  (input  #\u+D800)
  (error  CDZ0002))

(case "a char order agrees with its scalar value"
  (doc    "Witnesses collections-and-text.md #A Char Is A Single Unicode Scalar Value (2nd sentence: a
           char's ordering is the numeric order of its scalar value): `(< #\\a #\\b)` is true because
           the scalar value of `a` (97) is less than that of `b` (98). Pins that a Char order and the
           string order defined on scalar values agree by construction — a Char is comparable and its
           order is its scalar order.")
  (input  (< #\a #\b))
  (output (: true Bool)))

(case "two equal chars compare equal"
  (doc    "`(= #\\a #\\a)` is true — a char's value is its scalar, and two chars are equal exactly when
           their scalar values are equal. Pins Char equality as scalar equality, the equality companion
           of the Char order case.")
  (input  (= #\a #\a))
  (output (: true Bool)))

(case "two chars with different scalar values are unequal"
  (doc    "`(= #\\a #\\b)` is false — the discriminator for the equal-chars case above: Char equality is a
           genuine scalar comparison, not a blanket true. `a` (97) and `b` (98) have distinct scalar values,
           so they are unequal. Pins that Char `=` distinguishes chars (the false companion of `(= #\\a
           #\\a)`).")
  (input  (= #\a #\b))
  (output (: false Bool)))

(case "the greater-than operator on chars follows scalar order"
  (doc    "`(> #\\b #\\a)` is true — `b` (scalar 98) is greater than `a` (97), the `>` companion of the
           `(< #\\a #\\b)` order case. Pins that the strict-greater operator over Char also follows the
           scalar order (both directions of the Char order are reachable).")
  (input  (> #\b #\a))
  (output (: true Bool)))

(case "the three-way comparison orders chars by scalar value — Less"
  (doc    "`(compare #\\a #\\b)` is `(Ordering.Less unit)` — Char offers a total order (its scalar order,
           collections-and-text.md #A Char Is A Single Unicode Scalar Value), so `compare` reports it as
           the Less variant exactly as over Int64/Float64 (core-semantics.md #A Total Order Is Observed
           Through A Three-Way Comparison). Pins that the three-way comparison spans Char, the compare
           companion of the `(< #\\a #\\b)` operator case.")
  (input  (compare #\a #\b))
  (output (: (Less unit) Ordering)))

(case "the three-way comparison orders chars by scalar value — Greater"
  (doc    "`(compare #\\b #\\a)` is `(Ordering.Greater unit)` — the Greater variant over Char, so `b`
           orders after `a` by scalar value. The Greater companion of the Less case, pinning that compare's
           direction agrees with `>` on Char.")
  (input  (compare #\b #\a))
  (output (: (Greater unit) Ordering)))

(case "the three-way comparison orders chars by scalar value — Equal"
  (doc    "`(compare #\\a #\\a)` is `(Ordering.Equal unit)` — two chars of the same scalar report the middle
           variant. With the Less and Greater cases this pins all three Ordering variants are reachable over
           Char and discriminated by the scalar relation, exactly as the Int64/Float64 triples are.")
  (input  (compare #\a #\a))
  (output (: (Equal unit) Ordering)))

; --- String operations at RUN TIME: a string not fixed at compile time ---------------------------------
; The string cases above operate on CONSTANT string literals, so their lengths / slices / concatenations
; fold at compile time. A string chosen at run time — an `(if …)` selecting between two literals produces
; ONE `String` handle unified from both branches, whose identity is known only at run time — exercises the
; runtime query path: the length op reads the actual handle, not a folded constant. These pin that path,
; the string analogue of the runtime-index `List.at` and runtime-key `Map.lookup` cases. (`String.byte-len`
; and `String.at`/`String.concat` accept a runtime string; `String.scalar-len` on a runtime string and
; `String.slice` with a runtime bound are later increments the seed declines — not witnessed here.)

(case "the byte length of a run-time-selected string reads the actual handle"
  (doc    "`(String.byte-len (if b \"hello\" \"hi\"))` — the `if` selects one of two literals at run time,
           yielding one `String` handle whose length is not a compile-time constant. `b`=true → \"hello\"
           (5 bytes), `b`=false → \"hi\" (2 bytes). Pins that `String.byte-len` reads the runtime handle's
           length rather than folding a constant, the string companion of runtime `List.len`.")
  (input  (do (def (main (: b Bool)) (String.byte-len (if b "hello" "hi"))) (export main)))
  (call   main (: true Bool)) (output (: 5 Int64))
  (call   main (: false Bool)) (output (: 2 Int64)))

(case "the byte length of a run-time multibyte string exceeds its scalar length"
  (doc    "`(String.byte-len (if b \"café\" \"ab\"))` = 5 for \"café\" — the é is a 2-byte UTF-8 scalar, so
           the BYTE length (5) exceeds the 4 SCALARS (collections-and-text.md — byte length counts the
           UTF-8 encoding, not the scalars). Pins that the runtime byte-length op counts encoded bytes on a
           string whose content is decided at run time, not the scalar count.")
  (input  (do (def (main (: b Bool)) (String.byte-len (if b "café" "ab"))) (export main)))
  (call   main (: true Bool)) (output (: 5 Int64))
  (call   main (: false Bool)) (output (: 2 Int64)))

(case "a run-time-index scalar read is present in bounds and absent out of bounds"
  (doc    "`(String.at \"abc\" i)` at a run-time index reads the one-scalar substring at scalar position
           `i` — total (collections-and-text.md #A String's Scalars Are Addressable): in bounds → `(Some
           <one-scalar string>)` (i=0 → \"a\", byte-len 1), out of bounds → `None` (i=5 → -1), never a trap.
           The string companion of the runtime-index `List.at`; the index is a parameter, so the read runs
           on the value heap rather than folding.")
  (input  (do (def (main (: i Int64)) (match (String.at "abc" i) ((Some s) (String.byte-len s)) (None -1))) (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: -1 Int64)))

(case "concatenation with a run-time-selected operand joins the actual strings"
  (doc    "`(String.concat (if b \"ab\" \"abcd\") \"z\")` joins a run-time-selected left operand with a
           constant right one; the result's byte length is 3 for \"ab\"+\"z\" and 5 for \"abcd\"+\"z\". Pins
           that `String.concat` joins the ACTUAL runtime string (not a folded constant) — the total binary
           join the compiler itself uses to build messages, exercised with a non-constant operand.")
  (input  (do (def (main (: b Bool)) (String.byte-len (String.concat (if b "ab" "abcd") "z"))) (export main)))
  (call   main (: true Bool)) (output (: 3 Int64))
  (call   main (: false Bool)) (output (: 5 Int64)))

; --- A run-time-index String.at result compares by CONTENT ----------------------------------------
; `String.at` at a run-time index returns the one-scalar substring as `Some(<slice>)`, where the slice is
; a ROPE — a byte offset INTO the source string, not a flat leaf. String content-equality (`=`) and
; map/set-key hashing compare PHYSICAL bytes, so a rope slice would compare by its OFFSET and never match
; a flat twin of identical content: `(= (String.at s i) "a")` was SILENTLY false even when the char at `i`
; IS 'a' (a wrong value, valid wasm, no diagnostic), so a hand-written lexer scanning a runtime string
; char-by-char could not classify a character. The fix compacts the fresh slice to an independent flat
; leaf at the producer, so the result compares by content everywhere it is used. These pin: a runtime
; `String.at` result equals the same character obtained as a literal (and is unequal to a different one),
; and a recursive char-scan counts matching characters correctly.
(case "a run-time-index String.at result compares equal to the same character as a literal"
  (doc    "`(= (Option.expect (String.at \"abc\" i) \"c\") \"a\")` at a RUNTIME index `i`: index 0 is \"a\"
           (→ 1), index 1 is \"b\" (→ 0). The `String.at` result is a one-scalar rope slice, so its content
           equality against the literal \"a\" must compare by CONTENT, not the slice's rope offset. Before
           the producer-side slice compaction this returned 0 at index 0 — a runtime `String.at` result
           never compared equal to the same character obtained any other way (a silent wrong value; the
           wasm validates). The constant-index form folds and already compared correctly, hiding the bug.")
  (input  (do (def (main (: i Int64))
                (if (= (Option.expect (String.at "abc" i) "c") "a") 1 0)) (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: 0 Int64)))

(case "a recursive scan counts a runtime string's matching characters"
  (doc    "`count-a \"banana\"` — a recursive char-scan reading each scalar with `String.at` at a runtime
           index and counting `(= (String.at s i) \"a\")`. \"banana\" has three 'a's, so the result must be
           3. Before the fix it returned 0: each `String.at` result is a `bytes-slice` rope reached through
           `Option.expect`, and its content-equality compared by rope offset, never matching the flat
           literal \"a\". The char-by-char lexer idiom over a runtime string — a compiler-in-Cadenza
           tokenizing its input. The fix compacts the fresh slice at the producer (and `dup`s the borrowed
           source so the slice's reference is independent — the same string threads on into the recursion).")
  (input  (do
            (def (at (: s String) (: i Int64)) (Option.expect (String.at s i) "ok"))
            (def (cnt (: s String) (: i Int64) (: acc Int64))
              (if (= i (String.byte-len s)) acc
                  (cnt s (+ i 1) (if (= (at s i) "a") (+ acc 1) acc))))
            (def (main) (cnt "banana" 0 0))
            (export main)))
  (output (: 3 Int64)))

; --- Two matched String KEYS live at once across a recursion: a borrowed lookup key is not freed --------
; `Map.lookup`/`Set.contains` BORROW their key — the runtime reads it without consuming it (`champ_hash`/
; `champ_eq` over its bytes). A String key read out of a live sum-payload (a `Node`'s `String` field, bound
; by a match arm) is a BORROW the enclosing tree still owns — so the lookup emit must NOT drop it. It used
; to drop the key UNCONDITIONALLY (correct only for the OWNED-temporary key a constant/rope produces),
; freeing the borrowed key under its owner. With TWO such keys live at one node — a tree-walker consulting a
; node's OWN key AND its child's key, both String sum-payload projections looked up in a `Map String Int64`
; — the second lookup freed a key the first still needed, corrupting a comparison and dropping a per-node
; decision: a SILENT WRONG COUNT (valid wasm, `cdz check` clean, no diagnostic). The trigger needs the two
; keys live SIMULTANEOUSLY across a recursion ≥3 deep; the IDENTICAL tree with Int64 keys (no heap key, no
; borrow) computes correctly, pinning the tree/recursion logic. Same ownership root as the runtime-String
; content-equality and String.at families above; the fix gates the key drop on OWNED-vs-BORROWED (a boxed
; scalar / compacted rope / fresh owned compound is dropped, a borrowed param/local/projection is not).
(case "a recursive walk consulting a node's own and its child's String key computes correctly"
  (doc    "A binary tree whose `Node`s carry a `String` operator key; `pc` counts nodes whose left child
           binds LOOSER — `(< (top l) (pv op))`, comparing the LEFT CHILD's key precedence `(top l)` to the
           node's OWN key precedence `(pv op)` (both looked up in a `Map String Int64`). On the nested tree
           `c{ b{ a{L,L}, L }, L }` the count is 2 (c: top(b)=2 < 3 → 1; b: top(a)=1 < 2 → 1; a: 99 < 1 →
           0). It used to return 1 — with TWO matched `String` sum-payloads (the node's own key and its
           child's) live at once across a recursion ≥3 deep, the second borrowed lookup key was freed under
           its owner and its comparison flipped (a silent wrong value; the wasm validates). The IDENTICAL
           tree with Int64 keys returns 2 (pinning the oracle and that the tree/recursion logic is right).
           `Map.lookup` BORROWS its key, so a key read out of a still-live node is left to its owner, not
           dropped — the pretty-printer precedence-parenthesization idiom over a runtime expression tree.")
  (input  (do
            (type T (Leaf Int64) (Node String T T))
            (def (pv (: op String))
              (match (Map.lookup (Map.insert (Map.insert (Map.insert (map) "a" 1) "b" 2) "c" 3) op)
                (((. Option Some) p) p)
                (((. Option None) _) 0)))
            (def (top (: t T)) (match t (((. T Leaf) _) 99) (((. T Node) op _ _) (pv op))))
            (def (pc (: t T))
              (match t
                (((. T Leaf) _) 0)
                (((. T Node) op l r) (+ (if (< (top l) (pv op)) 1 0) (+ (pc l) (pc r))))))
            (def (main (: d Int64))
              (pc ((. T Node) "c"
                    ((. T Node) "b" ((. T Node) "a" ((. T Leaf) 0) ((. T Leaf) 0)) ((. T Leaf) 0))
                    ((. T Leaf) 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

; --- A rope String NESTED IN A COMPOUND compares/keys by CONTENT (v-runtime) ----------------------
; The top-level `=`/map-key compaction (above) canonicalizes a rope String when the string IS the
; operand/key. But the value heap is TAGLESS: `champ_eq`/`champ_hash`'s structural walk compares a
; nested leaf by its PHYSICAL raw bytes and cannot tell a rope-of-bytes child from a compound child, so
; a rope String NESTED in a tuple/record/sum-payload/map-key compared UNEQUAL to its flat twin of equal
; content (a silent wrong value; valid wasm, no diagnostic) — and a compound map key containing an
; assembled string could not be looked up by its literal twin (silent None). The fix canonicalizes a
; String/Bytes leaf with `bytes-compact` AT EACH COMPOUND CONSTRUCTION SITE — the nested-leaf twin of
; `box-float`'s NaN normalize-on-construct — so no compound ever holds a rope and the tagless walk's
; physical compare is exact. `rep s n` builds a rope by `n` concats (rope, content "hi"+n×"x"); one
; concat suffices. Controls pin that a FLAT nested runtime string, identically-built ropes both sides,
; and folded constants were never affected — it is the ROPE-in-a-compound face exactly.
(case "a one-concat rope nested in a tuple equals its flat twin"
  (doc    "`(= (tuple (rep \"hi\" 1) 1) (tuple \"hix\" 1))` — the left tuple's string element is a runtime
           ROPE (one `String.concat`, content \"hix\"), the right's is the flat literal \"hix\". Structural
           equality compares component-wise and string equality is by content, so the tuples are equal → 1.
           Before the construction-site compaction the walk compared the nested rope leaf physically (rope
           header bytes ≠ flat bytes) → 0. MINIMAL: one concat. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (tuple (rep "hi" 1) 1) (tuple "hix" 1)) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a rope in an Option payload equals its flat-twin payload"
  (doc    "`(= (Option.Some (rep \"hi\" 3)) (Option.Some \"hixxx\"))` — tags match (both Some), payloads are
           content-equal strings (rope vs flat \"hixxx\") → true → 1. The sum-payload face of the nested-rope
           miss; the float twin (`(= (Some Float64.nan) (Some Float64.nan))`) already passed because a NaN is
           canonicalized when its leaf is boxed — a String leaf needs the same treatment. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (Option.Some (rep "hi" 3)) (Option.Some "hixxx")) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a rope in a record field equals its flat-twin field"
  (doc    "`(= (record (f (rep \"hi\" 3)) (g 1)) (record (f \"hixxx\") (g 1)))` — same field set, field `g`
           equal, field `f` content-equal strings (rope vs flat) → true → 1. A record IS a tuple at run
           time, so the same nested-rope face as the tuple case, keyed by field. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (record (f (rep "hi" 3)) (g 1)) (record (f "hixxx") (g 1))) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a compound map key containing a rope is found by its flat-twin key"
  (doc    "`(Map.insert Map.empty (tuple (rep \"hi\" 3) 1) 42)` keys the map by a TUPLE whose string element
           is a runtime rope (content \"hixxx\"); `(Map.lookup … (tuple \"hixxx\" 1))` looks up with the
           flat-twin tuple. Equal keys → must find 42. Before the construction-site compaction the tuple key
           was CHAMP-hashed with its nested rope leaf uncompacted, landing in a different slot than the
           flat-twin query key → None (→ -1). The idiomatic \"key a map by (name, arity) where name was
           assembled by concat\". Expected: 42.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert Map.empty (tuple (rep "hi" 3) 1) 42) (tuple "hixxx" 1))
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

(case "a compound map key whose LIST element is a rope is found by its flat-twin key"
  (doc    "`(Map.insert Map.empty (list (rep \"hi\" 3)) 42)` keys the map by a single-element LIST whose
           element is a runtime rope (content \"hixxx\"); `(Map.lookup … (list \"hixxx\"))` looks up with the
           flat-twin list. A list element is stored on the value heap exactly like a tuple element, so the
           nested-rope face reaches a list too; construction-site compaction canonicalizes the element leaf,
           so the key hashes into the same CHAMP slot as its flat twin → 42 (before the fix: None → -1). (A
           direct `=` on two lists is a separate, not-yet-built compare; the map-KEY path exercises the
           list-element compaction here.) Expected: 42.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert Map.empty (list (rep "hi" 3)) 42) (list "hixxx"))
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

(case "a FLAT runtime string nested in a tuple is unaffected (control)"
  (doc    "`(= (tuple (rep \"hi\" 0) 1) (tuple \"hi\" 1))` — `rep \"hi\" 0` returns the source string with NO
           concat, so the nested element is a FLAT runtime leaf, not a rope. It compared equal to its flat
           twin before AND after the fix — isolating the bug to the ROPE case, not runtime-ness. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (tuple (rep "hi" 0) 1) (tuple "hi" 1)) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "identically-built ropes on both sides of a nested compare are equal (control)"
  (doc    "`(= (tuple (rep \"hi\" 3) 1) (tuple (rep \"hi\" 3) 1))` — both nested elements are ropes built the
           SAME way, so their physical shapes matched and this compared equal even BEFORE the fix (the
           physical compare happened to agree). Pins that the fix does not disturb the already-equal case.
           Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (tuple (rep "hi" 3) 1) (tuple (rep "hi" 3) 1)) 1 0))
            (export main)))
  (output (: 1 Int64)))

; The nested-rope construction-site compaction (above) RECURSES: a rope nested TWO levels deep is
; canonicalized because each construction site compacts its String children AND the inner compound is
; built before the outer, so no compound at any depth ever holds a rope. These pin depth-2 (tuple-in-
; tuple, sum-in-record, doubly-nested map key) — the shapes a real value tree (a compiler AST node's
; fields) reaches; they held once the leaf-level fix landed, so they guard that the recursion isn't
; later narrowed to depth 1.
(case "a rope two levels deep (tuple in a tuple) equals its flat twin"
  (doc    "`(= (tuple (tuple (rep \"hi\" 3) 1) 2) (tuple (tuple \"hixxx\" 1) 2))` — the rope is the string
           element of the INNER tuple, itself an element of the outer tuple. Both tuples' string leaves are
           compacted at their construction sites (inner built first), so the structural walk compares by
           content at depth 2 → 1. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (tuple (tuple (rep "hi" 3) 1) 2) (tuple (tuple "hixxx" 1) 2)) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a rope in a sum payload inside a record field equals its flat twin"
  (doc    "`(= (record (f (Option.Some (rep \"hi\" 3))) (g 1)) (record (f (Option.Some \"hixxx\")) (g 1)))` —
           the rope sits in an `Option.Some` payload that is itself a record field. The sum payload compacts
           at its construction, the record stores the (already-canonical) sum handle → content-equal → 1.
           Mixes the sum-payload and record-field faces at depth 2. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (if (= (record (f (Option.Some (rep "hi" 3))) (g 1)) (record (f (Option.Some "hixxx")) (g 1))) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a doubly-nested tuple map key containing a rope is found by its flat twin"
  (doc    "The depth-2 compound-KEY face: a map keyed by `(tuple (tuple (rep \"hi\" 3) 1) 2)` (a rope nested
           two levels deep in the key) is looked up with the flat-twin key → 42. Every construction site on
           the key path compacts its string leaf, so the whole key hashes canonically → the flat-twin query
           lands in the same CHAMP slot. Expected: 42.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert Map.empty (tuple (tuple (rep "hi" 3) 1) 2) 42) (tuple (tuple "hixxx" 1) 2))
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

; --- Runtime String.from-bytes: the UTF-8 validation-boundary edges --------------------------------
; The runtime decode cases above pin a valid appender rope, an all-invalid buffer (0xFF), and a
; flattened multibyte rope. These pin the VALIDATION BOUNDARY precisely — the malformed classes the
; strict validator must reject even though every byte is individually plausible, and the well-formed
; two-byte sequence it must accept, over Bytes whose elements arrive as runtime UInt8 wraps.

(case "String.from-bytes rejects a lone continuation byte"
  (doc    "A single byte 0x80 — a CONTINUATION byte with no lead — is not well-formed UTF-8, so the
           total decode yields None → -1. Distinct from the 0xFF invalid-LEAD case above: 0x80 is a
           byte that IS valid inside a multibyte sequence, just not at the start — a validator that
           only rejected never-valid bytes (0xFE/0xFF) accepts it.")
  (input  (do
            (def (main (: a Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap a))))
                ((Some s) (String.byte-len s))
                ((None _) -1)))
            (export main)))
  (call   main (: 128 Int64))
  (output (: -1 Int64)))

(case "String.from-bytes rejects an overlong encoding"
  (doc    "`[0xC0 0x80]` — the OVERLONG encoding of NUL (a 2-byte form of a value that must be 1
           byte). Each byte is structurally plausible (a 2-byte lead followed by a continuation), so
           a validator that only checked lead/continuation SHAPE accepts it; strict UTF-8
           (`std::str::from_utf8` semantics, which the runtime op pins itself to) rejects overlongs →
           None → -1. The classic smuggling vector for security filters — worth its own pin.")
  (input  (do
            (def (main (: a Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap a) (UInt8.wrap 128))))
                ((Some s) (String.byte-len s))
                ((None _) -1)))
            (export main)))
  (call   main (: 192 Int64))
  (output (: -1 Int64)))

(case "String.from-bytes accepts a two-byte sequence split across a concat seam"
  (doc    "The lead byte 0xC3 and its continuation 0xA9 (é) arrive in SEPARATE ropes joined by
           `Bytes.concat` — the multibyte sequence exists only in the FLATTENED buffer. The decode
           accepts it (byte-len 2), pinning that validation runs over the flattened bytes, not
           per-leaf (a per-leaf validator sees a dangling lead in one leaf and a lone continuation in
           the other and wrongly rejects a well-formed string).")
  (input  (do
            (def (main (: a Int64))
              (match (String.from-bytes (Bytes.concat (Bytes.of (list (UInt8.wrap a))) (Bytes.of (list (UInt8.wrap 169)))))
                ((Some s) (String.byte-len s))
                ((None _) -1)))
            (export main)))
  (call   main (: 195 Int64))
  (output (: 2 Int64)))

(case "a String param threaded through a self-recursive loop and concatenated each step is retained"
  (doc    "The idiomatic pretty-printer shape: `build(s, n, acc) = build(s, n-1, String.concat(acc, s))`
           — a String PARAM `s` threaded UNCHANGED through the recursion AND consumed by `String.concat`
           each step. Regression: `s` is consumed by `String.concat` (rc--) yet re-passed to the self-call,
           so the shared ROPE was freed while still referenced and the rope walk read OUT OF BOUNDS past a
           depth threshold — `cdz check` clean, `cdz compile` ok, RAN → wasm trap at n≥4. ROOT: `is_heap_
           type` (the Perceus retain-candidate gate) included `Bytes` but NOT `String` — though a String is
           a heap rope exactly as Bytes is — so `s` was never a retain candidate and no `dup` was emitted
           (the List ops worked because `List` WAS in `is_heap_type`). The fix adds `String`/`Symbol`. Now
           `s` is duped before the consuming concat and the threaded copy stays live. `build(\"x\", 8, \"\")`
           → an 8-char string → byte-len 8.")
  (input  (do
            (def (build (: s String) (: n Int64) (: acc String))
              (if (= n 0) acc (build s (- n 1) (String.concat acc s))))
            (def (run (: n Int64)) (String.byte-len (build "x" n "")))
            (export run)))
  (call   run (: 8 Int64))
  (output (: 8 Int64)))

; --- The threaded-String-param retain: the consumption-shape faces ----------------------------------
; e38228f35 added String/Symbol to the Perceus retain-candidate gate (a param threaded through a
; self-recursive loop AND consumed by concat each step was freed while referenced — an OOB trap past
; depth 4; its pin covers the accumulate-into-acc shape). These pin the sibling consumption shapes,
; promoted from passing breaker probes.

(case "a threaded String param consumed twice per step survives the loop"
  (doc    "`go(s, n, acc) = go(s, n-1, acc + byte-len(concat s s))` — the threaded `s` is consumed
           TWICE per iteration (both concat operands) and re-passed: each step adds 4 (\"abab\"),
           n = 5 → 20. The double-consume face needs TWO retains per step; an off-by-one frees the
           rope on the second consume and re-trips the OOB walk the fix closed.")
  (input  (do
            (def (go (: s String) (: n Int64) (: acc Int64))
              (if (= n 0) acc
                  (go s (- n 1) (+ acc (String.byte-len (String.concat s s))))))
            (def (main (: n Int64))
              (go "ab" n 0))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 20 Int64)))

(case "a threaded String param consumed as the right concat operand survives the loop"
  (doc    "`(String.concat \"k\" s)` — the threaded param is the RIGHT operand (the fix's shape
           consumes it as acc's appendee on the left path): each step adds byte-len(\"k\"+\"abc\") = 4,
           n = 6 → 24. The operand-position face of the retain (a consume-site scan keyed to one
           operand slot misses the other).")
  (input  (do
            (def (go (: s String) (: n Int64) (: acc Int64))
              (if (= n 0) acc
                  (go s (- n 1) (+ acc (String.byte-len (String.concat "k" s))))))
            (def (main (: n Int64))
              (go "abc" n 0))
            (export main)))
  (call   main (: 6 Int64))
  (output (: 24 Int64)))

(case "a runtime string pattern dispatches by content (value-eq compare)"
  (doc    "A match with STRING-LITERAL arms over a RUNTIME String value dispatches by CONTENT — the
           `Str`-probe LitTest emits a `value-eq` (`champ_eq`) compare against the literal, `bytes-compact`ing
           the leaf to canonical flat form first so a rope and its flat twin compare equal. Before, a runtime
           string pattern DECLINED ('a string pattern over a runtime payload is not yet supported (only a
           constant folds)'). The scrutinee is a `String.concat` result — a genuine runtime ROPE, not a
           constant that folds — so this exercises the runtime path. `classify (\"a\"+\"b\") = \"ab\"` selects the
           first arm → 1. A generation that skipped this would decline every runtime string match (an
           interpreter dispatching on a keyword, a lexer classifying a token).")
  (input  (do (def (classify (: s String)) (match s ("ab" 1) ("cd" 2) (_ 0)))
              (def (main) (classify (String.concat "a" "b")))
              (export main)))
  (output (: 1 Int64)))

(case "a runtime string pattern falls through to the wildcard when no literal matches"
  (doc    "The non-match dual of the runtime string dispatch: a rope `\"xy\"` matching neither `\"ab\"` nor
           `\"cd\"` falls through to the wildcard → 0. Confirms each `value-eq` arm genuinely runs (a real
           content compare that can FAIL), not a blind first-arm fold. Paired with the positive case, pins
           that a runtime string match is a correct content-dispatch, not a decline and not a mis-match.")
  (input  (do (def (classify (: s String)) (match s ("ab" 1) ("cd" 2) (_ 0)))
              (def (main) (classify (String.concat "x" "y")))
              (export main)))
  (output (: 0 Int64)))
