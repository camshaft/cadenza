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
; FAMILIES OF MEASURE — a DIMENSION groups a FAMILY of interconvertible units. `metre`, `millimetre`,
; and `inch` are three units of the one dimension `length`; each carries an EXACT `Rational` scale to
; the dimension's REFERENCE unit (metre = 1, mm = 1/1000, inch = 127/5000). `(Unit.of #"inch")` names
; such a family unit; the reference is the scale-1 unit. Combining two quantities whose units SHARE a
; dimension is well-formed even when the units DIFFER — each converts to the reference by its exact
; scale and combines there (automatic exact conversion, a principled exception to no-silent-promotion:
; the conversion is exact and canonical, and a mix of DIMENSIONS is still CDZ0501). A PREFIX applies an
; exact scale: SI decimal (kilo 10³, milli 10⁻³ = 1/1000) and IEC binary (kibi 2¹⁰, mebi 2²⁰). Exact
; mixing NEEDS exact magnitudes — `1 inch + 1 mm` is exact only over `Rational` — the deep tie to the
; numeric model. The `(Unit.base …)` cases above are the degenerate one-unit-per-dimension case; the
; `(Unit.of …)`/`(Unit.prefix …)` cases below add the family/scale/prefix layer over it.
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
  (input  (Qty.of 5.0 (Unit.base #"metre")))
  (output (: (Qty.of 5.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "Qty.value recovers the underlying numeric value, discarding the unit"
  (doc    "`(Qty.value (Qty.of 5.0 (Unit.base #\"metre\")))` = 5.0 : Float64 — the explicit exit from
           the dimensional layer (the widening that requires no check, verification-layers.md #Refinement
           Coercions Are Checked). The unit leaves the value only through this explicit call, never
           implicitly; the recovered value is the ordinary numeric it always was underneath.")
  (input  (Qty.value (Qty.of 5.0 (Unit.base #"metre"))))
  (output (: 5.0 Float64)))

(case "a dimensionless quantity carries the group identity Unit.one"
  (doc    "`(Qty.of 3.0 Unit.one)` is a dimensionless quantity — `Unit.one` is the identity of the unit
           group. Its erased value is 3.0, but its static type `(Qty Float64 Unit.one)` is DISTINCT from
           the bare `Float64`: crossing between them is explicit (`Qty.of` in, `Qty.value` out), never an
           implicit coercion, exactly as the numeric core never silently promotes between numeric types.")
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
  (input  (+ (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: (Qty.of 5.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "adding quantities of incompatible dimension is a compile-time error"
  (doc    "`(+ (Qty.of 1.0 metre) (Qty.of 1.0 second))` combines a length with a time — incompatible
           dimensions — so the compiler rejects it at COMPILE TIME with CDZ0501 (units-of-measure.md
           #Dimensional Mismatch Is An Error). There is no runtime trap: units are erased before the
           program runs, so a dimensional inconsistency can only be a compile-time event. THE core case
           the whole layer exists for — a length is never added to a time.")
  (input  (+ (Qty.of 1.0 (Unit.base #"metre")) (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "subtracting quantities of incompatible dimension is a compile-time error"
  (doc    "The subtraction companion: `(- (Qty.of 5.0 metre) (Qty.of 2.0 second))` is the same
           dimensional-mismatch rejection (CDZ0501) as addition — `-` requires equal dimensions exactly
           as `+` does. Pins that the obligation is on the operator class, not just on `+`.")
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
  (input  (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: (Qty.of 6.0 (Unit.^ (Unit.base #"metre") 2)) (Qty Float64 (Unit.^ (Unit.base #"metre") 2)))))

(case "dividing quantities divides their dimensions"
  (doc    "`(/ (Qty.of 6.0 metre) (Qty.of 2.0 second))` derives metre/second — a velocity — with value
           3.0. The classic derived unit falls out of the group quotient rather than needing to be
           enumerated. The underlying Float64 division runs unchanged on the erased values.")
  (input  (/ (Qty.of 6.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"second"))))
  (output (: (Qty.of 3.0 (Unit./ (Unit.base #"metre") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"metre") (Unit.base #"second"))))))

(case "scaling a quantity by a dimensionless quantity keeps its dimension"
  (doc    "`(* (Qty.of 2.0 metre) (Qty.of 3.0 Unit.one))` multiplies a length by a dimensionless scalar:
           metre·one = metre, value 6.0. Pins that `Unit.one` is the group identity — multiplying by it
           leaves the dimension unchanged — so scaling by a constant does not change a quantity's
           dimension.")
  (input  (* (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 Unit.one)))
  (output (: (Qty.of 6.0 (Unit.base #"metre")) (Qty Float64 (Unit.base #"metre")))))

(case "a unit multiplied by its own inverse cancels to the dimensionless unit"
  (doc    "`(/ (Qty.of 6.0 metre) (Qty.of 2.0 metre))` derives metre/metre = Unit.one — the base cancels
           its inverse (the free-abelian-group law) — leaving a dimensionless `(Qty Float64 Unit.one)`
           with value 3.0. Pins that dimensional composition CANCELS: a ratio of like quantities is
           dimensionless, decided by the exponent map going to all-zero, not by syntax.")
  (input  (/ (Qty.of 6.0 (Unit.base #"metre")) (Qty.of 2.0 (Unit.base #"metre"))))
  (output (: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))))

; ============================================================================================
; Comparison — same dimension required (the ordering/equality obligation)
; ============================================================================================

(case "comparing two quantities of the same dimension yields a Bool"
  (doc    "`(< (Qty.of 2.0 metre) (Qty.of 3.0 metre))` compares two lengths and is true — comparison
           requires EQUAL dimensions (you can order two lengths) and yields a bare Bool. The underlying
           Float64 comparison runs unchanged on the erased values.")
  (input  (< (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"metre"))))
  (output (: true Bool)))

(case "comparing quantities of incompatible dimension is a compile-time error"
  (doc    "`(< (Qty.of 2.0 metre) (Qty.of 3.0 second))` orders a length against a time — incompatible
           dimensions — so the compiler rejects it (CDZ0501): comparison, like `+`/`-`, requires equal
           dimensions (units-of-measure.md #Dimensional Mismatch Is An Error). You cannot ask whether a
           length is less than a time.")
  (input  (< (Qty.of 2.0 (Unit.base #"metre")) (Qty.of 3.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "equality across incompatible dimensions is a compile-time error"
  (doc    "`(= (Qty.of 1.0 metre) (Qty.of 1.0 second))` compares a length to a time for equality —
           incompatible dimensions — rejected with CDZ0501, not silently false. A dimensional mismatch
           is a compile error even under `=`, because the operands cannot inhabit one dimension; there is
           no dimension at which a length equals a time.")
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
  (input  (= (Qty.value (Qty.of 5.0 (Unit.base #"metre"))) 5.0))
  (output (: true Bool)))

(case "the underlying numeric type obeys the numeric core — no silent promotion under a unit"
  (doc    "`(+ (Qty.of 2 (Unit.base #\"metre\")) (Qty.of 3.0 (Unit.base #\"metre\")))` adds an Int64
           quantity to a Float64 quantity — SAME dimension (metre) but DIFFERENT underlying numeric type
           — rejected with the numeric no-promotion diagnostic CDZ0301, exactly as bare `(+ 2 3.0)` is
           (numeric-model.md #Numeric Types Do Not Silently Promote). Pins that the unit layer sits OVER
           the numeric core and does not relax it: the dimensions agree, but the numeric types must too.")
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
  (input  (do
            (def (speed d t) (/ d t))
            (def (main) (speed (Qty.of 100.0 (Unit.base #"metre")) (Qty.of 4.0 (Unit.base #"second")))) (export main)))
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
  (input  (+ (Qty.of (Rational.of 1 2) (Unit.base #"feet"))
             (Qty.of (Rational.of 1 2) (Unit.base #"second"))))
  (error  CDZ0501))

(case "a rational-magnitude quantity erases to its exact rational value"
  (doc    "`(Qty.value (Qty.of (Rational.of 1 3) feet))` = 1/3 : Rational. The unit erases (checked then
           discarded), the EXACT rational magnitude remains — the erased value of `(Qty Rational u)` is
           exactly the underlying Rational, unchanged. Pins that units erase before emission while
           rationals do not: dimension is a compile-time layer, exactness is a runtime value.")
  (input  (Qty.value (Qty.of (Rational.of 1 3) (Unit.base #"feet"))))
  (output (: 1/3 Rational)))

; ============================================================================================
; Families of measure — many interconvertible units per dimension; mixing converts exactly
; ============================================================================================
; A DIMENSION (the group element, e.g. `length`) groups a FAMILY of units — metre, millimetre, inch —
; each carrying an EXACT `Rational` scale to the dimension's REFERENCE unit (metre = 1, mm = 1/1000,
; inch = 127/5000). `(Unit.of #"inch")` names such a family unit (declared in the prelude under its
; dimension with its scale); the reference unit is the scale-1 unit, so `(Unit.base #"metre")` above is
; the length reference. Combining two quantities whose units SHARE A DIMENSION is well-formed even when
; the units DIFFER: each operand converts to the reference unit by its exact scale and combines there
; (units-of-measure.md #Combining Units Of One Dimension Is Well-Formed). This is automatic exact
; conversion — a principled exception to no-silent-promotion, defensible because the conversion is EXACT
; and CANONICAL (one right answer, the value at the reference unit) unlike a lossy Int/Float promotion,
; and because a mix of DIMENSIONS is still CDZ0501. Exact mixing needs exact magnitudes: `1 inch + 1 mm`
; is exact only over `Rational` (options/numeric-model/), the deep tie between the two layers.

(case "converting a unit to another unit of the same dimension is an exact rational scale"
  (doc    "`(Unit.in (Unit.of #\"metre\") (Qty.of (Rational.of 1 1) (Unit.of #\"inch\")))` converts 1 inch
           to metres by its exact scale 127/5000 — an exact `Rational` conversion, not an approximation
           (units-of-measure.md #A Unit Carries An Exact Scale To Its Dimension's Reference). The result
           is `(Qty Rational metre)` = 127/5000 m. Pins that a within-dimension conversion is the exact
           scale the family declares.")
  (input  (Unit.in (Unit.of #"metre") (Qty.of (Rational.of 1 1) (Unit.of #"inch"))))
  (output (: (Qty.of (Rational.of 127 5000) (Unit.of #"metre")) (Qty Rational (Unit.of #"metre")))))

(case "adding two units of one dimension converts to the reference unit exactly"
  (doc    "THE mixing case: `(+ (Qty 1 inch) (Qty 1 mm))` over exact rational magnitudes — both are
           dimension `length`, so each converts to the reference `metre` by its exact scale (1 inch =
           127/5000 m, 1 mm = 1/1000 m) and they add there: 127/5000 + 1/1000 = 127/5000 + 5/5000 =
           132/5000 = 33/1250 m. The result is `(Qty Rational metre)` — the common reference unit, a
           deterministic function of the operand units (units-of-measure.md #Combining Units Of One
           Dimension Is Well-Formed). Exact because the magnitudes are `Rational`.")
  (input  (+ (Qty.of (Rational.of 1 1) (Unit.of #"inch")) (Qty.of (Rational.of 1 1) (Unit.of #"millimetre"))))
  (output (: (Qty.of (Rational.of 33 1250) (Unit.of #"metre")) (Qty Rational (Unit.of #"metre")))))

(case "mixing units of DIFFERENT dimensions is still a compile-time error"
  (doc    "`(+ (Qty 1 inch) (Qty 1 second))` mixes `length` and `time` — DIFFERENT dimensions — so it is
           CDZ0501, unchanged. Automatic conversion applies WITHIN a dimension (inch↔mm), never ACROSS
           one (inch↔second): there is no scale relating a length to a time. Pins that the mixing
           relaxation does not weaken the dimensional safety the layer exists for.")
  (input  (+ (Qty.of (Rational.of 1 1) (Unit.of #"inch")) (Qty.of (Rational.of 1 1) (Unit.of #"second"))))
  (error  CDZ0501))

(case "comparing two units of one dimension converts before comparing"
  (doc    "`(< (Qty 25 mm) (Qty 1 inch))` compares a length in mm to a length in inches — same dimension,
           different units — so each converts to the reference and compares there: 25 mm = 25/1000 =
           1/40 m, 1 inch = 127/5000 m; 1/40 = 125/5000 < 127/5000, so it is true. Pins that comparison,
           like `+`/`-`, converts differing units of one dimension rather than rejecting them.")
  (input  (< (Qty.of (Rational.of 25 1) (Unit.of #"millimetre")) (Qty.of (Rational.of 1 1) (Unit.of #"inch"))))
  (output (: true Bool)))

; ============================================================================================
; Prefixes — SI decimal (powers of ten) and IEC binary (powers of two) as exact scales
; ============================================================================================
; A PREFIX applies an exact scale to a unit, producing another unit of the SAME dimension
; (units-of-measure.md #A Scaled Unit Is A Unit Scaled By An Exact Factor). SI decimal prefixes are
; powers of ten (kilo 10³, milli 10⁻³ = 1/1000 — an exact Rational); IEC binary prefixes are powers of
; two (kibi 2¹⁰ = 1024, mebi 2²⁰). `kilobyte` (1000 byte) and `kibibyte` (1024 byte) are DISTINCT units
; of one dimension with distinct exact scales — never silently equated.

(case "an SI decimal prefix scales a unit by a power of ten"
  (doc    "`(Unit.in (Unit.of #\"metre\") (Qty.of (Rational.of 3 1) (Unit.prefix kilo (Unit.of #\"metre\"))))`
           converts 3 km to metres: `(Unit.prefix kilo metre)` has scale 1000·1 = 1000, so 3 km = 3000 m.
           Pins that a prefixed unit is a unit of the same dimension differing by the exact prefix
           factor.")
  (input  (Unit.in (Unit.of #"metre") (Qty.of (Rational.of 3 1) (Unit.prefix kilo (Unit.of #"metre")))))
  (output (: (Qty.of (Rational.of 3000 1) (Unit.of #"metre")) (Qty Rational (Unit.of #"metre")))))

(case "a negative-power SI prefix is an exact rational scale"
  (doc    "`(Unit.in (Unit.of #\"second\") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #\"second\"))))`
           converts 5 ms to seconds: `milli` = 10⁻³ = 1/1000, so 5 ms = 5/1000 = 1/200 s. Pins that
           negative-power prefixes are exact `Rational` scales — the second reason exact rationals are
           load-bearing for units (a milli/micro/nano factor has no exact float or integer form).")
  (input  (Unit.in (Unit.of #"second") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #"second")))))
  (output (: (Qty.of (Rational.of 1 200) (Unit.of #"second")) (Qty Rational (Unit.of #"second")))))

(case "an IEC binary prefix scales a unit by a power of two"
  (doc    "`(Unit.in (Unit.of #\"byte\") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #\"byte\"))))`
           converts 1 MiB to bytes: `mebi` = 2²⁰ = 1048576, so 1 MiB = 1048576 byte. Pins the binary
           prefix family (kibi/mebi/gibi) alongside the decimal one — distinct scales for `information`.")
  (input  (Unit.in (Unit.of #"byte") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #"byte")))))
  (output (: (Qty.of (Rational.of 1048576 1) (Unit.of #"byte")) (Qty Rational (Unit.of #"byte")))))

(case "a decimal kilobyte and a binary kibibyte are distinct units of one dimension"
  (doc    "`(+ (Qty 1 KiB) (Qty 1 kB))` over the `information` dimension: kibibyte = 1024 byte and
           kilobyte = 1000 byte are DISTINCT units with distinct exact scales, so mixing them converts to
           the reference `byte` and sums to 1024 + 1000 = 2024 byte — NOT 2000, never silently equated.
           Pins that the two prefix systems are genuinely different scales the arithmetic keeps distinct
           (the classic KiB-vs-kB conflation is caught, not hidden).")
  (input  (+ (Qty.of (Rational.of 1 1) (Unit.prefix kibi (Unit.of #"byte")))
             (Qty.of (Rational.of 1 1) (Unit.prefix kilo (Unit.of #"byte")))))
  (output (: (Qty.of (Rational.of 2024 1) (Unit.of #"byte")) (Qty Rational (Unit.of #"byte")))))

; ============================================================================================
; Prefix conversion over CONCRETE numerics — the bignum-free realization (Float rounds, Int
; exact/truncates). A unit's SCALE is compile-time metadata (a machine-integer ratio: kilo = 1000/1,
; kibi = 1024/1), and a mixed-unit combine converts each operand to the dimension's reference by
; `value * num / den` in the quantity's OWN inner numeric type — losing precision "only where the
; underlying numeric type is itself inexact" (units-of-measure.md #A Unit Carries An Exact Scale To Its
; Dimension's Reference). So the family/prefix machinery needs NO arbitrary-precision Rational: over
; Float64 or Int64 it works today. (The `(Rational.of …)` cases above pin the EXACT-magnitude form,
; realized when Rational lands; these pin the SAME conversions over the numerics the seed already has.)

(case "a prefixed unit combines with its base by converting to the reference (Float)"
  (doc    "`(+ (Qty.of 1.0 (Unit.prefix kilo metre)) (Qty.of 500.0 metre))` mixes kilometres and metres —
           one dimension, two scales — so each converts to the reference `metre` (1 km = 1000 m) and they
           add: 1000 + 500 = 1500 m. Over Float64 the scale multiply is ordinary float arithmetic; the
           result is a `(Qty Float64 metre)` at the reference unit.")
  (input  (Qty.value (+ (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"metre")))
                        (Qty.of 500.0 (Unit.base #"metre")))))
  (output (: 1500.0 Float64)))

(case "a decimal kilobyte and a binary kibibyte convert distinctly over Int64"
  (doc    "`(+ (Qty 1 KiB) (Qty 1 kB))` over Int64: kibibyte = 1024 byte and kilobyte = 1000 byte are
           DISTINCT units with distinct exact scales, so each converts to the reference `byte` and sums to
           1024 + 1000 = 2024 byte — NOT 2000, never silently equated (the classic KiB-vs-kB conflation
           caught). The conversion is exact integer arithmetic (both scales are whole).")
  (input  (Qty.value (+ (Qty.of 1 (Unit.prefix kibi (Unit.base #"byte")))
                        (Qty.of 1 (Unit.prefix kilo (Unit.base #"byte"))))))
  (output (: 2024 Int64)))

(case "comparing quantities of one dimension at different scales converts before comparing"
  (doc    "`(< (Qty 500.0 metre) (Qty 1.0 (Unit.prefix kilo metre)))` compares metres to kilometres — one
           dimension, two scales — so each converts to the reference and compares there: 500 m < 1000 m,
           so it is true. Comparison, like `+`/`-`, converts differing units of one dimension rather than
           rejecting them.")
  (input  (< (Qty.of 500.0 (Unit.base #"metre"))
             (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"metre")))))
  (output (: true Bool)))

(case "mixing a prefixed unit across DIFFERENT dimensions is still a compile-time error"
  (doc    "`(+ (Qty 1.0 (Unit.prefix kilo metre)) (Qty 1.0 second))` mixes `length` and `time` — different
           dimensions — so it is CDZ0501, exactly as the unprefixed case is. A prefix scales WITHIN a
           dimension; it never bridges two, so the dimensional safety the layer exists for is untouched.")
  (input  (+ (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"metre")))
             (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Named FAMILY units over concrete numerics — `Unit.of` consults the family registry (a name → its
; reference dimension + exact machine-integer scale), so `inch`/`foot`/`kilometre` are units of one
; dimension that auto-convert exactly as prefixed units do. The `(Rational.of …)` family cases above
; pin the exact-magnitude form (realized when Rational lands); these pin the same conversions over the
; numerics the seed already has (Float rounds, Int is exact/truncates — spec §A Unit Carries An Exact
; Scale). The family vocabulary is prelude DATA, not a privileged in-compiler list.

(case "adding two named family units of one dimension converts to the reference (Float)"
  (doc    "`(+ (Qty.of 1.0 (Unit.of #\"inch\")) (Qty.of 1.0 (Unit.of #\"millimetre\")))` — inch and
           millimetre are named units of `length` (inch = 127/5000 m, mm = 1/1000 m) — each converts to
           the reference `metre` and adds: 127/5000 + 1/1000 = 33/1250 = 0.0264 m. Over Float64 the exact
           scales apply as ordinary float arithmetic.")
  (input  (Qty.value (+ (Qty.of 1.0 (Unit.of #"inch"))
                        (Qty.of 1.0 (Unit.of #"millimetre")))))
  (output (: 0.0264 Float64)))

(case "a named family unit combines across DIFFERENT dimensions is a compile-time error"
  (doc    "`(+ (Qty.of 1.0 (Unit.of #\"inch\")) (Qty.of 1.0 (Unit.of #\"second\")))` mixes `length` and
           `time` — different dimensions — so it is CDZ0501, exactly as the base-unit case is. A family
           unit names a measure of ONE dimension; combining across dimensions is rejected regardless of
           whether the units are base or named family units.")
  (input  (+ (Qty.of 1.0 (Unit.of #"inch")) (Qty.of 1.0 (Unit.of #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Unit.in — EXPLICIT conversion to a chosen unit over concrete numerics. `(Unit.in TARGET q)` converts
; q's magnitude from its unit to TARGET (result `(Qty T TARGET)`), the way a program pins a specific
; result unit rather than the auto-chosen reference (units-of-measure.md #A Unit Conversion Is The
; Arithmetic The Source Denotes). The `(Rational.of …)` Unit.in cases above pin the exact-magnitude form
; (realized when Rational lands); these pin the same conversions over Float/Int.

(case "Unit.in converts a quantity to a chosen larger unit (Float)"
  (doc    "`(Unit.in metre (Qty.of 3.0 kilometre))` converts 3 km to metres: 3 * 1000 = 3000 m. The
           magnitude is multiplied by the source-to-target scale ratio (km's 1000 over metre's 1) in the
           inner Float64 type; the result is `(Qty Float64 metre)`.")
  (input  (Qty.value (Unit.in (Unit.of #"metre") (Qty.of 3.0 (Unit.of #"kilometre")))))
  (output (: 3000.0 Float64)))

(case "Unit.in converts a quantity to a chosen smaller unit exactly (Int)"
  (doc    "`(Unit.in kilometre (Qty.of 2000 metre))` converts 2000 m to kilometres: 2000 / 1000 = 2 km,
           exact integer arithmetic (the ratio divides). Pins that Unit.in over Int64 is exact when the
           conversion is whole; a non-dividing ratio truncates (opting into integer math).")
  (input  (Qty.value (Unit.in (Unit.of #"kilometre") (Qty.of 2000 (Unit.of #"metre")))))
  (output (: 2 Int64)))

(case "Unit.in to a unit of a different dimension is a compile-time error"
  (doc    "`(Unit.in metre (Qty.of 3.0 second))` asks to convert a time to a length — different
           dimensions — so it is CDZ0501. Unit.in converts WITHIN a dimension (metre↔km), never ACROSS
           one; there is no scale relating a length to a time.")
  (input  (Unit.in (Unit.of #"metre") (Qty.of 3.0 (Unit.of #"second"))))
  (error  CDZ0501))

; ============================================================================================
; RUNTIME mixed-unit conversion — the scale multiply reaches the emitted component only when a magnitude
; is a RUNTIME value (units-of-measure.md #A Unit Conversion Is The Arithmetic The Source Denotes). A
; quantity built from a runtime parameter does not fold; the compiler emits `value * num / den` as real
; arithmetic in the inner type. These pass a runtime argument via `(call main …)`.

(case "a runtime mixed-unit sum emits the scale conversion (Int)"
  (doc    "`(+ (Qty.of v kilometre) (Qty.of 500 metre))` with `v` a runtime Int64 parameter: km converts
           to the reference metre by *1000 emitted at run time, so v=1 → 1000 + 500 = 1500 m. Pins that
           the conversion works on a NON-constant magnitude, not only a compile-time literal.")
  (needs  units-of-measure)
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of v (Unit.prefix kilo (Unit.base #"metre")))
                            (Qty.of 500 (Unit.base #"metre")))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1500 Int64)))

(case "a runtime Unit.in conversion emits the scale multiply (Int)"
  (doc    "`(Unit.in metre (Qty.of v kilometre))` with `v` a runtime Int64: converts v km to metres by
           *1000 at run time, so v=3 → 3000 m. The explicit-conversion companion of the runtime mixed
           sum.")
  (needs  units-of-measure)
  (input  (do
            (def (main (: v Int64))
              (Qty.value (Unit.in (Unit.of #"metre") (Qty.of v (Unit.of #"kilometre")))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 3000 Int64)))
