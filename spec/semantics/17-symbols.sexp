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
; Symbols support `=`, use as a map key, AND a total ORDER by content-lexicographic UTF-8 byte order
; (`<` `<=` `>` `>=` compare the interned bytes lexicographically). This is the additive content-
; lexicographic order the earlier equality-only version deferred to a "later decision" — now blessed
; (the order is well-defined because a Symbol's identity is its content, and it is backend-consistent:
; rust represents a Symbol as a String whose UTF-8 byte Ord agrees with the wasm byte-leaf order,
; including on multibyte/non-ASCII pairs). Equality remains the compiler's hot path.
;
; A later generation realizes the Symbol form (it is not on the ignition
; path — the seed clears ignition with `Int64`, `Bytes`, and `String`; options/realized-capability-set/).
; The seed does not realize it, so it DECLINES these cases — they pin the contract the
; realization must meet.
; ============================================================================================
; Construction and identity — Symbol.of interns a String; equality is by content
; ============================================================================================
(case
  "a symbol is constructed from a string"
  (doc
    "`(Symbol.of \"map-insert\")` interns the string \"map-insert\" to a Symbol value — the
           nominal name value a symbol table is keyed on. This is the value the Cadenza-authored
           compiler interns an identifier or a node-kind name to. Its canonical written form is
           `(Symbol.of \"map-insert\")`, as a byte sequence's is `(Bytes.of (list …))`.")
  (input (Symbol.of "map-insert"))
  (output (: (Symbol.of "map-insert") Symbol)))

(case
  "interning the same string twice yields equal symbols"
  (doc
    "THE core case: `(= (Symbol.of \"map-insert\") (Symbol.of \"map-insert\"))` is true —
           interning the same string twice yields equal Symbols. This is the O(1) equality the form
           exists for: a realization interns to one shared identity so `=` is a handle compare, but
           the OBSERVABLE law is just that two symbols of the same content are equal (core-semantics.md
           #Equality Is Structural). Holds whether or not the two calls are the same call site.")
  (input (= (Symbol.of "map-insert") (Symbol.of "map-insert")))
  (output (: true Bool)))

(case
  "symbols interned from different strings are unequal"
  (doc
    "`(= (Symbol.of \"map-insert\") (Symbol.of \"map-lookup\"))` is false — Symbols of different
           content are distinct (the companion of the idempotence case). Pins that Symbol equality is a
           genuine content test, not a blanket true, so a symbol-table lookup distinguishes names.")
  (input (= (Symbol.of "map-insert") (Symbol.of "map-lookup")))
  (output (: false Bool)))

(case
  "symbols order by content-lexicographic UTF-8 byte order — less-than"
  (doc
    "`(< (Symbol.of \"a\") (Symbol.of \"b\"))` is true — Symbols carry a total order by the
           content-lexicographic UTF-8 byte order of their interned bytes ('a'=0x61 < 'b'=0x62). This is
           the additive content-lexicographic order the earlier equality-only version deferred to a
           'later decision' — now blessed: the order is well-defined precisely because a Symbol's
           identity is its content (see the load-bearing content-identity constraint above), and it is
           backend-consistent (rust represents a Symbol as a String whose UTF-8 byte Ord agrees with the
           wasm byte-leaf order on multibyte/non-ASCII pairs).")
  (input (< (Symbol.of "a") (Symbol.of "b")))
  (output (: true Bool)))

(case
  "symbol ordering is reflexive at the boundary — less-than-or-equal on equal content"
  (doc
    "`(<= (Symbol.of \"a\") (Symbol.of \"a\"))` is true — `<=` includes equality, and two Symbols
           of the same content compare equal-under-order (consistent with `=`). Pins the reflexive edge
           of the content-lexicographic order.")
  (input (<= (Symbol.of "a") (Symbol.of "a")))
  (output (: true Bool)))

(case
  "symbol ordering — greater-than agrees with the byte order"
  (doc
    "`(> (Symbol.of \"b\") (Symbol.of \"a\"))` is true — `>` is the mirror of `<` over the same
           content-lexicographic UTF-8 byte order ('b'=0x62 > 'a'=0x61). Pins that the ordering ops are
           mutually consistent, not independently defined.")
  (input (> (Symbol.of "b") (Symbol.of "a")))
  (output (: true Bool)))

(case
  "symbol ordering — greater-than-or-equal is false when strictly less"
  (doc
    "`(>= (Symbol.of \"a\") (Symbol.of \"b\"))` is false — 'a' is strictly less than 'b' under the
           content-lexicographic byte order, so `>=` does not hold. The negative companion pinning that
           all four relational ops evaluate consistently over the same total order.")
  (input (>= (Symbol.of "a") (Symbol.of "b")))
  (output (: false Bool)))

(case
  "a genuinely-runtime symbol orders by content-lexicographic byte order"
  (doc
    "The cases above compare CONSTANT symbols (folded before emit). Forcing genuinely-runtime symbols —
           `(Symbol.of (String.concat s \"\"))` off a parameter — makes the backend WALK the interned byte
           leaves rather than fold: `(< (mk \"alpha\") (mk \"beta\"))` is true ('a'=0x61 < 'b'=0x62) → 1. Pins
           the runtime Symbol-ordering emit (the byte-lexicographic leaf walk, backend-consistent: rust's
           String Ord agrees with the wasm byte-leaf order), distinct from the constant-fold cases above.")
  (input
    (do
      (def (mk (: s String)) (Symbol.of (String.concat s "")))
      (def (main) (if (< (mk "alpha") (mk "beta")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a genuinely-runtime symbol three-way compare agrees with the boolean ordering"
  (doc
    "The three-way `(Ordering.of x y)` over genuinely-runtime Symbols (`Symbol.of (String.concat s \"\")`
           off a parameter, so no fold) yields the Ordering variant matching the boolean `<` above:
           `(Ordering.of (mk \"alpha\") (mk \"beta\"))` is `Less` → 1 ('a'=0x61 < 'b'=0x62). core-semantics.md
           #A Total Order Is Observed Through A Three-Way Comparison (the boolean ordering operators MUST
           agree with the three-way comparison): the runtime Symbol compare desugars to the nested-if over
           the SAME `Core::StrCmp` content-lexicographic byte walk the boolean `<` emits — no new runtime
           op, both backends agree. The three-way twin of the runtime-Symbol `<` case above.")
  (input
    (do
      (def (mk (: s String)) (Symbol.of (String.concat s "")))
      (def
        (cmp (: x Symbol) (: y Symbol))
        (match
          (Ordering.of x y)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp (mk "alpha") (mk "beta")))
      (export main)))
  (output (: 1 Int64)))

(case
  "a symbol's identity is its content, not how the content was derived"
  (doc
    "`(= (Symbol.of (String.concat \"map\" \"-insert\")) (Symbol.of \"map-insert\"))` is true: a
           Symbol interned from a COMPUTED string equals one interned from the literal of the same
           content. Pins that identity is content-derived, not derivation-path- or allocation-order-
           derived (memory-and-resource-model.md #Sharing Is Not Observable; deterministic-value-form.md
           #A Value Has One Canonical Byte Form) — a first-seen-order id would make these two distinct.")
  (input (= (Symbol.of (String.concat "map" "-insert")) (Symbol.of "map-insert")))
  (output (: true Bool)))

(case
  "the boolean-literal coercion composes over a runtime Symbol equality"
  (doc
    "The `(= bexpr true)` = bexpr / `(= bexpr false)` = ¬bexpr boolean coercion (03-equality) composes
           over a Symbol content-equality operand, exactly as it does over an Int `<`, a float `=`, and a
           String `=`. Over a runtime Symbol `s = #\"add\"` (built via `Symbol.of (String.concat …)` so it
           is not a constant fold): the inner `(= s #\"add\")` is the runtime Symbol content-eq (true), the
           outer `(= … true)` yields that Bool (→ then-arm 1) and `(= … false)` negates it (→ else-arm 0).
           `10*t + f` = 10*1 + 0 = 10. Pins the bool-literal coercion over a runtime SYMBOL equality (the
           symbol twin of the String/float `=` coercion cases), both backends.")
  (input
    (do
      (def (t (: s Symbol)) (if (= (= s #"add") true) 1 0))
      (def (f (: s Symbol)) (if (= (= s #"add") false) 1 0))
      (def
        (main)
        (+ (* 10 (t (Symbol.of (String.concat "ad" "d")))) (f (Symbol.of (String.concat "ad" "d")))))
      (export main)))
  (output (: 10 Int64)))

; ============================================================================================
; Crossing back to text — Symbol.to-string recovers the interned content
; ============================================================================================
(case
  "a symbol converts back to its content string"
  (doc
    "`(Symbol.to-string (Symbol.of \"map-insert\"))` = \"map-insert\": a Symbol carries its
           content and hands it back as a String. This is the only way to observe a Symbol's content —
           together with `=` it is the whole observable surface, which is why an allocation-order id has
           nothing to attach to. The compiler uses it to render a name back for a diagnostic.")
  (input (Symbol.to-string (Symbol.of "map-insert")))
  (output (: "map-insert" String)))

(case
  "symbol identity follows String normalization"
  (doc
    "Because a Symbol wraps a String, symbol identity inherits String's normalized-contents
           equality (collections-and-text.md #String Equality Follows Normalized Contents): the composed
           \"café\" (…U+00E9) and the decomposed \"café\" (…e + U+0301) are the same text, so the
           Symbols interned from them are equal. This is the reason a Symbol is String-backed rather
           than Bytes-backed — two spellings of one source name intern to one symbol. Companion of the
           13-strings normalization cases, lifted through the Symbol tag.")
  (input (= (Symbol.of "café") (Symbol.of "café")))
  (output (: true Bool)))

; ============================================================================================
; The empty symbol is an ordinary Symbol value (the degenerate boundary)
; ============================================================================================
(case
  "the empty symbol equals itself"
  (doc
    "`(Symbol.of \"\")` interns the empty string to a Symbol — a first-class value equal only to
           another empty symbol. Pins that interning handles the zero-length name (an anonymous or
           generated name), the Symbol companion of the empty-string and empty-byte-sequence clusters.")
  (input (= (Symbol.of "") (Symbol.of "")))
  (output (: true Bool)))

(case
  "the empty symbol converts to the empty string"
  (doc
    "`(Symbol.to-string (Symbol.of \"\"))` = \"\": the empty symbol's content is the empty
           string. Pins that the round-trip through Symbol.to-string handles the zero-length content,
           not underflowing or reading a phantom scalar.")
  (input (Symbol.to-string (Symbol.of "")))
  (output (: "" String)))

; ============================================================================================
; The payoff — a runtime Symbol compared by `=` (the symbol-table hot path)
; ============================================================================================
; Symbol equality is realized for a RUNTIME Symbol — one from a function parameter, a call, an `if` —
; not only for two compile-time constants, because that is the whole use: a compiler compares a Symbol
; carried at run time (an identifier read from the AST) against interned constants to dispatch. These
; pin that `=` on a runtime Symbol operand is a genuine constant-time content test.
(case
  "a runtime symbol compared to an interned constant matches by content"
  (doc
    "`resolve` takes a Symbol parameter and compares it to the interned constant
           `(Symbol.of \"map-insert\")`; called with an equal symbol it returns 1. This is the
           symbol-table hot path — a name carried at run time compared against a known symbol by a
           handle compare rather than a byte scan. The equality is over a RUNTIME Symbol operand, not
           two constants.")
  (input
    (do
      (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
      (def (main) (resolve (Symbol.of "map-insert")))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime symbol that differs from the interned constant does not match"
  (doc
    "The companion with an unequal runtime operand: `resolve` called with `(Symbol.of \"other\")`
           compares it to `(Symbol.of \"map-insert\")` and returns 0. Confirms the runtime Symbol
           comparison is a genuine content test (1 for the matching name, 0 for a different one), not a
           blanket answer.")
  (input
    (do
      (def (resolve s) (if (= s (Symbol.of "map-insert")) 1 0))
      (def (main) (resolve (Symbol.of "other")))
      (export main)))
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
(case
  "a runtime symbol matches a symbol-literal pattern arm by content"
  (doc
    "`classify` matches a Symbol parameter against symbol-literal arms `(#\"add\" 1) (#\"sub\" 2)`;
           called with `#\"add\"` (built via `Symbol.of` over a runtime rope so it is not a constant fold)
           it takes the first arm → 1. The symbol-literal pattern is the `match` sibling of the
           `if (= s #\"add\")` dispatch: it classifies to a `Str` probe typed `Symbol` and emits a content
           value-eq test. Pins that a symbol scrutinee accepts a symbol-literal pattern.")
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "ad" "d"))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime symbol not among the literal arms falls through to the wildcard"
  (doc
    "The companion miss: `classify` called with a runtime `#\"xyz\"` matches neither `#\"add\"` nor
           `#\"sub\"` and falls through to the `_` arm → 0. Confirms the symbol-literal match is a genuine
           per-arm content dispatch (1 for a listed name, 0 for an unlisted one), not a blanket answer.")
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "x" "yz"))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a symbol-literal pattern nested in a variant payload matches by content"
  (doc
    "The NESTED face: a sum whose payload is a Symbol (`(type W (Mk Symbol))`) matched with a
           symbol-literal payload sub-pattern `(Mk #\"add\")`. The pattern imposes the `Mk` discriminant
           AND a content lit-test on the payload — `f` called with `(Mk #\"add\")` (payload built at run
           time) fires the first arm → 1. Pins that the symbol-literal probe classifies as a nested payload
           sub-pattern (the `SumPayload`-position twin of the top-level cases), typed `Symbol`.")
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of (String.concat "ad" "d")))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a nested symbol-literal payload falls through on a non-matching symbol"
  (doc
    "The nested miss: `f` called with `(Mk #\"sub\")` does not match the `(Mk #\"add\")` arm and
           takes the `(Mk _)` fall-through → 0. Confirms the nested symbol-literal test is a genuine
           content compare, the companion of the nested-hit case.")
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of (String.concat "su" "b")))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a Symbol-payload literal probe inside a RECURSIVE fn matches (recursive-specialization control)"
  (doc
    "A nested symbol-literal payload probe `(Mk #\"go\")` inside a self-recursive `walk` — the
           recursive-fn dimension of the nested-payload cases above. `walk` counts `n` down and, at the
           base, probes its `W` argument's Symbol payload against the literal `#\"go\"`; `(walk 2 (Mk
           #\"go\"))` recurses twice then matches → 40. Pins that wasm's recursive-fn specialization of a
           literal-payload probe is SOUND for an interned-heap-handle payload (Symbol), a positive control
           isolating a known BigInt-payload materialization defect in the SAME recursive context (breaker
           FINDING #22: a nonzero BigInt literal probe in a recursive fn miscompiles — HELD pin — while
           this Symbol twin computes correctly, proving the breakage is BigInt-materialization-specific,
           not the recursive-specialization machinery). Rust declines the literal-payload probe honestly
           (todo, 'a non-scalar literal-payload probe is not rendered by the Rust backend'), so this is
           also a rust-coverage marker for when that renders.")
  (input
    (do
      (type W (Mk Symbol))
      (def
        (walk (: n Int64) (: w W))
        (if (< n 1) (match w ((Mk #"go") 40) (_ -1)) (walk (- n 1) w)))
      (def (main) (walk 2 (Mk #"go")))
      (export main)))
  (output (: 40 Int64)))

; The landed symbol-literal cases pin the first-arm match, the wildcard miss, and a nested-payload match.
; These pin the neighbors: hitting the SECOND arm (arms are tried by CONTENT, so a symbol-literal match is
; order-independent across disjoint literals), the equivalence to the `if (= s lit)` CHAIN the doc names as
; the sibling dispatch, and a symbol-literal pattern over a MULTI-BYTE symbol (content-match, not a byte or
; ASCII assumption). All build the scrutinee via `Symbol.of` over a runtime rope so it is not a constant fold.
(case
  "a symbol-literal match reaches the second arm by content"
  (doc
    "`classify` matches a Symbol against `(#\"add\" 1) (#\"sub\" 2) (_ 0)`; called with a runtime
           `#\"sub\"` it takes the SECOND arm → 2. Pins that symbol-literal arms are tried by content across
           the whole arm list (order-independent for disjoint literals), not just the first — the miss-past-
           the-first-arm companion of the landed first-arm-match case.")
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "su" "b"))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a PER-CALL-selected runtime symbol dispatches through the = chain"
  (doc
    "Every runtime-Symbol pin above fixes the symbol content per program (a rope of constant text);
           here the CONTENT is selected by the boundary parameter — `(Symbol.of (if (= n 0) \"add\"
           \"sub\"))` — so one compiled `=`-dispatch answers differently per call (1 at n=0, 2 at n=1).
           Pins the interning + content-eq of a genuinely per-call symbol. (The symbol-literal MATCH over
           the same value still declines — the match desugar's Str-probe path doesn't reach a per-call
           symbol yet; the `=` chain is the working form, so the match-agrees-with-chain pin below can
           extend to per-call symbols when that lands.)")
  (input
    (do
      (def (main (: n Int64)) (if (= (Symbol.of (if (= n 0) "add" "sub")) #"add") 1 2))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "a symbol-literal match agrees with the equivalent if-(= s lit) chain"
  (doc
    "The symbol-literal `match` is the sibling of an `if (= s lit)` dispatch chain (the landed doc's
           framing): over the same runtime `#\"sub\"`, `(match s (#\"add\" 1) (#\"sub\" 2) (_ 0))` and
           `(if (= s #\"add\") 1 (if (= s #\"sub\") 2 0))` both give 2, so their difference is 0. Pins that
           the symbol-literal match desugars to the same content-eq dispatch as the explicit chain — a
           regression that changed the arm-selection order or the eq test would make them disagree.")
  (input
    (do
      (def (via-match (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (via-chain (: s Symbol)) (if (= s #"add") 1 (if (= s #"sub") 2 0)))
      (def (main) (let ((s (Symbol.of (String.concat "su" "b")))) (- (via-match s) (via-chain s))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a symbol-literal match agrees with its if-(= s lit) chain on the DEFAULT fall-through arm"
  (doc
    "The match≡chain case above tests a HIT arm (`#\"sub\"`); this pins the DEFAULT face. A symbol
           matching NO literal arm (`#\"xyz\"`, built at runtime) must take the wildcard `(_ 9)` in the
           match AND the trailing `else 9` in the chain — the arm a dropped-wildcard or a diverged
           desugaring would get wrong (the hit case cannot witness the fall-through). `(match s (#\"add\"
           1) (#\"sub\" 2) (_ 9))` and `(if (= s #\"add\") 1 (if (= s #\"sub\") 2 9))` both give 9, so
           their difference is 0. Completes the symbol match≡=-chain equivalence (hit + default), the
           symbol twin of the String fall-through pin.")
  (input
    (do
      (def (via-match (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 9)))
      (def (via-chain (: s Symbol)) (if (= s #"add") 1 (if (= s #"sub") 2 9)))
      (def (main) (let ((s (Symbol.of (String.concat "x" "yz")))) (- (via-match s) (via-chain s))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a symbol-literal pattern over a multi-byte symbol matches by content"
  (doc
    "`(match s (#\"café\" 1) (_ 0))` with a runtime `#\"café\"` (é = 2 UTF-8 bytes) takes the literal
           arm → 1. Pins that the symbol-literal content test compares the full UTF-8 byte content, not an
           ASCII or byte-length assumption — a multi-byte symbol matches its multi-byte literal exactly, the
           symbol-pattern companion of the multi-byte `Symbol.of` equality.")
  (input
    (do
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
; A Symbol stored INTO the value heap — `Set.of`/`Set.len` materializing a symbol set, a Symbol map key —
; now RUNS on all three backends (a Symbol IS a String byte-leaf on the value heap, so it boxes/reads-back
; and hashes+compares by content wherever a String does — collections-and-text.md #A Symbol Is An Interned
; Name). So a symbol set DEDUPLICATES by content and a Symbol-keyed map looks up + overwrites by content,
; exactly as the scalar/String collection cases do. The cases just below pin the `Set.contains` membership
; dispatch; the two after them pin the heap-materialization slice (Set.len dedup + Symbol map key).
(case
  "a runtime symbol is found among a set of known symbols"
  (doc
    "`dispatch` takes a Symbol parameter and asks whether it is one of the known node-kind names
           `{map-insert, map-lookup}` via `Set.contains`; called with `map-lookup` it returns true. This
           is the symbol-table dispatch the form exists for — a name carried at run time tested for
           membership in a fixed set of interned constants by content, a handle compare rather than a
           byte scan. The membership test is over a RUNTIME Symbol operand (a parameter), not a
           constant, the set-lifted companion of \"a runtime symbol compared to an interned constant
           matches by content\".")
  (input
    (do
      (def (dispatch s) (Set.contains #set((Symbol.of "map-insert") (Symbol.of "map-lookup")) s))
      (def (main) (dispatch (Symbol.of "map-lookup")))
      (export main)))
  (output (: true Bool)))

(case
  "a runtime symbol not among the known symbols is rejected"
  (doc
    "The companion with an unknown name: `dispatch` called with `other` — not one of the known
           node-kind names — returns false. Confirms `Set.contains` on a runtime Symbol is a genuine
           content test (true for a member, false for a non-member), not a blanket answer, so an
           unrecognized identifier is distinguished from a known one.")
  (input
    (do
      (def (dispatch s) (Set.contains #set((Symbol.of "map-insert") (Symbol.of "map-lookup")) s))
      (def (main) (dispatch (Symbol.of "other")))
      (export main)))
  (output (: false Bool)))

(case
  "membership of a runtime symbol is by content, not derivation"
  (doc
    "`dispatch` is queried with a Symbol interned from the COMPUTED string
           `(String.concat \"map\" \"-insert\")`; it is found in the known set that holds
           `(Symbol.of \"map-insert\")`, so the result is true. Pins that set membership follows the
           content-derived identity (memory-and-resource-model.md #Sharing Is Not Observable) — the
           membership analogue of \"a symbol's identity is its content, not how the content was
           derived\" — so a name assembled at run time still dispatches to the same table entry.")
  (input
    (do
      (def (dispatch s) (Set.contains #set((Symbol.of "map-insert") (Symbol.of "map-lookup")) s))
      (def (main) (dispatch (Symbol.of (String.concat "map" "-insert"))))
      (export main)))
  (output (: true Bool)))

(case
  "a set of symbols deduplicates by content — Set.len materializes the symbol set"
  (doc
    "The heap-MATERIALIZATION face (beyond membership): `Set.of` over a list of Symbols builds a
           symbol set on the value heap, and `Set.len` reads its cardinality — DEDUPED by content, since a
           Symbol is a String byte-leaf that hashes+compares by content on the CHAMP path. `{map-insert,
           map-lookup, map-insert}` names `map-insert` twice; the set holds it once → len 2. A runtime
           Symbol element `x` (a parameter, so it does not fold) matching an interned `map-insert` collapses
           into it (len stays 2), while a fresh name would grow it to 3. Pins that a symbol set materializes
           + dedups by content — the symbol-table-build the dispatch cases query.")
  (input
    (do
      (def (mk (: x Symbol)) (Set.len #set((Symbol.of "map-insert") (Symbol.of "map-lookup") x)))
      (def
        (main (: which Int64))
        (if (= which 0) (mk (Symbol.of "map-insert")) (mk (Symbol.of "map-remove"))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a Symbol map key looks up by content and an overwrite does not grow the map"
  (doc
    "The Symbol-MAP-KEY face: a `Map` keyed by Symbol — the symbol table a self-hosting compiler
           keys node metadata on. Insert `map-insert → 42`, then insert the SAME key (by content) `→ 99`:
           the map still has ONE entry (`Map.len` 1 — an overwrite, not a grow), and `Map.lookup` by a
           runtime Symbol built from a computed string finds 99 (the latest value), by content. Pins that a
           Symbol hashes+matches as a map key exactly like a String key — insert/overwrite/lookup all by
           content on the CHAMP key path.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m
              (Map.insert
                (Map.insert Map.empty (Symbol.of "map-insert") 42)
                (Symbol.of "map-insert")
                99)))
          (+
            (* 10 (Map.len m))
            (match
              (Map.lookup m (Symbol.of (String.concat "map" "-insert")))
              ((Some v) v)
              ((None) -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 109 Int64)))

(case
  "a Symbol nested in a TUPLE map key keys by content through the compound descent"
  (doc
    "The compound-key face of the Symbol key (the bare Symbol-key case above): the symbol sits in a
           tuple beside a RUNTIME int — `(tuple (Symbol.of \"k\") n)` — and the lookup key rebuilds both
           components; at `n = 5` the lookup `(tuple (Symbol.of \"k\") 5)` HITS (42), at `n = 9` it MISSES
           (-1). The CHAMP hash/eq must descend the tuple into the Symbol's byte-leaf content (the same
           per-leaf discipline the float/Rational/BigInt tuple-key pins fix for their leaf kinds — this
           completes the heap-leaf-kind matrix with the Symbol leaf).")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m #map((= #tuple((Symbol.of "k") n) 42))))
          (match (Map.lookup m #tuple((Symbol.of "k") 5)) ((Some v) v) ((None u) -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64)))

(case
  "a Symbol stored as a map VALUE round-trips through lookup to its content string"
  (doc
    "The VALUE-slot face: the pins above key BY symbols; a compiler symbol table equally maps
           name→Symbol (an id→name interner, a rename map), so the Symbol must round-trip through the
           map's VALUE slot — lookup → Some sy → Symbol.to-string → byte-len, with a miss face. A
           value-slot rep that stored the symbol as anything but its canonical leaf (or lost the tag
           on the way out) breaks the to-string. k=1 → \"a\" → 1; k=2 → \"bb\" → 2; k=9 miss → 0.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 #"a") (= 2 #"bb")))
          (match (Map.lookup m k) ((Some sy) (String.byte-len (Symbol.to-string sy))) ((None _u) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a tuple with a Symbol leaf as a SET element dedups and membership-checks by content"
  (doc
    "The SET-element companion of the tuple-map-key case above: the CHAMP hash/eq descends a
           compound SET element into its Symbol byte-leaf. A literal #\"a\" and a runtime-interned
           (Symbol.of \"a\") inside otherwise-equal tuples must DEDUP (content identity, not
           derivation) → len 2; membership rebuilds the tuple with a runtime n — hit at n=1,
           miss at n=9. len·10 + contains: 21 / 20.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def st #set(#tuple(#"a" 1) #tuple((Symbol.of "a") 1) #tuple(#"b" 1)))
          (+ (* (Set.len st) 10) (if (Set.contains st #tuple(#"a" n)) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 9 Int64))
  (output (: 20 Int64)))

; The case above interns a Symbol from a string the compiler can still FOLD (`(String.concat "map"
; "-insert")` = the constant `"map-insert"`). Interning a GENUINELY-RUNTIME string — one arriving at
; the call boundary, unfoldable — also works: a Symbol IS a String byte-leaf at run time (the value
; heap is tagless; a Symbol has no separate intern table, it compares via its physical bytes like a
; String), so `Symbol.of` on a runtime string CANONICALIZES its byte-rope to a flat leaf, and two
; symbols of equal content compare equal because both are canonical. That IS interning under a
; by-content representation — no runtime `str-intern` op needed. These pin the runtime-string→Symbol
; path (the intern analogue of the runtime String.slice byte-walk).
(case
  "Set.to-list over symbols enumerates in content-byte order regardless of insertion order"
  (doc
    "The ENUMERATION face of the symbol order (the </<= pins above compare pairs; this pins the
           to-list SORT): symbols inserted out of order — #\"c\", a runtime-interned mode symbol, #\"aa\",
           and a DUPLICATE #\"c\" — dedup to 3 and enumerate content-byte-ordered, digit-encoded by
           byte-len per position. Regression pin for the wasm shape_of Symbol descriptor arm
           (096c1652a): shape_of ADMITTED Symbol as orderable but built no descriptor, so wasm's
           to-list sort declined while the comparison pins passed — a check/emit divergence. mode 1:
           {aa,b,c} → 211; mode 2: {aa,c,zzzz} → 214 (the runtime symbol sorts LAST — order is by
           content, not insertion or length). +3000 for len 3.")
  (input
    (do
      (def
        (fold-lens (: xs (List Symbol)) (: acc Int64))
        (match
          xs
          (#list() acc)
          (#list(h (.. t)) (fold-lens t (+ (* acc 10) (String.byte-len (Symbol.to-string h)))))))
      (def
        (main (: mode Int64))
        (do
          (def s (if (= mode 1) "b" "zzzz"))
          (def
            st
            (Set.insert (Set.insert (Set.insert (Set.insert #set() #"c") (Symbol.of s)) #"aa") #"c"))
          (+ (* (Set.len st) 1000) (fold-lens (Set.to-list st) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3211 Int64))
  (call main (: 2 Int64))
  (output (: 3214 Int64))
  (live-objects 0))

(case
  "a runtime string interns to a symbol matched by content"
  (doc
    "`Symbol.of` on a GENUINELY-RUNTIME string — built by the `rep` concat loop `(rep \"\" 3)` =
           \"xxx\", a byte-rope the compiler cannot fold — interns it, and the resulting Symbol compares
           EQUAL to the constant `#\"xxx\"` of the same content: a runtime Symbol is a canonical byte leaf
           compared by its bytes, not a compile-time intern id. Pins that a name assembled from genuinely-
           runtime data still dispatches to the same identity (the intern analogue of runtime String.slice).")
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (= (Symbol.of (rep "" 3)) #"xxx"))
      (export main)))
  (output (: true Bool)))

(case
  "a runtime symbol round-trips back to its string"
  (doc
    "`Symbol.to-string (Symbol.of s)` on a runtime string `s` recovers the SAME content String —
           both directions are a byte-leaf retag (a Symbol and its String share the tagless rep). `s` is
           the runtime rope `(rep \"xx\" 3)` = \"xxxxx\"; observed by the recovered string's byte length
           (5), exercising both runtime retags in one chain. The inverse of the intern above.")
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (String.byte-len (Symbol.to-string (Symbol.of (rep "xx" 3)))))
      (export main)))
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
(case
  "a symbol compared to a string is a type error"
  (doc
    "`(= (Symbol.of \"x\") \"x\")` compares a Symbol to the untagged String of the same content —
           across the nominal boundary — so the compiler rejects it (CDZ0202, type-system.md #Nominal
           Types Are Not Comparable Across Their Boundary). A Symbol never silently compares equal to
           the String it was interned from; to compare content you write
           `(= (Symbol.to-string s) \"x\")`.")
  (input (= (Symbol.of "x") "x"))
  (error CDZ0202))

(case
  "a string compared to a symbol is a type error"
  (doc
    "The order-flipped companion: `(= \"x\" (Symbol.of \"x\"))` is the same nominal-boundary
           violation regardless of which operand carries the Symbol tag — CDZ0202. Pins that the tag is
           checked on either side of the comparison, mirroring the nominal-record boundary cases.")
  (input (= "x" (Symbol.of "x")))
  (error CDZ0202))

(case
  "a symbol compared to a number is a kind-boundary type error"
  (doc
    "`(< (Symbol.of \"x\") 1)` compares a Symbol to a number — two DIFFERENT kinds with no shared
           order (a Symbol is an interned text value, not a scalar). Distinct from the Symbol-vs-String
           NOMINAL boundary above: this is a KIND boundary (CDZ0201, `type-system.md` — an operation is not
           defined across a kind boundary), the same category as `String`-vs-number and `Bool`-vs-number.
           Pins that a Symbol-vs-number operand pair is NAMED as a kind-boundary error, not left to the
           opaque generic scheme-unify 'type mismatch: Symbol and Int64 must be the same type here' it fell
           through to before (Symbol was the scalar-adjacent kind missing from the cross-kind classifier).")
  (input (do (def (main) (< (Symbol.of "x") 1)) (export main)))
  (error CDZ0201))

; The nominal boundary holds in a `match` PATTERN too, not just `=`. A String and a Symbol literal
; pattern share the `Core::ConstStr` rep, but the two types are distinct across the nominal boundary, so
; a text-literal pattern must match a scrutinee of its OWN kind: a `"add"` (String) pattern over a Symbol
; scrutinee — or a `#"add"` (Symbol) pattern over a String scrutinee — is a pattern/scrutinee type
; mismatch (CDZ0201, the general shape/type-mismatch code the char/bool-over-int pattern cases carry),
; the pattern-path sibling of the `=` CDZ0202 above. Pins that the pattern path is NOT more permissive
; than `=` on this boundary — the type comes from the PATTERN's kind, not the scrutinee. (The check is
; structural: it holds even for a CONSTANT scrutinee, before any fold.)
(case
  "a string-literal pattern over a symbol scrutinee is a type error"
  (doc
    "`(match (Symbol.of \"x\") (\"add\" 1) (_ 0))` matches a Symbol scrutinee against a STRING literal
           pattern `\"add\"` — across the nominal boundary — so the compiler rejects it (CDZ0201, the
           pattern/scrutinee type-mismatch code). The `match` sibling of `(= (Symbol.of \"x\") \"add\")` →
           CDZ0202: a symbol-literal `#\"add\"` matches a Symbol, a string-literal `\"add\"` matches a
           String, and the two do not cross. Pins the pattern path respects the Symbol↔String boundary.")
  (input (match (Symbol.of "x") ("add" 1) (_ 0)))
  (error CDZ0201))

(case
  "a symbol-literal pattern over a string scrutinee is a type error"
  (doc
    "The order-flipped companion: `(match \"x\" (#\"add\" 1) (_ 0))` matches a String scrutinee against
           a SYMBOL literal pattern `#\"add\"` — the same nominal-boundary violation with the tag on the
           pattern side — CDZ0201. Pins that the boundary is checked whichever kind the pattern carries, the
           pattern-path mirror of the two `=` CDZ0202 cases above.")
  (input (match "x" (#"add" 1) (_ 0)))
  (error CDZ0201))

; The scalar boundary above pins the top-level text-literal pattern. The type comes from the PATTERN's
; kind, so the same CDZ0201 must fire wherever a text-literal pattern sits — a NESTED sum-payload sub-
; pattern, a SIBLING arm in an otherwise same-kind match, and a TUPLE-element sub-pattern — not only the
; top-level scalar position. (The scalar fix keyed `pat_ty` on the pattern's origin; these pin that the
; nested-payload and tuple-element positions do the same, and that a per-arm mix does not let one crossing
; arm slip through because a sibling arm is same-kind.) A same-kind control alongside each proves the
; boundary check does not over-reject the legitimate case.
(case
  "a string-literal pattern in a Symbol sum payload is a type error"
  (doc
    "The NESTED-payload face of the boundary: `(type W (Mk Symbol))` matched with a STRING-literal
           payload sub-pattern `(Mk \"add\")` over a Symbol payload crosses the nominal boundary → CDZ0201,
           the sum-payload twin of the top-level `\"add\"`-over-Symbol case. Pins the payload sub-pattern
           keys its expected type on the PATTERN kind (String) too, not the Symbol payload type — the same
           discipline as `pattern_constraints` for the nested position.")
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk "add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of "add"))))
      (export main)))
  (error CDZ0201))

(case
  "a symbol-literal pattern in a String sum payload is a type error"
  (doc
    "The order-flipped nested companion: `(type W (Mk String))` with a SYMBOL-literal payload sub-
           pattern `(Mk #\"add\")` over a String payload → CDZ0201. Pins the nested-payload boundary holds
           whichever kind the pattern carries, the payload twin of the flipped top-level case.")
  (input
    (do
      (type W (Mk String))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (String.concat "ad" "d"))))
      (export main)))
  (error CDZ0201))

(case
  "a same-kind symbol-literal payload sub-pattern still dispatches (nested control)"
  (doc
    "The nested control that must NOT over-reject: `(Mk #\"add\")` over a Symbol payload is same-kind,
           so it dispatches — `f (Mk #\"add\")` → 1. Pins the nested-payload boundary check rejects only the
           crossing case, not the legitimate same-kind one, alongside the two nested rejects above.")
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of "add"))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a crossing text-literal arm is rejected even beside a same-kind sibling arm"
  (doc
    "The per-arm face: a Symbol scrutinee with a same-kind first arm `#\"add\"` AND a crossing STRING
           sibling arm `\"sub\"` — `(match s (#\"add\" 1) (\"sub\" 2) (_ 0))` — still faults CDZ0201. Pins
           the expected type is keyed per-arm on each pattern's origin (`probe_pats[i]`), so a legitimate
           same-kind arm does not let a crossing sibling arm slip through — a whole-match check that keyed on
           the scrutinee, or only the first arm, would miss this.")
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) ("sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of "add")))
      (export main)))
  (error CDZ0201))

(case
  "the mirror per-arm mix — a crossing #sym sibling over a String scrutinee — is rejected"
  (doc
    "The flipped per-arm case: a String scrutinee with a same-kind `\"add\"` arm and a crossing SYMBOL
           sibling arm `#\"sub\"` → CDZ0201. Pins the per-arm boundary check fires whichever kind the
           crossing arm carries, the mirror of the case above.")
  (input
    (do
      (def (classify (: s String)) (match s ("add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (String.concat "ad" "d")))
      (export main)))
  (error CDZ0201))

(case
  "a runtime Symbol value dispatches on symbol literals"
  (doc
    "The positive same-kind case: a RUNTIME `Symbol` scrutinee (built via `Symbol.of` from a
           parameter-selected string, so it does not fold) matched against `#\"add\"` / `#\"sub\"` symbol
           literals with a catch-all → each arm is a content `value-eq` (a Symbol shares the String
           byte-leaf rep, so it dispatches exactly as a runtime string keyword match). `#\"add\"`→1,
           `#\"sub\"`→2, any other symbol→the `_` tail. Pins that symbol dispatch is not only a
           constant fold — a runtime symbol is dispatched by content, the symbol twin of runtime string
           dispatch (10-bytes has the Bytes twin).")
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main (: n Int64)) (classify (Symbol.of (if (= n 0) "add" (if (= n 1) "sub" "zz")))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64)))

(case
  "a string-literal sub-pattern in a Symbol tuple element is a type error"
  (doc
    "The TUPLE-element face: `(: p (Tuple Symbol Int64))` matched with `(tuple \"add\" n)` — a STRING
           literal over the Symbol element — crosses the boundary → CDZ0201. Pins the tuple-element position
           keys its expected type on the PATTERN kind too, the positional twin of the sum-payload nested
           cases.")
  (input
    (do
      (def (f (: p (Tuple Symbol Int64))) (match p (#tuple("add" n) n) (#tuple(_ n) 0)))
      (def (main) (f #tuple((Symbol.of "add") 5)))
      (export main)))
  (error CDZ0201))

(case
  "a same-kind symbol-literal tuple sub-pattern still dispatches (tuple control)"
  (doc
    "The tuple control: `(tuple #\"add\" n)` over a Symbol element is same-kind, so it dispatches —
           `f (tuple #\"add\" 5)` binds n=5 → 5. Pins the tuple-element boundary check rejects only the
           crossing case, the same-kind companion of the tuple reject above.")
  (input
    (do
      (def (f (: p (Tuple Symbol Int64))) (match p (#tuple(#"add" n) n) (#tuple(_ n) 0)))
      (def (main) (f #tuple((Symbol.of "add") 5)))
      (export main)))
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
(case
  "the reader literal reads to Symbol.of"
  (doc
    "`#\"map-insert\"` reads to `(Symbol.of \"map-insert\")`, so the two denote one Symbol value
           and `(= #\"map-insert\" (Symbol.of \"map-insert\"))` is true. Pins the reader sugar against
           the canonical form it expands to.")
  (input (= #"map-insert" (Symbol.of "map-insert")))
  (output (: true Bool)))

(case
  "a reader literal carries a qualified name with a dot"
  (doc
    "`#\"List.at\"` interns the string \"List.at\" — a qualified name whose dot the bare-token
           form (`#name`, the ML surface's identifier-only convenience) could not carry unambiguously
           (it would read as member access) — so `(= #\"List.at\" (Symbol.of \"List.at\"))` is true.
           Pins that the string-form literal interns arbitrary content, the reason the canonical form
           is `#\"…\"` and the bare `#name` sugar is confined to plain identifiers.")
  (input (= #"List.at" (Symbol.of "List.at")))
  (output (: true Bool)))

(case
  "the empty reader literal is the empty symbol"
  (doc
    "`#\"\"` reads to `(Symbol.of \"\")`, the empty symbol — `(= #\"\" (Symbol.of \"\"))` is true.
           Pins that the reader sugar handles the zero-length case, the degenerate boundary of the
           literal form.")
  (input (= #"" (Symbol.of "")))
  (output (: true Bool)))

(case
  "a MULTIBYTE symbol map key is found by its runtime-CONCAT-interned twin"
  (doc
    "A MULTIBYTE symbol as a map key, probed by a runtime-CONCAT-interned twin: `#\"日本\"` (a reader
           literal) must be the SAME interned symbol as `(Symbol.of (String.concat \"日\" \"本\"))` — the
           runtime intern path over multibyte content bytes. mode=1 finds 42; mode=2 concats \"日x\" and
           misses (-1). An intern table that keyed on a truncated or unit-miscounted byte view splits the
           literal from its runtime twin.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def m (Map.insert Map.empty #"日本" 42))
          (def probe (Symbol.of (String.concat "日" (if (= mode 1) "本" "x"))))
          (match (Map.lookup m probe) ((Some v) v) ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64)))

(case
  "multibyte symbols order by UNSIGNED content bytes - a CJK scalar sorts after z"
  (doc
    "Symbol ordering over MULTIBYTE content: a CJK scalar's UTF-8 bytes (0xE6…) compare UNSIGNED,
           so `#\"日\"` sorts after `z` (0x7A) — a signed-i8 byte compare would flip it (0xE6 as -26 < z).
           mode=1: z < 日 -> 10; mode=2: 日 = 日 (same interned symbol via the concat path) -> 1.")
  (input
    (do
      (def (mk (: s String)) (Symbol.of (String.concat s "")))
      (def
        (main (: mode Int64))
        (do
          (def a (mk (if (= mode 1) "z" "日")))
          (def b (mk "日"))
          (+ (* (if (< a b) 1 0) 10) (if (= a b) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64)))

(case
  "Symbol.of over a slice VIEW interns exactly the window, flat or rope-backed"
  (doc
    "The INTERN consumer of the slice-view family: `Symbol.of` must read only the view's
           window, not the backing string. Slicing \"xkeyz\" [1,4) — flat (mode 1) or through the rope
           `(String.concat \"xk\" \"eyz\")` whose seam falls INSIDE the window (mode 2) — interns the
           same symbol as the literal `#\"key\"` (1). An intern that hashed the backing bytes (or a
           seam-truncated read) misses. mode 3 slices [0,3) = \"xke\" — a different window over the
           SAME base — and must NOT equal `#\"key\"` (0).")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def
            v
            (if
              (= mode 2)
              (Option.expect (String.slice (String.concat "xk" "eyz") 1 4) "in")
              (if
                (= mode 3)
                (Option.expect (String.slice "xkeyz" 0 3) "in")
                (Option.expect (String.slice "xkeyz" 1 4) "in"))))
          (if (= (Symbol.of v) #"key") 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64)))

(case
  "a Symbol interned from a rope-slice view keys maps and removes set elements"
  (doc
    "Composes the view intern (Symbol.of over a rope-backed slice, seam inside the window) with
           the CHAMP surfaces: mode 1 looks up {#\"key\" -> 42} with the view-interned symbol (42);
           mode 2 removes it from {#\"key\", #\"other\"} (len 1). Symbols are the FULLY-INTERNED key
           kind — if the view intern produced a content-twin-but-distinct id, symbol eq (id compare)
           would miss BOTH surfaces at once, unlike String/Bytes keys where only the hash path is at
           risk. mode 3 is the wrong-window control: `Symbol.of` of [0,3) = \"xke\" misses (-1).")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def
            sv
            (Symbol.of
              (Option.expect
                (String.slice (String.concat "xk" (if (> mode 1000) "zzz" "eyz")) 1 4)
                "in")))
          (if
            (= mode 1)
            (match (Map.lookup (Map.insert Map.empty #"key" 42) sv) ((Some x) x) ((None _u) -1))
            (if
              (= mode 2)
              (Set.len (Set.remove #set(#"key" #"other") sv))
              (match
                (Map.lookup
                  (Map.insert Map.empty #"key" 42)
                  (Symbol.of (Option.expect (String.slice "xkeyz" 0 3) "in")))
                ((Some x) x)
                ((None _u) -1))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a view-interned Symbol participates in Set.to-list content order"
  (doc
    "Composes the view-intern face with the orderable-descriptor sort: the symbol interned from
           `slice(concat(...), 1, 3)` joins {#\"b\", #\"c\"} and `Set.to-list` must place it by CONTENT
           bytes — mode 1 interns \"aa\" which sorts FIRST (e0 = #\"aa\": 11); mode 0 interns \"az\",
           before \"b\" but not \"aa\" (1). A sort keyed on intern-table ids (allocation order) instead
           of content puts the LAST-interned view symbol last and flips mode 1.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def
            sv
            (Symbol.of
              (Option.expect (String.slice (String.concat "xa" (if (> mode 0) "az" "zz")) 1 3) "in")))
          (def xs (Set.to-list #set(#"b" sv #"c")))
          (def (at (: i Int64)) (Option.expect (List.at xs i) "in"))
          (+ (* 10 (if (= (at 0) #"aa") 1 0)) (if (= (at 2) #"c") 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; --- The re-intern identity loop over a runtime rope. ---
(case
  "re-interning a symbol's recovered string yields an equal symbol, and the string content-matches"
  (doc
    "Closes the re-intern LOOP on a runtime rope (the :548 round-trip observes byte-len only; the const idempotence pin :61 never crosses to-string): (= (Symbol.of (Symbol.to-string sym)) sym) — the full of→to-string→of chain lands on an EQUAL symbol. A to-string that copied with drift, or an intern keyed on rope chunk shape rather than content, breaks the loop while both existing pins stay green.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def s (String.concat "sym-" (if (= k 1) "a" "b")))
          (def sym (Symbol.of s))
          (def back (Symbol.to-string sym))
          (+
            (* 100 (if (= back s) 1 0))
            (+ (* 10 (if (= (Symbol.of back) sym) 1 0)) (String.byte-len back)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 115 Int64)))

; --- Symbol.to-string output vs a slice VIEW of matching content. ---
(case
  "Symbol.to-string output compares equal to a slice VIEW of matching content"
  (doc
    "The symbol→string crossing meets the view rep: `Symbol.to-string` (an interned-content
           read) against a borrowed slice view — equal in both spellings when the window matches
           (11 at mode 1, [1,4) = \"key\"), unequal at the shifted window (0 at mode 0, [0,3) =
           \"xke\"). The to-string result's rep (interned leaf) and the view's rep ([off,len]
           borrow) are the LAST unpaired string-rep combination — a to-string that returned a
           non-canonical rep (or an eq that trusted rep identity) breaks the match row.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def
            v
            (Option.expect (String.slice "xkeyz" (if (= mode 1) 1 0) (if (= mode 1) 4 3)) "in"))
          (+
            (* 10 (if (= (Symbol.to-string #"key") v) 1 0))
            (if (= v (Symbol.to-string (Symbol.of "key"))) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

; -- runtime Symbol content-dispatch match (Symbol.of over a runtime rope defeats the fold; migration from
; rcdzc a_symbol_literal_pattern_dispatches_by_content, 2026-08-27): a symbol-literal pattern dispatches by
; content across the arms, and a nested symbol-literal payload sub-pattern matches by content.
(case
  "a runtime symbol-literal pattern dispatches to the first arm by content"
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "ad" "d"))))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a runtime symbol-literal pattern dispatches to the second arm by content"
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "su" "b"))))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a runtime symbol-literal pattern falls through to the wildcard on an unlisted symbol"
  (input
    (do
      (def (classify (: s Symbol)) (match s (#"add" 1) (#"sub" 2) (_ 0)))
      (def (main) (classify (Symbol.of (String.concat "x" "y"))))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a nested symbol-literal payload pattern matches by content"
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of (String.concat "ad" "d")))))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a nested symbol-literal payload pattern falls through on a non-match"
  (input
    (do
      (type W (Mk Symbol))
      (def (f (: w W)) (match w ((Mk #"add") 1) ((Mk _) 0)))
      (def (main) (f (Mk (Symbol.of (String.concat "su" "b")))))
      (export main)))
  (call main)
  (output (: 0 Int64)))

; -- runtime Symbol.of interning + Symbol.to-string round-trip (Symbol.of over a runtime rope defeats the
; fold; migration from rcdzc a_runtime_string_interns_to_a_symbol_by_content, 2026-08-27).
(case
  "a runtime Symbol.of of a rope equals a symbol literal by content"
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= (Symbol.of (rep "" 3)) #"xxx") 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a runtime Symbol.of of a different-length rope does not equal the literal"
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= (Symbol.of (rep "" 2)) #"xxx") 1 0))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a runtime Symbol.of then Symbol.to-string round-trips the bytes"
  (input
    (do
      (def (rep s n) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (String.byte-len (Symbol.to-string (Symbol.of (rep "xx" 3)))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

; -- the String/Symbol nominal boundary in match patterns (migration from rcdzc
; a_string_or_symbol_literal_pattern_respects_the_nominal_boundary, 2026-08-27; the same-kind dispatch
; controls are covered by the symbol/string content-dispatch cases): a cross-boundary literal pattern faults.
(case
  "a String literal pattern over a Symbol scrutinee is rejected at the nominal boundary"
  (input (do (def (f (: s Symbol)) (match s ("add" 1) (_ 0))) (export f)))
  (error CDZ0201))

(case
  "a Symbol literal pattern over a String scrutinee is rejected at the nominal boundary"
  (input (do (def (f (: s String)) (match s (#"add" 1) (_ 0))) (export f)))
  (error CDZ0201))

; -- a Symbol tuple element compares by content in a direct tuple = (migrated from rcdzc
; a_symbol_tuple_element_compares_by_content_on_the_value_heap; the map-key CHAMP-eq face is @459): a
; Symbol IS a String byte-leaf handle at run time, so a Symbol tuple element boxes/reads-back/compares by
; CONTENT exactly like a String element in a runtime tuple `=`.
(case
  "sqte1 a Symbol tuple element compares by content in a runtime tuple equality"
  (doc
    "For runtime n: `(tuple (Symbol.of \"a\") n)` equals itself (→1 flag), differs from a tuple with a
           different scalar sibling `(+ n 1)` (→0), and differs from one with a different Symbol `(Symbol.of
           \"b\")` (→0, content comparison). Weighted so the checksum is 1 iff all three hold; n=7.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (if (= #tuple((Symbol.of "a") n) #tuple((Symbol.of "a") n)) 1 0)
          (+
            (* 10 (if (= #tuple((Symbol.of "a") n) #tuple((Symbol.of "a") (+ n 1))) 1 0))
            (* 100 (if (= #tuple((Symbol.of "a") n) #tuple((Symbol.of "b") n)) 1 0)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64)))
