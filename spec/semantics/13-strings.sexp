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

(case "a runtime string orders by content-lexicographic byte order"
  (doc    "Runtime String ORDERING (`<`): `(rep \"app\" …)` builds genuinely-runtime strings whose content is
           compared content-lexicographically. `\"apple\" < \"applf\"` — same first four bytes, then `e`(0x65)
           < `f`(0x66) → the first differing byte decides → true → 1. A runtime String is a UTF-8 byte leaf
           and its blessed total order is the content-lexicographic byte order (core-semantics.md #Compound
           Ordering Is Lexicographic); the seed walks both leaves' bytes (`bytes-get`/`bytes-len`) rather than
           declining. The concat forces a runtime value (a bare literal would const-fold the compare).")
  (input  (do
            (def (mk (: s String)) (String.concat s ""))
            (def (main) (if (< (mk "apple") (mk "applf")) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "runtime string ordering makes a proper prefix less than its extension"
  (doc    "The prefix rule: `\"app\" < \"apple\"` → true → 1. The two share every byte of the shorter string,
           so no byte differs within the common length; a list/string that is a PROPER PREFIX of another
           compares LESS than the longer one (core-semantics.md #Compound Ordering Is Lexicographic —
           shorter-is-less on a common prefix). Pins the length tiebreak of the runtime byte-lexicographic
           walk, distinct from the first-differing-byte case above.")
  (input  (do
            (def (mk (: s String)) (String.concat s ""))
            (def (main) (if (< (mk "app") (mk "apple")) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "runtime string ordering compares bytes UNSIGNED (a multi-byte scalar exceeds ASCII)"
  (doc    "Content-lexicographic order compares the UTF-8 bytes as UNSIGNED values: `\"café\" > \"cafz\"`
           because at byte 3 the `é` encoding's lead byte `0xC3` (195) is GREATER than `z` = `0x7A` (122) —
           so `(< \"café\" \"cafz\")` is FALSE → 0. A SIGNED byte compare would read `0xC3` as −61 < 122 and
           wrongly answer true; pins that the walk is unsigned (which for well-formed UTF-8 makes the byte
           order agree with the Unicode scalar order). The multi-byte companion of the ASCII cases above.")
  (input  (do
            (def (mk (: s String)) (String.concat s ""))
            (def (main) (if (< (mk "café") (mk "cafz")) 1 0))
            (export main)))
  (output (: 0 Int64)))

(case "runtime string ordering surfaces all four relational operators"
  (doc    "`<=`, `>`, `>=` over runtime strings agree with `<` and with each other (one total order surfaced
           through every boolean operator, core-semantics.md #A Total Order Is Observed Through A Three-Way
           Comparison). Packs four checks: `\"app\" <= \"apple\"` (1), `\"apple\" <= \"apple\"` (1, reflexive),
           `\"banana\" > \"apple\"` (1), `\"apple\" >= \"apple\"` (1) → 1000+100+10+1 = 1111. Pins that the
           three-way order derives `<=`/`>`/`>=` consistently, including the equal case on each.")
  (input  (do
            (def (le (: a String) (: b String)) (if (<= (String.concat a "") (String.concat b "")) 1 0))
            (def (gt (: a String) (: b String)) (if (> (String.concat a "") (String.concat b "")) 1 0))
            (def (ge (: a String) (: b String)) (if (>= (String.concat a "") (String.concat b "")) 1 0))
            (def (main)
              (+ (* 1000 (le "app" "apple"))
                 (+ (* 100 (le "apple" "apple"))
                    (+ (* 10 (gt "banana" "apple")) (ge "apple" "apple")))))
            (export main)))
  (output (: 1111 Int64)))

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

; --- A runtime String `match` is OBSERVABLY EQUIVALENT to its desugared `(= s literal)` if-chain --------
; The runtime String match above "lowers to a chain of `(= s literal)` value-eq tests, the wildcard the
; tail" (its own doc). That desugaring is a backend-independent lowering choice — so a String match and the
; hand-written `if (= s "a") … else if (= s "b") … else DEFAULT` chain over the SAME scrutinee MUST compute
; the SAME value for every input, including the FALL-THROUGH (no literal matches → the wildcard / else).
; These pin the equivalence by computing BOTH forms in one program and checking they agree — a matched arm
; AND the default arm — so a lowering that reordered the arm tests, dropped the wildcard, or diverged the
; match from its `=`-chain desugaring would be caught, both backends. (A String is a heap value, so its
; runtime scrutinee is constructed inside `main` via `(if …)`, not passed as a call arg.)
(case "a runtime String match on a hit arm agrees with its desugared (= s literal) if-chain"
  (doc    "`viamatch s = (match s (\"add\" 1) (\"sub\" 2) (_ 9))` and `viachain s = (if (= s \"add\") 1 (if
           (= s \"sub\") 2 9))` are the match and its `=`-chain desugaring. On a hit (`\"add\"`, built at
           runtime via `(if true \"add\" \"sub\")` so it is not a constant): match → 1, chain → 1, combined
           `10*match + chain` = 11. Pins the two forms agree on a matched arm, both backends.")
  (input  (do
            (def (viamatch (: s String)) (match s ("add" 1) ("sub" 2) (_ 9)))
            (def (viachain (: s String)) (if (= s "add") 1 (if (= s "sub") 2 9)))
            (def (main) (+ (* 10 (viamatch (if true "add" "sub"))) (viachain (if true "add" "sub"))))
            (export main)))
  (output (: 11 Int64)))

(case "a runtime String match and its (= s literal) if-chain agree on the DEFAULT fall-through arm"
  (doc    "The fall-through case: a scrutinee matching NO literal (`\"xyz\"`, built at runtime via `(if false
           \"add\" \"xyz\")`) must take the wildcard `(_ 9)` in the match AND the trailing `else 9` in the
           chain — the arm a dropped-wildcard or diverged desugaring would get wrong. `10*viamatch + viachain`
           = 10*9 + 9 = 99. Pins match ≡ `=`-chain on the DEFAULT arm, both backends.")
  (input  (do
            (def (viamatch (: s String)) (match s ("add" 1) ("sub" 2) (_ 9)))
            (def (viachain (: s String)) (if (= s "add") 1 (if (= s "sub") 2 9)))
            (def (main) (+ (* 10 (viamatch (if false "add" "xyz"))) (viachain (if false "add" "xyz"))))
            (export main)))
  (output (: 99 Int64)))

(case "a String match with disjoint literal arms is order-independent (reordering the arms preserves the result)"
  (doc    "The runtime String match dispatches by a chain of `(= s literal)` tests; when the literal patterns
           are MUTUALLY DISJOINT, at most one can match, so the arm ORDER is immaterial — a lowering that
           reordered the probe chain (e.g. for efficiency) must give the same result. `fwd s = (match s
           (\"add\" 1) (\"sub\" 2) (\"mul\" 3) (_ 9))` and `rev s = (match s (\"mul\" 3) (\"sub\" 2) (\"add\"
           1) (_ 9))` have the SAME arms in reverse order. On `\"sub\"` (built at runtime): both select 2, so
           `10*fwd + rev` = 22. Pins arm order-independence for disjoint string literals, both backends.")
  (input  (do
            (def (fwd (: s String)) (match s ("add" 1) ("sub" 2) ("mul" 3) (_ 9)))
            (def (rev (: s String)) (match s ("mul" 3) ("sub" 2) ("add" 1) (_ 9)))
            (def (main) (+ (* 10 (fwd (if true "sub" "x"))) (rev (if true "sub" "x"))))
            (export main)))
  (output (: 22 Int64)))

(case "a String match with a BOUND default arm equals its (= s literal) if-chain with a bound else"
  (doc    "The default arm need not be a wildcard `_` — it may BIND the scrutinee and use it. `viamatch s =
           (match s (\"add\" 1) (other (String.byte-len other)))` binds `other` = the whole String in the
           default arm; its `=`-chain desugaring is `viachain s = (if (= s \"add\") 1 (String.byte-len s))`
           (the else reuses the scrutinee, not a fresh name). On `\"wxyz\"` (no hit, byte-len 4): match → 4,
           chain → 4, `100*viamatch + viachain` = 404. Pins match ≡ `=`-chain when the default BINDS and
           consumes the scrutinee (not only a constant wildcard body), both backends.")
  (input  (do
            (def (viamatch (: s String)) (match s ("add" 1) (other (String.byte-len other))))
            (def (viachain (: s String)) (if (= s "add") 1 (String.byte-len s)))
            (def (main) (+ (* 100 (viamatch (if false "add" "wxyz"))) (viachain (if false "add" "wxyz"))))
            (export main)))
  (output (: 404 Int64)))

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

(case "String.concat with an empty operand is the identity on a RUNTIME string (the emit path, not the fold)"
  (doc    "The two identity cases above use CONSTANT operands, so they fold at compile time. This pins the
           SAME left/right identity on a RUNTIME string — `s` built via `(if true \"hi\" \"x\")` so it is
           not a constant — exercising the runtime `String.concat` EMIT, which must handle a zero-length
           operand (an empty rope leaf / empty UTF-8 span) rather than assuming a non-empty side. `(String.
           concat \"\" s)` and `(String.concat s \"\")` both equal `s` = \"hi\"; `(and …)` of the two → 1.
           A runtime concat that mishandled the empty operand (read a phantom byte, or dropped the non-empty
           side) would break here where the folded cases could not catch it, both backends.")
  (input  (do
            (def (lid (: s String)) (String.concat "" s))
            (def (rid (: s String)) (String.concat s ""))
            (def (main) (if (and (= (lid (if true "hi" "x")) "hi") (= (rid (if true "hi" "x")) "hi")) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "runtime String.concat is associative — (a·b)·c equals a·(b·c), both equal the flat concatenation"
  (doc    "Concatenation is associative: `(String.concat (String.concat a b) c)` and `(String.concat a
           (String.concat b c))` build the SAME string. Over runtime operands (`a` via `(if true \"a\"
           \"z\")` so nothing folds), with a multi-byte scalar in the middle: a=\"a\", b=\"é\" (1 scalar,
           2 UTF-8 bytes), c=\"bc\" — both groupings yield \"aébc\". Pins that the runtime concat EMIT (a
           rope build/append) is associative and does not depend on grouping — a rope-rebalance or a
           left-vs-right append that split the multi-byte scalar or reordered bytes at the join would make
           the two groupings differ. `(and (= left \"aébc\") (= right \"aébc\"))` → 1, both backends.")
  (input  (do
            (def (lft (: a String) (: b String) (: c String)) (String.concat (String.concat a b) c))
            (def (rgt (: a String) (: b String) (: c String)) (String.concat a (String.concat b c)))
            (def (main) (if (and (= (lft (if true "a" "z") "é" "bc") "aébc")
                                 (= (rgt (if true "a" "z") "é" "bc") "aébc")) 1 0))
            (export main)))
  (output (: 1 Int64)))

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

; Normalization is a WHOLE-VALUE property, so it must hold on the ADDRESSING axis too, not only on `=`
; (:510) and scalar-len (:518). A DECOMPOSED literal "cafe\u0301" (c, a, f, e, U+0301 combining acute — five
; raw scalars) is normalized by the reader to the four-scalar NFC form (c, a, f, é), so `String.at` and
; `String.scalar-len` operate on the NORMALIZED sequence: index 3 is the composed "é" (U+00E9), not the bare
; "e", and the string ends at scalar 4 (index 4 is None), not at the raw fifth scalar. A lowering that
; normalized for `=`/scalar-len but byte-walked the RAW scalars for `String.at` would return "e" at index 3
; (or index into the combining accent) and report length 5 — this pins the indexing axis against that.
; (This is the reader-normalized LITERAL path; `String.from-bytes` is a faithful byte decode that does NOT
; NFC-normalize — the deliberate known gap pinned at :1395 — so the two paths are distinct.)
(case "String.at on a decomposed literal returns the composed scalar at the normalized index"
  (doc    "The decomposed \"café\" (c, a, f, e + U+0301 combining acute — five raw scalars) is normalized by
           the reader to the four-scalar NFC form, so `(String.at \"café\" 3)` = \"é\" (the COMPOSED U+00E9),
           not the bare \"e\" of the un-normalized fifth-from-last scalar. Indexing addresses the NORMALIZED
           sequence (collections-and-text.md #String Equality Follows Normalized Contents applied to the
           addressing axis), the indexing companion of the `=` (:510) and scalar-len (:518) normalization
           cases. A raw-scalar byte walk would return \"e\".")
  (input  (String.at "café" 3))
  (output (: (Some "é") (Option String))))

(case "a decomposed literal indexes and measures by its normalized length, not the raw scalar count"
  (doc    "The combined end-boundary face: the decomposed \"café\" has scalar-len 4 (the NFC form c, a, f, é),
           NOT the raw 5 (e + combining acute), AND `(String.at \"café\" 4)` is None — index 4 is past the
           normalized four-scalar end, not the raw fifth scalar. Sentinel `10·scalar-len + (at 4 ? 1 : 0)` =
           40. Pins that BOTH the length measure and the addressing bound use the normalized sequence; a
           lowering counting the raw 5 scalars would give 51 (len 5, and .at 4 = Some of the combining acute).")
  (input  (do (def (main)
                (+ (* 10 (String.scalar-len "café"))
                   (match (String.at "café" 4) ((Some _c) 1) ((None _u) 0))))
              (export main)))
  (call   main)
  (output (: 40 Int64)))

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
  (doc    "`(String.slice \"hello\" a b)` with RUNTIME bounds a=1, b=4 yields Some \"ell\" — scalars 1..4.
           Passing the slice BOUNDS as `main` parameters defeats const-folding (the folder cannot evaluate a
           slice whose indices are unknown at compile time), so this exercises the runtime UTF-8 slice walk —
           not the const-fold — and it must agree with the folded literal cases above. (Runtime BOUNDS, not a
           runtime string: a `String.concat s \"\"` over a literal `s` β-reduces + folds back to a literal.)")
  (input  (do
            (def (main (: a Int64) (: b Int64)) (Option.expect (String.slice "hello" a b) "in range"))
            (export main)))
  (call   main (: 1 Int64) (: 4 Int64))
  (output (: "ell" String)))

(case "a runtime string slice addresses scalar values, not bytes"
  (doc    "`(String.slice \"aébc\" a b)` with RUNTIME bounds a=1, b=3 yields Some \"éb\" — scalars 1 and 2
           (é is one scalar, TWO UTF-8 bytes). Runtime bounds force the runtime slice walk (the folder can't
           fold an unknown-index slice). A slice that indexed by BYTE offset would split é or read the wrong
           range; pins that the runtime walk maps scalar offsets to byte offsets, exactly as String.at does
           (13-strings §reading a string's scalar addresses scalar values, not bytes).")
  (input  (do
            (def (main (: a Int64) (: b Int64)) (Option.expect (String.slice "aébc" a b) "in range"))
            (export main)))
  (call   main (: 1 Int64) (: 3 Int64))
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

; --- String.slice with RUNTIME scalar-index arguments over a MULTI-BYTE string --------------------
; The runtime-slice cases above supply CONSTANT slice indices (the seed emits the runtime UTF-8 byte-walk
; for the string, but the offsets fold). When the slice INDICES are runtime values (fn parameters at the
; call boundary), the byte-walk must map each SCALAR offset to a byte offset at run time — the multi-byte
; correctness the scalar `String.at` cases pin, now on the runtime-index slice path. Over "café" (scalars
; c,a,f,é; é is 2 UTF-8 bytes, byte-len 5) a byte-indexing slice would split é or read the wrong span; the
; scalar semantics must isolate whole scalars. These pin runtime-index slice on both a 2-byte scalar (é)
; and a 4-byte supplementary-plane scalar (😀), by byte-length and by content.

(case "a runtime-index string slice isolates a multi-byte scalar by its scalar offset"
  (doc    "`(String.slice \"café\" a b)` with RUNTIME indices a=3, b=4 selects scalar 3 — é, which occupies
           2 UTF-8 bytes — so the slice's `String.byte-len` is 2. A byte-indexing lowering would take byte
           offset 3 (the middle of é) and split the code unit; the scalar semantics map scalar offset → byte
           offset at run time and isolate the whole é. Pins the runtime-index slice's scalar-vs-byte mapping
           on a 2-byte scalar (the runtime-index companion of the constant-index `String.at \"café\" 3` = é).")
  (input  (do (def (main (: a Int64) (: b Int64))
                (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
              (export main)))
  (call   main (: 3 Int64) (: 4 Int64))
  (output (: 2 Int64)))

(case "a runtime-index string slice of the ASCII prefix before a multi-byte scalar"
  (doc    "`(String.slice \"café\" 0 3)` with runtime indices selects scalars 0..2 (c,a,f — all ASCII), byte-len
           3. The prefix companion: the walk stops exactly at the scalar-3 boundary (byte 3), not inside é.")
  (input  (do (def (main (: a Int64) (: b Int64))
                (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
              (export main)))
  (call   main (: 0 Int64) (: 3 Int64))
  (output (: 3 Int64)))

(case "a runtime-index string slice spanning ASCII and a multi-byte scalar"
  (doc    "`(String.slice \"café\" 1 4)` with runtime indices selects scalars 1..3 — a,f (1 byte each) and é
           (2 bytes) — byte-len 4. Pins that the runtime walk accumulates the correct byte span across a
           mix of single- and multi-byte scalars, not a fixed bytes-per-scalar assumption.")
  (input  (do (def (main (: a Int64) (: b Int64))
                (String.byte-len (Option.expect (String.slice "café" a b) "in range")))
              (export main)))
  (call   main (: 1 Int64) (: 4 Int64))
  (output (: 4 Int64)))

(case "a runtime-index string slice compares equal to the expected multi-byte scalar by content"
  (doc    "The content companion (not only byte-length): `(String.slice \"café\" a b)` with runtime a=3, b=4
           equals the string \"é\" by content. Confirms the isolated slice is the RIGHT scalar, not merely a
           2-byte span that happens to have the right length.")
  (input  (do (def (main (: a Int64) (: b Int64))
                (= (Option.expect (String.slice "café" a b) "in range") "é"))
              (export main)))
  (call   main (: 3 Int64) (: 4 Int64))
  (output (: true Bool)))

(case "a runtime-index string slice isolates a supplementary-plane scalar (four UTF-8 bytes)"
  (doc    "`(String.slice \"a😀b\" a b)` with runtime a=1, b=2 selects scalar 1 — 😀 (U+1F600), ONE scalar
           occupying FOUR UTF-8 bytes — so byte-len is 4. A byte- or UTF-16-index walk would land inside the
           emoji's encoding; the scalar walk maps scalar offset 1..2 to the whole four-byte span. Pins the
           runtime-index slice at the supplementary-plane boundary that most tempts a code-unit miscount.")
  (input  (do (def (main (: a Int64) (: b Int64))
                (String.byte-len (Option.expect (String.slice "a😀b" a b) "in range")))
              (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 4 Int64)))

; --- String.slice across the SEAM of a genuinely-runtime string ROPE ------------------------------
; The runtime-index cases above slice a FLAT literal ("hello", "café") — the string is a single leaf and
; only the bounds are runtime. A genuine multi-chunk ROPE — a `String.concat` whose left chunk is chosen
; by a run-time `if` (a folded literal concat β-reduces back to a literal, 13-strings §... at ~:661, so a
; runtime-selected chunk is required to defeat the fold) — reaches the slice as a deferred concatenation,
; so the byte-walk must cross the leaf boundary between chunks. These pin the seam-crossing slice: a span
; that begins in the left chunk and ends in the right must read the logical scalars in order across the
; physical seam, including the scalar→byte mapping when a chunk carries a multi-byte scalar.

(case "a runtime string slice spans the seam of a runtime-assembled rope"
  (doc    "`(String.slice (String.concat (pick b \"abc\" \"xyz\") \"def\") lo hi)` over a rope built at run time
           (the left chunk chosen by a run-time `if`, so the concat cannot fold to a literal), with runtime
           bounds spanning the seam: b=true lo=2 hi=4 selects scalars 2..3 = \"cd\" — 'c' the last scalar of
           the left chunk, 'd' the first of the right — reading across the physical leaf boundary of a
           deferred concatenation (#Sharing Is Not Observable). The b=false branch slices the other rope
           (\"xyzdef\") at the same span → \"zd\", pinning that either runtime chunk assembles a real rope the
           slice crosses, not a pre-folded flat leaf. Both backends.")
  (input  (do
            (def (pick (: b Bool) (: t String) (: f String)) (if b t f))
            (def (main (: b Bool) (: lo Int64) (: hi Int64))
              (Option.expect (String.slice (String.concat (pick b "abc" "xyz") "def") lo hi) "in range"))
            (export main)))
  (call   main (: true Bool) (: 2 Int64) (: 4 Int64))  (output (: "cd" String))
  (call   main (: false Bool) (: 2 Int64) (: 4 Int64)) (output (: "zd" String)))

(case "a runtime rope slice maps scalar offsets to bytes across the seam for a multi-byte scalar"
  (doc    "The multi-byte companion of the rope-seam slice: over a runtime rope `(String.concat (pick b \"aé\"
           \"aX\") \"bc\")`, b=true lo=1 hi=3 selects scalars 1..2 = \"éb\" — é (scalar 1, TWO UTF-8 bytes) is the
           last scalar of the left chunk and b (scalar 2) the first of the right, so the slice crosses the
           seam AND maps a multi-byte scalar's offset to its byte span. A byte-indexing walk would split é or
           miscount across the seam; the scalar walk isolates \"éb\" (byte-len 3). Pins the across-seam
           scalar→byte mapping on a genuine rope, both backends.")
  (input  (do
            (def (pick (: b Bool) (: t String) (: f String)) (if b t f))
            (def (main (: b Bool) (: lo Int64) (: hi Int64))
              (String.byte-len (Option.expect (String.slice (String.concat (pick b "aé" "aX") "bc") lo hi) "in range")))
            (export main)))
  (call   main (: true Bool) (: 1 Int64) (: 3 Int64)) (output (: 3 Int64)))

; --- `String.at i` and `String.slice i (i+1)` are the SAME single-scalar addressing — they must agree ---
; `String.at` and `String.slice` are the two runtime scalar-addressing String ops (both byte-walk the UTF-8
; leaf, mapping scalar offsets to byte offsets). A single-scalar slice `[i, i+1)` selects exactly the one
; scalar `String.at i` returns — so `String.slice s i (i+1)` and `String.at s i` MUST produce the equal
; one-scalar substring for the same runtime `s`/`i`. They lower through separate emit paths (String.at's
; one-scalar read vs String.slice's start..end byte-walk), so this pins the two paths AGREE — a lowering
; that computed a different byte span for one (an off-by-one scalar→byte map, or slicing bytes not scalars)
; would diverge them. Checked over a MULTI-BYTE scalar (é = 1 scalar, 2 bytes) so a byte/scalar confusion
; in either path is observable, on both backends.
(case "a single-scalar String.slice equals the String.at of the same index (the two scalar-addressing paths agree)"
  (doc    "`(String.slice s i (+ i 1))` and `(String.at s i)` both address the single scalar at index `i`,
           so their one-scalar substrings are equal. Over a runtime `s = \"aébc\"` (é is one scalar, TWO
           UTF-8 bytes) at `i = 1`: both yield \"é\" — `(= (slice…) (at…))` → true (1). Pins that the
           String.slice byte-walk and the String.at read map scalar→byte identically for a single scalar
           (a byte/scalar off-by-one in either path would make them unequal), both backends. `s` is built
           via `(if true …)` so it is a runtime value, not a folded constant.")
  (input  (do
            (def (viaslice (: s String) (: i Int64)) (Option.expect (String.slice s i (+ i 1)) "in bounds"))
            (def (viaat (: s String) (: i Int64)) (Option.expect (String.at s i) "in bounds"))
            (def (main) (if (= (viaslice (if true "aébc" "x") 1) (viaat (if true "aébc" "x") 1)) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "the scalar-len of an in-bounds String.slice equals its span (end - start), independent of byte width"
  (doc    "`String.slice` addresses SCALARS, so an in-bounds `(String.slice s i j)` returns exactly `j - i`
           scalar values regardless of how many BYTES each occupies — `(String.scalar-len (slice s i j))`
           == `j - i`. Over a runtime `s = \"aébcd\"` (é is one scalar, TWO UTF-8 bytes) with the span
           `1..4`: the slice is \"ébc\" — 3 scalars (though 4 bytes), so scalar-len = 3 = 4 - 1. Pins the
           slice's scalar count is its scalar SPAN, not a byte count — a slice that returned a byte-measured
           length, or that mis-mapped the multi-byte scalar, would give the wrong count. The scalar-count
           companion of the slice≡at case above, both backends. `s` is built via `(if true …)` so it is a
           runtime value, not a folded constant.")
  (input  (do
            (def (spanlen (: s String) (: i Int64) (: j Int64))
              (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
            (def (main) (spanlen (if true "aébcd" "x") 1 4))
            (export main)))
  (output (: 3 Int64)))

; slice LENGTH-ALGEBRA neighbors (breaker): the case above pins scalar-len(slice i j) == j - i. These pin
; the companions: on the SAME multi-byte slice the BYTE-len differs from the scalar-len (proving the two
; measures genuinely diverge), a full-string slice (0..scalar-len) recovers the whole string's length by
; BOTH measures, and an empty span (i=j) is scalar-len 0 even at a multi-byte position. Over "aébcd" (a,é,
; b,c,d — 5 scalars; é is 2 UTF-8 bytes, so byte-len 6). `s` is a runtime value via `(if true …)`.

(case "the byte-len of a multi-byte String.slice exceeds its scalar-len (the two measures diverge)"
  (doc    "The SAME slice the span case measures at scalar-len 3, measured by BYTE-len, is 4: `(String.slice
           \"aébcd\" 1 4)` = \"ébc\" — 3 scalars but 4 bytes (é=2, b=1, c=1). Pins that byte-len and scalar-len
           of one slice genuinely DIVERGE for multi-byte content — the slice carries the real UTF-8 bytes,
           and byte-len counts them while scalar-len counts the span. A slice that stored a scalar-count as
           its byte length (or vice versa) would make these equal.")
  (input  (do
            (def (bytelen (: s String) (: i Int64) (: j Int64))
              (String.byte-len (Option.expect (String.slice s i j) "in bounds")))
            (def (main) (bytelen (if true "aébcd" "x") 1 4))
            (export main)))
  (output (: 4 Int64)))

(case "a full-string String.slice recovers the whole string's length by both measures"
  (doc    "Slicing the FULL span `0..scalar-len` returns the whole string: scalar-len of `(String.slice
           \"aébcd\" 0 5)` is 5 (= the string's scalar-len), and its byte-len is 6 (= the string's byte-len,
           é contributing 2). Pins the identity slice at both measures — the span 0..n is the whole string,
           not off-by-one at either end.")
  (input  (do
            (def (spanlen (: s String) (: i Int64) (: j Int64))
              (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
            (def (main) (spanlen (if true "aébcd" "x") 0 5))
            (export main)))
  (output (: 5 Int64)))

(case "a full-string String.slice byte-len equals the whole string's byte-len"
  (doc    "The byte-len companion of the full-string slice: `(String.byte-len (String.slice \"aébcd\" 0 5))`
           is 6 — a=1, é=2, b=1, c=1, d=1. Pins that the identity slice preserves the exact UTF-8 byte count,
           not a scalar-count (which would be 5).")
  (input  (do
            (def (bytelen (: s String) (: i Int64) (: j Int64))
              (String.byte-len (Option.expect (String.slice s i j) "in bounds")))
            (def (main) (bytelen (if true "aébcd" "x") 0 5))
            (export main)))
  (output (: 6 Int64)))

(case "an empty-span String.slice at a multi-byte position has scalar-len zero"
  (doc    "An empty span `i=j` returns the empty string — scalar-len 0 — even when `i` sits at a multi-byte
           scalar boundary. `(String.scalar-len (String.slice \"aébcd\" 1 1))` = 0 (index 1 is é's position).
           Pins that a zero-width span is genuinely empty regardless of the byte width at that position — the
           j - i = 0 span case, the interior-multibyte companion of the empty-span boundary.")
  (input  (do
            (def (spanlen (: s String) (: i Int64) (: j Int64))
              (String.scalar-len (Option.expect (String.slice s i j) "in bounds")))
            (def (main) (spanlen (if true "aébcd" "x") 1 1))
            (export main)))
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

(case "the scalar length of a GENUINELY-runtime multi-byte string walks the UTF-8 leaf"
  (doc    "The case above passes a bare literal, which const-FOLDS (`chars().count()`) before emit — it
           never exercises the runtime `String.scalar-len` path. Forcing a genuine runtime String with
           `(String.concat s \"\")` (the same runtime-forcing the runtime `String.at` cases use) makes the
           backend WALK the UTF-8 byte leaf, counting LEAD bytes (`(byte & 0xC0) != 0x80`, not the
           `10xxxxxx` continuation bytes) — the scalar count for well-formed UTF-8. `\"café\"` is 4 scalars
           (c,a,f,é) though 5 bytes (é is a 2-byte encoding). Pins the emit-side scalar-len walk (reusing
           `String.at`'s scalar-scan machinery over the already-exported `bytes-len`/`bytes-get` ops — no
           new runtime op, frozen hash unchanged); the const case above pins the fold. Was a decline before
           this (\"a runtime string's scalar length needs a UTF-8 decoding walk\").")
  (input   (do
             (def (slen s) (String.scalar-len (String.concat s "")))
             (def (main)   (slen "café")) (export main)))
  (output  (: 4 Int64)))

(case "the scalar length of a runtime string spanning every UTF-8 width"
  (doc    "The full-width witness: a genuinely-runtime `\"café—😀\"` mixes 1-byte (c,a,f), 2-byte (é),
           3-byte (—, U+2014) and 4-byte (😀, U+1F600) encodings — 6 scalars, 12 bytes. `String.scalar-len`
           counts 6 (every lead byte), confirming the walk's `(byte & 0xC0) != 0x80` lead-byte test handles
           all four UTF-8 encoding widths, not just the 2-byte case. Contrast `String.byte-len` = 12. The
           multi-width companion of the café case.")
  (input   (do
             (def (slen s) (String.scalar-len (String.concat s "😀")))
             (def (main)   (slen "café—")) (export main)))
  (output  (: 6 Int64)))

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

; The runtime String.at cases above force ropes via `(String.concat s "")` — an EMPTY right chunk, so
; the rope has no real interior seam and every scalar lives in one leaf. These use a GENUINE two-chunk
; rope (if-selected left chunk ++ non-empty right chunk) with MULTIBYTE content in the left chunk: the
; scalar→byte mapping must carry ACROSS the seam (scalar 2 begins at byte 3 when the left chunk is
; "aé", at byte 2 when it is "xy" — same scalar index, different byte offset per branch), and a scalar
; read AT the seam-adjacent position must return the multibyte scalar whole.

(case "String.at addresses scalars across the seam of a runtime multibyte rope"
  (doc    "`(String.at (String.concat (pick b) \"bc\") 2)` — the rope is `\"aé\" ++ \"bc\"` (b>0) or
           `\"xy\" ++ \"bc\"` (b≤0). Scalar 2 is \"b\" in BOTH branches, but its BYTE offset differs: 3
           after the two-byte é, 2 after ascii — so a byte-indexed (or per-leaf-only) walk gives the
           wrong answer in exactly one branch. Both calls must see \"b\" → 1. Pins the scalar→byte map
           spans the concat seam with multibyte content upstream of it. Expected: 1, 1.")
  (input  (do
            (def (pick (: b Int64))
              (if (> b 0) "aé" "xy"))
            (def (main (: b Int64))
              (match (String.at (String.concat (pick b) "bc") 2)
                ((Some c) (if (= c "b") 1 0))
                ((None u) -1)))
            (export main)))
  (call   main (: 1 Int64))  (output (: 1 Int64))
  (call   main (: -1 Int64)) (output (: 1 Int64)))

(case "String.at reads a multibyte scalar whole at its position in a runtime two-chunk rope"
  (doc    "The same two-chunk shape read AT the multibyte scalar: index 1 of `\"aé\" ++ \"cd\"` is \"é\"
           (→ 1); the control branch `\"ab\" ++ \"cd\"` has \"b\" there (→ 2). The é spans two UTF-8
           bytes ending at the leaf boundary — a reader that split the scalar at the leaf edge or
           returned a one-BYTE slice would fail the content compare. Together with the across-the-seam
           case this pins both halves of multibyte addressing in a genuine rope: landing ON the wide
           scalar, and landing PAST it. Expected: 1 (b=1), 2 (b=-1).")
  (input  (do
            (def (pick (: b Int64))
              (if (> b 0) "aé" "ab"))
            (def (main (: b Int64))
              (match (String.at (String.concat (pick b) "cd") 1)
                ((Some c) (if (= c "é") 1 (if (= c "b") 2 0)))
                ((None u) -1)))
            (export main)))
  (call   main (: 1 Int64))  (output (: 1 Int64))
  (call   main (: -1 Int64)) (output (: 2 Int64)))

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

(case "decoding an EMPTY byte sequence yields Some of the empty string"
  (doc    "The degenerate VALID boundary of the decode (the invalid cases below and the well-formed case
           above all feed NON-empty input): zero bytes is trivially well-formed UTF-8 — the empty string
           encodes to zero bytes — so `(String.from-bytes (Bytes.of (list)))` is `(Some "")`, NOT None. A
           state-machine decoder with an off-by-one on the input length, or one that treated "consumed no
           bytes / produced no scalar" as a decode failure, would wrongly return None for empty input. Pins
           that empty is a valid decode (collections-and-text.md #Decoding Bytes To A String Is Total).")
  (input  (= (String.from-bytes (Bytes.of (list))) (Some "")))
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

(case "decoding a lone continuation byte yields none"
  (doc    "`(Bytes.of (list 128))` is `0x80` — a CONTINUATION byte (`10xxxxxx`) with no preceding lead byte.
           A well-formed UTF-8 sequence never starts with a continuation, so `String.from-bytes` yields
           `None`. Distinct from the lone-`0xFF` case (0xFF is never a valid byte at all) and the overlong
           case (structurally-paired but non-canonical): here the byte is a valid CONTINUATION shape but
           appears with no lead to continue — the STATE-MACHINE failure mode a decoder that only rejected
           `0xFF`/overlong could miss. Pins that a stray continuation is rejected.")
  (input  (= (String.from-bytes (Bytes.of (list 128))) None))
  (output (: true Bool)))

(case "decoding a truncated multi-byte sequence yields none"
  (doc    "`(Bytes.of (list 195))` is `0xC3` — a 2-byte LEAD (`110xxxxx`) with NO following continuation byte
           (the sequence ends mid-codepoint). `é` needs `C3 A9`; `C3` alone is truncated, so
           `String.from-bytes` yields `None`. The dual of the lone-continuation case: a lead expecting a
           continuation that never arrives (a decode that ran off the end of the input). Together they pin
           BOTH state-machine failure faces — a continuation with no lead, and a lead with no continuation —
           beyond the byte-value (`0xFF`) and shortest-form (overlong) rejections above.")
  (input  (= (String.from-bytes (Bytes.of (list 195))) None))
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

; The invalid-UTF8 cases above decode CONSTANT `(Bytes.of (list …))` literals, which the fold can validate at
; compile time. A GENUINELY-runtime byte ROPE — a `Bytes.concat` of chunks chosen by a run-time `if` (a
; folded-literal concat would collapse back to a literal) — reaches the emitted `str-from-bytes` decode as a
; multi-chunk deferred concatenation, so the UTF-8 validator must walk the logical bytes ACROSS the leaf seam.
; The sharpest face: a multi-byte scalar STRADDLING the seam — its lead byte the last of the left chunk, its
; continuation the first of the right — must decode as one scalar (a validator that assumed a scalar lies
; within a single leaf would wrongly reject it). Pins the runtime decode over a rope, including the seam-straddle.

(case "String.from-bytes validates a multi-byte scalar straddling a runtime byte-rope's seam"
  (doc    "Over a rope `(Bytes.concat left right)` assembled at run time (the left chunk chosen by a run-time
           `if`, so the concat cannot fold): `sel`=0 builds `[99, 195] ++ [169]` — the é lead `C3`(195) ends
           the left chunk, its continuation `A9`(169) starts the right — a VALID `cé` across the seam →
           Some (→ 1). `sel`=1 builds `[99, 195] ++ [99]` — the lead `C3` then a NON-continuation `c`(99)
           across the seam — INVALID (a lead with no continuation) → None (→ 0). Pins that the runtime
           `str-from-bytes` decode walks the logical bytes across the leaf boundary and validates a scalar
           that spans the seam, matching the const decode of the same byte sequence. Both backends via the
           `(call)` form (a nullary rope const-folds; a runtime-selected chunk forces the genuine rope).")
  (input  (do
            (def (pickb (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
            (def (validq (: b Bytes)) (match (String.from-bytes b) ((Some _s) 1) ((None) 0)))
            (def (main (: sel Int64))
              (validq (Bytes.concat (Bytes.of (list 99 195))
                                    (pickb sel (Bytes.of (list 169)) (Bytes.of (list 99))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: 0 Int64)))

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

; --- KNOWN GAP (operator ruling 2026-07-18): String.from-bytes does NOT NFC-normalize --------------
; A string LITERAL is NFC-normalized at parse time (the reader; 01-literals + the normalization cases
; above), so two literals differing only in normalization denote ONE value. `String.from-bytes` is
; DIFFERENT: it is a FAITHFUL byte decode — it validates UTF-8 well-formedness (the None cases above) but
; PRESERVES the exact scalar sequence, WITHOUT re-normalizing to NFC. This is a DELIBERATE known gap, not a
; bug: NFC normalization needs the Unicode canonical-composition DATA TABLES, and the operator ruled that
; carrying those in the dependency-free compiler core (which must port cleanly to the Cadenza self-host) is
; not worth the bloat right now. CONSEQUENCE: `from-bytes` obeys "equality follows normalization" only if
; the input bytes were ALREADY NFC — normalization is the CALLER'S responsibility on the from-bytes path
; (a literal on the reader path is already normalized; bytes from an external source may not be). So a
; DECOMPOSED "café" (e + U+0301, bytes `63 61 66 65 CC 81`) decodes to a String that is NOT `=` to the
; composed literal "café" (U+00E9) — because the seed does not normalize either side of `from-bytes`, the
; decoded decomposed form keeps its 6 bytes / 5 scalars and differs from the 5-byte / 4-scalar NFC literal.
; (If the "equal strings differing only in normalization" cases above are ever satisfied by adding NFC at
; the reader, this from-bytes case STILL declines-to-normalize until NFC is carried into the core — track
; both as the same Unicode-normalization gap.) A future `from-bytes-normalizing` (or carrying the tables)
; would flip this; a `from-bytes-raw` escape hatch is MOOT while the default already preserves bytes.
(case "String.from-bytes does not NFC-normalize — a decomposed form stays distinct (known gap)"
  (doc    "`from-bytes` is a faithful byte decode: it does NOT re-normalize to NFC (a deliberate known gap —
           the Unicode composition tables would bloat the dependency-free core; operator ruling 2026-07-18).
           So decoding the DECOMPOSED \"café\" bytes (`e` + U+0301 combining acute = `… 101 204 129`) yields
           a String whose scalars are the decomposed sequence, which is NOT `=` to the COMPOSED literal
           \"café\" (U+00E9) — the seed normalizes NEITHER side of from-bytes. NFC is the caller's
           responsibility on the from-bytes path (a literal is normalized at parse; external bytes are not).
           Pins the non-normalization as INTENDED, documented behavior: `(= (from-bytes decomposed) \"café\")`
           is FALSE. Contrast the well-formed-decode case above (composed bytes → Some \"café\", which DOES
           equal the literal because those bytes are already NFC). Flips only when NFC is carried into the
           core; a from-bytes-raw hatch is moot while the default preserves bytes.")
  (input  (= (String.from-bytes (Bytes.of (list 99 97 102 101 204 129))) (Some "café")))
  (output (: false Bool)))

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

(case "two dependent utf8 bin segments each sized by its own preceding length byte"
  (doc    "The dependent-cursor face: a `bin` pattern reads a length byte, decodes that many bytes, then
           reads a SECOND length byte and decodes that many more — `(bin (u8 a) (utf8 s1 a) (u8 b) (utf8 s2
           b))`. The second segment's length `b` is a value read AFTER the first segment consumed `a` bytes,
           so the match cursor must advance past `s1` before reading `b`. Against `(list 2 65 66 1 67)` —
           a=2 → \"AB\", then b=1 → \"C\" — binds s1=\"AB\", s2=\"C\", and `(String.concat s1 s2)` = \"ABC\".
           Extends the single-dependent `(bin (u8 n) (utf8 name n))` case to a MULTI-segment pattern where a
           later segment's length depends on a value the pattern read earlier, pinning that the decode cursor
           threads correctly across segments (options/binary-syntax/).")
  (input  (match (Bytes.of (list 2 65 66 1 67))
            ((bin (u8 a) (utf8 s1 a) (u8 b) (utf8 s2 b)) (String.concat s1 s2))
            (_ "x")))
  (output (: "ABC" String)))

(case "a dependent utf8 bin segment whose length exceeds the remaining bytes is a non-match"
  (doc    "The same `(bin (u8 n) (utf8 name n))` pattern against `(list 5 102 111)` — n=5 but only 2 bytes
           follow — cannot read the 5 bytes the length demands, so the `utf8` segment does NOT match and
           control falls to the catch-all, yielding \"invalid\". The over-length read is a NON-MATCH, never
           a trap or an out-of-bounds read past the buffer: a dependent length that outruns the input is
           handled by the same exhaustiveness-required catch-all every `bin` match obeys (the length-overrun
           companion of the ill-formed-UTF-8 non-match; collections-and-text.md #Decoding Bytes To A String
           Is Total, Not Trapping).")
  (input  (match (Bytes.of (list 5 102 111))
            ((bin (u8 n) (utf8 name n)) name)
            (_ "invalid")))
  (output (: "invalid" String)))

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

; The three scalar-at cases above use a CONSTANT string AND a CONSTANT index, so they fold to a
; `Leaf::Char` at compile time (`lower_str_scalar_at`) and never exercise a runtime read. `String.scalar-at`
; with a RUNTIME index — even over a constant string — cannot fold: its result is a `Char`, and a runtime
; Char has NO machine representation in the seed yet (`Ty::Char` → no boundary/slot valtype, lir.rs ~394;
; `Core::ConstChar` only folds, select.rs ~5698). So the op soundly DECLINES rather than emitting a
; possibly-truncated scalar read. This is the reject-don't-miscompile boundary for the scalar-at EMIT path
; (distinct from the runtime-char MATCH decline below): the runtime half `op_bytes_scalar_at` is built +
; proven in cdz-runtime but stays UNEXPORTED (frozen hash unchanged) until the runtime Char rep lands. A
; future change that emitted a runtime scalar-at (a wrong i32-codepoint box, a truncated read) instead of
; declining would flip this `(declines)`; it upgrades to an executing witness when the Char rep is wired.
(case "String.scalar-at over a runtime index declines pending the runtime Char rep"
  (doc    "`(String.scalar-at \"café\" i)` with `i` a runtime Int64 PARAMETER cannot fold (the result is a
           Char, and a runtime Char has no seed representation yet), so `String.scalar-at` soundly DECLINES
           rather than emitting a possibly-truncated scalar read — the reject-don't-miscompile boundary for
           the scalar-at emit path. Contrast the constant-index `(String.scalar-at \"café\" 3)` case above,
           which folds to `(Some #\\é)`. Grades the scalar-at emit-path decline as an intentional, tracked
           boundary: the runtime read `op_bytes_scalar_at` is proven in cdz-runtime but unexported until the
           runtime Char rep lands (companion to the runtime-char match decline below).")
  (input  (do
            (def (main (: i Int64)) (String.scalar-at "café" i))
            (export main)))
  (declines))

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

(case "converting a NEGATIVE integer to a char yields None"
  (doc    "The LOW-end companion of the out-of-range cases above (which pin the high end U+10FFFF/U+110000
           and the surrogate block): no negative integer is a Unicode scalar value, so `(Char.from-int -1)`
           yields None — handled as data, not a trap, and NOT wrapped to a huge unsigned value that might
           alias a valid scalar. Pins the lower bound of the valid-scalar check.")
  (input  (match (Char.from-int -1) ((Some c) (Char.to-int c)) ((None u) -1)))
  (output (: -1 Int64)))

(case "converting zero to a char yields Some — U+0000 (NUL) is a valid scalar"
  (doc    "The load-bearing low-boundary pin: U+0000 (NUL) IS a valid Unicode scalar and MUST convert, so
           `(Char.from-int 0)` is `Some` and its `Char.to-int` round-trips to 0. A lower-bound check written
           `> 0` instead of `>= 0`, or one that excluded NUL as a control character, would wrongly reject it.
           The accept-side companion of the negative-rejection case (collections-and-text.md #A Char Converts
           To And From An Integer Totally).")
  (input  (match (Char.from-int 0) ((Some c) (Char.to-int c)) ((None u) -1)))
  (output (: 0 Int64)))

; The cases above use U+D800 (first surrogate) and U+110000 (one PAST the max). These pin the EXACT
; boundaries where an off-by-one in the range/surrogate check surfaces: U+10FFFF (the MAXIMUM valid scalar,
; one below the U+110000 rejection) is Some; U+DFFF (the LAST surrogate, the block's upper endpoint) is
; None; and U+E000 (the first scalar AFTER the surrogate block) is Some. A check written `< 0x110000`
; instead of `<= 0x10FFFF`, or a surrogate block off by one at either end, flips one of these.

(case "Char.from-int at the maximum valid scalar U+10FFFF is Some"
  (doc    "`(Char.from-int 1114111)` — 1114111 is U+10FFFF, the MAXIMUM Unicode scalar value — is a valid
           scalar, so from-int yields Some. The just-below-the-ceiling companion of the U+110000 (1114112)
           rejection above: 10FFFF is IN range, 110000 is one PAST. Pins the exact upper boundary of the
           valid-scalar check (`<= 0x10FFFF`, not `< 0x110000` off-by-one — both reject 110000 but only the
           correct bound accepts 10FFFF).")
  (input  (match (Char.from-int 1114111) ((Some _) 1) ((None _) 0)))
  (output (: 1 Int64)))

(case "Char.from-int at the last surrogate U+DFFF is None"
  (doc    "`(Char.from-int 57343)` — 57343 is U+DFFF, the LAST (highest) surrogate code point — is not a
           scalar, so from-int yields None. The upper-endpoint companion of the U+D800 (55296, the FIRST
           surrogate) case: the surrogate block is [U+D800, U+DFFF] inclusive, so both endpoints reject.
           Pins the block's upper edge (a block ending at 0xDFFE would wrongly accept 0xDFFF).")
  (input  (match (Char.from-int 57343) ((Some _) 1) ((None _) 0)))
  (output (: 0 Int64)))

(case "Char.from-int at U+E000 (first scalar after the surrogate block) is Some"
  (doc    "`(Char.from-int 57344)` — 57344 is U+E000, the FIRST scalar value immediately after the surrogate
           block (which ends at U+DFFF) — is valid, so from-int yields Some. Pins that the surrogate
           exclusion ends exactly at U+DFFF: U+E000 is accepted, so the block is [D800, DFFF] and not one
           wider. The lower-boundary complement of the last-surrogate case.")
  (input  (match (Char.from-int 57344) ((Some _) 1) ((None _) 0)))
  (output (: 1 Int64)))

(case "the maximum valid scalar U+10FFFF round-trips through to-int"
  (doc    "`(Char.to-int (Char.from-int 1114111))` recovers 1114111 — the max scalar survives the char
           round-trip intact. The extreme companion of the mid-range round-trip below: a conversion that
           truncated or mis-handled the 21-bit-wide maximum scalar would lose it.")
  (input  (= (Char.to-int (Option.expect (Char.from-int 1114111) "max scalar")) 1114111))
  (output (: true Bool)))

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

(case "a multibyte-scalar char orders by scalar value, not UTF-8 byte length"
  (doc    "The >127 companion of the ASCII char-order case: ordering is the NUMERIC order of the
           SCALAR VALUE (collections-and-text.md #A Char Is A Single Unicode Scalar Value), even when
           the UTF-8 encodings differ in LENGTH. Built via `Char.from-int` (97 = a, 1 byte; 233 = e-acute,
           2 bytes; 128512 = an emoji, 4 bytes): a < e-acute < emoji by scalar (97 < 233 < 128512) —
           encoded `10*(a<e) + (e<r)` = 11. The ASCII pins never leave the 1-byte range where scalar
           and byte order coincide; this is the first multibyte witness, distinguishing scalar order
           from a byte-length or byte-sequence comparison.")
  (input  (do
            (def (main (: k Int64))
              (match (Char.from-int 97)
                ((Some a)
                  (match (Char.from-int 233)
                    ((Some e)
                      (match (Char.from-int 128512)
                        ((Some r) (+ (* 10 (if (< a e) 1 0)) (if (< e r) 1 0)))
                        (None -3)))
                    (None -2)))
                (None -1)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 11 Int64)))

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

(case "arithmetic between a char and an Int64 is rejected with the plain Char.to-int fix"
  (doc    "The Int64 BASE CASE of the char-with-number family the float/BigInt/Rational cases below refer to
           ('the integer sibling `(+ #\\a 1)` keeps the plain `Char.to-int` wrap'). `(+ #\\a 1)` mixes a
           `Char` with an Int64 — a Char is not a number (collections-and-text.md #A Char Is A Single Unicode
           Scalar Value), so it is CDZ0203. Unlike the wider-numeric siblings, the repair is the PLAIN
           `(Char.to-int #\\a)` — `Char.to-int` yields Int64, which is exactly the sibling operand's type, so
           NO second conversion step is needed (the float/BigInt/Rational cases wrap it further because Int64
           does not implicitly promote to those). Pins the one-step fix for the Int64 mix, the base the
           two-step-fix cases are measured against. The program's outcome is the rejection.")
  (input  (do (def (main) (+ #\a 1)) (export main)))
  (error  CDZ0203))

(case "comparing a char to a FLOAT is rejected with a working two-step conversion fix"
  (doc    "A `Char` is not a number, so `(< #\\a 1.0)` is CDZ0203 (collections-and-text.md #A Char Is A
           Single Unicode Scalar Value — a char compares to a char, not to a raw number). The DIAGNOSTIC's
           fix must actually type-check: `Char.to-int` yields Int64, but Cadenza never implicitly promotes
           Int64 → Float, so a bare `(Char.to-int #\\a)` still fails against a `Float64` (CDZ0301). The fix
           therefore wraps BOTH steps — `(Float64.of-int (Char.to-int #\\a))` — matching the sibling float's
           width (a `Float32` sibling gets `Float32.of-int`). Pins that the char-to-number reject offers a
           REPAIR that resolves the error in one shot for the float case (the integer sibling keeps the plain
           `Char.to-int` wrap). The program's outcome is the rejection; there is no value.")
  (input  (do (def (main) (< #\a 1.0)) (export main)))
  (error  CDZ0203))

(case "arithmetic between a char and a FLOAT is rejected with a working two-step conversion fix"
  (doc    "The ARITHMETIC twin of the char-vs-float comparison case: `(+ #\\a 1.0)` mixes a `Char` with a
           `Float64`, which is CDZ0301 (numeric-model.md #An Arithmetic Operator Requires Both Operands To
           Be One Numeric Type — a Char is not a number, and Cadenza never silently promotes). As with the
           comparison, the fix must type-check: `Char.to-int` yields Int64, and Int64 + Float64 re-fails, so
           the repair is the two-step `(Float64.of-int (Char.to-int #\\a))` matching the sibling float's
           width. Pins fix PARITY between arithmetic and comparison for a char-with-float mix (the integer
           sibling `(+ #\\a 1)` keeps the plain `Char.to-int` wrap). The program's outcome is the rejection.")
  (input  (do (def (main) (+ #\a 1.0)) (export main)))
  (error  CDZ0301))

(case "arithmetic between a char and a BigInt is rejected with a working two-step conversion fix"
  (doc    "The BigInt sibling of the char-with-float arithmetic case: `(+ #\\a (BigInt.of 5))` mixes a `Char`
           with a `BigInt` — CDZ0301 (a Char is not a number). `Char.to-int` yields Int64, and Int64 + BigInt
           re-fails (no implicit promotion), so the working repair is the two-step `(BigInt.of (Char.to-int
           #\\a))`. Pins fix parity across the numeric tower: Char + {Int, Float, BigInt, Rational} each offer
           a repair that type-checks in one shot (Int keeps the plain `Char.to-int`; the wider types wrap it
           in the target's `of`/`of-int`).")
  (input  (do (def (main) (+ #\a (BigInt.of 5))) (export main)))
  (error  CDZ0301))

(case "arithmetic between a char and a Rational is rejected with a working two-step conversion fix"
  (doc    "The Rational sibling: `(+ #\\a (Rational.of-int 5))` mixes a `Char` with a `Rational` — CDZ0301.
           The working repair is `(Rational.of-int (Char.to-int #\\a))` (Int64 scalar then lifted to the whole
           rational), completing char-with-numeric fix parity alongside the Int/Float/BigInt cases.")
  (input  (do (def (main) (+ #\a (Rational.of-int 5))) (export main)))
  (error  CDZ0301))

; --- Char-LITERAL patterns: a `match` dispatches by scalar value ----------------------------------
; A char is a scalar whose identity IS its Unicode scalar value (collections-and-text.md #A Char Is A
; Single Unicode Scalar Value), so a char-literal pattern `(#\a …)` matches by that value — the Char
; analogue of an Int/Bool/String/Symbol-literal match arm (core-semantics.md #Matching Selects The
; First Arm Whose Pattern Matches). A char match dispatches exactly as the char `=` above compares:
; `(match c (#\a 1) (#\b 2) (_ 0))` selects the arm whose char equals `c`. Char is an OPEN type (any
; scalar value), so a char match — like an Int match — needs a wildcard tail to be exhaustive; without
; one it is CDZ0210, and a char pattern over a non-Char scrutinee is a CDZ0201 shape error. (These
; witness the CONSTANT-scrutinee dispatch; a Char has no run-time value form in the seed, exactly as
; the scalar-access cases above note, so every char match folds at compile time.)

(case "a char-literal pattern selects the arm whose char matches"
  (doc    "`(match #\\b (#\\a 1) (#\\b 2) (_ 0))` is 2 — the `#\\b` scrutinee equals the second arm's char
           literal, so that arm is selected (core-semantics.md #Matching Selects The First Arm Whose
           Pattern Matches). The Char analogue of an Int/Bool/String-literal match: dispatch is by scalar
           value, exactly as `(= #\\b #\\b)` holds. Pins that a char literal is a valid match pattern.")
  (input  (do (def (main) (match #\b (#\a 1) (#\b 2) (_ 0))) (export main)))
  (call   main)
  (output (: 2 Int64)))

(case "a char not among the literal arms falls through to the wildcard"
  (doc    "`(match #\\z (#\\a 1) (#\\b 2) (_ 0))` is 0 — `#\\z` matches neither char-literal arm, so the
           wildcard `_` tail covers it (core-semantics.md #Matching Selects The First Arm Whose Pattern
           Matches — the wildcard is the last, always-matching arm). The miss companion of the char-match
           hit; pins that char dispatch is genuine (a non-listed char is NOT silently mapped to an arm).")
  (input  (do (def (main) (match #\z (#\a 1) (#\b 2) (_ 0))) (export main)))
  (call   main)
  (output (: 0 Int64)))

(case "a char-literal pattern nested in a variant payload matches by scalar value"
  (doc    "`(match (Tok.Ch #\\a) ((Tok.Ch #\\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0))` is 97 — the variant
           carries a `Char` payload and the arm `(Tok.Ch #\\a)` matches a `Tok.Ch` whose payload equals
           `#\\a`, exactly as the String/Symbol-payload literal arms do. Pins a char literal as a valid
           NESTED sub-pattern (the payload twin of the top-level char match; the `#\\a` payload variant of
           the `(Ch Char)` case in 05-compound-types).")
  (input  (do (type Tok (Ch Char) (End))
              (def (main) (match (Tok.Ch #\a) ((Tok.Ch #\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0)))
              (export main)))
  (call   main)
  (output (: 97 Int64)))

(case "a nested char-literal payload falls through on a non-matching char"
  (doc    "`(match (Tok.Ch #\\z) ((Tok.Ch #\\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0))` is 1 — the payload
           `#\\z` does not equal the `(Tok.Ch #\\a)` arm's literal, so the match falls to the `(Tok.Ch _)`
           arm binding any char. The miss companion of the nested-payload hit; pins that a nested char
           literal genuinely discriminates within a variant, not a blanket match on the constructor.")
  (input  (do (type Tok (Ch Char) (End))
              (def (main) (match (Tok.Ch #\z) ((Tok.Ch #\a) 97) ((Tok.Ch _) 1) ((Tok.End) 0)))
              (export main)))
  (call   main)
  (output (: 1 Int64)))

; The landed char-pattern cases use CONSTANT char scrutinees. These pin the neighbors: a RUNTIME char (from
; Char.from-int, not a const fold) reaching a later arm; a char pattern over a NON-char scrutinee is a type
; error (the char pattern enforces its type from the PATTERN — unlike the String/Symbol pattern leak, the
; char path is sound); and a supplementary-plane char literal matches by scalar value.

(case "a runtime char reaches a later char-literal arm by scalar value"
  (doc    "`(match c (#\\a 1) (#\\b 2) (_ 0))` with a RUNTIME `c` = `(Char.from-int 98)` = `#\\b` (not a
           constant fold) takes the SECOND arm → 2. Pins that char-literal dispatch works on a runtime char
           value across the arm list (the runtime companion of the landed constant `#\\b` case), so the
           scalar-value compare runs at run time, not only in the constant fold.")
  (input  (do
            (def (classify (: c Char)) (match c (#\a 1) (#\b 2) (_ 0)))
            (def (main) (classify (Option.expect (Char.from-int 98) "b")))
            (export main)))
  (call   main)
  (output (: 2 Int64)))

(case "a char-literal pattern over a non-char scrutinee is a type error"
  (doc    "`(match n (#\\a 1) (_ 0))` with `n : Int64` — a Char pattern over an Int64 scrutinee — is rejected
           CDZ0201 ('match pattern type Char does not match scrutinee type Int64'). Pins that the char
           pattern enforces its type FROM THE PATTERN: a Char pattern requires a Char scrutinee, so a
           non-char scrutinee (Int64 here, and equally a String) is caught — the char path does NOT have the
           String/Symbol pattern's cross-nominal leak, it derives the pattern type soundly.")
  (input  (do (def (main (: n Int64)) (match n (#\a 1) (_ 0))) (export main)))
  (call   main (: 97 Int64))
  (error  CDZ0201))

(case "a supplementary-plane char literal matches by scalar value"
  (doc    "`(match #\\😀 (#\\a 1) (#\\😀 2) (_ 0))` is 2 — the supplementary-plane scalar U+1F600 (😀, above
           the BMP) equals the second arm's char literal, dispatched by scalar value. Pins that char-literal
           dispatch compares the full 21-bit scalar, not a truncated code unit, so a supplementary-plane
           char discriminates correctly — the char-pattern companion of the supplementary-plane String.at.")
  (input  (do (def (main) (match #\😀 (#\a 1) (#\😀 2) (_ 0))) (export main)))
  (call   main)
  (output (: 2 Int64)))

; Every char-pattern case above dispatches a CONSTANT char scrutinee — even the "runtime char" case folds,
; because its `(Char.from-int 98)` argument is a compile-time constant that reduces to `#\b` before the
; match. A GENUINELY runtime char — `(Char.from-int n)` where `n` is a def PARAMETER supplied by `(call …)`
; — has no runtime Char representation in the seed yet (a Char is not `ty_heap_walkable`: the scalar runtime
; char rep is a later increment, select.rs ~5413), so a match (or `=`) on it DECLINES. This grades that
; boundary: a runtime char scrutinee is a sound reject-don't-miscompile decline, NOT a bug — the constant
; path dispatches correctly (cases above) and the runtime path refuses rather than emitting a wrong compare.
; A future change that made a runtime char match/`=` silently MISCOMPILE (a truncated-code-unit compare, a
; wrong scalar box) instead of declining would flip this `(declines)`. Upgrades to an executing witness when
; the runtime Char rep lands.
(case "a match on a genuinely-runtime char (from a runtime Char.from-int) declines pending the runtime Char rep"
  (doc    "`classify` matches a Char against char-literal arms; called with `(Char.from-int n)` where `n` is
           a runtime Int64 PARAMETER (so the char is not constant-folded to a literal like the cases above),
           the seed has no runtime scalar-char representation to dispatch on, so it soundly DECLINES rather
           than emitting a possibly-truncated compare. Contrast the constant `(Char.from-int 98)` case above
           (which folds to `#\\b` and matches → 2). Grades the genuinely-runtime char match decline as an
           intentional, tracked boundary pending the runtime Char rep (a Char is not ty_heap_walkable yet).")
  (input  (do
            (def (classify (: c Char)) (match c (#\a 1) (#\b 2) (_ 0)))
            (def (main (: n Int64)) (classify (Option.expect (Char.from-int n) "in range")))
            (export main)))
  (declines))

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

; The retention case above checks a deep rope's byte-LEN (=8) but not that its CONTENT reads back correctly
; through the depth. A many-chunk rope (built by repeated String.concat) is a deep byte-rope; String.at /
; String.scalar-len / `=` must traverse it correctly at every position, not just measure its length. This
; pins that: a 20-chunk "ab" rope (40 scalars) indexes right at the start, deep interior, and last position,
; equals its flat twin, and is None past the end — the content-through-depth companion of the byte-len case.
(case "a deep many-chunk runtime string rope indexes and measures correctly through its depth"
  (doc    "A 20-chunk rope built by repeated `String.concat` of \"ab\" (a deep runtime byte-rope, 40 scalars).
           `String.scalar-len` = 40; `String.at` reads the right scalar at index 0 (\"a\"), 1 (\"b\"), the
           deep interior 38 (\"a\") and last 39 (\"b\"); the rope `=` its 40-char flat twin (rope operands are
           compacted before the byte-compare, so a rope equals its flat form); and `String.at 40` is None
           (past the end). Result `(40, \"a\", \"b\", \"a\", \"b\", 1, 0)`. Pins that a MANY-chunk rope's
           addressing/length/equality traverse the full depth correctly, not just the 2-chunk ropes and the
           byte-len-only retention case — the content-through-depth companion of the deep-rope retention case
           above.")
  (input  (do
            (def (build (: n Int64) (: acc String))
              (if (= n 0) acc (build (- n 1) (String.concat acc "ab"))))
            (def (at (: s String) (: i Int64)) (match (String.at s i) ((Some c) c) ((None _u) "X")))
            (def (main (: n Int64))
              (let ((r (build 20 "")))
                (tuple
                  (String.scalar-len r)
                  (at r 0) (at r 1) (at r 38) (at r 39)
                  (if (= r "abababababababababababababababababababab") 1 0)
                  (match (String.at r 40) ((Some _c) 1) ((None _u) 0)))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: (tuple 40 "a" "b" "a" "b" 1 0) (Tuple Int64 String String String String Int64 Int64))))

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

; --- Runtime string patterns: the order, boundary, and composition faces ----------------------------
; 155cfa329 emits string-literal match arms by content (value-eq); its pins cover dispatch +
; wildcard fall-through. These pin the semantics a probe-chain desugar could scramble, promoted
; from passing breaker probes.

(case "first-match-wins holds with a duplicate string-literal arm"
  (doc    "`(match s (\"e\" 40) (\"k5\" 50) (\"e\" 99) (_ -1))` on a runtime \"e\" → 40 — the FIRST
           duplicate wins and the later one is dead (the string analogue of the scalar
           duplicate-literal order pins; a chain built from a keyed set instead of source order
           answers 99 or nondeterministically).")
  (input  (do (def (classify (: s String)) (match s ("e" 40) ("k5" 50) ("e" 99) (_ -1)))
              (def (main) (classify (String.concat "e" "")))
              (export main)))
  (output (: 40 Int64)))

(case "the empty string is a matchable literal"
  (doc    "`(\"\" 10)` matches a runtime-built empty string (concat of two empties) → 10. The
           zero-length boundary of the content compare (a probe that tests a first byte before the
           length check reads out of bounds or falls through).")
  (input  (do (def (classify (: s String)) (match s ("" 10) ("x" 20) (_ -1)))
              (def (main) (classify (String.concat "" "")))
              (export main)))
  (output (: 10 Int64)))

(case "a prefix literal does not shadow a longer string arm"
  (doc    "Arms `\"ab\"` then `\"abc\"` on a runtime \"abc\": content equality is whole-string (length
           + bytes), so the prefix arm does NOT match and the exact arm fires → 2. A prefix-compare
           probe (memcmp without the length gate) answers 1.")
  (input  (do (def (classify (: s String)) (match s ("ab" 1) ("abc" 2) (_ -1)))
              (def (main) (classify (String.concat "ab" "c")))
              (export main)))
  (output (: 2 Int64)))

(case "a guard composes with a string-literal arm"
  (doc    "`((guard \"ab\" (> k 3)) 1) (\"ab\" 2)` — the SAME literal guarded then bare: k = 5 passes
           the guard → 1; k = 1 fails and falls to the bare twin → 2. Pins guard fall-through
           through the string content-probe chain (the desugar must AND the guard onto the first
           probe without consuming the literal for the second).")
  (input  (do (def (classify (: s String) (: k Int64))
                (match s ((guard "ab" (> k 3)) 1) ("ab" 2) (_ -1)))
              (def (main (: k Int64)) (classify (String.concat "a" "b") k))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64))
  (call   main (: 1 Int64))
  (output (: 2 Int64)))

; ============================================================================================
; An exported entry with a STRING parameter, called with a string argument — the exported-entry String-arg
; boundary. The emitted rust LIBRARY (`fn main(s: String) -> i64`) is valid, but the rust-target test DRIVER
; that marshals the `"abc"` argument used to pass it as a `&str` literal against the owned-`String` param →
; E0308 (a differential: wasm cleanly declines, rust FAILED to build). Fixed in the rust gate harness
; (`rust_call_arg` now wraps a string-literal arg `.to_string()` so it crosses as an owned String). A String
; param on a HELPER already built on both backends; this pins the exported-entry surface (breaker-found,
; corpus-bugfix). On wasm this DECLINES (a String across the component entry boundary is unrealized) — a
; sound todo; on rust it now runs → 3, matching the recorded value.

(case "an exported entry with a String parameter is called with a string argument"
  (doc    "`(def (main (: s String)) (String.byte-len s))` exported and called with `\"abc\"` → the UTF-8
           byte length 3. The rust DRIVER now marshals the string arg as an owned `String` (`\"abc\".to_string()`),
           matching the emitted `fn main(s: String)` signature — no more E0308. (wasm declines the String
           entry arg — a sound todo; rust computes it.)")
  (input  (do (def (main (: s String)) (String.byte-len s)) (export main)))
  (call   main (: "abc" String))
  (output (: 3 Int64)))
