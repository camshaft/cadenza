; Sets — the third built-in collection beside List and Map, witnessing collections-and-text.md
; #Sets (set-collection decision, options/set-collection/). A Set is an UNORDERED collection of
; UNIQUE elements of one type: it contains each element at most once, two sets are equal exactly
; when they contain equal elements independent of insertion order, membership is a TOTAL predicate
; (never traps, and there is no positional access because a set is unordered), and iteration visits
; elements in a deterministic element-derived order agreeing with the canonical byte form. A Set is
; a PRIMITIVE collection, not a `Map<T, Unit>` and not a hand-deduped `List`: it rides the same
; deterministic-value-form machinery Map does (a fixed element-derived order, deterministic-value-
; form.md #Ordering Of Aggregate Members Is Fixed), so the junk unit values a `Map<T, Unit>` would
; carry never enter its equality or its byte form.
;
; The value form is `(Set.of (list …))` — the canonical written form, exactly as a byte sequence is
; `(Bytes.of (list …))` and a symbol is `(Symbol.of …)`. `Set` is an ordinary prelude record (like
; `Bytes`, `String`, `Symbol`), so `Set.of` is `(. Set of)`, `Set.contains` is `(. Set contains)`,
; and so on — member access into a prelude record, not new core syntax. A self-hosting compiler
; keys its free-variable sets, visited sets, and declared-capability sets on this form.
;
; `sets` is a fresh capability the seed does NOT realize (distinct from the realized
; `collections`, exactly as `symbols` is distinct from it): a later generation realizes the
; persistent-set runtime (the same ordered persistent structure the map runtime targets, with the
; value column dropped; options/realized-capability-set/). The seed does not realize `sets`, so it
; DECLINES these — they pin the contract the realization must meet.
; (The tag must be a fresh capability, not `collections`: `collections` is realized, so the seed
; would RUN these and reject the unbound `Set` prelude name with a coded diagnostic — a gate FAIL —
; rather than skip. This is why `symbols`/`units-of-measure`/`binary-matching` each use their own
; unrealized tag.) Several cases are written in an equality position because a set is not yet a
; producible top-level value (mirroring the map cases in 05-compound-types.sexp); the equality still
; forces the set to be built, exercising the uniqueness and order-independence invariants.
; ============================================================================================
(case
  "a set is constructed from a list of its elements"
  (doc
    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements: `(Set.of (list
           1 2 3))` is the set {1, 2, 3}, and its canonical written form is `(Set.of (list 1 2 3))`
           with the elements in their canonical (sorted) order. A set of Int64 has type `(Set Int64)`.")
  (input #set(1 2 3))
  (output (: #set(1 2 3) (Set Int64))))

(case
  "a set collapses a duplicate element"
  (doc
    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements (2nd sentence: a
           set contains each element at most once): `(Set.of (list 1 2 2 3))` names 2 twice, but the set
           holds it once — it equals `(Set.of (list 1 2 3))`. Pins that construction deduplicates rather
           than building a multiset. MUST be true.")
  (input (= #set(1 2 2 3) #set(1 2 3)))
  (output (: true Bool)))

(case
  "a set collapses a duplicate STRING element (a non-int scalar key path)"
  (doc
    "The dedup case above uses Int64 elements; a String element deduplicates the same way through
           the non-int scalar key path: `(Set.of (list \"a\" \"b\" \"a\"))` names \"a\" twice but the set
           holds it once — it equals `(Set.of (list \"a\" \"b\"))`. Pins that construction deduplicates by
           value for a String (scalar-key) element, not only for Int64. MUST be true.")
  (input (= #set("a" "b" "a") #set("a" "b")))
  (output (: true Bool)))

(case
  "set equality is independent of the order elements are written"
  (doc
    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements (3rd sentence:
           two sets are equal exactly when they contain equal elements, independent of insertion order):
           `(Set.of (list 3 1 2))` and `(Set.of (list 1 2 3))` contain the same elements, so they are
           EQUAL regardless of the written order — the set analogue of order-independent map equality.
           Pins that set `=` compares element SETS, not positional lists. MUST be true.")
  (input (= #set(3 1 2) #set(1 2 3)))
  (output (: true Bool)))

(case
  "membership of a present element is true"
  (doc
    "Witnesses collections-and-text.md #Set Membership Is Total: `(Set.contains (Set.of (list 1 2
           3)) 2)` tests whether 2 is in the set — it is, so the total predicate yields true (a Bool,
           never a trap and never an Option). The membership companion of a map lookup, but a set's
           membership is a plain Bool because there is no associated value to return.")
  (input (Set.contains #set(1 2 3) 2))
  (output (: true Bool)))

; The dedup/membership cases above use SCALAR (Int64) elements. A Set element may be a COMPOUND value —
; a tuple or a list — in which case dedup and membership compare the WHOLE compound by value: two tuples
; are the same element only if ALL components are equal, and tuple element order matters (⟨5,1⟩ ≠ ⟨1,5⟩).
; These pin the compound-element path (runtime operands, so nothing folds): a Set keyed by a tuple/list
; hashes and compares the full structure, exactly as a Map does for a compound key.
(case
  "a set of tuples deduplicates by the whole tuple and its membership is component-order-sensitive"
  (doc
    "`(Set.of (list (tuple a 1) (tuple b 2) (tuple a 1)))` over runtime a/b: the repeated `(tuple a 1)`
           collapses (dedup by the WHOLE tuple value), so with a=5,b=5 the set is {⟨5,1⟩, ⟨5,2⟩} — len 2,
           the two tuples distinct because their SECOND components differ. Membership is total and compares
           the whole tuple: `(Set.contains s (tuple a 1))` is true, but `(Set.contains s (tuple 1 a))` is
           FALSE — ⟨1,5⟩ is not ⟨5,1⟩, tuple component ORDER matters. Encodes 100·len + 10·has⟨a,1⟩ + has⟨1,a⟩
           = 210. Pins compound-element dedup + order-sensitive compound membership.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((s #set(#tuple(a 1) #tuple(b 2) #tuple(a 1))))
          (+
            (* 100 (Set.len s))
            (+ (* 10 (if (Set.contains s #tuple(a 1)) 1 0)) (if (Set.contains s #tuple(1 a)) 1 0)))))
      (export main)))
  (call main (: 5 Int64) (: 5 Int64))
  (output (: 210 Int64))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 210 Int64)))

(case
  "a set of lists deduplicates by the whole list, distinguishing element order"
  (doc
    "The list-element companion: `(Set.of (list (list a b) (list b a) (list a b)))`. With a=3,b=8 the
           lists `[3,8]` and `[8,3]` are DISTINCT elements (list order matters), and the repeated `[3,8]`
           collapses → len 2. With a=5,b=5 all three are `[5,5]`, one element → len 1. Pins that a Set over
           a list element dedups by the whole list value including element order, the list twin of the
           tuple case.")
  (input
    (do
      (def (main (: a Int64) (: b Int64)) (Set.len #set(#list(a b) #list(b a) #list(a b))))
      (export main)))
  (call main (: 3 Int64) (: 8 Int64))
  (output (: 2 Int64))
  (call main (: 5 Int64) (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a set of records deduplicates by the whole record and its membership is field-order-INDEPENDENT"
  (doc
    "The record-element companion of the tuple/list cases above, with the twist that distinguishes a
           record from a tuple: field order does NOT matter. `(Set.of (list (record (x 1) (y 2)) (record (x 3) (y k)) (record (x 1) (y 2))))`
           over a runtime `k`: the repeated `(record (x 1) (y 2))` collapses (dedup by the WHOLE record
           value), so with k=4 the set is {⟨x1,y2⟩, ⟨x3,y4⟩} — len 2. Membership compares the whole record:
           `(Set.contains s (record (x 1) (y 2)))` is true, AND `(Set.contains s (record (y 2) (x 1)))` is
           ALSO true — the record written with its fields in REVERSE order is the SAME element, because a
           record canonicalizes by sorted field name (unlike a tuple, whose component ORDER is part of its
           identity — the tuple case above is order-SENSITIVE). Encodes 100·len + 10·has⟨x1,y2⟩ + has⟨y2,x1⟩
           = 100·2 + 10·1 + 1 = 211. Pins the compound-element CHAMP path over records + the field-order-
           independence a tuple element cannot witness.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((s #set(#record((= x 1) (= y 2)) #record((= x 3) (= y k)) #record((= x 1) (= y 2)))))
          (+
            (* 100 (Set.len s))
            (+
              (* 10 (if (Set.contains s #record((= x 1) (= y 2))) 1 0))
              (if (Set.contains s #record((= y 2) (= x 1))) 1 0)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 211 Int64)))

(case
  "a set of MAPS deduplicates by full map content"
  (doc
    "The map-element member of the compound-element family (tuple/list/record above): set elements
           that are themselves MAPS, deduplicated by FULL map content — `{{1↦a}, {1↦10}, {2↦10}}` at
           a=10 collapses the first two (same single entry) → len 2; at a=99 all three differ (by value,
           by value, by key) → len 3. A CHAMP-of-CHAMPs: the outer set's hash/compare must walk each
           element map's entries (a hash over the map handle, or a compare stopping at the key set,
           conflates one of the pairs). Expected: 2 (a=10), 3 (a=99).")
  (input
    (do
      (def
        (main (: a Int64))
        (Set.len
          #set(#map((= 1 a)) #map((= 1 10)) #map((= 2 10)))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 2 Int64))
  (call main (: 99 Int64))
  (output (: 3 Int64)))

(case
  "a set of SETS deduplicates by inner-set value and finds a member built in a different order"
  (doc
    "The set-element member of the compound-element family (tuple/list/map above): set elements that
           are themselves SETS. Because a Set is ORDER-INDEPENDENT, the inner sets `{a,b}` and `{b,a}` are the
           SAME value, so `(Set.of (list {a,b} {b,a} {a}))` collapses the first two → `Set.len` 2; and
           `Set.contains` for `{b,a}` (built in the opposite order) is TRUE — the outer set finds the member
           by its canonical CONTENT, not by insertion order or handle. A Set-of-Sets is a CHAMP-of-CHAMPs
           where the OUTER hash/compare must reduce each element set to its canonical (sorted) form (a walk
           that hashed insertion order would keep `{a,b}` and `{b,a}` distinct → len 3, has 0). Encodes
           10·len + has = 10·2 + 1 = 21. MUST be 21.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((ss #set(#set(a b) #set(b a) #set(a))))
          (+ (* 10 (Set.len ss)) (if (Set.contains ss #set(b a)) 1 0))))
      (export main)))
  (call main (: 3 Int64) (: 8 Int64))
  (output (: 21 Int64)))

(case
  "a Set.of over RECURSIVE-sum elements dedups by spine content"
  (doc
    "The unbounded-depth member of the compound-element family: elements are runtime-built Peano
           spines — `{(mk a), (mk 3)}` at `a = 3` collapses (two separately-built equal 4-node spines share
           one canonical content) → len 1; at `a = 2` distinct depths → len 2. The set's hash/compare must
           walk the WHOLE recursive spine per element. Note the construction-path asymmetry: this Set.of
           batch build computes on every backend while the equivalent Set.insert chain onto Set.empty still
           declines (the two paths lower separately) — so this pins the WORKING path; the insert twin joins
           when its emit lands.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
      (def (main (: a Int64)) (Set.len #set((mk a) (mk 3))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64)))

(case
  "a Set.of over user-sum variants keeps same-payload different-variant elements distinct"
  (doc
    "The variant-tag member: `{(A n), (B n), (A 5)}` at `n = 5` — `(A n)` and `(A 5)` collapse (same
           tag, same payload) while `(B 5)` stays distinct (different TAG, same payload) → len 2; at
           `n = 7` all three differ → len 3. The element hash must read both the variant tag and the
           payload (a payload-only hash would collapse A/B; a tag-only hash would collapse the two As at
           n=7). The set companion of the sum-as-map-key discrimination pins.")
  (input
    (do
      (type T (A Int64) (B Int64))
      (def (main (: n Int64)) (Set.len #set((A n) (B n) (A 5))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (call main (: 7 Int64))
  (output (: 3 Int64)))

(case
  "a Set.of over PRELUDE-Option elements dedups Some by payload and keeps None distinct"
  (doc
    "The user-sum variant case above declares its own sum; this pins the PRELUDE Option as the
           element (the ord-wrapper builtin-sum face, which lowered separately and used to decline on the
           rust targets while user sums worked): `{(Some a), (None unit), (Some a), (Some (+ a 1))}` at
           a=3 — the two `(Some 3)`s collapse, `(None unit)` and `(Some 4)` stay distinct → len 3. The
           element compare must read the variant tag (None ≠ Some) and descend the Some payload, exactly
           as for a user sum.")
  (input
    (do
      (def (main (: a Int64)) (Set.len #set((Some a) (None unit) (Some a) (Some (+ a 1)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64)))

(case
  "a Set.of over Result elements separates Ok from Err with EQUAL payloads"
  (doc
    "The Result companion: `{(Ok a), (Err a)}` with the SAME payload value in both — only the
           variant tag distinguishes them, so a payload-only element hash would collapse the set to 1.
           Expected len 2. The prelude two-arm sum where each arm carries the same scalar, the sharpest
           tag-must-participate witness.")
  (input
    (do
      (def
        (main (: a Int64))
        (Set.len #set((: (Ok a) (Result Int64 Int64)) (: (Err a) (Result Int64 Int64)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "Set.contains dispatches on an Option element at a runtime payload"
  (doc
    "The membership face of the prelude-Option element: `{(Some 5), (None unit)}` probed with
           `(Some a)` — a=5 hits (the stored Some's payload matches), a=6 misses (same tag, different
           payload → false, not a trap). One compiled contains must walk tag-then-payload per call.")
  (input
    (do
      (def (main (: a Int64)) (if (Set.contains #set((Some 5) (None unit)) (Some a)) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "membership of an absent element is false, not a trap"
  (doc
    "The absent companion: `(Set.contains (Set.of (list 1 2 3)) 5)` tests an element not in the
           set, so the total predicate yields false — NOT a trap and NOT an error (collections-and-text.md
           #Set Membership Is Total). Pins that absence is an ordinary false, the reason membership needs
           no Option: a Bool already distinguishes present from absent totally.")
  (input (Set.contains #set(1 2 3) 5))
  (output (: false Bool)))

(case
  "membership of an absent element in the empty set is false"
  (doc
    "The degenerate boundary: `(Set.contains (Set.of (list)) 1)` tests membership in the empty set
           — nothing is present — so it is false. Pins that the total predicate handles the empty set
           without underflow, mirroring the empty-list / empty-map degenerate cases.")
  (input (Set.contains #set() 1))
  (output (: false Bool)))

; --- Set patterns match by CONTAINMENT ------------------------------------------------------
; A `#set(e…)` PATTERN in a match matches iff the scrutinee set CONTAINS every listed element — a subset
; test, NOT set equality. This mirrors the map pattern (`#map((= k v)…)` matches any map containing the
; listed keys, binding their values) and the open-row record pattern (a subset of fields): a collection
; pattern names a REQUIRED SUBSTRUCTURE, ignoring the rest. So `#set(1)` matches `{1,2}`, the empty pattern
; `#set()` matches every set, an order-permuted pattern matches (a set is unordered), and a pattern naming
; an element the scrutinee lacks (a superset, or a disjoint element) does NOT match and falls to the next
; arm. Element order and duplicates in the pattern are immaterial (it denotes a set). All const-folded here.
(case
  "a set pattern matches a scrutinee set with the SAME elements"
  (input (match #set(1 2) (#set(1 2) 9) (_ 0)))
  (output (: 9 Int64)))

(case
  "a set pattern is order-independent (a set is unordered)"
  (input (match #set(1 2) (#set(2 1) 9) (_ 0)))
  (output (: 9 Int64)))

(case
  "a set pattern naming a SUBSET matches by containment, not equality"
  (doc
    "`#set(1)` over `{1,2}` MATCHES — a set pattern is a containment (subset) test: the scrutinee
           contains the listed element, so it matches even though the sets are not equal. This is the set
           analogue of the map pattern matching a map that contains the named key.")
  (input (match #set(1 2) (#set(1) 9) (_ 0)))
  (output (: 9 Int64)))

(case
  "the empty set pattern matches every set"
  (doc
    "`#set()` names no required element, so it matches any scrutinee set (the empty set is a subset of
           every set) — the containment identity, the set twin of a bare wildcard for the membership axis.")
  (input (match #set(1 2) (#set() 9) (_ 0)))
  (output (: 9 Int64)))

(case
  "a set pattern naming an element the scrutinee lacks does NOT match"
  (doc
    "The containment test is directional: `#set(1 2 3)` over `{1,2}` does NOT match (the scrutinee
           lacks 3), and a disjoint `#set(3)` does not either — both fall to the `_` arm → 0. Pins that a
           set pattern requires its elements to be PRESENT, so a superset or disjoint pattern is refuted.")
  (input (match #set(1 2) (#set(1 2 3) 9) (_ 0)))
  (output (: 0 Int64)))

(case
  "a set pattern disjoint from the scrutinee does not match"
  (input (match #set(1 2) (#set(3) 9) (_ 0)))
  (output (: 0 Int64)))

; A set-pattern element is an ORDINARY VALUE EXPRESSION (the set twin of a map KEY), not a binder — the
; pattern matches iff the scrutinee CONTAINS each element's value. So a RUNTIME in-scope name works as an
; element: `#set(k)` for a parameter `k` matches when `k`'s value is a member, reads no binding, and is NOT
; flagged an unused match binding (it is a value ref, not a binder — a regression guard for #6693, where
; `arm_pattern_binders` wrongly collected an in-scope element as a binder and spuriously raised CDZ0306). A
; bare name the scrutinee-scope does NOT bind is therefore a genuine UNBOUND VALUE reference (CDZ0101), not
; a binder — to bind set contents, use `Set.contains` / `Set.len` instead.
(case
  "a set pattern with a RUNTIME in-scope element matches by membership of its value"
  (input (do (def (f (: k Int64)) (match #set(1 2) (#set(k) 9) (_ 0))) (export f)))
  (call f (: 1 Int64))
  (output (: 9 Int64))
  (call f (: 5 Int64))
  (output (: 0 Int64))
  ; `k` is a VALUE expression (the membership element), NOT a binder — so it must not be mis-collected as an
  ; arm binder and spuriously warned CDZ0306 unused (the regression this guards; migrated from rcdzc
  ; a_set_membership_element_is_a_value_expression_not_a_binder).
  (no-diagnostic "unused"))

(case
  "a set-pattern element that names no in-scope value is an unbound value reference (CDZ0101)"
  (input (do (def (main) (match #set(1 2) (#set(a) 9) (_ 0))) (export main)))
  ; the CDZ0101 carries a STEER (v-spec-oracle ruling #6685): a set names members BY VALUE and does not bind
  ; them; use an in-scope value or query the whole set with Set.contains / Set.len. (Steer facets migrated
  ; from rcdzc a_set_membership_element_is_a_value_expression_not_a_binder.)
  (error CDZ0101
    (message "unbound name `a`")
    (message "does not bind")
    (message "Set.contains")))

; A set match must END IN A CATCH-ALL (`_` or a whole-set binder): a set's element set is UNBOUNDED, so
; membership patterns — including the matches-any `#set()` — never exhaust it, and the exhaustiveness checker
; conservatively rejects a set match without a terminal catch-all (CDZ0210). This is the set analogue of the
; map rule (05 "the exhaustiveness checker conservatively rejects `(map)` as a match's terminal catch-all").
; Note `#set()` DOES match any set WHEN REACHED (pinned above) — but it is not accepted AS the exhaustiveness
; witness. The batch-97/#6693 set-pattern cases above all pair a membership arm with a `_`, exactly as this
; rule requires.
(case
  "a set match with no terminal catch-all is rejected as non-exhaustive (CDZ0210)"
  (input (do (def (f (: s (Set Int64))) (match s (#set() 1))) (export f)))
  (error CDZ0210 (message "a set match must end in a catch-all") (message "unbounded")))

; --- Set rest patterns ---------------------------------------------------------------------
; A set membership pattern MAY end in a REST BINDER — `#set(e… .. rest)` — which binds `rest` to a set of
; the same element type containing every element of the scrutinee EXCEPT the named ones, so the named
; elements are consumed and the remainder is available (core-semantics.md §"A Set Is Matched By Element-
; Membership Patterns"). The rest binder is the ONLY binder position in a set pattern (the named elements
; stay ordinary value expressions per the containment pins above); the residual is a genuine Set value —
; equal to the scrutinee with the named elements removed, and to the empty set when every element is named.
; Containment is unrelaxed by a rest binder: the arm still fires only when the scrutinee CONTAINS every
; named element, so a rest arm naming an absent element is refuted and falls through. Const-folded here (the
; residual is compared by set equality, so these cases are runtime-independent). Landed #6711 (matcher) atop
; #6698 (Resolved::SetRest + (Set E) typing); the set twin of the map / record rest binder.
(case
  "a set rest pattern binds the residual = scrutinee MINUS the named element"
  (input (match #set(1 2 3) (#set(2 (.. rest)) (if (= rest #set(1 3)) 1 0)) (_ -1)))
  (output (: 1 Int64)))

(case
  "the set rest binder holds exactly the scrutinee elements minus the named ones"
  (input (match #set(1 2 3) (#set(1 (.. rest)) (if (= rest #set(2 3)) 100 -100)) (_ -1)))
  (output (: 100 Int64)))

(case
  "a set rest pattern naming every element binds the EMPTY residual set"
  (input (match #set(1 2 3) (#set(1 2 3 (.. rest)) (if (= rest #set()) 7 -7)) (_ -1)))
  (output (: 7 Int64)))

(case
  "a set rest pattern still requires the named elements PRESENT (absent named refutes the arm)"
  (input (match #set(1 2) (#set(3 (.. rest)) 9) (_ -1)))
  (output (: -1 Int64)))

(case
  "the empty set has cardinality zero"
  (doc
    "The degenerate cardinality boundary: `(Set.len (Set.of (list)))` is 0 — the empty set holds no
           elements. The len companion of the empty-set membership pin above, and the both-backend witness
           of the unconstrained-empty-`Set.of (list)` emit: wasm defaults the element type in emit, and the
           rust backend grounds the empty `BTreeSet` to a concrete element type (not a bare `BTreeSet<_>`
           that fails rustc inference with E0282). Mirrors the empty-list / empty-map degenerate cardinality.")
  (input (Set.len #set()))
  (output (: 0 Int64)))

(case
  "an empty set passed to a recursive callee with a non-Int64 element param grounds its element type from the param"
  (doc
    "The empty-set element type must be fixed by the CALLEE's declared parameter at a CALL-ARGUMENT
           position — not just by an enclosing insert. `(loop 3 (Set.of (list)))` where `loop`'s param is
           `(: s (Set Float64))` and NO insert anywhere: the callee param `(Set Float64)` is the only fixer.
           wasm defaults the empty set's element type in emit and runs (0); the RUST backend used to emit the
           i64 DEFAULT `BTreeSet<i64>` for the empty-set call-arg (it grounded via an enclosing insert/remove
           but did NOT consult the callee param type), giving E0308 'expected BTreeSet<__CdzF64>, found
           BTreeSet<i64>' — a build failure while wasm computed. The fix grounds the empty-collection element
           from the callee param at the call-arg. SET-SPECIFIC (the List twin already grounded). Pins the
           build-set-by-recursion-over-floats idiom with an empty seed → 0 on all backends.")
  (input
    (do
      (def (loop (: n Int64) (: s (Set Float64))) (if (= n 0) (Set.len s) (loop (- n 1) s)))
      (def (main) (loop 3 #set()))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "an empty map passed to a recursive callee with a non-Int64 key param grounds its key type from the param"
  (doc
    "The Map twin of the empty-set-at-call-arg case above: `(loop 3 Map.empty)` where `loop`'s param is
           `(: m (Map Float64 Int64))` and no insert — the callee param `(Map Float64 Int64)` is the only fixer
           for the empty map's key/value types. wasm runs (0); the rust backend must ground the empty
           `BTreeMap` key/value from the callee param (not the i64 default) so it does not E0308. Pins that the
           empty-collection call-arg element-type grounding covers Map keys as well as Set elements → 0.")
  (input
    (do
      (def (loop (: n Int64) (: m (Map Float64 Int64))) (if (= n 0) (Map.len m) (loop (- n 1) m)))
      (def (main) (loop 3 Map.empty))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "the number of elements counts distinct elements"
  (doc
    "`(Set.len (Set.of (list 1 2 2 3)))` is 3 — the count of DISTINCT elements, since the duplicate
           2 is held once (collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that
           len reports the set's cardinality after deduplication, not the source list's length 4.")
  (input (Set.len #set(1 2 2 3)))
  (output (: 3 Int64)))

(case
  "inserting an element yields a set containing it"
  (doc
    "`(Set.insert (Set.of (list 1 2)) 3)` produces a new set {1, 2, 3} — the value heap is
           immutable, so insert returns a new set rather than mutating (memory-and-resource-model.md).
           It equals `(Set.of (list 1 2 3))`. MUST be true.")
  (input (= (Set.insert #set(1 2) 3) #set(1 2 3)))
  (output (: true Bool)))

(case
  "inserting a present element is a no-op value"
  (doc
    "`(Set.insert (Set.of (list 1 2 3)) 2)` inserts an element already present, so the result still
           holds 2 once — it equals the original `(Set.of (list 1 2 3))` (collections-and-text.md #A Set
           Is A Collection Of Unique Elements: each element at most once). Pins that insert preserves
           uniqueness rather than creating a second 2. MUST be true.")
  (input (= (Set.insert #set(1 2 3) 2) #set(1 2 3)))
  (output (: true Bool)))

(case
  "removing an element yields a set without it"
  (doc
    "`(Set.remove (Set.of (list 1 2 3)) 2)` produces a new set {1, 3} without 2 — it equals
           `(Set.of (list 1 3))`. Pins that remove drops exactly the named element and returns a new
           persistent set. MUST be true.")
  (input (= (Set.remove #set(1 2 3) 2) #set(1 3)))
  (output (: true Bool)))

(case
  "the union contains the elements of either set"
  (doc
    "Witnesses set algebra: `(Set.union (Set.of (list 1 2)) (Set.of (list 2 3)))` is {1, 2, 3} —
           every element in either operand, with the shared 2 held once. It equals `(Set.of (list 1 2
           3))`. MUST be true.")
  (input (= (Set.union #set(1 2) #set(2 3)) #set(1 2 3)))
  (output (: true Bool)))

(case
  "the intersection contains the elements in both sets"
  (doc
    "`(Set.intersection (Set.of (list 1 2 3)) (Set.of (list 2 3 4)))` is {2, 3} — the elements
           present in both operands — equal to `(Set.of (list 2 3))`. MUST be true.")
  (input (= (Set.intersection #set(1 2 3) #set(2 3 4)) #set(2 3)))
  (output (: true Bool)))

(case
  "the difference contains the elements not in the second set"
  (doc
    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list 2 3)))` is {1} — the elements of the
           first set not in the second — equal to `(Set.of (list 1))`. Pins the asymmetry of difference:
           elements of the second operand not in the first do not appear. MUST be true.")
  (input (= (Set.difference #set(1 2 3) #set(2 3)) #set(1)))
  (output (: true Bool)))

(case
  "the intersection of sets of TUPLES matches elements by their whole-tuple content"
  (doc
    "The intersection extends to COMPOUND elements: `(Set.intersection {(1,2),(3,4)} {(3,4),(5,6)})` =
           {(3,4)} — the tuple `(3, 4)` is the only element in both. Its membership is decided by the whole
           tuple's content (the same content-address equality that dedups a set of tuples), NOT by identity,
           so a separately-built `(tuple 3 4)` in each operand intersects. Pins set intersection over the
           CHAMP compound-element path (a distinct hashing/compare from the scalar cases above). Asserted by
           STRUCTURAL equality to `(Set.of (list (tuple 3 4)))` — a set-value `=` walking the CHAMP compound
           elements — so a WRONG surviving tuple (e.g. keeping (1,2) or (5,6)) would fail, not merely a
           wrong count as a `Set.len`-only check would miss.")
  (input
    (=
      (Set.intersection #set(#tuple(1 2) #tuple(3 4)) #set(#tuple(3 4) #tuple(5 6)))
      #set(#tuple(3 4))))
  (output (: true Bool)))

(case
  "the difference of sets of TUPLES removes elements by their whole-tuple content"
  (doc
    "The difference likewise extends to compound elements: `(Set.difference {(1,2),(3,4)} {(1,2)})` =
           {(3,4)} — the tuple `(1, 2)` present in the second operand is removed by content, leaving one
           element. The compound-element companion of the scalar difference case, pinning that a tuple in
           the subtrahend is matched by its whole content on the CHAMP path. Asserted by STRUCTURAL equality
           to `(Set.of (list (tuple 3 4)))` — the set-value `=` compares the whole surviving element, so a
           difference that removed the wrong tuple (leaving (1,2)) would fail, which a `Set.len`-only check
           of the count would not catch.")
  (input (= (Set.difference #set(#tuple(1 2) #tuple(3 4)) #set(#tuple(1 2))) #set(#tuple(3 4))))
  (output (: true Bool)))

(case
  "two sets of TUPLES built in different orders compare equal through the compound elements"
  (doc
    "The runtime whole-set eq face: the intersection/difference pins above assert set-eq on
           CONST-folded operands; here a RUNTIME v inside one tuple forces the LIVE CHAMP walk, the
           operands are built in different orders, and the v=5 NEGATIVE face witnesses the equality
           discriminates (the landed pins have no unequal face).")
  (input
    (do
      (def
        (main (: v Int64))
        (do
          (def s1 #set(#tuple(1 2) #tuple(3 v)))
          (def s2 #set(#tuple(3 4) #tuple(1 2)))
          (if (= s1 s2) 1 0)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "the two algebraic subset formulations agree on subset, superset, and equal operands"
  (doc
    "No subset predicate exists in the surface; the two algebraic encodings — A⊆B as
           (A∪B)=B and as (A∩B)=A — route through DIFFERENT op+eq pipelines and must agree at every
           face: proper subset (11), proper superset (00), equal operands (11, reflexivity). A bug
           in union, intersection, or set-eq breaks one digit of the agreement.")
  (input
    (do
      (def (sub1 (: a (Set Int64)) (: b (Set Int64))) (if (= (Set.union a b) b) 1 0))
      (def (sub2 (: a (Set Int64)) (: b (Set Int64))) (if (= (Set.intersection a b) a) 1 0))
      (def (both (: a (Set Int64)) (: b (Set Int64))) (+ (* (sub1 a b) 10) (sub2 a b)))
      (def
        (main (: n Int64))
        (do
          (def small #set(1 n))
          (def big #set(1 2 3))
          (+ (* (both small big) 10000) (+ (* (both big small) 100) (both small small)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 110011 Int64))
  (call main (: 9 Int64))
  (output (: 11 Int64)))

(case
  "difference DISTRIBUTES over union but is NOT associative - both facts witnessed live"
  (doc
    "A LAW and a NON-law in one program: (A∪B)∖C = (A∖C)∪(B∖C) must HOLD (digit 1) while
           (A∖B)∖C = A∖(B∖C) must FAIL ({1} vs {1,2}, digit 0) — the law digit catches an op bug,
           the NON-law digit catches an over-eager rewrite treating difference as associative.
           Runtime n threads all three sets.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def a #set(1 n))
          (def b #set(n 3))
          (def c #set(n))
          (def
            dist
            (if
              (=
                (Set.difference (Set.union a b) c)
                (Set.union (Set.difference a c) (Set.difference b c)))
              1
              0))
          (def
            nonassoc
            (if
              (= (Set.difference (Set.difference a b) c) (Set.difference a (Set.difference b c)))
              1
              0))
          (+ (* dist 10) nonassoc)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 10 Int64)))

(case
  "SYMMETRIC difference built two ways from primitives agrees and dedups the overlap"
  (doc
    "No sym-diff op exists; the two derived forms — (A∪B)∖(A∩B) and (A∖B)∪(B∖A) — must
           agree (three ops each through different pipelines), with len + membership digits. The
           n=1 face feeds Set.of DUPLICATE-carrying lists ((1 2 1) → {1,2}) so construction dedup
           composes into the algebra; 1 lands in the intersection there, so contains-1 is 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def a #set(1 2 n))
          (def b #set(2 n 4))
          (def l (Set.difference (Set.union a b) (Set.intersection a b)))
          (def r (Set.union (Set.difference a b) (Set.difference b a)))
          (+ (* (if (= l r) 1 0) 100) (+ (* (Set.len l) 10) (if (Set.contains l 1) 1 0)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 121 Int64))
  (call main (: 1 Int64))
  (output (: 110 Int64)))

(case
  "the union of sets of TUPLES holds a shared tuple once and preserves component order"
  (doc
    "The UNION member of the compound-element algebra (intersection/difference above): `(Set.union
           {(1,2),(3,a)} {(3,4),(5,6)})` at a = 4 shares the tuple (3,4) — the union must hold it ONCE
           (len 3, dedup by whole-tuple content across the CHAMP union walk), while at a = 9 the operands
           are disjoint (len 4). Membership after the union stays component-ORDER-sensitive: `(contains u
           (tuple 3 a))` is true (the shared/first-operand tuple survives) but `(contains u (tuple 4 3))`
           — the shared tuple's components CROSSED — is false. Encodes 100·len + 10·has(3,a) + has(4,3) =
           310 (a=4) / 410 (a=9). Runtime `a` keeps the sets and the walk out of the fold. Expected: 310,
           410.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((s1 #set(#tuple(1 2) #tuple(3 a))) (s2 #set(#tuple(3 4) #tuple(5 6))))
          (let
            ((u (Set.union s1 s2)))
            (+
              (* 100 (Set.len u))
              (+ (* 10 (if (Set.contains u #tuple(3 a)) 1 0)) (if (Set.contains u #tuple(4 3)) 1 0))))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 310 Int64))
  (call main (: 9 Int64))
  (output (: 410 Int64)))

(case
  "set algebra descends into FLOAT-leaf tuple elements"
  (doc
    "The FLOAT-leaf dimension of the compound-element set algebra above (those pins use integer
           tuples; the float leaf exercises the canonical-byte compare inside the CHAMP walk): `a =
           {(x,1),(2.5,2)}`, `b = {(0.5,1),(9.5,9)}` over runtime `x`. At `x = 0.5` the tuples `(0.5,1)`
           intersect by float-leaf content — union 3, intersection 1, difference 1 → 311. At `x = 7.5`
           the operands are disjoint — union 4, intersection 0, difference 2 → 402. A union/intersection
           that compared float leaves by identity (or a difference that missed the float slot) would break
           one of the encoded digits. All three ops in one shape, both regimes.")
  (input
    (do
      (def
        (main (: x Float64))
        (let
          ((a #set(#tuple(x 1) #tuple(2.5 2))) (b #set(#tuple(0.5 1) #tuple(9.5 9))))
          (+
            (* 100 (Set.len (Set.union a b)))
            (+ (* 10 (Set.len (Set.intersection a b))) (Set.len (Set.difference a b))))))
      (export main)))
  (call main (: 0.5 Float64))
  (output (: 311 Int64))
  (call main (: 7.5 Float64))
  (output (: 402 Int64)))

(case
  "a binary set operation leaves BOTH its operands unchanged — set-algebra persistence"
  (doc
    "The persistence face of the binary set algebra, the two-operand twin of the single-element
           Set.insert/Set.remove persistence pins: `Set.difference` (like union/intersection) produces a NEW
           set and MUST leave BOTH operands unchanged (a value must not be observably mutated through one
           reference while read through another). `a = {0,1,2}` and `b = {1,2}` are genuine runtime sets, each
           read AFTER the difference. `d = (Set.difference a b)` = {0}. Encodes `1000·len(d) + 100·len(a) +
           10·len(b) + (a contains 2 ? 1 : 0)`: d is {0} (len 1); the original `a` is STILL {0,1,2} — len 3
           AND `Set.contains a 2` is true (the 2 that the difference removed FROM THE RESULT is untouched in
           `a`); the original `b` is still {1,2} (len 2). → 1321. If the difference FBIP-mutated a shared
           operand in place (a retain missing on a multi-use binding), a's later len/membership read would
           see the mutation. Both backends. Completes the persistence family across binary ops, beside the
           single-element insert/remove/take cases.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (main)
        (let
          ((a (build 0 3 #set())) (b (build 1 3 #set())))
          (let
            ((d (Set.difference a b)))
            (+
              (* 1000 (Set.len d))
              (+ (* 100 (Set.len a)) (+ (* 10 (Set.len b)) (if (Set.contains a 2) 1 0)))))))
      (export main)))
  (output (: 1321 Int64)))

(case
  "a 1000-element set drains through Set.remove to empty"
  (doc
    "The Set-side deep shrink at scale (the 50-entry Map grow/shrink/regrow twin exists; the SET
           trie's full-drain node collapse was unpinned at any size): build 1000 elements (multi-level
           CHAMP), then `Set.remove` every one — len 1000 before, len 0 after (10000000 = 1000·10000+0).
           A remove path that mis-merged a collapsing node partway down loses or strands an element.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (drain (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (drain (+ i 1) n (Set.remove s i)) s))
      (def
        (main (: n Int64))
        (let ((s (build 0 n #set()))) (+ (* 10000 (Set.len s)) (Set.len (drain 0 n s)))))
      (export main)))
  (call main (: 1000 Int64))
  (output (: 10000000 Int64)))

(case
  "a TOGGLE fold flips set membership per occurrence — odd count in, even count out"
  (doc
    "The parity/light-switch idiom: each occurrence FLIPS membership (`contains ? remove :
           insert`), so an element ends IN the set iff it occurs an ODD number of times. The drain
           case above removes monotonically; the churn here is INSERT-AFTER-REMOVE of the same
           element within one fold — the CHAMP node must rebuild correctly through a full
           insert→remove→insert cycle at one key. Over `(n 1 2 n 1 3 n)` the runtime n merges its
           occurrence count with whichever literal it collides with: n=5 → n×3 (odd, IN), 1×2 (out),
           2,3×1 (in) → {5,2,3}, len 3, encoding 301; n=1 → 1 occurs 5× (IN) → {1,2,3} → 311; n=2 →
           2 occurs 4× (even, OUT — one flip miscounted at the collided key leaves it in) and the
           1s cancel too → {3} → 100. Encoding: len·100 + contains(1)·10 + contains(n).")
  (input
    (do
      (def
        (toggle (: xs (List Int64)) (: s (Set Int64)))
        (match
          xs
          (#list() s)
          (#list(h (.. t)) (toggle t (if (Set.contains s h) (Set.remove s h) (Set.insert s h))))))
      (def
        (main (: n Int64))
        (do
          (def s (toggle #list(n 1 2 n 1 3 n) #set()))
          (+ (* 100 (Set.len s)) (+ (* 10 (if (Set.contains s 1) 1 0)) (if (Set.contains s n) 1 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 301 Int64))
  (call main (: 1 Int64))
  (output (: 311 Int64))
  (call main (: 2 Int64))
  (output (: 100 Int64))
  (live-objects 0))

(case
  "TWO-SUM finds a complement pair via a seen-set built during one walk"
  (doc
    "The complement-lookup idiom: one walk over `(2 7 11 15 3)` tests `target − h` against the
           set of elements seen SO FAR, inserting h only after the test — so an element can never
           pair with ITSELF (target 4 = 2+2 must return -1, not pair the single 2 with its own
           entry; the test-before-insert ordering is the pin). Returns the SECOND element's index at
           the first hit. The toggle pin above churns one key; here the set only GROWS but the probed
           key (the complement) is never the inserted key (h) — a CHAMP lookup keyed on the wrong
           side finds h and answers one step early. Faces: target 9 → i=1 (2+7, immediate); 18 → 2
           (7+11); 5 → 4 (2+3 — the complement entered the set FOUR steps before the hit); 4 → -1
           (the self-pair trap); 14 → 4 (11+3 — TWO candidate pairs exist but 7+7 needs a self-pair
           and must NOT fire at i=1).")
  (input
    (do
      (def
        (walk (: xs (List Int64)) (: target Int64) (: seen (Set Int64)) (: i Int64))
        (match
          xs
          (#list() -1)
          (#list(h (.. t))
            (if (Set.contains seen (- target h)) i (walk t target (Set.insert seen h) (+ i 1))))))
      (def (two-sum (: xs (List Int64)) (: target Int64)) (walk xs target #set() 0))
      (def (main (: target Int64)) (two-sum #list(2 7 11 15 3) target))
      (export main)))
  (call main (: 9 Int64))
  (output (: 1 Int64))
  (call main (: 18 Int64))
  (output (: 2 Int64))
  (call main (: 5 Int64))
  (output (: 4 Int64))
  (call main (: 4 Int64))
  (output (: -1 Int64))
  (call main (: 14 Int64))
  (output (: 4 Int64))
  ; Per-call (B2 #5101): the seen-set accumulator leak SCALES with the number of `Set.insert seen h`
  ; before the walk stops, then plateaus — 1/2/4/5/4 inserts → 3/5/9/9/9 live. The whole-case
  ; `known-leak 3` only matched call 0 (target 9, 1 insert); the true per-call vector is recorded here.
  ; Underlying leak = a runtime Set/CHAMP accumulator + per-iteration walk cells not reclaimed (routed
  ; to v-core-opt; distinct from the 3050 String-view/backing class). (v-memory-safety, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "HAPPY NUMBER iteration detects the 4-cycle with a seen-set and counts steps to resolution"
  (doc
    "Cycle detection via a seen-set over a NUMERIC orbit (the two-sum above probes complements;
           here the set tracks the iteration's own history): the squared-digit-sum map either reaches
           the fixed point 1 (happy) or falls into the 4→16→37→58→89→145→42→20→4 cycle — the walk
           stops the moment a value RE-APPEARS (the seen-check must precede the step, or the cycle
           spins forever; same termination discipline as the graph pins but over an arithmetic orbit,
           not an explicit edge list). Faces: 19 → happy in 4 steps (82→68→100→1 — 104); 4 → INSIDE
           the cycle from the start, detected when 4 recurs after the full 8-step loop (8); 7 →
           happy in 5 (105); 1 → the fixed point itself, ZERO steps, seen-set never grows (100).
           Encoding: happy-bit·100 + steps.")
  (input
    (do
      (def
        (sq-digits (: n Int64) (: acc Int64))
        (if (= n 0) acc (sq-digits (/ n 10) (+ acc (* (% n 10) (% n 10))))))
      (def
        (walk (: n Int64) (: seen (Set Int64)) (: steps Int64))
        (if
          (= n 1)
          #tuple(1 steps)
          (if
            (Set.contains seen n)
            #tuple(0 steps)
            (walk (sq-digits n 0) (Set.insert seen n) (+ steps 1)))))
      (def (main (: n Int64)) (match (walk n #set() 0) (#tuple(h steps) (+ (* h 100) steps))))
      (export main)))
  (call main (: 19 Int64))
  (output (: 104 Int64))
  (call main (: 4 Int64))
  (output (: 8 Int64))
  (call main (: 7 Int64))
  (output (: 105 Int64))
  (call main (: 1 Int64))
  (output (: 100 Int64))
  ; per-call (B2): the seen-set orbit accumulator scales with iteration length then plateaus (was coarse
  ; whole-case known-leak 2, matched only call 0). true vector: 2/1/1/0. (v-memory-safety re-baseline, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "graph REACHABILITY drains a worklist against a visited-set over a Map adjacency list"
  (doc
    "The worklist algorithm — the compiler's own reachability shape: a `(Map Int64 (List Int64))`
           adjacency graph, a LIST worklist popped from the front with each node's neighbors pushed
           to the BACK, and a visited SET guarding re-entry (the seen-check must fire BEFORE
           expansion, or the 5⇄6 CYCLE face loops forever — termination itself is the pinned
           property). The graph has a DIAMOND (1→2→4, 1→3→4 — node 4 enters the worklist TWICE and
           must be expanded once) and a 2-cycle island DISCONNECTED from it (5⇄6 — reachability from
           1 must NOT leak into the island, and from 5 it must terminate despite the cycle). Faces:
           start=1 → {1,2,3,4} (len 4, sum 10 → 410); start=5 → the cycle island {5,6} (211);
           start=4 → the sink alone (104). Encoding: len·100 + element sum via Set.to-list.")
  (input
    (do
      (def
        (nbrs (: g (Map Int64 (List Int64))) (: n Int64))
        (match (Map.lookup g n) ((Some xs) xs) ((None _u) #list())))
      (def
        (push-all (: xs (List Int64)) (: work (List Int64)))
        (match xs (#list() work) (#list(h (.. t)) (push-all t (List.push work h)))))
      (def
        (drain (: g (Map Int64 (List Int64))) (: work (List Int64)) (: seen (Set Int64)))
        (match
          work
          (#list() seen)
          (#list(h (.. t))
            (if
              (Set.contains seen h)
              (drain g t seen)
              (drain g (push-all (nbrs g h) t) (Set.insert seen h))))))
      (def
        (sum-set (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-set t (+ acc h)))))
      (def
        (main (: start Int64))
        (do
          (def
            g
            (Map.insert
              (Map.insert
                (Map.insert
                  (Map.insert
                    (Map.insert (Map.insert Map.empty 1 #list(2 3)) 2 #list(4))
                    3
                    #list(4))
                  4
                  #list())
                5
                #list(6))
              6
              #list(5)))
          (def seen (drain g #list(start) #set()))
          (+ (* (Set.len seen) 100) (sum-set (Set.to-list seen) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 410 Int64))
  (call main (: 5 Int64))
  (output (: 211 Int64))
  (call main (: 4 Int64))
  (output (: 104 Int64))
  ; per-call (B2): the worklist/visited-set accumulator scales with the traversal (was coarse whole-case 15,
  ; matched only call 0). true vector: 15/9/4. (v-memory-safety re-baseline, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "BIPARTITE check two-colors components and rejects the odd cycle"
  (doc
    "The 2-coloring member of the graph family (reachability above, topo-sort below, HAPPY
           orbit further down): BFS coloring over the same Map-adjacency shape — each neighbor gets
           1−parent's color through the worklist; a SAME-color edge discovered mid-drain is the
           odd-cycle witness and rejects immediately (visit-nbrs threads a Bool ok flag through its
           tuple so the failure short-circuits the drain). Absence is typed, not sentinel: `col-of`
           returns `(Option Int64)` — `(None unit)` for an uncolored node, `(Some c)` for a colored
           one — and every caller MATCHES it (the worklist-node lookup in `drain` is `(Some uc)` by
           construction, so its `None` arm traps). The outer loop seeds color 0 at each
           still-uncolored start, covering MULTI-COMPONENT graphs. Faces: mode 1 = even 4-cycle
           1→2→3→4→1 → bipartite (1); mode 2 = ODD 3-cycle 1→2→3→1 → rejected (0); mode 3 = two
           DISCONNECTED edges 1→2, 3→4 → each component colors independently (1).")
  (input
    (do
      (def
        (nbrs (: g (Map Int64 (List Int64))) (: n Int64))
        (match (Map.lookup g n) ((Some xs) xs) ((None _u) #list())))
      (def
        (col-of (: colors (Map Int64 Int64)) (: n Int64))
        (match (Map.lookup colors n) ((Some c) (Some c)) ((None _u) (None unit))))
      (def
        (visit-nbrs
          (: es (List Int64))
          (: uc Int64)
          (: colors (Map Int64 Int64))
          (: work (List Int64)))
        (match
          es
          (#list() #tuple(true colors work))
          (#list(v (.. t))
            (match
              (col-of colors v)
              ((None _u) (visit-nbrs t uc (Map.insert colors v (- 1 uc)) (List.push work v)))
              ((Some vc) (if (= vc uc) #tuple(false colors work) (visit-nbrs t uc colors work)))))))
      (def
        (drain (: g (Map Int64 (List Int64))) (: work (List Int64)) (: colors (Map Int64 Int64)))
        (match
          work
          (#list() #tuple(true colors))
          (#list(u (.. t))
            (match
              (col-of colors u)
              ((None _u) (trap "unreachable: worklist node is uncolored"))
              ((Some uc)
                (match
                  (visit-nbrs (nbrs g u) uc colors t)
                  (#tuple(ok colors2 work2) (if ok (drain g work2 colors2) #tuple(false colors2)))))))))
      (def
        (all-nodes (: ns (List Int64)) (: g (Map Int64 (List Int64))) (: colors (Map Int64 Int64)))
        (match
          ns
          (#list() 1)
          (#list(s (.. t))
            (match
              (col-of colors s)
              ((None _u)
                (match
                  (drain g #list(s) (Map.insert colors s 0))
                  (#tuple(ok colors2) (if ok (all-nodes t g colors2) 0))))
              ((Some _c) (all-nodes t g colors))))))
      (def
        (main (: mode Int64))
        (do
          (def
            g
            (if
              (= mode 1)
              (Map.insert
                (Map.insert (Map.insert (Map.insert Map.empty 1 #list(2)) 2 #list(3)) 3 #list(4))
                4
                #list(1))
              (if
                (= mode 2)
                (Map.insert (Map.insert (Map.insert Map.empty 1 #list(2)) 2 #list(3)) 3 #list(1))
                (Map.insert (Map.insert Map.empty 1 #list(2)) 3 #list(4)))))
          (def ns (if (= mode 3) #list(1 2 3 4) (if (= mode 1) #list(1 2 3 4) #list(1 2 3))))
          (all-nodes ns g Map.empty)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  ; per-call (B2): the two-coloring visited/queue accumulator varies per component (was coarse whole-case 35,
  ; matched only call 0). true vector: 35/20/34. (v-memory-safety re-baseline, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "dropping a set derived by insert must not free members shared with the survivor"
  (doc
    "The SET member of the generation-sharing reclaim family (map/list members in 05-compound,
           rope in 13-strings): Sets are CHAMP-backed like Maps but with ELEMENT-ONLY nodes, so the
           reclaim walk has its own shape. s2 = Set.insert s1 4 shares s1's interior nodes; mode 1
           keeps the BASE and drops the derivative — the Set.contains walk then traverses exactly
           the shared nodes after the drop; mode 2 keeps the derivative past the base's last use
           and additionally hits the fresh element. Encodes len·100 + contains-2·10 + contains-4:
           mode 1 → 300+10+0 = 310, mode 2 → 400+10+1 = 411.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (Set Int64)))
        (if (> i n) acc (build (+ i 1) n (Set.insert acc i))))
      (def
        (main (: mode Int64))
        (do
          (def s1 (build 1 3 #set()))
          (def s2 (Set.insert s1 4))
          (def keep (if (= mode 1) s1 s2))
          (+
            (* (Set.len keep) 100)
            (+ (* (if (Set.contains keep 2) 1 0) 10) (if (Set.contains keep 4) 1 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 310 Int64))
  (call main (: 2 Int64))
  (output (: 411 Int64))
  ; RECLAIM WIN: the SET member of the if-join-shared-child family — mode 2 (keep=s2 the derivative
  ; sharing s1's CHAMP nodes) formerly leaked 1 (the shallow cross-arm dup residual). The FAMILY fix
  ; #5382 (dup-skip for the in-place-reuse base of Set.insert; my directed distinguisher) reclaims it
  ; fully. Co-verified [0,0] on fresh cdz/store 05WfA5uY. (v-memory-safety, coord v-core-opt)
  (live-objects 0 0))

(case
  "a TOPOLOGICAL sort drains min-ready nodes and verifies every edge points forward"
  (doc
    "The dependency-ordering sibling of the reachability pin above (same Map-adjacency shape,
           opposite discipline: reachability EXPANDS from a start, topo-sort RETIRES nodes whose
           in-degree hits zero). In-degrees accumulate through the upsert `bump`; each round scans
           for the SMALLEST unretired zero-in-degree node (deterministic order — no ready-list
           ambiguity), retires it, and decrements its neighbors via bump -1 (the SAME upsert helper
           driven in both directions). Certified structurally: `edges-fwd` re-walks every edge
           checking source-position < target-position in the output (the defining property, checked
           independently of the expected literal). The extra=2 face adds edge 4→2, closing the CYCLE
           2→3→4→2: the drain STOPS when no zero-in-degree node remains — order (1 5), fwd-check 0
           (edge into the cycle fails) → 150; the acyclic face orders 1,2,3,5,4 with fwd 1 → 123541.")
  (input
    (do
      (def
        (nbrs (: g (Map Int64 (List Int64))) (: n Int64))
        (match (Map.lookup g n) ((Some xs) xs) ((None _u) #list())))
      (def
        (bump (: m (Map Int64 Int64)) (: k Int64) (: d Int64))
        (match (Map.lookup m k) ((Some v) (Map.insert m k (+ v d))) ((None _u) (Map.insert m k d))))
      (def
        (fold-edges (: es (List Int64)) (: mm (Map Int64 Int64)))
        (match es (#list() mm) (#list(e (.. et)) (fold-edges et (bump mm e 1)))))
      (def
        (indeg-of (: g (Map Int64 (List Int64))) (: ns (List Int64)) (: m (Map Int64 Int64)))
        (match ns (#list() m) (#list(h (.. t)) (indeg-of g t (fold-edges (nbrs g h) m)))))
      (def
        (get0 (: m (Map Int64 Int64)) (: k Int64))
        (match (Map.lookup m k) ((Some v) v) ((None _u) 0)))
      (def
        (min-ready
          (: ns (List Int64))
          (: indeg (Map Int64 Int64))
          (: done (Set Int64))
          (: best Int64))
        (match
          ns
          (#list() best)
          (#list(h (.. t))
            (min-ready
              t
              indeg
              done
              (if
                (if (Set.contains done h) false (= (get0 indeg h) 0))
                (if (< best 0) h (if (< h best) h best))
                best)))))
      (def
        (dec-nbrs (: es (List Int64)) (: indeg (Map Int64 Int64)))
        (match es (#list() indeg) (#list(e (.. t)) (dec-nbrs t (bump indeg e -1)))))
      (def
        (drain
          (: g (Map Int64 (List Int64)))
          (: ns (List Int64))
          (: indeg (Map Int64 Int64))
          (: done (Set Int64))
          (: acc (List Int64)))
        (do
          (def pick (min-ready ns indeg done -1))
          (if
            (< pick 0)
            acc
            (drain g ns (dec-nbrs (nbrs g pick) indeg) (Set.insert done pick) (List.push acc pick)))))
      (def
        (pos-of (: xs (List Int64)) (: v Int64) (: i Int64))
        (match xs (#list() -1) (#list(h (.. t)) (if (= h v) i (pos-of t v (+ i 1))))))
      (def
        (all-fwd (: es (List Int64)) (: order (List Int64)) (: hpos Int64))
        (match
          es
          (#list() 1)
          (#list(e (.. et)) (if (< hpos (pos-of order e 0)) (all-fwd et order hpos) 0))))
      (def
        (edges-fwd (: g (Map Int64 (List Int64))) (: order (List Int64)) (: ns (List Int64)))
        (match
          ns
          (#list() 1)
          (#list(h (.. t))
            (if (= (all-fwd (nbrs g h) order (pos-of order h 0)) 1) (edges-fwd g order t) 0))))
      (def
        (chk (: rs (List Int64)) (: acc Int64))
        (match rs (#list() acc) (#list(h (.. t)) (chk t (+ (* acc 10) h)))))
      (def
        (main (: extra Int64))
        (do
          (def ns #list(1 2 3 4 5))
          (def
            g0
            (Map.insert
              (Map.insert
                (Map.insert (Map.insert (Map.insert Map.empty 1 #list(3)) 2 #list(3)) 3 #list(4))
                4
                #list())
              5
              #list(4)))
          (def g (if (> extra 0) (Map.insert g0 4 #list(extra)) g0))
          (def order (drain g ns (indeg-of g ns Map.empty) #set() #list()))
          (+ (* (chk order 0) 10) (edges-fwd g order ns))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 123541 Int64))
  (call main (: 2 Int64))
  (output (: 150 Int64))
  ; per-call (B2): the in-degree/ready-set accumulator differs per graph (was coarse whole-case 83, matched
  ; only call 0). true vector: 83/24. (v-memory-safety re-baseline, coord v-corpus-harness)
  (live-objects known-leak))

; --- The algebraic laws the three operations satisfy: the empty set as identity/annihilator, and ----
; --- the union laws (commutative, idempotent). These pin the operations' DEFINING identities, which
; --- the overlapping-operand cases above (which give a nontrivial result) do not exercise — a
; --- degenerate operand (the empty set, the same set twice, disjoint sets) forces the boundary of
; --- each operation. A set is a collection of unique elements (collections-and-text.md #A Set Is A
; --- Collection Of Unique Elements), so these are the ordinary laws of finite-set algebra.
(case
  "union with the empty set is the set itself"
  (doc
    "`(Set.union (Set.of (list 1 2 3)) (Set.of (list)))` is {1, 2, 3} — the empty set is the
           identity of union, so unioning it in adds nothing. Pins the identity law the overlapping-union
           case does not (it has elements on both sides); the empty set is a genuine operand, not a
           trap. MUST be true.")
  (input (= (Set.union #set(1 2 3) #set()) #set(1 2 3)))
  (output (: true Bool)))

(case
  "intersection with the empty set is the empty set"
  (doc
    "`(Set.intersection (Set.of (list 1 2 3)) (Set.of (list)))` is {} — the empty set is the
           annihilator of intersection, since no element is in both. Pins the annihilator law (the dual of
           the union-identity case) and that intersecting down to nothing yields the genuine empty set.
           MUST be true.")
  (input (= (Set.intersection #set(1 2 3) #set()) #set()))
  (output (: true Bool)))

(case
  "an empty set in one if-branch unifies with a non-empty set in the other and its length reads correctly"
  (doc
    "A runtime `if` selecting between a non-empty `(Set.of (list 1 2 3))` and an EMPTY `(Set.of (list))`:
           the two branches must unify to one `(Set Int64)` — the empty branch takes its element type from the
           non-empty sibling, so the empty set is well-typed (no unconstrained-element-type failure) and its
           length reads correctly. `Set.len` of the selected set: b>0 → the {1,2,3} branch → 3; b≤0 → the empty
           branch → 0. Pins that an empty set literal in a branch position is grounded by its sibling's element
           type and enumerates as a genuine empty set at run time, on both backends.")
  (input (do (def (main (: b Int64)) (Set.len (if (> b 0) #set(1 2 3) #set()))) (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: -1 Int64))
  (output (: 0 Int64)))

(case
  "the intersection of disjoint sets is empty"
  (doc
    "`(Set.intersection (Set.of (list 1 2)) (Set.of (list 3 4)))` is {} — two sets sharing no
           element intersect to nothing. Pins that intersection over disjoint operands (no shared element,
           yet both non-empty) is the empty set, the complement of the overlapping-intersection case which
           has a shared element. MUST be true.")
  (input (= (Set.intersection #set(1 2) #set(3 4)) #set()))
  (output (: true Bool)))

(case
  "the difference of a set with itself is empty"
  (doc
    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list 1 2 3)))` is {} — removing a set's own
           elements leaves nothing. Pins the self-difference law (A ∖ A = ∅), the degenerate boundary the
           asymmetric-difference case above does not reach. MUST be true.")
  (input (= (Set.difference #set(1 2 3) #set(1 2 3)) #set()))
  (output (: true Bool)))

(case
  "union is commutative"
  (doc
    "`(Set.union A B)` equals `(Set.union B A)` for A = {1, 2}, B = {2, 3}: the union does not
           depend on operand order (both are {1, 2, 3}). Pins commutativity of union directly as a value
           equality between the two orderings — a law that follows from a set being an order-independent
           collection (the written-order-independence case, lifted to the operation). MUST be true.")
  (input (= (Set.union #set(1 2) #set(2 3)) (Set.union #set(2 3) #set(1 2))))
  (output (: true Bool)))

(case
  "union of a set with itself is the set (idempotent)"
  (doc
    "`(Set.union (Set.of (list 1 2 3)) (Set.of (list 1 2 3)))` is {1, 2, 3} — unioning a set with
           itself introduces no duplicates (a set holds each element once), so union is idempotent. Pins
           A ∪ A = A, the duplicate-collapsing law of union at the whole-set level (the operation-level
           companion of \"a set collapses a duplicate element\"). MUST be true.")
  (input (= (Set.union #set(1 2 3) #set(1 2 3)) #set(1 2 3)))
  (output (: true Bool)))

(case
  "intersection of a set with itself is the set (idempotent)"
  (doc
    "`(Set.intersection (Set.of (list 1 2 3)) (Set.of (list 1 2 3)))` is {1, 2, 3} — every element is
           in both operands, so the intersection is the whole set. Pins A ∩ A = A, the intersection
           companion of the union-idempotent law above; an intersection that de-duplicated incorrectly or
           dropped a shared element would fail this. MUST be true.")
  (input (= (Set.intersection #set(1 2 3) #set(1 2 3)) #set(1 2 3)))
  (output (: true Bool)))

(case
  "the SAME runtime set as both operands of union intersection and difference computes each law live"
  (doc
    "The idempotent/self-difference laws above compare two structurally-equal LITERALS, which fold —
           this passes ONE runtime-built set (a recursive Set.insert loop) as BOTH operands of all three
           binary ops in one expression: `(union s s)` = A ∪ A = A (len n), `(intersection s s)` = A (len
           n), `(difference s s)` = ∅ (len 0), encoded 100·|A∪A| + 10·|A∩A| + |A∖A| = 330 at n = 3. Beyond
           the algebra, the shape is a Perceus face the shared-across-statements pins don't reach: `s` is a
           CONSUMED operand TWICE AT ONE CALL SITE (both argument positions of one consuming op), so the
           retain must dup it twice per op — and again for the next op — from one binding. A missed dup
           reads a freed CHAMP; a leaked one nets live cells. Expected: 330.")
  (input
    (do
      (def (build (: n Int64) (: s (Set Int64))) (if (< n 1) s (build (- n 1) (Set.insert s n))))
      (def
        (main (: n Int64))
        (let
          ((s (build n #set())))
          (+
            (* 100 (Set.len (Set.union s s)))
            (+ (* 10 (Set.len (Set.intersection s s))) (Set.len (Set.difference s s))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 330 Int64)))

(case
  "the difference of a set with the empty set is the set (identity)"
  (doc
    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list)))` is {1, 2, 3} — removing nothing leaves
           the set unchanged. Pins A ∖ ∅ = A, the identity element of difference and the companion of the
           self-difference law A ∖ A = ∅ above (the two boundaries of `difference`: subtracting everything
           gives empty, subtracting nothing gives the whole set). MUST be true.")
  (input (= (Set.difference #set(1 2 3) #set()) #set(1 2 3)))
  (output (: true Bool)))

(case
  "union is associative"
  (doc
    "`(A ∪ B) ∪ C` equals `A ∪ (B ∪ C)` for overlapping A={1,2}, B={2,3}, C={3,4} — the union
           regrouping does not change the result (both are {1,2,3,4}). The MULTI-way companion of
           commutativity: a canonical-order or dedup bug in the 3-way fold could break associativity while
           the 2-way commutativity above still passes, so this pins the associative regrouping directly.
           MUST be true.")
  (input
    (=
      (Set.union (Set.union #set(1 2) #set(2 3)) #set(3 4))
      (Set.union #set(1 2) (Set.union #set(2 3) #set(3 4)))))
  (output (: true Bool)))

(case
  "union dedups overlapping elements by content, counted once"
  (doc
    "`(Set.len (Set.union {1,2,3} {2,3,4}))` is 4, not 6: the shared elements 2 and 3 are held once in
           the union, not double-counted. The operation-level dedup over TWO multi-element sets (the
           existing runtime union-dedup case only overlaps a single element), pinning that union merges
           by content across a genuine multi-element overlap. MUST be 4.")
  (input (Set.len (Set.union #set(1 2 3) #set(2 3 4))))
  (output (: 4 Int64)))

; The set-algebra cases here use SMALL sets (a handful of elements, single CHAMP leaf). At higher
; cardinality a Set is a MULTI-LEVEL CHAMP trie, so union/intersection/difference must merge/traverse deep
; tries correctly. This pins the large-set algebra: two 40-element runtime sets with a known 20-element
; overlap (a=[0,40), b=[20,60)) — union=60, intersection=20, difference(a∖b)=20 — plus membership
; spot-checks (an element only in b is in the union; a shared element is in the intersection; an
; a-only element is in the difference but a shared element is NOT). A deep-trie merge that mis-navigated,
; double-counted the overlap, or dropped a node would flip a cardinality or a membership bit.
(case
  "large-set algebra over a multi-level CHAMP trie computes correct union/intersection/difference"
  (doc
    "Two 40-element runtime sets built by a push-loop (each spans >1 CHAMP node): a = [0,40),
           b = [20,60), overlapping on [20,40) (20 elements). `Set.len` of the union is 60 (0..59, the
           overlap counted once), of the intersection 20 ([20,40)), of the difference a∖b 20 ([0,20)).
           Membership spot-checks: 55 ∈ union (b-only), 25 ∈ intersection (shared), 5 ∈ a∖b (a-only), 25 ∉
           a∖b (shared, correctly excluded from the difference). Result `(60, 20, 20, 1, 1, 1, 0)`. Pins
           that the CHAMP set-algebra operations merge/traverse the deep multi-level trie by content — the
           large-cardinality companion of the small-set union/intersection/difference law cases above.")
  (input
    (do
      (def
        (fill (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (fill (+ i 1) n (Set.insert s i)) s))
      (def
        (main (: z Int64))
        (let
          ((a (fill 0 40 #set())) (b (fill 20 60 #set())))
          #tuple((Set.len (Set.union a b))
            (Set.len (Set.intersection a b))
            (Set.len (Set.difference a b))
            (if (Set.contains (Set.union a b) 55) 1 0)
            (if (Set.contains (Set.intersection a b) 25) 1 0)
            (if (Set.contains (Set.difference a b) 5) 1 0)
            (if (Set.contains (Set.difference a b) 25) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (tuple 60 20 20 1 1 1 0) (Tuple Int64 Int64 Int64 Int64 Int64 Int64 Int64)))
  (live-objects known-leak))

(case
  "self-difference of a 100-element trie IS the canonical empty set by equality"
  (doc
    "The self-difference law (A ∖ A = ∅) at MULTI-LEVEL trie scale, upgraded from cardinality to
           IDENTITY: the runtime self-difference case above checks `Set.len` = 0 at n=3 (one leaf); here a
           100-element set's self-difference must EQUAL `(Set.of (list))` by canonical `=` (10) as well as
           by len (+0). The difference walk tears down a deep trie node by node — a walk that left any
           residual interior structure (a non-collapsed empty branch) would report len 0 yet fail the
           canonical-equality check. The empty-set seed is `(Set.of (list))` — the `Set.insert`-onto-
           `Set.empty` chain does not yet emit (see the construction-path asymmetry notes).")
  (input
    (do
      (def
        (build (: i Int64) (: acc (Set Int64)))
        (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
      (def
        (main (: n Int64))
        (do
          (def s (build n #set()))
          (def d (Set.difference s s))
          (+ (* 10 (if (= d #set()) 1 0)) (Set.len d))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 10 Int64)))

(case
  "union with a DERIVED empty (self-difference) is identity at trie scale"
  (doc
    "The identity-element law (∅ ∪ A = A) where the empty operand is DERIVED — `(Set.difference s
           s)` — rather than a literal, and A is a 100-element multi-level trie: the union must return a
           set EQUAL to the original by canonical `=` (10) with membership intact (+1). Composes the
           self-difference collapse with the union merge walk: a derived empty carrying structural residue
           would poison the union's merge (a wrong branch copied in), breaking equality with the pristine
           operand. The union/intersection/difference identity laws above all use LITERAL empties; this
           pins the derived-empty face at depth.")
  (input
    (do
      (def
        (build (: i Int64) (: acc (Set Int64)))
        (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
      (def
        (main (: n Int64))
        (do
          (def s (build n #set()))
          (def rt (Set.union (Set.difference s s) s))
          (+ (* 10 (if (= rt s) 1 0)) (if (Set.contains rt 57) 1 0))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 11 Int64)))

(case
  "intersection of a trie with its churned-back subset is the subset itself"
  (doc
    "The two-differently-built-operands face of trie-scale intersection: `full` = a 100-element
           build; `odds` = full with every even element REMOVED (a churned derivation, its trie shaped
           by 50 removals); their intersection must EQUAL `odds` by canonical `=` (10) with membership
           correct (even 2 absent → +0). The merge walk receives one operand built by pure insertion
           and one shaped by removal-collapse — canonical inputs commute through the intersection
           regardless of construction history (the algebra face of the churn-identity family). The
           empty-set seed is `(Set.of (list))` — the `Set.insert`-onto-`Set.empty` chain does not yet
           emit (the construction-path asymmetry notes at the recursive-sum and remove-path cases).")
  (input
    (do
      (def
        (build (: i Int64) (: acc (Set Int64)))
        (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
      (def
        (drop-half (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (> i n) s (drop-half (+ i 2) n (Set.remove s i))))
      (def
        (main (: n Int64))
        (do
          (def full (build n #set()))
          (def odds (drop-half 2 n full))
          (def inter (Set.intersection full odds))
          (+ (* 10 (if (= inter odds) 1 0)) (if (Set.contains inter 2) 1 0))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 10 Int64)))

(case
  "a SET churned to empty equals a fresh empty set and re-accepts elements"
  (doc
    "The Set face of the churn-to-empty family (the Map twins are in 05-compound-types): grow a
           set to 120 elements, remove every one — the survivor must EQUAL `(Set.of (list))` by
           canonical `=` (100), report len 0 (+0·10), and RE-ACCEPT an element with membership intact
           (+1) → 101. The full drain collapses every node the growth created, and the re-insert
           proves no tombstone or stale-node residue corrupts the re-grown trie. The empty-set seed is
           `(Set.of (list))` — `Set.empty` is an intended surface member whose emit has not landed
           (the insert-onto-Set.empty chain still declines; see the construction-path asymmetry notes).")
  (input
    (do
      (def
        (grow (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (= i n) s (grow (+ i 1) n (Set.insert s i))))
      (def
        (shrink (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (= i n) s (shrink (+ i 1) n (Set.remove s i))))
      (def
        (main (: n Int64))
        (do
          (def emptied (shrink 1 n (grow 1 n #set())))
          (+
            (* 100 (if (= emptied #set()) 1 0))
            (+ (* 10 (Set.len emptied)) (if (Set.contains (Set.insert emptied 42) 42) 1 0)))))
      (export main)))
  (call main (: 120 Int64))
  (output (: 101 Int64)))

(case
  "intersection is associative"
  (doc
    "`(A ∩ B) ∩ C` equals `A ∩ (B ∩ C)` for A={1,2,3,4}, B={2,3,4,5}, C={3,4,5,6} — both regroupings
           yield {3,4}. The intersection companion of union associativity; pins that the meet regrouping is
           order-independent. MUST be true.")
  (input
    (=
      (Set.intersection (Set.intersection #set(1 2 3 4) #set(2 3 4 5)) #set(3 4 5 6))
      (Set.intersection #set(1 2 3 4) (Set.intersection #set(2 3 4 5) #set(3 4 5 6)))))
  (output (: true Bool)))

; The lattice laws that COMPOSE the operations, over RUNTIME-element sets so the nested CHAMP union/intersection
; actually runs (a const-set shape could fold the algebra away at compile time — the runtime operand is what
; exercises the heap path). Intersection commutativity is the meet companion of union-is-commutative above;
; distributivity and absorption are the two-operation laws a bounded-lattice set algebra must satisfy — a
; mis-slotting or a dropped element on the nested compose path would break them where a single-op law would not.
(case
  "intersection is commutative over runtime-element sets"
  (doc
    "The meet companion of union-is-commutative: `(Set.intersection A B) = (Set.intersection B A)` over
           RUNTIME elements. A={a,b,c}, B={b,c,d} with a,b,c,d runtime → both {b,c}. A runtime operand so the
           CHAMP intersection genuinely runs (a const shape could fold). Encoded as a Bool→Int (1 true). MUST
           be 1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64) (: c Int64) (: d Int64))
        (let
          ((sa #set(a b c)) (sb #set(b c d)))
          (if (= (Set.intersection sa sb) (Set.intersection sb sa)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 4 Int64))
  (output (: 1 Int64)))

(case
  "intersection distributes over union on runtime-element sets"
  (doc
    "The lattice distributive law `A ∩ (B ∪ C) = (A ∩ B) ∪ (A ∩ C)`, over RUNTIME elements so the nested
           CHAMP union/intersection composition runs live. A={a,b,c}, B={b,d}, C={c,e} → both sides {b,c}.
           Pins that composing union INSIDE intersection agrees with distributing it out; an implementation
           that mis-slotted or dropped an element on the nested compose path would diverge here while the
           single-operation laws still passed. MUST be 1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64) (: c Int64) (: d Int64) (: e Int64))
        (let
          ((sa #set(a b c)) (sb #set(b d)) (sc #set(c e)))
          (if
            (=
              (Set.intersection sa (Set.union sb sc))
              (Set.union (Set.intersection sa sb) (Set.intersection sa sc)))
            1
            0)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 4 Int64) (: 5 Int64))
  (output (: 1 Int64)))

(case
  "the absorption law holds on runtime-element sets"
  (doc
    "The lattice absorption law `A ∪ (A ∩ B) = A`, over RUNTIME elements. A={a,b}, B={b,c} → A ∩ B = {b},
           A ∪ {b} = {a,b} = A. Pins that intersecting-then-unioning-back absorbs to A — the union adds nothing
           not already in A (the meet is a subset of A). A join that duplicated or a meet that leaked would
           break this. MUST be 1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64) (: c Int64))
        (let
          ((sa #set(a b)) (sb #set(b c)))
          (if (= (Set.union sa (Set.intersection sa sb)) sa) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64) (: 3 Int64))
  (output (: 1 Int64)))

(case
  "the set De Morgan law relates difference-over-union to intersection-of-differences on runtime sets"
  (doc
    "The relative-complement De Morgan law `A \\ (B ∪ C) = (A \\ B) ∩ (A \\ C)`, over RUNTIME-element sets
           so the nested CHAMP difference/union/intersection composition runs live. A={a,b,c,d}, B={b}, C={c}:
           B ∪ C = {b,c}, so A \\ (B ∪ C) = {a,d}; and A \\ B = {a,c,d}, A \\ C = {a,b,d}, their intersection =
           {a,d}. Both sides {a,d}. Pins that removing a union equals intersecting the individual removals — a
           three-operation composition an implementation could break on the nested difference path while the
           single-operation difference laws (difference-with-itself, NOT-commutative) still passed. Completes
           the two/three-operation lattice-law cluster beside distributive and absorption. MUST be 1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64) (: c Int64) (: d Int64))
        (let
          ((sa #set(a b c d)) (sb #set(b)) (sc #set(c)))
          (if
            (=
              (Set.difference sa (Set.union sb sc))
              (Set.intersection (Set.difference sa sb) (Set.difference sa sc)))
            1
            0)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 4 Int64))
  (output (: 1 Int64)))

(case
  "difference is NOT commutative"
  (doc
    "`{1,2,3} \\ {2,3,4}` = {1} but `{2,3,4} \\ {1,2,3}` = {4} — set difference is directional, so
           `A \\ B` and `B \\ A` are DIFFERENT sets (unequal). The contrast to union/intersection
           commutativity: pins that difference does NOT commute (a `=` between the two orderings is FALSE),
           so a bug treating difference symmetrically would be caught. MUST be false.")
  (input (= (Set.difference #set(1 2 3) #set(2 3 4)) (Set.difference #set(2 3 4) #set(1 2 3))))
  (output (: false Bool)))

(case
  "the empty set is equal to the empty set"
  (doc
    "`(= (Set.of (list)) (Set.of (list)))` is true — two empty sets contain the same (no) elements
           (collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that the empty set
           is a genuine value equal to itself, the set companion of the empty-string / empty-map cases.")
  (input (= #set() #set()))
  (output (: true Bool)))

; --- Two sets with DIFFERENT elements are the SAME TYPE: comparing them is well-typed, not a --------
; --- shape error. A set's elements are runtime data, not part of its type (exactly as a map's key ---
; set is), so `(Set Int64)` is one type regardless of which ints a value holds. This is the crucial
; counterpoint the map-comparison cases (05-compound-types.sexp) already pin, carried onto the set
; path: differing elements ⇒ the comparison is FALSE (they do not contain the same elements,
; collections-and-text.md #A Set Is A Collection Of Unique Elements), NOT a CDZ0201 shape rejection.
; Contrast records/tuples, whose field set / arity IS their type.
(case
  "two sets with different elements are unequal, not a type error"
  (doc
    "`(Set.of (list 1 2))` and `(Set.of (list 1 3))` are both `(Set Int64)` — the SAME type, since
           a set's elements are runtime data, not part of its type (unlike a record's fixed field set).
           So the comparison is well-typed and FALSE (they do not contain the same elements), NOT a type
           error. Pins that a set's elements are runtime data — the set analogue of the different-keyset
           map comparison. MUST be false.")
  (input (= #set(1 2) #set(1 3)))
  (output (: false Bool)))

(case
  "two sets of different sizes are unequal, not a type error"
  (doc
    "`(Set.of (list 1))` and `(Set.of (list 1 2))` differ in cardinality — runtime data, not part
           of the type — so the comparison is well-typed and FALSE (collections-and-text.md #A Set Is A
           Collection Of Unique Elements). The size-difference companion; contrast records `(= (record
           (a 1)) (record (a 1) (b 2)))`, which IS a type error because a record's field set is its
           shape. MUST be false.")
  (input (= #set(1) #set(1 2)))
  (output (: false Bool)))

(case
  "a set with elements of two different types is a type error"
  (doc
    "`(Set.of (list 1 true))` would need elements of one type, but the list mixes an Int64 and a
           Bool — not a homogeneous element type — so the set is ill-typed and the compiler rejects it
           (CDZ0201, collections-and-text.md #A Set Is A Collection Of Unique Elements: elements of one
           type), exactly as a heterogeneous list is rejected. The homogeneity flows in through the
           list `Set.of` consumes.")
  (input #set(1 true))
  (error CDZ0201))

(case
  "a set built at run time escapes to the host as its value form"
  (doc
    "A Set built at RUN TIME (an insert-loop, not a constant `Set.of`) crosses the host boundary.
           A runtime collection has no fixed value-form template (its size is dynamic), so it escapes via
           the runtime value-encode walker guided by a compiler-baked shape descriptor whose PARAMETRIC
           frame renders the element type — the value form is `((. Set of) (list …))` with elements in
           CANONICAL key order under `(Set Int64)`. `build` inserts 3,2,1 onto an empty set → the sorted
           `(list 1 2 3)`. This declined before as needing a value-form walker.")
  (input
    (do
      (def (build s n) (if (< n 1) s (build (Set.insert s n) (- n 1))))
      (def (main) (build #set() 3))
      (export main)))
  (output (: #set(1 2 3) (Set Int64)))
  (live-objects known-leak))

; The escape case above crosses an INSERT-built set. A set produced by set ALGEBRA (union / intersection /
; difference) is also a runtime handle that must escape to the host as its value form — exercising the
; value-encode walker on an algebra-op RESULT (distinct from reading its cardinality/membership, which the
; algebra cases below do). A runtime element in one operand forces the algebra to run at run time, and the
; whole result set crosses, rendered in CANONICAL sorted key order.
(case
  "a runtime set-union result escapes to the host as its value form"
  (doc
    "`(Set.union (Set.of (list 1 2)) (Set.insert (Set.of (list)) x))` unions a constant set with a
           runtime-built singleton, and the RESULT set escapes to the host. With x=5 the union is {1,2,5} →
           `((. Set of) (list 1 2 5))`; with x=1 the shared element is held once → {1,2}. Pins that a
           set-ALGEBRA result crosses the boundary via the value-encode walker (not only an insert-built
           set), rendered in canonical sorted order — the union companion of the insert-built escape.")
  (input (do (def (main (: x Int64)) (Set.union #set(1 2) (Set.insert #set() x))) (export main)))
  (call main (: 5 Int64))
  (output (: #set(1 2 5) (Set Int64)))
  (call main (: 1 Int64))
  (output (: #set(1 2) (Set Int64)))
  (live-objects known-leak))

(case
  "a runtime set-difference result escapes to the host as its value form"
  (doc
    "`(Set.difference (Set.of (list 1 2 3)) (Set.insert (Set.of (list)) x))` removes a runtime element
           and the RESULT escapes: x=2 → {1,3} → `((. Set of) (list 1 3))`; x=9 (absent) → the unchanged
           {1,2,3}. Pins that a difference result crosses as its canonical value form, the subtractive
           companion of the union-escape case (the value-encode walker handles an algebra result of any of
           the three set operations).")
  (input
    (do (def (main (: x Int64)) (Set.difference #set(1 2 3) (Set.insert #set() x))) (export main)))
  (call main (: 2 Int64))
  (output (: #set(1 3) (Set Int64)))
  (call main (: 9 Int64))
  (output (: #set(1 2 3) (Set Int64)))
  (live-objects known-leak))

; --- RUNTIME-element `Set.of`: equality and set algebra over a set whose element is a runtime value ----
; The cases above build every set from CONSTANT `Set.of` literals or a constant insert-loop, so they
; exercise only the compile-time constant-set folds. A `Set.of` over a list that CONTAINS a runtime
; element — `(Set.of (list 1 2 x))` where `x` is a boundary parameter — is a DIFFERENT path: the set is
; built at run time by `set-empty` + a `set-insert` per element, and its equality / set algebra CANNOT be
; folded (the runtime element is undecidable at compile time), so each defers to the runtime CHAMP
; operation. These pin that the deferral is CORRECT — the same order-independent, canonical-by-construction
; result the constant path records. (Regression witnesses: the constant-set equality fold and the
; set-algebra fold each treated a runtime element as ABSENT, mis-folding a runtime-element set comparison
; to a definite wrong value — even a set was NOT equal to ITSELF — and dropping/keeping the wrong element
; in a difference/intersection; the fix declines the fold to the runtime walk when any element is not a
; compile-time constant, exactly as the sibling map-equality / map-insert folds already guard their keys.)
(case
  "a runtime-element set equals itself"
  (doc
    "`(= (Set.of (list x)) (Set.of (list x)))` with `x` a runtime parameter builds two sets from a
           list carrying a runtime element, so the comparison CANNOT fold — it defers to the runtime
           `value-eq` walk over two CHAMP handles, which are canonical by construction. A set is always
           equal to itself, so the result is true for every `x` (reflexivity). Pins that a runtime-element
           set comparison is not mis-folded to a constant `false` (the const-set-equality fold treated the
           runtime element as absent, folding even reflexivity to `false`).")
  (input (do (def (main (: x Int64)) (= #set(x) #set(x))) (export main)))
  (call main (: 9 Int64))
  (output (: true Bool)))

(case
  "a runtime-element set's equality is independent of written order"
  (doc
    "`(= (Set.of (list 1 2 x)) (Set.of (list x 2 1)))` — the same three elements written in a
           different order, one of them a runtime value. Sets are unordered (collections-and-text.md #A Set
           Is A Collection Of Unique Elements), so the two are equal regardless of order and regardless of
           `x`. Pins order-independence on the RUNTIME path — the const-set fold's order-independence,
           carried onto the deferred `value-eq` walk. True for every `x`.")
  (input (do (def (main (: x Int64)) (= #set(1 2 x) #set(x 2 1))) (export main)))
  (call main (: 9 Int64))
  (output (: true Bool))
  (call main (: 3 Int64))
  (output (: true Bool)))

(case
  "a runtime-element set compares equal to the constant set of the same elements"
  (doc
    "`(= (Set.of (list 1 2 x)) (Set.of (list 1 2 3)))` — with `x` = 3 the two sets contain exactly
           {1,2,3} and are EQUAL; with `x` = 9 they differ and are UNEQUAL. The left set is built at run
           time (a runtime element), the right is a constant fold, and the comparison defers to the runtime
           walk. Pins that a runtime-built set and a constant set of the same elements agree — the runtime
           and constant construction paths produce byte-identical canonical CHAMP handles, and that a
           GENUINELY different element set is `false`, not accidentally `true`.")
  (input (do (def (main (: x Int64)) (= #set(1 2 x) #set(1 2 3))) (export main)))
  (call main (: 3 Int64))
  (output (: true Bool))
  (call main (: 9 Int64))
  (output (: false Bool)))

(case
  "a runtime element collapses against a constant one at build"
  (doc
    "`(Set.len (Set.of (list 1 2 x)))` is 2 when `x` = 1 (it collapses against the constant 1, held
           once) and 3 when `x` = 9 (a distinct third element). Pins that construction deduplicates by
           VALUE across a mix of constant and runtime elements — the uniqueness invariant holds when the
           source list is built at run time, not only when every element is a literal.")
  (input (do (def (main (: x Int64)) (Set.len #set(1 2 x))) (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64)))

(case
  "difference over a runtime-element set removes exactly that element"
  (doc
    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list x)))` removes `x` from {1,2,3}: with `x`
           = 2 the result is {1,3} (does not contain 2), with `x` = 9 the result is unchanged {1,2,3}
           (still contains 2). The subtrahend is built from a runtime element, so the algebra defers to the
           runtime `set-difference` over CHAMP handles. Pins that a runtime-element operand is subtracted
           by VALUE (a regression witness: the set-algebra fold reported the runtime element absent and
           subtracted nothing, leaving 2 in the result).")
  (input
    (do
      (def (main (: x Int64)) (Set.contains (Set.difference #set(1 2 3) #set(x)) 2))
      (export main)))
  (call main (: 2 Int64))
  (output (: false Bool))
  (call main (: 9 Int64))
  (output (: true Bool)))

(case
  "difference cardinality with a runtime-element subtrahend"
  (doc
    "`(Set.len (Set.difference (Set.of (list 1 2 3)) (Set.of (list x))))` is 2 when `x` ∈ {1,2,3}
           (one element removed) and 3 when `x` is absent from the first set. The size companion of the
           membership case above, over the deferred runtime `set-difference`.")
  (input (do (def (main (: x Int64)) (Set.len (Set.difference #set(1 2 3) #set(x)))) (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64)))

(case
  "intersection cardinality with a runtime-element operand"
  (doc
    "`(Set.len (Set.intersection (Set.of (list 1 2 3)) (Set.of (list x))))` is 1 when `x` ∈ {1,2,3}
           (the one shared element) and 0 when `x` is absent from the first set. The intersection defers to
           the runtime `set-intersection` because its second operand carries a runtime element. Pins that a
           runtime element is intersected by VALUE (a regression witness: the fold reported it absent and
           produced the empty intersection even when `x` was present).")
  (input
    (do (def (main (: x Int64)) (Set.len (Set.intersection #set(1 2 3) #set(x)))) (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64)))

(case
  "union cardinality with a runtime-element operand"
  (doc
    "`(Set.len (Set.union (Set.of (list 1 2 3)) (Set.of (list x))))` is 3 when `x` ∈ {1,2,3} (the
           shared element is held once) and 4 when `x` is a new element. The union defers to the runtime
           `set-union`; pins that a runtime element already present is not double-counted — the
           uniqueness invariant on the deferred path.")
  (input (do (def (main (: x Int64)) (Set.len (Set.union #set(1 2 3) #set(x)))) (export main)))
  (call main (: 2 Int64))
  (output (: 3 Int64))
  (call main (: 9 Int64))
  (output (: 4 Int64)))

(case
  "membership of a runtime element in a runtime-element set"
  (doc
    "`(Set.contains (Set.of (list 1 2 x)) x)` is true for every `x` — a set built from a list
           containing `x` contains `x`. The membership predicate over a runtime-built set, deferring to the
           runtime `set-contains`. Pins that a runtime element is found by VALUE after construction.")
  (input (do (def (main (: x Int64)) (Set.contains #set(1 2 x) x)) (export main)))
  (call main (: 9 Int64))
  (output (: true Bool))
  (call main (: 2 Int64))
  (output (: true Bool)))

; --- `Set.insert` / `Set.remove` at a RUNTIME element: the functional single-element edits -------------
; The `Set.insert`/`Set.remove` cases above use CONSTANT elements, so the result folds. Inserting or
; removing a RUNTIME element (a boundary parameter) into/from a constant set cannot fold — the edit runs on
; the persistent CHAMP at run time (the operand set folds to a constant, but the edit element is dynamic).
; These pin that the runtime edit preserves the uniqueness invariant (insert of a present element is a
; no-op, remove of an absent element is a no-op), observed through membership and cardinality.
(case
  "inserting a runtime element adds it or is a no-op if already present"
  (doc
    "`(Set.len (Set.insert (Set.of (list 1 2 3)) x))` is 4 when `x` is a NEW element (4 → the set
           grows) and 3 when `x` is already present (2 → held once, insert is a no-op value,
           collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that a runtime insert
           preserves uniqueness — the cardinality reflects present-vs-absent decided at run time, deferring
           to the runtime `set-insert`.")
  (input (do (def (main (: x Int64)) (Set.len (Set.insert #set(1 2 3) x))) (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 2 Int64))
  (output (: 3 Int64)))

(case
  "inserting a runtime element yields a set that contains it"
  (doc
    "`(Set.contains (Set.insert (Set.of (list 1 2)) x) x)` is true for every `x` — the element just
           inserted at run time is present. Pins that a runtime `set-insert` actually adds the element
           (found by value afterward), the membership companion of the cardinality case.")
  (input (do (def (main (: x Int64)) (Set.contains (Set.insert #set(1 2) x) x)) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool))
  (call main (: 1 Int64))
  (output (: true Bool)))

(case
  "removing a runtime element drops it or is a no-op if absent"
  (doc
    "`(Set.contains (Set.remove (Set.of (list 1 2 3)) x) 2)` — removing `x` from {1,2,3} then testing
           for 2: when `x`=2 the removed element IS 2, so 2 is gone (false); when `x`=9 (absent) the set is
           unchanged and still holds 2 (true — removal is total, collections-and-text.md #A Set Is A
           Collection Of Unique Elements). Pins that a runtime `set-remove` drops exactly the named element
           and is a no-op on an absent one.")
  (input (do (def (main (: x Int64)) (Set.contains (Set.remove #set(1 2 3) x) 2)) (export main)))
  (call main (: 2 Int64))
  (output (: false Bool))
  (call main (: 9 Int64))
  (output (: true Bool)))

(case
  "removing a runtime element lowers the cardinality only when present"
  (doc
    "`(Set.len (Set.remove (Set.of (list 1 2 3)) x))` is 2 when `x` ∈ {1,2,3} (one element dropped)
           and 3 when `x` is absent (removal is total, the set is unchanged). The cardinality companion of
           the membership case, over the runtime `set-remove`.")
  (input (do (def (main (: x Int64)) (Set.len (Set.remove #set(1 2 3) x))) (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64)))

; --- A Set threaded as a recursive ACCUMULATOR — the seen-set / visited-set idiom ----------------------
; A compiler carries a Set as an accumulator across a recursion — a set of visited nodes (cycle detection),
; free variables collected, or declared capabilities — inserting as it walks, then querying membership or
; cardinality. This is the set analogue of the map-accumulator (05-compound-types) and distinct from the
; runtime set-escape case above (which RETURNS the set): here the set is THREADED as its own parameter and
; consumed to a SCALAR (`Set.len` / `Set.contains`), with dedup happening DURING the runtime accumulation.
(case
  "a set threaded as a recursive accumulator dedups during accumulation"
  (doc
    "`build` inserts `n % 3` for n, n-1, …, 1 into a set THREADED as its own parameter, then `Set.len`
           measures it. The inserted values cycle through {0,1,2}, so the set holds at most 3 distinct
           elements regardless of `n`: `build 6` inserts 0,2,1,0,2,1 → 3; `build 2` inserts 2,1 → 2;
           `build 0` → 0 (the empty accumulator). Pins that a set carried as a recursive accumulator dedups
           its inserts at run time (the uniqueness invariant across the threaded accumulation), consumed to
           a scalar — the seen-set idiom, the set companion of the map accumulator.")
  (input
    (do
      (def
        (build (: n Int64) (: s (Set Int64)))
        (if (= n 0) s (build (- n 1) (Set.insert s (% n 3)))))
      (def (main (: n Int64)) (Set.len (build n #set())))
      (export main)))
  (call main (: 6 Int64))
  (output (: 3 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a set accumulator is queried for membership after building"
  (doc
    "The visited-set query: `build 5` accumulates {1,2,3,4,5} through the threaded set parameter, then
           `Set.contains` tests a runtime query element — q=3 → present (1), q=9 → absent (0). Pins that a
           set grown across a recursion answers a membership query afterward — the cycle-detection /
           already-seen check a compiler pass makes while walking, the set companion of the map-lookup
           accumulator query.")
  (input
    (do
      (def (build (: n Int64) (: s (Set Int64))) (if (= n 0) s (build (- n 1) (Set.insert s n))))
      (def (main (: q Int64)) (if (Set.contains (build 5 #set()) q) 1 0))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64)))

; --- A Set consumed by Set.insert in one operand is UNCHANGED for a later read of the same binding ------
; The set analogue of the shared-`let` List persistence cases (05-compound-types): `Set.insert` is
; PERSISTENT — it produces a new set and MUST leave its operand unchanged (collections-and-text.md: a value
; must not be observably mutated through one reference while read through another). A set bound by `let`
; and read TWICE — once consumed by an insert, once read as the original — is SHARED, so the consuming op
; must copy, not FBIP-mutate in place. The compiler emits a Perceus RETAIN (`dup`) at the consumed
; occurrence of a binding with a later live use; without it the CHAMP insert would mutate the shared trie
; and the later read would see the grown set (the same defect the List/projection persistence cases pin).
(case
  "a set consumed by Set.insert in one operand is unchanged for a later read of the same binding"
  (doc
    "`s = build 0 3` = {0,1,2} (a genuine runtime set, no const-fold); read twice: the left operand
           inserts 99 and measures (→ 4), the right reads the ORIGINAL `s` size (→ 3), so 4 + 3 = 7. If the
           insert mutated the shared `s` in place (a CHAMP FBIP update whose retain was missing on a
           multi-use binding), the second read would see {0,1,2,99} → 8. Order-sensitive (reading `s` first
           → 7 regardless), the tell of an in-place mutation. Pins that a persistent Set.insert leaves a
           shared operand unchanged — the Set companion of the List.push persistence case.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert acc i)) acc))
      (def
        (main (: n Int64))
        (let ((s (build 0 n #set()))) (+ (Set.len (Set.insert s 99)) (Set.len s))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 7 Int64))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a set consumed by Set.remove in one operand is unchanged for a later read of the same binding"
  (doc
    "The removal-side twin of the Set.insert persistence case above: `Set.remove` is PERSISTENT too —
           it produces a new set without the element and MUST leave its operand unchanged. `s = build 0 n` is
           a genuine runtime set (no const-fold), read TWICE: `(Set.remove s 1)` bound to `s2`, and `s` read
           as the original. Encodes `100*(Set.len s) + 10*(Set.len s2) + (Set.contains s 1 ? 1 : 0)`. At n=3
           the original `s` = {0,1,2} keeps len 3 AND still contains 1 (→ 100*3 + 10*2 + 1 = 321) while `s2` =
           {0,2} dropped it (len 2). If the remove FBIP-mutated the shared CHAMP trie in place (a retain
           missing on the multi-use binding), the original read would see {0,2} → len 2 and contains-1 = 0 →
           220. At n=1 the set is {0}, removing the absent 1 leaves both unchanged (len 1, contains-1 = 0 →
           100*1 + 10*1 + 0 = 110), pinning that a remove of an absent element is also persistent. Completes
           the removal-side persistence family (Map.remove + Set.remove). Both backends.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert acc i)) acc))
      (def
        (main (: n Int64))
        (let
          ((s (build 0 n #set())))
          (let
            ((s2 (Set.remove s 1)))
            (+ (* 100 (Set.len s)) (+ (* 10 (Set.len s2)) (if (Set.contains s 1) 1 0))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 321 Int64))
  (call main (: 1 Int64))
  (output (: 110 Int64)))

; --- Set.contains / Set.remove / Set.insert must NOT fold against a set holding a RUNTIME element -------
; `Set.of (list …)` folds a CONSTANT list to a canonical constant `Core::SetOf`, and `Set.contains`/
; `Set.remove`/`Set.insert` fold against such a constant set by comparing elements at COMPILE TIME
; (`const_compound_eq`). But a `Set.of` can carry a NON-CONSTANT element (a call/param result that did not
; fold — `(Set.of (list (add 2 3)))`). Comparing a runtime element to a constant query at compile time is
; `None` (unknown), so folding such a set SILENTLY MISCOMPILED: `Set.contains` answered `false` for a query
; that equals the runtime element at run time; `Set.remove` RETAINED a runtime element equal to the query
; (cardinality stayed high); `Set.insert` could add a duplicate of a runtime element. The fix declines the
; fold unless the ENTIRE set is a compile-time constant (`is_const_value`), running the real champ op
; otherwise — the same all-constant guard the set-algebra fold already applied. These pin all three ops
; over a set built from a genuinely non-foldable (recursive-call) element, at scalar AND rope element types.
(case
  "Set.contains does not fold a set whose element is a runtime scalar"
  (doc
    "`(Set.contains (Set.of (list (add 2 3))) 5)` — the set's sole element `(add 2 3)` is a recursive
           call (non-foldable), evaluating to 5 at run time; membership of the literal 5 must be true → 1.
           Before the fix the `Set.contains` fold saw a `Core::SetOf` shape + a constant query and compared
           only the CONSTANT elements (none), folding to `false` → 0 though the runtime element IS 5. The
           fold now declines (a non-constant element) and the runtime `set-contains` answers correctly.")
  (input
    (do
      (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
      (def (main) (if (Set.contains #set((add 2 3)) 5) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Set.remove does not fold a set whose element is a runtime scalar"
  (doc
    "`(Set.len (Set.remove (Set.of (list (add 2 3))) 5))` — removing the literal 5 from a set whose sole
           element `(add 2 3)`=5 (runtime) must drop it → cardinality 0. Before the fix the fold RETAINED the
           runtime element (its compile-time equality to 5 is unknown, so `retain` kept it) → 1. The fold now
           declines and the runtime `set-remove` removes the matching element.")
  (input
    (do
      (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
      (def (main) (Set.len (Set.remove #set((add 2 3)) 5)))
      (export main)))
  (output (: 0 Int64)))

(case
  "Set.insert does not fold a duplicate against a runtime element"
  (doc
    "`(Set.len (Set.insert (Set.of (list (add 2 3))) 5))` — inserting 5 into a set whose sole element
           `(add 2 3)`=5 (runtime) is a duplicate, so the cardinality stays 1. Before the fix the fold could
           not see the runtime element equalled 5 (its const probe missed it) and would ADD 5 as a second
           element → 2. The fold now declines and the runtime `set-insert` dedups against the canonical
           champ set.")
  (input
    (do
      (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
      (def (main) (Set.len (Set.insert #set((add 2 3)) 5)))
      (export main)))
  (output (: 1 Int64)))

(case
  "Set.contains finds a runtime STRING-rope element built via Set.of"
  (doc
    "`(Set.contains (Set.of (list (rep \"hi\" 3))) \"hixxx\")` — the set's element is a runtime String
           ROPE (`rep` concatenates \"x\" three times → \"hixxx\"), membership-tested with the flat literal
           \"hixxx\" → 1. This is the reported adversarial finding: the `Set.contains` fold (mis)fired on a
           runtime-element `SetOf` and answered 0. The fold now declines and the runtime `set-contains`
           canonicalizes both the stored rope (compacted at Set.of construction) and the flat query, so they
           match. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (Set.contains #set((rep "hi" 3)) "hixxx") 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Set.remove of a rope-element set built via Set.of lowers the cardinality"
  (doc
    "`(Set.len (Set.remove (Set.of (list (rep \"hi\" 3))) \"hixxx\"))` — removing the flat literal
           \"hixxx\" from a set whose sole element is the equal runtime rope must drop it → 0. The
           stronger rope twin of the finding (it hits Set.remove too, not just Set.contains). Before the fix
           the fold retained the rope → 1. Expected: 0.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (Set.len (Set.remove #set((rep "hi" 3)) "hixxx")))
      (export main)))
  (output (: 0 Int64)))

(case
  "an all-constant Set.of still folds membership with a constant query (control)"
  (doc
    "`(Set.contains (Set.of (list 1 2 3)) 2)` — every element AND the query are compile-time constants,
           so the `Set.contains` fold is SOUND and still fires (→ 1); the absent constant query 9 folds to 0.
           Pins that the runtime-element guard did NOT disable the valuable all-constant fold. Two exports
           (a present and an absent constant query) so both fold branches are witnessed. Expected: 1, 0.")
  (input
    (do
      (def (has2) (if (Set.contains #set(1 2 3) 2) 1 0))
      (def (has9) (if (Set.contains #set(1 2 3) 9) 1 0))
      (export has2)
      (export has9)))
  (call has2)
  (output (: 1 Int64))
  (call has9)
  (output (: 0 Int64)))

(case
  "Set.to-list enumerates the elements as a List in canonical (sorted) order"
  (doc
    "`(List.at (Set.to-list (Set.of (list 5 2 8 2))) 0)` — Set.to-list yields the set's elements as a
           `List` in CANONICAL element-value order (sorted, deduped: {2,5,8}), NOT hash/insertion order,
           realizing collections-and-text.md §Map/Set iteration is deterministic. The element at index 0 is
           the smallest, 2. The inverse of Set.of. Expected: 2.")
  (input
    (do
      (def (main) (match (List.at (Set.to-list #set(5 2 8 2)) 0) ((Some v) v) ((None u) -1)))
      (export main)))
  (output (: 2 Int64)))

(case
  "Set.to-list length is the deduped distinct-element count"
  (doc
    "The dedup-cardinality face of Set.to-list: a list with duplicates `(list 3 1 2 1 3)` (five
           elements, three distinct) builds a set {1,2,3}, and `(List.len (Set.to-list …))` is the DEDUPED
           count 3 — not the input length 5. The canonical-order case pins index 0 and the interior order
           over already-distinct inputs; this pins that duplicates COLLAPSE in the enumerated list.")
  (input (do (def (main) (List.len (Set.to-list #set(3 1 2 1 3)))) (export main)))
  (output (: 3 Int64)))

(case
  "Set.to-list enumerates the FULL interior order, not just the smallest element"
  (doc
    "The element-0 case above only pins the smallest element (2) at index 0 — a canonical-order bug
           that kept the smallest first but mis-ordered the INTERIOR (e.g. {2,8,5}) would still pass it. This
           weights all three positions into one scalar: `100*nth0 + 10*nth1 + nth2` over `Set.of (list 5 2 8)`
           = 100*2 + 10*5 + 8 = 258, pinning the whole sorted sequence [2,5,8]. A mis-order like [2,8,5] would
           give 285. Pins the interior enumeration order an index-0-only check cannot see.")
  (input
    (do
      (def
        (main)
        (let
          ((xs (Set.to-list #set(5 2 8))))
          (+
            (+ (* 100 (Option.expect (List.at xs 0) "0")) (* 10 (Option.expect (List.at xs 1) "1")))
            (Option.expect (List.at xs 2) "2"))))
      (export main)))
  (output (: 258 Int64)))

(case
  "Set.to-list canonical order is independent of insertion order"
  (doc
    "The same full-sequence weighting over a set built in REVERSE insertion order `Set.of (list 8 5 2)`
           still yields 258 — Set.to-list orders by element VALUE (canonical sorted), not by the order elements
           were inserted. The insertion-independence companion of the full-order case, made observable through
           the whole sequence rather than a length or element-0 check (a set compares equal regardless of
           enumeration order, so only a to-list-INDEXED read can witness an order divergence).")
  (input
    (do
      (def
        (main)
        (let
          ((xs (Set.to-list #set(8 5 2))))
          (+
            (+ (* 100 (Option.expect (List.at xs 0) "0")) (* 10 (Option.expect (List.at xs 1) "1")))
            (Option.expect (List.at xs 2) "2"))))
      (export main)))
  (output (: 258 Int64)))

(case
  "Set.to-list over a 100-element trie enumerates strictly increasing end to end"
  (doc
    "The order rows above run on 3-element (single-leaf) sets; this pins the enumeration walk over
           a MULTI-LEVEL trie: 100 elements (i·13, spread across node splits) enumerate strictly
           increasing END TO END — an adjacent-pair walk requires every consecutive pair ascending and
           counts all 100, so one out-of-order pair or a dropped element poisons with -100000. The Set
           twin of the deep-trie Map.to-list sortedness pin: a per-node sort missing the cross-node
           merge order passes the small rows and fails here.")
  (input
    (do
      (def
        (build (: i Int64) (: acc (Set Int64)))
        (if (= i 0) acc (build (- i 1) (Set.insert acc (* i 13)))))
      (def
        (inc (: xs (List Int64)) (: prev Int64) (: cnt Int64))
        (match xs (#list() cnt) (#list(h (.. t)) (if (> h prev) (inc t h (+ cnt 1)) -100000))))
      (def (main (: n Int64)) (inc (Set.to-list (build n #set())) -1 0))
      (export main)))
  (call main (: 100 Int64))
  (output (: 100 Int64))
  (live-objects known-leak))

(case
  "Set.of over Set.to-list round-trips a 100-element trie to the identical set"
  (doc
    "The enumerate⇄rebuild closure for sets: `(Set.of (Set.to-list s))` over a 100-element
           multi-level trie must EQUAL the source by canonical `=` with the same cardinality — every
           element surfaced exactly once by the enumeration and re-canonicalized identically by the
           batch build. The Set twin of the Map enumerate-rebuild identity pin; an enumeration that
           dropped or duplicated an element, or a rebuild that canonicalized differently from the
           incremental Set.insert build, breaks the equality.")
  (input
    (do
      (def
        (build (: i Int64) (: acc (Set Int64)))
        (if (= i 0) acc (build (- i 1) (Set.insert acc (* i 11)))))
      (def
        (main (: n Int64))
        (do
          (def src (build n #set()))
          (def rt (Set.of (Set.to-list src)))
          (+ (* 10 (if (= rt src) 1 0)) (if (= (Set.len rt) n) 1 0))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 11 Int64))
  (live-objects known-leak))

(case
  "Set.to-list length is the set's cardinality (deduped)"
  (doc
    "`(List.len (Set.to-list (Set.of (list 3 1 2 1 3))))` — the enumerated list has one element per
           DISTINCT set element ({1,2,3} → 3), so its length equals Set.len. Pins the dedup + round count.
           Expected: 3.")
  (input (do (def (main) (List.len (Set.to-list #set(3 1 2 1 3)))) (export main)))
  (output (: 3 Int64)))

(case
  "Set.to-list orders a set of COMPOUND (tuple) elements lexicographically"
  (doc
    "The scalar cases above enumerate an Int set; this pins that a set of ORDERABLE COMPOUNDS enumerates
           in canonical LEXICOGRAPHIC order — the same total order the runtime `<` gives a tuple. `{(3,1),(1,2),
           (2,0)}` orders as `(1,2),(2,0),(3,1)` (first component decisive), so index 0 is `(1,2)` and its first
           component is 1. Regression witness for a wasm↔rust divergence where wasm FALSE-DECLINED a compound-
           element set ('no orderable descriptor') while rust computed the sorted order; the fix sorts by the
           descriptor-guided total order (`value_cmp_shaped`) on both. Expected: 1.")
  (input
    (do
      ; RUNTIME-IFIED (coord v-corpus-harness): thread the arg `n` into the first components so the
      ; #set is built at RUNTIME (not const-folded), exercising the compound-element Set.to-list
      ; reclaim. At n=0 the tuples are (3,1)/(1,2)/(2,0) — order + answer unchanged (index-0 (1,2), first 1).
      (def
        (main (: n Int64))
        (match
          (List.at (Set.to-list #set(#tuple((+ 3 n) 1) #tuple((+ 1 n) 2) #tuple((+ 2 n) 0))) 0)
          ((Some t) (. t 0))
          ((None u) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  ; RECLAIM WIN: was known-leak 2 (on an older compiler + when this const-folded); the native-#set-literal
  ; compound-element Set.to-list now reclaims fully at runtime (measured 0 on 05WfA5uY, fresh cdz, arg-varied
  ; n=5->6 confirms non-fold). Coverage preserved as a clean-0 reclaim assertion. (v-memory-safety)
  (live-objects 0))

; A single-field NEWTYPE `(type N (Mk Int64))` is a TRANSPARENT wrapper over its orderable Int64 payload, so
; a set of `N` elements HAS a total order (by the payload) and `Set.to-list` enumerates it — the orderability
; check must PEEL the nominal to its inner leaf. REGRESSION guard: #7234 moved the to-list orderability check
; to the shared front-end (CDZ0203 for a genuinely un-orderable element/key) but initially stopped honoring a
; newtype's orderable payload — it treated `Nominal { inner: Int64 }` as un-orderable and WRONGLY declined
; CDZ0203 (v-effects isolated it with this non-@invariant twin; concierge re-routed). Restored by peeling
; nominal/qty in `orderable_leaf_or_compound`. A newtype over an UN-orderable inner (a Set/Map, a float-leaf
; compound) still declines via the inner's own arm.
(case
  "Set.to-list over a newtype (Int64-payload) element enumerates via the newtype's orderable payload"
  (input
    (do
      (type N (Mk Int64))
      (def
        (main (: v Int64))
        (match
          (List.at (Set.to-list (Set.insert #set((N.Mk v)) (N.Mk 3))) 0)
          ((Some (N.Mk x)) x)
          ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "Set.to-list orders (Int,Bytes) tuples with the Bytes component as the tie-breaker"
  (doc
    "The tuple case above decides order on the FIRST (Int) component; this pins that when the first
           component TIES, the SECOND component — a `Bytes` leaf — breaks the tie by canonical unsigned-
           lexicographic byte order (the same total order a bare Bytes set gets). `{(1,[98]),(1,[97]),
           (2,[0])}`: both `(1,·)` elements share first component 1, so their Bytes leaf decides — [97] < [98]
           — giving canonical order `(1,[97]),(1,[98]),(2,[0])`. Index 0 is `(1,[97])`, weighted `100*1 + 97
           = 197`. Pins `value_cmp_shaped` DESCENDING into a compound's Bytes component as a secondary key
           (the Int-Int tuple case never exercises a Bytes sub-key, and the bare-Bytes set case is not a
           compound) — a Bytes tie-breaker that compared by handle identity or rope shape rather than
           content-order would mis-order the two `(1,·)` elements. Runtime Bytes leaves (nothing folds).
           Expected: 197.")
  (input
    (do
      (def
        (main (: k Int64))
        ; RUNTIME-IFIED (coord v-corpus-harness): thread `k` into the first components so the #set is
        ; built at RUNTIME (not const-folded). At k=0 the tuples are (1,[98])/(1,[97])/(2,[0]) — the
        ; first-component tie + Bytes tie-break + answer 197 all unchanged.
        (match
          (List.at
            (Set.to-list
              #set(#tuple((+ 1 k) (Bytes.of #list(98)))
                #tuple((+ 1 k) (Bytes.of #list(97)))
                #tuple((+ 2 k) (Bytes.of #list(0)))))
            0)
          ((Some t) (match (Bytes.at (. t 1) 0) ((Some v) (+ (* 100 (. t 0)) v)) ((None u) -1)))
          ((None u) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 197 Int64))
  ; RECLAIM WIN: was known-leak 2 (older compiler + const-fold); the native-#set-literal compound-element
  ; Set.to-list reclaims fully at runtime (measured 0 on 05WfA5uY, fresh cdz; k=5->697 confirms non-fold).
  ; Coverage preserved as a clean-0 reclaim assertion. (v-memory-safety)
  (live-objects 0))

(case
  "Set.to-list orders a set of RECORD elements canonically"
  (doc
    "The record companion of the tuple-element case: records order by comparing field values in the
           record's canonical (sorted) field order, so `{⟨x3,y a⟩, ⟨x1,y2⟩, ⟨x2,y0⟩}` enumerates with
           ⟨x1,y2⟩ first — index 0's `x` is 1. A record's runtime rep is a field-ordered tuple, but the
           SORT must consult the descriptor's canonical field order (not insertion or declaration
           accident) — the third compound-element kind (tuple/list/record) through the same
           `value_cmp_shaped` sort, over a runtime `a` so nothing folds. Expected: 1.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (List.at
            (Set.to-list
              #set(#record((= x 3) (= y a)) #record((= x 1) (= y 2)) #record((= x 2) (= y 0))))
            0)
          ((Some r) r.x)
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a float-carrying SUM as a Set key dedupes by payload and probes by content"
  (doc
    "The custom-Ord landing's key face (a monomorphic float-carrying sum is BTree-keyable on the
           rust backend; wasm keys via canonical bytes): {Temp x, Temp 1.5, Temp x, Missing} holds THREE
           keys (the duplicate Temp x dedupes) and a reconstructed (Temp x) probe hits. Pins that the
           sum's derived order/eq agree across backends on the discriminant + float-payload composite.")
  (input
    (do
      (type Reading (Temp Float64) (Missing))
      (def
        (main (: x Float64))
        (let
          ((s #set((Temp x) (Temp 1.5) (Temp x) (Missing))))
          (+ (Set.len s) (* 10 (if (Set.contains s (Temp x)) 1 0)))))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 13 Int64)))

(case
  "signed zeros INSIDE a sum payload are DISTINCT Set keys (the ±0.0 edge survives the wrapper)"
  (doc
    "The sign-of-zero edge one level down: Temp(+0.0) and Temp(-0.0) are distinct keys — the
           float-sum's order/eq must agree with the canonical-byte model (the box keeps a zero's sign
           bit) INSIDE the variant payload, exactly as the bare-float pins hold at top level. A
           float-sum Ord built on a partial_cmp that collapses ±0.0 would merge these to 2 keys.
           {Temp +0.0, Temp -0.0, Missing} = 3.")
  (input
    (do
      (type Reading (Temp Float64) (Missing))
      (def
        (main (: x Float64))
        (let ((s #set((Temp (- x x)) (Temp (* (- x x) -1.0)) (Missing)))) (Set.len s)))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 3 Int64)))

(case
  "Set.to-list orders float-carrying sums discriminant-first then by float payload"
  (doc
    "The ORDER face of the float-sum key family: to-list of {Missing, Temp 2.5, Temp 1.5} yields
           Temp(1.5), Temp(2.5), Missing — same-discriminant values order by their float payload's
           canonical bytes, and the later-declared variant sorts after. (wasm: todo until its to-list
           over a custom-Ord float-sum lands; rust + rust-async pin the pass.)")
  (input
    (do
      (type Reading (Temp Float64) (Missing))
      (def (rank (: r Reading)) (match r ((Temp f) (if (< f 2.0) 1 2)) ((Missing) 9)))
      (def
        (main (: x Float64))
        (let
          ((sorted (Set.to-list #set((Missing) (Temp x) (Temp 1.5)))))
          (match sorted (#list(a b c) (+ (rank a) (+ (* 10 (rank b)) (* 100 (rank c))))) (_ -1))))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 921 Int64)))

(case
  "a float-carrying sum as a MAP key is found by a reconstructed equal key"
  (doc
    "The Map twin of the float-sum Set-key pin: insert under (Temp x), look up with the
           RECONSTRUCTED (Temp (* x 1.0)) — the multiply is identity on the value but rebuilds the
           key, so the hit proves content-keying (not node identity) through the float-sum composite;
           the nullary (Missing) key coexists.")
  (input
    (do
      (type Reading (Temp Float64) (Missing))
      (def
        (main (: x Float64))
        (let
          ((m (Map.insert (Map.insert Map.empty (Temp x) 10) (Missing) 20)))
          (+
            (match (Map.lookup m (Temp (* x 1.0))) ((Some v) v) ((None _u) -1))
            (* 10 (match (Map.lookup m (Missing)) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 210 Int64)))

(case
  "Set.to-list over Float64 elements enumerates by canonical byte order"
  (doc
    "The FLOAT sibling of the compound-element to-list case: a set of Float64 elements enumerates by
           CANONICAL BYTE order — the element's bit pattern as an UNSIGNED integer, NOT numeric order. A float
           has no blessed NUMERIC total order (IEEE `<` is partial, NaN unordered), so `<` declines a float; but
           collections-and-text.md #Set Iteration Is Deterministic requires an element-derived order agreeing
           with the canonical byte form, which DOES totally order floats (NaN collapsed on construction, ±0.0
           distinct). By that order a NEGATIVE float (sign bit = high bit) sorts AFTER every positive: over a
           runtime `x` the set `{x, 0.5, 2.5}` at x=-1.0 enumerates `[0.5, 2.5, -1.0]`, so index 0 is 0.5 (→ 1),
           NOT -1.0 as numeric order would give. The length is the cardinality (3). Regression witness for a
           wasm↔rust divergence where wasm FALSE-DECLINED a float-element set ('no orderable descriptor') while
           rust computed the byte order; the fix gives `compare_scalar_leaf` a Float arm (`to_bits().cmp`)
           matching rust's `__CdzF64` wrapper, so both backends enumerate the same order.")
  (input
    (do
      (def
        (main (: x Float64))
        #tuple((List.len (Set.to-list #set(x 2.5 1.5)))
          (match
            (List.at (Set.to-list #set(x 0.5 2.5)) 0)
            ((Some f) (if (= f 0.5) 1 0))
            ((None u) -1))))
      (export main)))
  (call main (: 3.5 Float64))
  (output (: (tuple 3 1) (Tuple Int64 Int64)))
  (call main (: -1.0 Float64))
  (output (: (tuple 3 1) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "Set.to-list places a NaN element after the positives but before the negatives, by canonical byte order"
  (doc
    "The NaN-POSITION companion of the float byte-order case above: canonical (quiet) NaN's bits are
           `0x7ff8000000000000`, which as an UNSIGNED integer sorts AFTER every positive finite (whose sign
           bit is 0, so bits < `0x7ff8…`) but BEFORE every negative (sign bit = high bit → bits ≥ `0x8000…`).
           So a NaN is NOT ordered 'last' — it lands between the positives and the negatives. Over a set
           `{1.5, NaN, -2.0}` with a RUNTIME NaN (`(/ x x)` at x=0.0 — a const NaN has no value form and is
           compile-rejected, so the NaN must be produced at run time) the canonical order is `[1.5, NaN, -2.0]`,
           so index 1 is the NaN: `List.at … 1` equals the runtime NaN under the canonical byte form
           (`(= e nan)` is true — all NaNs share one canonical form, unlike IEEE `nan ≠ nan`). Pins the NaN
           slot in the `compare_scalar_leaf` `to_bits().cmp` order (matching rust's `__CdzF64`), the boundary
           the finite-only order-face above doesn't reach.")
  (input
    (do
      (def
        (main (: x Float64))
        (let
          ((nan (/ x x)))
          (match
            (List.at (Set.to-list #set(1.5 nan -2.0)) 1)
            ((Some e) (if (= e nan) 1 0))
            ((None u) -1))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 1 Int64)))

; The Set.to-list cases above enumerate a CONSTANT `Set.of` literal. A set built AT RUN TIME by a
; recursive `Set.insert` loop over a boundary parameter is a genuine runtime CHAMP the `set-to-list`
; runtime op (index 83) walks live — its canonical (sorted) order emerges from the cursor walk + the
; canonical-scalar sort, NOT from a folded pre-sorted literal. These pin the runtime enumeration op end
; to end: the order is canonical regardless of insertion order, and the enumerated list is consumed by
; a List.at/List.len fold (the idiom a self-hosted pass uses to iterate a set's members deterministically).
(case
  "Set.to-list over a RUNTIME-built set yields canonical order (first element is the minimum)"
  (doc
    "`ins n` inserts `20-n` for n=n..1 into a set built by a recursive `Set.insert` loop — so the
           elements arrive in DESCENDING order (19,18,…) but the set is unordered. `Set.to-list` enumerates
           them in canonical (ascending) order, so element 0 is the minimum. `ins 5` inserts {15,16,17,18,19};
           the first enumerated element is 15. Pins that the runtime set-to-list op sorts by canonical value,
           not insertion order, over a genuinely runtime-built CHAMP (not a folded constant `Set.of`).")
  (input
    (do
      (def (ins (: n Int64) (: s (Set Int64))) (if (< n 1) s (ins (- n 1) (Set.insert s (- 20 n)))))
      (def (main (: n Int64)) (Option.expect (List.at (Set.to-list (ins n #set())) 0) "empty"))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "Set.to-list canonical order is SIGNED at the integer extremes"
  (doc
    "A set holding BOTH i64 limits enumerates with the NEGATIVE extreme first and the positive
           extreme last — the canonical order is the SIGNED value order at the sign boundary. A sort
           comparing raw two's-complement bytes or unsigned values would place Int64.min (0x8000…)
           AFTER Int64.max (0x7FFF…), inverting the ends. `(Set.of (list max n 0 min))` with n=5:
           element 0 is min. The extreme-key companion of the ascending-order pins above, whose keys
           never cross the sign boundary.")
  (input
    (do
      (def
        (main (: n Int64))
        (Option.expect
          (List.at (Set.to-list #set(9223372036854775807 n 0 -9223372036854775808)) 0)
          "empty"))
      (export main)))
  (call main (: 5 Int64))
  (output (: -9223372036854775808 Int64)))

(case
  "Set.to-list over a runtime set sums its distinct elements"
  (doc
    "`ins n` inserts `(n*7) % 5` for n=n..1 — a runtime set whose elements cycle through {0,1,2,3,4}
           with many collisions (dedup). `Set.to-list` enumerates the distinct elements; a List.at fold sums
           them. `ins 10` deduplicates to {0,1,2,3,4}, sum 10. Pins that the runtime enumeration yields each
           DISTINCT element exactly once and the resulting list is fold-consumable (the set→list→fold idiom).")
  (input
    (do
      (def
        (ins (: n Int64) (: s (Set Int64)))
        (if (< n 1) s (ins (- n 1) (Set.insert s (% (* n 7) 5)))))
      (def
        (sumlist (: l (List Int64)) (: i Int64) (: a Int64))
        (if (= i (List.len l)) a (sumlist l (+ i 1) (+ a (Option.expect (List.at l i) "oob")))))
      (def (main (: n Int64)) (sumlist (Set.to-list (ins n #set())) 0 0))
      (export main)))
  (call main (: 10 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

; The Set.to-list order pins above are all over INT elements (and one tuple case). STRING elements take a
; DIFFERENT comparator arm — the zero-alloc scalar fast-path (`compare_scalar_leaf`) flattens a ROPE string
; to a leaf, then compares the borrowed UTF-8 byte slices — and that arm was rewritten for the alloc-bench
; regression (the sort comparator must be allocation-free on the scalar path). These pin the STRING sort
; order through `Set.to-list` over GENUINELY-RUNTIME ROPES (recursive String.concat defeats the fold), on
; the three faces the byte-lexicographic spec order (13-strings:53/66/78) distinguishes: smallest-first
; content, multibyte-after-ascii (UNSIGNED byte order), and prefix-before-extension (the length tiebreak).
(case
  "Set.to-list sorts runtime ROPE string elements smallest-first by content byte order"
  (doc
    "`rep` builds each element as a runtime ROPE (`(rep \"b\" 2)` concatenates two \"x\" → \"bxx\"),
           so the set holds three genuine rope strings {\"bxx\",\"axx\",\"cxx\"}. `Set.to-list` must sort
           by flattened CONTENT (byte-lexicographic, 13-strings:53), so element 0 is the \"a\" rope; the
           `=` against the flat literal \"axx\" confirms content (rope==flat, canonicalized), not just
           position. Pins the sort comparator's Str arm flattens ropes before comparing — a comparator
           reading only the first leaf of an unflattened rope would compare \"b\"/\"a\"/\"c\" correctly
           here but break on shared prefixes; the content check guards the flatten. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main (: n Int64))
        (match
          (List.at (Set.to-list #set((rep "b" n) (rep "a" n) (rep "c" n))) 0)
          ((Some s) (if (= s "axx") 1 0))
          ((None u) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "Set.to-list orders runtime Bytes elements by unsigned-lexicographic byte order — 0x80 sorts LAST, not as signed -128"
  (doc
    "Bytes gained a BLESSED TOTAL ORDER (§order, operator directive 2026-08-02; rcdzc+runtime #1120):
           content-lexicographic over UNSIGNED byte values, the same machinery as String/Symbol (03-equality:602
           is the bare-`<` witness). This pins the COLLECTION face: a `Set` of single-byte `Bytes` {0x80,0x05,0x7f}
           built runtime + inserted out of order enumerates via `Set.to-list` in UNSIGNED order [5, 127, 128] —
           first element's byte is 5, and the LAST is 128 (`0x80`). The 0x80-LAST is the discriminating assertion:
           a SIGNED byte order (the trap) would read `0x80` as -128 and sort it FIRST, making the last element 127.
           So this guards the unsigned-lexicographic contract THROUGH the to-list enumeration path (not just the
           bare relational op), uniform across backends (wasm bytes-len/get walk == rust `Vec<u8>` Ord). Encodes
           first-byte (100s) and last-byte (units) → 100*5 + 128 = 628.")
  (input
    (do
      (def (b1 (: n Int64)) (Bytes.of #list((UInt8.wrap n))))
      (def
        (lastbyte (: xs (List Bytes)) (: acc Int64))
        (match
          xs
          (#list() acc)
          (#list(h (.. t)) (lastbyte t (match (Bytes.at h 0) ((Some v) v) ((None u) -1))))))
      (def
        (main (: z Int64))
        (let
          ((xs (Set.to-list #set((b1 128) (b1 5) (b1 127)))))
          (+
            (*
              100
              (match
                xs
                (#list(h (.. t)) (match (Bytes.at h 0) ((Some v) v) ((None u) -1)))
                (#list() -2)))
            (lastbyte xs -9))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 628 Int64))
  (live-objects 0))

(case
  "Map.to-list enumerates runtime Bytes keys by unsigned-lexicographic byte order — same key-cmp as the Set element order"
  (doc
    "The Map-key companion of the Set.to-list Bytes-order pin above: a `Map` keyed by single-byte `Bytes`
           {0x80↦900, 0x05↦100, 0x7f↦700} inserted out of order enumerates its (key value) pairs via `Map.to-list`
           in UNSIGNED key order [5, 127, 128] — the FIRST pair's key-byte is 5 and the LAST is 128 (0x80), the
           same signed-vs-unsigned discriminator as the Set case (a signed order would put 0x80=-128 first → last
           key 127). Confirms the Bytes total order drives the CHAMP key enumeration identically for Map keys as
           for Set elements (same `value_cmp_shaped` Bytes arm). Encodes first-key-byte (100s) + last-key-byte
           (units) → 100*5 + 128 = 628.")
  (input
    (do
      (def (b1 (: n Int64)) (Bytes.of #list((UInt8.wrap n))))
      (def
        (lastkey (: xs (List (Tuple Bytes Int64))) (: acc Int64))
        (match
          xs
          (#list() acc)
          (#list(h (.. t))
            (match h (#tuple(k _v) (lastkey t (match (Bytes.at k 0) ((Some v) v) ((None u) -1))))))))
      (def
        (main (: z Int64))
        (let
          ((m (Map.insert (Map.insert (Map.insert Map.empty (b1 128) 900) (b1 5) 100) (b1 127) 700)))
          (let
            ((ps (Map.to-list m)))
            (+
              (*
                100
                (match
                  ps
                  (#list(h (.. t))
                    (match h (#tuple(k _v) (match (Bytes.at k 0) ((Some v) v) ((None u) -1)))))
                  (#list() -2)))
              (lastkey ps -9)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 628 Int64))
  (live-objects 0))

(case
  "Set.to-list orders a multibyte string element AFTER ascii by unsigned byte order"
  (doc
    "String order is UNSIGNED byte-lexicographic (13-strings:78 — a multi-byte scalar's lead byte
           0xC3 exceeds every ASCII byte), so in `{\"é\",\"z\",\"a\"}` the multibyte \"é\" sorts LAST:
           `Set.to-list` index 2 is \"é\" → 1. The control (n≤0 picks \"q\" instead of \"é\") reorders to
           {\"a\",\"q\",\"z\"} where index 2 is \"z\" → 2 — confirming the probe reads a real sort, not a
           fixed slot. A comparator sorting by SIGNED bytes (0xC3 as -61) or by scalar-count-then-bytes
           would place \"é\" first, flipping the answer. The runtime `if`-selected element keeps the set
           construction out of the constant fold. Expected: 1 (n=1), 2 (n=-1).")
  (input
    (do
      (def (pick (: n Int64)) (if (> n 0) "é" "q"))
      (def
        (main (: n Int64))
        (match
          (List.at (Set.to-list #set((pick n) "z" "a")) 2)
          ((Some s) (if (= s "é") 1 (if (= s "z") 2 0)))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: -1 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "Set.to-list sorts a PREFIX string before its extension (length tiebreak on equal prefixes)"
  (doc
    "`{\"axxx\",\"a\"}` — the flat literal \"a\" is a proper PREFIX of the rope `(rep \"a\" 3)` =
           \"axxx\"; byte-lexicographic order places a prefix BEFORE its extension (13-strings:66), so
           `Set.to-list` index 0 is \"a\" with byte-len 1. This is the tiebreak face the smallest-first
           case cannot witness (its elements differ at byte 0): the comparator must compare the COMMON
           prefix equal and then decide by LENGTH, not read past the shorter string's end or fall back
           to insertion order. One element is a runtime rope, the other a flat literal — the mixed-rep
           pair the flatten-then-compare path must canonicalize consistently. Expected: 1.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main (: n Int64))
        (match
          (List.at (Set.to-list #set((rep "a" n) "a")) 0)
          ((Some s) (String.byte-len s))
          ((None u) -1)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (live-objects 0))

; The cases above all enumerate a NON-EMPTY set. The empty boundary matters for a real pass that walks a
; possibly-empty symbol table / free-var set: `Set.to-list` of an EMPTY (but element-TYPED) set is the
; empty list, length 0. The set is emptied at RUN TIME (`Set.remove` of the sole element) so the element
; type is `Int64` (fixing the canonical-ordering descriptor) while the runtime CHAMP is empty — distinct
; from an untyped empty `Set.of (list)` literal, whose element type is undetermined.
(case
  "Set.to-list of a runtime-empty but element-typed set is the empty list"
  (doc
    "`(Set.remove (Set.of (list 1)) 1)` is a `Set Int64` emptied at run time; `Set.to-list` of it is
           the empty list, so `List.len` is 0. Pins the empty boundary of set enumeration — a pass walking a
           set that happens to be empty gets an empty list, not a trap — with the element type fixed to
           Int64 (so the canonical-order descriptor is well-defined), the shape a symbol-table / free-var
           enumeration takes when the collection is empty.")
  (input (do (def (main) (List.len (Set.to-list (Set.remove #set(1) 1)))) (export main)))
  (output (: 0 Int64)))

(case
  "Set.to-list of a CONSTANT empty set through an inlined nullary is the empty list"
  (doc
    "The constant-empty companion of the runtime-empty case above: a bare `(Set.of (list))` built by
           an inlined nullary `(es)` — its element type is UNDETERMINED (no elements pin it), the exact
           empty-collection shape a self-hosting compiler's fresh free-variable / visited set takes at
           seed. `(List.len (Set.to-list (es)))` is 0. Pins that an element-typeless constant empty set
           enumerates to the empty list (length 0), distinct from the runtime-emptied `Set.remove` path.")
  (input (do (def (es) #set()) (def (main) (List.len (Set.to-list (es)))) (export main)))
  (output (: 0 Int64)))

(case
  "insert-order does not leak into Set.to-list enumeration order"
  (doc
    "{3, 1, 2} built by inserts IN THAT ORDER enumerates [1, 2, 3] — element 0 is 1 and element
           2 is 3 -> 103. Insertion HISTORY is unobservable (canonical order); a cursor walking
           trie/hash order (which varies with insert sequence) or an append-in-insert-order
           enumeration leaks it. Complements the runtime-min case above (which pins the first
           element) by pinning a NON-head position of a scrambled successive-insert build.")
  (input
    (do
      (def
        (main (: d Int64))
        (let
          ((xs (Set.to-list (Set.insert (Set.insert (Set.insert #set() 3) 1) 2))))
          (+ (* 100 (Option.expect (List.at xs 0) "a")) (Option.expect (List.at xs 2) "c"))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 103 Int64)))

; Set.of over a RUNTIME (non-literal) list — a set built from a list the compiler cannot see the elements
; of (a `Set.to-list` result, a `List.concat`, a param/recursively-built list). `Set.of` semantically IS a
; left fold of `Set.insert` from the empty set, so the compiler synthesizes that fold and runs it at
; runtime (a constant `(list …)` literal still folds to a canonical set at compile time). The motivating
; consumer is the enumeration ROUND-TRIP: rebuilding a set from its own `Set.to-list` recovers an equal set.
(case
  "Set.of of a Set.to-list result reconstructs the same set (the enumeration round-trips)"
  (doc
    "The closure property: rebuilding a set from its own canonical enumeration recovers an EQUAL set.
           Over a RUNTIME set s = {a, b} (a duplicate `a` collapses), `(Set.of (Set.to-list s))` builds a new
           set by folding `Set.insert` over the enumerated list — and it equals `s`. Pins that `Set.to-list`
           loses/adds no element AND that `Set.of` over a runtime (non-literal) list constructs the same
           CHAMP as the source. A `Set.of` that only accepted a compile-time list literal would DECLINE this
           (the runtime-list construction is the synthesized fold); a to-list that dropped an element would
           break the round-trip. MUST be 1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let ((s #set(a b a))) (if (= (Set.of (Set.to-list s)) s) 1 0)))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "Set.of over a runtime list of TUPLE elements reconstructs the same set through the synthesized fold"
  (doc
    "The round-trip above uses SCALAR elements; this runs the same runtime-`Set.of` fold over COMPOUND
           elements — a set of TUPLES round-tripped through its own `Set.to-list`. `s = {(a,1), (b,2)}` (the
           repeated `(a,1)` collapses by whole-tuple value), then `(Set.of (Set.to-list s))` rebuilds by
           folding `Set.insert` over the enumerated tuple list. The rebuilt set has `Set.len` 2 AND `= s`
           (maps/sets of compounds compare by canonical value). Exercises the synthesized `Set.insert` fold
           over COMPOUND elements — the CHAMP compound-element hash/eq walk on the runtime-construction path,
           not just the scalar path the round-trip case above pins. Encodes 10·len + (= s) = 10·2 + 1 = 21.
           A fold that mis-hashed a tuple element or dropped one would flip a component. MUST be 21.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((s #set(#tuple(a 1) #tuple(b 2) #tuple(a 1))))
          (let ((r (Set.of (Set.to-list s)))) (+ (* 10 (Set.len r)) (if (= r s) 1 0)))))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 21 Int64))
  (live-objects known-leak))

(case
  "Set.of of a computed (concatenated) runtime list dedups by value"
  (doc
    "`Set.of` over a `List.concat` result — a runtime list the compiler has no element-list to fold at
           compile time. `(List.concat (list a b) (list b a))` = [a, b, b, a]; `Set.of` of it dedups to
           {a, b} → `Set.len` 2. Pins runtime-list set CONSTRUCTION (the synthesized `Set.insert` fold) with
           the dedup that makes it a set. A construction that kept duplicates (a plain list-copy) would give
           4. MUST be 2.")
  (input
    (do
      (def (main (: a Int64) (: b Int64)) (Set.len (Set.of (List.concat #list(a b) #list(b a)))))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 2 Int64))
  ; the runtime-list Set.of construction (synthesized monomorphic fold) leaks 6 on 05WfA5uY (fresh cdz,
  ; args defeat fold). [A prior re-pin to 0 was a STALE-CDZ artifact — an old cdz declined/folded runtime
  ; Set.of; the current compiler builds it and it leaks. Corrected back to 6.] (v-memory-safety)
  (live-objects known-leak))

; Building runtime sets at TWO different element types in ONE program. Each runtime-`Set.of` site gets its
; OWN synthesized fold def (`__set_of_rt$0`, `__set_of_rt$1`, …), so every fold is MONOMORPHIC — instantiated
; at exactly one element type — and no single generic def is instantiated at two types. This sidesteps the
; recursive-generic `Set.insert`/empty-seed element-var grounding tie that a single shared generic fold hit
; (formerly a CDZ0201 decline pinned here as `todo`). Both a `Set Int64` and a `Set Bool` build and dedup.
(case
  "runtime Set.of at two different element types in one program each build via a per-site monomorphic fold"
  (doc
    "Two runtime-`Set.of` constructions at DIFFERENT element types in one program: a `Set Int64` from
           `(List.concat (list a b) (list a))` and a `Set Bool` from `(List.concat (list (> a b)) (list (<
           a b)))`. Each site is rewritten to call its OWN synthesized fold def, so each fold is monomorphic
           (one element type) and both compile — no cross-type instantiation of a shared generic def, so the
           recursive-generic element-grounding tie never arises. Int set {a,b} = 2, Bool set {true,false} = 2
           → 2 + 10·2 = 22. A single-element-type program is byte-identical to the earlier one-def synthesis.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (+
          (Set.len (Set.of (List.concat #list(a b) #list(a))))
          (* 10 (Set.len (Set.of (List.concat #list((> a b)) #list((< a b))))))))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 22 Int64))
  ; the two per-site monomorphic runtime Set.of folds leak 9 on 05WfA5uY (fresh cdz, args defeat fold).
  ; [A prior re-pin to 0 was a STALE-CDZ artifact — an old cdz declined/folded runtime Set.of; the current
  ; compiler builds it and it leaks. Corrected back to 9.] (v-memory-safety)
  (live-objects known-leak))

; The N-site generalization of the per-site monomorphic fold: THREE runtime-`Set.of` sites at THREE distinct
; element types in one program — a `Set Int64`, a `Set Bool`, AND a `Set String`. Each site gets its own
; monomorphic fold def, so no synthesized def is instantiated at more than one element type and the
; recursive-generic tie never arises regardless of how many distinct types coexist. Int set {a,b} = 2; Bool
; set {a>b, a<b} = {false, true} = 2; String set {(if a>b "hi" "lo"), "lo"} — with a<b the branch selects
; "lo", so the set is {"lo"} = 1. 2 + 10·2 + 100·1 = 122. Guards that per-site synthesis scales past the
; pairwise case and that a String (heap-rope element) set participates.
(case
  "runtime Set.of at THREE different element types in one program all build via per-site monomorphic folds"
  (doc
    "Three runtime-`Set.of` constructions at DIFFERENT element types — `Set Int64` from `(list a b a)`,
           `Set Bool` from `(list (> a b) (< a b))`, and `Set String` from `(list (if (> a b) \"hi\" \"lo\")
           \"lo\")` — coexist in one program. Each site is rewritten to its OWN synthesized monomorphic fold,
           so N distinct element types coexist with no cross-type instantiation. With a=5, b=7: Int {5,7}=2,
           Bool {false,true}=2, String {\"lo\"}=1 (the `a>b` branch is false → \"lo\", deduped with the literal
           \"lo\"). 2 + 10·2 + 100·1 = 122. The N-site generalization of the two-type case above.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (+
          (Set.len #set(a b a))
          (+
            (* 10 (Set.len #set((> a b) (< a b))))
            (* 100 (Set.len #set((if (> a b) "hi" "lo") "lo"))))))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 122 Int64)))

(case
  "a runtime-keyed map entry enumerates as its key-value tuple"
  (doc
    "`(Map.to-list (Map.insert Map.empty k 42))` with k a parameter — the single entry
           enumerates as a (k, 42) tuple whose value projects 42. Pins the entry-tuple
           materialization over a runtime key (a folded key builds the tuple at compile time; this
           one must build it from live heap values).")
  (input
    (do
      (def
        (main (: k Int64))
        (. (Option.expect (List.at (Map.to-list (Map.insert Map.empty k 42)) 0) "e") 1))
      (export main)))
  (call main (: 7 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a Float element inserted into an empty (runtime) set boxes with box-float, not box-int"
  (doc
    "MISCOMPILE (invalid wasm, wasm-only): `Set.insert (Set.of (list)) x` with `x : Float64` — a
           SINGLE float insert into a runtime EMPTY set — imported `box-int` but the emit called
           `box-float`, so `box-float` was un-imported and the call resolved to `u32::MAX` → invalid
           component at load. ROOT: the import collector used `box_op_ty(elem_ty)` while the emit used
           `box_op_for(elem_node, elem_ty)`; for an empty base the element type is an unresolved `Var`, which
           `box_op_ty` DEFAULTS to `box-int` but `box_op_for` resolves from the element NODE (a Float →
           `box-float`) — a coemit mismatch, the empty-set String box-int bug's float twin. A CONSTANT float
           `Set.of` folds (never emits the insert), which is why only the runtime empty-base insert broke. Fix:
           the collector's Set/Map insert arms use `box_op_for` (node-aware) so imports match the emit.
           `Set.len` of the 1-element set is 1.")
  (input (do (def (main (: d Float64)) (Set.len (Set.insert #set() d))) (export main)))
  (call main (: 2.5 Float64))
  (output (: 1 Int64)))

(case
  "a Float VALUE inserted into an empty (runtime) map boxes with box-float, not box-int"
  (doc
    "The Map-VALUE twin of the empty-set float box case above: `Map.insert Map.empty 1 x` with a
           runtime `x : Float64` into an empty map (undetermined value type) — the value box op must come
           from `x`'s node type (`box-float`), imported to match the emit; before the collector used
           `box_op_for` for the value it grounded the `Var` value type to `box-int` → un-imported
           `box-float` → invalid wasm. One entry → `Map.len` = 1.")
  (input (do (def (main (: d Float64)) (Map.len (Map.insert (Map.empty) 1 d))) (export main)))
  (call main (: 2.5 Float64))
  (output (: 1 Int64)))

(case
  "a Float KEY inserted into an empty (runtime) map boxes with box-float, not box-int"
  (doc
    "The Map-KEY twin: `Map.insert Map.empty x 1` with a runtime `x : Float64` key into an empty map
           (undetermined key type) — the key box op comes from `x`'s node type (`box-float`), imported to
           match the emit (the same node-aware `box_op_for` collector fix). One entry → `Map.len` = 1.")
  (input (do (def (main (: d Float64)) (Map.len (Map.insert (Map.empty) d 1))) (export main)))
  (call main (: 3.5 Float64))
  (output (: 1 Int64)))

(case
  "a map insert forces a trapping value at construction — Map.len traps, not returns the count"
  (doc
    "Map (heap-materialized) construction is STRICT: inserting a trapping VALUE forces it at construction,
           so a trapping value traps even though only Map.len is later taken. `(Map.len (Map.insert (Map.empty)
           1 (/ 5 d)))` at d=0 — the value `(/ 5 0)` is a divide-by-zero — TRAPS rather than returning 1; at d=1
           the value is 5, len 1. Pins map-construction strictness (core-semantics.md #A Trap Occurs Only Where
           Its Computation Is Observed: a heap collection forces its entries at construction), the map twin of
           the list-len trapping-element pin (28-compiler-primitives).")
  (input (do (def (main (: d Int64)) (Map.len (Map.insert (Map.empty) 1 (/ 5 d)))) (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a set of a list with a trapping element forces it at construction — Set.len traps, not returns the count"
  (doc
    "Set (heap-materialized) construction is STRICT: a trapping element is forced at construction, so
           `(Set.len #set((/ 5 d) 2 3))` at d=0 — the element `(/ 5 0)` is a divide-by-zero — TRAPS
           rather than returning 3; at d=1 the elements are {5,2,3}, len 3. Pins set-construction strictness
           (via the strict native #set literal), the set twin of the map/list trapping-element pins.")
  (input (do (def (main (: d Int64)) (Set.len #set((/ 5 d) 2 3))) (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

; --- Float CHAMP keys/elements under the canonical byte form ----------------------------------------
; 9c2790cef fixed the Float element-boxing arm (box-float, not the defaulted box-int — my filed
; invalid-wasm; its pin covers the empty-set insert). These pin the canonical-form semantics the
; boxing now reaches, promoted from breaker probes held back until the fix: NaN is ONE key; -0.0
; and 0.0 are TWO.
(case
  "a NaN map key is found by a differently-produced NaN"
  (doc
    "Insert under `(/ x x)` at x = 0.0 (a computed NaN), look up with `Float64.nan` → 42.
           Every NaN shares one canonical byte form, so champ_hash/champ_eq land both spellings in
           one slot — the map-key face of the scalar NaN-equality rule (a raw-bits hash scatters
           NaN keys; raw f64.eq never matches them).")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup (Map.insert Map.empty (/ x x) 42) Float64.nan)
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 42 Int64)))

(case
  "negative zero and zero are distinct map keys"
  (doc
    "Insert 1 under -0.0 and 2 under 0.0: the map holds TWO entries and -0.0 looks up its own
           value → 10·2 + 1 = 21. The -0.0 complement of the NaN-unification key face (an f64.eq
           key compare collapses the pair to one entry).")
  (input
    (do
      (def
        (main (: d Int64))
        (+
          (* 10 (Map.len (Map.insert (Map.insert Map.empty -0.0 1) 0.0 2)))
          (match
            (Map.lookup (Map.insert (Map.insert Map.empty -0.0 1) 0.0 2) -0.0)
            ((Some v) v)
            ((None _) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 21 Int64)))

(case
  "a set dedups NaN elements and keeps zero signs distinct"
  (doc
    "Insert a computed NaN then `Float64.nan` → ONE element (canonical unification); insert
           -0.0 then 0.0 → TWO (distinct canonical forms): 10·1 + 2 = 12. The set-element face of
           both canonical-form rules through the fixed box-float path.")
  (input
    (do
      (def
        (main (: x Float64))
        (+
          (* 10 (Set.len (Set.insert (Set.insert #set() (/ x x)) Float64.nan)))
          (Set.len (Set.insert (Set.insert #set() -0.0) 0.0))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 12 Int64)))

(case
  "a computed float map key is found by its literal twin"
  (doc
    "Insert under `(+ x 1.25)` at x = 1.25, look up with the literal 2.5 → 42. The
           arithmetic-result key and the literal share one canonical form (float arithmetic is
           deterministic; the emitted add's bits equal the folded literal's) — the computed-key
           control beside the special-value faces.")
  (input
    (do
      (def
        (main (: x Float64))
        (match (Map.lookup (Map.insert Map.empty (+ x 1.25) 42) 2.5) ((Some v) v) ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 42 Int64)))

; The NaN-key / signed-zero / computed-key cases above are all Float64. `box-float32` canonicalizes at the
; 4-BYTE width (`f32::NAN.to_bits()`, sign-preserving zero) — a DISTINCT path from `box-float`'s 8-byte form
; — so the same canonical-byte CHAMP key/element rules must hold for a Float32 key at f32 width: a NaN
; unifies, a signed zero stays distinct, and a computed key hits its literal twin. These pin that f32 axis.
(case
  "a Float32 set dedups NaN elements and keeps zero signs distinct"
  (doc
    "The Float32 analogue of the Float64 set-dedup case: insert a computed f32 NaN (`(/ x x)` at x=0)
           then `Float32.nan` → ONE element (the 4-byte-width canonical quiet NaN unifies them); insert a
           Float32 `-0.0` then `0.0` → TWO (the sign bit is kept at f32 width). 10·1 + 2 = 12. Pins that the
           CHAMP set hash/eq canonicalizes a Float32 element at its OWN 32-bit width, not only Float64.")
  (input
    (do
      (def
        (main (: x Float32))
        (+
          (* 10 (Set.len (Set.insert (Set.insert #set() (/ x x)) Float32.nan)))
          (Set.len (Set.insert (Set.insert #set() (: -0.0 Float32)) (: 0.0 Float32)))))
      (export main)))
  (call main (: 0.0 Float32))
  (output (: 12 Int64)))

(case
  "a Float32 NaN map key is found by a differently-produced NaN"
  (doc
    "The Float32 map-key face: insert under a COMPUTED f32 NaN (`(/ x x)` at x=0) and look up with
           `Float32.nan` → 42. Both NaNs canonicalize to the one 4-byte quiet NaN, so they hash+compare
           equal and land in the same CHAMP slot (the f32-width analogue of the Float64 NaN-map-key case). A
           raw-bits hash would scatter differently-produced NaNs into distinct slots and MISS.")
  (input
    (do
      (def
        (main (: x Float32))
        (match
          (Map.lookup (Map.insert Map.empty (/ x x) 42) Float32.nan)
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 0.0 Float32))
  (output (: 42 Int64)))

(case
  "a computed Float32 map key is found by its literal twin"
  (doc
    "The Float32 computed-key control: insert under `(+ x 1.25)` at x=1.25 (a Float32 add) and look up
           with the literal `2.5` at f32 width → 7. The arithmetic-result key and the literal share one f32
           canonical byte form (f32 arithmetic is deterministic; the add's bits equal the folded literal's),
           the f32 companion of the Float64 computed-key case.")
  (input
    (do
      (def
        (main (: x Float32))
        (match
          (Map.lookup (Map.insert Map.empty (+ x (: 1.25 Float32)) 7) (: 2.5 Float32))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float32))
  (output (: 7 Int64)))

; The float-key cases above use a BARE float key. A float leaf INSIDE a COMPOUND key (a tuple element) must
; also key by content: the CHAMP hash/eq descends into the tuple and canonicalizes the float leaf at its
; width. This declined on the rust backend until the __CdzF ord-wrapper was threaded through tuple keys
; (v-rust-backend d0d18e257); now a (Tuple Float Int64) Map key / Set element keys by content on all three
; backends. These pin that compound-float-key path: hit-by-content, a MISS on a different float leaf, Set
; dedup of equal tuple-float elements, and canonical-NaN dedup with an f32 leaf.
(case
  "a tuple with a Float64 element is a Map key found by a separately-built equal key"
  (doc
    "A `(Tuple Float64 Int64)` Map key: insert under `(tuple (+ x 1.25) 3)` at x=1.25 (a RUNTIME float
           leaf, off the const-fold) and look up with `(tuple 2.5 3)` → 42. The CHAMP key hash/eq descends
           INTO the tuple and compares the float leaf by its canonical byte form, so the arithmetic-built
           and literal tuples are the same key. Pins the float-leaf-in-compound-key path (rust emits it via
           the tuple-threaded ord-wrapper, d0d18e257).")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup (Map.insert Map.empty #tuple((+ x 1.25) 3) 42) #tuple(2.5 3))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 42 Int64)))

(case
  "a tuple with a Float64 element MISSES a Map key with a different float leaf"
  (doc
    "The negative control of the compound-float-key case: the tuples share the Int64 element (3) but
           differ in the float leaf — look up `(tuple 9.5 3)` against a map keyed by `(tuple 2.5 3)` → None
           (-1). Confirms the compound-key compare is genuinely by the float leaf's content (not
           over-matching on the shared Int element or the tuple shape alone).")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup (Map.insert Map.empty #tuple((+ x 1.25) 3) 42) #tuple(9.5 3))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: -1 Int64)))

(case
  "a Set of tuples with a Float64 element dedups equal members by content"
  (doc
    "The Set companion: insert `(tuple (+ x 1.25) 3)` (runtime float leaf) then `(tuple 2.5 3)` into a
           set → ONE element (`Set.len` 1), the two tuple-float values sharing one canonical form. Pins that
           the CHAMP set dedup descends into the tuple's float leaf, the compound-key face of the bare-float
           set-dedup cases.")
  (input
    (do
      (def
        (main (: x Float64))
        (Set.len (Set.insert (Set.insert #set() #tuple((+ x 1.25) 3)) #tuple(2.5 3))))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 1 Int64)))

(case
  "a Set of tuples with an f32 NaN element dedups by the canonical quiet NaN"
  (doc
    "The Float32-NaN-in-a-tuple face: insert `(tuple (/ x x) 3)` (a computed f32 NaN at x=0) then
           `(tuple Float32.nan 3)` → ONE element. Both NaN leaves canonicalize to the one 4-byte quiet NaN
           inside the tuple, so the tuples dedup by content. Pins that the compound-key canonicalization
           reaches an f32 NaN leaf at its own width, the tuple companion of the bare-f32-NaN dedup case.")
  (input
    (do
      (def
        (main (: x Float32))
        (Set.len (Set.insert (Set.insert #set() #tuple((/ x x) 3)) #tuple(Float32.nan 3))))
      (export main)))
  (call main (: 0.0 Float32))
  (output (: 1 Int64)))

; The tuple-float-key cases above cover a float leaf in a TUPLE key; a float leaf in a RECORD key is the
; structural-record twin. It ALSO declined on rust until v-rust-backend threaded the ord-wrapper through
; record keys in sorted-field order (94ea8c58b); now a `(record (f Float) (n Int64))` Map key / Set element
; keys by content on all three. These pin the record-float-key path — hit-by-content, a MISS on a different
; float field, and Set dedup — the record companion of the tuple-float-key cases. (A float in a SUM PAYLOAD
; key still declines on rust — per-variant threading is a later increment — so those are kept out.)
(case
  "a structural record with a Float64 field is a Map key found by a separately-built equal key"
  (doc
    "A structural-record Map key `(record (f Float64) (n Int64))`: insert under `(record (f (+ x 1.25))
           (n 3))` at x=1.25 (a RUNTIME float field) and look up with `(record (f 2.5) (n 3))` → 42. The
           CHAMP key hash/eq descends into the record's fields (sorted-field order) and compares the float
           field by its canonical byte form. Pins the float-leaf-in-record-key path (rust emits it via the
           record-threaded ord-wrapper, 94ea8c58b), the record twin of the tuple-float-key case.")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert Map.empty #record((= f (+ x 1.25)) (= n 3)) 42)
            #record((= f 2.5) (= n 3)))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 42 Int64)))

(case
  "a structural record with a Float64 field MISSES a Map key with a different float field"
  (doc
    "The negative control: the records share the Int64 field (n=3) but differ in the float field —
           look up `(record (f 9.5) (n 3))` against a map keyed by `(record (f 2.5) (n 3))` → None (-1).
           Confirms the record-key compare is genuinely by the float field's content, not over-matching on
           the shared Int field or the record shape.")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert Map.empty #record((= f (+ x 1.25)) (= n 3)) 42)
            #record((= f 9.5) (= n 3)))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: -1 Int64)))

(case
  "a Set of structural records with a Float64 field dedups equal members by content"
  (doc
    "The Set companion: insert `(record (f (+ x 1.25)) (n 3))` (runtime float field) then `(record (f
           2.5) (n 3))` into a set → ONE element (`Set.len` 1), the two records sharing one canonical form.
           Pins that the CHAMP set dedup descends into the record's float field, the record companion of the
           tuple-float set-dedup case.")
  (input
    (do
      (def
        (main (: x Float64))
        (Set.len
          (Set.insert
            (Set.insert #set() #record((= f (+ x 1.25)) (= n 3)))
            #record((= f 2.5) (= n 3)))))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 1 Int64)))

; The tuple- and record-float-key cases above have the float leaf ONE level deep. A float leaf NESTED two
; levels deep — a tuple inside a tuple, or a tuple inside a record field — must ALSO key by content: the
; CHAMP hash/eq recurses through both compound layers to the float leaf. This declined on rust until
; v-rust-backend threaded the ord-wrapper through NESTED compound keys (f8e5b1c0d); now it keys by content
; on all three. These pin the nested path: tuple-in-tuple hit + MISS, record-field-of-tuple hit, and Set
; dedup. (A float in a SUM PAYLOAD key still declines on rust — per-variant threading, a later increment —
; so those stay out.)
(case
  "a nested tuple-in-tuple float key is a Map key found by a separately-built equal key"
  (doc
    "A doubly-nested compound key `(tuple (tuple Float64 Int64) Int64)`: insert under `(tuple (tuple
           (+ x 1.25) 3) 9)` at x=1.25 (a RUNTIME float leaf two levels deep) and look up with `(tuple
           (tuple 2.5 3) 9)` → 42. The CHAMP key hash/eq recurses through BOTH tuple layers to the float
           leaf and compares it by content. Pins the nested-compound-float-key path (rust emits it via the
           nested-threaded ord-wrapper, f8e5b1c0d), deeper than the one-level tuple-float-key case.")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert Map.empty #tuple(#tuple((+ x 1.25) 3) 9) 42)
            #tuple(#tuple(2.5 3) 9))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 42 Int64)))

(case
  "a nested tuple-in-tuple float key MISSES a Map key with a different deep float leaf"
  (doc
    "The negative control of the nested case: the outer/inner Int elements match but the deep float
           leaf differs — look up `(tuple (tuple 9.5 3) 9)` against a map keyed by `(tuple (tuple 2.5 3) 9)`
           → None (-1). Confirms the nested-key compare reaches the two-levels-deep float leaf's content,
           not over-matching on the shared Int elements or the compound shape.")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert Map.empty #tuple(#tuple((+ x 1.25) 3) 9) 42)
            #tuple(#tuple(9.5 3) 9))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: -1 Int64)))

(case
  "a record whose field is a tuple with a Float64 leaf is a Map key found by content"
  (doc
    "The mixed-nesting face: a RECORD whose field is a TUPLE carrying a float leaf, as a Map key —
           `(record (p (tuple Float64 Int64)) (n Int64))`. Insert under `(record (p (tuple (+ x 1.25) 3)) (n
           9))` and look up with `(record (p (tuple 2.5 3)) (n 9))` → 42. The CHAMP key hash/eq recurses
           record→tuple→float leaf, comparing by content across the mixed compound layers (record threaded
           over tuple, f8e5b1c0d). Pins the record-of-tuple-float nested key.")
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert Map.empty #record((= p #tuple((+ x 1.25) 3)) (= n 9)) 42)
            #record((= p #tuple(2.5 3)) (= n 9)))
          ((Some v) v)
          ((None _) -1)))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 42 Int64)))

(case
  "a Set of nested tuple-in-tuple float keys dedups equal members by content"
  (doc
    "The Set companion of the nested case: insert `(tuple (tuple (+ x 1.25) 3) 9)` (runtime deep float
           leaf) then `(tuple (tuple 2.5 3) 9)` into a set → ONE element (`Set.len` 1), the two doubly-nested
           tuples sharing one canonical form. Pins that the CHAMP set dedup recurses through both compound
           layers to the float leaf.")
  (input
    (do
      (def
        (main (: x Float64))
        (Set.len
          (Set.insert (Set.insert #set() #tuple(#tuple((+ x 1.25) 3) 9)) #tuple(#tuple(2.5 3) 9))))
      (export main)))
  (call main (: 1.25 Float64))
  (output (: 1 Int64)))

; --- CHAMP Set DEDUP follows the canonical FLOAT byte form (float-form × dedup intersection) ----------
; A Set dedups by hash+eq, and both must follow the SAME canonical byte form that scalar/compound `=`
; pins (03-equality NaN==NaN, -0.0 != +0.0). If the Set hashed/compared floats by IEEE == instead
; (nan != nan, -0.0 == +0.0), dedup would disagree with equality. Runtime float params (def args) keep
; the set off the const-fold path so the CHAMP heap dedup actually runs. NOTE: float-in-set currently
; declines on the RUST backend (a known coverage gap, same as the box-float insert case above) — these
; pin the WASM path.
(case
  "a set of two NaN floats dedups to one (canonical quiet-NaN)"
  (doc
    "CHAMP dedup follows the canonical float byte form: two runtime NaN elements both canonicalize
           to the one quiet-NaN (box-float), so `(Set.of (list nan nan))` has ONE element, `Set.len` = 1.
           IEEE == would treat nan != nan and keep both (len 2); the canonical byte form the scalar
           `nan == nan` case pins (03-equality) says one. Runtime Float64 params keep it off const-fold.")
  (input
    (do
      (def (build (: x Float64) (: y Float64)) (Set.len #set(x y)))
      (def (main (: d Int64)) (build Float64.nan Float64.nan))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a set of negative zero and positive zero keeps both (distinct sign bits)"
  (doc
    "The `-0.0 != +0.0` companion for CHAMP dedup: distinct sign bits are distinct canonical bytes,
           so `(Set.of (list -0.0 0.0))` keeps BOTH, `Set.len` = 2. IEEE == would treat -0.0 == +0.0 and
           dedup to 1; the canonical byte form the scalar `-0.0 != 0.0` case pins says two. Confirms Set
           dedup agrees with `=`, not with IEEE ==.")
  (input
    (do
      (def (build (: x Float64) (: y Float64)) (Set.len #set(x y)))
      (def (main (: d Int64)) (build -0.0 0.0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "a set of two identical positive floats dedups to one"
  (doc
    "The plain positive control: two identical runtime floats share canonical bytes, so
           `(Set.of (list 3.5 3.5))` dedups to ONE, `Set.len` = 1. Rules out an always-keep-both bug that
           would make the NaN case pass for the wrong reason.")
  (input
    (do
      (def (build (: x Float64) (: y Float64)) (Set.len #set(x y)))
      (def (main (: d Int64)) (build 3.5 3.5))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "Set.contains over negative zero does not find positive zero"
  (doc
    "`Set.contains` uses the same canonical hash+eq as dedup: a set holding -0.0 does NOT contain
           +0.0 (distinct canonical bytes) → false. The membership analogue of the -0.0 != +0.0 dedup
           case, pinning that contains and dedup share the float byte-form rule.")
  (input
    (do
      (def (test (: stored Float64) (: probe Float64)) (Set.contains #set(stored) probe))
      (def (main (: d Int64)) (if (test -0.0 0.0) 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "Set.contains over nan finds nan"
  (doc
    "The membership positive: a set holding a NaN DOES contain a NaN (both canonicalize to the one
           quiet-NaN) → true. The contains analogue of the NaN-dedup case.")
  (input
    (do
      (def (test (: stored Float64) (: probe Float64)) (Set.contains #set(stored) probe))
      (def (main (: d Int64)) (if (test Float64.nan Float64.nan) 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; --- A CONTEXT-TYPED empty float collection grounds its key/element wrapper WITHOUT a construction ----
; The rust backend represents a float key/element with a width-specific wrapper struct (`__CdzF64` /
; `__CdzF32`) whose comparison follows the canonical byte form (the wasm side needs no such struct — the
; canonicalization is in the heap ops). A CONTEXT-TYPED empty collection — `(: (Map.empty) (Map Float64
; Int64))` — annotates that wrapper type with NO constructor call, so the backend must inject the wrapper
; decl on the ANNOTATION (the type-param `<__CdzF64` position), not only on a `::new(` constructor; a gate
; that keyed on the constructor alone emitted a bare `BTreeMap<__CdzF64,_>` with no decl → rust "cannot
; find type `__CdzF64`". These pin the typed-empty float collection compiles and runs on BOTH backends
; (the divergence a decl-injection miss would produce is rust-only, so the wasm control matters), across
; Map key, a runtime insert onto the typed-empty map, a float Set element, and the narrow Float32 wrapper.
(case
  "a context-typed empty float map needs no key inserted to ground its wrapper type"
  (doc
    "`(: (Map.empty) (Map Float64 Int64))` is an empty float-keyed map grounded by its ANNOTATION, no
           insert — `Map.len` = 0. On rust the `__CdzF64` key wrapper is named in the map type-param with no
           constructor, so its decl must be injected on the annotation; a constructor-only gate would emit
           `BTreeMap<__CdzF64,_>` with no decl and fail to compile. Pins the typed-empty float map compiles
           and runs on both backends.")
  (input (do (def (main) (Map.len (: (Map.empty) (Map Float64 Int64)))) (export main)))
  (output (: 0 Int64)))

(case
  "a context-typed empty float map accepts a runtime float key insert"
  (doc
    "The construction companion: inserting a runtime `d : Float64` key into the typed-empty float map
           `(Map.insert (: (Map.empty) (Map Float64 Int64)) d 1)` → one entry, `Map.len` = 1. Pins the
           annotation-grounded wrapper agrees with the constructed-key path — the annotation and the insert
           name the SAME `__CdzF64` key type, so the decl injected for the annotation covers the insert.")
  (input
    (do
      (def (main (: d Float64)) (Map.len (Map.insert (: (Map.empty) (Map Float64 Int64)) d 1)))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 1 Int64)))

(case
  "a context-typed empty float set grounds its element wrapper type"
  (doc
    "The Set companion: `(: (Set.of (list)) (Set Float64))` is an empty float-element set grounded by
           its annotation — `Set.len` = 0. Pins the wrapper-decl injection covers a float Set ELEMENT type
           (`__CdzF64` in a set type-param), not only a Map key.")
  (input (do (def (main) (Set.len (: #set() (Set Float64)))) (export main)))
  (output (: 0 Int64)))

(case
  "a context-typed empty float32 map grounds the narrow-float wrapper"
  (doc
    "The narrow-width companion: `(: (Map.empty) (Map Float32 Int64))` grounds the `__CdzF32` wrapper
           (distinct from `__CdzF64`) — `Map.len` = 0. Pins the decl injection is width-specific and covers
           the Float32 wrapper too, the narrow-float dual of the Float64 typed-empty map.")
  (input (do (def (main) (Map.len (: (Map.empty) (Map Float32 Int64)))) (export main)))
  (output (: 0 Int64)))

; A collection construction whose element/key siblings INTERLEAVE a BigInt heap handle (i32) with GUARDED
; Int64 arithmetic (i64) must emit a VALID module. `(Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of
; (+ n 2))))` has a FIRST element that is a BigInt sum — its `bigint-add` operands stash i32 handles in
; scratch slots — and a SECOND element that boxes a checked `(+ n 2)` (an i64 overflow-guard temp). The
; `Set.of`/`Map.insert` emit arms used to lay every sibling at a FIXED scratch base, so the second element's
; i64 temp reused the slot number the first element's i32 handle had already typed → one wasm local declared
; at TWO widths → an invalid module (`expected i64, found i32`), rejected at load at every opt level. The fix
; advances each sibling's scratch floor past the running high-water (the disjoint-slot discipline tuples,
; records, and lists already applied). These pin the fix on both the Set constructor and the Map.insert twin.
(case
  "a set built from a BigInt sum and a BigInt.of over integer arithmetic has both elements"
  (doc
    "`(Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2))))` with n=5 holds the BigInt
           values 6 and 7 — two distinct elements, so Set.len = 2. The first element (a BigInt sum) stashes
           an i32 handle in a scratch slot; the second (a BigInt.of over a checked `(+ n 2)`) carries an i64
           overflow-guard temp — and the Set.of emit must keep them on disjoint slots or the wasm local is
           declared at two widths (invalid module). Reversed order, a plain list, and a bare `=` all worked;
           only the ordered [big-arith, of(i64-arith)] element pair inside a Set/Map build collided.")
  (input
    (do
      (def (main (: n Int64)) (Set.len #set((+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "a map built from a BigInt sum key and a BigInt.of-over-arithmetic key has both entries"
  (doc
    "The Map twin of the Set slot-clash guard: `(Map.insert (Map.insert (Map.empty) (+ (BigInt.of n)
           (BigInt.of 1)) 1) (BigInt.of (+ n 2)) 2)` with n=5 keys the BigInt values 6 and 7 — two distinct
           keys, so Map.len = 2. The first key is a BigInt sum (an i32 handle scratch); the second is a
           BigInt.of over a checked `(+ n 2)` (an i64 guard temp). The Map.insert emit must advance each
           sibling's scratch floor so the i32 key handle and the i64 arith temp never share one slot.")
  (input
    (do
      (def
        (main (: n Int64))
        (Map.len
          (Map.insert
            (Map.insert (Map.empty) (+ (BigInt.of n) (BigInt.of 1)) 1)
            (BigInt.of (+ n 2))
            2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

; The BigInt-key cases above pin the EMIT (slot-clash) face with SINGLE-limb values (6, 7 — fit an
; i64). These pin the HASH/COMPARE face at MULTI-limb magnitudes: `(* (BigInt.of n) (BigInt.of
; Int64.max))` products exceed one limb, so a CHAMP that hashed or compared only a truncated low limb
; (or the handle) would conflate keys sharing low bits and either dedup wrongly or miss lookups. The
; distinct-keys, dedup, and lookup-hit/miss faces each witness the full-value discipline.
(case
  "multi-limb BigInt map keys stay distinct and look up by full value"
  (doc
    "`big n = n · Int64.max` is a genuine MULTI-limb BigInt (≥ 2^63). Keys `big 2` and `big 3`
           must be two distinct entries (len 2), the lookup `big a` at a=2 hits (20), and at a=5 — a
           multi-limb value inserted never — misses cleanly (-1). A hash over a truncated low limb or a
           compare over the handle would break one of the three faces. Encodes 100·len + hit/miss.
           Expected: 220 (a=2), 199 (a=5).")
  (input
    (do
      (def (big (: n Int64)) (* (BigInt.of n) (BigInt.of 9223372036854775807)))
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert (Map.insert Map.empty (big 2) 20) (big 3) 30)))
          (+ (* 100 (Map.len m)) (match (Map.lookup m (big a)) ((Some v) v) ((None u) -1)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 220 Int64))
  (call main (: 5 Int64))
  (output (: 199 Int64)))

(case
  "multi-limb BigInt set elements deduplicate by full value"
  (doc
    "The Set twin: `{big a, big 2, big 3}` at a=2 collapses the repeated multi-limb value (len 2);
           at a=7 all three are distinct (len 3). Dedup must compare the FULL multi-limb magnitude —
           values sharing low-limb bits (all are Int64.max multiples) conflate under a truncated hash
           only if the full compare also fails, so the pair of calls witnesses both the collision path
           and the equality walk. Expected: 2 (a=2), 3 (a=7).")
  (input
    (do
      (def (big (: n Int64)) (* (BigInt.of n) (BigInt.of 9223372036854775807)))
      (def (main (: a Int64)) (Set.len #set((big a) (big 2) (big 3))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 7 Int64))
  (output (: 3 Int64)))

; The `Set.remove`/`Map.remove` twins of the disjoint-slot guard above. Unlike the constructor/insert arms
; (fixed by the sibling-scratch pass), the two REMOVE arms laid BOTH operands at a fixed `base + 1` — so a
; `remove` whose collection operand is a recursive call (an i32 list/set handle in a `dup` slot) and whose
; key/element is a checked `(+ v 1)` (an i64 overflow-guard temp) re-typed one wasm local to two widths →
; `expected i32, found i64`, a check-clean/compile-invalid MISCOMPILE (rejected at load at every opt level).
; The fix floats the key/element operand's scratch past the collection operand's high-water (the owned-drop
; tee stays at `base`, below both). These pin the fix on the last two compound-op arms with a fixed base.
(case
  "a Set.remove of a checked-arith element behind a recursive-call set is disjoint-slotted"
  (doc
    "`(Set.remove (g t) (+ v 1))` removes a CHECKED-ARITH element `(+ v 1)` (an i64 overflow-guard
           scratch temp) from the result of a RECURSIVE call `(g t)` (an i32 set handle in a `dup` slot).
           The set operand and the element operand need DISJOINT scratch slots — the `SetRemove` emit arm
           laid both at a fixed `base + 1`, so the i64 arith temp reused the i32 handle's slot number → one
           wasm local at two widths, an invalid module. `(g (ICons 5 (INil)))` removes 5+1=6 from the empty
           set (a no-op — 6 was never inserted) → Set.len 0. Companion to the `Set.of` disjoint-slot pin
           above and the `Map.remove` twin below; the last two fixed-base compound-op arms.")
  (input
    (do
      (type ILst (INil) (ICons Int64 ILst))
      (def (g xs) (match xs ((INil) #set()) ((ICons v t) (Set.remove (g t) (+ v 1)))))
      (def (main) (Set.len (g (ICons 5 (INil)))))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a Set.remove of a checked-arith element that is PRESENT drops exactly it (disjoint-slot value-parity)"
  (doc
    "Value-parity companion to the Set.remove disjoint-slot pin above: the two pins there observe a
           NO-OP removal (the checked-arith element is ABSENT → len unchanged), so they witness only that the
           module LOADS, not that disjoint-slotting removed the RIGHT element. Here the recursive base case
           holds {6, 9}, and `(Set.remove (g t) (+ v 1))` removes `5+1=6` — an element that is PRESENT —
           leaving {9}. A slot-collision that corrupted the i64 element temp (the miscompile the fix closed)
           would remove the wrong element (or none), so 6 would survive. Observed via membership:
           `(+ (* 10 contains-6) contains-9)` = 0*10 + 1 = 1 pins that exactly 6 was dropped and 9 survived.")
  (input
    (do
      (type ILst (INil) (ICons Int64 ILst))
      (def (g xs) (match xs ((INil) #set(6 9)) ((ICons v t) (Set.remove (g t) (+ v 1)))))
      (def
        (main)
        (let
          ((r (g (ICons 5 (INil)))))
          (+ (* 10 (if (Set.contains r 6) 1 0)) (if (Set.contains r 9) 1 0))))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a Map.remove of a checked-arith key behind a recursive-call map is disjoint-slotted"
  (doc
    "The Map twin of the `Set.remove` disjoint-slot pin above: `(Map.remove (g t) (+ v 1))` removes a
           CHECKED-ARITH key `(+ v 1)` (an i64 overflow-guard scratch temp) from the result of a RECURSIVE
           call `(g t)` (an i32 map handle in a `dup` slot). The `MapRemove` emit arm laid both operands at
           a fixed `base + 1`, colliding the i32 handle slot with the i64 arith temp → invalid wasm. `(g
           (ICons 5 (INil)))` removes key 5+1=6 from the base one-entry map `{0:0}` (a no-op — 6 is absent)
           → Map.len stays 1. The `remove`-arm companions to the `Set.of`/`Map.insert` disjoint-slot pins.")
  (input
    (do
      (type ILst (INil) (ICons Int64 ILst))
      (def
        (g xs)
        (match xs ((INil) (Map.insert Map.empty 0 0)) ((ICons v t) (Map.remove (g t) (+ v 1)))))
      (def (main) (Map.len (g (ICons 5 (INil)))))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a Map.remove of a checked-arith key that is PRESENT drops exactly it (disjoint-slot value-parity)"
  (doc
    "Value-parity companion to the Map.remove disjoint-slot pin above: that pin observes a NO-OP removal
           (the checked-arith key is ABSENT → len unchanged), witnessing only module validity, not that
           disjoint-slotting removed the RIGHT key. Here the recursive base holds `{6:60, 9:90}` and
           `(Map.remove (g t) (+ v 1))` removes key `5+1=6` — PRESENT — leaving `{9:90}`. A slot-collision
           that corrupted the i64 key temp (the miscompile the fix closed) would remove the wrong key (or
           none), so 6 would survive. Observed via lookup: `(+ (* 100 lookup-6) lookup-9)` where a missing key
           reads -1 → 100*(-1) + 90 = -10 pins that exactly key 6 was dropped and key 9 survived.")
  (input
    (do
      (type ILst (INil) (ICons Int64 ILst))
      (def
        (g xs)
        (match
          xs
          ((INil) (Map.insert (Map.insert Map.empty 6 60) 9 90))
          ((ICons v t) (Map.remove (g t) (+ v 1)))))
      (def
        (main)
        (let
          ((r (g (ICons 5 (INil)))))
          (+
            (* 100 (match (Map.lookup r 6) ((Some v) v) ((None u) -1)))
            (match (Map.lookup r 9) ((Some v) v) ((None u) -1)))))
      (export main)))
  (output (: -10 Int64))
  (live-objects 0))

; `Set.union`/`Set.intersection`/`Set.difference` (`Core::SetAlgebra`) emit BOTH set operands at a SHARED
; scratch `base` (select.rs:7755, `emit(lhs, base); emit(rhs, base)`) — the SAME fixed-base pattern that
; miscompiled the `Set.remove`/`Map.remove` arms above. This arm is IMMUNE to that width-collision class,
; though: each operand leaves its RESULT on the wasm stack and nothing is tee'd INTO `base` that must survive
; the sibling operand's emit, so no wasm local is ever re-typed at two widths. This pins that immunity — an
; operand carrying an i32 handle scratch (a recursive-call set) unioned with one carrying an i64 checked-arith
; temp (`Set.of` over `(+ n k)`) must compile and run, so a future rewrite that DID stash across `base` (the
; way the remove arms did) would flip this to invalid-module and be caught.
(case
  "a Set.union of a recursive-call set and a Set.of over checked-arith is disjoint-slotted"
  (doc
    "`(Set.union (g …) (Set.of (list (+ n 2) (+ n 3))))`: the lhs is a RECURSIVE call `(g …)` (an i32
           set handle in a `dup` slot), the rhs a `Set.of` whose elements are checked `(+ n k)` (i64
           overflow-guard temps). Both go through the `SetAlgebra` emit at a shared `base`, yet the arm is
           immune to the fixed-base width-collision (each operand leaves its result on the stack, nothing is
           stashed into `base` across the sibling). With n=50: `{6}` ∪ `{52, 53}` = three distinct elements,
           so Set.len = 3. Regression guard for the last shared-`base` compound arm not covered by the
           `remove` disjoint-slot pins above.")
  (input
    (do
      (type ILst (INil) (ICons Int64 ILst))
      (def (g xs) (match xs ((INil) #set()) ((ICons v t) (Set.insert (g t) (+ v 1)))))
      (def (main (: n Int64)) (Set.len (Set.union (g (ICons 5 (INil))) #set((+ n 2) (+ n 3)))))
      (export main)))
  (call main (: 50 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; ---- a runtime Bytes.slice VIEW as a CHAMP key must key by CONTENT, not by the view node --------------
; A runtime-start `Bytes.slice` produces a borrowed [off,len] VIEW over its parent. Used as a CHAMP key
; (Map key or Set member, either side of the lookup) it must hash + compare by its FLATTENED content, so
; it hits an equal-content flat Bytes — the equal-means-same-key contract. This missed on wasm (breaker
; finding #16): the rust emit's `key_needs_compaction` only compacted an OWNED String/Bytes key, so a
; BORROWED rope/slice key skipped the key-site bytes-compact and reached `champ_hash` as a raw view →
; hashed differently → lookup MISSED while value-`=` said EQUAL. Fixed (rcdzc `900a8ff3b`) by compacting
; ANY-ownership at the key site (bytes-compact is refcount-neutral, safe for a borrow). A CONST-start
; slice always worked (it compacts at fold). These pin every face: value-eq control, slice-probes-flat,
; slice-stored-flat-probes, Set membership, and the const-start control.
(case
  "value-eq CONTROL: a runtime slice compares equal to a flat Bytes of the same content"
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s) (if (= s (Bytes.of #list(20 30))) 1 0))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime slice PROBING a Map keyed by flat Bytes must hit by content"
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty (Bytes.of #list(20 30)) 42)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
            ((Some s) (match (Map.lookup m s) ((Some v) v) ((None u) -1)))
            ((None u) -2))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a runtime slice STORED as a Map key must be found by a flat Bytes probe"
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s)
            (match
              (Map.lookup (Map.insert Map.empty s 42) (Bytes.of #list(20 30)))
              ((Some v) v)
              ((None u) -1)))
          ((None u) -2)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a runtime slice probes a Set of flat Bytes by content"
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s) (if (Set.contains #set((Bytes.of #list(20 30))) s) 1 0))
          ((None u) -2)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "CONTROL: a CONST-start slice as a Map-lookup key hits on wasm"
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty (Bytes.of #list(20 30)) 42)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) 1 2)
            ((Some s) (match (Map.lookup m s) ((Some v) v) ((None u) -1)))
            ((None u) -2))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

; --- The #16 fix's PERIMETER: view-key canonicalization at EVERY champ entry point ------------------
; The canonicalization pins above cover lookup (both directions), contains, and value-eq. These pin the
; REMAINING champ entry points a view key reaches — batch build (Set.of), the plain remove, and the
; value-yielding take — so a rework that moved canonicalization from the shared key seam into per-op
; call sites cannot silently miss one.
(case
  "a slice-view SET element dedups against its flat twin in a Set.of batch build"
  (doc
    "`(Set.of (list s flat))` where `s` is a runtime-start slice view of equal content: the batch
           build's element canonicalization collapses the two to ONE slot (a=1 windows (20,30) = flat →
           len 1); a different window stays distinct (a=0 → len 2). The batch-build face of the champ
           canonicalization — insert-path pins can't witness Set.of's distinct construction route.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s) (Set.len #set(s (Bytes.of #list(20 30)))))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "Set.union dedups a slice-view element against a flat twin ACROSS the operand boundary"
  (doc
    "The set-ALGEBRA face of the view canonicalization (the entry-point pins cover per-element
           routes): one operand holds the runtime VIEW, the other its FLAT twin — the union's cross-trie
           merge must recognize them as one element. a=1 (view = (20,30)) → {view/flat, (1,2)} len 2;
           a=0 (view = (9,20)) → 3 distinct. A merge comparing view nodes structurally would keep both
           and answer 3 at a=1.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s)
            (Set.len (Set.union #set(s) #set((Bytes.of #list(20 30)) (Bytes.of #list(1 2))))))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "Set.intersection and Set.difference match a view against a flat element across operands"
  (doc
    "The remaining two algebra ops in one case: intersection of {view, (1,2)} with {flat (20,30)}
           finds the overlap exactly when the view's window equals the flat (1 at a=1, 0 at a=0), and
           difference of {(20,30), (1,2)} minus {view} removes the matched flat (1 at a=1, 2 at a=0 —
           encoded 10·inter + diff read as separate calls below). Both merges walk different tries per
           operand role; the canonicalization must hold on whichever side the view sits.")
  (input
    (do
      (def
        (main (: a Int64) (: which Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s)
            (if
              (= which 0)
              (Set.len
                (Set.intersection #set(s (Bytes.of #list(1 2))) #set((Bytes.of #list(20 30)))))
              (Set.len (Set.difference #set((Bytes.of #list(20 30)) (Bytes.of #list(1 2))) #set(s)))))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 1 Int64) (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 1 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "Map.remove by a slice-view key drops the flat-keyed entry"
  (doc
    "The remove face: `(Map.remove {flat↦42} s)` with the view key — a=1 (equal content) removes
           the entry (len 0); a=0 (different window) no-ops (len 1, removal total). A remove path that
           hashed the raw view node would miss the hit and leave a phantom entry.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty (Bytes.of #list(20 30)) 42)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
            ((Some s) (Map.len (Map.remove m s)))
            ((None u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "Map.take by a slice-view key yields the flat-keyed value"
  (doc
    "The value-yielding-remove face: `(Map.take {flat↦42} s)` — the hit binds `(Some 42)` with an
           empty rest (42+0); the miss binds `(None unit)` with the map intact (-1). Completes the champ
           entry-point sweep for view keys: lookup, contains, Set.of, remove, take all canonicalize.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty (Bytes.of #list(20 30)) 42)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
            ((Some s)
              (match
                (Map.take m s)
                (#tuple((Some v) rest) (+ v (Map.len rest)))
                (#tuple((None u) rest) (- 0 (Map.len rest)))))
            ((None u) -99))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  ; per-call (B2): coarse whole-case known-leak 3 matched only call 0; call 1 reclaims one more. true vector: 3/2.
  ; (v-memory-safety re-baseline, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "a float-field RECORD as a SET element dedups by content including the float leaf"
  (doc
    "The float-leaf record as a champ SET element (closed with the slice-canonicalization work —
           it used to decline): `{(record (f x) (n 1)), (record (f 2.5) (n 1))}` — x=0.5 keeps both (len
           2), x=2.5 collapses them (len 1). The element hash walks the record's float field by canonical
           bytes exactly as the map-KEY twin (pinned above) does.")
  (input
    (do
      (def (main (: x Float64)) (Set.len #set(#record((= f x) (= n 1)) #record((= f 2.5) (= n 1)))))
      (export main)))
  (call main (: 0.5 Float64))
  (output (: 2 Int64))
  (call main (: 2.5 Float64))
  (output (: 1 Int64)))

(case
  "a NESTED float tuple as a SET element dedups by deep content"
  (doc
    "The depth-2 companion: `(tuple (tuple x 1) 2)` elements — the float sits two levels down, so
           the element hash must descend both tuple layers to the canonical float bytes. x=0.5 vs 2.5
           distinct (len 2 via insert-insert); x=2.5 collapses (len 1).")
  (input
    (do
      (def
        (main (: x Float64))
        (Set.len (Set.insert (Set.insert #set() #tuple(#tuple(x 1) 2)) #tuple(#tuple(2.5 1) 2))))
      (export main)))
  (call main (: 0.5 Float64))
  (output (: 2 Int64))
  (call main (: 2.5 Float64))
  (output (: 1 Int64)))

(case
  "Set.of over Rational elements dedupes a normalized-equal literal and to-list enumerates ascending"
  (doc
    "The RATIONAL sibling of the Float64 to-list case: 19-sets had no Rational element face at all.
           `(Set.of (list 3/2 1/2 n/4))` at n=2 dedupes the normalized-equal 2/4 against 1/2 (len 2) and
           `Set.to-list` enumerates ascending by value — encoded per element as `(Rational.truncate (* r 2))`
           so 1/2 -> 1 and 3/2 -> 3 (213). n=1 keeps 1/4 distinct (301); n=6 normalizes 6/4 -> 3/2 (213
           again). A set path that hashed the as-written num/den pair would count three elements.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def xs (Set.to-list #set((Rational.of 3 2) (Rational.of 1 2) (Rational.of n 4))))
          (def
            a
            (match
              (List.at xs 0)
              ((Some r) (Rational.truncate (* r (Rational.of 2 1))))
              ((None _u) -99)))
          (def
            b
            (match
              (List.at xs 1)
              ((Some r) (Rational.truncate (* r (Rational.of 2 1))))
              ((None _u) -99)))
          (+ (* 100 (List.len xs)) (+ (* 10 a) b))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 213 Int64))
  (call main (: 1 Int64))
  (output (: 301 Int64))
  (call main (: 6 Int64))
  (output (: 213 Int64))
  (live-objects known-leak))

(case
  "a negative-denominator Rational sign-normalizes on the set-element path and dedupes its positive-denominator twin"
  (doc
    "The SIGN axis of Rational canonicalization on the set-element path (06-numeric pins it on the
           ARITHMETIC path only): `(Rational.of 1 (- 0 n))` builds a runtime NEGATIVE-denominator rational
           that must sign-normalize (1/-2 -> -1/2) and dedupe against its positive-denominator twin -1/2
           (n=2: len 2 -> 21). n=4 stays distinct as -1/4 (31); n=1 integer-normalizes 1/-1 -> -1 (31).
           The contains probe uses `-2/4`, stacking magnitude- on sign-normalization.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def s #set((Rational.of 1 (- 0 n)) (Rational.of -1 2) (Rational.of 1 2)))
          (+ (* 10 (Set.len s)) (if (Set.contains s (Rational.of -2 4)) 1 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 21 Int64))
  (call main (: 4 Int64))
  (output (: 31 Int64))
  (call main (: 1 Int64))
  (output (: 31 Int64)))

(case
  "Set.to-list over Rational elements enumerates ascending by value, not by numerator/denominator pair"
  (doc
    "Pins WHICH order `Set.to-list` enumerates rationals in: by VALUE, not by (numerator, denominator)
           lexicographic — the two genuinely diverge on {2/3, 1/3, 1/2}: value-ascending is 1/3 < 1/2 < 2/3
           (digits 2,3,4 via trunc(6r) -> 234) while lex would be 1/2 < 1/3 < 2/3 (324). The n=1 row is the
           lex-killer: 1/1 integer-normalizes to 1 and sorts LAST despite numerator 1 (346).")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def xs (Set.to-list #set((Rational.of 2 3) (Rational.of 1 n) (Rational.of 1 2))))
          (def
            (six (: i Int64))
            (match
              (List.at xs i)
              ((Some r) (Rational.truncate (* r (Rational.of 6 1))))
              ((None _u) -9)))
          (+ (* 100 (six 0)) (+ (* 10 (six 1)) (six 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 234 Int64))
  (call main (: 5 Int64))
  (output (: 134 Int64))
  (call main (: 1 Int64))
  (output (: 346 Int64))
  (live-objects known-leak))

(case
  "Set.intersection unifies an arithmetic-produced rational with its constructor-built normalized twin"
  (doc
    "Set.intersection across CONSTRUCTION paths: set `a` holds 1/2 produced by ARITHMETIC
           (`(+ 1/4 1/4)`, canonicalized in the add op); set `b` holds 1/2 built by the CONSTRUCTOR
           (`(Rational.of 2 4)`, canonicalized at construction). A lazily- or inconsistently-normalizing
           implementation splits exactly here and reports an empty intersection. n=2: len 1 + contains 1/2
           -> 11; the n=1 (1/4) and n=6 (3/2) rows are disjoint controls -> 0.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def a #set((+ (Rational.of 1 4) (Rational.of 1 4)) (Rational.of 5 3)))
          (def b #set((Rational.of n 4) (Rational.of 7 3)))
          (def i (Set.intersection a b))
          (+ (* 10 (Set.len i)) (if (Set.contains i (Rational.of 1 2)) 1 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 11 Int64))
  (call main (: 1 Int64))
  (output (: 0 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "MULTI-LIMB BigInts built by three arithmetic routes hash to one set element"
  (doc
    "Structural sharing can't answer this: v1 = 2^62·6, v2 = 2^62·2 + 2^62·4 + k, and the contains
           probe 2^61·12 all build 27670116110564327424 through DIFFERENT limb arithmetic. At k=0 the set
           dedupes them to ONE element and the third-route probe hits (11) — content hashing over the
           limb VALUE, not the construction path. k=±1 separates v2 by one unit in the LOW limb of a
           two-limb value (21): a hash that ignored low-limb bits or an eq that compared only limb
           COUNTS would still collapse them.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def v1 (* (BigInt.of 4611686018427387904) (BigInt.of 6)))
          (def
            v2
            (+
              (+
                (* (BigInt.of 4611686018427387904) (BigInt.of 2))
                (* (BigInt.of 4611686018427387904) (BigInt.of 4)))
              (BigInt.of k)))
          (def s #set(v1 v2))
          (+
            (* 10 (Set.len s))
            (if (Set.contains s (* (BigInt.of 2305843009213693952) (BigInt.of 12))) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: -1 Int64))
  (output (: 21 Int64)))

(case
  "Map.remove and Map.lookup by different-route MULTI-LIMB keys agree with the inserted key"
  (doc
    "The MAP twin of the multi-limb set-dedupe face, sharpened to REMOVE: insert 42 under
           2^62·6, remove by 2^61·12, look up by 2^62·2 + 2^62·4 — three limb-arithmetic routes to
           27670116110564327424. mode=1: the remove must find the slot (len 0) and the lookup miss
           (-1); a remove whose key-eq diverged from champ_hash's canonical view leaves a phantom
           entry (52 here). mode=2 skips the remove: len 1 + lookup hits through the third route (52).")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def m (Map.insert Map.empty (* (BigInt.of 4611686018427387904) (BigInt.of 6)) 42))
          (def
            m2
            (if (= mode 1) (Map.remove m (* (BigInt.of 2305843009213693952) (BigInt.of 12))) m))
          (+
            (* 10 (Map.len m2))
            (match
              (Map.lookup
                m2
                (+
                  (* (BigInt.of 4611686018427387904) (BigInt.of 2))
                  (* (BigInt.of 4611686018427387904) (BigInt.of 4))))
              ((Some v) v)
              ((None _u) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 2 Int64))
  (output (: 52 Int64)))

(case
  "a SET as a set element hashes by content: three build orders, one element"
  (doc
    "Set-of-sets pins that champ_hash/champ_eq over a set ELEMENT are content-based, not
           layout-based: {1,2,3} built by literal `Set.of`, by an insert chain seeded EMPTY (n,2,1 at
           n=3), and by a THIRD order for the contains probe (2,1,3) must be ONE element found by ANY
           spelling — a CHAMP whose internal node shape varies with insertion order would split them
           under a structural (layout) hash. n=3: len 1 + contains (11); n=4: {1,2,4} is a genuinely
           different set — len 2, and the {1,2,3} probe still finds the literal (21).")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def i1 #set(1 2 3))
          (def i2 (Set.insert (Set.insert (Set.insert #set() n) 2) 1))
          (def s #set(i1 i2))
          (+ (* 10 (Set.len s)) (if (Set.contains s (Set.insert (Set.insert #set(2) 1) 3)) 1 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 11 Int64))
  (call main (: 4 Int64))
  (output (: 21 Int64)))

(case
  "Map.to-list over Rational keys enumerates ascending by key value with the integer form last"
  (doc
    "The MAP twin of the rational set-enumeration pin: entries inserted in the order 2/3 -> 1,
           1/n -> 2, 1/2 -> 3; `Map.to-list` must enumerate by KEY VALUE, so the projected values read
           out in key-ascending order. n=3: keys 1/3 < 1/2 < 2/3 give 231 — a (numerator, denominator)
           lexicographic order would give 321, and INSERTION order 123. n=1: `(Rational.of 1 1)`
           integer-normalizes to 1, which sorts LAST despite numerator 1 (312) — the lex-killer row.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def
            m
            (Map.insert
              (Map.insert (Map.insert Map.empty (Rational.of 2 3) 1) (Rational.of 1 n) 2)
              (Rational.of 1 2)
              3))
          (def xs (Map.to-list m))
          (def (vat (: i Int64)) (. (Option.expect (List.at xs i) "in bounds") 1))
          (+ (* 100 (vat 0)) (+ (* 10 (vat 1)) (vat 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 231 Int64))
  (call main (: 1 Int64))
  (output (: 312 Int64))
  (live-objects known-leak))

(case
  "a String slice VIEW keys a map by content in both directions, rope-backed included"
  (doc
    "The STRING face of the slice-view-as-CHAMP-key family (the Bytes faces :2361/:2374 pin the
           borrowed-view compaction from the key_needs_compaction finding): mode 1 PROBES {\"key\"->42}
           with the view `slice(\"xkeyz\",1,4)`; mode 2 STORES the view as the key and probes with the
           flat literal; mode 3 stores a view of the ROPE `concat(\"xk\",\"eyz\")` — seam inside the
           window — and probes flat. All 42. mode 4 probes with the WRONG window [0,3) = \"xke\" (-1).
           A key path that hashed the view node (offset/parent) or skipped compaction on the STRING
           branch misses where Bytes hits.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def v (Option.expect (String.slice "xkeyz" 1 4) "in"))
          (def
            rv
            (Option.expect
              (String.slice (String.concat "xk" (if (> mode 1000) "zzz" "eyz")) 1 4)
              "in"))
          (if
            (= mode 1)
            (match (Map.lookup (Map.insert Map.empty "key" 42) v) ((Some x) x) ((None _u) -1))
            (if
              (= mode 2)
              (match (Map.lookup (Map.insert Map.empty v 42) "key") ((Some x) x) ((None _u) -1))
              (if
                (= mode 3)
                (match (Map.lookup (Map.insert Map.empty rv 42) "key") ((Some x) x) ((None _u) -1))
                (match
                  (Map.lookup
                    (Map.insert Map.empty "key" 42)
                    (Option.expect (String.slice "xkeyz" 0 3) "in"))
                  ((Some x) x)
                  ((None _u) -1)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 2 Int64))
  (output (: 42 Int64))
  (call main (: 3 Int64))
  (output (: 42 Int64))
  (call main (: 4 Int64))
  (output (: -1 Int64))
  ; mode 3 (view over a RUNTIME rope — concat with an if-branch operand — stored as CHAMP key) leaks 2:
  ; the slice-view does not retain/reclaim its backing runtime rope (String-family concat/rope-intermediate
  ; reclaim class, sumexpect_view_reclaim sub-gap; routed to v-core-opt, queued as a view/backing co-design
  ; arc behind glb1). Modes 1/2/4 reclaim clean: 2's flat-literal backing is immortal; 1/4 borrow-probe.
  ; Flips back to 0 when the view-owns-runtime-backing reclaim lands. (v-memory-safety, coord v-corpus-harness)
  (live-objects known-leak))

(case
  "Set.to-list over STRING elements orders by content with a rope participating"
  (doc
    "The STRING sibling of the Float64/compound to-list pins (and the ruled-decline Bytes face):
           string elements DO have a blessed order (content-lexicographic, 13-strings:53), so a set of
           {\"b\", rope, \"c\"} must enumerate ascending with the RUNTIME rope compared by content —
           mode 1 builds \"aa\" (concat \"a\"+\"a\") which sorts FIRST (e0 = \"aa\": 11); mode 0
           builds \"az\" which still sorts before \"b\" but is not \"aa\" (1). A sort that compared
           rope nodes structurally (or leaf-first) instead of by content misorders mode 0.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def r (String.concat "a" (if (> mode 0) "a" "z")))
          (def xs (Set.to-list #set("b" r "c")))
          (def (at (: i Int64)) (Option.expect (List.at xs i) "in"))
          (+ (* 10 (if (= (at 0) "aa") 1 0)) (if (= (at 2) "c") 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "tuple set elements order by string content across reps then the scalar tiebreak"
  (doc
    "The TUPLE leg of the mixed-rep content-order family (list leg banked alongside; the tuple
           orderable arm came with the compound-element sort fixes): {(view \"key\", 1), (rope, 2)} —
           mode 1 the rope is \"key\": first fields CONTENT-EQUAL across view/rope reps, so the sort
           falls to the Int tiebreak and (\"key\",1) enumerates first (second field 1). mode 0 the rope
           is \"kex\" < \"key\": (\"kex\",2) sorts first (2). A tuple compare that ranked the string
           field by rep identity never reaches the tiebreak and flips mode 1.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def a #tuple((Option.expect (String.slice "xkeyz" 1 4) "in") 1))
          (def b #tuple((String.concat "ke" (if (> mode 0) "y" "x")) 2))
          (def xs (Set.to-list #set(a b)))
          (match (List.at xs 0) ((Some t) (. t 1)) ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "one string content hashes identically from flat, rope, and view reps in one program"
  (doc
    "The TRIPLE-rep completeness witness the pairwise pins imply but never run together: ONE
           map keyed by the flat literal, probed by the ROPE and the VIEW; ONE set built from the
           rope, probed by the view and the flat literal — four digits, every cross-rep pair through
           champ_hash/eq in a single program (1111). Adds a miss row (mode 1 probes with rope
           \"kez\"): 0100 → 100. If ANY rep's hash normalized differently (rope leaf-wise, view
           parent-wise, flat direct), one digit drops — the transitivity of content identity across
           all three reps is exactly what a per-rep hash cache would break.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def m (Map.insert Map.empty "key" 42))
          (def rope (String.concat "ke" (if (= mode 1) "z" "y")))
          (def view (Option.expect (String.slice "xkeyz" 1 4) "in"))
          (def s #set(rope))
          (+
            (* 1000 (match (Map.lookup m rope) ((Some v) 1) ((None _u) 0)))
            (+
              (* 100 (match (Map.lookup m view) ((Some v) 1) ((None _u) 0)))
              (+ (* 10 (if (Set.contains s view) 1 0)) (if (Set.contains s "key") 1 0))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1111 Int64))
  (call main (: 1 Int64))
  (output (: 100 Int64)))

; Set.to-list over FLOAT-LEAF TUPLE elements DECLINES — a compound containing a float leaf offers no
; blessed total order (§319; the float-axis companion of 03-equality:626's bare `< float-tuple`
; decline, and the Set<Bytes> unordered ruling). Regression (breaker #34, 5 faces): wasm silently
; returned an EMPTY list (Set.len 3, to-list []) while rust ENUMERATED (both wrong) — the compound
; orderable-descriptor propagated float_ok into the compound arms. Fixed uniform-decline: v-wasm-opt
; 42b2a02b0 (recurse compound arms float_ok=false) + v-rust-backend to-list-only guard (construction/
; contains/remove still work — pin-211 honored). Concierge RULED (a) uniform decline. bare-float sets
; + int-leaf tuples still enumerate (unregressed).
(case
  "Set.to-list over float-leaf tuple elements is a coded CDZ0203 — a float-containing compound offers no total order (§319, 03:626 companion)"
  (input
    (do
      (def (main) (List.len (Set.to-list #set(#tuple(1.5 1) #tuple(2.5 2) #tuple(-1.0 3)))))
      (export main)))
  (error CDZ0203 (message "IEEE partial order")))

; The SET/MAP-leaf sibling of the float-leaf to-list decline above (fuzzer cdz-smith). An element whose
; type carries a SET or MAP leaf — here a `List (Map …)` — has NO blessed total order either, so its ordered
; enumeration is undefined: a coded CDZ0203, ALL-LEAF with the float case (the no-total-order family the
; ordering #7143 + compare #7210 reconcile unified — float AND set/map → one code). Formerly wasm declined
; CODELESS ("no orderable descriptor") while the rust backend ENUMERATED via `BTreeSet`'s `Ord` (an order the
; spec does not bless) — a backend divergence. Now declined in the shared front-end (`lower_set_to_list`), so
; both backends + `cdz check` agree. Construction of the set still works; only the ordered to-list declines.
(case
  "Set.to-list over set/map-leaf elements is a coded CDZ0203 — a set/map leaf carries no blessed total order (all-leaf sibling of the float case)"
  (input
    (do
      (def (main) (List.len (Set.to-list #set((Map.insert Map.empty 0 #list(8 7))))))
      (export main)))
  (error CDZ0203 (message "no blessed order")))

(case
  "Map.to-list orders by SYMBOL keys while float values — NaN included — ride along"
  (doc
    "The values-need-no-order guard for the float-in-compound enumeration seam: the map's KEYS
           are symbols (blessed order) and its VALUES are floats, one a computed NaN — to-list must
           order by the keys and carry the float values untouched (len 2, #\"a\" first → 21; the
           NaN value reads back as the canonical NaN: `(= v v)` is TRUE under canonical-byte equality
           → 211). An enumeration that consulted the VALUE type for orderability (or hashed the
           entry as a whole) would decline or empty a map whose values happen to be floats — only
           the KEY needs an order.")
  (input
    (do
      (def
        (main (: x Float64))
        (do
          (def m (Map.insert (Map.insert Map.empty #"b" 2.5) #"a" (/ x x)))
          (def xs (Map.to-list m))
          (+
            (* 100 (List.len xs))
            (+
              (* 10 (match (List.at xs 0) ((Some e) (if (= (. e 0) #"a") 1 0)) ((None _u) -1)))
              (match (List.at xs 0) ((Some e) (if (= (. e 1) (. e 1)) 1 0)) ((None _u) -1))))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 211 Int64)))

(case
  "hash-only set ops keep working on float-leaf tuples that have no blessed order"
  (doc
    "The ORDER/HASH split guard for the float-compound enumeration ruling: contains, remove and
           len need only champ_hash/eq — NO total order — so a set of float-leaf tuples must keep
           supporting them even while to-list declines (float compounds have no blessed order,
           03:626). len 2 + contains by the CANONICAL NaN spelling of a computed-NaN element + remove
           of the other element → 211. A fix that declined float-tuples at set-CONSTRUCTION (or
           routed contains through the orderable descriptor) would break membership where only
           ENUMERATION is unordered.")
  (input
    (do
      (def
        (main (: x Float64))
        (do
          (def s #set(#tuple(1.5 1) #tuple((/ x x) 2)))
          (+
            (* 100 (Set.len s))
            (+
              (* 10 (if (Set.contains s #tuple(Float64.nan 2)) 1 0))
              (Set.len (Set.remove s #tuple(1.5 1)))))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 211 Int64)))

; --- Set batch: the set-of-sets content hash, rope-vs-flat elements across algebra operands,
; and the Float32 canonical-byte enumeration order. ---
(case
  "a SET of SETS dedups elements by set CONTENT — insertion-order twins collapse"
  (doc
    "The CHAMP-hash-of-a-CHAMP face: {1,k} and {k,1} built in OPPOSITE insertion orders must hash identically for the outer dedup to collapse them — a hash over internal trie layout instead of canonical content splits the twins; the {2,1} membership probe (a third insertion order) must hit.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def s1 #set(1 k))
          (def s2 #set(k 1))
          (def s3 #set(9))
          (def nested #set(s1 s2 s3))
          (+ (* 10 (Set.len nested)) (if (Set.contains nested #set(2 1)) 1 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 21 Int64)))

(case
  "set algebra unifies a rope String element with its flat twin across operands"
  (doc
    "The 92 set-algebra pins are all scalar/tuple elements; here a ROPE String element in one operand meets its FLAT twin in the other — the cross-operand membership must content-canonicalize (a chunk-shape hash treats rope-apple and flat-apple as distinct: union 4/inter 0/diff 2 = 402 instead of 311). All three ops in one case; runtime branch defeats the fold.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def a #set((String.concat "ap" (if (= k 1) "ple" "x")) "banana"))
          (def b #set("apple" (String.concat "che" (if (= k 1) "rry" "z"))))
          (+
            (* 100 (Set.len (Set.union a b)))
            (+ (* 10 (Set.len (Set.intersection a b))) (Set.len (Set.difference a b))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 311 Int64)))

(case
  "Set.to-list over Float32 elements enumerates by canonical byte order at f32 width"
  (doc
    "The Float32 sibling of the Float64 to-list order pin (:1494): f32 elements enumerate by
           their 4-byte canonical form as unsigned bits — a NEGATIVE (sign bit high) sorts AFTER
           every positive, so {-1.0, 0.5, 2.5} leads with 0.5 (1 at x=-1.0), while a small positive
           x=0.25 sorts first (2). An orderable arm that compared f32 by NUMERIC < (or promoted to
           f64 bits before comparing) flips the negative row — the width-specific byte order is the
           pin.")
  (input
    (do
      (def
        (main (: x Float32))
        (do
          (def xs (Set.to-list #set(x (: 0.5 Float32) (: 2.5 Float32))))
          (match
            (List.at xs 0)
            ((Some v) (if (= v (: 0.5 Float32)) 1 (if (= v x) 2 0)))
            ((None _u) -1))))
      (export main)))
  (call main (: -1.0 Float32))
  (output (: 1 Int64))
  (call main (: 0.25 Float32))
  (output (: 2 Int64)))

; --- The mixed-width NaN tuple dedup (computed f32 NaN vs the canonical spelling). ---
(case
  "a mixed-width float tuple with a computed f32 NaN dedupes against the canonical spelling"
  (doc
    "Mixed f32/f64 in ONE set element: the tuple pairs a COMPUTED f32 NaN (`(/ y y)` at y=0)
           with an f64 leaf, deduping against `(tuple Float32.nan (: 1.5 Float64))` — the canonical-byte walk
           must canonicalize the f32 leaf AT F32 WIDTH inside the compound while the f64 leaf rides
           (len 1 at y=0; y=2 computes 1.0 ≠ NaN → 2). A hash that promoted the f32 leaf to f64
           bits before canonicalizing (or canonicalized only f64 NaNs) splits the spellings. The
           mixed-width set-element companion of the equality pins.")
  (input
    (do
      (def
        (main (: y Float32))
        (do
          (def s #set(#tuple((/ y y) (: 1.5 Float64)) #tuple(Float32.nan (: 1.5 Float64))))
          (Set.len s)))
      (export main)))
  (call main (: 0.0 Float32))
  (output (: 1 Int64))
  (call main (: 2.0 Float32))
  (output (: 2 Int64)))

; --- The drained-empty canonical rep. ---
(case
  "a DRAINED set compares equal to the literal empty set — removal restores the canonical empty rep"
  (doc
    "The 1000-drain pin checks LEN only; this pins the drained set VALUE-equal to the literal empty (a root keeping an empty-node skeleton after removal differs structurally from the canonical empty singleton — node-shape hash/eq splits them). Both empties enumerate to 0.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def es #set())
          (def s1 (Set.remove #set(k) k))
          (+
            (* 100 (List.len (Set.to-list es)))
            (+ (* 10 (List.len (Set.to-list s1))) (if (= es s1) 1 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

; --- Canonical order observed through an effect sink. ---
(case
  "a canonical-order set drain EMITS each element through an effect and the state digit-encodes the order"
  (doc
    "The canonical enumeration order OBSERVED through an effect sink (the to-list pins read by index): each element performs emit in walk order and the state digit-encodes 0->1->12->123 — any reorder/dup/skip diverges; a second op reads the total back.")
  (input
    (do
      (effect Sink (op emit (-> Int64 Unit)) (op total (-> Unit Int64)))
      (def
        (drain (: xs (List Int64)) (: i Int64))
        (match
          (List.at xs i)
          ((Option.Some v) (do (Sink.emit v) (drain xs (+ i 1))))
          ((Option.None _u) unit)))
      (def
        (main (: k Int64))
        (handle
          Sink
          0
          ((emit (v) s (resume unit (+ (* s 10) v))) (total (_u) s (resume s s)))
          (do (drain (Set.to-list #set(3 k 1)) 0) (Sink.total))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 123 Int64))
  (live-objects known-leak))

; --- Remove-path canonicalization for sets (the map companions live in 05-compound-types):
; a set reached VIA a remove must be byte-canonical with the directly-built set. Both sides
; use the Set.of batch path (the Set.insert-onto-Set.empty chain still declines — see the
; recursive-sum case's construction-path asymmetry note). ---
(case
  "a runtime set reached VIA remove equals the directly-built set and not a decoy"
  (doc
    "History-independence of the set deletion path: `(Set.remove (Set.of (list x 99)) 99)` — build
           {x, 99} at run time, remove 99 — must equal `(Set.of (list x))` built without 99 (tens digit 1),
           and must NOT equal the decoy `{x+1}` (ones digit 0) → 10. The decoy leg makes the equality a
           genuine content compare, not a trivially-true reflexive check; a remove that left residual node
           structure would flip the first leg.")
  (input
    (do
      (def
        (main (: x Int64))
        (+
          (* 10 (if (= (Set.remove #set(x 99) 99) #set(x)) 1 0))
          (if (= (Set.remove #set(x 99) 99) #set((+ x 1))) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "sets reached via DIFFERENCE and INTERSECTION equal the directly-built set"
  (doc
    "The set-algebra face of construction canonicalization: {x,7,99}∖{7,99} (difference walks the
           CHAMP removing/skipping) and {x,7}∩{x,42} (intersection builds a fresh result) must BOTH be
           byte-canonical with (Set.of (list x)) — tens digit the difference leg, ones the intersection
           leg → 11. An algebra op that assembled its result on a different node layout than direct
           construction would compare unequal while holding the same elements.")
  (input
    (do
      (def
        (main (: x Int64))
        (+
          (* 10 (if (= (Set.difference #set(x 7 99) #set(7 99)) #set(x)) 1 0))
          (if (= (Set.intersection #set(x 7) #set(x 42)) #set(x)) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a set reached via UNION (overlapping and self) equals the directly-built set"
  (doc
    "The union face completing the set-algebra canonicalization family (difference/intersection
           pinned above): {x,7} ∪ {7,42} — overlap 7 must dedup onto the same node layout as the direct
           {x,7,42} (tens digit), and the self-union {x} ∪ {x} must be byte-canonical with {x} itself
           (ones digit) → 11. A union that merged overlapping branches into a different (content-equal but
           structurally distinct) layout would flip the first leg.")
  (input
    (do
      (def
        (main (: x Int64))
        (+
          (* 10 (if (= (Set.union #set(x 7) #set(7 42)) #set(x 7 42)) 1 0))
          (if (= (Set.union #set(x) #set(x)) #set(x)) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

; ── CHAMP hash-collision node ────────────────────────────────────────────────────────────
; A Set/Map is a 5-bit-per-level CHAMP over a 32-bit FNV-1a key hash (CHAMP_LEVELS=7). When two DISTINCT
; keys share the FULL 32-bit hash, they exhaust every trie level with identical fragments and MUST land in
; the same COLLISION NODE, which stores them side-by-side and disambiguates by a byte-for-byte `champ_eq`
; linear scan (cdz-runtime `is_collision_node` / `champ_eq`). This path is otherwise unwitnessed by the
; corpus. 150512886 and 59555794 are two fixnum-range Int64s with EQUAL FNV-1a hash 0x9457bd5f (brute-forced
; against the runtime's exact `champ_node_raw_hash` over the 8 LE bytes of the decoded fixnum) — so as Set
; elements / Map keys they collide at the leaf. A runtime `z` (added to each) keeps the keys off the
; const-fold path so the collision node is actually built at RUN time.
(case
  "two keys sharing a full 32-bit hash occupy one CHAMP collision node as distinct Set elements"
  (doc
    "The CHAMP collision-node path: 150512886 and 59555794 share FNV-1a hash 0x9457bd5f, so a
           `Set` holding both must build a single collision node with BOTH entries — `Set.len` is 2 (a
           collision that dropped one key, or a scan that treated them as equal, would report 1), and
           `Set.contains` finds EACH by its byte-for-byte identity. The decoy `a+1` (a NON-colliding
           neighbor, absent from the set) must be reported absent — a collision-node scan that matched by
           hash-slot rather than by `champ_eq` content would spuriously find it. Result:
           1000·len + 100·(a∈s) + 10·(b∈s) + (a+1∈s) = 1000·2 + 100 + 10 + 0 = 2110.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 150512886 z)) (b (+ 59555794 z)) (s #set((+ 150512886 z) (+ 59555794 z))))
          (+
            (* 1000 (Set.len s))
            (+
              (* 100 (if (Set.contains s a) 1 0))
              (+ (* 10 (if (Set.contains s b) 1 0)) (if (Set.contains s (+ a 1)) 1 0))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2110 Int64)))

(case
  "a Map keyed by two full-hash-colliding keys stores + retrieves each value by identity"
  (doc
    "The Map face of the CHAMP collision node: insert 150512886->1 then 59555794->2 (the two keys
           share FNV-1a hash 0x9457bd5f), so the second insert extends the first's collision node rather
           than overwriting it — `Map.len` is 2, and each lookup returns its OWN value (not the sibling's:
           the values 1 and 2 differ, so a scan matching the wrong entry — swapping m[a]↔m[b] — would give
           the full result 221 (100·2 + 10·2 + 1), not 212). Result:
           100·len + 10·(m[a]) + m[b] = 100·2 + 10·1 + 2 = 212. A collision node that clobbered on the
           second insert would give len 1; a scan comparing by hash-slot alone would return the wrong value.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 150512886 z))
            (b (+ 59555794 z))
            (m (Map.insert (Map.insert Map.empty (+ 150512886 z) 1) (+ 59555794 z) 2)))
          (+
            (* 100 (Map.len m))
            (+ (* 10 (Option.expect (Map.lookup m a) "a")) (Option.expect (Map.lookup m b) "b")))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 212 Int64)))

(case
  "two COMPOUND keys sharing a full hash collide via the nested champ_eq walk in one collision node"
  (doc
    "The compound-key face of the CHAMP collision node: the scalar cases above collide two immediate
           Int64 keys; here the colliding keys are TUPLES `(tuple 150512886 z)` and `(tuple 59555794 z)`.
           The shallow-compound `champ_hash` folds each child's `champ_node_raw_hash`, and the first children
           (150512886 vs 59555794) hash EQUAL (0x9457bd5f) while the second child is the same `z`, so the two
           tuples share the whole 32-bit hash → one collision node, disambiguated by the NESTED `champ_eq`
           walk (not the scalar leaf compare). Both tuples are distinct Set elements (len 2), each found by
           its whole-tuple identity, and a tuple with a DIFFERENT first element (`(tuple (+ 150512887 z) z)`,
           whose hash almost surely does not collide) is absent. Result: 1000·2 + 100·1 + 10·1 + 0 = 2110. A
           nested-eq that bottomed out comparing only the shared second child would wrongly fuse the two.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((s #set(#tuple((+ 150512886 z) z) #tuple((+ 59555794 z) z))))
          (+
            (* 1000 (Set.len s))
            (+
              (* 100 (if (Set.contains s #tuple((+ 150512886 z) z)) 1 0))
              (+
                (* 10 (if (Set.contains s #tuple((+ 59555794 z) z)) 1 0))
                (if (Set.contains s #tuple((+ 150512887 z) z)) 1 0))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2110 Int64)))

; ── THREE-way CHAMP collision node ───────────────────────────────────────────────────────
; The 2-key cases above build a collision node of arity 2. A THREE-way collision reaches faces a pair
; cannot: the `collision_insert` APPEND onto an EXISTING collision node (2 entries → 3), remove that
; COLLAPSES a 3-entry node back toward canonical, and set-algebra that must split/rebuild a >2-entry node.
; The keys 1, 162287981, 530337573 are three fixnum-range Int64s that ALL share FNV-1a hash 0x3e801244
; (brute-forced against `champ_node_raw_hash` over the decoded fixnum's 8 LE bytes; breaker-probed 2026-08-03).
; A runtime `z` added to each keeps them off the const-fold path so the collision node is built at RUN time.
(case
  "three keys sharing one 32-bit hash build a 3-entry CHAMP collision node (collision_insert append)"
  (doc
    "`collision_insert` APPEND: inserting a third key that shares the collision node's hash extends the
           node to arity 3 rather than overwriting or mis-slotting — 1, 162287981, 530337573 all hash to
           0x3e801244, so a Set holding all three has `Set.len` 3 and finds EACH by its `champ_eq` identity,
           while a non-colliding neighbor (`a+1`) is absent. Result: 10000·len + 1000·(a∈s) + 100·(b∈s) +
           10·(c∈s) + (a+1∈s) = 10000·3 + 1000 + 100 + 10 + 0 = 31110. An append that clobbered an existing
           entry would drop len to 2; one that mis-scanned would miss a key.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 1 z))
            (b (+ 162287981 z))
            (c (+ 530337573 z))
            (s #set((+ 1 z) (+ 162287981 z) (+ 530337573 z))))
          (+
            (* 10000 (Set.len s))
            (+
              (* 1000 (if (Set.contains s a) 1 0))
              (+
                (* 100 (if (Set.contains s b) 1 0))
                (+ (* 10 (if (Set.contains s c) 1 0)) (if (Set.contains s (+ a 1)) 1 0)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 31110 Int64)))

(case
  "removing from a 3-entry collision node leaves a live 2-entry node; removing to 1 collapses to canonical"
  (doc
    "The remove faces of a 3-way collision node. `Set.remove` of the middle colliding key from
           {1,162287981,530337573} (all hash 0x3e801244) must leave a LIVE 2-entry collision node with the
           other two still found by identity (not corrupt the node) — tens+hundreds digits. Then removing a
           second colliding key COLLAPSES the node back to a single-key set that CONTENT-equals the
           directly-built `(Set.of (list a))` — ones digit (Set `=` is content equality per this file's def, so
           this leg witnesses that the right ELEMENT survives the two removes, i.e. no key wrongly dropped or
           kept — it does not, and cannot, observe internal node layout). Result: 100·(b∉ ∧ a∈ ∧ c∈ after
           1 remove) + 10·(len 2 after 1 remove) + (collapsed == direct single) = 100·1 + 10·1 + 1 = 111. A
           remove that dropped or retained the wrong element would flip a leg (a `Set.len`-2 check pins the
           intermediate arity; the finds pin membership).")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 1 z))
            (b (+ 162287981 z))
            (c (+ 530337573 z))
            (s3 #set((+ 1 z) (+ 162287981 z) (+ 530337573 z))))
          (let
            ((s2 (Set.remove s3 b)))
            (+
              (*
                100
                (if
                  (and (not (Set.contains s2 b)) (and (Set.contains s2 a) (Set.contains s2 c)))
                  1
                  0))
              (+ (* 10 (if (= (Set.len s2) 2) 1 0)) (if (= (Set.remove s2 c) #set(a)) 1 0))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64)))

(case
  "a Map over three full-hash-colliding keys retrieves each value; removing one keeps siblings' values"
  (doc
    "The Map face of the 3-way collision node: keys 1->10, 162287981->20, 530337573->30 (all hash
           0x3e801244) share one collision node; each lookup returns its OWN value (thousands+hundreds+tens
           digits: 1+2+3 packed). Then `Map.remove` of the middle key must keep BOTH siblings' values
           retrievable (a remove that corrupted the node would lose one) — ones digit checks a's value still
           reads 10. Result: 1000·(m[a]/10) + 100·(m[b]/10) + 10·(m[c]/10) + (m2[a]==10) =
           1000·1 + 100·2 + 10·3 + 1 = 1231. A collision remove that rebuilt the node wrong would drop a
           sibling value.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 1 z))
            (b (+ 162287981 z))
            (c (+ 530337573 z))
            (m
              (Map.insert
                (Map.insert (Map.insert Map.empty (+ 1 z) 10) (+ 162287981 z) 20)
                (+ 530337573 z)
                30)))
          (let
            ((m2 (Map.remove m b)))
            (+
              (* 1000 (/ (Option.expect (Map.lookup m a) "a") 10))
              (+
                (* 100 (/ (Option.expect (Map.lookup m b) "b") 10))
                (+
                  (* 10 (/ (Option.expect (Map.lookup m c) "c") 10))
                  (if (= (Option.expect (Map.lookup m2 a) "a2") 10) 1 0)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1231 Int64)))

(case
  "set algebra over a 3-entry collision node splits and rebuilds the colliding keys by content"
  (doc
    "Set difference/union must split + rebuild a collision node of arity 3 correctly. With the three
           colliding keys a=1,b=162287981,c=530337573 (hash 0x3e801244): `{a,b,c} ∖ {b}` = {a,c} (a live
           2-entry collision node, len 2 — tens digit), and `{a,b} ∪ {c}` CONTENT-equals the full 3-entry
           `(Set.of (list a b c))` (ones digit — Set `=` is content equality, so this pins that the union
           rebuild holds exactly the three elements, no key dropped or duplicated across the collision split;
           the `Set.len`-2 difference leg pins the split arity). Result:
           10·(len {a,b,c}∖{b} == 2) + ({a,b}∪{c} == {a,b,c}) = 10·1 + 1 = 11. Algebra that mishandled the
           collision node during the split/merge would give a wrong length or a wrong element set.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((a (+ 1 z)) (b (+ 162287981 z)) (c (+ 530337573 z)))
          (+
            (* 10 (if (= (Set.len (Set.difference #set(a b c) #set(b))) 2) 1 0))
            (if (= (Set.union #set(a b) #set(c)) #set(a b c)) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

; ── Collision node ACROSS the fixnum/boxed representation split ───────────────────────────
; The collision cases above collide keys of the SAME representation (all inline fixnums, or all tuples). The
; sharpest face is a collision where the two keys straddle the fixnum/boxed boundary: 134198332 is an inline
; FIXNUM immediate (≤ FIXNUM_MAX = 2^29-1 = 536870911), 536870918 is a HEAP-BOXED Int64 (just past it), and
; both share FNV-1a hash 0x0c35bac3 (breaker-probed 2026-08-03). They occupy ONE collision node whose
; `champ_eq` must compare an inline value against a boxed one by CONTENT — the canonicalize-at-construction
; invariant that an immediate hashes AND compares equal to its boxed twin (runtime open-Q#8). A runtime `z`
; keeps each key's construction (hence its representation) off the const-fold path.
(case
  "a collision node holds a FIXNUM immediate and a BOXED Int64 sharing one hash as distinct keys"
  (doc
    "The representation-boundary face of the CHAMP collision node: 134198332 (inline fixnum, ≤ 2^29-1)
           and 536870918 (heap-boxed, just past the window) share FNV-1a hash 0x0c35bac3, so they land in one
           collision node whose `champ_eq` compares ACROSS the inline-vs-boxed split. Both are distinct Set
           elements (len 2) — each found by identity, the immediate NOT fused with the boxed sibling — and a
           Map keys them to distinct values retrieved correctly. Result: 10000·(Set.len 2) + 1000·(imm∈s) +
           100·(boxed∈s) + 10·(m[imm]) + m[boxed] = 10000·2 + 1000 + 100 + 10·1 + 2 = 21112. A `champ_eq` that
           short-circuited on the representation tag (inline≠boxed) before comparing content would treat them
           as unequal AND as non-colliding, splitting the node wrong; one that mis-decoded a boxed operand as
           inline would fuse or miscompare them.")
  (input
    (do
      (def
        (main (: z Int64))
        (let
          ((imm (+ 134198332 z))
            (boxed (+ 536870918 z))
            (s #set((+ 134198332 z) (+ 536870918 z)))
            (m (Map.insert (Map.insert Map.empty (+ 134198332 z) 1) (+ 536870918 z) 2)))
          (+
            (* 10000 (Set.len s))
            (+
              (* 1000 (if (Set.contains s imm) 1 0))
              (+
                (* 100 (if (Set.contains s boxed) 1 0))
                (+
                  (* 10 (Option.expect (Map.lookup m imm) "imm"))
                  (Option.expect (Map.lookup m boxed) "boxed")))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 21112 Int64)))

(case
  "Set.to-list orders numerically across the fixnum/boxed representation seam"
  (doc
    "The ENUMERATION-order face of the representation seam (the collision pins above cover hash/eq
           identity across it): a set holding a negative BOXED value, a small FIXNUM, and a positive BOXED
           value must enumerate in NUMERIC order — negative-boxed, fixnum, positive-boxed → a<b<c and
           a = the negative element (111). A to-list that ordered by representation TAG (all inline
           fixnums before all heap-boxed values, or heap-address order within a tag class) would place
           the small fixnum first and flip the digits.")
  (input
    (do
      (def
        (main (: z Int64))
        (match
          (Set.to-list #set((+ z 536870920) (+ z 100) (- 0 (+ z 536870915))))
          (#list(a b c)
            (+
              (* 100 (if (< a b) 1 0))
              (+ (* 10 (if (< b c) 1 0)) (if (= a (- 0 (+ z 536870915))) 1 0))))
          (_other -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 111 Int64)))

; ── CHAMP values as keys INSIDE other CHAMP values — the nesting-depth ladder ─────────────────────
; The collision pins above cover intra-node identity; these pin the DESCENT: a Set as a Map key needs
; the runtime to hash and order the nested CHAMP by canonical content; a record wrapping a Set nests
; the descent through row layout; a Set OF maps-of-sets stacks it four deep. Every face contrasts a
; content-equal hit against a decoy miss, so a depth-limited (or handle-identity) hash fails visibly.
(case
  "a SET as a Map key hits by canonical content and a different set misses"
  (doc
    "A Map keyed by `(Set.of (list 1 2))` probed with the runtime-built `(Set.of (list k 1))`: at
           k=2 the sets are content-equal (insertion order irrelevant — CHAMP canonicalizes) → 42; at
           k=3 the probe is a different set → -1. The key path must hash + compare the NESTED CHAMP by
           its canonical content — a key hash over the set's handle (or a truncated-depth walk) misses
           the k=2 hit or false-hits the k=3 miss.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (Map.lookup (Map.insert Map.empty #set(1 2) 42) #set(k 1))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64)))

(case
  "a Map keyed by a record CONTAINING a Set hits through the triple-nested descent"
  (doc
    "The record layer between the two CHAMPs: the key `(record (s (Set.of (list 1 2))) (id 7))`
           nests map→record→set. The probe rebuilds the record with a runtime-element set — content-equal
           at n=2 (42), a different inner set at n=3 (-1). The key walk must descend row layout INTO the
           set's canonical content; a row hash that stopped at the set's slot handle splits the hit.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Map.lookup
            (Map.insert Map.empty #record((= s #set(1 2)) (= id 7)) 42)
            #record((= s #set(n 1)) (= id 7)))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64)))

(case
  "a Set of MAPS-OF-SETS dedupes by full-depth content"
  (doc
    "Four CHAMP layers: the outer Set's elements are Maps whose keys are Sets. At n=2 the first two
           elements are content-equal (their inner sets {1,2} and {2,1} canonicalize identically, same
           value \"v\") and the third differs by VALUE only (\"w\") → 2 elements. At n=3 the first
           element's inner set is {1,3} → all three distinct → 3. Dedupe must reach through map-entry →
           set-key at full depth; a hash truncated at any layer collapses or splits an element.")
  (input
    (do
      (def
        (main (: n Int64))
        (Set.len
          #set((Map.insert Map.empty #set(1 n) "v")
            (Map.insert Map.empty #set(2 1) "v")
            (Map.insert Map.empty #set(1 2) "w"))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 3 Int64))
  (output (: 3 Int64)))

(case
  "Map.to-list over an owned-temporary runtime map reclaims the source, leaving no live heap objects"
  (doc
    "`(Map.to-list (build 0 3 Map.empty))` enumerates a FRESH owned-temporary map (built by a
           recursive loop so it can't const-fold) and `List.len` borrows the result, looped 500x -> 1500
           (3 entries each). map-to-list only BORROWS its source, so the owned-temporary source map AND
           the fresh result list must both be reclaimed after the borrow -- net 0 live cells. > 0 = the
           source (or result) reclaim regressed; a trap = an over-drop double-free.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: mp (Map Int64 Int64)))
        (if (< i n) (build (+ i 1) n (Map.insert mp i (* i 10))) mp))
      (def
        (loop (: j Int64) (: n Int64) (: tot Int64))
        (if (< j n) (loop (+ j 1) n (+ tot (List.len (Map.to-list (build 0 3 Map.empty))))) tot))
      (def (main (: n Int64)) (loop 0 n 0))
      (export main)))
  (call main (: 500 Int64))
  (output (: 1500 Int64))
  (live-objects 0))

(case
  "Set.to-list over an owned-temporary runtime set reclaims the source, leaving no live heap objects"
  (doc
    "`(Set.to-list (build 0 3 (Set.of (list))))` enumerates a FRESH owned-temporary set (built by a
           recursive loop so it can't const-fold) and `List.len` borrows the result, looped 500x -> 1500.
           set-to-list only BORROWS its source, so the owned-temporary source set AND the fresh result
           list must both be reclaimed after the borrow -- net 0 live cells.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (loop (: j Int64) (: n Int64) (: tot Int64))
        (if (< j n) (loop (+ j 1) n (+ tot (List.len (Set.to-list (build 0 3 #set()))))) tot))
      (def (main (: n Int64)) (loop 0 n 0))
      (export main)))
  (call main (: 500 Int64))
  (output (: 1500 Int64))
  (live-objects 0))

(case
  "Map.to-list over a BORROWED param source reused across a loop borrows it (no consume) -- value-correct, no UAF"
  (doc
    "Companion to the owned-temporary reclaim cases above: here the map is a PARAM threaded UNCHANGED
           through the loop, so the ONE caller-owned map is BORROWED by `Map.to-list mp` every iteration and
           reused on the next -- it is NOT a fresh temp. If `to-list` CONSUMED its source, iteration 2 would
           read a freed map (UAF / double-free trap); running 500x -> 1500 (3 entries each) with no trap
           proves it only borrows. The fresh RESULT list `List.len` borrows each iter must still reclaim --
           so the live count stays a CONSTANT 1, not ~500 result lists. That residual 1 is the borrowed param
           map itself, dead after the loop but not dropped on the terminal tail arm (the self-loop-tail
           back-edge reclaim gap -- known-leak, tracked separately).")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: mp (Map Int64 Int64)))
        (if (< i n) (build (+ i 1) n (Map.insert mp i (* i 10))) mp))
      (def
        (loop (: j Int64) (: n Int64) (: mp (Map Int64 Int64)) (: tot Int64))
        (if (< j n) (loop (+ j 1) n mp (+ tot (List.len (Map.to-list mp)))) tot))
      (def (main (: n Int64)) (loop 0 n (build 0 3 Map.empty) 0))
      (export main)))
  (call main (: 500 Int64))
  (output (: 1500 Int64))
  (live-objects known-leak))

(case
  "Set.to-list over a BORROWED param source reused across a loop borrows it (no consume) -- value-correct, no UAF"
  (doc
    "Set companion to the borrowed-param Map.to-list case above: a SET param threaded UNCHANGED through
           the loop, borrowed by `Set.to-list s` every iteration and reused on the next. If `to-list` consumed
           its source, iteration 2 would UAF; 500x -> 1500 with no trap proves it only borrows. The fresh
           result list is reclaimed after the borrowing `List.len`, so the live count stays a CONSTANT 1 --
           the borrowed param set itself, dead after the loop but not dropped on the terminal tail arm (the
           self-loop-tail back-edge reclaim gap -- known-leak, tracked separately).")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (loop (: j Int64) (: n Int64) (: s (Set Int64)) (: tot Int64))
        (if (< j n) (loop (+ j 1) n s (+ tot (List.len (Set.to-list s)))) tot))
      (def (main (: n Int64)) (loop 0 n (build 0 3 #set()) 0))
      (export main)))
  (call main (: 500 Int64))
  (output (: 1500 Int64))
  (live-objects known-leak))

(case
  "Set.union over two owned-temporary runtime sets reclaims both operands and the result (no live objects)"
  (doc
    "`build` recurses inserting the runtime loop counter so each set is a genuine opaque runtime value
           (no const-fold). {0,1,2} u {2,3,4} = {0,1,2,3,4}, Set.len -> 5. The union CONSUMES both owned
           operands; the fresh owned union result is borrowed then dropped by Set.len -- net 0 live cells.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def (main) (Set.len (Set.union (build 0 3 #set()) (build 2 5 #set()))))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "Set.intersection over two owned-temporary runtime sets reclaims both operands and the result (no live objects)"
  (doc
    "{0,1,2} n {2,3,4} = {2}, Set.len -> 1. Both owned operand sets (consumed by the intersection) and
           the fresh owned result must all be reclaimed after the borrowing length read -- net 0.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def (main) (Set.len (Set.intersection (build 0 3 #set()) (build 2 5 #set()))))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "Set.difference over two owned-temporary runtime sets reclaims both operands and the result (no live objects)"
  (doc
    "{0,1,2} \\ {2,3,4} = {0,1}, Set.len -> 2. Both owned operand sets and the fresh owned difference
           result must all be reclaimed after the borrowing length read -- net 0.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def (main) (Set.len (Set.difference (build 0 3 #set()) (build 2 5 #set()))))
      (export main)))
  (call main)
  (output (: 2 Int64))
  (live-objects 0))

; -- Map.len / Set.len / Set.contains / Map.lookup over an OWNED-TEMPORARY collection reclaims it (migrated
; from rcdzc map_len_and_set_len_… / set_contains_and_map_lookup_… reclaim tests). `map-size`/`set-size`/
; `set-contains`/`map-lookup` BORROW the collection (return a scalar / a dup'd value), so a fresh owned
; temporary fed to one must be dropped after the borrow or it leaks a heap cell — the same class as
; List.len / Bytes.len / Set.to-list above. `build` recurses so the collection is a genuine runtime value
; (a constant folds away). The value is unchanged by the reclaim; the stress loops (a fresh temp per
; iteration) detect a leak (drift/OOM) or a premature free (trap). (The rcdzc tests asserted the reclaim
; via component_imports_op(...,'drop') — subsumed here by the live-objects reclaim witness.)
(case
  "otc1 Map.len over an owned-temporary map reclaims it (no live objects)"
  (doc
    "`(Map.len (build 0 3 Map.empty))` = 3 over a fresh owned map; the map is dropped after the
           borrowing size read. Value 3; a leaked map cell would show live objects.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
        (if (< i n) (build (+ i 1) n (Map.insert m i i)) m))
      (def (main) (Map.len (build 0 3 Map.empty)))
      (export main)))
  (call main)
  (output (: 3 Int64))
  (live-objects 0))

(case
  "otc2 Set.len over an owned-temporary set reclaims it (no live objects)"
  (doc
    "`(Set.len (build 0 3 (Set.of (list))))` = 3 over a fresh owned set; dropped after the borrowing
           size read. Value 3.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def (main) (Set.len (build 0 3 #set())))
      (export main)))
  (call main)
  (output (: 3 Int64))
  (live-objects 0))

(case
  "otc3 a borrowed map read by Map.len then reused by an insert is not freed early"
  (doc
    "`(let ((m (build 0 3))) (+ (Map.len m) (Map.len (Map.insert m 99 99))))` reads the borrowed `m`
           by the first Map.len and reuses it in the insert — the len must not free it (else UAF/double-free
           on the reuse). The insert result is a fresh temp reclaimed by its own len. 3 + 4 = 7.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
        (if (< i n) (build (+ i 1) n (Map.insert m i i)) m))
      (def (main) (let ((m (build 0 3 Map.empty))) (+ (Map.len m) (Map.len (Map.insert m 99 99)))))
      (export main)))
  (call main)
  (output (: 7 Int64))
  (live-objects 0))

(case
  "otc4 300x Set.contains over an owned-temporary set each reclaims (no leak/UAF drift)"
  (doc
    "Stress: 300x build a fresh owned {0,1,2} and read `(Set.contains … 1)` = true (+1). A leaked set
           per iteration would OOM/drift; a premature free would trap. Sum = 300.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (drive (: j Int64) (: m Int64) (: tot Int64))
        (if (< j m) (drive (+ j 1) m (+ tot (if (Set.contains (build 0 3 #set()) 1) 1 0))) tot))
      (def (main) (drive 0 300 0))
      (export main)))
  (call main)
  (output (: 300 Int64))
  (live-objects 0))

(case
  "otc5 300x Map.lookup(Some) over an owned-temporary map each reclaims AFTER the value-dup (no UAF)"
  (doc
    "Stress + the DELICATE Map.lookup case: the looked-up value is borrowed from the map and dup'd in
           the Some arm, so the owned-map drop must come AFTER that dup (not right after lookup, which would
           free the value -> UAF). 300x build {0:0,1:10,2:20}, lookup 1 -> 10; sum = 3000.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
        (if (< i n) (build (+ i 1) n (Map.insert m i (* i 10))) m))
      (def
        (drive (: j Int64) (: k Int64) (: tot Int64))
        (if
          (< j k)
          (drive (+ j 1) k (+ tot (Option.expect (Map.lookup (build 0 3 Map.empty) 1) "v")))
          tot))
      (def (main) (drive 0 300 0))
      (export main)))
  (call main)
  (output (: 3000 Int64))
  (live-objects 0))

(case
  "a let-bound set shared across a consuming Set.union and a later read is dup'd and reclaimed once (no live objects)"
  (doc
    "Set.union CONSUMES both operands, so a let-bound `s` reused AFTER the union must be dup'd by the
           Perceus retain BEFORE the union consumes it, or the later Set.len s reads a freed set. s={0,1,2};
           (Set.len (Set.union s {5,6,7})) + (Set.len s) = 6 + 3 = 9. The union result is dropped by its
           Set.len; the dup'd s is reclaimed by the enclosing let exactly once -- net 0, neither UAF nor
           double-free.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: s (Set Int64)))
        (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
      (def
        (main)
        (let ((s (build 0 3 #set()))) (+ (Set.len (Set.union s (build 5 8 #set()))) (Set.len s))))
      (export main)))
  (call main)
  (output (: 9 Int64))
  (live-objects 0))

; -- breaker batch 416 (2026-08-26): runtime Set.to-list ORDER semantics (#3747 same-hour probe).
; The materialized order is the blessed content TOTAL ORDER (sorted), deterministic and
; backend-UNIFORM: ints incl. negatives/gaps, lexicographic strings, and a 20-element multi-node
; CHAMP all sort; insertion order is irrelevant (walk-equality twin) and len+sum invariants hold.
; NOTE for the const lens: the pinned 'const Set materialized to a list declines (order not
; presumed)' soundness case predates this — the RUNTIME order is in fact canonical-sorted, so the
; const fold could safely mirror it (observation filed, not changed here).
(case
  "sto2 insertion-order INSENSITIVITY — {3,1,2} vs {2,3,1} to-list equality via tuple walk"
  (input
    (do
      (def
        (main (: n Int64))
        (if (= #tuple(1 (Set.to-list #set(3 1 2 n))) #tuple(1 (Set.to-list #set(n 2 3 1)))) 1 0))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64)))

(case
  "sto3 to-list length + sum are order-independent invariants"
  (input
    (do
      (def
        (sum-at (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) (+ v (sum-at xs (+ i 1)))) ((Option.None) 0)))
      (def
        (main (: n Int64))
        (let ((xs (Set.to-list #set(n 10 20)))) (+ (* 100 (List.len xs)) (sum-at xs 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 333 Int64))
  (live-objects known-leak))

(case
  "sto4 Set.to-list of a 3-1-2 built set is the SORTED 1,2,3"
  (input
    (do
      (def
        (at (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) v) ((Option.None) -1)))
      (def
        (main (: n Int64))
        (let ((xs (Set.to-list #set(3 1 2)))) (+ (* 100 (at xs 0)) (+ (* 10 (at xs 1)) (at xs 2)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 123 Int64)))

(case
  "sto5 a STRING set to-list is lexicographically sorted"
  (input
    (do
      (def
        (at (: xs (List String)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) v) ((Option.None) "?")))
      (def
        (main (: n Int64))
        (let
          ((xs (Set.to-list #set("b" "a" "c"))))
          (String.concat (at xs 0) (String.concat (at xs 1) (at xs 2)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: "abc" String))
  (live-objects known-leak))

(case
  "sto6 a 20-element multi-node set to-list starts sorted 1,2,3"
  (input
    (do
      (def (grow (: s (Set Int64)) (: k Int64)) (if (= k 0) s (grow (Set.insert s k) (- k 1))))
      (def
        (at (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) v) ((Option.None) -1)))
      (def
        (main (: n Int64))
        (let
          ((xs (Set.to-list (grow #set() 20))))
          (+ (* 10000 (at xs 0)) (+ (* 100 (at xs 1)) (at xs 2)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10203 Int64)))

(case
  "sto7 negatives and gaps — {100,-5,7} to-list is sorted"
  (input
    (do
      (def
        (at (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) v) ((Option.None) -999)))
      (def
        (main (: n Int64))
        (let
          ((xs (Set.to-list #set(100 -5 7))))
          (if (= (at xs 0) -5) (if (= (at xs 1) 7) (if (= (at xs 2) 100) 1 -3) -2) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "sto8 non-prefix string set to-list orders lexicographically ab,b,bb"
  (input
    (do
      (def
        (at (: xs (List String)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) v) ((Option.None) "?")))
      (def
        (main (: n Int64))
        (let
          ((xs (Set.to-list #set("bb" "ab" "b"))))
          (String.concat
            (at xs 0)
            (String.concat "|" (String.concat (at xs 1) (String.concat "|" (at xs 2)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: "ab|b|bb" String))
  (live-objects known-leak))

; -- breaker batch 418 (2026-08-26): NON-FINITE and SIGNED-ZERO floats as CHAMP keys — hash and
; equality agree everywhere: {+0.0,-0.0} are TWO members / -0.0 is not a member of {+0.0} / a Map
; discriminates the zero keys; two runtime NaNs dedup to one member and a NaN Map key is findable
; by another NaN (canonical NaN); +inf/-inf/NaN are three distinct members. Both backends.
; (Construction trap: (- 0.0 x) at x=+0.0 is +0.0 per IEEE — use (* x -1.0) to make a true -0.0.)
(case
  "nz2 {+0.0, -0.0} are TWO set members"
  (input (do (def (main (: x Float64)) (Set.len #set(x (* x -1.0)))) (export main)))
  (call main (: 0.0 Float64))
  (output (: 2 Int64)))

(case
  "nz3 -0.0 is NOT a member of {+0.0}"
  (input (do (def (main (: x Float64)) (if (Set.contains #set(x) (* x -1.0)) 1 0)) (export main)))
  (call main (: 0.0 Float64))
  (output (: 0 Int64)))

(case
  "nz4 a Map discriminates +0.0 and -0.0 keys"
  (input
    (do
      (def
        (main (: x Float64))
        (let
          ((m (Map.insert (Map.insert #map() x 10) (* x -1.0) 20)))
          (match
            (Map.lookup m (* x -1.0))
            ((Option.Some v)
              (match (Map.lookup m x) ((Option.Some w) (+ v (* 100 w))) ((Option.None) -2)))
            ((Option.None) -1))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 1020 Int64)))

(case
  "nk1 two runtime-computed NaNs dedup to ONE set member (canonical NaN)"
  (input
    (do
      (def
        (main (: x Float64))
        (Set.len #set((- (/ x 0.0) (/ x 0.0)) (- (/ (* x 2.0) 0.0) (/ (* x 2.0) 0.0)) 1.0)))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: 2 Int64)))

(case
  "nk2 +inf, -inf, and NaN are THREE distinct set members"
  (input
    (do
      (def (main (: x Float64)) (Set.len #set((/ x 0.0) (/ (- 0.0 x) 0.0) (- (/ x 0.0) (/ x 0.0)))))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: 3 Int64)))

(case
  "nk3 a NaN Map KEY is findable by another runtime NaN (hash and equality agree)"
  (input
    (do
      (def
        (main (: x Float64))
        (match
          (Map.lookup
            (Map.insert #map() (- (/ x 0.0) (/ x 0.0)) 42)
            (- (/ (* x 3.0) 0.0) (/ (* x 3.0) 0.0)))
          ((Option.Some v) v)
          ((Option.None) -1)))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: 42 Int64)))

; -- breaker batch 419 (2026-08-26): CHAR and SYMBOL as CHAMP members/keys — dedup, membership
; hit+miss, Char-keyed Map discrimination+lookup, Symbol dedup-by-content+membership. Both
; backends. Filed adjacent: Set.to-list/(Map.to-list) with CHAR elements/keys DECLINES even for a
; length-only read (the materialization's element-type admission lacks Char; int/string/bytes
; covered) — the Map/Set-values decline class, routed.
(case
  "ck1 a Char set dedups repeated members"
  (input (do (def (main (: n Int64)) (Set.len #set(#\a #\b (if (> n 0) #\a #\c)))) (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "ck2 Char set membership hits and misses"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s #set(#\a (if (> n 0) #\b #\z))))
          (+ (if (Set.contains s #\b) 10 0) (if (Set.contains s #\c) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10 Int64)))

(case
  "ck4 a Map keyed by Char discriminates and looks up"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m (Map.insert (Map.insert #map() #\x 10) (if (> n 0) #\y #\x) 20)))
          (match
            (Map.lookup m #\x)
            ((Option.Some v)
              (match (Map.lookup m #\y) ((Option.Some w) (+ v w)) ((Option.None) -2)))
            ((Option.None) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 30 Int64)))

(case
  "ck5b Symbol set dedup+membership, in-program strings"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s (if (> n 0) "hot" "warm")))
          (let
            ((st #set((Symbol.of "hot") (Symbol.of s) (Symbol.of "cold"))))
            (+ (Set.len st) (if (Set.contains st (Symbol.of "cold")) 100 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 102 Int64)))

; -- breaker batch 420 (2026-08-26): #3765 same-hour edge pins — the Set.to-list fold also fires
; on the Ast.encode/const-PARAM demand path (stl1) and its folded result flows out of the (const ...)
; block into runtime consumption (stl2). Filed coverage gaps (all reject the miscoded CDZ0201, the
; ci04 Malformed catch-all): CHAR and BYTES elements, RECURSION-BUILT sets (incl. typed-empty seed),
; and a const-param helper consuming the folded list.
(case
  "stl1 Set.to-list folds under the Ast.encode const-param demand path"
  (input
    (do
      (def (f (const (: n Int64))) (List.len (Set.to-list #set(n (* n 2) n))))
      (def (run) (= (Ast.encode (Ast.Int (BigInt.of (f 4)))) (Ast.encode (Ast.Int (BigInt.of 2)))))
      (export run)))
  (output (: true Bool)))

(case
  "stl2 a (const Set.to-list) result is consumed by runtime List.len outside the block"
  (input (do (def (main) (List.len (const (Set.to-list #set(1 2))))) (export main)))
  (output (: 2 Int64)))

; -- breaker batch 423 (2026-08-26): CROSS-FEATURE seam pins over the day's fixes — an
; escaped-closure hop answer indexes a const-folded Set.to-list (recovery x fold); two identical
; runtime encodes DEDUP as one set member (construction-side hash/eq admits arena Bytes; xf2 is
; wasm-only — the rust encode path is pending); and a handler threading a SET state materializes a
; sorted head from its answers. Filed refinement: Set.contains / Map.lookup with an ARENA-sourced
; Bytes PROBE decline (the flat-operands query-entry gate) while construction dedups fine.
(case
  "xf1 an escaped-closure hop answer indexes a const-folded Set.to-list"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def (ap (: g (-> Int64 Int64))) (g 1))
      (def
        (main (: n Int64))
        (handle
          E
          (% n 3)
          ((tick () s (resume (* s 10) (+ s 1))))
          (match
            (List.at (const (Set.to-list #set(30 10 20))) (ap (fn (x) (+ x (E.tick)))))
            ((Option.Some v) v)
            ((Option.None) -1))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 20 Int64)))

(case
  "xf2 two IDENTICAL runtime encodes dedup as ONE set member (membership over arena Bytes)"
  (input
    (do
      (def
        (main (: n Int64))
        (Set.len
          #set((Ast.encode (Ast.Int (BigInt.of n)))
            (Ast.encode (Ast.Int (BigInt.of n)))
            (Ast.encode (Ast.Int (BigInt.of (+ n 1)))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64)))

(case
  "xf5 a handler threads a SET state and the final answer materializes its sorted head"
  (input
    (do
      (effect E (op put (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          #set(n)
          ((put () s (resume (Set.len s) (Set.insert s (* n 2)))))
          (let
            ((a (E.put)))
            (let
              ((b (E.put)))
              (match
                (List.at (Set.to-list #set((* 10 a) b)) 0)
                ((Option.Some v) v)
                ((Option.None) -1))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2 Int64)))

; -- breaker batch 425 (2026-08-26): the #3786 flat-gate QUERY faces — Set.contains and Map.lookup
; with ARENA-sourced Bytes probes (Ast.encode) now compile and answer correctly (AstEncode/AstPrint
; classified Owned). OUTPUT-ONLY pins: a borrowing op does not yet reclaim its owned operand (the
; Blake3Of-class follow-up) — live-objects clauses arrive with that fix.
(case
  "xf3 Set.contains finds an arena-sourced Bytes member via a FRESH encode"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (Set.contains
            #set((Ast.encode (Ast.Int (BigInt.of n))))
            (Ast.encode (Ast.Int (BigInt.of n))))
          1
          0))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "xf4 a Map keyed by encode-Bytes discriminates two different encodes"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m
              (Map.insert
                (Map.insert #map() (Ast.encode (Ast.Int (BigInt.of n))) 10)
                (Ast.encode (Ast.Name "x"))
                20)))
          (match
            (Map.lookup m (Ast.encode (Ast.Int (BigInt.of n))))
            ((Option.Some v) v)
            ((Option.None) -1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 10 Int64))
  (live-objects 0))

; -- breaker batch 430 (2026-08-26): BACKEND-ASYMMETRY witnesses from the rust-async row audit —
; two wasm-only declines that BOTH rust targets run correctly: Char Set.to-list (sorted, ckr1) and a
; String ENTRY param (ckr2, which also exercises Symbol members). Rows: wasm todo / rust+async pass —
; they auto-flag when the wasm emit gaps close. Refines the routings: the Char to-list decline is a
; WASM-lowering gap (not runtime capability), and the String/Bytes entry-param gap is wasm-boundary
; specific.
(case
  "ckr1 Set.to-list of Chars sorts on the RUST targets (wasm declines — the emit gap)"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (List.at (Set.to-list #set(#\c (if (> n 0) #\a #\q) #\b)) 0)
          ((Option.Some ch) (if (= ch #\a) 1 0))
          ((Option.None) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "ckr2 a String ENTRY param crosses on the RUST targets (wasm declines — the boundary gap)"
  (input
    (do
      (def
        (main (: s String))
        (let
          ((st #set((Symbol.of "hot") (Symbol.of s) (Symbol.of "cold"))))
          (+ (Set.len st) (if (Set.contains st (Symbol.of "cold")) 100 0))))
      (export main)))
  (call main (: "hot" String))
  (output (: 102 Int64)))

; ── Reclaim: Set.remove drops the owned boxed element; a heap element into an empty (Var-typed) set boxes by its own type, 0-leak (migrated from rcdzc) ──
(case
  "Set.remove drops the owned boxed element temporary it only borrows (large-int elem, no leak)"
  (doc
    "The Set.remove twin of the Map.remove owned-key drop: Set.remove BORROWS its element, so an owned
           heap element temporary must be dropped after the borrow. A large-int element 100000000000 (>
           fixnum max) op_box_int heap-allocs such a box; removing the sole element yields an empty set
           (Set.len 0) and the box is reclaimed at the borrow's end -> live-objects 0 (a fixnum element boxes
           inline, no heap temporary). A missing drop would leave the un-dropped boxed element.")
  (input
    (do
      (def (main) (Set.len (Set.remove (Set.insert #set() 100000000000) 100000000000)))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a flat String inserted into an empty set boxes by its own type (not box-int) and adds no leak"
  (doc
    "An empty `(Set.of (list))` has an UNRESOLVED Var element type; the backend must box the inserted
           element by its OWN concrete type, not default the var to box-int. Inserting a flat String emitted
           an INVALID module (box-int on the i32 String handle -> expected i64 found i32) until box_op_for
           deferred to the element node's type. A running case here proves the module is valid; the 1-element
           set (Set.len 1) reclaims fully -> live-objects 0.")
  (input (do (def (main) (Set.len (Set.insert #set() "hi"))) (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime String rope inserted into an empty set boxes by its own type and its compaction is leak-neutral"
  (doc
    "The rope companion of the flat-String empty-set insert: a runtime `String.concat` rope element
           into the same unresolved-Var empty set boxes by the String type (not box-int -> invalid module)
           AND compacts at the champ site. Set.len 1, and the owned rope consumed by the insert plus its
           refcount-neutral compaction reclaim fully -> live-objects 0 (same 0 as the flat-String baseline,
           so the compaction added no leak).")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (Set.len (Set.insert #set() (rep "hi" 3))))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

; -- breaker batch 503 (2026-08-27): the #3964/#3967 to-list ORDER folds, fresh-surface verified
; by fold-vs-runtime consistency (const build vs branch-selected runtime build, results compared
; by =). Tuple elements (element-wise lexicographic), sum elements, and sum MAP KEYS
; (discriminant-then-payload) all agree const-vs-runtime, 0-leak.
(case
  "tlf1 Set.to-list of TUPLE elements orders identically const and runtime"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((cs (Set.to-list #set(#tuple(2 1) #tuple(1 9) #tuple(1 2))))
            (rs (Set.to-list #set(#tuple((+ n -3) 1) #tuple((- n 4) 9) #tuple((- n 4) 2)))))
          (if (= cs rs) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "tlf2 Set.to-list of SUM elements orders identically const and runtime (discriminant then payload)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((cs (Set.to-list #set((Option.Some 2) Option.None (Option.Some 1))))
            (rs (Set.to-list #set((Option.Some (- n 3)) Option.None (Option.Some (- n 4))))))
          (if (= cs rs) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "tlf3 Map.to-list with SUM keys orders identically const and runtime"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((cm
              (Map.to-list
                (Map.insert
                  (Map.insert (Map.insert Map.empty (Option.Some 2) 20) Option.None 30)
                  (Option.Some 1)
                  10)))
            (rm
              (Map.to-list
                (Map.insert
                  (Map.insert (Map.insert Map.empty (Option.Some (- n 3)) 20) Option.None 30)
                  (Option.Some (- n 4))
                  10))))
          (if (= cm rm) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "salg1 the set-algebra laws hold over two runtime HAMT sets — idempotence, commutativity, self-difference, membership"
  (doc
    "Structural-law fence for the persistent Set (HAMT) algebra, complementing the reclaim-focused
           union/intersection/difference cases above. Over two overlapping RUNTIME-built sets A={0..29},
           B={15..44} (recursive `Set.insert` at 30-element scale forces multi-level HAMT branching; the bound
           is a literal but the inserts run, so the sets are heap values not folded), asserts the laws that a
           HAMT node-merge/split regression would break: A∪A = A (idempotent union, 1), A\\A = the empty set
           (len 0), A∪B = B∪A (commutative, 1), A∩A = A (1), and post-op MEMBERSHIP — 44∈A∪B (from B, 1),
           20∈A∩B (the overlap, 1), 5∈A\\B (A-only, 1). Packed positionally: `1*10^6 + 0 + 1*10^4 + 1*10^3 +
           1*10^2 + 1*10 + 1` = 1011111. A union that dropped a merged element, an intersection that kept a
           non-common one, or a difference that mis-split a shared node would move a digit. 0-leak.")
  (input
    (do
      (def
        (mkrange (: s (Set Int64)) (: i Int64) (: lo Int64) (: hi Int64))
        (if (< (+ lo i) hi) (mkrange (Set.insert s (+ lo i)) (+ i 1) lo hi) s))
      (def
        (main)
        (let
          ((a (mkrange #set() 0 0 30)) (b (mkrange #set() 0 15 45)))
          (+
            (* 1000000 (if (= (Set.union a a) a) 1 0))
            (+
              (* 100000 (Set.len (Set.difference a a)))
              (+
                (* 10000 (if (= (Set.union a b) (Set.union b a)) 1 0))
                (+
                  (* 1000 (if (= (Set.intersection a a) a) 1 0))
                  (+
                    (* 100 (if (Set.contains (Set.union a b) 44) 1 0))
                    (+
                      (* 10 (if (Set.contains (Set.intersection a b) 20) 1 0))
                      (if (Set.contains (Set.difference a b) 5) 1 0)))))))))
      (export main)))
  (output (: 1011111 Int64))
  (live-objects 0))

; --- An UNDETERMINED Set/Map key element type is a compile-time determination fault, not a codeless bail -
; A Set/Map key is CANONICALIZED at the key site, which bakes the key type's shape descriptor. When the
; key's element type is genuinely UNDETERMINED — `(Set.of (list (list)))`, whose inner empty-list element
; nothing constrains — no shape can be baked, and the seed used to DECLINE codelessly at key canonicalization
; ("list-key canonicalization: key type has no bakeable shape descriptor"), letting a not-fully-determined
; program slip to a shape-less lower bail. That is a determination fault: it is now REJECTED at compile time
; with CDZ0203 "annotate the type" — the SAME determinacy code an unannotated escaping `(None)`/empty-list
; RESULT gets — the fix being an annotation that fixes the element type, not a codeless decline. A DETERMINED
; key (a non-empty inner list, or an annotated empty one) bakes its descriptor and compiles; a GENERIC
; Set/Map-consuming def is monomorphized before this emit-time check, so its key is concrete by then — no
; false reject (seq-286 / fuzzer #5, v-compiler-primitives + v-deferral-declines routed).
(case
  "an undetermined Set-key element type is rejected CDZ0203 (annotate), not a codeless decline"
  (doc
    "`(Set.of (list (list)))` builds a `(Set (List (List ?)))` whose inner empty-list element type
           nothing determines — no canonical key shape can be baked. Rejected at compile time with CDZ0203
           'not fully determined — annotate it', the determination-fault code (mirrors the unannotated
           escaping-result reject), rather than the former codeless key-canonicalization decline.")
  (input (do (def (main) (Set.len (Set.of #list(#list())))) (export main)))
  (error CDZ0203 (message "not fully determined")))

(case
  "a DETERMINED nested-list Set key bakes its shape and compiles (the determinacy control)"
  (doc
    "The control the reject above must be distinguished from: a non-empty inner list `(list 1)` pins the
           element type to `Int64`, so `(Set (List (List Int64)))` bakes a canonical key shape and compiles —
           `Set.len` of a one-member set is 1. Pins that the CDZ0203 fires ONLY on a genuinely undetermined
           key, never on a determined one.")
  (input (do (def (main) (Set.len (Set.of #list(#list(1))))) (export main)))
  (output (: 1 Int64)))

(case
  "an ANNOTATED empty-list Set key determines the element type and compiles"
  (doc
    "The annotation fix the CDZ0203 hint points at: `(: (list) (List Int64))` determines the empty
           inner list's element type, so the key shape bakes and it compiles (`Set.len` 1). Pins that the
           annotation clears the determination fault by construction.")
  (input (do (def (main) (Set.len (Set.of #list((: #list() (List Int64)))))) (export main)))
  (output (: 1 Int64)))

(case
  "a GENERIC Set-consuming def monomorphizes at a determined call — no false reject"
  (doc
    "The no-false-reject guard: a def polymorphic over the set's element (`(dup s) = (Set.union s s)`)
           carries a free key element var in its STANDALONE scheme, but the determinacy check runs at the
           emit of a MONOMORPHIZED instance — at the concrete call `(dup (Set.of (list (list 1))))` the
           element is `(List Int64)`, determined — so it compiles (`Set.len` 1), NOT a spurious CDZ0203.
           Pins that the reject is post-monomorphization and never fires on a legitimately-generic def.")
  (input
    (do
      (def (dup s) (Set.union s s))
      (def (main) (Set.len (dup (Set.of #list(#list(1))))))
      (export main)))
  (output (: 1 Int64)))
