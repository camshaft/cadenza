; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program That Terminates Ends In One Of Two Terminal Conditions. Results are
; (: <value> <Type>); observation of ordered host calls uses (host-calls ...).
(case
  "structural equality holds component-wise"
  (doc "Witnesses core-semantics.md #Equality Is Structural.")
  (input (= 3 3))
  (output (: true Bool)))

(case
  "constant compound equality folds structurally over Option, None, tuple, and nesting"
  (doc
    "Equality of two CONSTANT compounds folds STRUCTURALLY (core-semantics.md §Equality Is Structural:
           same type + component-wise equal), reducing to a boolean at compile time (was once declined
           'comparison of a compound value needs a heap walk'). Weighted so one result pins six facts:
           (= (Some 1)(Some 1))=T→1, (= (Some 1)(Some 2))=F→2, (= None None)=T→4, (= (tuple 1 2)(tuple 1 2))
           =T→8, (= (tuple 1 2)(tuple 1 3))=F→16, (= (Some (Some 1))(Some (Some 1)))=T→32, summing to 63.
           Relocated from rcdzc constant_compound_equality_folds_and_a_runtime_one_emits_a_heap_walk (its
           runtime-heap-walk compile+import pin stays in rcdzc).")
  (input
    (do
      (def
        (main)
        (+
          (if (= (Some 1) (Some 1)) 1 0)
          (+
            (if (= (Some 1) (Some 2)) 0 2)
            (+
              (if (= None None) 4 0)
              (+
                (if (= #tuple(1 2) #tuple(1 2)) 8 0)
                (+
                  (if (= #tuple(1 2) #tuple(1 3)) 0 16)
                  (if (= (Some (Some 1)) (Some (Some 1))) 32 0)))))))
      (export main)))
  (call main)
  (output (: 63 Int64)))

; Equality of a RUNTIME boolean against a boolean LITERAL: `(= b true)` is `b`, `(= b false)` is `¬b`.
; A Bool has exactly two values, so comparing one to a constant is a boolean coercion (whether the
; compiler folds it to the operand / a negation or emits an i32 compare, the VALUE is the operand or its
; negation). The operand here is a RUNTIME comparison result (`(< a b)`), so this exercises emitted code —
; a value-parity pin across both backends, the equality-against-literal companion of the `(if c false
; true)`→¬c boolean-coercion folds (02-binding-and-control).
(case
  "equality of a runtime boolean against the true literal is the boolean"
  (doc
    "`(= (< a b) true)` equals `(< a b)`: comparing a Bool to `true` yields the Bool itself.
           a=1,b=2 → `1<2`=true → true; a=2,b=1 → false → false. Pins `(= bexpr true)` = bexpr on a
           runtime boolean operand, both backends.")
  (input (do (def (main (: a Int64) (: b Int64)) (= (< a b) true)) (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: true Bool))
  (call main (: 2 Int64) (: 1 Int64))
  (output (: false Bool)))

(case
  "equality of a runtime boolean against the false literal negates it"
  (doc
    "The dual: `(= (< a b) false)` equals `¬(< a b)` — comparing a Bool to `false` negates it.
           a=1,b=2 → `1<2`=true, `= false` → false; a=2,b=1 → false, `= false` → true. Pins `(= bexpr
           false)` = ¬bexpr on a runtime boolean, both backends.")
  (input (do (def (main (: a Int64) (: b Int64)) (= (< a b) false)) (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: false Bool))
  (call main (: 2 Int64) (: 1 Int64))
  (output (: true Bool)))

; The boolean-coercion equality above also composes over a runtime FLOAT `=` — now that runtime scalar
; float equality is realized (the canonical-byte cases below), `(= (= x y) true/false)` nests a runtime
; float compare inside the bool-literal equality. The inner float `=` is the NaN-canonicalizing bit
; compare; the outer `= true`/`= false` coerces/negates its Bool result. These pin the composition (the
; earlier cases used an integer `<` as the inner Bool; these use a float `=`), on both backends.
(case
  "a runtime float equality feeds the true-literal boolean coercion"
  (doc
    "`(= (= x y) true)` over Float64 params: the inner `(= x y)` is the runtime canonical-byte float
           compare, the outer `= true` yields that Bool. (1.5,1.5) → equal → true; (1.5,2.5) → false.
           Pins the bool-literal-equality fold composing over a runtime FLOAT equality operand.")
  (input (do (def (main (: x Float64) (: y Float64)) (= (= x y) true)) (export main)))
  (call main (: 1.5 Float64) (: 1.5 Float64))
  (output (: true Bool))
  (call main (: 1.5 Float64) (: 2.5 Float64))
  (output (: false Bool)))

(case
  "a runtime float equality negated by the false-literal coercion"
  (doc
    "The dual: `(= (= x y) false)` negates the inner float equality — (1.5,1.5) → equal, `= false` →
           false; (1.5,2.5) → not equal, `= false` → true. Pins `(= bexpr false)` = ¬bexpr composing over
           a runtime float `=`, both backends.")
  (input (do (def (main (: x Float64) (: y Float64)) (= (= x y) false)) (export main)))
  (call main (: 1.5 Float64) (: 1.5 Float64))
  (output (: false Bool))
  (call main (: 1.5 Float64) (: 2.5 Float64))
  (output (: true Bool)))

; The same boolean-coercion also composes over a float ORDERING compare (`<`) — DISTINCT from the float
; `=` above. Float ordering is the IEEE PARTIAL order: a NaN operand makes `(< a b)` FALSE (unordered),
; so the inner Bool is not classically-complete. The `= true` coercion returns that Bool unchanged; the
; `= false` coercion NEGATES it — and negating an UNORDERED-false yields TRUE, the adversarial case. A
; fold that reused an equality-style canonical-bit path for the negation, or assumed the ordering compare
; partitions the space (so `¬(a<b)` ⟺ `a>=b`), would MISCOMPILE the NaN pair (where BOTH `a<b` and `a>=b`
; are false, yet `= false` must still flip the false to true). These pin the coercion over a float
; ORDERING operand (the earlier float cases used `=`; the Int cases used a total order), both backends.
(case
  "the true-literal coercion of a float ordering compare returns the compare, NaN stays false"
  (doc
    "`(= (< a b) true)` over Float64 params returns the ordering Bool unchanged: (1.0,2.0) → `1<2`
           true → true; the unordered (nan,1.0) → `nan<1` FALSE → false. Pins `(= bexpr true)` = bexpr
           composing over a float PARTIAL-order compare (not the total-order Int or the float `=` above),
           both backends.")
  (input (do (def (main (: a Float64) (: b Float64)) (= (< a b) true)) (export main)))
  (call main (: 1.0 Float64) (: 2.0 Float64))
  (output (: true Bool))
  (call main (: nan Float64) (: 1.0 Float64))
  (output (: false Bool)))

(case
  "a max-FOLD over a list containing a computed NaN keeps the ordered maximum"
  (doc
    "NaN riding a heap-list FOLD (the pins above are single compares): (if (< best h) h best)
           keeps the ordered max because BOTH NaN compares are false — a fold compiled with a
           total-order compare or a flipped-operand select would let NaN win. NaN mid-list and
           tail faces; both → m=7.0 → 1. (Float64.nan is the canonical NaN source — (/ 0.0 0.0)
           has no value form.)")
  (input
    (do
      (def
        (max-f (: xs (List Float64)) (: best Float64))
        (match xs (#list() best) (#list(h (.. t)) (max-f t (if (< best h) h best)))))
      (def
        (main (: mode Int64))
        (do
          (def nan Float64.nan)
          (def xs (if (= mode 1) #list(3.0 nan 7.0) #list(3.0 7.0 nan)))
          (def m (max-f xs 0.0))
          (if (= m 7.0) 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  ; INTERIM re-pin (v-memory-safety, 2026-08-30): this runtime-list max-fold over-retains 6 on current main
  ; (values 1/1 correct, NO trap — a genuine fold/list-reclaim over-retention, my #1-lever class, NOT a UAF).
  ; It was a CLEAN-pin (live-objects 0) VIOLATION surfaced by #6119's binary grade (clean->leak blocks); the
  ; over-retention is PRE-EXISTING on clean main (git-clean measure = 6, no fix applied) — it confounded the
  ; dqe fix gate (the "max-FOLD-NaN regression" was this pre-existing leak, not the fix). Accepted per
  ; accept-vs-fix policy (value-correct + no-trap = interim known-leak, seq-278). Real fix = the general
  ; fold/list-reclaim drop-pass (v-core-opt batch / my #1 lever) -> tightens to 0. Was (live-objects 0).
  (live-objects 0))

(case
  "a NaN selected through a runtime if-join stays self-equal and unordered downstream"
  (doc
    "NaN through a runtime if-JOIN, both disciplines checked downstream: (= r r) is the
           canonicalizing self-eq (1 in BOTH modes — value-eq, not f64.eq) while (< r 5.0) is the
           IEEE partial order (0 for NaN, 1 for 3.0). The join must neither lose the canonical NaN
           bit pattern nor let the eq/ord disciplines cross-contaminate.")
  (input
    (do
      (def
        (main (: c Int64))
        (do (def r (if (= c 1) Float64.nan 3.0)) (+ (* (if (= r r) 1 0) 10) (if (< r 5.0) 1 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10 Int64))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "the false-literal coercion negates a float ordering compare, turning an unordered pair true"
  (doc
    "The dual and the adversarial case: `(= (< a b) false)` = `¬(< a b)`. Finite ordered (1.0,2.0):
           `1<2` true, `= false` → false. The UNORDERED (nan,1.0): `nan<1` is FALSE, `= false` → TRUE —
           negating an unordered-false. Reversed finite (2.0,1.0): `2<1` false, `= false` → true. Pins that
           the negation acts on the Bool VALUE, not on an assumed `¬(a<b) ⟺ a>=b` (which fails for NaN,
           where both are false); both backends.")
  (input (do (def (main (: a Float64) (: b Float64)) (= (< a b) false)) (export main)))
  (call main (: 1.0 Float64) (: 2.0 Float64))
  (output (: false Bool))
  (call main (: nan Float64) (: 1.0 Float64))
  (output (: true Bool))
  (call main (: 2.0 Float64) (: 1.0 Float64))
  (output (: true Bool)))

(case
  "negative zero is not equal to positive zero"
  (doc
    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           -0.0 and 0.0 have distinct canonical byte forms, so they are not equal.")
  (input (= -0.0 0.0))
  (output (: false Bool)))

(case
  "every not-a-number value is equal to every not-a-number value"
  (doc
    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           all NaN values share one canonical byte form, so they compare equal. `Float64.nan` denotes the
           canonical not-a-number value of that width (options/code-shape/, deterministic-value-form.md).")
  (input (= Float64.nan Float64.nan))
  (output (: true Bool)))

(case
  "a not-a-number value is unequal to a finite float"
  (doc
    "The complement of the nan = nan rule (core-semantics.md #Floating-Point Equality Follows The
           Canonical Byte Form): the canonical NaN byte form differs from every FINITE float's byte form,
           so `(= Float64.nan 1.0)` is false — NaN equals only another NaN, never a finite value. A
           constant fold (both operands compile-time), consumed directly as a Bool.")
  (input (= Float64.nan 1.0))
  (output (: false Bool)))

(case
  "a finite float is unequal to a not-a-number value regardless of operand order"
  (doc
    "The operand-order twin of the case above: `(= 1.0 Float64.nan)` is equally false. `=` is
           symmetric, so the finite-vs-NaN inequality holds with the finite operand on either side — the
           constant fold must not treat the NaN operand specially by position.")
  (input (= 1.0 Float64.nan))
  (output (: false Bool)))

(case
  "a not-a-number leaf makes a compound unequal to the same compound with a finite leaf"
  (doc
    "The nan-vs-finite inequality recurses through a compound: `(tuple Float64.nan)` and `(tuple
           1.0)` differ at their sole leaf (NaN's canonical byte form vs the finite float's), so the
           tuples are unequal — the compound companion of the scalar case above, folded structurally.")
  (input (= #tuple(Float64.nan) #tuple(1.0)))
  (output (: false Bool)))

; --- COMPOUND value-equality over a runtime FLOAT LEAF (a float inside a tuple/sum) -------------------
; The scalar cases above fold at compile time (constant float operands). A RUNTIME float — a def parameter
; — stored in a compound and compared by `=` takes the runtime `value-eq`/`champ_eq` heap-walk. It follows
; the SAME canonical-byte-form semantics as the scalar `Core::FloatCompare` fix, WITHOUT extra machinery:
; the runtime `box-float`/`box-float32` (the sole float-leaf producers) canonicalize-on-construct — every
; NaN collapses to the one canonical quiet-NaN, ±0.0 keep distinct sign bits — so a float leaf already has
; the canonical byte form and the physical `champ_eq` walk is exact. (`ty_heap_walkable` admits a Float
; leaf; before this a compound-float `=` declined "comparison of a compound value needs a heap walk".)
(case
  "compound equality over a runtime float leaf: equal floats compare equal"
  (doc
    "`(= (tuple x 1) (tuple y 1))` over runtime Float64 params `x=y=3.5` — the float leaf is compared
           by the runtime value-eq heap-walk (its canonical byte form), so equal floats in a compound are
           equal → true. Pins runtime compound float equality (was a decline).")
  (input
    (do
      (def (eq (: x Float64) (: y Float64)) (= #tuple(x 1) #tuple(y 1)))
      (def (main) (eq 3.5 3.5))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a runtime float leaf: different floats compare unequal"
  (doc
    "The negative companion: `(= (tuple x) (tuple y))` with `x=3.5`, `y=2.5` — distinct canonical
           byte forms → false. Confirms the compound float walk is genuinely structural, not always-true.")
  (input
    (do
      (def (eq (: x Float64) (: y Float64)) (= #tuple(x) #tuple(y)))
      (def (main) (eq 3.5 2.5))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "compound equality over a runtime NaN float leaf: nan equals nan"
  (doc
    "A runtime NaN leaf in a compound compares EQUAL to another NaN (`box-float` canonicalizes every
           NaN to the one quiet-NaN, so `champ_eq` sees identical bytes) — the compound analogue of the
           scalar `nan == nan` case. `(= (tuple x 1) (tuple Float64.nan 1))` with `x = Float64.nan` → true.")
  (input
    (do
      (def (eq (: x Float64)) (= #tuple(x 1) #tuple(Float64.nan 1)))
      (def (main) (eq Float64.nan))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a runtime float leaf: negative zero is not equal to positive zero"
  (doc
    "`-0.0` and `+0.0` have distinct canonical byte forms (the box keeps the sign bit of a zero), so
           a compound holding `-0.0` is NOT equal to one holding `+0.0` — the compound analogue of the
           scalar `-0.0 != 0.0` case. `(= (tuple x) (tuple y))` with `x = -0.0`, `y = 0.0` → false.")
  (input
    (do
      (def (eq (: x Float64) (: y Float64)) (= #tuple(x) #tuple(y)))
      (def (main) (eq -0.0 0.0))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "equality over a runtime float in a SUM payload compares by the float leaf"
  (doc
    "The variant-payload companion (not only a tuple element): a float carried in a sum variant is
           compared by its canonical byte form through the value-eq walk. `(B.Wrap x)` vs `(B.Wrap y)` with
           `x=y=1.25` → true. Pins that `ty_heap_walkable` admits a Float leaf through a sum variant's
           payload, not just a tuple/record position.")
  (input
    (do
      (type B (Wrap Float64))
      (def (eq (: x Float64) (: y Float64)) (= (B.Wrap x) (B.Wrap y)))
      (def (main) (eq 1.25 1.25))
      (export main)))
  (call main)
  (output (: true Bool)))

; The compound-float-leaf cases above are all Float64. The value-eq heap walk canonicalizes each float leaf
; at ITS OWN declared width (`box-float` at f32 vs f64), so the same NaN-canonicalization / signed-zero
; discrimination must hold for a Float32 leaf, AND for a compound MIXING f32 and f64 leaves where each leaf
; is compared at its own width (the per-leaf-width dispatch the walk performs). These pin that axis:
; Float32-leaf nan==nan + -0.0≠+0.0, and a mixed f32/f64 tuple equal (both NaN) / unequal (differing f64
; leaf). A walk that canonicalized every float leaf at one fixed width would flip the mixed-width faces.
(case
  "compound equality over a runtime FLOAT32 NaN leaf: nan equals nan"
  (doc
    "The Float32 analogue of the compound NaN-leaf case: a runtime Float32 NaN in a tuple compares
           EQUAL to another (`box-float` canonicalizes a NaN at the 32-bit width, so `champ_eq` sees
           identical bytes). `(= (tuple x 1) (tuple Float32.nan 1))` with `x = Float32.nan` → true. Pins
           that the value-eq walk canonicalizes a Float32 leaf at f32 width, not only f64.")
  (input
    (do
      (def (eq (: x Float32)) (= #tuple(x 1) #tuple(Float32.nan 1)))
      (def (main) (eq Float32.nan))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a runtime FLOAT32 leaf: negative zero is not equal to positive zero"
  (doc
    "The Float32 signed-zero face: `-0.0` and `+0.0` at f32 have distinct canonical byte forms (the box
           keeps the zero's sign bit at 32-bit width too), so a tuple holding a Float32 `-0.0` is NOT equal
           to one holding `+0.0`. `(= (tuple x) (tuple y))` with `x = (: -0.0 Float32)`, `y = (: 0.0
           Float32)` → false. Pins signed-zero discrimination at f32 width in the compound walk.")
  (input
    (do
      (def (eq (: x Float32) (: y Float32)) (= #tuple(x) #tuple(y)))
      (def (main) (eq (: -0.0 Float32) (: 0.0 Float32)))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "compound equality over a MIXED Float32/Float64 tuple: each leaf canonicalized at its own width"
  (doc
    "A tuple mixing an f32 and an f64 leaf, both NaN, compares EQUAL — each float leaf is canonicalized
           at ITS OWN declared width by the value-eq walk (the f32 leaf at 32-bit, the f64 leaf at 64-bit),
           so both leaves see the identical canonical NaN bytes for their width. `(= (tuple a b) (tuple
           Float32.nan Float64.nan))` with `a = Float32.nan : Float32`, `b = Float64.nan : Float64` → true.
           Pins the per-leaf-width dispatch — a walk that canonicalized every float leaf at one fixed width
           would misread one of the two leaves.")
  (input
    (do
      (def (eq (: a Float32) (: b Float64)) (= #tuple(a b) #tuple(Float32.nan Float64.nan)))
      (def (main) (eq Float32.nan Float64.nan))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a MIXED Float32/Float64 tuple: a differing f64 leaf makes it unequal"
  (doc
    "The negative companion of the mixed-width case: the f32 leaves match (both NaN) but the f64 leaves
           differ (`1.5` vs `2.5`), so the tuples are unequal → false. Confirms the mixed-width walk is
           genuinely structural per leaf (it does not stop at the first matching leaf, and the f64 leaf is
           compared at its own width). `(= (tuple a b) (tuple Float32.nan (: 2.5 Float64)))` with `a =
           Float32.nan`, `b = (: 1.5 Float64)` → false.")
  (input
    (do
      (def (eq (: a Float32) (: b Float64)) (= #tuple(a b) #tuple(Float32.nan (: 2.5 Float64))))
      (def (main) (eq Float32.nan (: 1.5 Float64)))
      (export main)))
  (call main)
  (output (: false Bool)))

; --- COMPOUND value-equality over a runtime BIGINT / RATIONAL leaf ------------------------------------
; The numeric-tower siblings of the Float-leaf cases. A runtime BigInt is a CANONICAL sign-magnitude byte
; leaf (runtime `box_bigint`, the sole producer), and a runtime Rational is a NORMALIZED 2-BigInt-handle
; node (lowest terms, sign on the numerator — 06-numeric-model "one canonical byte form"). Both are
; canonical BY CONSTRUCTION, so `ty_heap_walkable` admits them and `champ_eq` compares a BigInt leaf by its
; bytes / descends a Rational's two canonical children — exactly the property that made the Float admission
; sound. Before this, a whole-compound `=` over a BigInt/Rational leaf declined "comparison of a compound
; value needs a heap walk" (forcing componentwise comparison — the CAD Rational-redirect blocker). A DIRECT
; scalar BigInt/Rational `=` already worked; this is the NESTED-leaf face.
(case
  "compound equality over a runtime BigInt leaf compares by the canonical bytes"
  (doc
    "`(= (tuple (BigInt.of a) 1) (tuple (BigInt.of b) 1))` over runtime BigInts — the BigInt leaf is
           compared by its canonical sign-magnitude bytes through the value-eq walk. a=b=7 → true; a=7,b=8
           → false. Pins the runtime BigInt compound-`=` face (was a decline).")
  (input
    (do
      (def (eq (: a Int64) (: b Int64)) (= #tuple((BigInt.of a) 1) #tuple((BigInt.of b) 1)))
      (def (main) (eq 7 7))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a runtime BigInt leaf distinguishes different values"
  (doc
    "The negative companion: different BigInts in the tuple → false (a=7, b=8). Confirms the BigInt
           compound walk is genuinely structural, not always-true.")
  (input
    (do
      (def (eq (: a Int64) (: b Int64)) (= #tuple((BigInt.of a) 1) #tuple((BigInt.of b) 1)))
      (def (main) (eq 7 8))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "compound equality over a runtime Rational leaf compares by the normalized form"
  (doc
    "`(= (tuple (Rational.of a 2) 1) (tuple (Rational.of b 2) 1))` — the Rational leaf (a normalized
           2-BigInt-handle node) is compared by `champ_eq` descending its canonical children. a=b=3 → true;
           a=3,b=5 → false. Pins the runtime Rational compound-`=` face.")
  (input
    (do
      (def (eq (: a Int64) (: b Int64)) (= #tuple((Rational.of a 2) 1) #tuple((Rational.of b 2) 1)))
      (def (main) (eq 3 3))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a Rational leaf respects normalization (1/2 = 2/4)"
  (doc
    "The normalization face: `(Rational.of 1 2)` and `(Rational.of 2 4)` both normalize to the lowest-
           terms `1/2` — the SAME canonical node — so a compound holding one equals a compound holding the
           other → true. Confirms the Rational leaf's canonical form (gcd-reduced) is what `champ_eq` walks,
           not the as-written numerator/denominator.")
  (input
    (do (def (main) (= #tuple((Rational.of 1 2) 1) #tuple((Rational.of 2 4) 1))) (export main)))
  (call main)
  (output (: true Bool)))

(case
  "a runtime Rational MAP KEY is found by a normalized-equal key"
  (doc
    "The CHAMP-KEY face of Rational equality (distinct from the tuple-element walk): insert a map under
           the key `(Rational.of 1 2)`, look it up with `(Rational.of 2 4)` — both normalize to the same
           lowest-terms `1/2` node, so `champ_hash`/`champ_eq` place + find them in the same slot → the
           stored 42. Pins that a Rational KEY hashes+matches by its canonical normalized form, not its
           as-written num/den — the path a CAD `Map Rational V` / a Rational-keyed table rests on.")
  (input
    (do
      (def
        (main)
        (Option.expect
          (Map.lookup (Map.insert (Map.empty) (Rational.of 1 2) 42) (Rational.of 2 4))
          "found"))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a GENUINELY-RUNTIME Rational map/set key is found by a normalized-equal key"
  (doc
    "The runtime companion of the bare-Rational-key case above (which builds a CONST `(Rational.of 1 2)`
           that folds): here the key is constructed at RUN TIME — `(Rational.of (if (> c 0) 1 1) 2)` — so its
           numerator arrives via a run-time `if` and the whole Rational cannot fold; it must be normalized by
           the runtime `box_rational_normalized` before its CHAMP key hash. Three faces over that runtime key:
           (1) a MAP insert under it, looked up by `(Rational.of 2 4)` → 42 (normalized-equal, found by
           canonical form); (2) the NEGATIVE control — looked up by a genuinely-different `(Rational.of 1 3)`
           → absent (-1), so it is a real content test, not a blanket hit; (3) the SET twin — `Set.contains`
           of `(Rational.of 2 4)` in a set built by inserting the runtime key → true (1). Encodes them as a
           tuple (42, -1, 1). Pins the runtime construct→normalize→CHAMP-key path (distinct from the const
           fold), the path a runtime-built `Map Rational V` / Rational-keyed table rests on. Both backends.")
  (input
    (do
      (def
        (main (: c Int64))
        (let
          ((half (Rational.of (if (> c 0) 1 1) 2)))
          #tuple((match
              (Map.lookup (Map.insert (Map.empty) half 42) (Rational.of 2 4))
              ((Some v) v)
              ((None u) -1))
            (match
              (Map.lookup (Map.insert (Map.empty) half 42) (Rational.of 1 3))
              ((Some v) v)
              ((None u) -1))
            (if (Set.contains (Set.insert #set() half) (Rational.of 2 4)) 1 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 42 -1 1) (Tuple Int64 Int64 Int64)))
  (live-objects known-leak))

(case
  "a trie of 40 RATIONAL keys with all-different denominators enumerates in numeric order"
  (doc
    "The Rational-key rows above run on 1-2 keys; this pins the canonical ORDER over a populated
           trie: 40 keys `i/(i+1)` — every denominator different, the sequence strictly ascending toward
           1 — must enumerate in NUMERIC order end to end (strictly-increasing walk counting all 40).
           Ordering adjacent fractions like 39/40 vs 40/41 requires genuine cross-multiplication in the
           canonical compare; a per-component (numerator-then-denominator) order would misplace nearly
           every pair. The Rational face of the deep-trie enumeration family.")
  (input
    (do
      (def
        (fill (: i Int64) (: m (Map Rational Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m (Rational.of i (+ i 1)) i))))
      (def
        (inc (: ps (List (Tuple Rational Int64))) (: prev Rational) (: cnt Int64))
        (match
          ps
          (#list() cnt)
          (#list(h (.. t)) (match h (#tuple(k _v) (if (< prev k) (inc t k (+ cnt 1)) -100000))))))
      (def (main (: n Int64)) (inc (Map.to-list (fill n Map.empty)) (Rational.of 0 1) 0))
      (export main)))
  (call main (: 40 Int64))
  (output (: 40 Int64))
  (live-objects known-leak))

(case
  "a Rational-keyed trie churned with DIFFERENTLY-normalized spellings equals the direct build"
  (doc
    "The normalization-identity churn: 29 keys (i = 1..n-1 at n = 30) are INSERTED as `2i/6` and
           REMOVED as `i/3` — differently-written spellings of the same rational — so every removal must
           land on its insert's slot through the canonical form. The survivor (seeded `1/2`) must EQUAL
           the direct build by canonical `=` (10) and still resolve when probed as `2/4` (+1 → 11). Two
           spellings per churn key (insert/remove) and two for the survivor (stored 1/2, probed 2/4),
           all converging on one canonical slot at trie depth — the churn face of the normalized-key
           family.")
  (input
    (do
      (def
        (grow (: i Int64) (: n Int64) (: m (Map Rational Int64)))
        (if (= i n) m (grow (+ i 1) n (Map.insert m (Rational.of (* i 2) 6) i))))
      (def
        (shrink (: i Int64) (: n Int64) (: m (Map Rational Int64)))
        (if (= i n) m (shrink (+ i 1) n (Map.remove m (Rational.of i 3)))))
      (def
        (main (: n Int64))
        (do
          (def direct (Map.insert Map.empty (Rational.of 1 2) 50))
          (def churned (shrink 1 n (grow 1 n direct)))
          (+
            (* 10 (if (= churned direct) 1 0))
            (match
              (Map.lookup churned (Rational.of 2 4))
              ((Some v) (if (= v 50) 1 0))
              ((None _u) -1)))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 11 Int64)))

(case
  "a COMPOUND map key with a Rational leaf hashes+matches by the leaf's normalized form"
  (doc
    "Composes the two faces above: the tuple-element walk (:194/:207) pins Rational `=` inside a tuple,
           and the CHAMP-KEY face (:211) pins a BARE Rational key — this pins their COMPOSITION, a compound
           key `(tuple (Rational.of 1 2) 5)` whose Rational LEAF must normalize on the map-key path. Insert
           under `(tuple 1/2 5)`, look up with `(tuple 2/4 5)` — the tuple's Rational element normalizes to
           the same 1/2 node, so `champ_hash`/`champ_eq` descend into the compound, canonicalize the leaf,
           and find the same slot → 42. A key path that canonicalized a bare Rational but NOT one nested in
           a tuple would false-miss here. The compound-key companion of the bare-Rational-key case.")
  (input
    (do
      (def
        (main)
        (Option.expect
          (Map.lookup
            (Map.insert (Map.empty) #tuple((Rational.of 1 2) 5) 42)
            #tuple((Rational.of 2 4) 5))
          "found"))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a LIST of Rationals as a map key normalizes EVERY element for the key hash"
  (doc
    "The collection-element upgrade of the tuple-leaf case above: the key is a LIST of three
           Rationals, and the probe spells every element differently — stored `[1/2, n/3, 3/4]`, probed
           `[2/4, 2n/6, 9/12]` → 42. The key path must canonicalize each element as the hash walks the
           list (a walk that normalized only the first element, or hashed the spelled forms, would
           miss). Extends the compound-key normalization contract from a fixed tuple slot to an
           arbitrary-length collection's elements.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def stored #list((Rational.of 1 2) (Rational.of n 3) (Rational.of 3 4)))
          (def probe #list((Rational.of 2 4) (Rational.of (* n 2) 6) (Rational.of 9 12)))
          (match (Map.lookup (Map.insert Map.empty stored 42) probe) ((Some v) v) ((None _u) -1))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64)))

(case
  "a MAP-valued key normalizes its Rational VALUES for the outer key hash"
  (doc
    "The deepest face of nested-key normalization: the KEY is itself a map whose VALUES are
           Rationals — stored `{1 ↦ 1/2, 2 ↦ n/3}`, probed `{1 ↦ 3/6, 2 ↦ 3n/9}` → 42. The outer key
           hash walks the inner map's entries, and each entry's VALUE leaf must canonicalize (a hash
           reaching keys but reading value leaves by spelling would miss). Together with the
           list-element case this pins that normalization reaches every leaf position — element, key,
           and value — of a collection-typed key.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def stored (Map.insert (Map.insert Map.empty 1 (Rational.of 1 2)) 2 (Rational.of n 3)))
          (def
            probe
            (Map.insert (Map.insert Map.empty 1 (Rational.of 3 6)) 2 (Rational.of (* n 3) 9)))
          (match (Map.lookup (Map.insert Map.empty stored 42) probe) ((Some v) v) ((None _u) -1))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64)))

(case
  "a SET of BigInts as a map key matches its construction-order and arithmetic twin"
  (doc
    "The BigInt face of the collection-key normalization family: the key is a Set holding one
           MULTI-LIMB BigInt (built `big·2`) and one small (`n`); the probe builds the SAME set with
           the elements written in the OTHER order and the multi-limb member computed as `2·big`
           (commuted operands — an independently-built heap twin) → 42. The set's canonical element
           order absorbs the batch order, and the multi-limb element hashes by numeric content across
           separately-allocated limb buffers. Completes the trio: list elements, map values, and set
           elements all canonicalize inside a collection-typed key.")
  (input
    (do
      (def big (BigInt.of 9223372036854775807))
      (def
        (main (: n Int64))
        (do
          (def stored #set((* big (BigInt.of 2)) (BigInt.of n)))
          (def probe #set((BigInt.of n) (* (BigInt.of 2) big)))
          (match (Map.lookup (Map.insert Map.empty stored 42) probe) ((Some v) v) ((None _u) -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64)))

(case
  "Map.remove of a Rational key canonicalizes: a normalized-equal key removes the entry"
  (doc
    "The DELETE-side companion of the Rational-map-key cases above (which pin lookup/insert
           canonicalization): `Map.remove` must also match the key by its normalized form. Insert under
           `(Rational.of 1 2)` (value 10) and `(Rational.of 1 3)` (value 20), then `Map.remove` with
           `(Rational.of 2 4)` — 2/4 normalizes to the same 1/2 node, so the remove drops the 1/2 entry (its
           lookup is now None) while the sibling 1/3 survives (its lookup is still Some 20). Encoded
           `100*(lookup 1/2 present ? 1 : 0) + (lookup 1/3 value)` = 100*0 + 20 = 20. Pins that remove
           canonicalizes the key on the CHAMP delete path, not only lookup/insert — a remove that hashed the
           as-written 2/4 would miss the 1/2 slot and delete nothing.")
  (input
    (do
      (def
        (main)
        (let
          ((m
              (Map.remove
                (Map.insert (Map.insert (Map.empty) (Rational.of 1 2) 10) (Rational.of 1 3) 20)
                (Rational.of 2 4))))
          (+
            (* 100 (match (Map.lookup m (Rational.of 1 2)) ((Some w) 1) ((None u) 0)))
            (match (Map.lookup m (Rational.of 1 3)) ((Some v) v) ((None u) -1)))))
      (export main)))
  (output (: 20 Int64)))

(case
  "equality over a Rational carried in a SUM payload respects normalization"
  (doc
    "The variant-payload face (a `Vec3r`-shaped value): a Rational in a sum variant is compared by its
           canonical normalized form through the value-eq walk. `(V.Mk (Rational.of 1 2))` equals
           `(V.Mk (Rational.of 2 4))` → true (both normalize to `1/2`); vs `(Rational.of 1 3)` → false. Pins
           that `ty_heap_walkable` admits a Rational leaf through a sum payload, not just a tuple position.")
  (input
    (do
      (type V (Mk Rational))
      (def (eq (: a Rational) (: b Rational)) (= (V.Mk a) (V.Mk b)))
      (def (main) (if (eq (Rational.of 1 2) (Rational.of 2 4)) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a runtime BigInt is found as a Set element by value"
  (doc
    "The Set/CHAMP-element face of BigInt equality: `(BigInt.of 5)` IS a member of a set built with
           `(BigInt.of 5)` → true, `(BigInt.of 9)` is NOT → false. The BigInt element/query compares by its
           canonical sign-magnitude bytes through `champ_eq`/`champ_hash`, so a runtime BigInt key hashes+
           matches its equal — the BigInt companion of the Rational map-key case.")
  (input
    (do
      (def (mem (: x BigInt)) (Set.contains #set((BigInt.of 5)) x))
      (def (main) (mem (BigInt.of 5)))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "a runtime BigInt absent from a Set is not found"
  (doc
    "The negative companion: `(BigInt.of 9)` is NOT in a set holding `(BigInt.of 5)` → false. Confirms
           the BigInt Set-membership is a genuine canonical-byte match, not always-present.")
  (input
    (do
      (def (mem (: x BigInt)) (Set.contains #set((BigInt.of 5)) x))
      (def (main) (mem (BigInt.of 9)))
      (export main)))
  (call main)
  (output (: false Bool)))

(case
  "equality over a compound mixing a float and a Bytes leaf walks both"
  (doc
    "A compound value-eq whose leaves span TWO of the newly-walkable types at once — a Float64 and a
           Bytes — exercises the heap-walk over a heterogeneous compound: `(= (tuple f b) (tuple f b'))`
           where `f=1.5` and `b`/`b'` are the same bytes (one via a `Bytes.concat`-shaped `rep b 0` = `b`).
           Both leaves compare by their canonical byte form → true. Pins that admitting Float AND Bytes in
           `ty_heap_walkable` composes — a mixed-leaf compound walks correctly, not just single-type ones.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (= n 0) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def (eq (: f Float64) (: b Bytes)) (= #tuple(f b) #tuple(f (rep b 0))))
      (def (main) (eq 1.5 (Bytes.of #list(104))))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "compound equality over a runtime SLICE-view Bytes leaf compares by window content"
  (doc
    "The view-leaf member of the mixed-compound walk family: the Bytes leaf inside the compound is a
           runtime-START `Bytes.slice` VIEW, so the per-leaf compare must flatten the view to its window
           content — `(= (tuple 1 s) (tuple 1 flat))` is true exactly when the window equals the flat twin
           (a=1 windows (20,30) → true; a=0 windows (9,20) → false). Pinned across three positions —
           tuple element, Option payload, list element — because the walk reaches leaves through distinct
           descent arms. (The BARE slice as a champ KEY is finding #16 — the compound-eq walk shown here
           is the arm that already canonicalizes; these pins keep it that way through the #16 fix.)")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
          ((Some s)
            (+
              (+
                (* 100 (if (= #tuple(1 s) #tuple(1 (Bytes.of #list(20 30)))) 1 0))
                (* 10 (if (= (Some s) (Some (Bytes.of #list(20 30)))) 1 0)))
              (if (= #list(s) #list((Bytes.of #list(20 30)))) 1 0)))
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 111 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a TUPLE-wrapped runtime slice as a Map key hits by content through the compound descent"
  (doc
    "The champ-KEY composition of the view-leaf walk: the map key is `(tuple 1 <Bytes>)` and the
           probe wraps a runtime slice — the compound-key champ descent flattens the view leaf, so the
           lookup HITS (42). Notable precisely because the BARE slice key misses today (finding #16, the
           top-level Bytes arm): the compound descent's leaf canonicalization is the CORRECT behavior the
           bare arm should share, pinned here so the #16 fix aligns to it rather than regressing it.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((m (Map.insert Map.empty #tuple(1 (Bytes.of #list(20 30))) 42)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) a 2)
            ((Some s) (match (Map.lookup m #tuple(1 s)) ((Some v) v) ((None u) -1)))
            ((None u) -2))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

; --- Runtime compound ORDERING: `<`/`<=`/`>`/`>=` over a runtime compound COMPUTES (blessed lexicographic) --
; The cases above pin runtime structural EQUALITY over a compound (the `value-eq`/`champ_eq` heap walk).
; ORDERING is the total-order companion, now BLESSED for compounds (operator ruling 2026-07-18; core-
; semantics.md #Compound Ordering Is Lexicographic): a tuple/record/list/sum whose components each offer a
; total order offers one too, LEXICOGRAPHICALLY — a tuple/record by field in canonical order, a list element-
; wise with a proper prefix LESS than its extension, a sum by discriminant-as-canonical-byte-form then payload.
; The runtime `value-cmp(a, b, desc)` op (a descriptor-guided three-way walk with the blessed per-leaf orders)
; emits it on wasm; the Rust backend's native derived `Ord` gives the same lexicographic order. A component
; whose leaf offers no total order — a FLOAT (IEEE partial, §319), a Bytes/Char/Set/Map (no blessed order) —
; makes the compound UN-orderable, so such a compound still declines (the reject-don't-miscompile carve-out).
(case
  "a runtime list orders lexicographically by its elements"
  (doc
    "`(< (mk 2) (mk 3))` where `mk` builds a runtime `(list 1 n)` compares two runtime lists element-
           wise: first elements equal (1=1), second decides (2<3) → true → 1. Pins runtime LIST ordering via
           the blessed lexicographic order (core-semantics.md #Compound Ordering Is Lexicographic) — the
           runtime `value-cmp` heap walk on wasm, native `Vec` `Ord` on rust, both agreeing. Was a uniform
           decline before the order was blessed.")
  (input
    (do (def (mk (: n Int64)) #list(1 n)) (def (main) (if (< (mk 2) (mk 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case
  "a runtime list that is a proper prefix orders less than its extension"
  (doc
    "The prefix rule: a list that is a proper PREFIX of another compares LESS (core-semantics.md
           #Compound Ordering Is Lexicographic — shorter-is-less on a common prefix). `(mk true)` = `(list 1)`,
           `(mk false)` = `(list 1 2)`; `[1] < [1,2]` → true → 1. Pins the length tiebreak of the runtime
           list-ordering walk, distinct from the first-differing-element case above.")
  (input
    (do
      (def (mk (: short Bool)) (if short #list(1) #list(1 2)))
      (def (main) (if (< (mk true) (mk false)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime tuple orders lexicographically by field"
  (doc
    "`(tuple 1 n)` compared by field in order: `(1,2) < (1,3)` — first field equal, second decides →
           true → 1. Pins runtime TUPLE ordering (the blessed lexicographic order over a fixed-arity product),
           the `value-cmp` walk on wasm + native tuple `Ord` on rust.")
  (input
    (do (def (mk (: n Int64)) #tuple(1 n)) (def (main) (if (< (mk 2) (mk 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case
  "a runtime sum orders by discriminant then payload"
  (doc
    "A sum orders by its VARIANT DISCRIMINANT first (as the canonical byte form encodes it — declaration
           order), then by payload within the same variant. `type Ord2 = A Int64 | B Int64`; `(A 9) < (B 0)` →
           true (variant A's disc < B's, payload ignored) → 1. Pins runtime SUM ordering, the discriminant-
           then-payload rule of the blessed compound order.")
  (input
    (do
      (type Ord2 (A Int64) (B Int64))
      (def (mk (: pick Bool) (: v Int64)) (if pick (Ord2.A v) (Ord2.B v)))
      (def (main) (if (< (mk true 9) (mk false 0)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a tuple mixing an int, a runtime ROPE, and a FLOAT compares whole by per-kind leaf walks"
  (doc
    "The compound walk dispatches per-LEAF-KIND in one traversal (i64 compare / rope
           content-canonicalize / canonical float-bit compare); the mode-2 face differs ONLY in the
           float leaf so a walk that skipped the third kind passes falsely.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def t1 #tuple(3 (String.concat "ab" "c") 2.5))
          (def t2 (if (= mode 1) #tuple(3 "abc" 2.5) #tuple(3 "abc" 2.6)))
          (if (= t1 t2) 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64)))

(case
  "a tuple key mixing int, rope, and float hashes and matches by all three leaf kinds"
  (doc
    "The CHAMP twin: hash AND eq both dispatch per-leaf-kind — the rope key canonicalizes at
           the champ site, the mode-2 probe differs only in the FLOAT leaf → miss. Adds the MIXED
           row to the tuple-leaf-kind matrix (single-heap-leaf rows landed earlier).")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def m (Map.insert Map.empty #tuple(3 (String.concat "ab" "c") 2.5) 42))
          (match
            (Map.lookup m (if (= mode 1) #tuple(3 "abc" 2.5) #tuple(3 "abc" 2.6)))
            ((Some v) v)
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64)))

(case
  "same-variant sums order by PAYLOAD when the discriminant ties"
  (doc
    "The sum-order pin above has the discriminant DECIDING; this pins the TIE — (A a) < (A b)
           at runtime payloads, both directions + the equal face (the discriminant-then-payload
           rule's second half witnessed).")
  (input
    (do
      (type Ord2 (A Int64) (B Int64))
      (def (main (: a Int64) (: b Int64)) (if (< (Ord2.A a) (Ord2.A b)) 1 0))
      (export main)))
  (call main (: 2 Int64) (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64) (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 2 Int64) (: 2 Int64))
  (output (: 0 Int64)))

(case
  "the compound comparator drives a full INSERTION SORT over runtime tuples"
  (doc
    "The ordering pins above each make ONE comparison; this drives the blessed lexicographic
           `<` as the comparator of a recursive insort over three tuples — {(1,9),(2,5),(2,a)} — and
           reads the MIDDLE of the sorted result per call: a=3 sorts (2,3) before (2,5) (middle 23);
           a=7 sorts it after (middle 25). The tie on field 0 forces the comparator into field 1 mid-
           sort, and each insort step re-runs the full compound compare — the sort-a-table idiom the
           single-comparison pins cannot witness.")
  (input
    (do
      (def
        (insort (: t (Tuple Int64 Int64)) (: q (List (Tuple Int64 Int64))))
        (match
          q
          (#list() #list(t))
          (#list(h (.. rest))
            (if (< t h) (List.concat #list(t) q) (List.concat #list(h) (insort t rest))))))
      (def
        (main (: a Int64))
        (let
          ((sorted (insort #tuple(2 a) (insort #tuple(2 5) (insort #tuple(1 9) #list())))))
          (match (List.at sorted 1) ((Some #tuple(x y)) (+ (* 10 x) y)) ((None u) -1))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 23 Int64))
  (call main (: 7 Int64))
  (output (: 25 Int64))
  (live-objects known-leak))

(case
  "String ordering drives an insort over runtime ROPES, verified by content"
  (doc
    "The String-comparator sort: three concat-built ropes (\"axx\", \"mxx\", \"zxx\" at n=2) insort
           by the blessed content-lexicographic `<`, and the MIDDLE of the sorted result content-equals
           the m-rope (1). Each insort comparison flattens/walks two ROPES (the single-comparison String
           pins can't witness repeated comparator invocation over unflattened trees mid-sort); the
           middle-read catches an order inversion at either end.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (insort (: t String) (: q (List String)))
        (match
          q
          (#list() #list(t))
          (#list(h (.. rest))
            (if (< t h) (List.concat #list(t) q) (List.concat #list(h) (insort t rest))))))
      (def
        (main (: n Int64))
        (let
          ((sorted (insort (rep "m" n) (insort (rep "z" n) (insort (rep "a" n) #list())))))
          (match (List.at sorted 1) ((Some s) (if (= s (rep "m" n)) 1 0)) ((None u) -1))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; The compound-ordering cases above all bottom out in an INTEGER leaf (the numeric leaf order). This pins
; that the compound walk uses the BLESSED per-leaf order for a STRING leaf too — a String's order is
; content-lexicographic over its Unicode scalar values (collections-and-text.md #An ordering over strings…),
; NOT the raw-byte order Bytes is denied. So a tuple whose decisive field is a runtime String orders by that
; String's blessed content order, on both backends (wasm value-cmp walk routes the String leaf to the same
; content compare as scalar `<` on String; rust's native `Vec`/`String` Ord agrees). Confirms the compound
; walk composes the blessed LEAF orders, not just Int — the String-leaf companion of the Int-leaf tuple case.
(case
  "a runtime compound with a String leaf orders by the blessed content-lexicographic String order"
  (doc
    "`(tuple 1 s)` compared by field: the first field (Int 1) ties, so the second (a runtime String s)
           decides — by the BLESSED content-lexicographic String order (§An ordering over strings…), not a
           raw-byte order. `(tuple 1 \"ab\") < (tuple 1 \"ac\")` → true (ab < ac) → 1. Pins that the compound
           `value-cmp` walk routes a String leaf to the same content compare as scalar String `<` (and rust's
           native Ord agrees) — the String-leaf companion of the Int-leaf tuple ordering; contrast Bytes,
           whose order is NOT blessed and declines.")
  (input
    (do
      (def (mk (: s String)) #tuple(1 s))
      (def (main) (if (< (mk "ab") (mk "ac")) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "runtime Bytes ordering is content-lexicographic over unsigned bytes — the blessed total order"
  (doc
    "`(< (mk 2) (mk 3))` where `mk` builds a runtime `(Bytes.of (list n 2))` asks for a three-way order
           on two runtime byte sequences. Bytes has a BLESSED TOTAL ORDER (§order): content-lexicographic over
           its UNSIGNED byte values — the SAME machinery as String/Symbol (all three are byte leaves), realized
           by the runtime `value_cmp_shaped` Bytes arm (over the flattened `raw` slice) and, on rust, `Vec<u8>`'s
           native `Ord`. `mk 2` = `[2,2]`, `mk 3` = `[3,2]`; the first byte 2<3 decides, so `(< [2,2] [3,2])` is
           TRUE → 1. Uniform across all backends (wasm bytes-len/bytes-get walk == rust slice cmp). This REVERSES
           the former uniform decline (operator directive 2026-08-02: 'we need total order on bytes … the
           lexicographic order is the right approach') — Bytes now joins Int/Float/Symbol/String as an orderable
           leaf that also composes soundly inside a compound (unlike a float, whose order is IEEE-partial).")
  (input
    (do
      (def (mk (: n Int64)) (Bytes.of #list((UInt8.wrap n) 2)))
      (def (main) (if (< (mk 2) (mk 3)) 1 0))
      (export main)))
  (output (: 1 Int64)))

; A compound containing a FLOAT leaf offers NO total order — the §319 carve-out, made concrete for a
; compound. core-semantics.md #Ordering Where Offered Is Total: a floating-point type MUST NOT be treated as
; offering an ordering (its relational operators are the IEEE PARTIAL order, not total), and #Compound
; Ordering Is Lexicographic offers a compound's order EXACTLY WHEN every component offers a total order — so
; a float component makes the compound un-orderable and `<` DECLINES (reject-don't-miscompile, uniform across
; backends), rather than manufacturing a byte-form total order that would diverge from the float relational
; ops. The Bytes-declines companion above, on the FLOAT axis (the axis the spec explicitly carves out).
(case
  "a runtime compound containing a float leaf is rejected CDZ0203 for ordering — floats offer no total order (§319)"
  (doc
    "`(< (tuple 1 (Float64.of-int a)) (tuple 1 (Float64.of-int b)))` asks for a three-way order on two
           tuples whose second field is a Float64. A float offers only the IEEE PARTIAL order (§319), and a
           compound is ordered only when EVERY component is (§Compound Ordering Is Lexicographic), so the float
           field makes the tuple un-orderable — a permanent no-total-order carve-out (the family of the
           pure-float compare, Set.to-list, and sum-payload compare), rejected CDZ0203 rather than manufacturing
           a byte-form order that would disagree with the float relational ops. The float-axis companion of the
           Bytes-ordering carve-out; contrast the tuple/list/sum ordering cases above (all-ordered-component
           compounds compute). (Int64→Float64 is `Float64.of-int`; `Float64.of` is the Float→Float width
           conversion — a prior version used it on an Int64, masking this carve-out behind a CDZ0301.)")
  (input
    (do
      (def (mk (: n Int64)) #tuple(1 (Float64.of-int n)))
      (def (main) (if (< (mk 2) (mk 3)) 1 0))
      (export main)))
  ; the CDZ0203 NAMES the carve-out + the actionable route (component-wise ordering), not just the code
  ; (migrated the message facets from rcdzc ordering_a_compound_with_an_unorderable_leaf_…).
  (error CDZ0203
    (message "has no total order, so it cannot be ordered")
    (message "float, set, or map leaf")
    (message "order its orderable components individually"))
  ; it is a PERMANENT carve-out, so the message must NOT read as a temporary "not yet built" limitation.
  (no-diagnostic "not yet built"))

; The two carve-outs above are on the BOOLEAN `<` path; the three-way `compare` mirrors them. A float
; `compare` DECLINES because a floating-point type is the IEEE PARTIAL order (§319 / numeric-model: the
; relational operators are a DISTINCT facility from the total order `compare` reports), so it offers no
; three-way comparison at all — the fix is the relational operators, which DO work on floats. This is the
; three-way twin of the float-compound `<` decline; distinct from it in that a BARE float `<` COMPUTES (the
; IEEE partial order) while a bare float `compare` cannot exist (there is no total order to report).
(case
  "a runtime float compare is a coded CDZ0203 — a float offers the IEEE partial order, not a total three-way"
  (doc
    "`(Ordering.of a b)` over runtime Float64 params asks for a THREE-WAY total-order comparison, but a
           floating-point type offers only the IEEE partial order (a not-a-number is unordered), so it has no
           `compare` — a PERMANENT carve-out, so it is REJECTED with a coded CDZ0203 (reject-don't-miscompile,
           §319), NOT a codeless not-yet decline. Contrast the runtime scalar/String/BigInt/Rational `compare`
           cases, which compute: those types offer a total order, float does not. The actionable path is the
           relational operators `<`/`<=`/`>`/`>=`, which DO work on floats (the IEEE partial order). The
           three-way twin of the float-compound ordering decline above. (Corpus-deprecation BUCKET-2: assert
           the code + the actionable message, replacing the former codeless decline.)")
  (input
    (do
      (def
        (main (: a Float64) (: b Float64))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (export main)))
  ; Pin the ACTIONABLE message IN FULL, not just the code: the diagnostic must NAME the IEEE-partial-order
  ; reason AND redirect to the relational operators that DO order floats, so the reader takes the concrete
  ; route instead of dead-ending at "no total order". The three `(message …)` clauses are AND-required; a
  ; future wording degrade that drops the redirect flips this case (portable-diagnostic-test capability).
  ; The named repair `(< a b)` over Float64 params is witnessed to compile+run clean by the `(= (< a b) true)`
  ; case above (line ~114). (Fully mirrors the former rcdzc rust test
  ; compare_on_a_float_names_the_relational_operators_as_the_fix, now deleted — corpus-covered.)
  (error CDZ0203
    (message "IEEE partial order")
    (message "no three-way comparison")
    (message "`<`, `<=`, `>`, `>=`")))

; The COMPOUND twin of the pure-float `compare` above: a tuple/record/list/sum whose leaves are NOT all
; orderable — here a FLOAT leaf (IEEE partial order only, §319; a Set/Map leaf is the same, no blessed
; order) — has no total order, so the three-way `compare` over it is a PERMANENT carve-out coded CDZ0203
; (the SAME no-total-order family as the pure-float compare above and the compound `<` decline at ~1015),
; NOT a not-yet. Contrast the all-orderable-leaf compound compare just below (Int-leaf tuple), which
; COMPUTES: the ONLY difference is the leaf type (Float64 here vs Int64 there). The diagnostic must NOT
; dead-end at "no total order" — it names the offending leaf kinds AND the actionable route (compare the
; orderable components individually). The three `(message …)` facets are AND-required so a wording degrade
; that drops the leaf-kinds or the route flips this case. Fully mirrors the former rcdzc rust test
; compare_of_a_compound_with_an_unorderable_leaf_names_the_component_wise_route (now deleted, corpus-
; covered) — coded CDZ0203 by #7210 (the three-way twin of #7143's compound-`<` ordering reconcile),
; replacing the former codeless decline (which now stays only for the cross-type/under-resolved fallback).
(case
  "a runtime compound compare with a float leaf is a coded CDZ0203 — the three-way twin of the compound `<` decline"
  (doc
    "`(Ordering.of #tuple(a 1) #tuple(b 1))` over runtime Float64-leaf tuples asks for a THREE-WAY
           total-order comparison, but a float leaf makes the whole compound un-orderable (a float offers only
           the IEEE partial order, §319; a Set/Map leaf carries no blessed order), so it has no `compare` — a
           PERMANENT carve-out rejected CDZ0203 (reject-don't-miscompile), NOT a codeless not-yet. The exact
           compound counterpart of the pure-float compare above and the three-way twin of the compound `<`
           ordering decline (~1015). The actionable route is to compare the orderable components individually
           (the Int leaf alone: `(Ordering.of 1 2)` computes — witnessed by the all-orderable case below).")
  (input
    (do
      (def
        (main (: a Float64) (: b Float64))
        (match
          (Ordering.of #tuple(a 1) #tuple(b 1))
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (export main)))
  (error CDZ0203
    (message "float, set, or map leaf")
    (message "has no total order")
    (message "no three-way `compare`")
    (message "compare its orderable components individually")))

; A runtime COMPOUND `compare` is orderable (all-orderable leaves) but the descriptor-guided `value-cmp`
; three-way heap walk is not wired yet — a genuine NOT-YET (distinct from the float permanent carve-out). The
; boolean compound `<` already COMPUTES via `Core::ValueCmp`; the three-way `compare` over the same value now
; ROUTES to `Core::ValueCmp { op: Prim::Compare }` in lower too (the lower-side is in place), and declines
; cleanly AT EMIT ("ValueCmp carries a non-ordering prim", both backends) until the emit's `op=Compare` arm
; lands (builds the Ordering sum from the walk's -1/0/1 as `res+1` — all-nullary enum discs; owned by v-runtime).
; This case pins the current decline so that emit arm flips it to an executing witness rather than silently
; changing behavior.
(case
  "a runtime compound compare COMPUTES the three-way Ordering via the value-cmp heap walk (§331)"
  (doc
    "`(Ordering.of (tuple a 1) (tuple b 1))` over runtime Int64-leaf tuples yields the three-way `Ordering`
           sum: the descriptor-guided `value-cmp` heap walk returns -1/0/1, and the emit builds the Ordering
           discriminant as `res + 1` (all-nullary enum: Less=disc 0, Equal=1, Greater=2). §331 — the boolean
           compound `<`/`<=`/`>`/`>=` (which already compute via the same walk coerced to bool) and the
           three-way `compare` now surface the SAME total order over a compound. a=1,b=2 → tuple(1,1) <
           tuple(2,1) (first leaf 1<2) → Less → 1. Was a NOT-YET decline (the value-cmp op=Compare emit arm,
           v-runtime) until this landed alongside the lower-side routing; now an executing witness on all
           three backends (wasm res+1; rust/rust-async a nested-if over the derived-Ord compound → Ordering ctor).")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (match
          (Ordering.of #tuple(a 1) #tuple(b 1))
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 1 Int64)))

; The §313-vs-§319 SPLIT made concrete: the SAME float-containing compound that DECLINES ordering (above)
; still EQUALITY-compares. Float EQUALITY follows the canonical byte form (§313, total — NaN canonicalized,
; ±0 distinct), so a float leaf inside a compound is equality-comparable by its bytes even though it is not
; orderable. So a float-compound is eq-comparable but not orderable — the two are NOT the same capability.
(case
  "a runtime compound with a float leaf still equality-compares though it declines ordering (§313 vs §319)"
  (doc
    "The equality companion of the float-compound ordering decline above: `(= (tuple 1 x) (tuple 1 y))`
           over runtime Float64 params — equality follows the canonical byte form (§313, total), so x=y=3.5 →
           the tuples compare EQUAL → 1, even though `<` on the same tuple type DECLINES (§319, floats offer
           no total order). Pins that float EQUALITY (total, canonical byte form) and float ORDERING (IEEE
           partial, not offered) are DISTINCT capabilities — a float-compound is eq-comparable but not
           orderable.")
  (input
    (do (def (main (: x Float64) (: y Float64)) (if (= #tuple(1 x) #tuple(1 y)) 1 0)) (export main)))
  (call main (: 3.5 Float64) (: 3.5 Float64))
  (output (: 1 Int64)))

(case
  "a runtime float is found as a Set element by canonical byte form"
  (doc
    "Set membership over a `Set Float64` with a runtime query: `1.5` IS a member of `(Set.of (list
           1.5 2.5))` → true, `9.9` is NOT → false. The float element/query is compared by its canonical
           byte form through the CHAMP `champ_eq`/`champ_hash` (box-float canonicalizes on construct), so a
           runtime float key hashes+matches its equal — the Set/CHAMP-key face of runtime float equality.")
  (input
    (do
      (def (mem (: x Float64)) (Set.contains #set(1.5 2.5) x))
      (def (main) (mem 1.5))
      (export main)))
  (call main)
  (output (: true Bool)))

(case
  "a runtime float absent from a Set is not found"
  (doc
    "The negative companion: a runtime float `9.9` NOT in `(Set.of (list 1.5 2.5))` → false. Confirms
           the float Set-membership is a genuine canonical-byte match, not always-present.")
  (input
    (do
      (def (mem (: x Float64)) (Set.contains #set(1.5 2.5) x))
      (def (main) (mem 9.9))
      (export main)))
  (call main)
  (output (: false Bool)))

; A `nan` value carries its DECLARING float width — `Float64.nan` is a Float64, `Float32.nan` a Float32 —
; so a CROSS-WIDTH comparison between them (or against a finite float of the other width) is the same
; no-silent-promotion type error a cross-width FINITE comparison is (CDZ0301, numeric-model.md #Numeric
; Types Do Not Silently Promote). `nan` is not width-polymorphic: it must impose its own width on the
; unification exactly as `(: 1.5 Float64)` does, or a Float32-vs-Float64 comparison slips past the check
; the finite case is rejected by. (A SAME-width nan comparison is fine — the case above; only crossing the
; width is the error.)
(case
  "comparing a Float32 nan to a Float64 nan is a cross-width type error"
  (doc
    "`(= Float32.nan Float64.nan)` compares a Float32 value with a Float64 value — distinct float
           types that do not silently unify (CDZ0301), exactly as the finite `(= (: 1.5 Float32) (: 1.5
           Float64))` is rejected. A `Float32.nan` is a Float32 and a `Float64.nan` is a Float64; their
           widths do not unify. Pins that a nan carries its declaring width into the comparison, not an
           unfixed width that would ground to whatever the other operand is.")
  (input (= Float32.nan Float64.nan))
  (error CDZ0301))

(case
  "comparing a Float32 nan to a Float64 finite value is a cross-width type error"
  (doc
    "`(= Float32.nan (: 1.5 Float64))` — a Float32 nan against a Float64 finite value: cross-width,
           so CDZ0301, exactly as `(= (: 1.5 Float32) (: 1.5 Float64))` is. Pins that a nan on EITHER side
           still imposes its declaring float width on the unification (the finite-vs-finite path already
           does), so a mixed nan/finite cross-width comparison is caught, not run to a value.")
  (input (= Float32.nan (: 1.5 Float64)))
  (error CDZ0301))

; A FUNCTION value has no decidable equality — two functions are equal iff they agree on every input, which
; is undecidable — so `=` on a function operand is a TYPE ERROR (CDZ0203, 'this operation is not defined on
; a function value'), not a reference/identity compare and not a run-to-a-value. This holds for a function
; LITERAL and for a function-TYPED parameter alike; the observation `=` is defined only over values with a
; canonical byte form, which a closure does not have.
(case
  "comparing two function literals with = is a type error"
  (doc
    "`(= (fn (x) x) (fn (y) y))` compares two function literals — a function has no decidable equality
           (equal iff equal on every input, undecidable), so `=` is not defined on it: CDZ0203, 'this
           operation is not defined on a function value'. Pins that `=` rejects a function operand rather
           than falling back to a reference/identity compare or running to a value — the observation is over
           values with a canonical byte form, which a closure lacks.")
  (input (do (def (main) (if (= (fn (x) x) (fn (y) y)) 1 0)) (export main)))
  (error CDZ0203))

(case
  "comparing two function-typed parameters with = is a type error"
  (doc
    "The parameter companion: `(= f g)` over two `(-> Int64 Int64)` parameters rejects CDZ0203 too —
           a function is incomparable whether written inline or bound as a parameter. Pins that the
           no-equality-on-functions rule follows the TYPE (a `->` type), not the syntactic form of the
           operand, so a comparison hidden behind a parameter is still caught at the operation.")
  (input
    (do
      (def (cmp (: f (-> Int64 Int64)) (: g (-> Int64 Int64))) (= f g))
      (def (main) (if (cmp (fn (x) x) (fn (y) y)) 1 0))
      (export main)))
  (error CDZ0203))

; --- RUNTIME scalar float equality (not a constant fold) — the canonical-byte BIT compare -----------
; The scalar cases above are CONSTANT operands (they fold in `lower`). These pin the RUNTIME path: two
; Float64/Float32 BOUNDARY PARAMETERS compared with `=`, which cannot fold and must emit the runtime
; compare. The seed does NOT emit IEEE `f64.eq` (which says `nan ≠ nan` and `-0.0 = 0.0` — the OPPOSITE
; of the canonical byte form); it emits a NaN-CANONICALIZING BIT compare — `canon(x) = select(x != x,
; CANON_NAN_BITS, reinterpret_int(x))` then integer `eq` — so `nan == nan` is TRUE and `-0.0 ≠ +0.0` at
; run time, matching the fold. A bare float parameter can carry a non-canonical NaN across the boundary,
; so the canonicalize is load-bearing. Equality only — float ordering (`<`/`>`) awaits a separate ruling.
; = spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
(case
  "a runtime Float64 equality compares by the canonical byte form"
  (doc
    "`def run(x, y) = if (= x y) 1 else 0` over two Float64 boundary parameters — the operands are
           runtime values, so the compare cannot fold and emits the runtime canonical-byte bit compare
           (NOT IEEE `f64.eq`). Equal operands → 1; unequal → 0. Pins that runtime scalar float equality
           is realized (was a decline: 'comparison of a compound value needs a heap walk'), the root of
           the long-standing scalar-Float-`==` gap.")
  (input (do (def (run (: x Float64) (: y Float64)) (if (= x y) 1 0)) (export run)))
  (call run (: 1.5 Float64) (: 1.5 Float64))
  (output (: 1 Int64))
  (call run (: 1.5 Float64) (: 2.5 Float64))
  (output (: 0 Int64)))

(case
  "a runtime negative zero is not equal to positive zero"
  (doc
    "The runtime companion of `(= -0.0 0.0)` = false: with `-0.0` and `0.0` arriving as runtime
           Float64 parameters, the canonical-byte bit compare keeps their sign bits distinct → NOT equal
           (0). An IEEE `f64.eq` emit would wrongly answer equal (1) — this pins the runtime path uses the
           canonical byte form, not the machine float-equal. `0.0` vs `0.0` → equal (1), the control.")
  (input (do (def (run (: x Float64) (: y Float64)) (if (= x y) 1 0)) (export run)))
  (call run (: -0.0 Float64) (: 0.0 Float64))
  (output (: 0 Int64))
  (call run (: 0.0 Float64) (: 0.0 Float64))
  (output (: 1 Int64)))

(case
  "a runtime NaN equals a runtime NaN under the canonical byte form"
  (doc
    "The sharpest runtime case: two `Float64.nan` values through boundary parameters compare EQUAL
           (1) — every NaN shares one canonical byte form. An IEEE `f64.eq` emit answers the OPPOSITE
           (`nan ≠ nan` → 0), so this pins that the runtime compare canonicalizes the NaN before the bit
           compare rather than emitting the machine float-equal. The runtime analogue of `(= Float64.nan
           Float64.nan)` = true.")
  (input (do (def (run (: x Float64) (: y Float64)) (if (= x y) 1 0)) (export run)))
  (call run (: nan Float64) (: nan Float64))
  (output (: 1 Int64))
  (call run (: nan Float64) (: 1.5 Float64))
  (output (: 0 Int64)))

(case
  "a runtime Float32 equality compares by the canonical byte form"
  (doc
    "The Float32 companion: the runtime compare canonicalizes at binary32 (an `i32.reinterpret_f32`
           bit compare with the Float32 canonical NaN), so `nan == nan` is true and `-0.0 ≠ +0.0` at the
           narrower width too. Pins the runtime float compare is width-correct (F32 uses i32 ops, not the
           f64 path).")
  (input (do (def (run (: x Float32) (: y Float32)) (if (= x y) 1 0)) (export run)))
  (call run (: 1.5 Float32) (: 1.5 Float32))
  (output (: 1 Int64))
  (call run (: -0.0 Float32) (: 0.0 Float32))
  (output (: 0 Int64)))

; --- Float ORDERING (`< <= > >=`) uses IEEE PARTIAL order, DISTINCT from the canonical-byte equality -----
; Operator ruling (2026-07-16): float ordering is a PARTIAL order (IEEE partialOrd), NOT total. A NaN
; operand is UNORDERED — every relational op with a NaN yields FALSE (it EVALUATES to false, does NOT trap
; or decline). `-0.0` and `+0.0` compare EQUAL-under-ordering (`-0.0 <= +0.0` true, neither strictly less).
; This DISAGREES with the canonical-byte EQUALITY above on BOTH NaN and signed zero — `==` says nan==nan
; TRUE / -0.0 != +0.0, ordering says nan<nan FALSE / -0.0 ==ord +0.0. That divergence is inherent to float
; (bit-equality vs numeric-ordering are different relations); these cases pin BOTH relations explicitly at
; the divergence points so a lowering can't silently converge them (e.g. by reusing the equality's
; canonical-bit compare for ordering, which would give the wrong signed-zero + NaN answers). Ordering emits
; the RAW IEEE `f64.lt/le/gt/ge` (wasm) / native Rust `<`/etc. — both give NaN→false, -0.0 ==ord +0.0.
; = spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
(case
  "runtime float ordering is a strict/non-strict partial order over finite values"
  (doc
    "`run(a,b) = if (< a b) 1 0` over Float64 boundary params: 1.0 < 2.0 → 1, 2.0 < 1.0 → 0, and the
           equal case 1.5 < 1.5 → 0 (strict). Pins runtime float `<` is realized (was declining
           'compound heap walk') and gives the ordinary order over finite operands.")
  (input (do (def (run (: a Float64) (: b Float64)) (if (< a b) 1 0)) (export run)))
  (call run (: 1.0 Float64) (: 2.0 Float64))
  (output (: 1 Int64))
  (call run (: 2.0 Float64) (: 1.0 Float64))
  (output (: 0 Int64))
  (call run (: 1.5 Float64) (: 1.5 Float64))
  (output (: 0 Int64)))

(case
  "a NaN operand makes every runtime float ordering relation false (unordered)"
  (doc
    "IEEE partial order: NaN is unordered, so a relational op with a NaN operand yields FALSE — it
           EVALUATES (not trap/decline). `run(a,b) = if (< a b) 1 0`: nan < 1.0 → 0, 1.0 < nan → 0, nan <
           nan → 0. This is the OPPOSITE of what a total-order reading (which declined a NaN ordering)
           would do, and DISTINCT from equality (`(= nan nan)` is TRUE) — pins the ordering's NaN case.")
  (input (do (def (run (: a Float64) (: b Float64)) (if (< a b) 1 0)) (export run)))
  (call run (: nan Float64) (: 1.0 Float64))
  (output (: 0 Int64))
  (call run (: 1.0 Float64) (: nan Float64))
  (output (: 0 Int64))
  (call run (: nan Float64) (: nan Float64))
  (output (: 0 Int64)))

(case
  "runtime float ordering treats negative and positive zero as equal, unlike equality"
  (doc
    "The signed-zero DIVERGENCE: under ORDERING `-0.0` and `+0.0` are EQUAL — `run(a,b) = if (<= a b)
           1 0` gives -0.0 <= 0.0 → 1 AND 0.0 <= -0.0 → 1 (neither strictly less, so both `<=` hold). This
           DISAGREES with EQUALITY, where `(= -0.0 0.0)` is FALSE (distinct canonical byte forms). Pinning
           both here makes the disagreement intentional: ordering uses IEEE partial (raw `f64.le`, -0.0
           ==ord +0.0), equality uses the canonical byte form (sign-significant). A `<` between them is
           false both ways (equal → not strictly less).")
  (input
    (do
      (def (le (: a Float64) (: b Float64)) (if (<= a b) 1 0))
      (def (lt (: a Float64) (: b Float64)) (if (< a b) 1 0))
      (def (run (: a Float64) (: b Float64)) (+ (* 10 (le a b)) (lt a b)))
      (export run)))
  (call run (: -0.0 Float64) (: 0.0 Float64))
  (output (: 10 Int64))
  (call run (: 0.0 Float64) (: -0.0 Float64))
  (output (: 10 Int64)))

(case
  "a constant float ordering with a NaN operand folds to false, not a decline"
  (doc
    "The CONST-fold companion of the runtime NaN-ordering case: a compile-time `(< Float64.nan 1.0)`
           now FOLDS to false (NaN unordered → false) rather than DECLINING as it did under the total-order
           reading. `run() = if (< Float64.nan 1.0) 1 0` → 0. Pins that the ordering ruling applies to the
           fold path too — the relational op always evaluates.")
  (input (do (def (run) (if (< Float64.nan 1.0) 1 0)) (export run)))
  (output (: 0 Int64)))

; --- Self-comparison (`x ⋈ x`) is the adversarial fold boundary: reflexivity is NOT universal under NaN --
; An optimizer that knows a relation's algebra is tempted to fold a self-comparison to a constant: `x <= x`
; → true (reflexive), `x < x` → false (irreflexive). That fold is a MISCOMPILE for float: NaN is unordered,
; so `nan <= nan` is FALSE — the `<=`/`>=` self-fold to `true` gives the wrong answer, and even `<`/`>`
; (which DO fold to false universally) must stay false for NaN, not accidentally become the reflexive-true
; sibling. These pin all four operators on a runtime self-operand at both a finite value (reflexivity holds)
; and NaN (reflexivity BREAKS), on BOTH backends, so no backend-independent algebraic pass may replace an
; `x ⋈ x` with a constant. The author's ordering cases above pin `<`/`<=` on DISTINCT operands + NaN; these
; pin the SELF case (same SSA value both sides) and the `>`/`>=` runtime mirror the arithmetic cases omit.
(case
  "runtime <= on a self operand is true for finite but FALSE for NaN (no reflexivity fold)"
  (doc
    "`run(x) = if (<= x x) 1 0`: x=1.5 → 1 (reflexive on a finite float), x=nan → 0 (NaN is unordered,
           so `nan <= nan` is FALSE). Pins that a self-comparison `x <= x` must NOT fold to the constant
           `true` — the reflexivity that holds for finite floats BREAKS for NaN, and the ordering evaluates
           to false rather than declining. Same value on both sides, so it also guards a CSE that dedups the
           operands then mis-concludes equality-ergo-reflexive.")
  (input (do (def (run (: x Float64)) (if (<= x x) 1 0)) (export run)))
  (call run (: 1.5 Float64))
  (output (: 1 Int64))
  (call run (: nan Float64))
  (output (: 0 Int64)))

(case
  "runtime < on a self operand is false for finite AND NaN (irreflexive, stays false)"
  (doc
    "`run(x) = if (< x x) 1 0`: x=1.5 → 0 (strict order is irreflexive) and x=nan → 0 (unordered).
           `x < x` DOES fold to false universally — but pin it so a pass that flips a self-`<` into its
           reflexive-`<=` sibling (which would give 1 on the finite case) is caught. Both inputs → 0.")
  (input (do (def (run (: x Float64)) (if (< x x) 1 0)) (export run)))
  (call run (: 1.5 Float64))
  (output (: 0 Int64))
  (call run (: nan Float64))
  (output (: 0 Int64)))

(case
  "runtime >= and > on a self operand mirror <= and < including the NaN self case"
  (doc
    "The `>=`/`>` mirror of the two self-operand cases — the ordering cases above only exercised
           `<`/`<=`. `run(x) = 10*(if (>= x x) 1 0) + (if (> x x) 1 0)`: x=1.5 → 10 (>= reflexive true, >
           irreflexive false), x=nan → 0 (both false, NaN unordered). Pins `>=` does NOT self-fold to true
           and `>` stays false, on both backends.")
  (input
    (do
      (def (ge (: x Float64)) (if (>= x x) 1 0))
      (def (gt (: x Float64)) (if (> x x) 1 0))
      (def (run (: x Float64)) (+ (* 10 (ge x)) (gt x)))
      (export run)))
  (call run (: 1.5 Float64))
  (output (: 10 Int64))
  (call run (: nan Float64))
  (output (: 0 Int64)))

; --- `<=` is NOT `(< or =)` for FLOAT: the two relations DIVERGE on NaN, so the derivation MISCOMPILES --
; The sharpest trap at the intersection of the two float relations pinned above. A tempting algebraic
; identity — `a <= b` ⟺ `a < b ∨ a = b` — HOLDS for integers (total order) but FAILS for float, because
; the `=` on the right is the CANONICAL-BYTE equality (nan = nan is TRUE, the cases below) while `<=` is
; the IEEE ordering (nan <= nan is FALSE, unordered). So for a NaN self-pair: `<=` gives FALSE, but
; `(< ∨ =)` gives `(false ∨ TRUE)` = TRUE — they disagree. A Core pass that rewrote `<=`/`>=` into a
; disjunction of `<`/`>` with `=` (a plausible simplification, e.g. to reuse one comparison primitive)
; would MISCOMPILE every NaN case. These pin the divergence explicitly — `<=` and `(< or =)` computed
; SIDE BY SIDE in one program, differing on NaN — so no lowering may substitute one for the other, both
; backends. (Ordering `<=` treats -0.0 ==ord +0.0 too, a second divergence from canonical-byte `=`.)
(case
  "float <= is NOT (< or =): they diverge on NaN because canonical-byte equality says nan = nan"
  (doc
    "`run(a,b) = 10*(if (<= a b) 1 0) + (if (or (< a b) (= a b)) 1 0)` computes the ordering `<=` and
           the derived `(< ∨ =)` side by side. Finite equal (1.5,1.5): both true → 11. Finite ordered
           (1.0,2.0): `1<=2` true, `1<2 ∨ 1=2` true → 11. The NaN pair (nan,nan): `nan<=nan` is FALSE
           (unordered) but `nan<nan ∨ nan=nan` = `false ∨ TRUE` = TRUE → 1 (le=0, oreq=1). Pins that `<=`
           must NOT be rewritten to `(< or =)` — they disagree on NaN — both backends.")
  (input
    (do
      (def (le (: a Float64) (: b Float64)) (if (<= a b) 1 0))
      (def (oreq (: a Float64) (: b Float64)) (if (or (< a b) (= a b)) 1 0))
      (def (run (: a Float64) (: b Float64)) (+ (* 10 (le a b)) (oreq a b)))
      (export run)))
  (call run (: 1.5 Float64) (: 1.5 Float64))
  (output (: 11 Int64))
  (call run (: 1.0 Float64) (: 2.0 Float64))
  (output (: 11 Int64))
  (call run (: nan Float64) (: nan Float64))
  (output (: 1 Int64)))

(case
  "float >= is NOT (> or =): the same NaN divergence mirrors on the greater-or-equal side"
  (doc
    "The `>=` mirror: `run(a,b) = 10*(if (>= a b) 1 0) + (if (or (> a b) (= a b)) 1 0)`. Finite equal
           (2.0,2.0): both true → 11. The NaN pair (nan,nan): `nan>=nan` FALSE (unordered) but
           `nan>nan ∨ nan=nan` = `false ∨ TRUE` = TRUE → 1. Pins `>=` must not be rewritten to `(> or =)`,
           both backends.")
  (input
    (do
      (def (ge (: a Float64) (: b Float64)) (if (>= a b) 1 0))
      (def (oreq (: a Float64) (: b Float64)) (if (or (> a b) (= a b)) 1 0))
      (def (run (: a Float64) (: b Float64)) (+ (* 10 (ge a b)) (oreq a b)))
      (export run)))
  (call run (: 2.0 Float64) (: 2.0 Float64))
  (output (: 11 Int64))
  (call run (: nan Float64) (: nan Float64))
  (output (: 1 Int64)))

; --- Float ordering is NOT TRANSITIVE through a NaN: a chained `a < b < c` can't fold to `a < c` --------
; Another algebraic identity that holds for a total order but fails for the float PARTIAL order:
; TRANSITIVITY. For integers `(a < b) ∧ (b < c)` ⟹ `a < c`, so a pass could drop the middle test or fold
; the chain to the endpoints. For float this is UNSOUND: if the MIDDLE operand `b` is NaN, both `a < b`
; and `b < c` are FALSE (unordered), so the conjunction is false — but `a < c` over the finite endpoints
; may be TRUE. So `(and (< a b) (< b c))` and `(< a c)` disagree when `b` is NaN (e.g. a=1, b=nan, c=3:
; the chain is false, `1 < 3` is true). A Core pass that folded a chained ordering by transitivity — or
; dropped the `b` comparison as "implied" — would MISCOMPILE. This pins the chain must evaluate BOTH links
; (the middle operand's NaN-ness is observable), both backends.
(case
  "a chained float ordering is not foldable by transitivity — a NaN middle breaks the chain"
  (doc
    "`run(a,b,c) = if (and (< a b) (< b c)) 1 0` — a chained `a < b < c`. Fully ordered (1,2,3): both
           links true → 1. A NaN MIDDLE (1,nan,3): `1<nan` FALSE and `nan<3` FALSE → the conjunction is 0,
           even though the finite endpoints satisfy `1 < 3` — so the chain must NOT be folded to `(< a c)`
           (which would give 1). Descending (3,2,1): `3<2` false → 0. Pins float ordering is not transitive
           through a NaN, so both links are evaluated, both backends.")
  (input
    (do
      (def (run (: a Float64) (: b Float64) (: c Float64)) (if (and (< a b) (< b c)) 1 0))
      (export run)))
  (call run (: 1.0 Float64) (: 2.0 Float64) (: 3.0 Float64))
  (output (: 1 Int64))
  (call run (: 1.0 Float64) (: nan Float64) (: 3.0 Float64))
  (output (: 0 Int64))
  (call run (: 3.0 Float64) (: 2.0 Float64) (: 1.0 Float64))
  (output (: 0 Int64)))

; --- Float equality follows the canonical byte form RECURSIVELY, inside compound values --
; #Equality Is Structural: "Two values MUST be equal when they have the same type and their contents
; are equal component-wise" — and each float COMPONENT is compared by #Floating-Point Equality Follows
; The Canonical Byte Form (every NaN equal to every NaN; -0.0 distinct from 0.0). The scalar cases
; above pin the float rule at top level; these pin that structural equality applies the SAME rule to a
; float NESTED in a tuple/list/record/sum, not a naive f64.eq. This is the sharpest adversarial float
; case: a lowering that recurses into a compound with wasm's f64.eq gives the OPPOSITE answer for both
; NaN (f64.eq says nan≠nan → the tuples wrongly unequal) and -0.0 (f64.eq says -0.0=0.0 → wrongly
; equal). The seed's `cval_eq` recurses through `float_canonical_eq`, so it must match the scalar rule.
(case
  "a NaN nested in a tuple compares equal under the canonical byte form"
  (doc
    "`(= (tuple Float64.nan) (tuple Float64.nan))` = true: structural equality compares the tuples
           component-wise (core-semantics.md #Equality Is Structural), and the float component follows the
           canonical-byte-form rule where every NaN equals every NaN — exactly as the scalar
           `(= Float64.nan Float64.nan)` does. A recursion using wasm's f64.eq would answer false (nan ≠
           nan); this pins the canonical-byte-form rule holds for a float INSIDE a compound.")
  (input (= #tuple(Float64.nan) #tuple(Float64.nan)))
  (output (: true Bool)))

(case
  "a negative zero nested in a tuple is distinct from positive zero"
  (doc
    "`(= (tuple -0.0) (tuple 0.0))` = false: the float components -0.0 and 0.0 have distinct
           canonical byte forms, so the tuples are unequal — the compound companion of the scalar
           `(= -0.0 0.0)` = false. A recursion using wasm's f64.eq would answer true (-0.0 = 0.0),
           silently collapsing the distinction the canonical byte form preserves.")
  (input (= #tuple(-0.0) #tuple(0.0)))
  (output (: false Bool)))

(case
  "identical negative zeros nested in a tuple compare equal"
  (doc
    "The control the case above pairs with: `(= (tuple -0.0) (tuple -0.0))` = true — two -0.0
           components share one canonical byte form, so the tuples are equal. Confirms the nested
           comparison is a genuine value test (true for matching -0.0, false against 0.0), not a
           blanket answer.")
  (input (= #tuple(-0.0) #tuple(-0.0)))
  (output (: true Bool)))

; The nested-equality cases above compare CONSTANT compounds (they fold). These pin the RUNTIME heap-walk
; through DEEP nesting: a compound built from a boundary parameter (so it cannot fold) compared component-
; wise down multiple levels — a record inside a tuple, and three tuple levels deep. The value-eq walk must
; descend to the runtime leaf and compare it, the shape a structural-equality check over a built IR node
; takes.
(case
  "a runtime record nested in a tuple compares component-wise"
  (doc
    "`(= (tuple (record (x n) (y 2)) 5) (tuple (record (x 3) (y 2)) 5))` with `n` a boundary parameter
           — the tuples cannot fold, so the runtime `value-eq` walk descends: tuple element 0 is a record
           whose field `x` is the runtime `n`. n=3 → the records (hence tuples) are equal → true; n=9 →
           `x` differs → false. Pins that the structural-equality walk recurses through a RECORD nested in a
           TUPLE at run time (a heap value inside a heap value), comparing the runtime leaf.")
  (input
    (do
      (def
        (main (: n Int64))
        (= #tuple(#record((= x n) (= y 2)) 5) #tuple(#record((= x 3) (= y 2)) 5)))
      (export main)))
  (call main (: 3 Int64))
  (output (: true Bool))
  (call main (: 9 Int64))
  (output (: false Bool)))

(case
  "a runtime three-level nested tuple compares equal by a deep walk"
  (doc
    "Three tuple levels deep: `(= (tuple 1 (tuple 2 (tuple n 4))) (tuple 1 (tuple 2 (tuple 3 4))))`
           with `n` a parameter. The `value-eq` walk descends all three levels to reach `n` — n=3 → equal
           at every level → true; n=9 → the innermost element differs → false. Pins that the deep structural
           walk reaches a leaf several nesting levels down at run time, not only one level.")
  (input
    (do
      (def (main (: n Int64)) (= #tuple(1 #tuple(2 #tuple(n 4))) #tuple(1 #tuple(2 #tuple(3 4)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: true Bool))
  (call main (: 9 Int64))
  (output (: false Bool)))

; --- RUNTIME compound equality with a FLOAT leaf — the canonical-byte rule through the heap walk -------
; The nested-float cases far above are CONSTANT compounds (they fold via const_compound_eq). These pin the
; RUNTIME heap-walk over a float leaf: a compound built from a boundary Float parameter cannot fold, so the
; `value-eq`/`champ_eq` walk must compare the float leaf. That walk is a RAW-BYTE compare — correct ONLY
; because a float boxed into a heap value is CANONICALIZED at construction (`op_box_float` normalizes a NaN
; to one canonical byte form and preserves a zero's sign), so the nested-runtime answer matches the scalar
; `Core::FloatCompare` and the constant fold: `nan == nan` TRUE, `-0.0 != +0.0`. Before, `Ty::Float` was
; excluded from `ty_heap_walkable` (the decline predated the canonicalize-on-construct invariant), so a
; runtime float leaf in a compound `=` declined "comparison of a compound value needs a heap walk".
; = spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
(case
  "a runtime float leaf in a tuple compares by the canonical byte form"
  (doc
    "`(= (tuple a) (tuple b))` over two Float64 boundary parameters — the tuples cannot fold, so the
           `value-eq` heap walk compares the boxed float leaves. Equal floats → the tuples are equal (1);
           unequal → 0. Pins that a runtime float leaf in a compound is walkable (was a decline), the
           compound companion of runtime scalar float equality.")
  (input
    (do (def (main (: a Float64) (: b Float64)) (if (= #tuple(a) #tuple(b)) 1 0)) (export main)))
  (call main (: 1.5 Float64) (: 1.5 Float64))
  (output (: 1 Int64))
  (call main (: 1.5 Float64) (: 2.5 Float64))
  (output (: 0 Int64)))

(case
  "a runtime NaN leaf in a tuple compares equal, a runtime -0.0 leaf stays distinct"
  (doc
    "The sharp canonical-byte cases through the RUNTIME heap walk: `(= (tuple a) (tuple b))` with a,b
           runtime Float64. Two NaN leaves compare EQUAL (1) — box-float canonicalized both to one byte
           form, so the raw-byte `champ_eq` sees identical bytes — and a -0.0 leaf against a +0.0 leaf
           stays UNEQUAL (0), their sign bits preserved. A heap walk using a raw IEEE compare would answer
           the OPPOSITE for both. Pins the nested-runtime float rule agrees with the scalar `FloatCompare`
           and the constant fold.")
  (input
    (do (def (main (: a Float64) (: b Float64)) (if (= #tuple(a) #tuple(b)) 1 0)) (export main)))
  (call main (: nan Float64) (: nan Float64))
  (output (: 1 Int64))
  (call main (: -0.0 Float64) (: 0.0 Float64))
  (output (: 0 Int64)))

(case
  "a NaN nested in a list compares equal under the canonical byte form"
  (doc
    "The list companion: `(= (list Float64.nan 1.0) (list Float64.nan 1.0))` = true — element-wise
           equality compares nan against nan (equal, canonical byte form) and 1.0 against 1.0 (equal), so the
           lists are equal. Pins that the canonical-byte-form float rule recurses through list elements
           too, alongside an ordinary equal float element.")
  (input (= #list(Float64.nan 1.0) #list(Float64.nan 1.0)))
  (output (: true Bool)))

(case
  "a NaN nested in a sum payload compares equal under the canonical byte form"
  (doc
    "The sum companion: `(= (Some Float64.nan) (Some Float64.nan))` = true — the variant tags match
           (both Some) and the payloads compare by the canonical-byte-form rule where nan equals nan. Pins that
           structural equality applies the float rule to a Sum's payload, not only to tuple/list
           elements.")
  (input (= (Some Float64.nan) (Some Float64.nan)))
  (output (: true Bool)))

(case
  "a RUNTIME list of floats compares equal element-wise (the value-eq-shaped walk)"
  (doc
    "The literal-list case above CONST-FOLDS; this pins the RUNTIME path — a `List Float64` built from a
           boundary Float parameter (no fold). `champ_eq` (`value-eq`) is UNSOUND for a list (an RRB spine is
           element- but not shape-canonical) and `value-cmp` DECLINES a float (no total order), so this routes
           to the descriptor-guided `value-eq-shaped` element-wise walk: `(list x x) = (list x x)` compares each
           float element by canonical byte form → true. Built via `(list x x)` on a runtime param so no operand
           folds. Expected: true.")
  (input (do (def (main (: x Float64)) (= #list(x x) #list(x x))) (export main)))
  (call main (: 3.5 Float64))
  (output (: true Bool)))

(case
  "a RUNTIME list of floats with a NaN element compares equal (value-eq-shaped canonical byte form)"
  (doc
    "The runtime-list NaN face: a `List Float64` holding a runtime NaN — `x` forced to NaN via `(- x
           Float64.nan)` so the list is genuinely runtime-built — compares equal to another such list, because
           the value-eq-shaped walk canonicalizes each float leaf (`nan == nan`), NOT a raw `f64.eq` (which
           would answer false). Distinguishes the shaped walk's float handling from a bit compare. Expected:
           true.")
  (input
    (do
      (def (main (: x Float64)) (= #list((- x Float64.nan)) #list((- x Float64.nan))))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: true Bool)))

(case
  "a RUNTIME list of floats with a differing element compares unequal (value-eq-shaped)"
  (doc
    "The not-equal face of the runtime value-eq-shaped list walk: two `List Float64` values built from
           DIFFERENT boundary params differ in their sole element, so the element-wise walk finds the mismatch
           → false. Guards that the walk actually compares elements (a blanket true would pass the equal cases).
           Expected: false.")
  (input (do (def (main (: a Float64) (: b Float64)) (= #list(a) #list(b))) (export main)))
  (call main (: 3.5 Float64) (: 2.5 Float64))
  (output (: false Bool)))

; A SUM whose variants carry a non-byte-canonical `List` AND a non-orderable `Float` (the `Ast` shape:
; `Ast.List (List Ast)` + `Ast.Float Float64`) falls between the two cheaper runtime-= paths — `value-eq`
; (champ_eq) is unsound for its List payload (an RRB spine is element- not shape-canonical), and `value-cmp`
; declines its Float payload (no total ORDER). So a runtime `=` on it routes to the descriptor-guided
; `value-eq-shaped` walk, which descends a Sum (discriminant then payload) and compares a float leaf by
; canonical byte form. The `value-eq-shaped` classification now descends a Sum (both `ty_contains_list` and
; `eq_shaped_walkable`), so an Ast-shaped value admits the walk the runtime already implements.
(case
  "a runtime structural = on a sum with List and Float payloads walks it (value-eq-shaped over a Sum)"
  (doc
    "`(= (Ast.Int n) (Ast.Int 3))` over a runtime `n` — the `Ast` sum has an `Ast.List (List Ast)`
           variant (so `value-eq`/champ_eq is unsound — a list spine is not byte-canonical) AND an `Ast.Float`
           variant (so `value-cmp` declines — a float has no total order), so it takes the `value-eq-shaped`
           element-wise walk, which descends the Sum by discriminant-then-payload. Same variant + equal payload
           → true (n=3 → 1), a differing payload → false (n=5 → 0). Regression witness for the runtime `=` on a
           sum falling between both cheaper paths (declined 'needs a heap walk (not yet built)'); the fix
           descends a Sum in the value-eq-shaped classification, routing to the runtime walk's existing
           Shape::Sum arm. (Wasm computes; the Rust backend's structural-eq walk renders a NON-recursive
           float/list-carrying sum, but `Ast` is RECURSIVE (`Ast.List (List Ast)`) and the rust emit expands
           the walk inline — which would loop on a recursive sum — so it declines cleanly there: a graded
           todo, reject-don't-miscompile, pending a named-helper-fn emit. The non-recursive sum companion just
           below DOES compute on rust.)")
  (input
    (do
      (type Ast (Int Int64) (Float Float64) (List (List Ast)))
      (def (mk (: n Int64)) (Ast.Int n))
      (def (main (: n Int64)) (if (= (mk n) (Ast.Int 3)) 1 0))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a runtime = on a NON-recursive sum with a Float payload walks it on both backends"
  (doc
    "The non-recursive companion of the Ast case: a sum `FV` with an `FV.F Float64` variant (so NOT
           native-Eq, and `value-cmp` declines the float) but NO recursive/List variant — so BOTH backends
           compute it via the value-eq-shaped Sum walk (wasm's iterative runtime walk; the rust emit expands
           a `match (l, r)` over the two variants, comparing the float payload by canonical byte form). Same
           variant + equal payload → true; a different variant → false. `(FV.I n) = (FV.I 3)` → true at n=3
           (→ 1); `(FV.F 2.5) = (FV.F 2.5)` → true (→ 1, the float leaf by canonical bytes); `(FV.I n) =
           (FV.F 2.5)` → false (→ 0, discriminant differs). Pins that a NON-recursive float-carrying sum's
           runtime `=` computes on all three backends (distinct from the recursive Ast, which is rust-todo).")
  (input
    (do
      (type FV (I Int64) (F Float64))
      (def
        (main (: n Int64))
        #tuple((if (= (FV.I n) (FV.I 3)) 1 0)
          (if (= (FV.F 2.5) (FV.F 2.5)) 1 0)
          (if (= (FV.I n) (FV.F 2.5)) 1 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: (tuple 1 1 0) (Tuple Int64 Int64 Int64)))
  (live-objects known-leak))

(case
  "a runtime = on a RECURSIVE-through-List sum walks runtime-built trees on every backend"
  (doc
    "The recursive companion of the Ast decline note above, now that the rust emit routes a
           monomorphic user sum through a generated recursive helper (call-indirection, so a
           self-referential sum no longer expands the inline match unboundedly): `(type Ast (Lit Int64)
           (Node (List Ast)))` — recursive THROUGH a List element — and two runtime-built two-child trees
           `(Node (list (Lit a) (Lit (+ a 1))))` compare structurally: equal at `a = 5`, unequal at
           `a = 7`. The walk descends sum → list spine → each child sum → payload. Pins the recursive
           sum-through-collection equality on all three backends.")
  (input
    (do
      (type Ast (Lit Int64) (Node (List Ast)))
      (def (mk (: n Int64)) (Node #list((Lit n) (Lit (+ n 1)))))
      (def (main (: a Int64)) (= (mk a) (mk 5)))
      (export main)))
  (call main (: 5 Int64))
  (output (: true Bool))
  (call main (: 7 Int64))
  (output (: false Bool)))

(case
  "a runtime = on a sum recursive through a RECORD-of-TUPLE payload walks structurally"
  (doc
    "The record-payload face of the recursive-sum walk: `(type Tree (Leaf) (Branch (Record (: v
           Int64) (: kids (Tuple Tree Tree)))))` — the recursion re-enters through a record FIELD holding a
           tuple of children, so the walk must descend sum → record (sorted fields) → tuple → child sums.
           Runtime-built one-branch trees compare equal at `a = 5`, unequal at `a = 7`. With the
           list-element case above, pins both re-entry wrappers (collection spine and record/tuple fields)
           of the recursive equality walk.")
  (input
    (do
      (type Tree (Leaf) (Branch (Record (: v Int64) (: kids (Tuple Tree Tree)))))
      (def (mk (: n Int64)) (Branch #record((= v n) (= kids #tuple((Leaf) (Leaf))))))
      (def (main (: a Int64)) (= (mk a) (mk 5)))
      (export main)))
  (call main (: 5 Int64))
  (output (: true Bool))
  (call main (: 7 Int64))
  (output (: false Bool)))

(case
  "a runtime = over MUTUALLY-recursive sums walks across the type cycle"
  (doc
    "The mutual-recursion face: `(type E (Num Int64) (Neg T))` and `(type T (Wrap E))` recurse
           through EACH OTHER, so the equality walk (and on rust, the generated per-type helper fns) must
           handle a type CYCLE spanning two declarations — `__eq_E` calling `__eq_T` calling `__eq_E` —
           not only direct self-reference. `(Neg (Wrap (Num a)))` compares equal at `a = 4`, unequal at
           `a = 6`. A cycle-detection keyed on a single type (or helpers generated per-type without
           cross-references) would either loop or decline this shape.")
  (input
    (do
      (type E (Num Int64) (Neg T))
      (type T (Wrap E))
      (def (mk (: n Int64)) (Neg (Wrap (Num n))))
      (def (main (: a Int64)) (= (mk a) (mk 4)))
      (export main)))
  (call main (: 4 Int64))
  (output (: true Bool))
  (call main (: 6 Int64))
  (output (: false Bool)))

(case
  "a negative zero in a record field is distinct from positive zero"
  (doc
    "The record companion of the nested -0.0 case: `(= (record (x -0.0)) (record (x 0.0)))` =
           false — the field `x` holds -0.0 in one record and 0.0 in the other, distinct canonical byte
           forms, so the records are unequal. Pins the canonical-byte-form float distinction through a
           record field, the field-access analogue of the tuple-element case.")
  (input (= #record((= x -0.0)) #record((= x 0.0))))
  (output (: false Bool)))

; The runtime float-leaf cases above are ONE level deep (a float directly in a tuple/list/sum/record). These
; pin the canonical-byte float compare TWO levels deep — a float leaf in a record-of-tuple — so the heap walk
; keeps canonicalizing the float at depth, not only at the top level. A walk that stopped applying the
; canonical-byte rule below depth 1 would give the WRONG NaN/-0.0 answer for a nested float (nan!=nan or
; -0.0==+0.0). Both operands built from a boundary Float parameter (genuinely runtime, no fold).
(case
  "a runtime NaN float leaf two levels deep (a record-of-tuple) compares equal by the canonical byte form"
  (doc
    "The DEPTH companion of the single-level float-leaf cases: a NaN two levels down — `(record (t
           (tuple <nan> 3)))` — must still compare by the canonical byte form (`nan == nan`) → true. `x` is a
           boundary Float parameter forced to NaN via `(- x Float64.nan)` so the compound is runtime-built
           (no fold). Pins the heap walk canonicalizes a float leaf at depth 2, not only depth 1 — a walk
           that raw-compared a nested float would answer false (nan != nan under a bit compare).")
  (input
    (do
      (def
        (main (: x Float64))
        (= #record((= t #tuple((- x Float64.nan) 3))) #record((= t #tuple((- x Float64.nan) 3)))))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: true Bool)))

(case
  "a runtime -0.0 float leaf two levels deep stays distinct from positive zero"
  (doc
    "The signed-zero DEPTH companion: `-0.0` two levels down — `(record (t (tuple -0.0 3)))` vs the same
           with `0.0` — stays DISTINCT (canonical byte forms differ) → false, even nested. `z` is a boundary
           Float parameter (`(* z -0.0)` yields -0.0 at runtime, no fold). Pins the canonical byte distinction
           for signed zero holds at depth 2 — a walk that stopped distinguishing signed zero below the top
           level would wrongly answer true.")
  (input
    (do
      (def
        (main (: z Float64))
        (= #record((= t #tuple((* z -0.0) 3))) #record((= t #tuple(0.0 3)))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: false Bool)))

; Float64 equality is a REALIZED seed capability (options/realized-capability-set/: "Float64
; literals/equality"), so it must hold for a RUNTIME float operand — one from a function parameter,
; a call, an if — not only for two compile-time-constant literals. The cases above compare constant
; floats; these compare a runtime float against a constant. The seed emits only the CONSTANT float
; equality (folded at compile time) and declines a runtime one ("non-constant float equality
; (canonical byte form) not yet emitted") — a not-yet-emitted runtime path within a realized
; capability. The value itself is carried correctly (a runtime float identity `(f 3.5)` → 3.5); only
; the equality comparison of a runtime float is missing.
(case
  "runtime float equality compares by canonical byte form"
  (doc
    "`f` takes a Float64 parameter and compares it to the literal 3.5; f(3.5) is true. Float
           equality is realized (options/realized-capability-set/), so it must apply to a runtime
           float operand, matching the canonical-byte-form comparison the constant cases above use.
           The seed declines (\"non-constant float equality … not yet emitted\") — it folds constant
           float equality but has not emitted the runtime comparison.")
  (input (do (def (f x) (= x 3.5)) (def (main) (f 3.5)) (export main)))
  (output (: true Bool)))

(case
  "runtime float inequality compares by canonical byte form"
  (doc
    "The companion with an unequal runtime operand: f(2.5) compares 2.5 to 3.5 and is false.
           Confirms the runtime float comparison is a genuine value test (true for 3.5, false for
           2.5), not a constant fold. The seed declines the same way.")
  (input (do (def (f x) (= x 3.5)) (def (main) (f 2.5)) (export main)))
  (output (: false Bool)))

; --- Equality of two RUNTIME strings, neither a compile-time literal --------------------------
; core-semantics.md #Equality Is Structural + #String Equality Follows Normalized Contents: `=` on
; two String operands compares their normalized contents. The top-of-file string-equality cases
; (13-strings.sexp) compare two LITERAL strings, folded at compile time; a comparison in which AT
; LEAST ONE operand is a literal also folds (the compiler holds one side statically). The demanding
; shape is two operands that are BOTH runtime values — two function parameters, two payload-bound
; names — with no literal to fold against: a String is a Bytes-backed heap value, so comparing two of
; them is a runtime heap walk. The seed folds the literal cases but declines the two-runtime case
; ("runtime compound equality (heap walk) not yet emitted") — a not-yet-emitted runtime path within a
; realized capability, the String companion of the runtime-float-equality cases above. A program that
; compares names it read from data (a symbol table, an AST node's head, a proof term's variable name)
; hits exactly this; the recorded true/false is the oracle a generation that emits the heap-walk
; comparison reproduces.
(case
  "two runtime strings compare equal by their contents"
  (doc
    "`eq2` compares its two String PARAMETERS — both runtime values, neither a literal the
           compiler can fold against. `(eq2 \"foo\" \"foo\")` is true. String equality is realized
           (collections-and-text.md #String Equality Follows Normalized Contents), so it must hold when
           BOTH operands are runtime, not only when one side is a literal (which folds). The seed
           declines (\"runtime compound equality (heap walk) not yet emitted\"): it folds a literal-side
           comparison but has not emitted the two-runtime heap walk. A program comparing two names read
           from data takes this shape.")
  (input (do (def (eq2 a b) (= a b)) (def (main) (eq2 "foo" "foo")) (export main)))
  (output (: true Bool)))

(case
  "two unequal runtime strings compare false by their contents"
  (doc
    "The companion with unequal runtime operands: `(eq2 \"foo\" \"bar\")` is false. Confirms the
           two-runtime string comparison is a genuine content test, not a constant fold (true for equal
           contents, false for different). The seed declines the same way as the equal case.")
  (input (do (def (eq2 a b) (= a b)) (def (main) (eq2 "foo" "bar")) (export main)))
  (output (: false Bool)))

(case
  "a runtime string compared against a literal folds against the literal side"
  (doc
    "The control the two cases above must be distinguished from: when ONE operand is a literal,
           the comparison folds against that side and the seed compiles it. `f` compares its String
           parameter to the literal \"x\"; `(f \"x\")` is true. Pins that the runtime-string equality
           gap is specifically the BOTH-runtime case — a literal on either side is already emitted.")
  (input (do (def (f s) (= s "x")) (def (main) (f "x")) (export main)))
  (output (: true Bool)))

(case
  "a runtime string bound from a sum payload compares equal to a string parameter"
  (doc
    "The two-runtime-string case above compares two direct PARAMETERS; this compares a String bound
           from a SUM-VARIANT PAYLOAD (`s` from `(Wrap.Wrap s)`) against a String parameter (`name`) —
           still two runtime operands with no literal to fold, but one is now a heap value extracted from a
           constructor payload rather than a bare parameter. `(payload-is (Wrap.Wrap \"foo\") \"foo\")` is
           true by String equality (collections-and-text.md #String Equality Follows Normalized Contents).
           A generation that emits the two-runtime heap walk for bare parameters but not for a
           payload-extracted operand declines here (\"runtime compound equality (heap walk) not yet
           emitted\") — the payload/aliased-operand companion of the two-parameter case; a program that
           compares a name it destructured from a data node against an expected name takes exactly this
           shape.")
  (input
    (do
      (type Wrap (Wrap String))
      (def (payload-is w name) (match w ((Wrap.Wrap s) (= s name))))
      (def (main) (payload-is (Wrap.Wrap "foo") "foo"))
      (export main)))
  (output (: true Bool)))

; --- Equality of two RUNTIME compound values (a heap walk over the value heap) -----------------
; core-semantics.md #Equality Is Structural: two values are equal when they have the same type and
; their contents are equal component-wise; #Values Are Equal … agrees with the canonical byte form. The
; component-wise cases above compare compound values built from LITERALS (folded at compile time). The
; demanding shape is two compound values BUILT AT RUN TIME — a sum/record/tuple whose contents come
; from a parameter or a call — so the comparison is a walk over two heap values, not a constant fold.
; The compiler emits this as the runtime `value-eq` op (the tagless `champ_eq` walk the map/set key path
; already uses): equal iff same shape AND equal component-wise, discriminant before payload. It is
; realized for a compound whose LEAVES are all SCALAR (Int/Bool/Unit) — canonical by construction — so a
; program comparing two runtime AST nodes / proof terms / records for structural equality runs; a
; compound carrying a collection/text leaf (a List/Bytes/String, whose canonical form needs a
; compaction) still declines (a later increment). The first two cases below `(= (mk 1) (mk 1))` INLINE
; their tiny builder and fold to a constant (so they pass by the fold, not the walk); the recursion-built
; cases that follow defeat the fold and genuinely exercise `value-eq`.
(case
  "two runtime sum values compare equal by a heap walk"
  (doc
    "`mk` builds a runtime sum `(N.I n)` from its parameter, so both operands of `(= (mk 1) (mk
           1))` are heap values, not folded constants. Structural equality (core-semantics.md #Equality
           Is Structural) makes them equal, so the program is true. The seed declines (\"runtime
           compound equality (heap walk) not yet emitted\"): it folds equality of compile-time-known
           compounds but has not emitted the runtime heap walk. The runtime-compound companion of the
           runtime-float and two-runtime-string equality cases above — all three are the same
           not-yet-emitted runtime comparison. A generation emitting the heap walk reproduces true.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (main) (if (= (mk 1) (mk 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "two differing runtime sum values compare unequal by a heap walk"
  (doc
    "The companion with unequal runtime compounds: `(mk 1)` is `(N.I 1)` and `(mk2 2)` is `(N.I
           2)`, so the heap walk finds their payloads differ and the comparison is false → 0. Confirms
           the runtime compound comparison is a genuine structural test, not a constant fold. The seed
           declines the same way as the equal case.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (main) (if (= (mk 1) (mk 2)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "a runtime sum whose payload comes from recursion compares equal by a heap walk"
  (doc
    "The GENUINELY non-foldable sum-equality shape — the two `mk`/`if`-shaped cases above inline
           their tiny builder and reduce to a CONSTANT compound the compiler folds, so they never reach
           the runtime `value-eq` path. Here one operand's payload is `(sumto 3)` = 3+2+1 = 6, a value
           produced by RECURSION the compiler cannot fold to a literal, so `(N.I (sumto 3))` is a genuine
           heap value; comparing it to `(N.I 6)` walks the two heap sums. Equal discriminant AND equal
           payload → true → 1 (core-semantics.md #Equality Is Structural). Pins that `=` emits the
           runtime structural comparison (`value-eq`), not only the compile-time fold — the observable
           of the heap walk the two cases above document but do not exercise.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
      (def (main) (if (= (N.I (sumto 3)) (N.I 6)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a runtime sum whose payload comes from recursion compares unequal by a heap walk"
  (doc
    "The unequal companion of the recursion-built heap walk: `(sumto 3)` = 6, so `(N.I (sumto 3))`
           carries 6 while `(N.I 7)` carries 7 — the heap walk finds the payloads differ and the
           comparison is false → 0. Confirms `value-eq` is a genuine content test on the recursion-built
           (unfoldable) operand, not a fold that happened to say true. The discriminant agrees (both `I`),
           so this isolates the PAYLOAD comparison.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
      (def (main) (if (= (N.I (sumto 3)) (N.I 7)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "two recursion-built linked lists compare equal by a deep heap walk"
  (doc
    "The RECURSIVE-SUM heap walk: `build n` constructs a descending cons-list `[n, n-1, …, 1]`
           whose length and spine are decided at run time (no fixed literal spine to fold), so `(build
           3)` is a genuine multi-node heap value. `(= (build 3) (build 3))` walks BOTH cons-lists
           node-by-node — each `Cons` tuple's head and tail, recursively to `Nil` — and finds them
           structurally equal → 1. This is the deep-structure shape a self-hosted compiler comparing two
           AST subtrees takes; it CANNOT fold (the spine is runtime-built). Pins that `value-eq` recurses
           through a nested recursive sum, not just a one-level payload. Both operands are OWNED
           temporaries the borrowing compare must reclaim (no leak).")
  (input
    (do
      (type IntList (Cons (Tuple Int64 IntList)) Nil)
      (def (build n) (if (< n 1) (IntList.Nil ()) (IntList.Cons #tuple(n (build (- n 1))))))
      (def (main) (if (= (build 3) (build 3)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "two recursion-built linked lists of different lengths compare unequal by a heap walk"
  (doc
    "The unequal companion of the deep list walk: `(build 3)` = `[3,2,1]` and `(build 2)` =
           `[2,1]` differ at the FIRST node (head 3 vs 2, and different spine length), so the heap walk
           returns false → 0 without needing to prove the whole structure. Confirms the recursive
           `value-eq` is a genuine structural comparison over the runtime-built spine, not a fold.")
  (input
    (do
      (type IntList (Cons (Tuple Int64 IntList)) Nil)
      (def (build n) (if (< n 1) (IntList.Nil ()) (IntList.Cons #tuple(n (build (- n 1))))))
      (def (main) (if (= (build 3) (build 2)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "recursive-sum equality is decided by RUNTIME parameters, both regimes in one export"
  (doc
    "The parameterized face of the recursive-sum heap walk: the deep-list cases above compare
           builds with CONSTANT arguments (equal or unequal is fixed at authoring time); here `(= (mk a)
           (mk b))` over RUNTIME `a`/`b` — a Peano-style `(type Nat (Z) (S Nat))` built by recursion —
           reports true at (3,3) and false at (3,2) from ONE compiled comparison. Pins that the emitted
           walk is a genuine runtime decision over both spines, not a specialization per call site.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
      (def (main (: a Int64) (: b Int64)) (= (mk a) (mk b)))
      (export main)))
  (call main (: 3 Int64) (: 3 Int64))
  (output (: true Bool))
  (call main (: 3 Int64) (: 2 Int64))
  (output (: false Bool)))

(case
  "recursive-sum equality discriminates a difference at the DEEPEST node"
  (doc
    "The full-spine-walk guard: the unequal deep-list case above differs at the FIRST node (the
           walk may return false immediately); here two 4-node lists agree on every head EXCEPT the
           LAST — `(mk 3 x)` builds `[3,2,1,x]` and the comparand is `[3,2,1,99]` — so the walk must
           recurse through all the equal prefix nodes to find the tail difference. True at `x = 99`,
           false at `x = 7`. A walk that short-circuited on prefix equality (or compared only k levels)
           would report true for both calls.")
  (input
    (do
      (type L (Nil) (Cons Int64 L))
      (def
        (mk (: n Int64) (: last Int64))
        (if (= n 0) (Cons last (Nil)) (Cons n (mk (- n 1) last))))
      (def (main (: x Int64)) (= (mk 3 x) (mk 3 99)))
      (export main)))
  (call main (: 99 Int64))
  (output (: true Bool))
  (call main (: 7 Int64))
  (output (: false Bool)))

(case
  "compound equality short-circuits at the first differing element — a later trapping element is not forced"
  (doc
    "The short-circuit complement of the deepest-node walk: `(= (tuple 1 (/ 5 d)) (tuple 9 9))` compares
           element 0 first (1 vs 9) — they DIFFER, so the result is decided FALSE without element 1, whose
           `(/ 5 d)` at d=0 is a divide-by-zero. The trapping element is UNOBSERVED (its value cannot change the
           already-decided result), so it is NOT forced and its trap does NOT occur (core-semantics.md #A Trap
           Occurs Only Where Its Computation Is Observed). At d=0 the comparison is false, NOT a trap.")
  (input (do (def (main (: d Int64)) (= #tuple(1 (/ 5 d)) #tuple(9 9))) (export main)))
  (call main (: 0 Int64))
  (output (: false Bool)))

(case
  "compound equality forces through an equal-prefix element to the deciding element, whose trap occurs"
  (doc
    "The anchor to the short-circuit case: `(= (tuple 9 (/ 5 d)) (tuple 9 9))` — element 0 is EQUAL (9 = 9),
           so the comparison must continue to element 1 to decide, forcing `(/ 5 d)`; at d=0 that is a
           divide-by-zero, so the comparison TRAPS. Pins that short-circuit stops at the first DIFFERENCE only —
           an equal prefix is forced through, and the first not-yet-decided element IS observed.")
  (input (do (def (main (: d Int64)) (= #tuple(9 (/ 5 d)) #tuple(9 9))) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "list construction strictly evaluates its element arguments — a trapping argument traps in an = operand"
  (doc
    "Operator ruling — strict list construction: evaluating a `(list …)` expression evaluates every element
           ARGUMENT, so a trapping argument traps whenever the constructor is reached, independent of the consumer
           and of any comparison short-circuit. `(= (list 9 (/ 5 d)) (list 9 9))` at d=0 constructs the left operand,
           evaluating `(/ 5 d)` BEFORE `=` runs, so it TRAPS on divide-by-zero — the trap is a property of
           constructing the list operand, not of the comparison reaching element 1 (core-semantics.md #A Trap Occurs
           Only Where Its Computation Is Observed — a heap-collection constructor is strict in its element
           arguments; the optimizer may elide the heap object but must preserve argument evaluation). The same
           holds for `(= (list 1 (/ 5 d)) (list 9 9))` @d=0: a trapping argument in a constructed list operand
           traps even past the first difference — the outcome does not depend on the comparison. A structural `=`
           may still short-circuit WHICH already-evaluated values it inspects, but that does not defer a constructed
           operand's argument evaluation. Contrast a tuple (`(= (tuple 1 (/ 5 d)) (tuple 9 9))` @d=0 → false, above):
           tuple/record construction is lazy in an unprojected element, a heap-collection constructor is not.")
  (input (do (def (main (: d Int64)) (= #list(9 (/ 5 d)) #list(9 9))) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "list construction traps at a first-DIFFERING position too — the = short-circuit does not save the trapping arg"
  (doc
    "The first-difference twin of the case above, pinning the runtime path fixed by the const-eq fold decline
           (v-core-opt CASE1, #5241): `(= (list 1 (/ 5 d)) (list 9 9))` — element 0 DIFFERS (1 vs 9), so a lazy
           comparison would decide FALSE at element 0 and never force `(/ 5 d)`. Under strict list construction
           (operator ruling A, #5194) the left operand is CONSTRUCTED — evaluating `(/ 5 d)` — before `=` runs, so
           at d=0 it TRAPS on divide-by-zero (it previously folded to false, dropping the trap; the fold now
           DECLINES for a list/set/map operand with a trap-possible arg and routes to the materializing runtime
           value-eq). At d=1 the list is `(list 1 5)`, which differs from `(list 9 9)` at element 0 → false. Pins
           that the eq short-circuit governs only WHICH already-evaluated values are inspected, never whether a
           constructed operand's arguments are evaluated. Contrast the tuple short-circuit case above, which stays
           false — tuple/record construction is lazy in an unprojected element, a heap-collection constructor is not.")
  (input (do (def (main (: d Int64)) (= #list(1 (/ 5 d)) #list(9 9))) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 1 Int64))
  (output (: false Bool)))

(case
  "list construction strictly evaluates its element arguments — a trapping argument traps under List.len"
  (doc
    "The consumer-independent companion: `(List.len (list 1 (/ 5 d)))` at d=0 TRAPS because constructing the
           list evaluates its element arguments — `(/ 5 d)` — even though only the length is taken and the heap
           object may be elided (operator ruling — strict list construction; core-semantics.md #A Trap Occurs Only
           Where Its Computation Is Observed). The list analog of 19-sets `(Set.len (Set.of (list (/ 5 d) 2 3)))`:
           same construction, ANY consumer — the trapping argument traps.")
  (input (do (def (main (: d Int64)) (List.len #list(1 (/ 5 d)))) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a dead-let list construction with a trapping scalar arg traps — args forced though the list is discarded"
  (doc
    "The bound-and-discarded face of strict list construction (operator ruling A, #5194; v-core-opt CASE2
           #5328): `(let ((x #list(1 (/ 5 d)))) 0)` binds a list to `x`, DISCARDS it, and returns 0 — yet the
           construction still evaluates its element ARGUMENTS, so at d=0 the trapping `(/ 5 d)` TRAPS even though
           the list object is never observed (the optimizer elides the allocation but force-evaluates the
           trap-possible arg rather than §283-eliding it — decompose-and-mark, not build-and-reclaim). At d=1 the
           arg is fine and the discarded list folds away → 0. A PURE dead list (no trapping/effectful arg) is
           still fully elided (§283). Completes the consumer-independent strict-construction set alongside the
           List.len and `=` operand cases above — same construction, ANY consumer (even discard), the trapping
           argument traps. (Interim backend limitation: only SCALAR trap args are forced here; a heap-PRODUCING
           trap arg — e.g. a Rational.of overflow — inside a dead discarded ctor is not yet forced, pinned once
           the backend extends forcing to heap args.)")
  (input (do (def (main (: d Int64)) (let ((x #list(1 (/ 5 d)))) 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 1 Int64))
  (output (: 0 Int64)))

(case
  "a dead-let SET construction with a trapping scalar arg traps — a set constructor is strict in its element args too"
  (doc
    "The set twin of the scalar dead-let list case above: strict heap-collection construction is not
           list-specific. `(let ((x #set(1 (/ 5 d)))) 0)` binds a SET to `x`, DISCARDS it, and returns 0 — yet at
           d=0 the trapping `(/ 5 d)` TRAPS even though the set is never observed (operator ruling A #5194; a
           `(list/set/map …)` constructor evaluates its element arguments whenever reached, in any consumer, even
           discard — core-semantics.md #A Trap Occurs Only Where Its Computation Is Observed). At d=1 the arg is
           fine and the discarded set folds away → 0. Pins that a future set-specific construction path (dedup /
           canonicalization) cannot quietly drop the element-argument evaluation the list path preserves; the set
           consumer face is otherwise pinned via 19-sets `(Set.len (Set.of (list (/ 5 d) 2 3)))`. Scalar trap arg
           (backend-independent — green on wasm, rust, rust-async).")
  (input (do (def (main (: d Int64)) (let ((x #set(1 (/ 5 d)))) 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 1 Int64))
  (output (: 0 Int64)))

(case
  "a dead-let list construction with a HEAP-producing trapping arg also traps — force-eval extends to heap leaves"
  (doc
    "The heap-producing-arg companion of the scalar dead-let case above (v-core-opt #5339 closed the
           documented interim gap): `(let ((x #list((Rational.of 1 d)))) 0)` binds a list whose element is a
           HEAP-producing `Rational.of`, DISCARDS `x`, and returns 0 — yet at d=0 the zero-denominator
           `(Rational.of 1 0)` TRAPS (its `unreachable` kind, the same trap the DIRECT `(Rational.of 1 0)`
           raises), because strict list construction now force-evaluates a trap-possible HEAP-producing element
           arg too, not only scalars (the discarded fresh handle is rc-reclaimed, no leak). At d=1 the arg is
           fine and the discarded list folds away → 0. Completes ruling-A (#5194) strict dead-ctor argument
           forcing over BOTH scalar and heap-producing element args.")
  (input (do (def (main (: d Int64)) (let ((x #list((Rational.of 1 d)))) 0)) (export main)))
  (call main (: 0 Int64))
  (trap "unreachable")
  (call main (: 1 Int64))
  (output (: 0 Int64)))

(case
  "list construction strictly evaluates an EFFECTFUL element argument — the perform runs even when the list is discarded"
  (doc
    "The effect companion of the strict-construction rule (operator ruling — strict list construction;
           core-semantics.md #A Trap Occurs Only Where Its Computation Is Observed says a heap-collection ctor
           evaluates its element arguments so their traps AND EFFECTS occur, regardless of consumer, including
           when the collection is bound and then discarded). Here the second element `((. P acc) 5)` PERFORMS
           `acc 5`, threading 5 into the handler's state; the list `x` is then DISCARDED and `main` returns
           `((. P rd))` = the state. Result 5 proves the perform RAN at construction even though `x` is never
           observed — the effectful element argument is strict. GREEN regression guard: it must stay 5 so a
           strict-construction fix for pure-trapping arguments does not regress the already-strict effect path.")
  (input
    (do
      (effect P (op acc (-> Int64 Int64)) (op rd (-> Int64)))
      (def
        (main)
        (handle
          P
          (: 0 Int64)
          ((acc (v) s (resume v (+ s v))) (rd () s (resume s s)))
          (let ((x #list(1 (P.acc 5)))) (P.rd))))
      (export main)))
  (output (: 5 Int64)))

(case
  "list construction strictly evaluates an EFFECTFUL element argument — the perform runs even when = short-circuits"
  (doc
    "The equality companion: the same performing list `(list 1 ((. P acc) 5))` is the LEFT operand of an
           `=` against `(list 9 9)`, which mismatches at element 0. The comparison short-circuits, and its bool
           is discarded; `main` returns `((. P rd))` = the handler state. Result 5 proves the perform RAN when
           the left operand was CONSTRUCTED — before/independent of the comparison deciding — so a constructed
           `=` operand's effectful element argument is strict, not deferred by the short-circuit. GREEN
           regression guard (must stay 5): pairs with the pure-trapping `=` operand case above, whose trap the
           strict-construction fix must add without disturbing this already-strict effect path.")
  (input
    (do
      (effect P (op acc (-> Int64 Int64)) (op rd (-> Int64)))
      (def
        (main)
        (handle
          P
          (: 0 Int64)
          ((acc (v) s (resume v (+ s v))) (rd () s (resume s s)))
          (let ((b (= #list(1 (P.acc 5)) #list(9 9)))) (P.rd))))
      (export main)))
  (output (: 5 Int64)))

(case
  "recursive-sum equality over FLOAT payloads compares by canonical float bytes along the walk"
  (doc
    "The float-leaf member of the recursive-walk family (the Int64-payload cases above compare
           integer leaves): `(type FL (FNil) (FCons Float64 FL))` — each spine node carries a Float64, so
           the structural walk's per-node compare descends into the CANONICAL float byte form (the
           equality float leaves already have at top level, now nested under recursion). `(= (mk 3 x) (mk
           3 2.5))` from one compiled walk: true at x=2.5 (three float leaves all equal), false at x=0.5.
           A walk that compared float leaves by raw bits without canonicalization, or that skipped float
           payloads in the spine compare, diverges at one of the calls.")
  (input
    (do
      (type FL (FNil) (FCons Float64 FL))
      (def (mk (: n Int64) (: f Float64)) (if (= n 0) (FNil) (FCons f (mk (- n 1) f))))
      (def (main (: x Float64)) (= (mk 3 x) (mk 3 2.5)))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: true Bool))
  (call main (: 0.5 Float64))
  (output (: false Bool)))

(case
  "two runtime sums with the same payload but different variants compare unequal by a heap walk"
  (doc
    "The discriminant half of the runtime heap walk: `pick` builds `(N.I n)` or `(N.J n)` from a
           runtime boolean, so `(pick true 5)` = `(N.I 5)` and `(pick false 5)` = `(N.J 5)` are two
           genuine heap sums carrying the SAME payload 5 under DIFFERENT variants. The heap walk compares
           the discriminant BEFORE the payload (core-semantics.md #Equality Is Structural), so they are
           unequal → 0 even though their payloads match. Pins that runtime `value-eq` — like the constant
           fold — distinguishes `I 5` from `J 5`; an implementation comparing only payloads would wrongly
           report equal.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (pick b n) (if b (N.I n) (N.J n)))
      (def (main) (if (= (pick true 5) (pick false 5)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "two runtime tuples compare equal by a heap walk"
  (doc
    "The TUPLE companion of the runtime sum heap walk: `mk` builds `(tuple n (+ n 1))` from its
           parameter, so `(mk 3)` = `(tuple 3 4)` is a runtime heap tuple, not a folded constant.
           `(= (mk 3) (mk 3))` walks both tuples element-wise and finds them equal → 1. Pins that
           `value-eq` handles a runtime tuple (a positional product) the same as a sum — the structural
           equality is over ANY compound, not sum-specific.")
  (input (do (def (mk n) #tuple(n (+ n 1))) (def (main) (if (= (mk 3) (mk 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case
  "two runtime records compare equal by a heap walk"
  (doc
    "The RECORD companion: `mk` builds `(record (x n) (y (+ n 1)))` from its parameter, a runtime
           heap record. `(= (mk 3) (mk 3))` walks both by field and finds them equal → 1. Records
           canonicalize their field order before the walk (deterministic-value-form.md #A Value Has One
           Canonical Byte Form), so the comparison is over the canonical form, not the written order.
           Together with the tuple and sum cases this pins runtime `value-eq` across every scalar-leaf
           compound shape.")
  (input
    (do
      (def (mk n) #record((= x n) (= y (+ n 1))))
      (def (main) (if (= (mk 3) (mk 3)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a CONSTANT recursive sum compares equal to a differently-built RUNTIME one"
  (doc
    "A mixed-provenance equality: the LEFT operand `(S (S Z))` is a COMPILE-TIME-CONSTANT recursive
           `Nat`, the RIGHT `(mk k)` is a RUNTIME-built spine of the same shape (`mk` recurses `k` times).
           `value-eq` must reconcile a folded constant sum with a heap-walked runtime one — the const side
           has a statically-known spine, the runtime side is discovered variant-by-variant. At `k = 2` both
           are `S(S(Z))` → equal → 1; at `k = 3` the runtime spine is one deeper → unequal → 0 (the
           companion case). Pins that structural equality composes a CONSTANT operand with a RUNTIME operand
           over a recursive sum, not only two runtime operands.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk (: n Int64)) (if (> n 0) (Nat.S (mk (- n 1))) (Nat.Z)))
      (def (main (: k Int64)) (if (= (Nat.S (Nat.S (Nat.Z))) (mk k)) 1 0))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64)))

(case
  "two runtime Ok values of a multi-parameter sum compare equal by a heap walk"
  (doc
    "The MULTI-PARAMETER-sum companion: `Result` has TWO type parameters (`Ok a`, `Err b`), and
           `(Ok (sumto 3))` fixes only `a = Int64` — the `Err` parameter `b` is a PHANTOM no value here
           instantiates. `(= (Ok (sumto 3)) (Ok 6))` builds both operands from recursion (unfoldable), so
           the runtime `value-eq` heap walk runs; the two `Ok` values carry equal Int64 payloads → 1. Pins
           that an UNCONSTRAINED type parameter of a SIBLING variant (`Err b`) does not block the walk: a
           phantom parameter carries no runtime structure, so it is scalar-safe. A generation that walked
           every variant's payload type and rejected the free `b` declined this though the compared `Ok`
           values are exactly walkable — the walkability check must admit a bare unconstrained variable.")
  (input
    (do
      (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
      (def (main) (if (= (Ok (sumto 3)) (Ok 6)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "two differing runtime Ok values of a multi-parameter sum compare unequal by a heap walk"
  (doc
    "The unequal companion: `(sumto 3)` = 6, so `(Ok (sumto 3))` carries 6 while `(Ok 7)` carries 7
           — the heap walk finds the payloads differ and the comparison is false → 0. Confirms the
           phantom-`Err`-parameter `Result` comparison is a genuine content test, not a fold that
           happened to say true.")
  (input
    (do
      (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
      (def (main) (if (= (Ok (sumto 3)) (Ok 7)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "a runtime sum equality drives a tail-recursive loop"
  (doc
    "The runtime heap walk `=` used as the CONDITION of a tail-recursive function that compiles to a
           wasm LOOP: `find` searches upward from 0 for the `n` whose `(N.I n)` equals `(N.I 3)`, so the
           comparison runs each iteration and the else-branch `(find (+ n 1))` iterates. `find(0)` = 3. This
           pins that a runtime `value-eq` in a loop's condition COMPOSES with the loop's own scratch: the
           i32 heap-handle slots the compare stashes must not collide with the i64 arithmetic slot the
           `(+ n 1)` iteration uses — the sibling branches must allocate their scratch ABOVE the condition's
           high-water. A generation that reused the condition's handle slot for the branch's arithmetic
           emitted an invalid module (`expected i64, found i32`); this exercises the branch-scratch
           discipline that keeps a heap-handle condition and a scalar branch in one function well-typed.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (find n) (if (= (mk n) (mk 3)) n (find (+ n 1))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64)))

(case
  "a runtime sum match drives a tail-recursive loop"
  (doc
    "The `match` companion of the value-eq-in-a-loop case: a runtime sum MATCH (built by `bump`, so
           it does not fold to a scalar compare) is the CONDITION of the tail-recursive `find`, and the
           else-branch `(find (+ n 1))` iterates the loop. `find(0)` = 3. Pins the same branch-scratch
           discipline for a `MatchSum` condition — its i32 scrutinee-handle slot must not collide with the
           i64 iteration arithmetic — which a folding match (`(match (N.I n) …)` reducing to `n == 3`) would
           never exercise. `bump` keeps the scrutinee a genuine heap value, so the match is a real runtime
           dispatch in the loop condition.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (bump n) (if (< n 0) (N.J n) (N.I n)))
      (def (find n) (if (match (bump n) ((N.I x) (= x 3)) ((N.J _) false)) n (find (+ n 1))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64)))

(case
  "a guarded wildcard arm falls through to a tail-recursive call"
  (doc
    "A `match` whose FIRST arm is a GUARDED WILDCARD (`(guard x <cond>)`) and whose fall-through arm
           TAIL-CALLS the enclosing function, compiled as a wasm LOOP: `find` returns `n` once `(> n 2)`
           holds, else `(find (+ n 1))` iterates. `find(0)` = 3. A guarded wildcard emits `if <guard>
           <body> else <fall-through>` with NO separate probe test (a wildcard needs none), so the guard's
           `if` is the ONLY block its body and fall-through nest inside. A generation that counted a
           (non-existent) probe `if` too made the fall-through's self-tail-call `br` one level too far —
           PAST the loop — producing an invalid module (`expected i64 but nothing on stack`). Pins that a
           guarded-wildcard arm's block nesting is exactly its guard `if`, so a tail call in its
           fall-through iterates the loop rather than escaping it.")
  (input
    (do
      (def (find n) (match n ((guard x (> x 2)) x) (_ (find (+ n 1)))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64)))

(case
  "a value-eq guard on a wildcard arm drives a tail-recursive loop"
  (doc
    "The heap-handle companion of the guarded-wildcard loop case: the guard is a runtime `value-eq`
           (`(= (mk x) (mk 3))`, `mk` building genuine sum values), so BOTH fixes compose — the guard's i32
           handle scratch must sit above the fall-through's i64 iteration arithmetic (the branch-scratch
           discipline), AND the guarded-wildcard block nesting must be exactly one `if` (the tail-depth
           discipline). `find(0)` = 3. This is the exact shape a proof/AST search takes: scan upward,
           returning when a structural equality on a constructed term holds, else recurse.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (find n) (match n ((guard x (= (mk x) (mk 3))) x) (_ (find (+ n 1)))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64)))

(case
  "a runtime sum equality as a match SCRUTINEE drives a tail-recursive loop"
  (doc
    "The runtime heap walk `=` used as the SCRUTINEE of a `match` (a Bool the arms dispatch on),
           inside a tail-recursive loop: `find` matches `(= (mk n) (mk 3))` — `true` returns `n`, `false`
           iterates `(find (+ n 1))`. `find(0)` = 3. The scrutinee is a COMPUTED value (a value-eq, not a
           bare local), so it is evaluated ONCE into a slot; its i32 heap-handle scratch must not be reused
           by the arm bodies' i64 iteration arithmetic — the probe chain starts ABOVE the scrutinee emit's
           high-water, not a bare `base+1`. A generation that fixed the probe floor at `base+1` reused a
           value-eq handle slot for the branch arithmetic (`expected i64, found i32`).")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (find n) (match (= (mk n) (mk 3)) (true n) (false (find (+ n 1)))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64)))

(case
  "a value-eq guard on a LITERAL-probe arm drives a tail-recursive loop"
  (doc
    "The literal-probe companion of the guarded-wildcard loop case: the first arm is `(guard 3 <cond>)`
           — a LITERAL probe (`n == 3`) AND a runtime `value-eq` guard — with a fall-through that iterates.
           `find(0)` climbs until `n == 3`, where the guard `(= (mk n) (mk 3))` also holds, returning 300.
           A literal-probe-plus-guard nests `if (n==3) { if <guard> body else <fall> } else <fall>` — the
           guard's i32 handle scratch (in the THEN) types a slot the OUTER probe-else's i64 iteration
           arithmetic must not reuse (the two `if` branches share one function-global local declaration).
           Pins that the probe-else starts scratch above the THEN's high-water — the same discipline the
           `if`-condition and guarded-wildcard cases exercise, here at the literal-probe/guard seam.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def (find n) (match n ((guard 3 (= (mk n) (mk 3))) 300) (_ (find (+ n 1)))))
      (def (main) (find 0))
      (export main)))
  (output (: 300 Int64)))

(case
  "a value-eq guard on a SUM-match arm drives a tail-recursive loop"
  (doc
    "The sum-match-decision-tree companion: the scrutinee is a genuine heap SUM (`(bump n)`, a call so
           it does not fold), matched by a variant pattern `(N.I x)` with a runtime `value-eq` GUARD, and a
           fall-through arm that iterates. `find(0)` climbs until `x == 3`. The decision tree emits `if
           (sum-disc == I) { if <guard> body else <fall> } else <fall>`; the guard's i32 handle scratch (in
           the disc-matched THEN) types a slot the disc-switch's ELSE fall-through i64 iteration arithmetic
           must not reuse. Pins the branch-scratch discipline at the SUM-match seam (`emit_sum_cont`'s
           guarded-arm + disc-switch), distinct from the scalar-match probe chain: the fall-through of BOTH
           the guard `if` and the disc-switch `if` must clear the arm's heap-handle high-water.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (bump n) (if (< n 0) (N.J n) (N.I n)))
      (def (mk n) (N.I n))
      (def (find n) (match (bump n) ((guard (N.I x) (= (mk x) (mk 3))) x) (_ (find (+ n 1)))))
      (def (main) (find 0))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "two constant sums with the same payload but different variants are not equal"
  (doc
    "Constant compound equality folds STRUCTURALLY (core-semantics.md #Equality Is Structural), and
           structural equality compares the VARIANT before the payload: `(= (Ok 1) (Err 1))` is FALSE even
           though both carry the payload 1, because `Ok` and `Err` are different variants. Pins the
           discriminant half of the fold — an implementation that compared only payloads (a heap walk that
           skipped the variant tag) would wrongly report true here, conflating `Ok 1` and `Err 1`. The
           companion of `(= (Ok 1) (Ok 1))` = true: same variant AND same payload.")
  (input (= (Ok 1) (Err 1)))
  (output (: false Bool)))

(case
  "nested-Option equality observes the OUTER variant and matches identical nesting"
  (doc
    "The DEPTH companion of the `(Ok 1)` vs `(Err 1)` discriminant case: the variant tag must be
           observed at the OUTER level of a NESTED sum too. At type `Option (Option Int64)`, `(Some (None))`
           (outer `Some`, inner `None`) and `(None)` (outer `None`) are DIFFERENT values — the outer
           discriminant differs — so `=` is FALSE; while `(Some (None))` equals itself (identical nesting) →
           TRUE. Encoded `10·(SomeNone = None ? 1 : 0) + (SomeNone = SomeNone ? 1 : 0)` = `10·0 + 1` = 1. A
           heap walk that compared payloads without the outer variant tag — conflating the inner `None`
           payload of `(Some None)` with the outer `None` — would flip the first compare to true → 11. Pins
           that `=` observes the discriminant at the OUTER level of a nested Option, the runtime/value
           companion of the match-exhaustiveness cases that distinguish `(Some (None _))` from `(None _)`.")
  (input
    (do
      (def
        (main (: k Int64))
        (+
          (*
            10
            (if
              (= (: (Some (None)) (Option (Option Int64))) (: (None) (Option (Option Int64))))
              1
              0))
          (if
            (= (: (Some (None)) (Option (Option Int64))) (: (Some (None)) (Option (Option Int64))))
            1
            0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "two constant records with the same fields in different written order are equal"
  (doc
    "Constant record equality folds structurally and compares fields as a SET keyed by name, not by
           written order: `(= (record (x 1) (y 2)) (record (y 2) (x 1)))` is true — both denote the same
           value (a record's canonical form sorts its fields by key, deterministic-value-form.md #A Value
           Has One Canonical Byte Form). Pins that the equality fold normalizes field order before
           comparing, so the same record written two ways is one value — not a position-wise comparison
           that would call these unequal.")
  (input (= #record((= x 1) (= y 2)) #record((= y 2) (= x 1))))
  (output (: true Bool)))

(case
  "a runtime compound structural equality is expressible as a hand-written recursive comparator"
  (doc
    "The route around the not-yet-emitted heap walk, and the shape a program needing runtime
           compound equality writes today: an explicit recursive comparator that dispatches on each
           value's variant and compares the leaves with scalar `=` (which IS emitted for runtime
           scalars). `same` compares two `N` values by matching both and comparing the bound Int64
           payloads; `(same (mk 1) (mk 1))` is true → 1. Pins that structural equality of runtime
           compounds is ALREADY achievable by hand — the missing built-in `=` heap walk is a
           convenience over this, not a new expressive power — so a program (a proof kernel comparing
           terms, a compiler comparing AST nodes) is not blocked, only more verbose.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk n) (N.I n))
      (def
        (same a b)
        (match
          a
          ((N.I x) (match b ((N.I y) (= x y)) ((N.J _) false)))
          ((N.J x) (match b ((N.J y) (= x y)) ((N.I _) false)))))
      (def (main) (if (same (mk 1) (mk 1)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "an offered ordering is total and deterministic"
  (doc "Witnesses core-semantics.md #Ordering Where Offered Is Total: Int64 offers a total order.")
  (input (< 2 3))
  (output (: true Bool)))

; The Int64 total order above uses mid-range 2,3. These pin its EXTREMES (Int64.min < Int64.max holds and
; its reverse is false — the widest possible ordered pair) and the CROSS-TYPE rejection: ordering, like
; equality, is defined only within one ordered type, so `< Int64 Bool` is a type error, not a coercion —
; the ordering companion of the cross-width/function-value equality rejections.
(case
  "comparing an Int64 to a Bool with < is a type error"
  (doc
    "`(< 5 true)` orders an Int64 against a Bool — two DIFFERENT types. Ordering is defined only within
           one ordered type (Cadenza has no cross-type coercion), so it is CDZ0203, exactly as a cross-type
           `=` is. Pins that the `<` operator's operands must share a type — the ordering analogue of the
           cross-width-float and function-value equality type errors, not a silent Int-vs-Bool comparison.")
  (input (do (def (main) (if (< 5 true) 1 0)) (export main)))
  (error CDZ0203))

(case
  "the Int64 total order holds at its extremes"
  (doc
    "`(< Int64.min Int64.max)` — the widest ordered pair — is true, and its reverse `(< Int64.max
           Int64.min)` is false. Pins the total order at the type's boundary values (the mid-range `(< 2 3)`
           cannot witness the extremes): a comparison that mis-signed or wrapped at Int64.min/max would flip
           one of these. -2^63 < +2^63-1 is the maximal true ordering; the reverse is the maximal false.")
  (input (< -9223372036854775808 9223372036854775807))
  (output (: true Bool)))

(case
  "the reversed extreme ordering is false"
  (doc
    "The complement fixing the direction at the extremes: `(< Int64.max Int64.min)` = `(< 2^63-1 -2^63)`
           is false — the maximum is not below the minimum. Together with the case above this pins the total
           order's direction across the full Int64 range, ruling out a sign-confusion at the boundary that a
           mid-range pair would not expose.")
  (input (< 9223372036854775807 -9223372036854775808))
  (output (: false Bool)))

(case
  "an entrypoint returning a comparison presents a Bool result at the boundary"
  (doc
    "Type-directed emission at the component boundary: a nullary `main` whose body is an Int64
           comparison has result type Bool, so the `run` export is framed at the Bool boundary valtype,
           not the s64 an arithmetic result would use. `(lt 20 22)` is true. The companion below returns
           the arithmetic i64 (42) through the SAME entrypoint shape, so the pair pins that the boundary
           result type tracks the program's RESULT TYPE — a comparison crosses as Bool, an arithmetic
           expression as Int64 — rather than a fixed valtype. This is the observable of a compiler that
           reads a program's result kind and frames `run` accordingly; the result kind is one of a fixed
           set (Int64 / Bool), selected by the operator that produces the result (a comparison yields
           Bool, `+`/`-`/`*`/`/`/`%` yield Int64).")
  (input (do (def (lt a b) (< a b)) (def (main) (lt 20 22)) (export main)))
  (output (: true Bool)))

(case
  "an entrypoint returning arithmetic presents an Int64 result at the boundary"
  (doc
    "The Int64 companion to the Bool-boundary case above: the same nullary-`main`-calls-a-helper
           shape, but the body is an arithmetic expression whose result type is Int64, so `run` is framed
           at the Int64 boundary valtype and `(add 20 22)` crosses as 42. Together the two cases pin that
           the entrypoint's boundary result type is type-directed — Bool for a comparison, Int64 for
           arithmetic — the same program shape emitting a different boundary type from its result type
           alone.")
  (input (do (def (add a b) (+ a b)) (def (main) (add 20 22)) (export main)))
  (output (: 42 Int64)))

; --- Bool offers a total order in which false is less than true --------------------------
; core-semantics.md #Ordering Where Offered Is Total, 3rd sentence: "The Bool type MUST offer a
; total order in which false is less than true." Bool is not only an equality type — it carries a
; definite order with false below true, the conventional boolean ranking. These pin the direction of
; the order (false < true, not the reverse) and that all four ordering operators observe it, so a
; comparison of two Bools yields the ordered result rather than declining or treating Bool as
; unordered. The seed declines Bool ordering ("non-integer operand to integer op") — it emits the
; ordering operators only for Int64 — so a generation that does not yet emit the Bool comparison
; declines rather than running (reject-don't-miscompile); the gate scores the decline as todo, not
; disagreement, and the requirement marks the emission a later generation adds.
(case
  "false is less than true"
  (doc
    "Witnesses core-semantics.md #Ordering Where Offered Is Total (Bool clause): `(< false
           true)` is true because false is the lesser of the two Bool values — the direction of the
           Bool order.")
  (input (< false true))
  (output (: true Bool)))

(case
  "true is not less than false"
  (doc
    "The complement fixing the order's direction: `(< true false)` is false, because true is
           not below false. Together with the case above this pins false < true rather than the
           reverse ranking, so the order is antisymmetric in the specified direction.")
  (input (< true false))
  (output (: false Bool)))

(case
  "true is greater than false"
  (doc
    "The `>` companion: `(> true false)` is true, the mirror of `(< false true)`. Pins that the
           strict greater-than operator observes the same Bool order.")
  (input (> true false))
  (output (: true Bool)))

(case
  "a boolean is less-than-or-equal to itself"
  (doc
    "`(<= false false)` is true: the inclusive ordering operator is reflexive on Bool, as a
           total order requires. Pins `<=` on equal Bool operands.")
  (input (<= false false))
  (output (: true Bool)))

(case
  "a boolean is greater-than-or-equal to itself"
  (doc
    "`(>= true true)` is true: `>=` is reflexive on Bool. Completes the four ordering operators
           over the Bool order.")
  (input (>= true true))
  (output (: true Bool)))

; --- A total order is observed through a three-way `compare` yielding Ordering ------------------
; core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison: a type that offers a
; total order offers a `compare` yielding an `Ordering` value with exactly three variants — `Less`,
; `Equal`, `Greater` — so a single comparison reports the full relation, not one boolean bit of it.
; `Ordering` is an ORDINARY closed prelude sum (like Option, Result, Sign), so its value form is
; `(Ordering.Less unit)` etc. — a nullary variant applied to unit, the same `(Variant unit)` form
; every nullary variant takes — and a consumer deconstructs it with an exhaustive three-arm match.
; `compare` is the PRIMITIVE from which `<` `>` `<=` `>=` `=` are each definable (the operators MUST
; AGREE with it), so a type has one order surfaced two ways that cannot disagree. It also names the
; canonical element order Set/Map serialize in. Ordering is a FRESH capability the
; seed does not realize (distinct from `collections`). A later generation realizes `compare`; until
; then the seed DECLINES these rather than running them to a wrong value.
(case
  "comparing a lesser value to a greater yields Less"
  (doc
    "`(Ordering.of 1 2)` is `(Ordering.Less unit)` — the three-way comparison reports that 1 is less
           than 2 as the `Less` variant of the Ordering sum, not a boolean (core-semantics.md #A Total
           Order Is Observed Through A Three-Way Comparison). Pins the Less arm of the three-way result.")
  (input (Ordering.of 1 2))
  (output (: (Less unit) Ordering)))

(case
  "comparing equal values yields Equal"
  (doc
    "`(Ordering.of 2 2)` is `(Ordering.Equal unit)` — the middle variant, distinct from both Less and
           Greater. Pins that the three-way comparison reports equality as its own variant rather than
           collapsing it into one of the strict relations.")
  (input (Ordering.of 2 2))
  (output (: (Equal unit) Ordering)))

(case
  "comparing a greater value to a lesser yields Greater"
  (doc
    "`(Ordering.of 3 2)` is `(Ordering.Greater unit)` — the Greater variant. Together with the Less and
           Equal cases this pins all three variants of the Ordering result are reachable and correctly
           discriminated by the value relation.")
  (input (Ordering.of 3 2))
  (output (: (Greater unit) Ordering)))

(case
  "the three-way comparison is deconstructed by an exhaustive match"
  (doc
    "An Ordering value is an ordinary closed sum, so it is matched with the uniform `(Ctor _)`
           patterns over its three variants (core-semantics.md #A Total Order Is Observed Through A
           Three-Way Comparison, 2nd sentence): matching `(Ordering.of 1 2)` selects the `Less` arm, yielding
           -1. Pins that a comparison result dispatches through the same exhaustive match as any other
           sum, so every consumer handles all three cases.")
  (input
    (match (Ordering.of 1 2) ((Ordering.Less _) -1) ((Ordering.Equal _) 0) ((Ordering.Greater _) 1)))
  (output (: -1 Int64)))

(case
  "the boolean less-than operator agrees with the three-way comparison"
  (doc
    "core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison (3rd sentence: the
           boolean ordering operators MUST agree with the three-way comparison): `(< 1 2)` is true
           exactly when `(Ordering.of 1 2)` is `(Ordering.Less unit)`. This case pins that agreement — `(< 1
           2)` is true and the compare above is Less, so a type's one order is surfaced two ways that
           cannot diverge.")
  (input (< 1 2))
  (output (: true Bool)))

; --- Runtime SCALAR three-way `compare`: `(Ordering.of a b)` over runtime Int64/Bool COMPUTES -------------
; core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison (3rd sentence: the boolean
; ordering operators MUST agree with the three-way comparison). The constant cases above fold at compile
; time; these pin that a RUNTIME scalar pair (function parameters — no compile-time value) is compared the
; same way, yielding the same three Ordering variants. No new runtime op: the three-way is the nested-if
; `if (a < b) Less else if (a > b) Greater else Equal` over the SAME machine `<`/`>` the boolean operators
; emit — so the two surfaces cannot diverge (the §331 agreement) at runtime as well as at fold time. Each
; operand is read twice (the `<` and the `>`) but MATERIALIZED ONCE, so a trapping/effectful operand runs
; exactly once.
(case
  "the three-way comparison over a runtime lesser scalar yields Less"
  (doc
    "`(Ordering.of a b)` over runtime Int64 params `a=3, b=5` selects the `Less` arm → 1. Pins that a
           runtime scalar three-way compare computes (not just a constant fold), agreeing with `(< 3 5)`.")
  (input
    (do
      (def
        (cmp (: a Int64) (: b Int64))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp 3 5))
      (export main)))
  (output (: 1 Int64)))

(case
  "the three-way comparison over equal runtime scalars yields Equal"
  (doc
    "`(Ordering.of a b)` over runtime Int64 params `a=b=5` selects the `Equal` arm → 2. Pins the middle
           variant of a runtime scalar three-way compare.")
  (input
    (do
      (def
        (cmp (: a Int64) (: b Int64))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp 5 5))
      (export main)))
  (output (: 2 Int64)))

(case
  "the three-way comparison over a runtime greater scalar yields Greater"
  (doc
    "`(Ordering.of a b)` over runtime Int64 params `a=9, b=5` selects the `Greater` arm → 3. With the two
           cases above, all three variants of a runtime scalar three-way compare are reachable.")
  (input
    (do
      (def
        (cmp (: a Int64) (: b Int64))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp 9 5))
      (export main)))
  (output (: 3 Int64)))

(case
  "the three-way comparison over runtime booleans orders false before true"
  (doc
    "`(Ordering.of a b)` over runtime Bool params `a=false, b=true` yields `Less` → 1: Bool offers the
           total order `false < true` (core-semantics.md #Ordering Where Offered Is Total), and the runtime
           three-way compare surfaces it exactly as the boolean `<` does. `a`/`b` are computed at runtime
           (`(< 9 2)`=false, `(< 2 9)`=true) so neither folds.")
  (input
    (do
      (def
        (cmp (: a Bool) (: b Bool))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp (< 9 2) (< 2 9)))
      (export main)))
  (output (: 1 Int64)))

(case
  "the three-way comparison over a runtime scalar performs the operand exactly once"
  (doc
    "`(Ordering.of (+ a 1) 5)` over runtime Int64 `a=4` computes `(+ 4 1)=5` then compares to 5 →
           `Equal` → 2. The operand `(+ a 1)` is read by both the internal `<` and `>` but is materialized
           ONCE (a single evaluation), so an effectful/trapping operand would run exactly once — this pins
           the value side of that materialize-once lowering.")
  (input
    (do
      (def
        (cmp (: a Int64))
        (match
          (Ordering.of (+ a 1) 5)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp 4))
      (export main)))
  (output (: 2 Int64)))

(case
  "the three-way comparison orders strings lexicographically"
  (doc
    "`(Ordering.of \"a\" \"b\")` is `(Ordering.Less unit)` — String offers a total order (the
           lexicographic order of its Unicode scalar values, collections-and-text.md #String Comparison
           Is Defined On Scalar Values), so compare works over it exactly as over Int64. Pins that the
           three-way comparison is offered by every type with a total order, not only the numeric types.")
  (input (Ordering.of "a" "b"))
  (output (: (Less unit) Ordering)))

(case
  "the three-way comparison over a genuinely-runtime String computes content-lexicographically"
  (doc
    "The String case above compares CONSTANT strings (folded before emit). Forcing genuinely-runtime
           strings — `(String.concat s \"z\")` off a parameter, so neither operand is compile-time known —
           makes `compare` WALK the content: `(Ordering.of (mk \"a\") (mk \"b\"))` is `Less` → 1 ('az' before
           'bz', content-lexicographic over Unicode scalars, collections-and-text.md #String Comparison Is
           Defined On Scalar Values). No new runtime op — the three-way desugars to the nested-if over the
           SAME `Core::StrCmp` byte-lex walk the boolean String `<`/`>` emit, so the two surfaces agree at
           runtime (§331) on both backends.")
  (input
    (do
      (def (mk (: s String)) (String.concat s "z"))
      (def
        (cmp (: x String) (: y String))
        (match
          (Ordering.of x y)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp (mk "a") (mk "b")))
      (export main)))
  (output (: 1 Int64)))

(case
  "the three-way comparison over an equal runtime String yields Equal"
  (doc
    "`(Ordering.of (mk \"m\") (mk \"m\"))` over runtime strings (built via concat off a literal, so the
           two share content but are distinct allocations) yields `Equal` → 2 — the content-lexicographic
           walk reports equality by CONTENT, not allocation identity (memory-and-resource-model.md #Sharing
           Is Not Observable). The middle-variant companion of the runtime-String Less case.")
  (input
    (do
      (def (mk (: s String)) (String.concat s "!"))
      (def
        (cmp (: x String) (: y String))
        (match
          (Ordering.of x y)
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (def (main) (cmp (mk "m") (mk "m")))
      (export main)))
  (output (: 2 Int64)))

(case
  "the three-way comparison orders Float64 by numeric value — Less"
  (doc
    "`(Ordering.of 1.5 2.5)` is `(Ordering.Less unit)`: Float64 offers the same total order the numeric
           model defines for it, and `compare` reports it as the Less variant exactly as over Int64
           (core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison). Pins that the
           three-way comparison spans the OTHER realized numeric type, not just Int64 — the Float64
           companion of `(Ordering.of 1 2)`. (A NaN operand is not ordered and declines here — the finite
           float order is what is pinned.)")
  (input (Ordering.of 1.5 2.5))
  (output (: (Less unit) Ordering)))

(case
  "the three-way comparison orders Float64 by numeric value — Equal"
  (doc
    "`(Ordering.of 2.5 2.5)` is `(Ordering.Equal unit)` — two equal finite Float64 values report the
           middle variant, the Float64 companion of `(Ordering.of 2 2)`. Pins that Float64 equality-under-order
           agrees with the value relation (distinct from both strict arms).")
  (input (Ordering.of 2.5 2.5))
  (output (: (Equal unit) Ordering)))

(case
  "the three-way comparison orders Float64 by numeric value — Greater"
  (doc
    "`(Ordering.of 2.5 1.5)` is `(Ordering.Greater unit)` — with the Less and Equal Float64 cases this
           pins all three Ordering variants are reachable over Float64 and correctly discriminated by the
           numeric relation, exactly as the Int64 triple does.")
  (input (Ordering.of 2.5 1.5))
  (output (: (Greater unit) Ordering)))

(case
  "a shorter string that is a prefix of a longer one compares Less"
  (doc
    "`(Ordering.of \"ab\" \"abc\")` is `(Ordering.Less unit)`: with equal leading scalars, the shorter
           string orders before the longer (collections-and-text.md #String Comparison Is Defined On
           Scalar Values — lexicographic order treats end-of-string as least). Pins the length-tiebreak
           edge of the lexicographic order that `(Ordering.of \"a\" \"b\")` (a first-scalar difference) does
           not exercise.")
  (input (Ordering.of "ab" "abc"))
  (output (: (Less unit) Ordering)))

(case
  "the three-way comparison orders Bool with false below true"
  (doc
    "`(Ordering.of false true)` is `(Ordering.Less unit)` — Bool carries the total order false < true
           (the order the boolean-ordering cases above test through `<`/`>`), and `compare` reports it as
           the Less variant. Pins that the three-way comparison is offered over Bool (a finite non-numeric
           type), the compare-primitive companion of the `(< false true)` operator cases.")
  (input (Ordering.of false true))
  (output (: (Less unit) Ordering)))

(case
  "the boolean less-than operator agrees with compare over Bool"
  (doc
    "core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison (the operators MUST
           agree with the three-way comparison): `(< false true)` is true exactly when
           `(Ordering.of false true)` is `(Ordering.Less unit)`. This pins that agreement for Bool — the same
           one-order-surfaced-two-ways law the Int64 case pins, over the boolean order — so `<` on Bool and
           `compare` on Bool cannot diverge.")
  (input (< false true))
  (output (: true Bool)))

(case
  "a program that makes a host call has that call in its observable behavior"
  (doc
    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module declares a unit-returning effect `log` and the entrypoint delegates it to the host,
           so its operation `log.emit` is bound (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates); the run makes one host call and returns the unit value — the
           normal-termination value of a program evaluated only for its effect (core-semantics.md #An
           Expression Evaluated Only For Its Effect Yields The Unit Value). The (output …) primary clause
           pins the terminal condition; the (host-calls …) observation pins the call sequence.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (host (log) (log.emit "hello")))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "hello" String))))

(case
  "host calls are observed in the order they were made"
  (doc
    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior and
           #A Sequencing Block Evaluates Its Forms In Order (3rd sentence: an earlier form's host call is
           observed before a later form's): the two host calls are sequenced by a (do …) block, so
           \"first\" is observed before \"second\". The run terminates normally with the unit value
           (core-semantics.md #An Expression Evaluated Only For Its Effect Yields The Unit Value); the
           (output …) clause pins that terminal condition and the (host-calls …) observation pins the order.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (host (log) (do (log.emit "first") (log.emit "second"))))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "first" String)) (call log.emit (: "second" String))))

; --- Left-to-right evaluation order WITHIN an expression --------------------------------------------
; The case above pins that a `do` block sequences its FORMS in order. This pins the finer guarantee that
; the sub-expressions WITHIN a single expression — a call's arguments, a binary operator's operands, a
; `let`'s initializers, a tuple's elements — are evaluated LEFT TO RIGHT (core-semantics.md #Host Calls
; Are Ordered And Part Of Observable Behavior; #A Sequencing Block Evaluates Its Forms In Order). Each
; sub-expression is a host call `(log.emit …)`, and the `host-responses` are consumed in CALL order, so
; the order is observable TWO ways at once: the `(host-calls …)` observation asserts the emit sequence
; directly, AND the operator is NON-COMMUTATIVE (`-`) with distinct responses, so evaluating in the wrong
; order would consume the responses swapped and produce a DIFFERENT value (`3 - 10 = -7`, not `10 - 3 = 7`)
; — the recorded `(output …)` alone would catch a right-to-left evaluator even without the observation.
; (A sub-expression whose result is UNUSED is dead-code-eliminated and makes no host call, so each case
; CONSUMES every sub-expression it evaluates — the tuple binds then reads both elements.)
(case
  "function arguments are evaluated left to right"
  (doc
    "`(diff (log.emit \"first\") (log.emit \"second\"))` calls two host effects as the arguments to
           `diff = a - b`. The arguments evaluate left to right, so `first` is emitted before `second` and
           `a` gets the first response (10), `b` the second (3) → 10 - 3 = 7. A right-to-left evaluator
           would emit `second` first, bind `a`=3 and `b`=10, and compute -7 — caught by BOTH the value and
           the host-call order. Pins argument evaluation order, observable through the ordered host calls.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def (diff a b) (- a b))
      (def (main) (host (log) (diff (log.emit "first") (log.emit "second"))))
      (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 3 Int64)))
  (output (: 7 Int64))
  (host-calls (call log.emit (: "first" String)) (call log.emit (: "second" String))))

(case
  "binary operator operands are evaluated left to right"
  (doc
    "`(- (log.emit \"left\") (log.emit \"right\"))` — the two operands of `-` are host effects. The
           left operand evaluates first (emitting `left`, consuming response 10), then the right (`right`,
           response 3) → 10 - 3 = 7. The operator-position companion of the argument case: operand order,
           not only call-argument order, is left to right. A swapped order gives -7.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def (main) (host (log) (- (log.emit "left") (log.emit "right"))))
      (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 3 Int64)))
  (output (: 7 Int64))
  (host-calls (call log.emit (: "left" String)) (call log.emit (: "right" String))))

(case
  "let bindings' initializers are evaluated in binding order"
  (doc
    "`(let ((x (log.emit \"x\")) (y (log.emit \"y\"))) (- x y))` — the initializers run in binding
           order, so `x` is emitted and bound (response 10) before `y` (response 4) → 10 - 4 = 6. Pins that
           a multi-binding `let` evaluates its initializers top to bottom (the order a later binding could
           depend on an earlier one relies on), observable through the ordered host calls.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def (main) (host (log) (let ((x (log.emit "x")) (y (log.emit "y"))) (- x y))))
      (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 4 Int64)))
  (output (: 6 Int64))
  (host-calls (call log.emit (: "x" String)) (call log.emit (: "y" String))))

(case
  "tuple elements are evaluated left to right"
  (doc
    "`(tuple (log.emit \"a\") (log.emit \"b\"))` — the elements evaluate left to right, so `a` is
           emitted (response 10) before `b` (response 4). The tuple is bound and BOTH elements read back
           (`(- (. t 0) (. t 1))` = 10 - 4 = 6) so neither emit is dead-code-eliminated — an unused element
           would be dropped, making no host call. Pins that a compound constructor evaluates its components
           left to right, observable through the ordered host calls.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def
        (main)
        (host (log) (let ((t #tuple((log.emit "a") (log.emit "b")))) (- (. t 0) (. t 1)))))
      (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 4 Int64)))
  (output (: 6 Int64))
  (host-calls (call log.emit (: "a" String)) (call log.emit (: "b" String))))

; --- Control flow SELECTS which effects are observed ------------------------------------------------
; The ordering cases above evaluate every sub-expression. Control flow instead makes only SOME
; sub-expressions run: an `if` evaluates one branch, `and`/`or` skip the right operand when the left
; decides (core-semantics.md #Conditionals Evaluate One Branch; #Boolean Connectives Short-Circuit). The
; existing `if`/`and` cases witness this via a TRAP that does not fire (a negative — the run terminates
; normally). These witness it POSITIVELY: each branch/operand is a host effect, so the `(host-calls …)`
; observation records EXACTLY which effects ran — the taken branch's, and only it; the evaluated operand's,
; and none on the skipped path (`(host-calls)` = no call). A runtime `Bool` parameter drives the choice, so
; the selection happens at run time, not by a constant fold. This is the observable-effect complement of
; the trap-shielding cases (02-binding-and-control): the wrong branch/operand does not merely avoid a trap,
; its effect is genuinely never performed.
(case
  "a conditional performs only the taken branch's effect — then"
  (doc
    "`(if b (log.emit \"then\") (log.emit \"else\"))` with `b`=true performs ONLY the then branch's
           effect — `then` is emitted, `else` is not (core-semantics.md #Conditionals Evaluate One Branch).
           The positive-observation companion of the trap-shielding conditional case: not only does the
           unselected branch avoid a trap, its host effect is never performed. The condition is a runtime
           parameter, so the selection is a run-time event.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def (main (: b Bool)) (host (log) (if b (log.emit "then") (log.emit "else"))))
      (export main)))
  (host-responses (respond log.emit (: 1 Int64)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (host-calls (call log.emit (: "then" String))))

(case
  "a conditional performs only the taken branch's effect — else"
  (doc
    "The false-condition companion: with `b`=false only the ELSE branch's effect is performed —
           `else` is emitted, `then` is not. Together with the `then` case, this pins that a runtime `if`
           performs exactly one branch's effect, the one its condition selects.")
  (input
    (do
      (effect log (op emit (-> String Int64)))
      (def (main (: b Bool)) (host (log) (if b (log.emit "then") (log.emit "else"))))
      (export main)))
  (host-responses (respond log.emit (: 2 Int64)))
  (call main (: false Bool))
  (output (: 2 Int64))
  (host-calls (call log.emit (: "else" String))))

(case
  "and short-circuit does not perform the right operand's effect"
  (doc
    "`(and b (log.emit \"rhs\"))` with `b`=false short-circuits, so the right operand's host effect is
           NOT performed — `(host-calls)` records NO call, and the `and` is false → 0 (core-semantics.md
           #Boolean Connectives Short-Circuit). The positive-observation companion of the trap-based
           short-circuit case (02-binding-and-control): the skipped operand's effect genuinely does not
           occur, not merely a skipped trap.")
  (input
    (do
      (effect log (op emit (-> String Bool)))
      (def (main (: b Bool)) (host (log) (if (and b (log.emit "rhs")) 1 0)))
      (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call main (: false Bool))
  (output (: 0 Int64))
  (host-calls))

(case
  "and performs the right operand's effect when the left is true"
  (doc
    "The non-short-circuit path: with `b`=true the right operand IS evaluated, so its effect `rhs` is
           performed (`(host-calls)` records the one call) and, its response being true, the `and` is true →
           1. Pins that a `true` left operand reaches the right operand's effect — the observable complement
           of the skip case above.")
  (input
    (do
      (effect log (op emit (-> String Bool)))
      (def (main (: b Bool)) (host (log) (if (and b (log.emit "rhs")) 1 0)))
      (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (host-calls (call log.emit (: "rhs" String))))

(case
  "or short-circuit does not perform the right operand's effect"
  (doc
    "`(or b (log.emit \"rhs\"))` with `b`=true short-circuits, so the right operand's host effect is
           NOT performed — `(host-calls)` records no call, and the `or` is true → 1. The `or` mirror of the
           `and` short-circuit-effect case: a `true` left operand skips the right operand's effect.")
  (input
    (do
      (effect log (op emit (-> String Bool)))
      (def (main (: b Bool)) (host (log) (if (or b (log.emit "rhs")) 1 0)))
      (export main)))
  (host-responses (respond log.emit (: false Bool)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (host-calls))

(case
  "or performs the right operand's effect when the left is false"
  (doc
    "The non-short-circuit path: with `b`=false the right operand IS evaluated, so `rhs` is performed
           (`(host-calls)` records the call) and, its response being true, the `or` is true → 1. Pins that a
           `false` left operand reaches the right operand's effect.")
  (input
    (do
      (effect log (op emit (-> String Bool)))
      (def (main (: b Bool)) (host (log) (if (or b (log.emit "rhs")) 1 0)))
      (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call main (: false Bool))
  (output (: 1 Int64))
  (host-calls (call log.emit (: "rhs" String))))

; --- The FloatCompare hoist preserves canonical-byte semantics --------------------------------------
; 551cdf619 extends the common-operator if-arm hoist to Core::FloatCompare — `(if c (= a k) (= b k))`
; over floats emits one canon-and-compare over the selected operand. The hoist must preserve the
; canonical byte form the scalar cases above pin (NaN == NaN; -0.0 distinct from 0.0): a hoist that
; lowered the merged compare to bare f64.eq inverts both. Promoted from passing breaker probes.
(case
  "the selected operand decides a hoisted float equality"
  (doc
    "`(if (> c 0) (= a 1.5) (= b 1.5))` → the hoisted `(= (if c a b) 1.5)`: c = 1 selects
           a = 1.5 → true → 1; c = 0 selects b = 9.0 → false → 0. The float twin of the integer
           comparison-hoist selection pin (a positional mispairing answers the other arm's boolean).")
  (input
    (do
      (def (main (: c Int64) (: a Float64) (: b Float64)) (if (if (> c 0) (= a 1.5) (= b 1.5)) 1 0))
      (export main)))
  (call main (: 1 Int64) (: 1.5 Float64) (: 9.0 Float64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 1.5 Float64) (: 9.0 Float64))
  (output (: 0 Int64)))

(case
  "NaN equality survives the hoisted float compare by canonical byte form"
  (doc
    "A runtime NaN (`(/ 0.0 0.0)`) compared against `Float64.nan` through hoisted if-arms:
           c = 1 → the NaN arm → TRUE (1, every NaN equals every NaN under the canonical byte form);
           c = 0 → `(= nan 2.0)` → 0. The hoist merges the two compares over one selected operand —
           a merge that dropped the canonicalization (bare f64.eq) answers 0 on the first call, the
           exact inversion the canonical form exists to prevent.")
  (input
    (do
      (def
        (main (: c Int64) (: x Float64))
        (let ((n (/ x x))) (if (if (> c 0) (= n Float64.nan) (= n 2.0)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 0.0 Float64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 0.0 Float64))
  (output (: 0 Int64)))

(case
  "negative zero stays distinct from zero through the hoisted float compare"
  (doc
    "`(if (> c 0) (= a 0.0) (= b 0.0))` with a = -0.0, b = 0.0: the hoisted compare receives
           the SELECTED operand and must answer by canonical bytes — c = 1 → -0.0 ≠ 0.0 → 0; c = 0 →
           0.0 = 0.0 → 1. The -0.0 complement of the NaN pin (bare f64.eq answers 1 on the first
           call — the other half of the inversion).")
  (input
    (do
      (def (main (: c Int64) (: a Float64) (: b Float64)) (if (if (> c 0) (= a 0.0) (= b 0.0)) 1 0))
      (export main)))
  (call main (: 1 Int64) (: -0.0 Float64) (: 0.0 Float64))
  (output (: 0 Int64))
  (call main (: 0 Int64) (: -0.0 Float64) (: 0.0 Float64))
  (output (: 1 Int64)))

(case
  "a tuple = whose Bool element derives from a const-divisor rem is emitted as valid wasm"
  (doc
    "MISCOMPILE (invalid wasm, wasm-only): a compound (tuple) `=` whose Bool element is derived from a
           CONST-DIVISOR `%` or `/` — `(= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 2) 0)))` — emitted an
           invalid component (`func failed to validate: type mismatch: expected i32, found i64`). ROOT: the
           two identical `(= (% s 2) 0)` elements are `core_eq`, so the non-loop CSE pass materializes the
           shared `(% s 2)` into an i64 slot ONCE — but it did NOT advance the scratch floor past the
           const-divisor strength-reduction's own transient i64 dividend scratch, so the next allocation (the
           i32 Bool slot of the `= … 0`) reused that i64 slot → one wasm local declared at two widths. Fix:
           the CSE materialization raises `body_base` past `high` after emitting the representative (mirroring
           the LICM-hoist arm) so a later slot never reuses the rep's transient scratch at a different width;
           `emit_div_rem` also reserves its dividend scratch above `*high`. NOT modulo-specific — const-`/`
           reproduces identically. A tuple equals itself → `true`; a `%2==0` vs `%3==0` element differs at
           s=4 → the tuples differ → `false`.")
  (input
    (do (def (main (: s Int64)) (= #tuple(5 (= (% s 2) 0)) #tuple(5 (= (% s 2) 0)))) (export main)))
  (call main (: 4 Int64))
  (output (: true Bool))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case
  "a tuple = with differing const-divisor Bool elements compares unequal"
  (doc
    "The discriminating companion: the two tuple elements derive from DIFFERENT const divisors
           (`% s 2` vs `% s 3`), so at s = 4 the first Bool is `4%2==0` = true and the second `4%3==0` = false
           — the tuples differ, `=` is false. Pins that the fix computes the real element values (not a
           degenerate always-equal), and that the two distinct `%` subexpressions each emit valid wasm.")
  (input
    (do (def (main (: s Int64)) (= #tuple(5 (= (% s 2) 0)) #tuple(5 (= (% s 3) 0)))) (export main)))
  (call main (: 4 Int64))
  (output (: false Bool)))

(case
  "the float ordering-versus-equality split on the zero pair"
  (doc
    "`(= -0.0 0.0)` is FALSE (distinct canonical byte forms) while `(<= -0.0 0.0)` is TRUE
           (IEEE order-equal) — both on ONE pair in one body → 0 + 1 = 1. The landed ordering cases
           pin each side separately; this pins the SPLIT itself side by side, the sharpest
           two-relations-one-pair discriminator (a lowering that reused the equality path for `<=`'s
           equal-case answers 11; one that reused ordering for `=` answers 10).")
  (input
    (do (def (main (: d Int64)) (+ (if (= -0.0 0.0) 10 0) (if (<= -0.0 0.0) 1 0))) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "infinities order beyond every finite value"
  (doc
    "`(/ 1.0 0.0)` = +∞ exceeds 1.0; `(/ -1.0 0.0)` = -∞ is below -1000000.0 → 10 + 1 = 11.
           Float division by zero is total (the never-traps rule) and the resulting infinities take
           their IEEE places in the runtime order — the infinity face of the partial-order landing
           (its pins cover finite values and NaN).")
  (input
    (do
      (def (main (: x Float64)) (+ (if (< 1.0 (/ 1.0 x)) 10 0) (if (< (/ -1.0 x) -1000000.0) 1 0)))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 11 Int64)))

; A `=` between two `Float64.of-int` conversions where the LEFT converts a bare PARAM and the RIGHT converts
; an ARITH result. The wasm `Core::FloatCompare` emit left the LEFT operand's canonicalized f64 bits PENDING
; ON THE STACK, then emitted the RIGHT operand's inner `(+ n 1)` (a CHECKED i64 add) reusing scratch at the
; SAME fixed `base` — re-typing a wasm local the left emit had already fixed (f64/i64), producing an INVALID
; module (`function[0]` fails to compile). ORDER-SPECIFIC: only param-left/arith-right triggered it (arith-
; left's i64 scratch is consumed before any f64 is pending); the mirror + two-params + arith-alone all
; compiled. The fix floats the RHS operand's scratch above the LHS's high-water (the disjoint-slot discipline
; shared with `Map.remove`/`Set.of`). n=100 → of-int(100)=100.0 ≠ 101.0=of-int(101) → false. Both backends.
(case
  "a param-left arith-right Float64.of-int equality compiles to a valid module and computes"
  (doc
    "Regression guard for the FloatCompare operand-pair sibling-scratch collision: `(= (Float64.of-int
           n) (Float64.of-int (+ n 1)))` — the left converts the bare param `n`, the right converts the
           checked-arith `(+ n 1)`. The wasm emit used to lay the right's i64 arith temp at the same scratch
           base the left's pending f64 needed → invalid module (order-specific: only this operand order). Now
           the right operand's scratch floats above the left's high-water. n=100 → 100.0 ≠ 101.0 → false.")
  (input
    (do (def (main (: n Int64)) (= (Float64.of-int n) (Float64.of-int (+ n 1)))) (export main)))
  (call main (: 100 Int64))
  (output (: false Bool)))

; The double-precision integer-boundary semantic pin the emit fix above UNBLOCKS (breaker's original probe):
; Float64 (binary64) has a 52-bit mantissa, so every integer up to 2^53 is representable exactly, but 2^53
; and 2^53+1 both round to the SAME f64 (the first adjacent-integer collapse). So `(= (Float64.of-int n)
; (Float64.of-int (+ n 1)))` — DISTINCT integers n and n+1 — is FALSE for small n (100 ≠ 101) but TRUE at
; n = 2^53 = 9007199254740992 (n and n+1 collapse to one f64). Pins the exact float-precision boundary.
(case
  "Float64.of-int collapses adjacent integers at the 2^53 precision boundary"
  (doc
    "`(= (Float64.of-int n) (Float64.of-int (+ n 1)))` compares the f64 conversions of two DISTINCT
           integers n, n+1. Below 2^53 they are distinct f64s → false (n=100 → 100.0 ≠ 101.0). AT n = 2^53 =
           9007199254740992, binary64's 52-bit mantissa cannot represent 2^53+1, so both round to the same
           f64 → true. Pins the double-precision integer-exactness boundary (adjacent ints collapse at 2^53),
           riding the param-left/arith-right emit fix above.")
  (input
    (do (def (main (: n Int64)) (= (Float64.of-int n) (Float64.of-int (+ n 1)))) (export main)))
  (call main (: 9007199254740992 Int64))
  (output (: true Bool))
  (call main (: 100 Int64))
  (output (: false Bool)))

(case
  "list ordering recurses into string elements by content across mixed reps"
  (doc
    "List `<` is blessed lexicographic-by-element (the Int pins); this recurses the element
           compare into STRING content with MIXED reps: a = [view \"key\", \"b\"] vs b = [rope, \"c\"].
           mode 1: the rope is \"key\" — the FIRST elements are equal ACROSS reps (view vs rope), so
           the tiebreak falls to \"b\" < \"c\" and a < b (1). mode 0: the rope is \"kex\" < \"key\",
           so b sorts first and a < b is FALSE (0) — the first-element compare must consult content,
           not rep identity; an eq-by-rep walk would fall through to the tiebreak and wrongly answer 1.")
  (input
    (do
      (def
        (main (: mode Int64))
        (do
          (def a #list((Option.expect (String.slice "xkeyz" 1 4) "in") "b"))
          (def b #list((String.concat "ke" (if (> mode 0) "y" "x")) "c"))
          (if (< a b) 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

; --- #42 Option-sum declared-order (Some<None) compare witnesses (v-rust-backend 7392dc3b8) ------
(case
  "three-way compare orders (Some 3) below None per the declared discriminant order"
  (input
    (do
      (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k)))
      (def
        (main (: a Int64) (: b Int64))
        (match
          (Ordering.of (mk a) (mk b))
          ((Ordering.Less _u) 1)
          ((Ordering.Equal _u) 2)
          ((Ordering.Greater _u) 3)))
      (export main)))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "the boolean ordering operator places (Some 3) below None like the three-way compare"
  (input
    (do
      (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k)))
      (def (main (: a Int64) (: b Int64)) (if (< (mk a) (mk b)) 1 0))
      (export main)))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "Result ordering agrees across targets — Ok below Err on the shared declaration order"
  (input
    (do
      (def (mk (: k Int64)) (if (= k 0) (: (Result.Err "e") (Result Int64 String)) (Result.Ok k)))
      (def (main (: a Int64) (: b Int64)) (if (< (mk a) (mk b)) 1 0))
      (export main)))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a tuple containing an Option leaf orders by the declared Some-below-None"
  (input
    (do
      (def (mk (: k Int64)) #tuple(7 (if (= k 0) (: (None unit) (Option Int64)) (Some k))))
      (def
        (main (: a Int64) (: b Int64))
        (match
          (Ordering.of (mk a) (mk b))
          ((Ordering.Less _u) 1)
          ((Ordering.Equal _u) 2)
          ((Ordering.Greater _u) 3)))
      (export main)))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a list of Options orders its elements by the declared Some-below-None"
  (input
    (do
      (def (mk (: k Int64)) #list((if (= k 0) (: (None unit) (Option Int64)) (Some k))))
      (def (main (: a Int64) (: b Int64)) (if (< (mk a) (mk b)) 1 0))
      (export main)))
  (call main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "Set.to-list over Option elements enumerates Some-first per the declared order"
  (doc
    "The #42 collection-key completion (v-rust-backend __CdzOpt Ord-wrapper, d946b02af): a Set of
           Option values enumerates its elements in the DECLARED Some-below-None order, not std's flipped
           None<Some. `Set.of (Some k) (None) (Some 1)` sorts to `(Some 1) (Some k) None`, so the head is a
           `Some` — the inner read yields `1`. rust Set/Map are BTreeSet/BTreeMap ordering keys by the KEY's
           derived Ord; the Ord-wrapper makes an Option key use declared Some<None, matching wasm. Completes
           #42 alongside the compare witnesses above (the compare-side fix ordered <, this orders the
           collection-key enumeration).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def s #set((Some k) (: (None unit) (Option Int64)) (Some 1)))
          (match
            (List.at (Set.to-list s) 0)
            ((Option.Some v) (match v ((Option.Some inner) inner) ((Option.None _u) -99)))
            ((Option.None _u) -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects 0))

; --- #43 all-nullary-sum discriminant order + render (v-wasm-opt cf0c05ae8 + v-runtime f9f8717c) ------
(case
  "an all-nullary user sum orders by discriminant — Lo below Hi"
  (input
    (do
      (type Tri (Lo) (Mid) (Hi))
      (def (mk (: k Int64)) (if (< k 0) (Tri.Lo unit) (if (= k 0) (Tri.Mid unit) (Tri.Hi unit))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (* 100 (if (< (mk a) (mk b)) 1 0))
          (+
            (* 10 (if (= (mk a) (mk b)) 1 0))
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))))
      (export main)))
  (call main (: -7 Int64) (: 9 Int64))
  (output (: 101 Int64)))

(case
  "the Sign sum orders Neg below Pos per its declaration"
  (input
    (do
      (def
        (mk (: k Int64))
        (if (< k 0) (Sign.Neg unit) (if (= k 0) (Sign.Zero unit) (Sign.Pos unit))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (* 100 (if (< (mk a) (mk b)) 1 0))
          (+
            (* 10 (if (= (mk a) (mk b)) 1 0))
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))))
      (export main)))
  (call main (: -7 Int64) (: 9 Int64))
  (output (: 101 Int64)))

(case
  "Ordering values order Less below Equal below Greater"
  (input
    (do
      (def (mk (: k Int64)) (Ordering.of k 0))
      (def
        (main (: a Int64) (: b Int64))
        (+ (* 10 (if (< (mk a) (mk b)) 1 0)) (if (< (mk b) (mk a)) 1 0)))
      (export main)))
  (call main (: -7 Int64) (: 9 Int64))
  (output (: 10 Int64)))

(case
  "Set.to-list over an all-nullary sum enumerates in discriminant order"
  (input
    (do
      (type Tri (Lo) (Mid) (Hi))
      (def (mk (: k Int64)) (if (< k 0) (Tri.Lo unit) (if (= k 0) (Tri.Mid unit) (Tri.Hi unit))))
      (def
        (main (: k Int64))
        (do
          (def s #set((Tri.Hi unit) (mk k) (Tri.Lo unit)))
          (+
            (* 10 (Set.len s))
            (match
              (List.at (Set.to-list s) 0)
              ((Option.Some v) (match v ((Tri.Lo _u) 1) ((Tri.Mid _u) 2) ((Tri.Hi _u) 3)))
              ((Option.None _u) -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 31 Int64))
  (live-objects 0))

(case
  "nullary variants of a payload-carrying sum order by discriminant"
  (input
    (do
      (type Mix (P Int64) (N1) (N2))
      (def (mk (: k Int64)) (if (< k 0) (Mix.N1 unit) (if (= k 0) (Mix.N2 unit) (Mix.P k))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: -1 Int64) (: 0 Int64))
  (output (: 10 Int64)))

(case
  "runtime Bools order false below true with compare and equality agreeing"
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (do
          (def x (= a 1))
          (def y (= b 1))
          (+
            (* 100 (if (< x y) 1 0))
            (+
              (*
                10
                (match
                  (Ordering.of x y)
                  ((Ordering.Less _u) 1)
                  ((Ordering.Equal _u) 2)
                  ((Ordering.Greater _u) 3)))
              (if (= x y) 1 0)))))
      (export main)))
  (call main (: 0 Int64) (: 1 Int64))
  (output (: 110 Int64)))

(case
  "a nested all-nullary sum renders the correct variant across the boundary (render half, v-runtime f9f8717c)"
  (input (do (type Tri (Lo) (Mid) (Hi)) (def (main) #tuple((Tri.Hi unit) 5)) (export main)))
  (call main)
  (output (: #tuple((Hi unit) 5) (Tuple Tri Int64))))

(case
  "a tuple key with a computed-NaN leaf is found by the canonical NaN probe"
  (doc
    "Composes the tuple-NaN equality pin (:176) with the CHAMP key path (the bare-NaN map key
           is 19-sets :1815): the key's float leaf is a COMPUTED `(/ x x)` NaN inside a tuple, and
           the probe spells it `Float64.nan` — champ_hash/eq must descend into the tuple and unify
           the two NaN spellings through the canonical byte form (hit → 10); the second-slot control
           (nan, 2) misses (0). A tuple hash that used the raw computed bit pattern (a different
           qNaN payload than the canonical constant) splits the spellings only in the COMPOUND case.")
  (input
    (do
      (def
        (main (: x Float64))
        (do
          (def m (Map.insert Map.empty #tuple((/ x x) 1) 42))
          (+
            (* 10 (match (Map.lookup m #tuple(Float64.nan 1)) ((Some v) 1) ((None _u) 0)))
            (match (Map.lookup m #tuple(Float64.nan 2)) ((Some v) 1) ((None _u) 0)))))
      (export main)))
  (call main (: 0.0 Float64))
  (output (: 10 Int64)))

; --- The compound-order walk: RECURSIVE descent (sum-in-sum), HEAP payloads (rope-in-sum), and the
; record's canonical field order — the perimeter of the two open cross-target sum-order findings
; (#42 rust builtin-Option flip, #43 wasm all-nullary), pinned on the shapes BOTH backends get right
; so those fixes cannot regress the working walk. All runtime-fed (no folds).
(case
  "a sum nested as another sum's payload orders by the inner discriminant when outers tie"
  (doc
    "The RECURSIVE-descent face of the sum order (the Ord2 pin above is one level): OB wraps IB; two OW values tie on the outer discriminant, so the walk must descend into the payload and compare the INNER sum's discriminant (IW=0 < IE=1 → the IE-carrying value is GREATER → 30). A walk that stopped at the outer tag would call them Equal; eq (0) confirms compare's verdict is not equality-blind.")
  (input
    (do
      (type IB (IW Int64) (IE))
      (type OB (OW IB) (OE))
      (def (mki (: k Int64)) (if (= k 1) (IB.IE unit) (IB.IW k)))
      (def (mk (: k Int64)) (if (= k 0) (OB.OE unit) (OB.OW (mki k))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 5 Int64))
  (output (: 30 Int64)))

(case
  "a sum nested as another sum's payload orders by the deep scalar payload when both levels tie"
  (doc
    "Both discriminant levels tie (OW(IW _) vs OW(IW _)) so the DEEP scalar decides: 3 < 5 → Less (10). Pins that the recursive walk reaches a depth-2 payload scalar after two tag ties.")
  (input
    (do
      (type IB (IW Int64) (IE))
      (type OB (OW IB) (OE))
      (def (mki (: k Int64)) (if (= k 1) (IB.IE unit) (IB.IW k)))
      (def (mk (: k Int64)) (if (= k 0) (OB.OE unit) (OB.OW (mki k))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 10 Int64)))

(case
  "the outer discriminant is decisive over any inner content for nested sums"
  (doc
    "OE (disc 1) vs OW(IE) (disc 0): the outer discriminants differ so the inner content must never be read — OE > OW whatever the payload (30). The decisive-tag face of the nested walk; with the two tie faces above it pins tag-first at EVERY level.")
  (input
    (do
      (type IB (IW Int64) (IE))
      (type OB (OW IB) (OE))
      (def (mki (: k Int64)) (if (= k 1) (IB.IE unit) (IB.IW k)))
      (def (mk (: k Int64)) (if (= k 0) (OB.OE unit) (OB.OW (mki k))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 0 Int64) (: 1 Int64))
  (output (: 30 Int64)))

(case
  "a sum with a rope String payload orders by content when variants tie"
  (doc
    "The HEAP-payload face of the sum order: two T-variant values tie on the discriminant and carry ROPE Strings (runtime String.concat) — the payload compare must content-canonicalize mid-sum (a chunk-shape/pointer compare would order by allocation, not content). T\"ab\" < T\"ac\" → Less (10).")
  (input
    (do
      (type SP (T String) (U))
      (def
        (mk (: k Int64))
        (if (= k 0) (SP.U unit) (SP.T (String.concat "a" (if (= k 1) "b" "c")))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 10 Int64)))

(case
  "a sum with a rope String payload orders by discriminant before any content"
  (doc
    "Discriminant-first with a heap payload present: T (disc 0) < U (disc 1) regardless of the rope content (10) — the payload is never read when tags differ.")
  (input
    (do
      (type SP (T String) (U))
      (def
        (mk (: k Int64))
        (if (= k 0) (SP.U unit) (SP.T (String.concat "a" (if (= k 1) "b" "c")))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64))
  (output (: 10 Int64)))

(case
  "a sum with a rope String payload compares Equal exactly when = is true"
  (doc
    "The agreement face (core-semantics.md #331): compare says Equal on two rope-vs-flat-equal T values exactly where = says true (21). A compare that keyed on chunk shape would say unequal while = (content) says equal — the divergence this pins against.")
  (input
    (do
      (type SP (T String) (U))
      (def
        (mk (: k Int64))
        (if (= k 0) (SP.U unit) (SP.T (String.concat "a" (if (= k 1) "b" "c")))))
      (def
        (main (: a Int64) (: b Int64))
        (+
          (*
            10
            (match
              (Ordering.of (mk a) (mk b))
              ((Ordering.Less _u) 1)
              ((Ordering.Equal _u) 2)
              ((Ordering.Greater _u) 3)))
          (if (= (mk a) (mk b)) 1 0)))
      (export main)))
  (call main (: 1 Int64) (: 1 Int64))
  (output (: 21 Int64)))

(case
  "record ordering compares in canonical sorted field order, not written order"
  (doc
    "The RECORD face of the compound order (core-semantics.md:341 — 'the same canonical order its equality and canonical byte form use'): fields written (zebra, apple) compare in SORTED order, so apple decides FIRST — (z1,a9) vs (z2,a0) is Greater (3) by apple 9>0, though written-order zebra 1<2 would say Less. Runtime k blocks the fold.")
  (input
    (do
      (def (mk (: z Int64) (: a Int64)) #record((= zebra z) (= apple a)))
      (def
        (main (: k Int64))
        (match
          (Ordering.of (mk 1 9) (mk (+ 2 k) 0))
          ((Ordering.Less _u) 1)
          ((Ordering.Equal _u) 2)
          ((Ordering.Greater _u) 3)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

(case
  "record ordering falls to the later canonical field when the earlier ties"
  (doc
    "The tie face: apple 5 = 5, so zebra (canonically SECOND) decides — 1 < 4 → Less (1). With the decisive face above it pins both directions of the sorted-field walk.")
  (input
    (do
      (def (mk (: z Int64) (: a Int64)) #record((= zebra z) (= apple a)))
      (def
        (main (: k Int64))
        (match
          (Ordering.of (mk 1 5) (mk (+ 4 k) 5))
          ((Ordering.Less _u) 1)
          ((Ordering.Equal _u) 2)
          ((Ordering.Greater _u) 3)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; --- The compound-order HEAP-LEAF matrix, remaining rows: Symbol, Rational, BigInt inside tuples
; (the String/rope row landed with the drain-F perimeter; scalar Int/Bool rows are the original
; pins). Each leaf kind has its own canonical compare (content-lexicographic / exact cross-multiply
; with reduction / arbitrary-precision) that must run MID-WALK with ties falling through.
(case
  "a Symbol leaf in a tuple orders content-lexicographically and decisively before later fields"
  (doc
    "The SYMBOL row of the compound-order heap-leaf matrix (String/rope, Rational, BigInt are the siblings): runtime-interned Symbols inside tuples — sym decisive before the numeric field ((alpha,9)<(beta,0) → 1), sym TIE falling to the number (Ordering.of Equal-path → Less at k=5), and an eq control. A walk comparing Symbols by intern handle/allocation order instead of content breaks the first face.")
  (input
    (do
      (def (mk (: s String) (: n Int64)) #tuple((Symbol.of (String.concat s "")) n))
      (def
        (main (: k Int64))
        (+
          (* 100 (if (< (mk "alpha" 9) (mk "beta" 0)) 1 0))
          (+
            (*
              10
              (match
                (Ordering.of (mk "beta" 1) (mk "beta" (+ 1 k)))
                ((Ordering.Less _u) 1)
                ((Ordering.Equal _u) 2)
                ((Ordering.Greater _u) 3)))
            (if (= (mk "alpha" 5) (mk "alpha" 5)) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 121 Int64))
  (call main (: 5 Int64))
  (output (: 111 Int64)))

(case
  "a Rational leaf in a tuple orders by exact value with canonical-form ties falling through"
  (doc
    "The RATIONAL row of the heap-leaf matrix: (a) exact cross-multiply decisive mid-walk (1/3 < 3/6); (b) the CANONICAL tie — compare (1/2,5) vs (3/6,5) must see the rationals EQUAL (an unreduced num/den compare orders them) and fall to the tied scalar → Equal; (c) canonical tie via 2/6=1/3 falling to the second field. Runtime a blocks the fold.")
  (input
    (do
      (def (mk (: n Int64) (: d Int64) (: t Int64)) #tuple((Rational.of n d) t))
      (def
        (main (: a Int64))
        (+
          (* 100 (if (< (mk 1 3 9) (mk a 6 0)) 1 0))
          (+
            (*
              10
              (match
                (Ordering.of (mk 1 2 5) (mk a 6 5))
                ((Ordering.Less _u) 1)
                ((Ordering.Equal _u) 2)
                ((Ordering.Greater _u) 3)))
            (if (< (mk 2 6 1) (mk 1 3 2)) 1 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 121 Int64)))

(case
  "a multi-limb BigInt leaf in a tuple orders by the high limb and ties fall to the next field"
  (doc
    "The BIGINT row of the heap-leaf matrix, at MULTI-limb scale: arith-built 2^64-magnitude operands EQUAL in the low limb — a walk comparing only 64 bits calls them equal and falls through wrongly; the high limb must decide ((h3,9)<(h5,0) → 1). compare Equal on identical multi-limb values, then tie → next field.")
  (input
    (do
      (def
        (mk (: h Int64) (: t Int64))
        (do
          (def b64 (* (BigInt.of 4294967296) (BigInt.of 4294967296)))
          #tuple((+ (* b64 (BigInt.of h)) (BigInt.of 5)) t)))
      (def
        (main (: a Int64))
        (+
          (* 100 (if (< (mk 3 9) (mk a 0)) 1 0))
          (+
            (*
              10
              (match
                (Ordering.of (mk 5 1) (mk a 1))
                ((Ordering.Less _u) 1)
                ((Ordering.Equal _u) 2)
                ((Ordering.Greater _u) 3)))
            (if (< (mk 5 1) (mk 5 2)) 1 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 121 Int64)))

; --- Ordering as a first-class value: the lazy comparator chain. ---
(case
  "a LAZY comparator chain (Ordering + thunk) short-circuits on decisive and falls through on Equal"
  (doc
    "Ordering consumed as a FIRST-CLASS value driving a user combinator (the sort-by-key-then-key idiom): chain o1 k returns o1 unless Equal, then forces thunk k. Decisive-first must NOT invoke k (the wildcard `other` arm re-binds and returns the builtin sum intact); Equal falls to the thunk (String tie → age compare); Equal→Equal ties out. Ordering as fn param AND fn result.")
  (input
    (do
      (def
        (chain (: o1 Ordering) (: k (-> Unit Ordering)))
        (match o1 ((Ordering.Equal _u) (k unit)) (other other)))
      (def
        (cmp-person (: n1 String) (: a1 Int64) (: n2 String) (: a2 Int64))
        (chain (Ordering.of n1 n2) (fn ((: _u Unit)) (Ordering.of a1 a2))))
      (def
        (ord-code (: o Ordering))
        (match o ((Ordering.Less _u) 1) ((Ordering.Equal _u) 2) ((Ordering.Greater _u) 3)))
      (def
        (main (: a Int64))
        (+
          (* 100 (ord-code (cmp-person "amy" 30 "bob" a)))
          (+
            (* 10 (ord-code (cmp-person "bob" 25 "bob" a)))
            (ord-code (cmp-person "bob" a "bob" a)))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 112 Int64)))

; --- Unorderable/incomparable leaves inside compounds: Bytes/Float sum payloads decline compare;
; a closure leaf declines equality (no reference-eq fallback). ---
(case
  "compare on a sum whose payload is BYTES orders by discriminant then Bytes payload lexicographically"
  (doc
    "A Bytes leaf now offers a blessed TOTAL order (§order, content-lexicographic over unsigned bytes),
           so it composes soundly inside a sum PAYLOAD (unlike a float, whose IEEE partial order still declines
           below). A same-variant compare enters the discriminant-then-payload walk (`ValueCmp{op:Compare}`):
           both are `BP.T`, so the payloads decide — `a = T([1,k])`, `b = T([1,3])`. With `k = 5` the payload
           `[1,5]` > `[1,3]` (first byte equal, second 5>3), so `compare a b` = Greater → 3. This REVERSES the
           former decline (operator directive 2026-08-02); the sum-payload Bytes walk now agrees with the bare
           Bytes `<` and both backends. Contrast the FLOAT-payload sibling below, which still declines (§319).")
  (input
    (do
      (type BP (T Bytes) (U))
      (def
        (main (: k Int64))
        (do
          (def a (BP.T (Bytes.of #list(1 (UInt8.wrap k)))))
          (def b (BP.T (Bytes.of #list(1 3))))
          (match
            (Ordering.of a b)
            ((Ordering.Less _u) 1)
            ((Ordering.Equal _u) 2)
            ((Ordering.Greater _u) 3))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "compare on a sum whose payload is a FLOAT is rejected CDZ0203 (no total order; the IEEE partial order never enters the walk)"
  (doc
    "The float sibling: a Float64 payload makes the sum un-orderable per the §319 carve-out; same-variant compare declines rather than smuggling the IEEE partial order (or the canonical byte order) into the sum walk.")
  (input
    (do
      (type FP (T Float64) (U))
      (def
        (main (: k Int64))
        (do
          (def a (FP.T (if (= k 1) 1.5 2.5)))
          (def b (FP.T 2.0))
          (match
            (Ordering.of a b)
            ((Ordering.Less _u) 1)
            ((Ordering.Equal _u) 2)
            ((Ordering.Greater _u) 3))))
      (export main)))
  (error CDZ0203))

(case
  "equality on a TUPLE containing a closure is rejected CDZ0216 (NotEquatable — the fn leaf poisons the compound walk)"
  (doc
    "A fn value is NEVER equatable — CDZ0216 (NotEquatable), a PERMANENT reject (v-deferral grade). A fn INSIDE a compound must not fall back to handle/reference eq — (= (tuple 1 f) (tuple 1 f)) holds the SAME f both sides, so a reference-eq walk would return TRUE, silently blessing identity semantics the spec forbids. Distinct from the direct bare-fn = (CDZ0203 type-side); the compound-walk hits the closure leaf and rejects NotEquatable. Must NEVER flip to pass-with-1.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def f (fn ((: x Int64)) (+ x k)))
          (def t1 #tuple(1 f))
          (def t2 #tuple(1 f))
          (if (= t1 t2) 1 0)))
      (export main)))
  (error CDZ0216))

(case
  "a tuple with a Char leaf orders by codepoint — Char is a blessed compound-ordering leaf"
  (doc
    "`(< (mk #\\a) (mk #\\b))` where `mk` builds a `(tuple 1 c)` with a Char component. Compound
           ordering is offered exactly when EVERY component offers a total order, and a Char DOES: a
           Unicode scalar value has a total order by codepoint — the same order scalar `(Ordering.of #\\a
           #\\b)` computes (13-strings:3092). So the tuple walk orders the Char leaf by codepoint: the
           first components tie (1 = 1), the Char leaf decides — #\\a (U+0061) < #\\b (U+0062) — so
           `(< (tuple 1 #\\a) (tuple 1 #\\b))` is true → 1. Char joins the blessed compound-ordering leaf
           vocabulary (Int/Bool/String/Symbol/Bytes/…), exactly as Bytes was blessed into the walk by
           PR#1120; FLOAT (IEEE partial order) and a CLOSURE leaf remain the carve-outs. Uniform across
           backends (the runtime `value_cmp_shaped` orders a Char-in-compound as its codepoint `Shape::Int`
           — no runtime change was needed to bless it, only the `is_orderable_compound` guard).")
  (input
    (do (def (mk (: c Char)) #tuple(1 c)) (def (main) (if (< (mk #\a) (mk #\b)) 1 0)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "cho1 a runtime-selected Char in a compound orders by full CODEPOINT at run time — a multi-byte scalar sorts above ASCII"
  (doc
    "The RUNTIME + multi-byte companion of the fold-path ASCII tuple case above (#4862): the Char field
           of a `(tuple c 0)` is SELECTED at run time by a branch (`(if b #\\a #\\🦀)`), so the comparison runs
           through the runtime `value_cmp_shaped` (a Char-in-compound orders as its codepoint `Shape::Int`), not
           const-fold. Ordered against `(tuple #\\m 0)`: at b=true the tuple holds `#\\a` (U+0061=97) < `#\\m`
           (U+006D=109) → 1; at b=false it holds `#\\🦀` (U+1F980=128384) which is NOT < `#\\m` → 0. The 🦀 leg
           is the byte-vs-codepoint fence — a compare keyed on a low byte or the UTF-8 first byte (0xF0) rather
           than the full scalar would mis-order it. Uniform across wasm/rust/rust-async. (Runtime Chars have no
           representation, so the Char is a compile-time constant SELECTED at run time — the only way the
           runtime Char-in-compound path is reachable.)")
  (input
    (do (def (main (: b Bool)) (if (< #tuple((if b #\a #\🦀) 0) #tuple(#\m 0)) 1 0)) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

; breaker probe W — stress the cmp-walk recursion just pinned in 3c223e37b one level DEEPER:
; a LIST of user SUMS whose payload is itself a LIST — the walk must recurse list→sum→list.
; Also the discriminant-before-payload rule at the deeper level, and prefix tiebreak on the
; INNER list.
; Hand-derived (type W (Leaf Int64) (Node (List Int64))):
;   a1 = [Node [1,2]], a2 = [Node [1,3]]: outer lists len-1, elem: same disc(Node), payload [1,2]<[1,3] → true → 1.
;   b1 = [Leaf 5], b2 = [Node [0]]: disc Leaf(0) < Node(1) → true → 1 (payload never read; a walk
;     that read Node's list against Leaf's scalar would type-confuse/crash).
;   c1 = [Node [1]], c2 = [Node [1,2]]: inner prefix rule → true → 1.
;   main = 100*1 + 10*1 + 1 = 111.
(case
  "the compare walk recurses list-of-sums-of-lists with discriminant-first at depth"
  (input
    (do
      (type W (Leaf Int64) (Node (List Int64)))
      (def
        (main (: k Int64))
        (+
          (* 100 (if (< #list((Node #list(1 k))) #list((Node #list(1 3)))) 1 0))
          (+
            (* 10 (if (< #list((Leaf 5)) #list((Node #list(0)))) 1 0))
            (if (< #list((Node #list(1))) #list((Node #list(1 k)))) 1 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 111 Int64)))

(case
  "a user (type Ordering …) shadows the built-in, so Ordering.of is no longer the three-way comparison"
  (doc
    "`Ordering.of` is the NAMESPACED three-way comparison — an associated function on the BUILT-IN
           `Ordering` record (the former top-level `compare`), reached by ordinary member access. A user
           `(type Ordering …)` shadows the built-in (a top-level type declaration resolves before the
           prelude) and carries no associated `of` member, so `Ordering.of` is an ordinary unknown-member
           access (CDZ0201), NOT the comparison. This is the binding-respecting property namespacing on a
           shadowable record delivers and a bare global `compare` could not: the comparison follows binding.")
  (input (do (type Ordering (Foo)) (def (main) (Ordering.of 1 2)) (export main)))
  (error CDZ0201))

(case
  "a value-equality over a borrowed heap list leaves no live heap objects"
  (doc
    "`(let ((xs (build 3))) (if (= xs (build 3)) 1 0))` — the let-bound list `xs` is BORROWED by `=`
           (structural equality) and compared to a fresh `(build 3)` (an owned temporary `=` drops); the
           result is the scalar 1, so `xs` is used only as the borrowed operand. `=` must NOT drop `xs` (it
           only borrows) — the enclosing `let` reclaims it exactly once. So after the run the live-cell count
           is 0: the fresh operand reclaimed by `=`, `xs` by the `let`, neither leaked nor double-freed.")
  (input
    (do
      (type IntList (Cons (Tuple Int64 IntList)) Nil)
      (def (build n) (if (< n 1) (IntList.Nil ()) (IntList.Cons #tuple(n (build (- n 1))))))
      (def (main) (let ((xs (build 3))) (if (= xs (build 3)) 1 0)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime String rope compares equal to its flat twin and leaves no live heap objects"
  (doc
    "`rep` appends \"x\" 3x via String.concat -> an OWNED rope whose content is \"hixxx\"; `(= rope
           \"hixxx\")` is true (value-eq compacts the rope operand first so rope-bytes match the flat leaf --
           a champ_eq physical-byte miscompile without it) -> 1. The owned rope operand + its compacted flat
           leaf net to 0 live cells after the borrowing compare.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= (rep "hi" 3) "hixxx") 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime Bytes rope compares equal to its flat twin and leaves no live heap objects"
  (doc
    "`rep` appends byte 120 ('x') once via Bytes.concat -> an OWNED rope whose content is [104,105,120];
           `(= rope (Bytes.of (list 104 105 120)))` is true (the direct-Bytes value-eq compaction) -> 1. The
           owned rope operand + its compacted flat leaf net to 0 live cells after the borrowing compare.")
  (input
    (do
      (def
        (rep (: b Bytes) (: n Int64))
        (if (< n 1) b (rep (Bytes.concat b (Bytes.of #list(120))) (- n 1))))
      (def (main) (if (= (rep (Bytes.of #list(104 105)) 1) (Bytes.of #list(104 105 120))) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "String.scalar-len over an owned-temporary runtime rope leaves no live heap objects"
  (doc
    "`rep` appends \"x\" 3x via String.concat -> an OWNED rope \"hixxx\" (5 unicode scalars);
           `String.scalar-len` borrows it (bytes-len/bytes-get walk) -> 5, and the owned rope must be
           reclaimed after the walk (the Owned-operand gate, like Bytes.len) -- net 0 live cells.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (String.scalar-len (rep "hi" 3)))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a runtime String ordering compare over a let-bound rope operand leaves no live heap objects"
  (doc
    "`rep` appends \"x\" 3x via String.concat -> an OWNED rope \"hixxx\", LET-BOUND as `r` and KEPT (used
           as a direct `<` operand AND read again by String.byte-len). `(< r \"zzzzzzzz\")` is true, so main
           returns byte-len 5. StrCmp borrows r (leaves it to its owner), so the kept let must drop it -- net
           0 live cells (a let-bound-direct-operand mis-classification leaked it pre-fix).")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (let ((r (rep "hi" 3))) (if (< r "zzzzzzzz") (String.byte-len r) -1)))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "runtime structural equality of two owned cons-lists reclaims both operands (no live objects)"
  (doc
    "Two OWNED cons-list operands (each `(build 3)` recursion-built so neither folds); `=` (value-eq)
           borrows both and the emit must drop each after the compare -> `(= (build 3) (build 3))` is true so
           main returns 1, and both whole lists must be reclaimed -- net 0 live cells.")
  (input
    (do
      (type IntList (Cons (Tuple Int64 IntList)) Nil)
      (def (build n) (if (< n 1) (IntList.Nil ()) (IntList.Cons #tuple(n (build (- n 1))))))
      (def (main) (if (= (build 3) (build 3)) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "runtime shaped equality of two owned List-Float64 operands reclaims both (no live objects)"
  (doc
    "Two OWNED runtime `List Float64` operands, each `(build 0 Float64.nan)` built opaquely through a
           recursive float-param builder (so neither const-folds); `=` routes to the shaped-float walk
           (value-eq-shaped) which borrows both and drops each owned temporary -> equal -> main returns 1;
           both built lists must be reclaimed -- net 0 live cells.")
  (input
    (do
      (def (build (: n Int64) (: x Float64)) (if (< n 0) (build (+ n 1) x) #list(x)))
      (def (main) (if (= (build 0 Float64.nan) (build 0 Float64.nan)) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a rope String nested in a tuple compares equal to its flat twin and reclaims both tuples (no live objects)"
  (doc
    "The value heap is TAGLESS, so champ_eq compares a nested leaf by physical bytes; the fix compacts a
           String leaf at the COMPOUND CONSTRUCTION SITE so no compound holds a rope. `rep \"hi\" 3` builds an
           OWNED rope \"hixxx\"; `(= (tuple (rep \"hi\" 3) 1) (tuple \"hixxx\" 1))` is true -> 1. The compact
           consumes each rope + stores a flat leaf, so the two tuples the borrowing value-eq drops net to 0.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def (main) (if (= #tuple((rep "hi" 3) 1) #tuple("hixxx" 1)) 1 0))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a compound map key whose string element is a rope is found by its flat-twin key"
  (doc
    "A tuple key whose string element is a rope must hash into the SAME CHAMP slot as its flat-twin
           query key (the construction-site compact stores a flat leaf, so the hash matches). Insert
           (tuple (rep \"hi\" 3) 1)->42, look up (tuple \"hixxx\" 1) -> Some 42 (was None -> -1 without the
           compact, hashing the uncompacted rope into a different slot).")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main)
        (match
          (Map.lookup (Map.insert Map.empty #tuple((rep "hi" 3) 1) 42) #tuple("hixxx" 1))
          ((Some v) v)
          ((None) -1)))
      (export main)))
  (call main)
  (output (: 42 Int64)))

; -- breaker batch 406 (2026-08-26): bare runtime Bytes EQUALITY over provably-FLAT operands
; (Bytes.of, Bytes.concat) — the = twins of the pinned bare-< total-order case. Controls for the
; filed decline: bare compare with a VIEW (slice), arena (Ast.encode / String.to-bytes), or
; laundered-param operand declines while the compound-walk path flattens correctly (finding-#16
; family, bare-position canonicalization).
(case
  "bfl1 bare equality over flat Bytes.of runtime twins"
  (input
    (do
      (def (mk (: n Int64)) (Bytes.of #list((UInt8.wrap n) 2)))
      (def (f (: n Int64)) (if (= (mk n) (mk n)) 1 0))
      (export f)))
  (call f (: 2 Int64))
  (output (: 1 Int64)))

(case
  "bfl2 bare equality across Bytes.concat vs Bytes.of flat twins"
  (input
    (do
      (def
        (f (: n Int64))
        (if
          (=
            (Bytes.concat (Bytes.of #list((UInt8.wrap n))) (Bytes.of #list(2)))
            (Bytes.of #list((UInt8.wrap n) 2)))
          1
          0))
      (export f)))
  (call f (: 5 Int64))
  (output (: 1 Int64)))

; -- breaker batch 407 (2026-08-26): bare ORDER over arena-sourced Bytes works — bo1 pins that
; `(< (Ast.encode a) (Ast.encode b))` compiles and compares content-lexicographically (encode(7) <
; encode(8) on the payload byte), so the flat-operands gate is EQUALITY-specific: bare `=` over the
; same operands declines (filed) while `<` and the tuple-walk `=` (bo2) both handle arena sources.
; De-confound note: earlier order declines were String ENTRY params (a separate filed decline —
; String/Bytes entry params decline wholesale, even for .len, while Symbol/BigInt entry params pass).
(case
  "bo1 bare order over two runtime encode results compares content-lexicographically"
  (input
    (do
      (def
        (f (: n Int64))
        (if (< (Ast.encode (Ast.Int (BigInt.of n))) (Ast.encode (Ast.Int (BigInt.of (+ n 1))))) 1 0))
      (export f)))
  (call f (: 7 Int64))
  (output (: 1 Int64)))

(case
  "bo2 tuple-walk equality over two identical runtime encodes agrees"
  (input
    (do
      (def
        (f (: n Int64))
        (if
          (=
            #tuple(1 (Ast.encode (Ast.Int (BigInt.of n))))
            #tuple(1 (Ast.encode (Ast.Int (BigInt.of n)))))
          1
          0))
      (export f)))
  (call f (: 7 Int64))
  (output (: 1 Int64)))

; -- breaker batch 414 (2026-08-26): RUNTIME-built collection equality pins — two runtime Maps,
; two runtime Sets, and runtime lists of records all compare by value (contrast: BARE runtime
; Bytes = remains the filed flat-operands-only decline).
(case
  "ce08 equality of two RUNTIME-built Maps"
  (input (do (def (f (: n Int64)) (= (Map.insert #map() n 1) (Map.insert #map() n 1))) (export f)))
  (call f 3)
  (output (: true Bool)))

(case
  "ce09 equality of two RUNTIME-built Sets"
  (input (do (def (f (: n Int64)) (= #set(n 2) #set(2 n))) (export f)))
  (call f 1)
  (output (: true Bool)))

(case
  "ce10 equality of RUNTIME lists of records"
  (input (do (def (f (: n Int64)) (= #list(#record((= a n))) #list(#record((= a n))))) (export f)))
  (call f 4)
  (output (: true Bool)))

; -- breaker batch 425 (2026-08-26): bare Bytes equality over NON-FLAT operands on #3786 — encode
; results, String.to-bytes twins, param-laundered arena bytes, and a slice VIEW vs its flat twin all
; compare correctly (the former flat-operands-only gate is closed). OUTPUT-ONLY pins pending the
; borrowing-op owned-operand reclaim follow-up.
(case
  "aeq1 bare equality over two runtime Ast.encode results"
  (input
    (do
      (def
        (f (: n Int64))
        (if (= (Ast.encode (Ast.Int (BigInt.of n))) (Ast.encode (Ast.Int (BigInt.of n)))) 1 0))
      (export f)))
  (call f (: 7 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "aeq2 bare equality of String.to-bytes twins"
  (input
    (do
      (def
        (main (: n Int64))
        (let ((s (if (= n 1) "hi" "yo"))) (if (= (String.to-bytes s) (String.to-bytes s)) 1 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "aeq3 arena Bytes laundered through fn params bare-compare equal"
  (input
    (do
      (def (cmp (: a Bytes) (: b Bytes)) (if (= a b) 1 0))
      (def
        (f (: n Int64))
        (cmp (Ast.encode (Ast.Int (BigInt.of n))) (Ast.encode (Ast.Int (BigInt.of n)))))
      (export f)))
  (call f (: 7 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "aeq4 a slice VIEW bare-compares equal to its flat twin"
  (input
    (do
      (def
        (f (: n Int64))
        (if
          (=
            (Option.expect (Bytes.slice (Bytes.of #list(9 (UInt8.wrap n) 2 7)) 1 2) "in bounds")
            (Bytes.of #list((UInt8.wrap n) 2)))
          1
          0))
      (export f)))
  (call f (: 5 Int64))
  (output (: 1 Int64))
  (live-objects 0))

; ── value-eq: a BORROWED runtime String rope compares equal to its flat twin (compaction is leak-neutral; migrated from rcdzc) ──
(case
  "a borrowed runtime String rope compares equal to its flat twin and its compaction is leak-neutral"
  (doc
    "`rep` builds an OWNED rope \"hixxx\" (three String.concat), stored as a map value; `f` looks it up
           and compares the BORROWED Some payload `s` against the flat literal \"hixxx\" INSIDE the arm. The
           `=` operand `s` is a BORROWED rope — the case the owned-only compaction missed: `=` lowers to
           champ_eq (physical bytes), and a concat rope's bytes differ from a flat leaf's, so it once compared
           UNEQUAL (0). The emit now compacts a borrowed String operand in place before the compare -> equal
           (1). Compacting a BORROWED operand is refcount-neutral (in-place flatten, same handle, no drop
           follows the borrow), so it leaves the SAME live count as the byte-identical flat-value baseline
           below (known-leak 2, a pre-existing map-temporary residual the scalar-returning main does not yet
           reclaim). Their equal 2 is the leak-neutrality guard: a compaction leak would push this above 2.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (f (: mp (Map String String)) (: k String))
        (match (Map.lookup mp k) ((Some s) (if (= s "hixxx") 1 0)) ((None) -1)))
      (def (main) (f (Map.insert (Map.empty) "y" (rep "hi" 3)) "y"))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "the flat-value baseline for the borrowed-rope-eq leak-neutrality (same map/value-box residual)"
  (doc
    "The byte-identical flat-value baseline for the borrowed-rope-eq neutrality pin above: the map value
           is the flat literal \"hixxx\" (no rope, no compaction needed). It builds the SAME map + value-box
           shape, whose pre-existing map-temporary residual (2 cells, orthogonal to the compaction) the
           scalar-returning main does not yet reclaim. Its known-leak 2 equalling the rope program's 2 proves
           the borrowed-operand compaction added nothing.")
  (input
    (do
      (def
        (f (: mp (Map String String)) (: k String))
        (match (Map.lookup mp k) ((Some s) (if (= s "hixxx") 1 0)) ((None) -1)))
      (def (main) (f (Map.insert (Map.empty) "y" "hixxx") "y"))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "Ordering.of yields the three-way ordering of two Int64 operands"
  (doc
    "`Ordering.of a b` deconstructed by a three-arm match → -1/0/1: 1<2 Less→-1, 2=2 Equal→0, 3>2 Greater→1.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (match
          (Ordering.of a b)
          ((Ordering.Less _) -1)
          ((Ordering.Equal _) 0)
          ((Ordering.Greater _) 1)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 2 Int64) (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64) (: 2 Int64))
  (output (: 1 Int64)))

(case
  "Ordering.of orders strings lexicographically in the three-way comparison"
  (doc
    "`Ordering.of` on string constants: \"a\" < \"b\" Less→-1, \"b\" = \"b\" Equal→0. main = 10*first + second = -10.")
  (input
    (do
      (def
        (main)
        (+
          (*
            10
            (match
              (Ordering.of "a" "b")
              ((Ordering.Less _) -1)
              ((Ordering.Equal _) 0)
              ((Ordering.Greater _) 1)))
          (match
            (Ordering.of "b" "b")
            ((Ordering.Less _) -1)
            ((Ordering.Equal _) 0)
            ((Ordering.Greater _) 1))))
      (export main)))
  (output (: -10 Int64)))

; ── breaker batch 522: the DUAL-USE (projection + value-eq) nested-compound leak family
; (issues/BUG-nested-compound-dual-use-projection-plus-value-eq-leaks-operand-tree, routed
; v-memory-safety). A nested compound BOTH projected-into and eq'd leaks its whole node tree per
; dual-used side; flat dual-use, eq-only, and unequal walks are all clean (pinned as 0-controls —
; they must STAY 0 through the fix, and the VALUES fence an over-drop that would corrupt the
; projection reads). dqe4/dqe5/dqe6 now reclaim CLEAN (fixed): a nested compound projected via a
; scalar-bottomed chain through compound intermediates is a pure BORROW, so a borrow-only projected
; binder must not mint the spurious unbalanced dup that mark_binder_dups's Proj arm minted; fixed in
; reclaim.rs by gating the compound-projection consuming/borrow transparency on binder_never_escapes
; (dqe17-safe: a binder that ESCAPES an arm keeps the dup). dqe7/dqe8 (champ-key / Set membership) now
; reclaim CLEAN too: their second use CONSUMES the operand as a KEY into a temporary collection on EVERY
; path, so that consume already carries its own dup and the compound-projection keep-alive dup is SURPLUS —
; suppressed by extending the Proj gate with binder_must_escape (a sound all-paths under-approximation that
; distinguishes this UNCONDITIONAL consume from dqe17's CONDITIONAL arm-escape, which still keeps its dup).
(case
  "dqe1 FLAT tuple dual-use (projections + runtime equality) reclaims clean — the control the nested cells contrast"
  (input
    (do
      (def
        (main (: n Int64))
        (let ((a #tuple(n 2)) (b #tuple(n 2))) (+ (. a 0) (+ (. b 1) (if (= a b) 100000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100003 Int64))
  (live-objects 0))

(case
  "dqe2 nested runtime tuples under equality ALONE (two walks, no projections) reclaim clean"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (if (= a b) 100000 0) (if (= b a) 10000 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 110000 Int64))
  (live-objects 0))

(case
  "dqe3 nested runtime tuples with an UNEQUAL leaf under equality reclaim clean"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n (+ n 1))))))
          (if (= a b) 100000 1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "dqe4 ONE nested operand dual-used (deep projection + equality) reclaims clean — borrow-only projected binder mints no spurious dup"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (* 1000 (. (. (. a 1) 1) 1)) (if (= a b) 100000 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101000 Int64))
  (live-objects 0))

(case
  "dqe5 BOTH nested operands dual-used (projections + equality) reclaim clean — borrow-only projected binders mint no spurious dup"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (* 1000 (. (. (. a 1) 1) 1)) (+ (. (. b 1) 0) (if (= a b) 100000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101001 Int64))
  (live-objects 0))

; ── breaker batch 523: the dual-use leak GENERALIZED (same issue file, scope corrected) — the
; second consumer can be ANY heap walker (order / champ-key / Set.contains), not just value-eq;
; two walkers WITHOUT a projection are clean (dqe9). Fix must target the generic dup/drop
; placement for projected-and-walked nested compounds: dqe6 (ORDERING / ValueCmp, a BORROW like value-eq)
; now reclaims clean with the same borrow-only projected-binder dup-suppression as dqe4/5. dqe7/dqe8 now
; reclaim clean too — their second use CONSUMES the operand as a KEY into a temporary Map/Set on EVERY path
; (an UNCONDITIONAL escape whose own consume-dup covers the refcount), so the compound-projection keep-alive
; dup is surplus and is suppressed via the binder_must_escape all-paths gate; dqe17's CONDITIONAL arm-escape
; is preserved (it still needs its dup for the non-escape path).
(case
  "dqe6 a nested operand dual-used by projection + ORDERING walk reclaims clean — borrow-only projected binder mints no spurious dup"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (* 1000 (. (. (. a 1) 1) 1)) (if (< a b) 100000 1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1001 Int64))
  (live-objects 0))

(case
  "dqe7 a nested operand dual-used by projection + CHAMP-key descent (insert one, look up by the equal twin) leaks its tree"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+
            (* 1000 (. (. (. a 1) 1) 1))
            (match (Map.lookup (Map.insert (Map.empty) a 42) b) ((Some v) v) ((None u) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1042 Int64))
  (live-objects 0))

(case
  "dqe8 a nested operand dual-used by projection + Set membership descent leaks its tree"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (* 1000 (. (. (. a 1) 1) 1)) (if (Set.contains #set(a) b) 100 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1100 Int64))
  (live-objects 0))

(case
  "dqe9 TWO walkers (equality AND ordering) on the same nested operands with no projection reclaim clean"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (if (= a b) 100000 0) (if (< a b) 10000 1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100001 Int64))
  (live-objects 0))

; ── breaker batch 524: the ESCAPE cells of the dual-borrow leak (ownership settled: v-core-opt,
; mark_binder_dups end-of-scope drop). A walked operand ESCAPING through a branch arm leaks BOTH
; sides today (6) — an existing under-drop. The coming fix must drop the dead sibling WITHOUT
; dropping the escapee: the 1001 values (read through the returned tree in the caller) fence the
; UAF side; the known-leak 6 clauses flip to 0. dqe12 = callee-scope eq-only 0-control.
(case
  "dqe10 an eq'd nested operand ESCAPING whole through the branch arm leaks both sides today (escapee must survive the coming end-of-scope drop)"
  (input
    (do
      (def
        (f (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (if (= a b) a #tuple(9 #tuple(9 #tuple(9 9))))))
      (def (main (: n Int64)) (let ((r (f n))) (+ (* 1000 (. (. (. r 1) 1) 1)) (. r 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1001 Int64))
  (live-objects known-leak))

(case
  "dqe11 an eq'd nested operand whose COMPONENT escapes through the branch arm leaks both sides today (partial escape)"
  (input
    (do
      (def
        (h (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (if (= a b) (. a 1) #tuple(9 #tuple(9 9)))))
      (def (main (: n Int64)) (let ((r (h n))) (+ (* 1000 (. (. r 1) 1)) (. r 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1001 Int64))
  (live-objects known-leak))

(case
  "dqe12 eq-only nested operands confined to a callee scope (nothing escapes) reclaim clean"
  (input
    (do
      (def
        (g (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+ (* 100000 (if (= a b) 1 0)) 0)))
      (def (main (: n Int64)) (g n))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100000 Int64))
  (live-objects 0))

; ── breaker batch 525: KIND/CONSUMER narrowing of the dual-borrow leak (issue leg-1) — only the
; TUPLE positional-index projection mints the never-dropped dup. The exact leaking shape flips
; clean when the extraction is a match-destructure (dqe13, same shape as dqe4), a record field
; read (dqe14, heap field), or a sum match-extract (dqe15, heap payload). 0-controls: must STAY
; 0 through the fix, values fence an over-drop on the correctly-releasing paths.
(case
  "dqe13 the dqe4 shape with MATCH-destructure instead of tuple projection reclaims clean (the leak is projection-specific)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple(n #tuple(n #tuple(n n)))))
          (+
            (* 1000 (match a (#tuple(p q) (match q (#tuple(r s) (match s (#tuple(t u) u)))))))
            (if (= a b) 100000 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101000 Int64))
  (live-objects 0))

(case
  "dqe14 a RECORD with a heap (list) field dual-used by field-read + equality reclaims clean (named-field access releases its dup)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a #record((= x n) (= y #list(n 5)))) (b #record((= x n) (= y #list(n 5)))))
          (+ (* 1000 (List.len a.y)) (if (= a b) 100000 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 102000 Int64))
  (live-objects 0))

(case
  "dqe15 an Option with a heap (list) payload dual-used by match-extract + equality reclaims clean"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (Option.Some #list(n 5))) (b (Option.Some #list(n 5))))
          (+
            (* 1000 (match a ((Option.Some t) (List.len t)) ((Option.None) -1)))
            (if (= a b) 100000 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 102000 Int64))
  (live-objects 0))

; ── breaker batch 526: leg-2 trigger table (walker-conditioned branch arm-escape). The escapee's
; tree leaked ONCE when it is a non-operand (dqe16), TWICE when it is an operand (dqe10/11); untaken
; escape arms (dqe17) and scalar conditions (dqe18) are clean — the dup is minted on the TAKEN path
; in the shadow of the walker call. dqe19 = leg-1 cross-scope: projection + walker on a RETURNED
; binding. UPDATE: dqe16 (non-operand escapee) and dqe19 (cross-scope projection+walker) now reclaim
; CLEAN — fixed by the landed projected-binder dup-suppression / must-escape reclaim (#6834/#7051): the
; walker's borrow no longer mints an unbalanced keep-alive dup on the escapee/returned binding. dqe10/dqe11
; (the escapee is ALSO a walked OPERAND, so its tree AND the dead sibling's leak = 6) still leak — the
; two-sided end-of-scope drop (drop the dead sibling WITHOUT dropping the escapee) is not yet placed.
(case
  "dqe16 a walker-conditioned branch escaping a NON-operand heap binding leaks the escapee's tree once"
  (input
    (do
      (def
        (f (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n))))
            (b #tuple(n #tuple(n #tuple(n n))))
            (c #tuple(n #tuple(n n))))
          (if (= a b) c #tuple(9 #tuple(9 9)))))
      (def (main (: n Int64)) (let ((r (f n))) (+ (* 1000 (. (. r 1) 1)) (. r 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1001 Int64))
  (live-objects 0))

(case
  "dqe17 a walker-conditioned escape arm left UNTAKEN (operands unequal) reclaims clean — the dup is minted on the taken path only"
  (input
    (do
      (def
        (f (: n Int64))
        (let
          ((a #tuple(n #tuple(n #tuple(n n)))) (b #tuple((+ n 1) #tuple(n #tuple(n n)))))
          (if (= a b) a #tuple(9 #tuple(9 #tuple(9 9))))))
      (def (main (: n Int64)) (let ((r (f n))) (+ (* 1000 (. (. (. r 1) 1) 1)) (. r 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 9009 Int64))
  (live-objects 0))

(case
  "dqe18 a SCALAR-conditioned branch escaping a heap value reclaims clean — leg-2 requires the walker condition"
  (input
    (do
      (def
        (f (: n Int64))
        (let ((a #tuple(n #list(n 5))) (b #tuple((+ n 1) #list(n 6)))) (if (> n 0) a b)))
      (def (main (: n Int64)) (let ((r (f n))) (+ (* 1000 (List.len (. r 1))) (. r 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2001 Int64))
  (live-objects 0))

(case
  "dqe19 leg-1 cross-scope: projection + walker on a RETURNED binding leaks its tree in the caller"
  (input
    (do
      (def
        (f (: n Int64))
        (let ((a #tuple(n #list(n 5))) (b #tuple((+ n 1) #list(n 6)))) (if (> n 0) a b)))
      (def
        (main (: n Int64))
        (let
          ((r (f n)))
          (+ (* 1000 (List.len (. r 1))) (+ (. r 0) (if (= r #tuple(n #list(n 5))) 100000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 102001 Int64))
  (live-objects 0))

; ── breaker batch 546: Float special values as CHAMP keys/elements — the hash/eq AGREEMENT cells.
; Cadenza equality is canonical-byte (= -0.0 0.0) is FALSE (pinned above); these pin that the
; content hash AGREES: a -0.0 key misses a 0.0 probe and hits -0.0; canonical NaN self-contains
; and round-trips as a key. A hash/eq divergence here = silent lookup misses.
(case
  "fk1 a -0.0 Map key misses a 0.0 probe and hits a -0.0 probe (lookup agrees with canonical-byte equality)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m (Map.insert (Map.empty) (if (> n 0) -0.0 1.5) 42)))
          (+
            (* 100 (match (Map.lookup m 0.0) ((Some v) v) ((None u) -1)))
            (match (Map.lookup m -0.0) ((Some v) v) ((None u) -1)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: -58 Int64))
  (live-objects 0))

(case
  "fk2 canonical NaN self-contains as a Set element"
  (input
    (do
      (def
        (main (: n Int64))
        (if (Set.contains (Set.insert #set() (if (> n 0) Float64.nan 1.5)) Float64.nan) 1000 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "fk3 canonical NaN round-trips as a Map key"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Map.lookup (Map.insert (Map.empty) (if (> n 0) Float64.nan 2.5) 5) Float64.nan)
          ((Some v) v)
          ((None u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 5 Int64))
  (live-objects 0))

; ── breaker batch 558: ORDERING walks at depth (eq-at-depth is fenced by itf; order was not).
; od1 = lexicographic < through an immortal 33-trie vs a runtime twin differing at the LAST
; element (both directions + the shorter-prefix rule); od2 = rope order across 50 concat seams
; discriminating at the final char. Single-use walkers — census 0 (no dqe dual-use trip).
(case
  "od1 lexicographic order through an immortal 33-trie discriminates at the last element (both directions + shorter-prefix)"
  (input
    (do
      (def
        (bldx (: i Int64) (: x Int64))
        (if (= i 0) #list() (List.push (bldx (- i 1) x) (if (= i 33) x i))))
      (def
        (main (: n Int64))
        (+
          (if
            (<
              #list(1
                2
                3
                4
                5
                6
                7
                8
                9
                10
                11
                12
                13
                14
                15
                16
                17
                18
                19
                20
                21
                22
                23
                24
                25
                26
                27
                28
                29
                30
                31
                32
                33)
              (bldx 33 (+ n 33)))
            1
            0)
          (+
            (if
              (<
                (bldx 33 (+ n 33))
                #list(1
                  2
                  3
                  4
                  5
                  6
                  7
                  8
                  9
                  10
                  11
                  12
                  13
                  14
                  15
                  16
                  17
                  18
                  19
                  20
                  21
                  22
                  23
                  24
                  25
                  26
                  27
                  28
                  29
                  30
                  31
                  32
                  33))
              10
              0)
            (if
              (<
                #list(1
                  2
                  3
                  4
                  5
                  6
                  7
                  8
                  9
                  10
                  11
                  12
                  13
                  14
                  15
                  16
                  17
                  18
                  19
                  20
                  21
                  22
                  23
                  24
                  25
                  26
                  27
                  28
                  29
                  30
                  31
                  32)
                #list(1
                  2
                  3
                  4
                  5
                  6
                  7
                  8
                  9
                  10
                  11
                  12
                  13
                  14
                  15
                  16
                  17
                  18
                  19
                  20
                  21
                  22
                  23
                  24
                  25
                  26
                  27
                  28
                  29
                  30
                  31
                  32
                  33))
              100
              0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101 Int64))
  (live-objects 0))

(case
  "od2 string order across fifty rope seams discriminates at the final divergent char"
  (input
    (do
      (def (grow (: s String) (: k Int64)) (if (= k 0) s (grow (String.concat s "x") (- k 1))))
      (def
        (main (: n Int64))
        (let
          ((a (grow (if (> n 0) "abc" "z") 50))
            (b (String.concat (grow (if (> n 0) "abc" "z") 49) "y")))
          (+ (if (< a b) 1 0) (if (< b a) 10 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "sy1 a RUNTIME-built symbol (Symbol.of of a computed string) is intern-consistent with the compile-time literal: eq, Map-key hit, Set contains (+ the miss control)"
  (doc
    "The interning identity cell: a symbol built at runtime from a concat-computed string must be
           the SAME key as the #\"foo\" literal everywhere — equality, champ lookup, set membership —
           or runtime-symbol code silently misses literal-keyed tables. The n=0 trial derives \"fox\"
           and must miss on all three. Fixed 1-cell residue (the runtime string), both trials.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((rs (Symbol.of (String.concat "fo" (if (> n 0) "o" "x")))))
          (+
            (if (= rs #"foo") 1 0)
            (+
              (*
                10
                (match
                  (Map.lookup (Map.insert (Map.empty) #"foo" 42) rs)
                  ((Some v) v)
                  ((None u) -1)))
              (if (Set.contains (Set.insert #set() rs) #"foo") 1000 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1421 Int64))
  (call main (: 0 Int64))
  (output (: -10 Int64))
  (live-objects known-leak))

; ── breaker batch 591: the NaN self-equal-but-UNORDERED invariant across =/<=/>= (03-equality's
; "self-equal and unordered downstream" pinned for the RELATIONAL ops specifically), plus the
; ±inf ORDERED contrast. fno1: a runtime NaN is `= ` itself (canonical-byte, TRUE) yet `<=` and
; `>=` itself are FALSE (IEEE partial order) — the intentional split a "make <= consistent with ="
; refactor would silently break. fno2: ±infinity is FULLY ORDERED (inf > finite, -inf < finite,
; -inf < inf, inf <= inf) — the contrast proving only NaN is unordered, not all non-finite.
(case
  "fno1 a runtime NaN is self-EQUAL (canonical-byte) but self-UNORDERED under <= and >= (the intentional =/order split)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((nan (/ (Float64.of-int (- n 1)) 0.0)))
          (+ (if (<= nan nan) 1 0) (+ (if (>= nan nan) 10 0) (if (= nan nan) 100 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100 Int64)))

(case
  "fno2 runtime +/-infinity is FULLY ORDERED (only NaN is unordered, not all non-finite)"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((inf (/ (Float64.of-int n) 0.0)) (ninf (/ (Float64.of-int (- 0 n)) 0.0)))
          (+
            (if (< 1000000.0 inf) 1 0)
            (+ (if (< ninf -1000000.0) 10 0) (+ (if (< ninf inf) 100 0) (if (<= inf inf) 1000 0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1111 Int64)))

; NATIVE #-form compound EQUALITY (M3 native-ast-compound-data). The alias/legacy spellings of compound
; equality are pinned above and throughout; these pin the NATIVE `#word(…)` spelling across every collection
; kind, so `Eq`/`const_compound_eq`'s structural walk reads the native ctor-leaf heads + FieldPair entries
; exactly like the alias. The distinguishing semantics: record/map/set equality is by CONTENTS (key/element
; set), order-INDEPENDENT; a list is ORDERED (element sequence); nesting recurses. (The native #tuple cases
; live above alongside the float-leaf pins.)
(case
  "native #record equality is by field set, order-INDEPENDENT"
  (doc
    "`#record((= x 1) (= y 2))` = `#record((= y 2) (= x 1))` — a record compares by its field→value map, so
        a different field WRITE order is still equal. The native-spelling twin of the alias record-eq pins.")
  (input (do (def (main) (= #record((= x 1) (= y 2)) #record((= y 2) (= x 1)))) (export main)))
  (output (: true Bool)))

(case
  "native #map equality is by entry set, order-INDEPENDENT"
  (doc
    "`#map((= 1 10) (= 2 20))` = `#map((= 2 20) (= 1 10))` — a map compares by its key→value associations,
        independent of insertion/write order.")
  (input (do (def (main) (= #map((= 1 10) (= 2 20)) #map((= 2 20) (= 1 10)))) (export main)))
  (output (: true Bool)))

(case
  "native #set equality is by element set, order-INDEPENDENT"
  (doc
    "`#set(1 2 3)` = `#set(3 2 1)` — a set compares by membership, independent of element order.")
  (input (do (def (main) (= #set(1 2 3) #set(3 2 1))) (export main)))
  (output (: true Bool)))

(case
  "native #list equality is ORDERED — a different element order is NOT equal"
  (doc
    "`#list(1 2)` /= `#list(2 1)` — a list compares by its element SEQUENCE, so reordering breaks equality
        (the contrast to the order-independent set/map/record above).")
  (input (do (def (main) (= #list(1 2) #list(2 1))) (export main)))
  (output (: false Bool)))

(case
  "native nested #record-in-#list equality recurses"
  (doc
    "`#list(#record((= a 1)))` = `#list(#record((= a 1)))` — the structural walk descends the native
        ctor-leaf heads recursively, comparing the inner record by field set.")
  (input (do (def (main) (= #list(#record((= a 1))) #list(#record((= a 1))))) (export main)))
  (output (: true Bool)))

(case
  "native #set of #tuples is order-independent over compound elements"
  (doc
    "`#set(#tuple(1 2) #tuple(3 4))` = `#set(#tuple(3 4) #tuple(1 2))` — a set of compound elements compares
        by membership, and each element by the native #tuple structural walk.")
  (input
    (do (def (main) (= #set(#tuple(1 2) #tuple(3 4)) #set(#tuple(3 4) #tuple(1 2)))) (export main)))
  (output (: true Bool)))
