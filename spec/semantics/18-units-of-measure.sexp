; Units of measure — the optional, compile-time-only dimensional-analysis layer over the numeric
; core, witnessing the units-of-measure decision (options/units-of-measure/). A QUANTITY is a value
; of the type `(Qty T u)`: an underlying numeric value of type `T` (any type the numeric model
; admits) carried with a COMPILE-TIME unit `u`. `(Qty.of x u)` attaches unit `u` to `x`; `(Qty.value
; q)` recovers the underlying numeric value, discarding the unit. The point of the layer is that
; combining incompatible dimensions is a COMPILE-TIME error and the whole apparatus is ERASED before
; emission — a length is never added to a time, a velocity is length/time, and none of it costs a byte
; at runtime (units-of-measure.md; the one piece of earlier Cadenza's identity that survives the clean
; room).
;
; UNITS FORM A FREE ABELIAN GROUP over named base dimensions (options/units-of-measure/):
;   Unit.one              — the dimensionless unit (the group identity)
;   (Unit.base #"metre")  — a base dimension named by a Symbol (options/symbol-interning/)
;   (Unit.* a b)          — the product of two units
;   (Unit./ a b)          — the quotient of two units
;   (Unit.^ u n)          — a unit raised to a compile-time INTEGER power n (may be negative)
; Because the group is abelian and free, two units are the SAME DIMENSION exactly when every base
; dimension appears to the same integer exponent — order does not matter and a base cancels its
; inverse. Dimensional equality is decided by comparing canonical exponent maps, a pure compile-time
; computation with no solver. `(Qty T u)` is an ordinary type-constructor application, exactly as
; `(Int N)` is the integer constructor applied to a compile-time width — a quantity type is the same
; shape indexed by a compile-time UNIT instead of a compile-time natural.
;
; THE LOAD-BEARING CONSTRAINT — units are CHECKED THEN ERASED. Adding a unit to a numeric value MUST
; NOT change the value's numeric byte form or its runtime behavior (units-of-measure.md #Dimensional
; Analysis Does Not Alter The Numeric Core): `(Qty.of 5.0 metre)` and the bare `5.0` are BYTE-IDENTICAL
; in the emitted component, differing only in the erased static type these cases record in `(: … T)`.
; No unit, base name, or exponent ever appears in the emitted component (units-of-measure.md #Dimensions
; Are Checked Then Erased). A dimensional mismatch is therefore ALWAYS a compile-time rejection
; (CDZ0501), NEVER a runtime trap — units are gone before the program runs. This is the refinement-
; erases-to-its-base-type discipline (verification-layers.md) applied to dimensions, so a component
; derived from well-dimensioned source with the capability included is byte-identical to one derived
; with it excluded — dimensional discharge does not change emitted bytes.
;
; Tagged `(needs units-of-measure)`: dimensional analysis is an OPTIONAL verification layer a later
; generation realizes (units-of-measure.md #This Capability Is Optional; it is not on the ignition
; path — the seed clears ignition with the numeric core alone; options/realized-capability-set/). The
; seed does not realize it, so its behavior gate SKIPS these cases — they pin the contract the
; realization must meet, they are not seed declines.

; ============================================================================================
; Construction and observation — Qty.of attaches a unit; Qty.value recovers the numeric value
; ============================================================================================

(case "a quantity is constructed from a numeric value and a unit"
  (doc    "`(Qty.of 5.0 (Unit.base #\"metre\"))` attaches the base dimension `metre` to the Float64
           value 5.0, producing a `(Qty Float64 metre)`. The unit is a COMPILE-TIME value; the recorded
           type documents the erased static type — the emitted value is just the Float64 5.0.")
  (needs  units-of-measure)
  (input  (Qty.of 5.0 (Unit.base #"metre")))
  (output (: (Qty.of 5.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "Qty.value recovers the underlying numeric value, discarding the unit"
  (doc    "`(Qty.value (Qty.of 5.0 (Unit.base #\"metre\")))` = 5.0 : Float64 — the explicit exit from
           the dimensional layer (the widening that requires no check, verification-layers.md #Refinement
           Coercions Are Checked). The unit leaves the value only through this explicit call, never
           implicitly; the recovered value is the ordinary numeric it always was underneath.")
  (needs  units-of-measure)
  (input  (Qty.value (Qty.of 5.0 (Unit.base #"metre"))))
  (output (: 5.0 Float64)))

(case "a dimensionless quantity carries the group identity Unit.one"
  (doc    "`(Qty.of 3.0 Unit.one)` is a dimensionless quantity — `Unit.one` is the identity of the unit
           group. Its erased value is 3.0, but its static type `(Qty Float64 Unit.one)` is DISTINCT from
           the bare `Float64`: crossing between them is explicit (`Qty.of` in, `Qty.value` out), never an
           implicit coercion, exactly as the numeric core never silently promotes between numeric types.")
  (needs  units-of-measure)
  (input  (Qty.of 3.0 Unit.one))
  (output (: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))))

; ============================================================================================
; Addition and subtraction — same dimension required, dimension preserved
; ============================================================================================

(case "adding two quantities of the same dimension keeps that dimension"
  (doc    "`(+ (Qty.of 2.0 metre) (Qty.of 3.0 metre))` = a `(Qty Float64 metre)` with value 5.0. The
           underlying Float64 addition runs unchanged on the erased values; the unit layer adds one
           obligation — the two dimensions must be EQUAL — and contributes nothing to the emitted
           arithmetic (units-of-measure.md #Dimensional Analysis Does Not Alter The Numeric Core).")
  (needs  units-of-measure)
  (input  (+ (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: (Qty.of 5.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "adding quantities of incompatible dimension is a compile-time error"
  (doc    "`(+ (Qty.of 1.0 metre) (Qty.of 1.0 second))` combines a length with a time — incompatible
           dimensions — so the compiler rejects it at COMPILE TIME with CDZ0501 (units-of-measure.md
           #Dimensional Mismatch Is An Error). There is no runtime trap: units are erased before the
           program runs, so a dimensional inconsistency can only be a compile-time event. THE core case
           the whole layer exists for — a length is never added to a time.")
  (needs  units-of-measure)
  (input  (+ (Qty.of 1.0 (Unit.base #"metre")) (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "subtracting quantities of incompatible dimension is a compile-time error"
  (doc    "The subtraction companion: `(- (Qty.of 5.0 metre) (Qty.of 2.0 second))` is the same
           dimensional-mismatch rejection (CDZ0501) as addition — `-` requires equal dimensions exactly
           as `+` does. Pins that the obligation is on the operator class, not just on `+`.")
  (needs  units-of-measure)
  (input  (- (Qty.of 5.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Multiplication and division — dimensions compose by the group operation
; ============================================================================================

(case "multiplying quantities multiplies their dimensions"
  (doc    "`(* (Qty.of 2.0 metre) (Qty.of 3.0 metre))` derives the dimension metre·metre = metre² and
           has value 6.0 — an area (units-of-measure.md #Dimensional Mismatch Is An Error: an operation
           that derives a dimension MUST produce the dimension its rule defines). Multiplication never
           requires equal dimensions; it composes them by the group product.")
  (needs  units-of-measure)
  (input  (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: (Qty.of 6.0 (Unit.^ (Unit.base #"metre") 2)) (Qty Float64 (Unit.^ (Unit.base #"metre") 2)))))

(case "dividing quantities divides their dimensions"
  (doc    "`(/ (Qty.of 6.0 metre) (Qty.of 2.0 second))` derives metre/second — a velocity — with value
           3.0. The classic derived unit falls out of the group quotient rather than needing to be
           enumerated. The underlying Float64 division runs unchanged on the erased values.")
  (needs  units-of-measure)
  (input  (/ (Qty.of 6.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"second"))))
  (output (: (Qty.of 3.0 (Unit./ (Unit.base #"metre") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"metre") (Unit.base #"second"))))))

(case "scaling a quantity by a dimensionless quantity keeps its dimension"
  (doc    "`(* (Qty.of 2.0 metre) (Qty.of 3.0 Unit.one))` multiplies a length by a dimensionless scalar:
           metre·one = metre, value 6.0. Pins that `Unit.one` is the group identity — multiplying by it
           leaves the dimension unchanged — so scaling by a constant does not change a quantity's
           dimension.")
  (needs  units-of-measure)
  (input  (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 Unit.one)))
  (output (: (Qty.of 6.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "a unit multiplied by its own inverse cancels to the dimensionless unit"
  (doc    "`(/ (Qty.of 6.0 metre) (Qty.of 2.0 metre))` derives metre/metre = Unit.one — the base cancels
           its inverse (the free-abelian-group law) — leaving a dimensionless `(Qty Float64 Unit.one)`
           with value 3.0. Pins that dimensional composition CANCELS: a ratio of like quantities is
           dimensionless, decided by the exponent map going to all-zero, not by syntax.")
  (needs  units-of-measure)
  (input  (/ (Qty.of 6.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"metre"))))
  (output (: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))))

; ============================================================================================
; Comparison — same dimension required (the ordering/equality obligation)
; ============================================================================================

(case "comparing two quantities of the same dimension yields a Bool"
  (doc    "`(< (Qty.of 2.0 metre) (Qty.of 3.0 metre))` compares two lengths and is true — comparison
           requires EQUAL dimensions (you can order two lengths) and yields a bare Bool. The underlying
           Float64 comparison runs unchanged on the erased values.")
  (needs  units-of-measure)
  (input  (< (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: true Bool)))

(case "comparing quantities of incompatible dimension is a compile-time error"
  (doc    "`(< (Qty.of 2.0 metre) (Qty.of 3.0 second))` orders a length against a time — incompatible
           dimensions — so the compiler rejects it (CDZ0501): comparison, like `+`/`-`, requires equal
           dimensions (units-of-measure.md #Dimensional Mismatch Is An Error). You cannot ask whether a
           length is less than a time.")
  (needs  units-of-measure)
  (input  (< (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "equality across incompatible dimensions is a compile-time error"
  (doc    "`(= (Qty.of 1.0 metre) (Qty.of 1.0 second))` compares a length to a time for equality —
           incompatible dimensions — rejected with CDZ0501, not silently false. A dimensional mismatch
           is a compile error even under `=`, because the operands cannot inhabit one dimension; there is
           no dimension at which a length equals a time.")
  (needs  units-of-measure)
  (input  (= (Qty.of 1.0 (Unit.base #"metre")) (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Dimensional equality is by canonical form, not syntax — differently-written equal dimensions
; ============================================================================================
; Two units are the same dimension exactly when their canonical exponent maps agree; the written form
; is irrelevant. `(Unit.* m m)` and `(Unit.^ m 2)` are one dimension, so an operation that derives one
; and an annotation written as the other agree.

(case "dimensional equality is decided by canonical exponent map, not written form"
  (doc    "`(+ (* (Qty.of 2.0 metre) (Qty.of 2.0 metre)) (Qty.of 1.0 (Unit.^ metre 2)))` adds an area
           written as metre·metre to one written as metre² — the SAME dimension by canonical exponent
           map ({metre: 2}) — so the addition is well-dimensioned and yields metre² with value 5.0. Pins
           that dimensional equality compares canonical forms, not syntax: metre·metre = metre².")
  (needs  units-of-measure)
  (input  (+ (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"metre")))
             (Qty.of 1.0 (Unit.^ (Unit.base #"metre") 2))))
  (output (: (Qty.of 5.0 (Unit.^ (Unit.base #"metre") 2)) (Qty Float64 (Unit.^ (Unit.base #"metre") 2)))))

; ============================================================================================
; Annotation — a dimensional annotation must match the derived dimension (CDZ0501)
; ============================================================================================

(case "annotating a quantity at a dimension the expression does not derive is an error"
  (doc    "`(: (* (Qty.of 2.0 metre) (Qty.of 3.0 metre)) (Qty Float64 metre))` annotates a product whose
           derived dimension is metre² at the dimension metre — a dimensional conflict — rejected with
           CDZ0501 (the dimensional specialization of the annotation-conflicts rejection; CDZ0203 names
           the general case, CDZ0501 names it when the conflict is dimensional). An annotation constrains
           but never contradicts the derived dimension.")
  (needs  units-of-measure)
  (input  (: (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre")))
             (Qty Float64 (Unit.base #"metre"))))
  (error  CDZ0501))

; ============================================================================================
; Erasure — a quantity is byte-identical to its underlying numeric (the numeric core is untouched)
; ============================================================================================
; The whole apparatus is erased before emission: `(Qty T u)` erases to `T`, so a quantity's recovered
; value is the identical numeric value form the bare literal has, and adding a unit changes no numeric
; byte form (units-of-measure.md #Dimensional Analysis Does Not Alter The Numeric Core). These pin that
; the layer is a compile-time check with zero runtime footprint.

(case "a quantity's erased value is the identical numeric value the bare literal has"
  (doc    "`(= (Qty.value (Qty.of 5.0 metre)) 5.0)` is true: the value recovered from a quantity is the
           SAME Float64 value form as the bare literal 5.0, because `(Qty Float64 metre)` erases to
           Float64 with no change to the numeric byte form. The comparison is between two bare Float64
           values (the quantity's dimension was discarded by Qty.value), so it is an ordinary numeric
           equality, not a dimensional one.")
  (needs  units-of-measure)
  (input  (= (Qty.value (Qty.of 5.0 (Unit.base #"metre"))) 5.0))
  (output (: true Bool)))

(case "the underlying numeric type obeys the numeric core — no silent promotion under a unit"
  (doc    "`(+ (Qty.of 2 (Unit.base #\"metre\")) (Qty.of 3.0 (Unit.base #\"metre\")))` adds an Int64
           quantity to a Float64 quantity — SAME dimension (metre) but DIFFERENT underlying numeric type
           — rejected with the numeric no-promotion diagnostic CDZ0301, exactly as bare `(+ 2 3.0)` is
           (numeric-model.md #Numeric Types Do Not Silently Promote). Pins that the unit layer sits OVER
           the numeric core and does not relax it: the dimensions agree, but the numeric types must too.")
  (needs  units-of-measure)
  (input  (+ (Qty.of 2 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (error  CDZ0301))

; ============================================================================================
; The payoff — a runtime quantity carried through a function (dimensions checked, then erased)
; ============================================================================================
; Dimensional checking is over the static types, so a quantity flowing through a function parameter is
; checked at the definition and erased at emission: the compiled `speed` is plain Float64 division. This
; pins that the layer works on runtime-carried quantities, not only compile-time constants, and that the
; derived-dimension rule holds through a call.

(case "a function deriving a velocity from a distance and a time"
  (doc    "`speed` takes a distance `(Qty Float64 metre)` and a time `(Qty Float64 second)` and returns
           their quotient — a `(Qty Float64 metre/second)`; called with 100 metre and 4 second it yields
           25.0 metre/second. The dimensions are checked at compile time and erased, so the emitted
           `speed` is plain Float64 division; the quantity types are the compile-time contract, not a
           runtime representation.")
  (needs  units-of-measure)
  (input  (module m
            (def (speed d t) (/ d t))
            (def (main) (speed (Qty.of 100.0 (Unit.base #"metre")) (Qty.of 4.0 (Unit.base #"second"))))))
  (output (: (Qty.of 25.0 (Unit./ (Unit.base #"metre") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"metre") (Unit.base #"second"))))))

; ============================================================================================
; Units compose with exact rationals — (Qty Rational u) is both dimensioned AND exact
; ============================================================================================
; The dimensional layer is GENERIC over the underlying numeric type T (a quantity is `(Qty T u)`), and
; `Rational` (options/numeric-model/) is one admissible T. The two layers compose ORTHOGONALLY — units
; track the DIMENSION, rationals track the EXACTNESS OF THE MAGNITUDE — so `(Qty Rational u)` is a
; quantity that is both dimensioned and exact. Dividing two such quantities divides the MAGNITUDES by
; exact rational division (no float rounding) AND the DIMENSIONS by the unit-group quotient in one
; operation. This is the payoff of keeping units generic over T rather than baking a fixed numeric type
; into the quantity: `feet / seconds` over exact magnitudes is dimensionally checked and exact at once.

(case "a quantity over an exact rational magnitude divides exactly and derives its dimension"
  (doc    "`(/ (Qty.of (Rational.of 1 3) feet) (Qty.of (Rational.of 1 2) second))` divides an exact
           rational distance by an exact rational time: the MAGNITUDE is (1/3)/(1/2) = 2/3 by exact
           rational division (no float rounding), and the DIMENSION is feet/second by the unit-group
           quotient — both in one operation. The result is `(Qty Rational feet/second)` with the exact
           magnitude 2/3. THE `feet / seconds` case: dimensioned and exact together.")
  (needs  units-of-measure)
  (input  (/ (Qty.of (Rational.of 1 3) (Unit.base #"feet"))
             (Qty.of (Rational.of 1 2) (Unit.base #"second"))))
  (output (: (Qty.of (Rational.of 2 3) (Unit./ (Unit.base #"feet") (Unit.base #"second")))
             (Qty Rational (Unit./ (Unit.base #"feet") (Unit.base #"second"))))))

(case "adding rational-magnitude quantities of incompatible dimension is still a compile-time error"
  (doc    "`(+ (Qty.of (Rational.of 1 2) feet) (Qty.of (Rational.of 1 2) second))` adds an exact-rational
           length to an exact-rational time — incompatible DIMENSIONS — rejected with CDZ0501 exactly as
           the Float64 case is. Pins that the dimensional check is over the unit, INDEPENDENT of the
           underlying numeric type T: choosing exact rational magnitudes does not relax the dimensional
           obligation.")
  (needs  units-of-measure)
  (input  (+ (Qty.of (Rational.of 1 2) (Unit.base #"feet"))
             (Qty.of (Rational.of 1 2) (Unit.base #"second"))))
  (error  CDZ0501))

(case "a rational-magnitude quantity erases to its exact rational value"
  (doc    "`(Qty.value (Qty.of (Rational.of 1 3) feet))` = 1/3 : Rational. The unit erases (checked then
           discarded), the EXACT rational magnitude remains — the erased value of `(Qty Rational u)` is
           exactly the underlying Rational, unchanged. Pins that units erase before emission while
           rationals do not: dimension is a compile-time layer, exactness is a runtime value.")
  (needs  units-of-measure)
  (input  (Qty.value (Qty.of (Rational.of 1 3) (Unit.base #"feet"))))
  (output (: 1/3 Rational)))
