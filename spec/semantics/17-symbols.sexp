; Symbols — an interned name value with O(1) equality, witnessing the symbol-interning
; decision (options/symbol-interning/). A Symbol is a NOMINAL value wrapping an interned
; String: `(Symbol.of s)` maps a String to a Symbol, and two Symbols are equal exactly when
; their underlying strings are equal (collections-and-text.md #String Equality Follows
; Normalized Contents, lifted through the Symbol tag). The point of the form is that equality
; is a constant-time identity comparison rather than an O(N) byte scan — a self-hosting
; compiler keys its symbol table (name → definition, node-kind dispatch, scope resolution) on
; Symbols so a name comparison is a handle compare, not a string compare.
;
; The value form is `(Symbol.of "<text>")` — the canonical written form, exactly as a byte
; sequence is written `(Bytes.of (list …))`. `#"<text>"` is READER SUGAR that reads to the same
; value (the cases at the end pin the equivalence), the way `a.b` is sugar for `(. a b)`; the
; canonical tree carries only `(Symbol.of …)`. `Symbol` is an ordinary prelude record bound in
; the prelude (like `Bytes`, `String`), so `Symbol.of` is `(. Symbol of)` and `Symbol.to-string`
; is `(. Symbol to-string)` — member access into a prelude record, not new core syntax.
;
; THE LOAD-BEARING CONSTRAINT — identity is CONTENT-DERIVED, never allocation order. A Symbol's
; identity (hence its equality and its canonical byte form) MUST be a deterministic function of its
; content, not of the order in which symbols were interned. The classic interning trick — hand out
; ids 0,1,2… in first-seen order and compare those — is FORBIDDEN here: such an id depends on
; evaluation order and would leak into observable behavior, violating deterministic-value-form.md
; (#A Value Has One Canonical Byte Form: equal values share one canonical form, derived from the
; value) and core-semantics.md #Equality Is Structural (equality agrees with the canonical byte
; form). The only observations a program can make of a Symbol are content-equality (`=`) and
; `Symbol.to-string`, both content-defined, so an allocation-order id is not merely forbidden but
; unobservable by construction. Interning — a runtime dedup table that lets `=` short-circuit to a
; handle compare — is therefore a pure representation optimization the runtime MAY perform
; invisibly (memory-and-resource-model.md #Sharing Is Not Observable): a Symbol built from a
; computed string and one built from a literal are ONE value, indistinguishable by every operation.
;
; Symbols are EQUALITY-ONLY in this version: `=` and use as a map key, no ordering (`<` `>` …). A
; content-lexicographic order MAY be added additively by a later decision if a sorted symbol table
; needs it; it is deliberately not pinned here (the equality path is the compiler's hot path).
;
; A later generation realizes the Symbol form (it is not on the ignition
; path — the seed clears ignition with `Int64`, `Bytes`, and `String`; options/realized-capability-set/).
; The seed does not realize it, so it DECLINES these cases — they pin the contract the
; realization must meet.

; ============================================================================================
; Construction and identity — Symbol.of interns a String; equality is by content
; ============================================================================================

(case "a symbol is constructed from a string"
  (doc    "`(Symbol.of \"map-insert\")` interns the string \"map-insert\" to a Symbol value — the
           nominal name value a symbol table is keyed on. This is the value the Cadenza-authored
           compiler interns an identifier or a node-kind name to. Its canonical written form is
           `(Symbol.of \"map-insert\")`, as a byte sequence's is `(Bytes.of (list …))`.")
  (input  (Symbol.of "map-insert"))
  (output (: (Symbol.of "map-insert") Symbol)))

(case "interning the same string twice yields equal symbols"
  (doc    "THE core case: `(= (Symbol.of \"map-insert\") (Symbol.of \"map-insert\"))` is true —
           interning the same string twice yields equal Symbols. This is the O(1) equality the form
           exists for: a realization interns to one shared identity so `=` is a handle compare, but
           the OBSERVABLE law is just that two symbols of the same content are equal (core-semantics.md
           #Equality Is Structural). Holds whether or not the two calls are the same call site.")
  (input  (= (Symbol.of "map-insert") (Symbol.of "map-insert")))
  (output (: true Bool)))

(case "symbols interned from different strings are unequal"
  (doc    "`(= (Symbol.of \"map-insert\") (Symbol.of \"map-lookup\"))` is false — Symbols of different
           content are distinct (the companion of the idempotence case). Pins that Symbol equality is a
           genuine content test, not a blanket true, so a symbol-table lookup distinguishes names.")
  (input  (= (Symbol.of "map-insert") (Symbol.of "map-lookup")))
  (output (: false Bool)))

(case "a symbol's identity is its content, not how the content was derived"
  (doc    "`(= (Symbol.of (String.concat \"map\" \"-insert\")) (Symbol.of \"map-insert\"))` is true: a
           Symbol interned from a COMPUTED string equals one interned from the literal of the same
           content. Pins that identity is content-derived, not derivation-path- or allocation-order-
           derived (memory-and-resource-model.md #Sharing Is Not Observable; deterministic-value-form.md
           #A Value Has One Canonical Byte Form) — a first-seen-order id would make these two distinct.")
  (input  (= (Symbol.of (String.concat "map" "-insert")) (Symbol.of "map-insert")))
  (output (: true Bool)))

(case "the boolean-literal coercion composes over a runtime Symbol equality"
  (doc    "The `(= bexpr true)` = bexpr / `(= bexpr false)` = ¬bexpr boolean coercion (03-equality) composes
           over a Symbol content-equality operand, exactly as it does over an Int `<`, a float `=`, and a
           String `=`. Over a runtime Symbol `s = #\"add\"` (built via `Symbol.of (String.concat …)` so it
           is not a constant fold): the inner `(= s #\"add\")` is the runtime Symbol content-eq (true), the
           outer `(= … true)` yields that Bool (→ then-arm 1) and `(= … false)` negates it (→ else-arm 0).
           `10*t + f` = 10*1 + 0 = 10. Pins the bool-literal coercion over a runtime SYMBOL equality (the
           symbol twin of the String/float `=` coercion cases), both backends.")
  (input  (do
            (def (t (: s Symbol)) (if (= (= s #"add") true) 1 0))
            (def (f (: s Symbol)) (if (= (= s #"add") false) 1 0))
            (def (main) (+ (* 10 (t (Symbol.of (String.concat "ad" "d"))))
                           (f (Symbol.of (String.concat "ad" "d")))))
            (export main)))
  (output (: 10 Int64)))

; ============================================================================================
; Crossing back to text — Symbol.to-string recovers the interned content
; ============================================================================================

(case "a symbol converts back to its content string"
  (doc    "`(Symbol.to-string (Symbol.of \"map-insert\"))` = \"map-insert\": a Symbol carries its
           content and hands it back as a String. This is the only way to observe a Symbol's content —
           together with `=` it is the whole observable surface, which is why an allocation-order id has
           nothing to attach to. The compiler uses it to render a name back for a diagnostic.")
  (input  (Symbol.to-string (Symbol.of "map-insert")))
  (output (: "map-insert" String)))

(case "symbol identity follows String normalization"
  (doc    "Because a Symbol wraps a String, symbol identity inherits String's normalized-contents
           equality (collections-and-text.md #String Equality Follows Normalized Contents): the composed
           \"café\" (…U+00E9) and the decomposed \"café\" (…e + U+0301) are the same text, so the
           Symbols interned from them are equal. This is the reason a Symbol is String-backed rather
           than Bytes-backed — two spellings of one source name intern to one symbol. Companion of the
           13-strings normalization cases, lifted through the Symbol tag.")
  (input  (= (Symbol.of "café") (Symbol.of "café")))
  (output (: true Bool)))

; ============================================================================================
; The empty symbol is an ordinary Symbol value (the degenerate boundary)
; ============================================================================================

(case "the empty symbol equals itself"
  (doc    "`(Symbol.of \"\")` interns the empty string to a Symbol — a first-class value equal only to
           another empty symbol. Pins that interning handles the zero-length name (an anonymous or
           generated name), the Symbol companion of the empty-string and empty-byte-sequence clusters.")
  (input  (= (Symbol.of "") (Symbol.of "")))
  (output (: true Bool)))

(case "the empty symbol converts to the empty string"
  (doc    "`(Symbol.to-string (Symbol.of \"\"))` = \"\": the empty symbol's content is the empty
           string. Pins that the round-trip through Symbol.to-string handles the zero-length content,
           not underflowing or reading a phantom scalar.")
  (input  (Symbol.to-string (Symbol.of "")))
  (output (: "" String)))

; ============================================================================================
; The payoff — a runtime Symbol compared by `=` (the symbol-table hot path)
; ============================================================================================
; Symbol equality is realized for a RUNTIME Symbol — one from a function parameter, a call, an `if` —
; not only for two compile-time constants, because that is the whole use: a compiler compares a Symbol
; carried at run time (an identifier read from the AST) against interned constants to dispatch. These
; pin that `=` on a runtime Symbol operand is a genuine constant-time content test.

(case "a runtime symbol compared to an interned constant matches by content"
  (doc    "`resolve` takes a Symbol parameter and compares it to the interned constant
           `(Symbol.of \"map-insert\")`; called with an equal symbol it returns 1. This is the
           symbol-table hot path — a name carried at run time compared against a known symbol by a
           handle compare rather than a byte scan. The equality is over a RUNTIME Symbol operand, not
           two constants.")
  (input  (do
            (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
            (def (main) (resolve (Symbol.of "map-insert"))) (export main)))
  (output (: 1 Int64)))

(case "a runtime symbol that differs from the interned constant does not match"
  (doc    "The companion with an unequal runtime operand: `resolve` called with `(Symbol.of \"other\")`
           compares it to `(Symbol.of \"map-insert\")` and returns 0. Confirms the runtime Symbol
           comparison is a genuine content test (1 for the matching name, 0 for a different one), not a
           blanket answer.")
  (input  (do
            (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
            (def (main) (resolve (Symbol.of "other"))) (export main)))
  (output (: 0 Int64)))

; ============================================================================================
; Symbol-LITERAL patterns — the `match` face of runtime-Symbol dispatch
; ============================================================================================
; The runtime-`=` cases above dispatch on a Symbol with an `if (= s #"…")` chain. The `match` sibling
; is a symbol-LITERAL pattern: `(match s (#"add" 1) (#"sub" 2) (_ 0))`. A Symbol shares the constant-
; string representation (`#"add"` is `(Symbol.of "add")`, a `Core::ConstStr`), so a symbol-literal
; pattern reuses the SAME machinery as a string-literal pattern (13-strings) — it classifies to a `Str`
; probe, folds against the constant, and emits a content `value-eq` test — with the probe's expected
; type set to `Symbol` (not `String`) so it agrees with a Symbol scrutinee across the nominal boundary
; (a Symbol is nominal over String). This is the pattern face of the `if (= s #"…")` chain, equivalent
; to it arm-for-arm.

(case "a runtime symbol matches a symbol-literal pattern arm by content"
  (doc    "`classify` matches a Symbol parameter against symbol-literal arms `(#\"add\" 1) (#\"sub\" 2)`;
           called with `#\"add\"` (built via `Symbol.of` over a runtime rope so it is not a constant fold)
           it takes the first arm → 1. The symbol-literal pattern is the `match` sibling of the
           `if (= s #\"add\")` dispatch: it classifies to a `Str` probe typed `Symbol` and emits a content
           value-eq test. Pins that a symbol scrutinee accepts a symbol-literal pattern.")
  (input  (do
            (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
            (def (main) (classify (Symbol.of (String.concat "ad" "d")))) (export main)))
  (output (: 1 Int64)))

(case "a runtime symbol not among the literal arms falls through to the wildcard"
  (doc    "The companion miss: `classify` called with a runtime `#\"xyz\"` matches neither `#\"add\"` nor
           `#\"sub\"` and falls through to the `_` arm → 0. Confirms the symbol-literal match is a genuine
           per-arm content dispatch (1 for a listed name, 0 for an unlisted one), not a blanket answer.")
  (input  (do
            (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
            (def (main) (classify (Symbol.of (String.concat "x" "yz")))) (export main)))
  (output (: 0 Int64)))

(case "a symbol-literal pattern nested in a variant payload matches by content"
  (doc    "The NESTED face: a sum whose payload is a Symbol (`(type W (Mk Symbol))`) matched with a
           symbol-literal payload sub-pattern `(Mk #\"add\")`. The pattern imposes the `Mk` discriminant
           AND a content lit-test on the payload — `f` called with `(Mk #\"add\")` (payload built at run
           time) fires the first arm → 1. Pins that the symbol-literal probe classifies as a nested payload
           sub-pattern (the `SumPayload`-position twin of the top-level cases), typed `Symbol`.")
  (input  (do
            (type W (Mk Symbol))
            (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
            (def (main) (f (Mk (Symbol.of (String.concat "ad" "d"))))) (export main)))
  (output (: 1 Int64)))

(case "a nested symbol-literal payload falls through on a non-matching symbol"
  (doc    "The nested miss: `f` called with `(Mk #\"sub\")` does not match the `(Mk #\"add\")` arm and
           takes the `(Mk _)` fall-through → 0. Confirms the nested symbol-literal test is a genuine
           content compare, the companion of the nested-hit case.")
  (input  (do
            (type W (Mk Symbol))
            (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
            (def (main) (f (Mk (Symbol.of (String.concat "su" "b"))))) (export main)))
  (output (: 0 Int64)))

; The landed symbol-literal cases pin the first-arm match, the wildcard miss, and a nested-payload match.
; These pin the neighbors: hitting the SECOND arm (arms are tried by CONTENT, so a symbol-literal match is
; order-independent across disjoint literals), the equivalence to the `if (= s lit)` CHAIN the doc names as
; the sibling dispatch, and a symbol-literal pattern over a MULTI-BYTE symbol (content-match, not a byte or
; ASCII assumption). All build the scrutinee via `Symbol.of` over a runtime rope so it is not a constant fold.

(case "a symbol-literal match reaches the second arm by content"
  (doc    "`classify` matches a Symbol against `(#\"add\" 1) (#\"sub\" 2) (_ 0)`; called with a runtime
           `#\"sub\"` it takes the SECOND arm → 2. Pins that symbol-literal arms are tried by content across
           the whole arm list (order-independent for disjoint literals), not just the first — the miss-past-
           the-first-arm companion of the landed first-arm-match case.")
  (input  (do
            (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
            (def (main) (classify (Symbol.of (String.concat "su" "b"))))
            (export main)))
  (output (: 2 Int64)))

(case "a symbol-literal match agrees with the equivalent if-(= s lit) chain"
  (doc    "The symbol-literal `match` is the sibling of an `if (= s lit)` dispatch chain (the landed doc's
           framing): over the same runtime `#\"sub\"`, `(match s (#\"add\" 1) (#\"sub\" 2) (_ 0))` and
           `(if (= s #\"add\") 1 (if (= s #\"sub\") 2 0))` both give 2, so their difference is 0. Pins that
           the symbol-literal match desugars to the same content-eq dispatch as the explicit chain — a
           regression that changed the arm-selection order or the eq test would make them disagree.")
  (input  (do
            (def (via-match (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
            (def (via-chain (: s Symbol)) (if (= s #"add") 1 (if (= s #"sub") 2 0)))
            (def (main) (let ((s (Symbol.of (String.concat "su" "b")))) (- (via-match s) (via-chain s))))
            (export main)))
  (output (: 0 Int64)))

(case "a symbol-literal match agrees with its if-(= s lit) chain on the DEFAULT fall-through arm"
  (doc    "The match≡chain case above tests a HIT arm (`#\"sub\"`); this pins the DEFAULT face. A symbol
           matching NO literal arm (`#\"xyz\"`, built at runtime) must take the wildcard `(_ 9)` in the
           match AND the trailing `else 9` in the chain — the arm a dropped-wildcard or a diverged
           desugaring would get wrong (the hit case cannot witness the fall-through). `(match s (#\"add\"
           1) (#\"sub\" 2) (_ 9))` and `(if (= s #\"add\") 1 (if (= s #\"sub\") 2 9))` both give 9, so
           their difference is 0. Completes the symbol match≡=-chain equivalence (hit + default), the
           symbol twin of the String fall-through pin.")
  (input  (do
            (def (via-match (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 9)))
            (def (via-chain (: s Symbol)) (if (= s #"add") 1 (if (= s #"sub") 2 9)))
            (def (main) (let ((s (Symbol.of (String.concat "x" "yz")))) (- (via-match s) (via-chain s))))
            (export main)))
  (output (: 0 Int64)))

(case "a symbol-literal pattern over a multi-byte symbol matches by content"
  (doc    "`(match s (#\"café\" 1) (_ 0))` with a runtime `#\"café\"` (é = 2 UTF-8 bytes) takes the literal
           arm → 1. Pins that the symbol-literal content test compares the full UTF-8 byte content, not an
           ASCII or byte-length assumption — a multi-byte symbol matches its multi-byte literal exactly, the
           symbol-pattern companion of the multi-byte `Symbol.of` equality.")
  (input  (do
            (def (classify (: s Symbol)) (match s (#"café" 1) (_ 0)))
            (def (main) (classify (Symbol.of "café")))
            (export main)))
  (output (: 1 Int64)))

; ============================================================================================
; Symbol-keyed membership — the symbol-table dispatch the form exists for
; ============================================================================================
; The header's motivation (a self-hosting compiler keys node-kind dispatch and scope resolution on
; Symbols so a name test is a handle compare, not a byte scan) shows up as a MEMBERSHIP test: is this
; runtime Symbol one of a known set of names? `Set.contains` over a set of interned constants, queried
; with a RUNTIME Symbol operand, is exactly that dispatch. These pin it as a genuine content test on a
; runtime operand — the realized companion of the runtime-`=` cases above, lifted to a set of names.
; (A Symbol stored INTO the value heap — `Set.of`/`Set.len` materializing a symbol set, a Symbol map
; key — still declines: "a … element of type Symbol needs the value heap", the not-yet-realized part
; of the form. `Set.contains` against a constant symbol list lowers to realized Symbol `=` checks, so
; it runs; these cases pin that realized slice, and the heap-Symbol cases will join them additively.)

(case "a runtime symbol is found among a set of known symbols"
  (doc    "`dispatch` takes a Symbol parameter and asks whether it is one of the known node-kind names
           `{map-insert, map-lookup}` via `Set.contains`; called with `map-lookup` it returns true. This
           is the symbol-table dispatch the form exists for — a name carried at run time tested for
           membership in a fixed set of interned constants by content, a handle compare rather than a
           byte scan. The membership test is over a RUNTIME Symbol operand (a parameter), not a
           constant, the set-lifted companion of \"a runtime symbol compared to an interned constant
           matches by content\".")
  (input  (do
            (def (dispatch s) (Set.contains (Set.of (list (Symbol.of "map-insert") (Symbol.of "map-lookup"))) s))
            (def (main) (dispatch (Symbol.of "map-lookup"))) (export main)))
  (output (: true Bool)))

(case "a runtime symbol not among the known symbols is rejected"
  (doc    "The companion with an unknown name: `dispatch` called with `other` — not one of the known
           node-kind names — returns false. Confirms `Set.contains` on a runtime Symbol is a genuine
           content test (true for a member, false for a non-member), not a blanket answer, so an
           unrecognized identifier is distinguished from a known one.")
  (input  (do
            (def (dispatch s) (Set.contains (Set.of (list (Symbol.of "map-insert") (Symbol.of "map-lookup"))) s))
            (def (main) (dispatch (Symbol.of "other"))) (export main)))
  (output (: false Bool)))

(case "membership of a runtime symbol is by content, not derivation"
  (doc    "`dispatch` is queried with a Symbol interned from the COMPUTED string
           `(String.concat \"map\" \"-insert\")`; it is found in the known set that holds
           `(Symbol.of \"map-insert\")`, so the result is true. Pins that set membership follows the
           content-derived identity (memory-and-resource-model.md #Sharing Is Not Observable) — the
           membership analogue of \"a symbol's identity is its content, not how the content was
           derived\" — so a name assembled at run time still dispatches to the same table entry.")
  (input  (do
            (def (dispatch s) (Set.contains (Set.of (list (Symbol.of "map-insert") (Symbol.of "map-lookup"))) s))
            (def (main) (dispatch (Symbol.of (String.concat "map" "-insert")))) (export main)))
  (output (: true Bool)))

; The case above interns a Symbol from a string the compiler can still FOLD (`(String.concat "map"
; "-insert")` = the constant `"map-insert"`). Interning a GENUINELY-RUNTIME string — one arriving at
; the call boundary, unfoldable — also works: a Symbol IS a String byte-leaf at run time (the value
; heap is tagless; a Symbol has no separate intern table, it compares via its physical bytes like a
; String), so `Symbol.of` on a runtime string CANONICALIZES its byte-rope to a flat leaf, and two
; symbols of equal content compare equal because both are canonical. That IS interning under a
; by-content representation — no runtime `str-intern` op needed. These pin the runtime-string→Symbol
; path (the intern analogue of the runtime String.slice byte-walk).
(case "a runtime string interns to a symbol matched by content"
  (doc    "`Symbol.of` on a GENUINELY-RUNTIME string — built by the `rep` concat loop `(rep \"\" 3)` =
           \"xxx\", a byte-rope the compiler cannot fold — interns it, and the resulting Symbol compares
           EQUAL to the constant `#\"xxx\"` of the same content: a runtime Symbol is a canonical byte leaf
           compared by its bytes, not a compile-time intern id. Pins that a name assembled from genuinely-
           runtime data still dispatches to the same identity (the intern analogue of runtime String.slice).")
  (input  (do (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
              (def (main) (= (Symbol.of (rep "" 3)) #"xxx")) (export main)))
  (output (: true Bool)))

(case "a runtime symbol round-trips back to its string"
  (doc    "`Symbol.to-string (Symbol.of s)` on a runtime string `s` recovers the SAME content String —
           both directions are a byte-leaf retag (a Symbol and its String share the tagless rep). `s` is
           the runtime rope `(rep \"xx\" 3)` = \"xxxxx\"; observed by the recovered string's byte length
           (5), exercising both runtime retags in one chain. The inverse of the intern above.")
  (input  (do (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
              (def (main) (String.byte-len (Symbol.to-string (Symbol.of (rep "xx" 3))))) (export main)))
  (output (: 5 Int64)))

; ============================================================================================
; The nominal boundary — a Symbol is not comparable to the underlying String (CDZ0202)
; ============================================================================================
; A Symbol is a NOMINAL value over String (type-system.md #User Types Are Declarable As Nominal Or
; Structural): it carries an orthogonal tag over the structural String it wraps, so a Symbol and the
; untagged String of the same content are declared distinct and comparing them is a type error
; (CDZ0202), exactly as comparing a nominal record to the plain record of its shape is (05-compound-
; types). Crossing the boundary is explicit — `Symbol.to-string` on one side or `Symbol.of` on the
; other — never an implicit unification. A generation that does not yet track the Symbol tag in
; comparison declines (reject-don't-miscompile), not miscompiles.

(case "a symbol compared to a string is a type error"
  (doc    "`(= (Symbol.of \"x\") \"x\")` compares a Symbol to the untagged String of the same content —
           across the nominal boundary — so the compiler rejects it (CDZ0202, type-system.md #Nominal
           Types Are Not Comparable Across Their Boundary). A Symbol never silently compares equal to
           the String it was interned from; to compare content you write
           `(= (Symbol.to-string s) \"x\")`.")
  (input  (= (Symbol.of "x") "x"))
  (error  CDZ0202))

(case "a string compared to a symbol is a type error"
  (doc    "The order-flipped companion: `(= \"x\" (Symbol.of \"x\"))` is the same nominal-boundary
           violation regardless of which operand carries the Symbol tag — CDZ0202. Pins that the tag is
           checked on either side of the comparison, mirroring the nominal-record boundary cases.")
  (input  (= "x" (Symbol.of "x")))
  (error  CDZ0202))

; The nominal boundary holds in a `match` PATTERN too, not just `=`. A String and a Symbol literal
; pattern share the `Core::ConstStr` rep, but the two types are distinct across the nominal boundary, so
; a text-literal pattern must match a scrutinee of its OWN kind: a `"add"` (String) pattern over a Symbol
; scrutinee — or a `#"add"` (Symbol) pattern over a String scrutinee — is a pattern/scrutinee type
; mismatch (CDZ0201, the general shape/type-mismatch code the char/bool-over-int pattern cases carry),
; the pattern-path sibling of the `=` CDZ0202 above. Pins that the pattern path is NOT more permissive
; than `=` on this boundary — the type comes from the PATTERN's kind, not the scrutinee. (The check is
; structural: it holds even for a CONSTANT scrutinee, before any fold.)

(case "a string-literal pattern over a symbol scrutinee is a type error"
  (doc    "`(match (Symbol.of \"x\") (\"add\" 1) (_ 0))` matches a Symbol scrutinee against a STRING literal
           pattern `\"add\"` — across the nominal boundary — so the compiler rejects it (CDZ0201, the
           pattern/scrutinee type-mismatch code). The `match` sibling of `(= (Symbol.of \"x\") \"add\")` →
           CDZ0202: a symbol-literal `#\"add\"` matches a Symbol, a string-literal `\"add\"` matches a
           String, and the two do not cross. Pins the pattern path respects the Symbol↔String boundary.")
  (input  (match (Symbol.of "x") ("add" 1) (_ 0)))
  (error  CDZ0201))

(case "a symbol-literal pattern over a string scrutinee is a type error"
  (doc    "The order-flipped companion: `(match \"x\" (#\"add\" 1) (_ 0))` matches a String scrutinee against
           a SYMBOL literal pattern `#\"add\"` — the same nominal-boundary violation with the tag on the
           pattern side — CDZ0201. Pins that the boundary is checked whichever kind the pattern carries, the
           pattern-path mirror of the two `=` CDZ0202 cases above.")
  (input  (match "x" (#"add" 1) (_ 0)))
  (error  CDZ0201))

; The scalar boundary above pins the top-level text-literal pattern. The type comes from the PATTERN's
; kind, so the same CDZ0201 must fire wherever a text-literal pattern sits — a NESTED sum-payload sub-
; pattern, a SIBLING arm in an otherwise same-kind match, and a TUPLE-element sub-pattern — not only the
; top-level scalar position. (The scalar fix keyed `pat_ty` on the pattern's origin; these pin that the
; nested-payload and tuple-element positions do the same, and that a per-arm mix does not let one crossing
; arm slip through because a sibling arm is same-kind.) A same-kind control alongside each proves the
; boundary check does not over-reject the legitimate case.

(case "a string-literal pattern in a Symbol sum payload is a type error"
  (doc    "The NESTED-payload face of the boundary: `(type W (Mk Symbol))` matched with a STRING-literal
           payload sub-pattern `(Mk \"add\")` over a Symbol payload crosses the nominal boundary → CDZ0201,
           the sum-payload twin of the top-level `\"add\"`-over-Symbol case. Pins the payload sub-pattern
           keys its expected type on the PATTERN kind (String) too, not the Symbol payload type — the same
           discipline as `pattern_constraints` for the nested position.")
  (input  (do
            (type W (Mk Symbol))
            (def (f (: w W)) (match w ((Mk "add") 1) ((Mk _) 0)))
            (def (main) (f (Mk (Symbol.of "add")))) (export main)))
  (error  CDZ0201))

(case "a symbol-literal pattern in a String sum payload is a type error"
  (doc    "The order-flipped nested companion: `(type W (Mk String))` with a SYMBOL-literal payload sub-
           pattern `(Mk #\"add\")` over a String payload → CDZ0201. Pins the nested-payload boundary holds
           whichever kind the pattern carries, the payload twin of the flipped top-level case.")
  (input  (do
            (type W (Mk String))
            (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
            (def (main) (f (Mk (String.concat "ad" "d")))) (export main)))
  (error  CDZ0201))

(case "a same-kind symbol-literal payload sub-pattern still dispatches (nested control)"
  (doc    "The nested control that must NOT over-reject: `(Mk #\"add\")` over a Symbol payload is same-kind,
           so it dispatches — `f (Mk #\"add\")` → 1. Pins the nested-payload boundary check rejects only the
           crossing case, not the legitimate same-kind one, alongside the two nested rejects above.")
  (input  (do
            (type W (Mk Symbol))
            (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
            (def (main) (f (Mk (Symbol.of "add")))) (export main)))
  (output (: 1 Int64)))

(case "a crossing text-literal arm is rejected even beside a same-kind sibling arm"
  (doc    "The per-arm face: a Symbol scrutinee with a same-kind first arm `#\"add\"` AND a crossing STRING
           sibling arm `\"sub\"` — `(match s (#\"add\" 1) (\"sub\" 2) (_ 0))` — still faults CDZ0201. Pins
           the expected type is keyed per-arm on each pattern's origin (`probe_pats[i]`), so a legitimate
           same-kind arm does not let a crossing sibling arm slip through — a whole-match check that keyed on
           the scrutinee, or only the first arm, would miss this.")
  (input  (do
            (def (classify (: s Symbol)) (match s (#"add" 1) ("sub" 2) (_ 0)))
            (def (main) (classify (Symbol.of "add"))) (export main)))
  (error  CDZ0201))

(case "the mirror per-arm mix — a crossing #sym sibling over a String scrutinee — is rejected"
  (doc    "The flipped per-arm case: a String scrutinee with a same-kind `\"add\"` arm and a crossing SYMBOL
           sibling arm `#\"sub\"` → CDZ0201. Pins the per-arm boundary check fires whichever kind the
           crossing arm carries, the mirror of the case above.")
  (input  (do
            (def (classify (: s String)) (match s ("add" 1) (#"sub" 2) (_ 0)))
            (def (main) (classify (String.concat "ad" "d"))) (export main)))
  (error  CDZ0201))

(case "a string-literal sub-pattern in a Symbol tuple element is a type error"
  (doc    "The TUPLE-element face: `(: p (Tuple Symbol Int64))` matched with `(tuple \"add\" n)` — a STRING
           literal over the Symbol element — crosses the boundary → CDZ0201. Pins the tuple-element position
           keys its expected type on the PATTERN kind too, the positional twin of the sum-payload nested
           cases.")
  (input  (do
            (def (f (: p (Tuple Symbol Int64))) (match p ((tuple "add" n) n) ((tuple _ n) 0)))
            (def (main) (f (tuple (Symbol.of "add") 5))) (export main)))
  (error  CDZ0201))

(case "a same-kind symbol-literal tuple sub-pattern still dispatches (tuple control)"
  (doc    "The tuple control: `(tuple #\"add\" n)` over a Symbol element is same-kind, so it dispatches —
           `f (tuple #\"add\" 5)` binds n=5 → 5. Pins the tuple-element boundary check rejects only the
           crossing case, the same-kind companion of the tuple reject above.")
  (input  (do
            (def (f (: p (Tuple Symbol Int64))) (match p ((tuple #"add" n) n) ((tuple _ n) 0)))
            (def (main) (f (tuple (Symbol.of "add") 5))) (export main)))
  (output (: 5 Int64)))

; ============================================================================================
; The reader literal — #"<text>" reads to (Symbol.of "<text>")
; ============================================================================================
; `#"<text>"` is reader sugar for `(Symbol.of "<text>")`: a `#` immediately before a string literal
; reads to a Symbol node, reusing String's existing lexing (including its escape rules), so the sugar
; introduces no new escape or token grammar. The CANONICAL literal is the string FORM (`#"…"`) — it
; interns arbitrary content, including a qualified name with a dot — and it does not use the `'` sigil,
; which is the natural shorthand for `quote` in this homoiconic language (12-metaprogramming). The ML
; SURFACE additionally accepts the bare-token spelling `#name` as a convenience when the content is a
; plain identifier (`#map-insert` == `#"map-insert"`); it is purely a surface projection — the quotes
; are required there whenever the content is not an identifier (a space, a leading digit, a dot), and
; the s-expr surface and the canonical tree carry only `#"…"` / `(Symbol.of …)`, as the tree carries
; `(. a b)` for the display sugar `a.b`.

(case "the reader literal reads to Symbol.of"
  (doc    "`#\"map-insert\"` reads to `(Symbol.of \"map-insert\")`, so the two denote one Symbol value
           and `(= #\"map-insert\" (Symbol.of \"map-insert\"))` is true. Pins the reader sugar against
           the canonical form it expands to.")
  (input  (= #"map-insert" (Symbol.of "map-insert")))
  (output (: true Bool)))

(case "a reader literal carries a qualified name with a dot"
  (doc    "`#\"List.at\"` interns the string \"List.at\" — a qualified name whose dot the bare-token
           form (`#name`, the ML surface's identifier-only convenience) could not carry unambiguously
           (it would read as member access) — so `(= #\"List.at\" (Symbol.of \"List.at\"))` is true.
           Pins that the string-form literal interns arbitrary content, the reason the canonical form
           is `#\"…\"` and the bare `#name` sugar is confined to plain identifiers.")
  (input  (= #"List.at" (Symbol.of "List.at")))
  (output (: true Bool)))

(case "the empty reader literal is the empty symbol"
  (doc    "`#\"\"` reads to `(Symbol.of \"\")`, the empty symbol — `(= #\"\" (Symbol.of \"\"))` is true.
           Pins that the reader sugar handles the zero-length case, the degenerate boundary of the
           literal form.")
  (input  (= #"" (Symbol.of "")))
  (output (: true Bool)))
