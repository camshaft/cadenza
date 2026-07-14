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

(case "a set is constructed from a list of its elements"
  (doc    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements: `(Set.of (list
           1 2 3))` is the set {1, 2, 3}, and its canonical written form is `(Set.of (list 1 2 3))`
           with the elements in their canonical (sorted) order. A set of Int64 has type `(Set Int64)`.")
  (input  (Set.of (list 1 2 3)))
  (output (: (Set.of (list 1 2 3)) (Set Int64))))

(case "a set collapses a duplicate element"
  (doc    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements (2nd sentence: a
           set contains each element at most once): `(Set.of (list 1 2 2 3))` names 2 twice, but the set
           holds it once — it equals `(Set.of (list 1 2 3))`. Pins that construction deduplicates rather
           than building a multiset. MUST be true.")
  (input  (= (Set.of (list 1 2 2 3)) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "set equality is independent of the order elements are written"
  (doc    "Witnesses collections-and-text.md #A Set Is A Collection Of Unique Elements (3rd sentence:
           two sets are equal exactly when they contain equal elements, independent of insertion order):
           `(Set.of (list 3 1 2))` and `(Set.of (list 1 2 3))` contain the same elements, so they are
           EQUAL regardless of the written order — the set analogue of order-independent map equality.
           Pins that set `=` compares element SETS, not positional lists. MUST be true.")
  (input  (= (Set.of (list 3 1 2)) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "membership of a present element is true"
  (doc    "Witnesses collections-and-text.md #Set Membership Is Total: `(Set.contains (Set.of (list 1 2
           3)) 2)` tests whether 2 is in the set — it is, so the total predicate yields true (a Bool,
           never a trap and never an Option). The membership companion of a map lookup, but a set's
           membership is a plain Bool because there is no associated value to return.")
  (input  (Set.contains (Set.of (list 1 2 3)) 2))
  (output (: true Bool)))

(case "membership of an absent element is false, not a trap"
  (doc    "The absent companion: `(Set.contains (Set.of (list 1 2 3)) 5)` tests an element not in the
           set, so the total predicate yields false — NOT a trap and NOT an error (collections-and-text.md
           #Set Membership Is Total). Pins that absence is an ordinary false, the reason membership needs
           no Option: a Bool already distinguishes present from absent totally.")
  (input  (Set.contains (Set.of (list 1 2 3)) 5))
  (output (: false Bool)))

(case "membership of an absent element in the empty set is false"
  (doc    "The degenerate boundary: `(Set.contains (Set.of (list)) 1)` tests membership in the empty set
           — nothing is present — so it is false. Pins that the total predicate handles the empty set
           without underflow, mirroring the empty-list / empty-map degenerate cases.")
  (input  (Set.contains (Set.of (list)) 1))
  (output (: false Bool)))

(case "the number of elements counts distinct elements"
  (doc    "`(Set.len (Set.of (list 1 2 2 3)))` is 3 — the count of DISTINCT elements, since the duplicate
           2 is held once (collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that
           len reports the set's cardinality after deduplication, not the source list's length 4.")
  (input  (Set.len (Set.of (list 1 2 2 3))))
  (output (: 3 Int64)))

(case "inserting an element yields a set containing it"
  (doc    "`(Set.insert (Set.of (list 1 2)) 3)` produces a new set {1, 2, 3} — the value heap is
           immutable, so insert returns a new set rather than mutating (memory-and-resource-model.md).
           It equals `(Set.of (list 1 2 3))`. MUST be true.")
  (input  (= (Set.insert (Set.of (list 1 2)) 3) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "inserting a present element is a no-op value"
  (doc    "`(Set.insert (Set.of (list 1 2 3)) 2)` inserts an element already present, so the result still
           holds 2 once — it equals the original `(Set.of (list 1 2 3))` (collections-and-text.md #A Set
           Is A Collection Of Unique Elements: each element at most once). Pins that insert preserves
           uniqueness rather than creating a second 2. MUST be true.")
  (input  (= (Set.insert (Set.of (list 1 2 3)) 2) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "removing an element yields a set without it"
  (doc    "`(Set.remove (Set.of (list 1 2 3)) 2)` produces a new set {1, 3} without 2 — it equals
           `(Set.of (list 1 3))`. Pins that remove drops exactly the named element and returns a new
           persistent set. MUST be true.")
  (input  (= (Set.remove (Set.of (list 1 2 3)) 2) (Set.of (list 1 3))))
  (output (: true Bool)))

(case "the union contains the elements of either set"
  (doc    "Witnesses set algebra: `(Set.union (Set.of (list 1 2)) (Set.of (list 2 3)))` is {1, 2, 3} —
           every element in either operand, with the shared 2 held once. It equals `(Set.of (list 1 2
           3))`. MUST be true.")
  (input  (= (Set.union (Set.of (list 1 2)) (Set.of (list 2 3))) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "the intersection contains the elements in both sets"
  (doc    "`(Set.intersection (Set.of (list 1 2 3)) (Set.of (list 2 3 4)))` is {2, 3} — the elements
           present in both operands — equal to `(Set.of (list 2 3))`. MUST be true.")
  (input  (= (Set.intersection (Set.of (list 1 2 3)) (Set.of (list 2 3 4))) (Set.of (list 2 3))))
  (output (: true Bool)))

(case "the difference contains the elements not in the second set"
  (doc    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list 2 3)))` is {1} — the elements of the
           first set not in the second — equal to `(Set.of (list 1))`. Pins the asymmetry of difference:
           elements of the second operand not in the first do not appear. MUST be true.")
  (input  (= (Set.difference (Set.of (list 1 2 3)) (Set.of (list 2 3))) (Set.of (list 1))))
  (output (: true Bool)))

(case "the empty set is equal to the empty set"
  (doc    "`(= (Set.of (list)) (Set.of (list)))` is true — two empty sets contain the same (no) elements
           (collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that the empty set
           is a genuine value equal to itself, the set companion of the empty-string / empty-map cases.")
  (input  (= (Set.of (list)) (Set.of (list))))
  (output (: true Bool)))

; --- Two sets with DIFFERENT elements are the SAME TYPE: comparing them is well-typed, not a --------
; --- shape error. A set's elements are runtime data, not part of its type (exactly as a map's key ---
; set is), so `(Set Int64)` is one type regardless of which ints a value holds. This is the crucial
; counterpoint the map-comparison cases (05-compound-types.sexp) already pin, carried onto the set
; path: differing elements ⇒ the comparison is FALSE (they do not contain the same elements,
; collections-and-text.md #A Set Is A Collection Of Unique Elements), NOT a CDZ0201 shape rejection.
; Contrast records/tuples, whose field set / arity IS their type.

(case "two sets with different elements are unequal, not a type error"
  (doc    "`(Set.of (list 1 2))` and `(Set.of (list 1 3))` are both `(Set Int64)` — the SAME type, since
           a set's elements are runtime data, not part of its type (unlike a record's fixed field set).
           So the comparison is well-typed and FALSE (they do not contain the same elements), NOT a type
           error. Pins that a set's elements are runtime data — the set analogue of the different-keyset
           map comparison. MUST be false.")
  (input  (= (Set.of (list 1 2)) (Set.of (list 1 3))))
  (output (: false Bool)))

(case "two sets of different sizes are unequal, not a type error"
  (doc    "`(Set.of (list 1))` and `(Set.of (list 1 2))` differ in cardinality — runtime data, not part
           of the type — so the comparison is well-typed and FALSE (collections-and-text.md #A Set Is A
           Collection Of Unique Elements). The size-difference companion; contrast records `(= (record
           (a 1)) (record (a 1) (b 2)))`, which IS a type error because a record's field set is its
           shape. MUST be false.")
  (input  (= (Set.of (list 1)) (Set.of (list 1 2))))
  (output (: false Bool)))

(case "a set with elements of two different types is a type error"
  (doc    "`(Set.of (list 1 true))` would need elements of one type, but the list mixes an Int64 and a
           Bool — not a homogeneous element type — so the set is ill-typed and the compiler rejects it
           (CDZ0201, collections-and-text.md #A Set Is A Collection Of Unique Elements: elements of one
           type), exactly as a heterogeneous list is rejected. The homogeneity flows in through the
           list `Set.of` consumes.")
  (input  (Set.of (list 1 true)))
  (error  CDZ0201))

(case "a set built at run time escapes to the host as its value form"
  (doc    "A Set built at RUN TIME (an insert-loop, not a constant `Set.of`) crosses the host boundary.
           A runtime collection has no fixed value-form template (its size is dynamic), so it escapes via
           the runtime value-encode walker guided by a compiler-baked shape descriptor whose PARAMETRIC
           frame renders the element type — the value form is `((. Set of) (list …))` with elements in
           CANONICAL key order under `(Set Int64)`. `build` inserts 3,2,1 onto an empty set → the sorted
           `(list 1 2 3)`. This declined before as needing a value-form walker.")
  (input  (do
            (def (build s n) (if (< n 1) s (build (Set.insert s n) (- n 1))))
            (def (main) (build (Set.of (list)) 3)) (export main)))
  (output (: ((. Set of) (list 1 2 3)) (Set Int64))))

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

(case "a runtime-element set equals itself"
  (doc    "`(= (Set.of (list x)) (Set.of (list x)))` with `x` a runtime parameter builds two sets from a
           list carrying a runtime element, so the comparison CANNOT fold — it defers to the runtime
           `value-eq` walk over two CHAMP handles, which are canonical by construction. A set is always
           equal to itself, so the result is true for every `x` (reflexivity). Pins that a runtime-element
           set comparison is not mis-folded to a constant `false` (the const-set-equality fold treated the
           runtime element as absent, folding even reflexivity to `false`).")
  (input  (do (def (main (: x Int64)) (= (Set.of (list x)) (Set.of (list x)))) (export main)))
  (call   main (: 9 Int64)) (output (: true Bool)))

(case "a runtime-element set's equality is independent of written order"
  (doc    "`(= (Set.of (list 1 2 x)) (Set.of (list x 2 1)))` — the same three elements written in a
           different order, one of them a runtime value. Sets are unordered (collections-and-text.md #A Set
           Is A Collection Of Unique Elements), so the two are equal regardless of order and regardless of
           `x`. Pins order-independence on the RUNTIME path — the const-set fold's order-independence,
           carried onto the deferred `value-eq` walk. True for every `x`.")
  (input  (do (def (main (: x Int64)) (= (Set.of (list 1 2 x)) (Set.of (list x 2 1)))) (export main)))
  (call   main (: 9 Int64)) (output (: true Bool))
  (call   main (: 3 Int64)) (output (: true Bool)))

(case "a runtime-element set compares equal to the constant set of the same elements"
  (doc    "`(= (Set.of (list 1 2 x)) (Set.of (list 1 2 3)))` — with `x` = 3 the two sets contain exactly
           {1,2,3} and are EQUAL; with `x` = 9 they differ and are UNEQUAL. The left set is built at run
           time (a runtime element), the right is a constant fold, and the comparison defers to the runtime
           walk. Pins that a runtime-built set and a constant set of the same elements agree — the runtime
           and constant construction paths produce byte-identical canonical CHAMP handles, and that a
           GENUINELY different element set is `false`, not accidentally `true`.")
  (input  (do (def (main (: x Int64)) (= (Set.of (list 1 2 x)) (Set.of (list 1 2 3)))) (export main)))
  (call   main (: 3 Int64)) (output (: true Bool))
  (call   main (: 9 Int64)) (output (: false Bool)))

(case "a runtime element collapses against a constant one at build"
  (doc    "`(Set.len (Set.of (list 1 2 x)))` is 2 when `x` = 1 (it collapses against the constant 1, held
           once) and 3 when `x` = 9 (a distinct third element). Pins that construction deduplicates by
           VALUE across a mix of constant and runtime elements — the uniqueness invariant holds when the
           source list is built at run time, not only when every element is a literal.")
  (input  (do (def (main (: x Int64)) (Set.len (Set.of (list 1 2 x)))) (export main)))
  (call   main (: 1 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64)) (output (: 3 Int64)))

(case "difference over a runtime-element set removes exactly that element"
  (doc    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list x)))` removes `x` from {1,2,3}: with `x`
           = 2 the result is {1,3} (does not contain 2), with `x` = 9 the result is unchanged {1,2,3}
           (still contains 2). The subtrahend is built from a runtime element, so the algebra defers to the
           runtime `set-difference` over CHAMP handles. Pins that a runtime-element operand is subtracted
           by VALUE (a regression witness: the set-algebra fold reported the runtime element absent and
           subtracted nothing, leaving 2 in the result).")
  (input  (do (def (main (: x Int64))
                (Set.contains (Set.difference (Set.of (list 1 2 3)) (Set.of (list x))) 2)) (export main)))
  (call   main (: 2 Int64)) (output (: false Bool))
  (call   main (: 9 Int64)) (output (: true Bool)))

(case "difference cardinality with a runtime-element subtrahend"
  (doc    "`(Set.len (Set.difference (Set.of (list 1 2 3)) (Set.of (list x))))` is 2 when `x` ∈ {1,2,3}
           (one element removed) and 3 when `x` is absent from the first set. The size companion of the
           membership case above, over the deferred runtime `set-difference`.")
  (input  (do (def (main (: x Int64))
                (Set.len (Set.difference (Set.of (list 1 2 3)) (Set.of (list x))))) (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64)) (output (: 3 Int64)))

(case "intersection cardinality with a runtime-element operand"
  (doc    "`(Set.len (Set.intersection (Set.of (list 1 2 3)) (Set.of (list x))))` is 1 when `x` ∈ {1,2,3}
           (the one shared element) and 0 when `x` is absent from the first set. The intersection defers to
           the runtime `set-intersection` because its second operand carries a runtime element. Pins that a
           runtime element is intersected by VALUE (a regression witness: the fold reported it absent and
           produced the empty intersection even when `x` was present).")
  (input  (do (def (main (: x Int64))
                (Set.len (Set.intersection (Set.of (list 1 2 3)) (Set.of (list x))))) (export main)))
  (call   main (: 2 Int64)) (output (: 1 Int64))
  (call   main (: 9 Int64)) (output (: 0 Int64)))

(case "union cardinality with a runtime-element operand"
  (doc    "`(Set.len (Set.union (Set.of (list 1 2 3)) (Set.of (list x))))` is 3 when `x` ∈ {1,2,3} (the
           shared element is held once) and 4 when `x` is a new element. The union defers to the runtime
           `set-union`; pins that a runtime element already present is not double-counted — the
           uniqueness invariant on the deferred path.")
  (input  (do (def (main (: x Int64))
                (Set.len (Set.union (Set.of (list 1 2 3)) (Set.of (list x))))) (export main)))
  (call   main (: 2 Int64)) (output (: 3 Int64))
  (call   main (: 9 Int64)) (output (: 4 Int64)))

(case "membership of a runtime element in a runtime-element set"
  (doc    "`(Set.contains (Set.of (list 1 2 x)) x)` is true for every `x` — a set built from a list
           containing `x` contains `x`. The membership predicate over a runtime-built set, deferring to the
           runtime `set-contains`. Pins that a runtime element is found by VALUE after construction.")
  (input  (do (def (main (: x Int64)) (Set.contains (Set.of (list 1 2 x)) x)) (export main)))
  (call   main (: 9 Int64)) (output (: true Bool))
  (call   main (: 2 Int64)) (output (: true Bool)))
