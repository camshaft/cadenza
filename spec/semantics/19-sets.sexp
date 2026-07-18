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

; --- The algebraic laws the three operations satisfy: the empty set as identity/annihilator, and ----
; --- the union laws (commutative, idempotent). These pin the operations' DEFINING identities, which
; --- the overlapping-operand cases above (which give a nontrivial result) do not exercise — a
; --- degenerate operand (the empty set, the same set twice, disjoint sets) forces the boundary of
; --- each operation. A set is a collection of unique elements (collections-and-text.md #A Set Is A
; --- Collection Of Unique Elements), so these are the ordinary laws of finite-set algebra.

(case "union with the empty set is the set itself"
  (doc    "`(Set.union (Set.of (list 1 2 3)) (Set.of (list)))` is {1, 2, 3} — the empty set is the
           identity of union, so unioning it in adds nothing. Pins the identity law the overlapping-union
           case does not (it has elements on both sides); the empty set is a genuine operand, not a
           trap. MUST be true.")
  (input  (= (Set.union (Set.of (list 1 2 3)) (Set.of (list))) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "intersection with the empty set is the empty set"
  (doc    "`(Set.intersection (Set.of (list 1 2 3)) (Set.of (list)))` is {} — the empty set is the
           annihilator of intersection, since no element is in both. Pins the annihilator law (the dual of
           the union-identity case) and that intersecting down to nothing yields the genuine empty set.
           MUST be true.")
  (input  (= (Set.intersection (Set.of (list 1 2 3)) (Set.of (list))) (Set.of (list))))
  (output (: true Bool)))

(case "the intersection of disjoint sets is empty"
  (doc    "`(Set.intersection (Set.of (list 1 2)) (Set.of (list 3 4)))` is {} — two sets sharing no
           element intersect to nothing. Pins that intersection over disjoint operands (no shared element,
           yet both non-empty) is the empty set, the complement of the overlapping-intersection case which
           has a shared element. MUST be true.")
  (input  (= (Set.intersection (Set.of (list 1 2)) (Set.of (list 3 4))) (Set.of (list))))
  (output (: true Bool)))

(case "the difference of a set with itself is empty"
  (doc    "`(Set.difference (Set.of (list 1 2 3)) (Set.of (list 1 2 3)))` is {} — removing a set's own
           elements leaves nothing. Pins the self-difference law (A ∖ A = ∅), the degenerate boundary the
           asymmetric-difference case above does not reach. MUST be true.")
  (input  (= (Set.difference (Set.of (list 1 2 3)) (Set.of (list 1 2 3))) (Set.of (list))))
  (output (: true Bool)))

(case "union is commutative"
  (doc    "`(Set.union A B)` equals `(Set.union B A)` for A = {1, 2}, B = {2, 3}: the union does not
           depend on operand order (both are {1, 2, 3}). Pins commutativity of union directly as a value
           equality between the two orderings — a law that follows from a set being an order-independent
           collection (the written-order-independence case, lifted to the operation). MUST be true.")
  (input  (= (Set.union (Set.of (list 1 2)) (Set.of (list 2 3)))
             (Set.union (Set.of (list 2 3)) (Set.of (list 1 2)))))
  (output (: true Bool)))

(case "union of a set with itself is the set (idempotent)"
  (doc    "`(Set.union (Set.of (list 1 2 3)) (Set.of (list 1 2 3)))` is {1, 2, 3} — unioning a set with
           itself introduces no duplicates (a set holds each element once), so union is idempotent. Pins
           A ∪ A = A, the duplicate-collapsing law of union at the whole-set level (the operation-level
           companion of \"a set collapses a duplicate element\"). MUST be true.")
  (input  (= (Set.union (Set.of (list 1 2 3)) (Set.of (list 1 2 3))) (Set.of (list 1 2 3))))
  (output (: true Bool)))

(case "union is associative"
  (doc    "`(A ∪ B) ∪ C` equals `A ∪ (B ∪ C)` for overlapping A={1,2}, B={2,3}, C={3,4} — the union
           regrouping does not change the result (both are {1,2,3,4}). The MULTI-way companion of
           commutativity: a canonical-order or dedup bug in the 3-way fold could break associativity while
           the 2-way commutativity above still passes, so this pins the associative regrouping directly.
           MUST be true.")
  (input  (= (Set.union (Set.union (Set.of (list 1 2)) (Set.of (list 2 3))) (Set.of (list 3 4)))
             (Set.union (Set.of (list 1 2)) (Set.union (Set.of (list 2 3)) (Set.of (list 3 4))))))
  (output (: true Bool)))

(case "union dedups overlapping elements by content, counted once"
  (doc    "`(Set.len (Set.union {1,2,3} {2,3,4}))` is 4, not 6: the shared elements 2 and 3 are held once in
           the union, not double-counted. The operation-level dedup over TWO multi-element sets (the
           existing runtime union-dedup case only overlaps a single element), pinning that union merges
           by content across a genuine multi-element overlap. MUST be 4.")
  (input  (Set.len (Set.union (Set.of (list 1 2 3)) (Set.of (list 2 3 4)))))
  (output (: 4 Int64)))

(case "intersection is associative"
  (doc    "`(A ∩ B) ∩ C` equals `A ∩ (B ∩ C)` for A={1,2,3,4}, B={2,3,4,5}, C={3,4,5,6} — both regroupings
           yield {3,4}. The intersection companion of union associativity; pins that the meet regrouping is
           order-independent. MUST be true.")
  (input  (= (Set.intersection (Set.intersection (Set.of (list 1 2 3 4)) (Set.of (list 2 3 4 5))) (Set.of (list 3 4 5 6)))
             (Set.intersection (Set.of (list 1 2 3 4)) (Set.intersection (Set.of (list 2 3 4 5)) (Set.of (list 3 4 5 6))))))
  (output (: true Bool)))

(case "difference is NOT commutative"
  (doc    "`{1,2,3} \\ {2,3,4}` = {1} but `{2,3,4} \\ {1,2,3}` = {4} — set difference is directional, so
           `A \\ B` and `B \\ A` are DIFFERENT sets (unequal). The contrast to union/intersection
           commutativity: pins that difference does NOT commute (a `=` between the two orderings is FALSE),
           so a bug treating difference symmetrically would be caught. MUST be false.")
  (input  (= (Set.difference (Set.of (list 1 2 3)) (Set.of (list 2 3 4)))
             (Set.difference (Set.of (list 2 3 4)) (Set.of (list 1 2 3)))))
  (output (: false Bool)))

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

; The escape case above crosses an INSERT-built set. A set produced by set ALGEBRA (union / intersection /
; difference) is also a runtime handle that must escape to the host as its value form — exercising the
; value-encode walker on an algebra-op RESULT (distinct from reading its cardinality/membership, which the
; algebra cases below do). A runtime element in one operand forces the algebra to run at run time, and the
; whole result set crosses, rendered in CANONICAL sorted key order.

(case "a runtime set-union result escapes to the host as its value form"
  (doc    "`(Set.union (Set.of (list 1 2)) (Set.insert (Set.of (list)) x))` unions a constant set with a
           runtime-built singleton, and the RESULT set escapes to the host. With x=5 the union is {1,2,5} →
           `((. Set of) (list 1 2 5))`; with x=1 the shared element is held once → {1,2}. Pins that a
           set-ALGEBRA result crosses the boundary via the value-encode walker (not only an insert-built
           set), rendered in canonical sorted order — the union companion of the insert-built escape.")
  (input  (do (def (main (: x Int64)) (Set.union (Set.of (list 1 2)) (Set.insert (Set.of (list)) x))) (export main)))
  (call   main (: 5 Int64)) (output (: ((. Set of) (list 1 2 5)) (Set Int64)))
  (call   main (: 1 Int64)) (output (: ((. Set of) (list 1 2)) (Set Int64))))

(case "a runtime set-difference result escapes to the host as its value form"
  (doc    "`(Set.difference (Set.of (list 1 2 3)) (Set.insert (Set.of (list)) x))` removes a runtime element
           and the RESULT escapes: x=2 → {1,3} → `((. Set of) (list 1 3))`; x=9 (absent) → the unchanged
           {1,2,3}. Pins that a difference result crosses as its canonical value form, the subtractive
           companion of the union-escape case (the value-encode walker handles an algebra result of any of
           the three set operations).")
  (input  (do (def (main (: x Int64)) (Set.difference (Set.of (list 1 2 3)) (Set.insert (Set.of (list)) x))) (export main)))
  (call   main (: 2 Int64)) (output (: ((. Set of) (list 1 3)) (Set Int64)))
  (call   main (: 9 Int64)) (output (: ((. Set of) (list 1 2 3)) (Set Int64))))

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

; --- `Set.insert` / `Set.remove` at a RUNTIME element: the functional single-element edits -------------
; The `Set.insert`/`Set.remove` cases above use CONSTANT elements, so the result folds. Inserting or
; removing a RUNTIME element (a boundary parameter) into/from a constant set cannot fold — the edit runs on
; the persistent CHAMP at run time (the operand set folds to a constant, but the edit element is dynamic).
; These pin that the runtime edit preserves the uniqueness invariant (insert of a present element is a
; no-op, remove of an absent element is a no-op), observed through membership and cardinality.

(case "inserting a runtime element adds it or is a no-op if already present"
  (doc    "`(Set.len (Set.insert (Set.of (list 1 2 3)) x))` is 4 when `x` is a NEW element (4 → the set
           grows) and 3 when `x` is already present (2 → held once, insert is a no-op value,
           collections-and-text.md #A Set Is A Collection Of Unique Elements). Pins that a runtime insert
           preserves uniqueness — the cardinality reflects present-vs-absent decided at run time, deferring
           to the runtime `set-insert`.")
  (input  (do (def (main (: x Int64)) (Set.len (Set.insert (Set.of (list 1 2 3)) x))) (export main)))
  (call   main (: 4 Int64)) (output (: 4 Int64))
  (call   main (: 2 Int64)) (output (: 3 Int64)))

(case "inserting a runtime element yields a set that contains it"
  (doc    "`(Set.contains (Set.insert (Set.of (list 1 2)) x) x)` is true for every `x` — the element just
           inserted at run time is present. Pins that a runtime `set-insert` actually adds the element
           (found by value afterward), the membership companion of the cardinality case.")
  (input  (do (def (main (: x Int64)) (Set.contains (Set.insert (Set.of (list 1 2)) x) x)) (export main)))
  (call   main (: 5 Int64)) (output (: true Bool))
  (call   main (: 1 Int64)) (output (: true Bool)))

(case "removing a runtime element drops it or is a no-op if absent"
  (doc    "`(Set.contains (Set.remove (Set.of (list 1 2 3)) x) 2)` — removing `x` from {1,2,3} then testing
           for 2: when `x`=2 the removed element IS 2, so 2 is gone (false); when `x`=9 (absent) the set is
           unchanged and still holds 2 (true — removal is total, collections-and-text.md #A Set Is A
           Collection Of Unique Elements). Pins that a runtime `set-remove` drops exactly the named element
           and is a no-op on an absent one.")
  (input  (do (def (main (: x Int64)) (Set.contains (Set.remove (Set.of (list 1 2 3)) x) 2)) (export main)))
  (call   main (: 2 Int64)) (output (: false Bool))
  (call   main (: 9 Int64)) (output (: true Bool)))

(case "removing a runtime element lowers the cardinality only when present"
  (doc    "`(Set.len (Set.remove (Set.of (list 1 2 3)) x))` is 2 when `x` ∈ {1,2,3} (one element dropped)
           and 3 when `x` is absent (removal is total, the set is unchanged). The cardinality companion of
           the membership case, over the runtime `set-remove`.")
  (input  (do (def (main (: x Int64)) (Set.len (Set.remove (Set.of (list 1 2 3)) x))) (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64)) (output (: 3 Int64)))

; --- A Set threaded as a recursive ACCUMULATOR — the seen-set / visited-set idiom ----------------------
; A compiler carries a Set as an accumulator across a recursion — a set of visited nodes (cycle detection),
; free variables collected, or declared capabilities — inserting as it walks, then querying membership or
; cardinality. This is the set analogue of the map-accumulator (05-compound-types) and distinct from the
; runtime set-escape case above (which RETURNS the set): here the set is THREADED as its own parameter and
; consumed to a SCALAR (`Set.len` / `Set.contains`), with dedup happening DURING the runtime accumulation.

(case "a set threaded as a recursive accumulator dedups during accumulation"
  (doc    "`build` inserts `n % 3` for n, n-1, …, 1 into a set THREADED as its own parameter, then `Set.len`
           measures it. The inserted values cycle through {0,1,2}, so the set holds at most 3 distinct
           elements regardless of `n`: `build 6` inserts 0,2,1,0,2,1 → 3; `build 2` inserts 2,1 → 2;
           `build 0` → 0 (the empty accumulator). Pins that a set carried as a recursive accumulator dedups
           its inserts at run time (the uniqueness invariant across the threaded accumulation), consumed to
           a scalar — the seen-set idiom, the set companion of the map accumulator.")
  (input  (do
            (def (build (: n Int64) (: s (Set Int64))) (if (= n 0) s (build (- n 1) (Set.insert s (% n 3)))))
            (def (main (: n Int64)) (Set.len (build n (Set.of (list))))) (export main)))
  (call   main (: 6 Int64)) (output (: 3 Int64))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "a set accumulator is queried for membership after building"
  (doc    "The visited-set query: `build 5` accumulates {1,2,3,4,5} through the threaded set parameter, then
           `Set.contains` tests a runtime query element — q=3 → present (1), q=9 → absent (0). Pins that a
           set grown across a recursion answers a membership query afterward — the cycle-detection /
           already-seen check a compiler pass makes while walking, the set companion of the map-lookup
           accumulator query.")
  (input  (do
            (def (build (: n Int64) (: s (Set Int64))) (if (= n 0) s (build (- n 1) (Set.insert s n))))
            (def (main (: q Int64)) (if (Set.contains (build 5 (Set.of (list))) q) 1 0)) (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64))
  (call   main (: 9 Int64)) (output (: 0 Int64)))

; --- A Set consumed by Set.insert in one operand is UNCHANGED for a later read of the same binding ------
; The set analogue of the shared-`let` List persistence cases (05-compound-types): `Set.insert` is
; PERSISTENT — it produces a new set and MUST leave its operand unchanged (collections-and-text.md: a value
; must not be observably mutated through one reference while read through another). A set bound by `let`
; and read TWICE — once consumed by an insert, once read as the original — is SHARED, so the consuming op
; must copy, not FBIP-mutate in place. The compiler emits a Perceus RETAIN (`dup`) at the consumed
; occurrence of a binding with a later live use; without it the CHAMP insert would mutate the shared trie
; and the later read would see the grown set (the same defect the List/projection persistence cases pin).

(case "a set consumed by Set.insert in one operand is unchanged for a later read of the same binding"
  (doc    "`s = build 0 3` = {0,1,2} (a genuine runtime set, no const-fold); read twice: the left operand
           inserts 99 and measures (→ 4), the right reads the ORIGINAL `s` size (→ 3), so 4 + 3 = 7. If the
           insert mutated the shared `s` in place (a CHAMP FBIP update whose retain was missing on a
           multi-use binding), the second read would see {0,1,2,99} → 8. Order-sensitive (reading `s` first
           → 7 regardless), the tell of an in-place mutation. Pins that a persistent Set.insert leaves a
           shared operand unchanged — the Set companion of the List.push persistence case.")
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (Set Int64)))
              (if (< i n) (build (+ i 1) n (Set.insert acc i)) acc))
            (def (main (: n Int64))
              (let ((s (build 0 n (Set.of (list)))))
                (+ (Set.len (Set.insert s 99)) (Set.len s))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 7 Int64))
  (call   main (: 1 Int64)) (output (: 3 Int64))
  (call   main (: 5 Int64)) (output (: 11 Int64)))

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

(case "Set.contains does not fold a set whose element is a runtime scalar"
  (doc    "`(Set.contains (Set.of (list (add 2 3))) 5)` — the set's sole element `(add 2 3)` is a recursive
           call (non-foldable), evaluating to 5 at run time; membership of the literal 5 must be true → 1.
           Before the fix the `Set.contains` fold saw a `Core::SetOf` shape + a constant query and compared
           only the CONSTANT elements (none), folding to `false` → 0 though the runtime element IS 5. The
           fold now declines (a non-constant element) and the runtime `set-contains` answers correctly.")
  (input  (do
            (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
            (def (main) (if (Set.contains (Set.of (list (add 2 3))) 5) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "Set.remove does not fold a set whose element is a runtime scalar"
  (doc    "`(Set.len (Set.remove (Set.of (list (add 2 3))) 5))` — removing the literal 5 from a set whose sole
           element `(add 2 3)`=5 (runtime) must drop it → cardinality 0. Before the fix the fold RETAINED the
           runtime element (its compile-time equality to 5 is unknown, so `retain` kept it) → 1. The fold now
           declines and the runtime `set-remove` removes the matching element.")
  (input  (do
            (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
            (def (main) (Set.len (Set.remove (Set.of (list (add 2 3))) 5))) (export main)))
  (output (: 0 Int64)))

(case "Set.insert does not fold a duplicate against a runtime element"
  (doc    "`(Set.len (Set.insert (Set.of (list (add 2 3))) 5))` — inserting 5 into a set whose sole element
           `(add 2 3)`=5 (runtime) is a duplicate, so the cardinality stays 1. Before the fix the fold could
           not see the runtime element equalled 5 (its const probe missed it) and would ADD 5 as a second
           element → 2. The fold now declines and the runtime `set-insert` dedups against the canonical
           champ set.")
  (input  (do
            (def (add (: x Int64) (: n Int64)) (if (< n 1) x (add (+ x 1) (- n 1))))
            (def (main) (Set.len (Set.insert (Set.of (list (add 2 3))) 5))) (export main)))
  (output (: 1 Int64)))

(case "Set.contains finds a runtime STRING-rope element built via Set.of"
  (doc    "`(Set.contains (Set.of (list (rep \"hi\" 3))) \"hixxx\")` — the set's element is a runtime String
           ROPE (`rep` concatenates \"x\" three times → \"hixxx\"), membership-tested with the flat literal
           \"hixxx\" → 1. This is the reported adversarial finding: the `Set.contains` fold (mis)fired on a
           runtime-element `SetOf` and answered 0. The fold now declines and the runtime `set-contains`
           canonicalizes both the stored rope (compacted at Set.of construction) and the flat query, so they
           match. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (Set.contains (Set.of (list (rep "hi" 3))) "hixxx") 1 0)) (export main)))
  (output (: 1 Int64)))

(case "Set.remove of a rope-element set built via Set.of lowers the cardinality"
  (doc    "`(Set.len (Set.remove (Set.of (list (rep \"hi\" 3))) \"hixxx\"))` — removing the flat literal
           \"hixxx\" from a set whose sole element is the equal runtime rope must drop it → 0. The
           stronger rope twin of the finding (it hits Set.remove too, not just Set.contains). Before the fix
           the fold retained the rope → 1. Expected: 0.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (Set.len (Set.remove (Set.of (list (rep "hi" 3))) "hixxx"))) (export main)))
  (output (: 0 Int64)))

(case "an all-constant Set.of still folds membership with a constant query (control)"
  (doc    "`(Set.contains (Set.of (list 1 2 3)) 2)` — every element AND the query are compile-time constants,
           so the `Set.contains` fold is SOUND and still fires (→ 1); the absent constant query 9 folds to 0.
           Pins that the runtime-element guard did NOT disable the valuable all-constant fold. Two exports
           (a present and an absent constant query) so both fold branches are witnessed. Expected: 1, 0.")
  (input  (do
            (def (has2) (if (Set.contains (Set.of (list 1 2 3)) 2) 1 0))
            (def (has9) (if (Set.contains (Set.of (list 1 2 3)) 9) 1 0))
            (export has2) (export has9)))
  (call   has2) (output (: 1 Int64))
  (call   has9) (output (: 0 Int64)))

(case "Set.to-list enumerates the elements as a List in canonical (sorted) order"
  (doc    "`(List.at (Set.to-list (Set.of (list 5 2 8 2))) 0)` — Set.to-list yields the set's elements as a
           `List` in CANONICAL element-value order (sorted, deduped: {2,5,8}), NOT hash/insertion order,
           realizing collections-and-text.md §Map/Set iteration is deterministic. The element at index 0 is
           the smallest, 2. The inverse of Set.of. Expected: 2.")
  (input  (do
            (def (main) (match (List.at (Set.to-list (Set.of (list 5 2 8 2))) 0)
                          ((Some v) v)
                          ((None u) -1))) (export main)))
  (output (: 2 Int64)))

(case "Set.to-list length is the set's cardinality (deduped)"
  (doc    "`(List.len (Set.to-list (Set.of (list 3 1 2 1 3))))` — the enumerated list has one element per
           DISTINCT set element ({1,2,3} → 3), so its length equals Set.len. Pins the dedup + round count.
           Expected: 3.")
  (input  (do
            (def (main) (List.len (Set.to-list (Set.of (list 3 1 2 1 3))))) (export main)))
  (output (: 3 Int64)))

; The Set.to-list cases above enumerate a CONSTANT `Set.of` literal. A set built AT RUN TIME by a
; recursive `Set.insert` loop over a boundary parameter is a genuine runtime CHAMP the `set-to-list`
; runtime op (index 83) walks live — its canonical (sorted) order emerges from the cursor walk + the
; canonical-scalar sort, NOT from a folded pre-sorted literal. These pin the runtime enumeration op end
; to end: the order is canonical regardless of insertion order, and the enumerated list is consumed by
; a List.at/List.len fold (the idiom a self-hosted pass uses to iterate a set's members deterministically).
(case "Set.to-list over a RUNTIME-built set yields canonical order (first element is the minimum)"
  (doc    "`ins n` inserts `20-n` for n=n..1 into a set built by a recursive `Set.insert` loop — so the
           elements arrive in DESCENDING order (19,18,…) but the set is unordered. `Set.to-list` enumerates
           them in canonical (ascending) order, so element 0 is the minimum. `ins 5` inserts {15,16,17,18,19};
           the first enumerated element is 15. Pins that the runtime set-to-list op sorts by canonical value,
           not insertion order, over a genuinely runtime-built CHAMP (not a folded constant `Set.of`).")
  (input  (do
            (def (ins (: n Int64) (: s (Set Int64)))
              (if (< n 1) s (ins (- n 1) (Set.insert s (- 20 n)))))
            (def (main (: n Int64)) (Option.expect (List.at (Set.to-list (ins n (Set.of (list)))) 0) "empty"))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))

(case "Set.to-list canonical order is SIGNED at the integer extremes"
  (doc    "A set holding BOTH i64 limits enumerates with the NEGATIVE extreme first and the positive
           extreme last — the canonical order is the SIGNED value order at the sign boundary. A sort
           comparing raw two's-complement bytes or unsigned values would place Int64.min (0x8000…)
           AFTER Int64.max (0x7FFF…), inverting the ends. `(Set.of (list max n 0 min))` with n=5:
           element 0 is min. The extreme-key companion of the ascending-order pins above, whose keys
           never cross the sign boundary.")
  (input  (do
            (def (main (: n Int64))
              (Option.expect
                (List.at (Set.to-list (Set.of (list 9223372036854775807 n 0 -9223372036854775808))) 0)
                "empty"))
            (export main)))
  (call   main (: 5 Int64))
  (output (: -9223372036854775808 Int64)))

(case "Set.to-list over a runtime set sums its distinct elements"
  (doc    "`ins n` inserts `(n*7) % 5` for n=n..1 — a runtime set whose elements cycle through {0,1,2,3,4}
           with many collisions (dedup). `Set.to-list` enumerates the distinct elements; a List.at fold sums
           them. `ins 10` deduplicates to {0,1,2,3,4}, sum 10. Pins that the runtime enumeration yields each
           DISTINCT element exactly once and the resulting list is fold-consumable (the set→list→fold idiom).")
  (input  (do
            (def (ins (: n Int64) (: s (Set Int64)))
              (if (< n 1) s (ins (- n 1) (Set.insert s (% (* n 7) 5)))))
            (def (sumlist (: l (List Int64)) (: i Int64) (: a Int64))
              (if (= i (List.len l)) a (sumlist l (+ i 1) (+ a (Option.expect (List.at l i) "oob")))))
            (def (main (: n Int64)) (sumlist (Set.to-list (ins n (Set.of (list)))) 0 0))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10 Int64)))

; The cases above all enumerate a NON-EMPTY set. The empty boundary matters for a real pass that walks a
; possibly-empty symbol table / free-var set: `Set.to-list` of an EMPTY (but element-TYPED) set is the
; empty list, length 0. The set is emptied at RUN TIME (`Set.remove` of the sole element) so the element
; type is `Int64` (fixing the canonical-ordering descriptor) while the runtime CHAMP is empty — distinct
; from an untyped empty `Set.of (list)` literal, whose element type is undetermined.
(case "Set.to-list of a runtime-empty but element-typed set is the empty list"
  (doc    "`(Set.remove (Set.of (list 1)) 1)` is a `Set Int64` emptied at run time; `Set.to-list` of it is
           the empty list, so `List.len` is 0. Pins the empty boundary of set enumeration — a pass walking a
           set that happens to be empty gets an empty list, not a trap — with the element type fixed to
           Int64 (so the canonical-order descriptor is well-defined), the shape a symbol-table / free-var
           enumeration takes when the collection is empty.")
  (input  (do
            (def (main) (List.len (Set.to-list (Set.remove (Set.of (list 1)) 1))))
            (export main)))
  (output (: 0 Int64)))

(case "insert-order does not leak into Set.to-list enumeration order"
  (doc    "{3, 1, 2} built by inserts IN THAT ORDER enumerates [1, 2, 3] — element 0 is 1 and element
           2 is 3 -> 103. Insertion HISTORY is unobservable (canonical order); a cursor walking
           trie/hash order (which varies with insert sequence) or an append-in-insert-order
           enumeration leaks it. Complements the runtime-min case above (which pins the first
           element) by pinning a NON-head position of a scrambled successive-insert build.")
  (input  (do
            (def (main (: d Int64))
              (let ((xs (Set.to-list (Set.insert (Set.insert (Set.insert (Set.of (list)) 3) 1) 2))))
                (+ (* 100 (Option.expect (List.at xs 0) "a"))
                   (Option.expect (List.at xs 2) "c"))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 103 Int64)))

(case "a runtime-keyed map entry enumerates as its key-value tuple"
  (doc    "`(Map.to-list (Map.insert Map.empty k 42))` with k a parameter — the single entry
           enumerates as a (k, 42) tuple whose value projects 42. Pins the entry-tuple
           materialization over a runtime key (a folded key builds the tuple at compile time; this
           one must build it from live heap values).")
  (input  (do
            (def (main (: k Int64))
              (. (Option.expect (List.at (Map.to-list (Map.insert Map.empty k 42)) 0) "e") 1))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 42 Int64)))

(case "a Float element inserted into an empty (runtime) set boxes with box-float, not box-int"
  (doc    "MISCOMPILE (invalid wasm, wasm-only): `Set.insert (Set.of (list)) x` with `x : Float64` — a
           SINGLE float insert into a runtime EMPTY set — imported `box-int` but the emit called
           `box-float`, so `box-float` was un-imported and the call resolved to `u32::MAX` → invalid
           component at load. ROOT: the import collector used `box_op_ty(elem_ty)` while the emit used
           `box_op_for(elem_node, elem_ty)`; for an empty base the element type is an unresolved `Var`, which
           `box_op_ty` DEFAULTS to `box-int` but `box_op_for` resolves from the element NODE (a Float →
           `box-float`) — a coemit mismatch, the empty-set String box-int bug's float twin. A CONSTANT float
           `Set.of` folds (never emits the insert), which is why only the runtime empty-base insert broke. Fix:
           the collector's Set/Map insert arms use `box_op_for` (node-aware) so imports match the emit.
           `Set.len` of the 1-element set is 1.")
  (input  (do
            (def (main (: d Float64))
              (Set.len (Set.insert (Set.of (list)) d)))
            (export main)))
  (call   main (: 2.5 Float64))
  (output (: 1 Int64)))

(case "a Float VALUE inserted into an empty (runtime) map boxes with box-float, not box-int"
  (doc    "The Map-VALUE twin of the empty-set float box case above: `Map.insert Map.empty 1 x` with a
           runtime `x : Float64` into an empty map (undetermined value type) — the value box op must come
           from `x`'s node type (`box-float`), imported to match the emit; before the collector used
           `box_op_for` for the value it grounded the `Var` value type to `box-int` → un-imported
           `box-float` → invalid wasm. One entry → `Map.len` = 1.")
  (input  (do
            (def (main (: d Float64))
              (Map.len (Map.insert (Map.empty) 1 d)))
            (export main)))
  (call   main (: 2.5 Float64))
  (output (: 1 Int64)))

(case "a Float KEY inserted into an empty (runtime) map boxes with box-float, not box-int"
  (doc    "The Map-KEY twin: `Map.insert Map.empty x 1` with a runtime `x : Float64` key into an empty map
           (undetermined key type) — the key box op comes from `x`'s node type (`box-float`), imported to
           match the emit (the same node-aware `box_op_for` collector fix). One entry → `Map.len` = 1.")
  (input  (do
            (def (main (: d Float64))
              (Map.len (Map.insert (Map.empty) d 1)))
            (export main)))
  (call   main (: 3.5 Float64))
  (output (: 1 Int64)))

; --- Float CHAMP keys/elements under the canonical byte form ----------------------------------------
; 9c2790cef fixed the Float element-boxing arm (box-float, not the defaulted box-int — my filed
; invalid-wasm; its pin covers the empty-set insert). These pin the canonical-form semantics the
; boxing now reaches, promoted from breaker probes held back until the fix: NaN is ONE key; -0.0
; and 0.0 are TWO.

(case "a NaN map key is found by a differently-produced NaN"
  (doc    "Insert under `(/ x x)` at x = 0.0 (a computed NaN), look up with `Float64.nan` → 42.
           Every NaN shares one canonical byte form, so champ_hash/champ_eq land both spellings in
           one slot — the map-key face of the scalar NaN-equality rule (a raw-bits hash scatters
           NaN keys; raw f64.eq never matches them).")
  (input  (do
            (def (main (: x Float64))
              (match (Map.lookup (Map.insert Map.empty (/ x x) 42) Float64.nan)
                ((Some v) v)
                ((None _) -1)))
            (export main)))
  (call   main (: 0.0 Float64))
  (output (: 42 Int64)))

(case "negative zero and zero are distinct map keys"
  (doc    "Insert 1 under -0.0 and 2 under 0.0: the map holds TWO entries and -0.0 looks up its own
           value → 10·2 + 1 = 21. The -0.0 complement of the NaN-unification key face (an f64.eq
           key compare collapses the pair to one entry).")
  (input  (do
            (def (main (: d Int64))
              (+ (* 10 (Map.len (Map.insert (Map.insert Map.empty -0.0 1) 0.0 2)))
                 (match (Map.lookup (Map.insert (Map.insert Map.empty -0.0 1) 0.0 2) -0.0)
                   ((Some v) v)
                   ((None _) -1))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 21 Int64)))

(case "a set dedups NaN elements and keeps zero signs distinct"
  (doc    "Insert a computed NaN then `Float64.nan` → ONE element (canonical unification); insert
           -0.0 then 0.0 → TWO (distinct canonical forms): 10·1 + 2 = 12. The set-element face of
           both canonical-form rules through the fixed box-float path.")
  (input  (do
            (def (main (: x Float64))
              (+ (* 10 (Set.len (Set.insert (Set.insert (Set.of (list)) (/ x x)) Float64.nan)))
                 (Set.len (Set.insert (Set.insert (Set.of (list)) -0.0) 0.0))))
            (export main)))
  (call   main (: 0.0 Float64))
  (output (: 12 Int64)))

(case "a computed float map key is found by its literal twin"
  (doc    "Insert under `(+ x 1.25)` at x = 1.25, look up with the literal 2.5 → 42. The
           arithmetic-result key and the literal share one canonical form (float arithmetic is
           deterministic; the emitted add's bits equal the folded literal's) — the computed-key
           control beside the special-value faces.")
  (input  (do
            (def (main (: x Float64))
              (match (Map.lookup (Map.insert Map.empty (+ x 1.25) 42) 2.5)
                ((Some v) v)
                ((None _) -1)))
            (export main)))
  (call   main (: 1.25 Float64))
  (output (: 42 Int64)))

; --- CHAMP Set DEDUP follows the canonical FLOAT byte form (float-form × dedup intersection) ----------
; A Set dedups by hash+eq, and both must follow the SAME canonical byte form that scalar/compound `=`
; pins (03-equality NaN==NaN, -0.0 != +0.0). If the Set hashed/compared floats by IEEE == instead
; (nan != nan, -0.0 == +0.0), dedup would disagree with equality. Runtime float params (def args) keep
; the set off the const-fold path so the CHAMP heap dedup actually runs. NOTE: float-in-set currently
; declines on the RUST backend (a known coverage gap, same as the box-float insert case above) — these
; pin the WASM path.

(case "a set of two NaN floats dedups to one (canonical quiet-NaN)"
  (doc    "CHAMP dedup follows the canonical float byte form: two runtime NaN elements both canonicalize
           to the one quiet-NaN (box-float), so `(Set.of (list nan nan))` has ONE element, `Set.len` = 1.
           IEEE == would treat nan != nan and keep both (len 2); the canonical byte form the scalar
           `nan == nan` case pins (03-equality) says one. Runtime Float64 params keep it off const-fold.")
  (input  (do (def (build (: x Float64) (: y Float64)) (Set.len (Set.of (list x y))))
              (def (main (: d Int64)) (build Float64.nan Float64.nan)) (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64)))

(case "a set of negative zero and positive zero keeps both (distinct sign bits)"
  (doc    "The `-0.0 != +0.0` companion for CHAMP dedup: distinct sign bits are distinct canonical bytes,
           so `(Set.of (list -0.0 0.0))` keeps BOTH, `Set.len` = 2. IEEE == would treat -0.0 == +0.0 and
           dedup to 1; the canonical byte form the scalar `-0.0 != 0.0` case pins says two. Confirms Set
           dedup agrees with `=`, not with IEEE ==.")
  (input  (do (def (build (: x Float64) (: y Float64)) (Set.len (Set.of (list x y))))
              (def (main (: d Int64)) (build -0.0 0.0)) (export main)))
  (call   main (: 0 Int64))
  (output (: 2 Int64)))

(case "a set of two identical positive floats dedups to one"
  (doc    "The plain positive control: two identical runtime floats share canonical bytes, so
           `(Set.of (list 3.5 3.5))` dedups to ONE, `Set.len` = 1. Rules out an always-keep-both bug that
           would make the NaN case pass for the wrong reason.")
  (input  (do (def (build (: x Float64) (: y Float64)) (Set.len (Set.of (list x y))))
              (def (main (: d Int64)) (build 3.5 3.5)) (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64)))

(case "Set.contains over negative zero does not find positive zero"
  (doc    "`Set.contains` uses the same canonical hash+eq as dedup: a set holding -0.0 does NOT contain
           +0.0 (distinct canonical bytes) → false. The membership analogue of the -0.0 != +0.0 dedup
           case, pinning that contains and dedup share the float byte-form rule.")
  (input  (do (def (test (: stored Float64) (: probe Float64)) (Set.contains (Set.of (list stored)) probe))
              (def (main (: d Int64)) (if (test -0.0 0.0) 1 0)) (export main)))
  (call   main (: 0 Int64))
  (output (: 0 Int64)))

(case "Set.contains over nan finds nan"
  (doc    "The membership positive: a set holding a NaN DOES contain a NaN (both canonicalize to the one
           quiet-NaN) → true. The contains analogue of the NaN-dedup case.")
  (input  (do (def (test (: stored Float64) (: probe Float64)) (Set.contains (Set.of (list stored)) probe))
              (def (main (: d Int64)) (if (test Float64.nan Float64.nan) 1 0)) (export main)))
  (call   main (: 0 Int64))
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

(case "a context-typed empty float map needs no key inserted to ground its wrapper type"
  (doc    "`(: (Map.empty) (Map Float64 Int64))` is an empty float-keyed map grounded by its ANNOTATION, no
           insert — `Map.len` = 0. On rust the `__CdzF64` key wrapper is named in the map type-param with no
           constructor, so its decl must be injected on the annotation; a constructor-only gate would emit
           `BTreeMap<__CdzF64,_>` with no decl and fail to compile. Pins the typed-empty float map compiles
           and runs on both backends.")
  (input  (do (def (main) (Map.len (: (Map.empty) (Map Float64 Int64)))) (export main)))
  (output (: 0 Int64)))

(case "a context-typed empty float map accepts a runtime float key insert"
  (doc    "The construction companion: inserting a runtime `d : Float64` key into the typed-empty float map
           `(Map.insert (: (Map.empty) (Map Float64 Int64)) d 1)` → one entry, `Map.len` = 1. Pins the
           annotation-grounded wrapper agrees with the constructed-key path — the annotation and the insert
           name the SAME `__CdzF64` key type, so the decl injected for the annotation covers the insert.")
  (input  (do (def (main (: d Float64)) (Map.len (Map.insert (: (Map.empty) (Map Float64 Int64)) d 1))) (export main)))
  (call   main (: 2.5 Float64))
  (output (: 1 Int64)))

(case "a context-typed empty float set grounds its element wrapper type"
  (doc    "The Set companion: `(: (Set.of (list)) (Set Float64))` is an empty float-element set grounded by
           its annotation — `Set.len` = 0. Pins the wrapper-decl injection covers a float Set ELEMENT type
           (`__CdzF64` in a set type-param), not only a Map key.")
  (input  (do (def (main) (Set.len (: (Set.of (list)) (Set Float64)))) (export main)))
  (output (: 0 Int64)))

(case "a context-typed empty float32 map grounds the narrow-float wrapper"
  (doc    "The narrow-width companion: `(: (Map.empty) (Map Float32 Int64))` grounds the `__CdzF32` wrapper
           (distinct from `__CdzF64`) — `Map.len` = 0. Pins the decl injection is width-specific and covers
           the Float32 wrapper too, the narrow-float dual of the Float64 typed-empty map.")
  (input  (do (def (main) (Map.len (: (Map.empty) (Map Float32 Int64)))) (export main)))
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

(case "a set built from a BigInt sum and a BigInt.of over integer arithmetic has both elements"
  (doc    "`(Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2))))` with n=5 holds the BigInt
           values 6 and 7 — two distinct elements, so Set.len = 2. The first element (a BigInt sum) stashes
           an i32 handle in a scratch slot; the second (a BigInt.of over a checked `(+ n 2)`) carries an i64
           overflow-guard temp — and the Set.of emit must keep them on disjoint slots or the wasm local is
           declared at two widths (invalid module). Reversed order, a plain list, and a bare `=` all worked;
           only the ordered [big-arith, of(i64-arith)] element pair inside a Set/Map build collided.")
  (input  (do (def (main (: n Int64))
                (Set.len (Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2)))))) (export main)))
  (call   main (: 5 Int64))
  (output (: 2 Int64)))

(case "a map built from a BigInt sum key and a BigInt.of-over-arithmetic key has both entries"
  (doc    "The Map twin of the Set slot-clash guard: `(Map.insert (Map.insert (Map.empty) (+ (BigInt.of n)
           (BigInt.of 1)) 1) (BigInt.of (+ n 2)) 2)` with n=5 keys the BigInt values 6 and 7 — two distinct
           keys, so Map.len = 2. The first key is a BigInt sum (an i32 handle scratch); the second is a
           BigInt.of over a checked `(+ n 2)` (an i64 guard temp). The Map.insert emit must advance each
           sibling's scratch floor so the i32 key handle and the i64 arith temp never share one slot.")
  (input  (do (def (main (: n Int64))
                (Map.len (Map.insert (Map.insert (Map.empty) (+ (BigInt.of n) (BigInt.of 1)) 1)
                                     (BigInt.of (+ n 2)) 2))) (export main)))
  (call   main (: 5 Int64))
  (output (: 2 Int64)))
