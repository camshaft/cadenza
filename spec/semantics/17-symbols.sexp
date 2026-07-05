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
; Tagged `(needs symbols)`: a later generation realizes the Symbol form (it is not on the ignition
; path — the seed clears ignition with `Int64`, `Bytes`, and `String`; options/realized-capability-set/).
; The seed does not realize it, so its behavior gate SKIPS these cases — they pin the contract the
; realization must meet, they are not seed declines.

; ============================================================================================
; Construction and identity — Symbol.of interns a String; equality is by content
; ============================================================================================

(case "a symbol is constructed from a string"
  (doc    "`(Symbol.of \"map-insert\")` interns the string \"map-insert\" to a Symbol value — the
           nominal name value a symbol table is keyed on. This is the value the Cadenza-authored
           compiler interns an identifier or a node-kind name to. Its canonical written form is
           `(Symbol.of \"map-insert\")`, as a byte sequence's is `(Bytes.of (list …))`.")
  (needs  symbols)
  (input  (Symbol.of "map-insert"))
  (output (: (Symbol.of "map-insert") Symbol)))

(case "interning the same string twice yields equal symbols"
  (doc    "THE core case: `(= (Symbol.of \"map-insert\") (Symbol.of \"map-insert\"))` is true —
           interning the same string twice yields equal Symbols. This is the O(1) equality the form
           exists for: a realization interns to one shared identity so `=` is a handle compare, but
           the OBSERVABLE law is just that two symbols of the same content are equal (core-semantics.md
           #Equality Is Structural). Holds whether or not the two calls are the same call site.")
  (needs  symbols)
  (input  (= (Symbol.of "map-insert") (Symbol.of "map-insert")))
  (output (: true Bool)))

(case "symbols interned from different strings are unequal"
  (doc    "`(= (Symbol.of \"map-insert\") (Symbol.of \"map-lookup\"))` is false — Symbols of different
           content are distinct (the companion of the idempotence case). Pins that Symbol equality is a
           genuine content test, not a blanket true, so a symbol-table lookup distinguishes names.")
  (needs  symbols)
  (input  (= (Symbol.of "map-insert") (Symbol.of "map-lookup")))
  (output (: false Bool)))

(case "a symbol's identity is its content, not how the content was derived"
  (doc    "`(= (Symbol.of (String.concat \"map\" \"-insert\")) (Symbol.of \"map-insert\"))` is true: a
           Symbol interned from a COMPUTED string equals one interned from the literal of the same
           content. Pins that identity is content-derived, not derivation-path- or allocation-order-
           derived (memory-and-resource-model.md #Sharing Is Not Observable; deterministic-value-form.md
           #A Value Has One Canonical Byte Form) — a first-seen-order id would make these two distinct.")
  (needs  symbols)
  (input  (= (Symbol.of (String.concat "map" "-insert")) (Symbol.of "map-insert")))
  (output (: true Bool)))

; ============================================================================================
; Crossing back to text — Symbol.to-string recovers the interned content
; ============================================================================================

(case "a symbol converts back to its content string"
  (doc    "`(Symbol.to-string (Symbol.of \"map-insert\"))` = \"map-insert\": a Symbol carries its
           content and hands it back as a String. This is the only way to observe a Symbol's content —
           together with `=` it is the whole observable surface, which is why an allocation-order id has
           nothing to attach to. The compiler uses it to render a name back for a diagnostic.")
  (needs  symbols)
  (input  (Symbol.to-string (Symbol.of "map-insert")))
  (output (: "map-insert" String)))

(case "symbol identity follows String normalization"
  (doc    "Because a Symbol wraps a String, symbol identity inherits String's normalized-contents
           equality (collections-and-text.md #String Equality Follows Normalized Contents): the composed
           \"café\" (…U+00E9) and the decomposed \"café\" (…e + U+0301) are the same text, so the
           Symbols interned from them are equal. This is the reason a Symbol is String-backed rather
           than Bytes-backed — two spellings of one source name intern to one symbol. Companion of the
           13-strings normalization cases, lifted through the Symbol tag.")
  (needs  symbols)
  (input  (= (Symbol.of "café") (Symbol.of "café")))
  (output (: true Bool)))

; ============================================================================================
; The empty symbol is an ordinary Symbol value (the degenerate boundary)
; ============================================================================================

(case "the empty symbol equals itself"
  (doc    "`(Symbol.of \"\")` interns the empty string to a Symbol — a first-class value equal only to
           another empty symbol. Pins that interning handles the zero-length name (an anonymous or
           generated name), the Symbol companion of the empty-string and empty-byte-sequence clusters.")
  (needs  symbols)
  (input  (= (Symbol.of "") (Symbol.of "")))
  (output (: true Bool)))

(case "the empty symbol converts to the empty string"
  (doc    "`(Symbol.to-string (Symbol.of \"\"))` = \"\": the empty symbol's content is the empty
           string. Pins that the round-trip through Symbol.to-string handles the zero-length content,
           not underflowing or reading a phantom scalar.")
  (needs  symbols)
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
  (needs  symbols)
  (input  (module m
            (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
            (def (main) (resolve (Symbol.of "map-insert")))))
  (output (: 1 Int64)))

(case "a runtime symbol that differs from the interned constant does not match"
  (doc    "The companion with an unequal runtime operand: `resolve` called with `(Symbol.of \"other\")`
           compares it to `(Symbol.of \"map-insert\")` and returns 0. Confirms the runtime Symbol
           comparison is a genuine content test (1 for the matching name, 0 for a different one), not a
           blanket answer.")
  (needs  symbols)
  (input  (module m
            (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
            (def (main) (resolve (Symbol.of "other")))))
  (output (: 0 Int64)))

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
  (needs  symbols)
  (input  (= (Symbol.of "x") "x"))
  (error  CDZ0202))

(case "a string compared to a symbol is a type error"
  (doc    "The order-flipped companion: `(= \"x\" (Symbol.of \"x\"))` is the same nominal-boundary
           violation regardless of which operand carries the Symbol tag — CDZ0202. Pins that the tag is
           checked on either side of the comparison, mirroring the nominal-record boundary cases.")
  (needs  symbols)
  (input  (= "x" (Symbol.of "x")))
  (error  CDZ0202))

; ============================================================================================
; The reader literal — #"<text>" reads to (Symbol.of "<text>")
; ============================================================================================
; `#"<text>"` is reader sugar for `(Symbol.of "<text>")`: a `#` immediately before a string literal
; reads to a Symbol node, reusing String's existing lexing (including its escape rules), so the sugar
; introduces no new escape or token grammar. It is the string FORM (`#"…"`), not a bare-token form
; (`#foo`) — so it interns arbitrary content, including a qualified name with a dot — and it does not
; use the `'` sigil, which is the natural shorthand for `quote` in this homoiconic language
; (12-metaprogramming). The canonical tree carries only `(Symbol.of …)`, as it carries `(. a b)` for
; the display sugar `a.b`.

(case "the reader literal reads to Symbol.of"
  (doc    "`#\"map-insert\"` reads to `(Symbol.of \"map-insert\")`, so the two denote one Symbol value
           and `(= #\"map-insert\" (Symbol.of \"map-insert\"))` is true. Pins the reader sugar against
           the canonical form it expands to.")
  (needs  symbols)
  (input  (= #"map-insert" (Symbol.of "map-insert")))
  (output (: true Bool)))

(case "a reader literal carries a qualified name with a dot"
  (doc    "`#\"List.at\"` interns the string \"List.at\" — a qualified name whose dot the bare-token
           form could not carry unambiguously (it would read as member access) — so
           `(= #\"List.at\" (Symbol.of \"List.at\"))` is true. Pins that the string-form literal
           interns arbitrary content, the reason it is `#\"…\"` rather than `#foo`.")
  (needs  symbols)
  (input  (= #"List.at" (Symbol.of "List.at")))
  (output (: true Bool)))

(case "the empty reader literal is the empty symbol"
  (doc    "`#\"\"` reads to `(Symbol.of \"\")`, the empty symbol — `(= #\"\" (Symbol.of \"\"))` is true.
           Pins that the reader sugar handles the zero-length case, the degenerate boundary of the
           literal form.")
  (needs  symbols)
  (input  (= #"" (Symbol.of "")))
  (output (: true Bool)))
