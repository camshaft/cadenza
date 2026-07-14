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
;   (Unit.base #"meter")  — a base dimension named by a Symbol (options/symbol-interning/)
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
; FAMILIES OF MEASURE — a DIMENSION groups a FAMILY of interconvertible units. `meter`, `millimeter`,
; and `inch` are three units of the one dimension `length`; each carries an EXACT `Rational` scale to
; the dimension's REFERENCE unit (meter = 1, mm = 1/1000, inch = 127/5000). `(Unit.of #"inch")` names
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
; Analysis Does Not Alter The Numeric Core): `(Qty.of 5.0 meter)` and the bare `5.0` are BYTE-IDENTICAL
; in the emitted component, differing only in the erased static type these cases record in `(: … T)`.
; No unit, base name, or exponent ever appears in the emitted component (units-of-measure.md #Dimensions
; Are Checked Then Erased). A dimensional mismatch is therefore ALWAYS a compile-time rejection
; (CDZ0501), NEVER a runtime trap — units are gone before the program runs. This is the refinement-
; erases-to-its-base-type discipline (verification-layers.md) applied to dimensions, so a component
; derived from well-dimensioned source with the capability included is byte-identical to one derived
; with it excluded — dimensional discharge does not change emitted bytes.
;
; Dimensional analysis is an OPTIONAL verification layer
; (units-of-measure.md #This Capability Is Optional; not on the ignition path — the seed clears ignition
; with the numeric core alone; options/realized-capability-set/). The behavior gate grades EVERY case by
; what the compiler DOES — a generation that has not realized units declines the case. The dimensional
; CORE is now REALIZED over the numeric types the compiler has: construction/
; observation/erasure, `+`/`-`/`*`/`/`/comparison with dimensions composing, CDZ0501 on incompatible
; dimensions, named families + SI/IEC prefixes (`Unit.of`/`Unit.prefix`), automatic and explicit
; (`Unit.in`) conversion — all over `Int`/`Float` magnitudes (a conversion "los[es] precision only where
; the underlying numeric type is itself inexact", #A Unit Carries An Exact Scale To Its Dimension's
; Reference: exact over `Int`, rounding over `Float`), constant AND runtime. The cases still tagged that
; DECLINE are exactly the ones whose magnitude is a `Rational` (`(Rational.of …)`) — EXACT-magnitude
; mixing needs the exact-rational numeric type, which a later increment realizes; they pin the contract
; that realization must meet, they are not miscompiles.

; ============================================================================================
; Construction and observation — Qty.of attaches a unit; Qty.value recovers the numeric value
; ============================================================================================

(case "a quantity is constructed from a numeric value and a unit"
  (doc    "`(Qty.of 5.0 (Unit.base #\"meter\"))` attaches the base dimension `meter` to the Float64
           value 5.0, producing a `(Qty Float64 meter)`. The unit is a COMPILE-TIME value; the recorded
           type documents the erased static type — the emitted value is just the Float64 5.0.")
  (input  (Qty.of 5.0 (Unit.base #"meter")))
  (output (: (Qty.of 5.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "Qty.value recovers the underlying numeric value, discarding the unit"
  (doc    "`(Qty.value (Qty.of 5.0 (Unit.base #\"meter\")))` = 5.0 : Float64 — the explicit exit from
           the dimensional layer (the widening that requires no check, verification-layers.md #Refinement
           Coercions Are Checked). The unit leaves the value only through this explicit call, never
           implicitly; the recovered value is the ordinary numeric it always was underneath.")
  (input  (Qty.value (Qty.of 5.0 (Unit.base #"meter"))))
  (output (: 5.0 Float64)))

(case "a dimensionless quantity carries the group identity Unit.one"
  (doc    "`(Qty.of 3.0 Unit.one)` is a dimensionless quantity — `Unit.one` is the identity of the unit
           group. Its erased value is 3.0, but its static type `(Qty Float64 Unit.one)` is DISTINCT from
           the bare `Float64`: crossing between them is explicit (`Qty.of` in, `Qty.value` out), never an
           implicit coercion, exactly as the numeric core never silently promotes between numeric types.")
  (input  (Qty.of 3.0 Unit.one))
  (output (: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))))

; Unit extraction — `(Qty.unit q)` recovers a quantity's UNIT as a first-class compile-time unit value,
; the inverse of `Qty.of`'s unit argument. It lets a program construct another quantity in the SAME unit
; as an existing one — `(Qty.of new (Qty.unit y))` — without re-spelling the unit expression. The unit is
; a compile-time value (like `(Unit.base …)`), erased before emission, so `Qty.unit` is used in a unit
; position, never as a runtime value; the reconstructed quantity is dimensionally checked in full.

(case "Qty.unit recovers a quantity's unit to build another quantity of the same unit"
  (doc    "`(Qty.of 9.0 (Qty.unit y))` where `y` is a `(Qty Float64 meter)` builds a NEW quantity in the
           same unit — meter — as `y`, without naming meter again. `Qty.unit` reads the unit off `y`'s
           solved type and yields it as a unit value, the inverse of the unit `Qty.of` attaches; the
           result is `(Qty Float64 meter)` with value 9.0. The `make another quantity of the same unit as
           this one` idiom, composing at the value level.")
  (input  (let ((y (Qty.of 3.0 (Unit.base #"meter"))))
            (Qty.of 9.0 (Qty.unit y))))
  (output (: (Qty.of 9.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "Qty.unit recovers a derived unit"
  (doc    "`(Qty.unit (speed))` where `speed` derives meter/second recovers the DERIVED unit — the whole
           free-abelian-group value, not just a base — so `(Qty.of 10.0 (Qty.unit (speed)))` is a
           `(Qty Float64 (meter/second))` with value 10.0. Pins that extraction carries a composed unit
           (a quotient, here a velocity), not only an atomic base.")
  (input  (do
            (def (speed)
              (/ (Qty.of 6.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"second"))))
            (def (main) (Qty.value (Qty.of 10.0 (Qty.unit (speed)))))
            (export main)))
  (output (: 10.0 Float64)))

(case "a quantity built from an extracted unit is dimensionally checked"
  (doc    "The unit `Qty.unit` yields is a REAL unit, checked like any other: `(+ (Qty.of 1.0 (Qty.unit
           y)) (Qty.of 2.0 second))` where `y` is a length adds a length (the extracted meter) to a time —
           incompatible dimensions — so it is CDZ0501, exactly as writing `meter` explicitly would be.
           Extraction is transparent to the dimensional check; it reuses a unit, it does not escape the
           checking.")
  (input  (let ((y (Qty.of 3.0 (Unit.base #"meter"))))
            (+ (Qty.of 1.0 (Qty.unit y)) (Qty.of 2.0 (Unit.base #"second")))))
  (error  CDZ0501))

(case "Qty.unit recovers the unit of a quantity whose magnitude is a runtime value"
  (doc    "`(Qty.unit (Qty.of n meter))` with `n` a boundary parameter recovers the unit — meter — from the
           quantity's TYPE, which is a compile-time concern independent of the runtime magnitude `n`. Building
           a new quantity `(Qty.of 1 <that unit>)` and adding it to another meter succeeds and yields 5
           (1 + 4), for every `n`. Pins that unit extraction reads the type (not the value), so it works over
           a runtime-magnitude quantity, and the recovered unit is a REAL meter that mixes with an explicit
           meter — the value-level `same unit as this one` idiom on the runtime path.")
  (input  (do (def (main (: n Int64))
                (Qty.value (+ (Qty.of 1 (Qty.unit (Qty.of n (Unit.base #"meter")))) (Qty.of 4 (Unit.base #"meter"))))) (export main)))
  (call   main (: 3 Int64)) (output (: 5 Int64))
  (call   main (: 100 Int64)) (output (: 5 Int64)))

(case "the unit recovered from a runtime-magnitude quantity is dimensionally checked"
  (doc    "The recovered unit is checked like any other even when read off a runtime-magnitude quantity:
           `(+ (Qty.of 1 (Qty.unit (Qty.of n meter))) (Qty.of 2 second))` adds the extracted meter to a
           second — incompatible dimensions — so it is CDZ0501, regardless of `n`. Pins that `Qty.unit` over
           a runtime magnitude still yields a real dimension the type checker enforces at compile time (the
           runtime companion of the constant dimensional-check case above).")
  (input  (do (def (main (: n Int64))
                (Qty.value (+ (Qty.of 1 (Qty.unit (Qty.of n (Unit.base #"meter")))) (Qty.of 2 (Unit.base #"second"))))) (export main)))
  (call   main (: 3 Int64)) (error CDZ0501))

; ============================================================================================
; Addition and subtraction — same dimension required, dimension preserved
; ============================================================================================

(case "adding two quantities of the same dimension keeps that dimension"
  (doc    "`(+ (Qty.of 2.0 meter) (Qty.of 3.0 meter))` = a `(Qty Float64 meter)` with value 5.0. The
           underlying Float64 addition runs unchanged on the erased values; the unit layer adds one
           obligation — the two dimensions must be EQUAL — and contributes nothing to the emitted
           arithmetic (units-of-measure.md #Dimensional Analysis Does Not Alter The Numeric Core).")
  (input  (+ (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter"))))
  (output (: (Qty.of 5.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

; The same-dimension add/sub above fold at compile time (both magnitudes constant). A RUNTIME magnitude —
; a boundary parameter — cannot fold: the underlying integer add/sub runs as an emitted instruction, with
; the unit still a compile-time-only obligation contributing nothing to the arithmetic. These pin that a
; same-UNIT sum/difference (no scale conversion — contrast the mixed-unit runtime sum later in this file,
; whose *1000 conversion IS emitted) reads the parameter's magnitude and adds it, `Qty.value` recovering
; the erased result. The unit layer is erased before run time, so only the numeric core's op is emitted.

(case "a runtime-magnitude same-unit sum adds the erased magnitudes"
  (doc    "`(+ (Qty.of n meter) (Qty.of 5 meter))` with `n` a boundary Int64 parameter: both operands are
           meters (equal dimension, no conversion), so the unit layer adds nothing and the erased `n + 5`
           runs as a plain integer add — 3+5 = 8, 100+5 = 105. Pins that a same-unit sum emits the numeric
           core's add on a runtime magnitude, distinct from the mixed-unit runtime sum (which additionally
           emits a scale conversion). `Qty.value` recovers the erased sum.")
  (input  (do (def (main (: n Int64))
                (Qty.value (+ (Qty.of n (Unit.base #"meter")) (Qty.of 5 (Unit.base #"meter"))))) (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64))
  (call   main (: 100 Int64)) (output (: 105 Int64)))

(case "a runtime-magnitude same-unit difference subtracts the erased magnitudes"
  (doc    "The subtraction companion: `(- (Qty.of n meter) (Qty.of 5 meter))` emits the erased `n - 5` as a
           plain integer subtract (the checked Int64 subtract of the numeric core), 20-5 = 15. Pins that `-`
           over equal dimensions runs the numeric core's subtract on a runtime magnitude, contributing no
           dimensional arithmetic — the operator-class obligation is compile-time only.")
  (input  (do (def (main (: n Int64))
                (Qty.value (- (Qty.of n (Unit.base #"meter")) (Qty.of 5 (Unit.base #"meter"))))) (export main)))
  (call   main (: 20 Int64)) (output (: 15 Int64))
  (call   main (: 5 Int64)) (output (: 0 Int64)))

(case "adding quantities of incompatible dimension is a compile-time error"
  (doc    "`(+ (Qty.of 1.0 meter) (Qty.of 1.0 second))` combines a length with a time — incompatible
           dimensions — so the compiler rejects it at COMPILE TIME with CDZ0501 (units-of-measure.md
           #Dimensional Mismatch Is An Error). There is no runtime trap: units are erased before the
           program runs, so a dimensional inconsistency can only be a compile-time event. THE core case
           the whole layer exists for — a length is never added to a time.")
  (input  (+ (Qty.of 1.0 (Unit.base #"meter")) (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "subtracting quantities of incompatible dimension is a compile-time error"
  (doc    "The subtraction companion: `(- (Qty.of 5.0 meter) (Qty.of 2.0 second))` is the same
           dimensional-mismatch rejection (CDZ0501) as addition — `-` requires equal dimensions exactly
           as `+` does. Pins that the obligation is on the operator class, not just on `+`.")
  (input  (- (Qty.of 5.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Multiplication and division — dimensions compose by the group operation
; ============================================================================================

(case "multiplying quantities multiplies their dimensions"
  (doc    "`(* (Qty.of 2.0 meter) (Qty.of 3.0 meter))` derives the dimension meter·meter = meter² and
           has value 6.0 — an area (units-of-measure.md #Dimensional Mismatch Is An Error: an operation
           that derives a dimension MUST produce the dimension its rule defines). Multiplication never
           requires equal dimensions; it composes them by the group product.")
  (input  (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter"))))
  (output (: (Qty.of 6.0 (Unit.^ (Unit.base #"meter") 2)) (Qty Float64 (Unit.^ (Unit.base #"meter") 2)))))

(case "dividing quantities divides their dimensions"
  (doc    "`(/ (Qty.of 6.0 meter) (Qty.of 2.0 second))` derives meter/second — a velocity — with value
           3.0. The classic derived unit falls out of the group quotient rather than needing to be
           enumerated. The underlying Float64 division runs unchanged on the erased values.")
  (input  (/ (Qty.of 6.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"second"))))
  (output (: (Qty.of 3.0 (Unit./ (Unit.base #"meter") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"meter") (Unit.base #"second"))))))

; The product/quotient above fold (constant magnitudes). A RUNTIME magnitude cannot fold: the erased
; multiply/divide is emitted, while the DIMENSION composes at compile time (meter·second, meter/second).
; These pin runtime `*`/`/` on a quantity — the magnitude arithmetic runs, and the derived dimension is
; still tracked statically (the round-trip case reads back through a cancelling divide, and the mismatch
; case shows the composed dimension is enforced against a later add). The unit is erased at run time.

(case "a runtime-magnitude product multiplies the erased magnitudes"
  (doc    "`(* (Qty.of n meter) (Qty.of 2 second))` with `n` a runtime Int64: the dimension composes to
           meter·second at compile time, and the erased `n * 2` runs as a plain integer multiply — 3·2 = 6,
           7·2 = 14. Pins that a runtime product emits the numeric core's multiply on the magnitude while
           the dimension is derived statically (contributing nothing to the emitted arithmetic).")
  (input  (do (def (main (: n Int64))
                (Qty.value (* (Qty.of n (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second"))))) (export main)))
  (call   main (: 3 Int64)) (output (: 6 Int64))
  (call   main (: 7 Int64)) (output (: 14 Int64)))

(case "a runtime-magnitude quotient divides the erased magnitudes"
  (doc    "`(/ (Qty.of n meter) (Qty.of 2 second))` derives the velocity dimension meter/second and emits
           the erased integer `n / 2` (the checked Int64 division of the numeric core) — 6/2 = 3, 10/2 = 5.
           The runtime companion of the constant velocity, exercising the emitted divide on a runtime
           magnitude while the derived dimension is a compile-time concern.")
  (input  (do (def (main (: n Int64))
                (Qty.value (/ (Qty.of n (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second"))))) (export main)))
  (call   main (: 6 Int64)) (output (: 3 Int64))
  (call   main (: 10 Int64)) (output (: 5 Int64)))

(case "a runtime product's derived dimension cancels correctly through a divide"
  (doc    "`(/ (* (Qty.of n meter) (Qty.of 3 second)) (Qty.of 3 second))` composes meter·second then
           divides by second, cancelling to meter — the derived dimension arithmetic (`m·s / s = m`) is
           tracked through the runtime ops at compile time, and the erased magnitude is `n·3 / 3 = n` (4 →
           4). Pins that the composed dimension of a RUNTIME product is a correct group element, not lost or
           mis-derived — the divide's dimension quotient sees the product's meter·second.")
  (input  (do (def (main (: n Int64))
                (Qty.value (/ (* (Qty.of n (Unit.base #"meter")) (Qty.of 3 (Unit.base #"second")))
                              (Qty.of 3 (Unit.base #"second"))))) (export main)))
  (call   main (: 4 Int64)) (output (: 4 Int64))
  (call   main (: 30 Int64)) (output (: 30 Int64)))

(case "a runtime product's derived dimension is enforced against an incompatible add"
  (doc    "The composed dimension of a runtime product is CHECKED like any other: `(+ (* (Qty.of n meter)
           (Qty.of 2 second)) (Qty.of 1 meter))` adds a meter·second (an area-like product) to a plain
           meter — incompatible dimensions — so the compiler rejects it (CDZ0501) even though the product's
           magnitude is a runtime parameter. Pins that a runtime `*` derives a real dimension the type
           checker enforces, not an erased-to-anything magnitude — the dimension mismatch is a compile-time
           event regardless of the runtime value.")
  (input  (do (def (main (: n Int64))
                (Qty.value (+ (* (Qty.of n (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second")))
                              (Qty.of 1 (Unit.base #"meter"))))) (export main)))
  (call   main (: 3 Int64)) (error CDZ0501))

(case "scaling a quantity by a dimensionless quantity keeps its dimension"
  (doc    "`(* (Qty.of 2.0 meter) (Qty.of 3.0 Unit.one))` multiplies a length by a dimensionless scalar:
           meter·one = meter, value 6.0. Pins that `Unit.one` is the group identity — multiplying by it
           leaves the dimension unchanged — so scaling by a constant does not change a quantity's
           dimension.")
  (input  (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 Unit.one)))
  (output (: (Qty.of 6.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "a unit multiplied by its own inverse cancels to the dimensionless unit"
  (doc    "`(/ (Qty.of 6.0 meter) (Qty.of 2.0 meter))` derives meter/meter = Unit.one — the base cancels
           its inverse (the free-abelian-group law) — leaving a dimensionless `(Qty Float64 Unit.one)`
           with value 3.0. Pins that dimensional composition CANCELS: a ratio of like quantities is
           dimensionless, decided by the exponent map going to all-zero, not by syntax.")
  (input  (/ (Qty.of 6.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter"))))
  (output (: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))))

; ============================================================================================
; Powers — Qty.pow raises a quantity to a compile-time NON-NEGATIVE integer power, composing the
; unit exactly as `Unit.^` does (the exponent map + scale are raised to that power) and the erased
; magnitude by repeated multiply. `(Qty.pow q n)` is the surface companion of the `*`-derived power
; (meter·meter = meter²): `(Qty.pow q 2)` and `(* q q)` derive the SAME dimension. The exponent is a
; compile-time integer read off the second argument (not an HM variable), like `Unit.^`'s power.
; ============================================================================================

(case "raising a quantity to a compile-time power composes the unit and the magnitude"
  (doc    "`(Qty.pow (Qty.of 3.0 meter) 2)` squares a length: the unit is raised to the 2nd power
           (meter²) exactly as `Unit.^` composes it, and the erased Float64 magnitude is 3·3 = 9.0. The
           surface companion of the `*`-derived area (units-of-measure.md #Dimensional Mismatch Is An
           Error: the operation produces the dimension its rule defines), so `Qty.pow` and repeated
           multiplication agree.")
  (input  (Qty.pow (Qty.of 3.0 (Unit.base #"meter")) 2))
  (output (: (Qty.of 9.0 (Unit.^ (Unit.base #"meter") 2)) (Qty Float64 (Unit.^ (Unit.base #"meter") 2)))))

(case "the power form derives the same dimension as repeated multiplication"
  (doc    "`(= (Qty.pow (Qty.of 2.0 meter) 2) (* (Qty.of 2.0 meter) (Qty.of 2.0 meter)))` is true: raising
           to the 2nd power and multiplying twice derive the SAME dimension (meter²) AND the same value
           (4.0), so the equality is well-dimensioned and holds. Pins that `Qty.pow n` is definitionally
           the n-fold product — the unit exponents compose identically, decided by the canonical map.")
  (input  (= (Qty.pow (Qty.of 2.0 (Unit.base #"meter")) 2)
             (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter")))))
  (output (: true Bool)))

(case "a quantity raised to the zeroth power is a dimensionless one"
  (doc    "`(Qty.pow (Qty.of 5.0 meter) 0)` is the empty product: the unit's exponents are all scaled to
           zero (Unit.one, the group identity) and the magnitude is the multiplicative identity 1.0. Pins
           that the zeroth power is dimensionless — meter⁰ = one — matching the free-abelian-group law
           that a zero exponent drops from the map.")
  (input  (Qty.value (Qty.pow (Qty.of 5.0 (Unit.base #"meter")) 0)))
  (output (: 1.0 Float64)))

(case "the power form cubes an integer-magnitude quantity exactly"
  (doc    "`(Qty.pow (Qty.of 2 meter) 3)` over Int64: the unit is meter³ and the erased magnitude is
           2·2·2 = 8 by exact integer multiplication. Pins that the power works over the integer numeric
           the seed has (the repeated multiply is the inner type's own `*`), not only over Float.")
  (input  (Qty.value (Qty.pow (Qty.of 2 (Unit.base #"meter")) 3)))
  (output (: 8 Int64)))

(case "a runtime-magnitude quantity raised to a power emits the repeated multiply"
  (doc    "`(Qty.pow (Qty.of x meter) 2)` with `x` a runtime Float64: the power can't be folded, so it
           emits x·x at run time, so x=3.0 → 9.0 m². The runtime companion of the constant square — the
           unit is a compile-time concern (meter²), only the magnitude's multiply is emitted.")
  (input  (do
            (def (main (: x Float64))
              (Qty.value (Qty.pow (Qty.of x (Unit.base #"meter")) 2)))
            (export main)))
  (call   main (: 3.0 Float64))
  (output (: 9.0 Float64)))

(case "a negative power is the reciprocal, deriving an inverse unit"
  (doc    "`(Qty.pow (Qty.of 2.0 second) -1)` raises a time to the -1 power: the unit is second⁻¹ = a
           frequency (the exponent map's entry is negated, which `Unit.^ -1` and `Unit./ Unit.one` denote
           identically), and the erased magnitude is the reciprocal 1/2 = 0.5. A negative power composes
           the inverse dimension exactly as `(/ (Qty.of 1.0 Unit.one) q)` would — the free-abelian-group
           inverse.")
  (input  (Qty.pow (Qty.of 2.0 (Unit.base #"second")) -1))
  (output (: (Qty.of 0.5 (Unit./ Unit.one (Unit.base #"second")))
             (Qty Float64 (Unit./ Unit.one (Unit.base #"second"))))))

(case "the negative power agrees with dividing into the dimensionless one"
  (doc    "`(= (Qty.pow (Qty.of 2.0 second) -1) (/ (Qty.of 1.0 Unit.one) (Qty.of 2.0 second)))` is true:
           raising to the -1 power and dividing one by the quantity derive the SAME inverse dimension
           (second⁻¹) AND the same value (0.5), so the equality is well-dimensioned and holds. Pins that
           `Qty.pow q -1` is definitionally the reciprocal — the group inverse — not a special case.")
  (input  (= (Qty.pow (Qty.of 2.0 (Unit.base #"second")) -1)
             (/ (Qty.of 1.0 Unit.one) (Qty.of 2.0 (Unit.base #"second")))))
  (output (: true Bool)))

(case "a negative power over an integer magnitude truncates the reciprocal"
  (doc    "`(Qty.pow (Qty.of 2 second) -1)` over Int64: the unit is second⁻¹ and the reciprocal 1/2 is
           computed by INTEGER division, which truncates toward zero to 0 — the documented precision loss
           `only where the underlying numeric type is itself inexact` (here Int64 division truncates,
           units-of-measure.md #A Unit Carries An Exact Scale). The dimension is exact regardless; only
           the integer magnitude truncates, exactly as `(/ 1 2)` does outside the units layer.")
  (input  (Qty.value (Qty.pow (Qty.of 2 (Unit.base #"second")) -1)))
  (output (: 0 Int64)))

(case "a runtime-magnitude quantity raised to a negative power emits the reciprocal"
  (doc    "`(Qty.pow (Qty.of x second) -1)` with `x` a runtime Float64: the reciprocal can't be folded, so
           it emits 1/x at run time, so x=4.0 → 0.25 s⁻¹. The runtime companion of the constant reciprocal
           — the inverse unit is a compile-time concern (second⁻¹), only the magnitude's division is
           emitted.")
  (input  (do
            (def (main (: x Float64))
              (Qty.value (Qty.pow (Qty.of x (Unit.base #"second")) -1)))
            (export main)))
  (call   main (: 4.0 Float64))
  (output (: 0.25 Float64)))

; ============================================================================================
; Comparison — same dimension required (the ordering/equality obligation)
; ============================================================================================

(case "comparing two quantities of the same dimension yields a Bool"
  (doc    "`(< (Qty.of 2.0 meter) (Qty.of 3.0 meter))` compares two lengths and is true — comparison
           requires EQUAL dimensions (you can order two lengths) and yields a bare Bool. The underlying
           Float64 comparison runs unchanged on the erased values.")
  (input  (< (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter"))))
  (output (: true Bool)))

(case "comparing quantities of incompatible dimension is a compile-time error"
  (doc    "`(< (Qty.of 2.0 meter) (Qty.of 3.0 second))` orders a length against a time — incompatible
           dimensions — so the compiler rejects it (CDZ0501): comparison, like `+`/`-`, requires equal
           dimensions (units-of-measure.md #Dimensional Mismatch Is An Error). You cannot ask whether a
           length is less than a time.")
  (input  (< (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"second"))))
  (error  CDZ0501))

(case "equality across incompatible dimensions is a compile-time error"
  (doc    "`(= (Qty.of 1.0 meter) (Qty.of 1.0 second))` compares a length to a time for equality —
           incompatible dimensions — rejected with CDZ0501, not silently false. A dimensional mismatch
           is a compile error even under `=`, because the operands cannot inhabit one dimension; there is
           no dimension at which a length equals a time.")
  (input  (= (Qty.of 1.0 (Unit.base #"meter")) (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Dimensional equality is by canonical form, not syntax — differently-written equal dimensions
; ============================================================================================
; Two units are the same dimension exactly when their canonical exponent maps agree; the written form
; is irrelevant. `(Unit.* m m)` and `(Unit.^ m 2)` are one dimension, so an operation that derives one
; and an annotation written as the other agree.

(case "dimensional equality is decided by canonical exponent map, not written form"
  (doc    "`(+ (* (Qty.of 2.0 meter) (Qty.of 2.0 meter)) (Qty.of 1.0 (Unit.^ meter 2)))` adds an area
           written as meter·meter to one written as meter² — the SAME dimension by canonical exponent
           map ({meter: 2}) — so the addition is well-dimensioned and yields meter² with value 5.0. Pins
           that dimensional equality compares canonical forms, not syntax: meter·meter = meter².")
  (input  (+ (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter")))
             (Qty.of 1.0 (^ (Unit.base #"meter") 2))))
  (output (: (Qty.of 5.0 (Unit.^ (Unit.base #"meter") 2)) (Qty Float64 (Unit.^ (Unit.base #"meter") 2)))))

; ============================================================================================
; Annotation — a dimensional annotation must match the derived dimension (CDZ0501)
; ============================================================================================

(case "annotating a quantity at a dimension the expression does not derive is an error"
  (doc    "`(: (* (Qty.of 2.0 meter) (Qty.of 3.0 meter)) (Qty Float64 meter))` annotates a product whose
           derived dimension is meter² at the dimension meter — a dimensional conflict — rejected with
           CDZ0501 (the dimensional specialization of the annotation-conflicts rejection; CDZ0203 names
           the general case, CDZ0501 names it when the conflict is dimensional). An annotation constrains
           but never contradicts the derived dimension.")
  (input  (: (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))
             (Qty Float64 (Unit.base #"meter"))))
  (error  CDZ0501))

; ============================================================================================
; Erasure — a quantity is byte-identical to its underlying numeric (the numeric core is untouched)
; ============================================================================================
; The whole apparatus is erased before emission: `(Qty T u)` erases to `T`, so a quantity's recovered
; value is the identical numeric value form the bare literal has, and adding a unit changes no numeric
; byte form (units-of-measure.md #Dimensional Analysis Does Not Alter The Numeric Core). These pin that
; the layer is a compile-time check with zero runtime footprint.

(case "a quantity's erased value is the identical numeric value the bare literal has"
  (doc    "`(= (Qty.value (Qty.of 5.0 meter)) 5.0)` is true: the value recovered from a quantity is the
           SAME Float64 value form as the bare literal 5.0, because `(Qty Float64 meter)` erases to
           Float64 with no change to the numeric byte form. The comparison is between two bare Float64
           values (the quantity's dimension was discarded by Qty.value), so it is an ordinary numeric
           equality, not a dimensional one.")
  (input  (= (Qty.value (Qty.of 5.0 (Unit.base #"meter"))) 5.0))
  (output (: true Bool)))

(case "the underlying numeric type obeys the numeric core — no silent promotion under a unit"
  (doc    "`(+ (Qty.of 2 (Unit.base #\"meter\")) (Qty.of 3.0 (Unit.base #\"meter\")))` adds an Int64
           quantity to a Float64 quantity — SAME dimension (meter) but DIFFERENT underlying numeric type
           — rejected with the numeric no-promotion diagnostic CDZ0301, exactly as bare `(+ 2 3.0)` is
           (numeric-model.md #Numeric Types Do Not Silently Promote). Pins that the unit layer sits OVER
           the numeric core and does not relax it: the dimensions agree, but the numeric types must too.")
  (input  (+ (Qty.of 2 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter"))))
  (error  CDZ0301))

; ============================================================================================
; The payoff — a runtime quantity carried through a function (dimensions checked, then erased)
; ============================================================================================
; Dimensional checking is over the static types, so a quantity flowing through a function parameter is
; checked at the definition and erased at emission: the compiled `speed` is plain Float64 division. This
; pins that the layer works on runtime-carried quantities, not only compile-time constants, and that the
; derived-dimension rule holds through a call.

(case "a function deriving a velocity from a distance and a time"
  (doc    "`speed` takes a distance `(Qty Float64 meter)` and a time `(Qty Float64 second)` and returns
           their quotient — a `(Qty Float64 meter/second)`; called with 100 meter and 4 second it yields
           25.0 meter/second. The dimensions are checked at compile time and erased, so the emitted
           `speed` is plain Float64 division; the quantity types are the compile-time contract, not a
           runtime representation.")
  (input  (do
            (def (speed d t) (/ d t))
            (def (main) (speed (Qty.of 100.0 (Unit.base #"meter")) (Qty.of 4.0 (Unit.base #"second")))) (export main)))
  (output (: (Qty.of 25.0 (Unit./ (Unit.base #"meter") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"meter") (Unit.base #"second"))))))

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
  (output (: (Qty.of 2/3 (Unit./ (Unit.base #"feet") (Unit.base #"second")))
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
; A DIMENSION (the group element, e.g. `length`) groups a FAMILY of units — meter, millimeter, inch —
; each carrying an EXACT `Rational` scale to the dimension's REFERENCE unit (meter = 1, mm = 1/1000,
; inch = 127/5000). `(Unit.of #"inch")` names such a family unit (declared in the prelude under its
; dimension with its scale); the reference unit is the scale-1 unit, so `(Unit.base #"meter")` above is
; the length reference. Combining two quantities whose units SHARE A DIMENSION is well-formed even when
; the units DIFFER: each operand converts to the reference unit by its exact scale and combines there
; (units-of-measure.md #Combining Units Of One Dimension Is Well-Formed). This is automatic exact
; conversion — a principled exception to no-silent-promotion, defensible because the conversion is EXACT
; and CANONICAL (one right answer, the value at the reference unit) unlike a lossy Int/Float promotion,
; and because a mix of DIMENSIONS is still CDZ0501. Exact mixing needs exact magnitudes: `1 inch + 1 mm`
; is exact only over `Rational` (options/numeric-model/), the deep tie between the two layers.

(case "converting a unit to another unit of the same dimension is an exact rational scale"
  (doc    "`(Unit.in (Unit.of #\"meter\") (Qty.of (Rational.of 1 1) (Unit.of #\"inch\")))` converts 1 inch
           to meters by its exact scale 127/5000 — an exact `Rational` conversion, not an approximation
           (units-of-measure.md #A Unit Carries An Exact Scale To Its Dimension's Reference). The result
           is `(Qty Rational meter)` = 127/5000 m. Pins that a within-dimension conversion is the exact
           scale the family declares.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of (Rational.of 1 1) (Unit.of #"inch"))))
  (output (: (Qty.of 127/5000 (Unit.base #"meter")) (Qty Rational (Unit.base #"meter")))))

(case "adding two units of one dimension converts to the reference unit exactly"
  (doc    "THE mixing case: `(+ (Qty 1 inch) (Qty 1 mm))` over exact rational magnitudes — both are
           dimension `length`, so each converts to the reference `meter` by its exact scale (1 inch =
           127/5000 m, 1 mm = 1/1000 m) and they add there: 127/5000 + 1/1000 = 127/5000 + 5/5000 =
           132/5000 = 33/1250 m. The result is `(Qty Rational meter)` — the common reference unit, a
           deterministic function of the operand units (units-of-measure.md #Combining Units Of One
           Dimension Is Well-Formed). Exact because the magnitudes are `Rational`.")
  (input  (+ (Qty.of (Rational.of 1 1) (Unit.of #"inch")) (Qty.of (Rational.of 1 1) (Unit.of #"millimeter"))))
  (output (: (Qty.of 33/1250 (Unit.base #"meter")) (Qty Rational (Unit.base #"meter")))))

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
  (input  (< (Qty.of (Rational.of 25 1) (Unit.of #"millimeter")) (Qty.of (Rational.of 1 1) (Unit.of #"inch"))))
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
  (doc    "`(Unit.in (Unit.of #\"meter\") (Qty.of (Rational.of 3 1) (Unit.prefix kilo (Unit.of #\"meter\"))))`
           converts 3 km to meters: `(Unit.prefix kilo meter)` has scale 1000·1 = 1000, so 3 km = 3000 m.
           Pins that a prefixed unit is a unit of the same dimension differing by the exact prefix
           factor.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of (Rational.of 3 1) (Unit.prefix kilo (Unit.of #"meter")))))
  (output (: (Qty.of 3000/1 (Unit.base #"meter")) (Qty Rational (Unit.base #"meter")))))

(case "a negative-power SI prefix is an exact rational scale"
  (doc    "`(Unit.in (Unit.of #\"second\") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #\"second\"))))`
           converts 5 ms to seconds: `milli` = 10⁻³ = 1/1000, so 5 ms = 5/1000 = 1/200 s. Pins that
           negative-power prefixes are exact `Rational` scales — the second reason exact rationals are
           load-bearing for units (a milli/micro/nano factor has no exact float or integer form).")
  (input  (Unit.in (Unit.of #"second") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #"second")))))
  (output (: (Qty.of 1/200 (Unit.base #"second")) (Qty Rational (Unit.base #"second")))))

(case "an IEC binary prefix scales a unit by a power of two"
  (doc    "`(Unit.in (Unit.of #\"byte\") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #\"byte\"))))`
           converts 1 MiB to bytes: `mebi` = 2²⁰ = 1048576, so 1 MiB = 1048576 byte. Pins the binary
           prefix family (kibi/mebi/gibi) alongside the decimal one — distinct scales for `information`.")
  (input  (Unit.in (Unit.of #"byte") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #"byte")))))
  (output (: (Qty.of 1048576/1 (Unit.base #"byte")) (Qty Rational (Unit.base #"byte")))))

(case "a decimal kilobyte and a binary kibibyte are distinct units of one dimension"
  (doc    "`(+ (Qty 1 KiB) (Qty 1 kB))` over the `information` dimension: kibibyte = 1024 byte and
           kilobyte = 1000 byte are DISTINCT units with distinct exact scales, so mixing them converts to
           the reference `byte` and sums to 1024 + 1000 = 2024 byte — NOT 2000, never silently equated.
           Pins that the two prefix systems are genuinely different scales the arithmetic keeps distinct
           (the classic KiB-vs-kB conflation is caught, not hidden).")
  (input  (+ (Qty.of (Rational.of 1 1) (Unit.prefix kibi (Unit.of #"byte")))
             (Qty.of (Rational.of 1 1) (Unit.prefix kilo (Unit.of #"byte")))))
  (output (: (Qty.of 2024/1 (Unit.base #"byte")) (Qty Rational (Unit.base #"byte")))))

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
  (doc    "`(+ (Qty.of 1.0 (Unit.prefix kilo meter)) (Qty.of 500.0 meter))` mixes kilometers and meters —
           one dimension, two scales — so each converts to the reference `meter` (1 km = 1000 m) and they
           add: 1000 + 500 = 1500 m. Over Float64 the scale multiply is ordinary float arithmetic; the
           result is a `(Qty Float64 meter)` at the reference unit.")
  (input  (Qty.value (+ (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"meter")))
                        (Qty.of 500.0 (Unit.base #"meter")))))
  (output (: 1500.0 Float64)))

(case "a decimal kilobyte and a binary kibibyte convert distinctly over Int64"
  (doc    "`(+ (Qty 1 KiB) (Qty 1 kB))` over Int64: kibibyte = 1024 byte and kilobyte = 1000 byte are
           DISTINCT units with distinct exact scales, so each converts to the reference `byte` and sums to
           1024 + 1000 = 2024 byte — NOT 2000, never silently equated (the classic KiB-vs-kB conflation
           caught). The conversion is exact integer arithmetic (both scales are whole).")
  (input  (Qty.value (+ (Qty.of 1 (Unit.prefix kibi (Unit.base #"byte")))
                        (Qty.of 1 (Unit.prefix kilo (Unit.base #"byte"))))))
  (output (: 2024 Int64)))

(case "a negative-power SI prefix converts over Float"
  (doc    "`(Unit.in (Unit.of #\"second\") (Qty.of 5.0 (Unit.prefix milli (Unit.of #\"second\"))))` converts
           5 ms to seconds over Float64: `milli` = 10⁻³ = 1/1000, so 5 ms = 5/1000 = 0.005 s. The
           exact-`Rational` case above pins 1/200; this pins the SAME negative-power conversion over the
           inexact numeric the seed already has — the scale ratio (1/1000) applies as float arithmetic, so
           the result is 0.005 (a value with no exact float form beyond this rounding, which is precisely
           the precision the spec permits `only where the underlying numeric type is itself inexact`).")
  (input  (Qty.value (Unit.in (Unit.of #"second") (Qty.of 5.0 (Unit.prefix milli (Unit.of #"second"))))))
  (output (: 0.005 Float64)))

(case "an IEC binary prefix converts exactly over Int64"
  (doc    "`(Unit.in (Unit.of #\"byte\") (Qty.of 1 (Unit.prefix mebi (Unit.of #\"byte\"))))` converts 1 MiB
           to bytes over Int64: `mebi` = 2²⁰ = 1048576, so 1 MiB = 1048576 byte. The exact-`Rational` case
           above pins the same magnitude; this pins the binary prefix (kibi/mebi/gibi) converting over the
           integer numeric the seed has — the whole scale is an exact integer multiply, so no precision is
           lost. Pairs with the decimal-prefix Float case to cover both prefix systems over concrete
           numerics via explicit `Unit.in`.")
  (input  (Qty.value (Unit.in (Unit.of #"byte") (Qty.of 1 (Unit.prefix mebi (Unit.of #"byte"))))))
  (output (: 1048576 Int64)))

(case "comparing quantities of one dimension at different scales converts before comparing"
  (doc    "`(< (Qty 500.0 meter) (Qty 1.0 (Unit.prefix kilo meter)))` compares meters to kilometers — one
           dimension, two scales — so each converts to the reference and compares there: 500 m < 1000 m,
           so it is true. Comparison, like `+`/`-`, converts differing units of one dimension rather than
           rejecting them.")
  (input  (< (Qty.of 500.0 (Unit.base #"meter"))
             (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"meter")))))
  (output (: true Bool)))

(case "mixing a prefixed unit across DIFFERENT dimensions is still a compile-time error"
  (doc    "`(+ (Qty 1.0 (Unit.prefix kilo meter)) (Qty 1.0 second))` mixes `length` and `time` — different
           dimensions — so it is CDZ0501, exactly as the unprefixed case is. A prefix scales WITHIN a
           dimension; it never bridges two, so the dimensional safety the layer exists for is untouched.")
  (input  (+ (Qty.of 1.0 (Unit.prefix kilo (Unit.base #"meter")))
             (Qty.of 1.0 (Unit.base #"second"))))
  (error  CDZ0501))

; ============================================================================================
; Named FAMILY units over concrete numerics — `Unit.of` consults the family registry (a name → its
; reference dimension + exact machine-integer scale), so `inch`/`foot`/`kilometer` are units of one
; dimension that auto-convert exactly as prefixed units do. The `(Rational.of …)` family cases above
; pin the exact-magnitude form (realized when Rational lands); these pin the same conversions over the
; numerics the seed already has (Float rounds, Int is exact/truncates — spec §A Unit Carries An Exact
; Scale). The family vocabulary is prelude DATA, not a privileged in-compiler list.

(case "adding two named family units of one dimension converts to the reference (Float)"
  (doc    "`(+ (Qty.of 1.0 (Unit.of #\"inch\")) (Qty.of 1.0 (Unit.of #\"millimeter\")))` — inch and
           millimeter are named units of `length` (inch = 127/5000 m, mm = 1/1000 m) — each converts to
           the reference `meter` and adds: 127/5000 + 1/1000 = 33/1250 = 0.0264 m. Over Float64 the exact
           scales apply as ordinary float arithmetic.")
  (input  (Qty.value (+ (Qty.of 1.0 (Unit.of #"inch"))
                        (Qty.of 1.0 (Unit.of #"millimeter")))))
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
  (doc    "`(Unit.in meter (Qty.of 3.0 kilometer))` converts 3 km to meters: 3 * 1000 = 3000 m. The
           magnitude is multiplied by the source-to-target scale ratio (km's 1000 over meter's 1) in the
           inner Float64 type; the result is `(Qty Float64 meter)`.")
  (input  (Qty.value (Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"kilometer")))))
  (output (: 3000.0 Float64)))

(case "Unit.in converts a quantity to a chosen smaller unit exactly (Int)"
  (doc    "`(Unit.in kilometer (Qty.of 2000 meter))` converts 2000 m to kilometers: 2000 / 1000 = 2 km,
           exact integer arithmetic (the ratio divides). Pins that Unit.in over Int64 is exact when the
           conversion is whole; a non-dividing ratio truncates (opting into integer math).")
  (input  (Qty.value (Unit.in (Unit.of #"kilometer") (Qty.of 2000 (Unit.of #"meter")))))
  (output (: 2 Int64)))

(case "Unit.in to a unit of a different dimension is a compile-time error"
  (doc    "`(Unit.in meter (Qty.of 3.0 second))` asks to convert a time to a length — different
           dimensions — so it is CDZ0501. Unit.in converts WITHIN a dimension (meter↔km), never ACROSS
           one; there is no scale relating a length to a time.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"second"))))
  (error  CDZ0501))

; ============================================================================================
; RUNTIME mixed-unit conversion — the scale multiply reaches the emitted component only when a magnitude
; is a RUNTIME value (units-of-measure.md #A Unit Conversion Is The Arithmetic The Source Denotes). A
; quantity built from a runtime parameter does not fold; the compiler emits `value * num / den` as real
; arithmetic in the inner type. These pass a runtime argument via `(call main …)`.

(case "a runtime mixed-unit sum emits the scale conversion (Int)"
  (doc    "`(+ (Qty.of v kilometer) (Qty.of 500 meter))` with `v` a runtime Int64 parameter: km converts
           to the reference meter by *1000 emitted at run time, so v=1 → 1000 + 500 = 1500 m. Pins that
           the conversion works on a NON-constant magnitude, not only a compile-time literal.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                            (Qty.of 500 (Unit.base #"meter")))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1500 Int64)))

(case "a runtime Unit.in conversion emits the scale multiply (Int)"
  (doc    "`(Unit.in meter (Qty.of v kilometer))` with `v` a runtime Int64: converts v km to meters by
           *1000 at run time, so v=3 → 3000 m. The explicit-conversion companion of the runtime mixed
           sum.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (Unit.in (Unit.of #"meter") (Qty.of v (Unit.of #"kilometer")))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 3000 Int64)))

; ============================================================================================
; DERIVED-dimension families — a named unit can name a DERIVED dimension (a rate = information/time, a
; frequency = 1/time), not only an atomic one. `mbps` is a unit of `byte/second`; `hertz` of `1/second`.
; This is the "name what it means to have bytes over time, its own family you convert between" case: the
; dimension a unit NAMES and the dimension arithmetic DERIVES (`bytes / seconds`) are the SAME free-
; abelian-group element, so a named rate and a computed rate mix and convert (units-of-measure.md #A
; Dimension Groups Interconvertible Units). Bignum-free: `mbps` = 10^6 bit / 8-bits-per-byte / second.

(case "a named rate unit converts to the reference rate (Float)"
  (doc    "`(Unit.in byte-per-second (Qty.of 1.0 mbps))` converts 1 megabit-per-second to bytes-per-
           second: a megabit is 10^6 bits = 10^6/8 bytes, so 1 mbps = 125000 byte/s. `mbps` is a named
           unit of the DERIVED dimension `information/time`, converting to its reference `byte/second`.")
  (input  (Qty.value (Unit.in (Unit.of #"byte-per-second") (Qty.of 1.0 (Unit.of #"mbps")))))
  (output (: 125000.0 Float64)))

(case "a rate derived by division mixes with a named rate unit of the same dimension"
  (doc    "`(bytes / seconds)` derives the dimension `byte/second` — the SAME dimension `mbps` names — so
           a computed rate and an `mbps` quantity combine and convert: (250000 byte / 1 s) + 1 mbps =
           250000 + 125000 = 375000 byte/s. Pins that a NAMED derived-dimension unit and a DERIVED-by-
           arithmetic dimension are one free-abelian-group element, mixing and converting freely.")
  (input  (Qty.value (Unit.in (Unit.of #"byte-per-second")
                       (+ (/ (Qty.of 250000.0 (Unit.of #"byte")) (Qty.of 1.0 (Unit.of #"second")))
                          (Qty.of 1.0 (Unit.of #"mbps"))))))
  (output (: 375000.0 Float64)))

(case "combining a named rate with a length is a dimensional error"
  (doc    "`(+ (Qty.of 1.0 mbps) (Qty.of 1.0 meter))` combines a rate (`information/time`) with a length
           — different dimensions — so it is CDZ0501. A named DERIVED-dimension unit obeys the same
           dimensional safety as an atomic one: its dimension is the exponent map `{byte:1, second:-1}`,
           incompatible with `{meter:1}`.")
  (input  (+ (Qty.of 1.0 (Unit.of #"mbps")) (Qty.of 1.0 (Unit.of #"meter"))))
  (error  CDZ0501))

; ============================================================================================
; USER family declarations — `(Unit.define #"name" base-unit num den)` declares a new family unit as an
; existing unit scaled by an exact machine-int ratio, so a program declares its OWN units, not only the
; built-in vocabulary (units-of-measure.md #A Dimension Groups Interconvertible Units; #A Named Unit's
; Conversion Is Unique). The declared name then resolves through `Unit.of` and converts like any family
; unit. A name declared with a conversion conflicting with the built-in table or an earlier declaration
; is CDZ0502 — a unit's name→conversion must be a well-defined function.

(case "a program declares its own family unit and converts with it"
  (doc    "`(Unit.define #\"furlong\" (Unit.of #\"foot\") 660 1)` declares a furlong as 660 feet; then
           `(Unit.in meter (Qty.of 1.0 furlong))` = 660 * 381/1250 = 201.168 m. Pins that a user-declared
           unit joins the family of its base's dimension and converts by the composed scale — the layer
           fixes the mechanism, a program supplies its own vocabulary.")
  (input  (do
            (Unit.define #"furlong" (Unit.of #"foot") 660 1)
            (def (main) (Qty.value (Unit.in (Unit.of #"meter") (Qty.of 1.0 (Unit.of #"furlong")))))
            (export main)))
  (output (: 201.168 Float64)))

(case "declaring a unit with a conversion conflicting with a built-in is an error"
  (doc    "`(Unit.define #\"foot\" (Unit.of #\"meter\") 2 1)` redeclares the built-in `foot` (381/1250 m)
           as 2 m — a conflicting conversion — so it is CDZ0502 (units-of-measure.md #A Named Unit's
           Conversion Is Unique): a unit's name must resolve to ONE conversion. A redeclaration that
           AGREED with the built-in would be admissible; a disagreement is rejected.")
  (input  (do
            (Unit.define #"foot" (Unit.of #"meter") 2 1)
            (def (main) 0)
            (export main)))
  (error  CDZ0502))

(case "redeclaring a built-in unit with its own conversion is admissible"
  (doc    "`(Unit.define #\"foot\" (Unit.of #\"meter\") 381 1250)` redeclares the built-in `foot` at its
           OWN scale (381/1250 m) — an AGREEING redeclaration — so it is admitted, not CDZ0502
           (units-of-measure.md #A Named Unit's Conversion Is Unique: a redeclaration that agrees is
           admissible; only a CONFLICTING one is rejected). `foot` still resolves, and 2 ft = 0.6096 m.
           The admissible companion of the conflict case: the check rejects a DISAGREEMENT, not a restated
           agreement.")
  (input  (do
            (Unit.define #"foot" (Unit.of #"meter") 381 1250)
            (def (main) (Qty.value (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"foot")))))
            (export main)))
  (output (: 0.6096 Float64)))

(case "an agreeing redeclaration compares the normalized ratio, not the literal numerator and denominator"
  (doc    "`(Unit.define #\"foot\" (Unit.of #\"meter\") 762 2500)` restates the built-in `foot` as 762/2500
           m, which REDUCES to the built-in 381/1250 — the same conversion written unreduced — so it
           agrees and is admitted (not CDZ0502). Pins that the uniqueness check compares the NORMALIZED
           ratio (a conversion is a rational number, not a syntactic num/den pair): 762/2500 and 381/1250
           are one conversion, so 2 ft = 0.6096 m as before.")
  (input  (do
            (Unit.define #"foot" (Unit.of #"meter") 762 2500)
            (def (main) (Qty.value (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"foot")))))
            (export main)))
  (output (: 0.6096 Float64)))

(case "redeclaring a user-declared unit with the same conversion is admissible"
  (doc    "`(Unit.define #\"span\" (Unit.of #\"meter\") 3 1)` twice declares `span` = 3 m identically — the
           agreement clause applies to a program's OWN earlier declaration, not only the built-in table
           (units-of-measure.md #A Named Unit's Conversion Is Unique) — so the second declaration is
           admitted and `span` resolves to one conversion: 2 span = 6.0 m. A CONFLICTING second
           declaration would be CDZ0502; a restated one is fine.")
  (input  (do
            (Unit.define #"span" (Unit.of #"meter") 3 1)
            (Unit.define #"span" (Unit.of #"meter") 3 1)
            (def (main) (Qty.value (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"span")))))
            (export main)))
  (output (: 6.0 Float64)))

(case "a plain number def is unaffected by a following statement (the `as` sugar stays in its statement)"
  (doc    "`def a() = 5.0` is a number, full stop. On the ML surface the `as` unit-conversion postfix
           (`value as meter` → `(Unit.in (Unit.of \"meter\") value)`) must apply only WITHIN one statement:
           a bare `as` beginning the NEXT line must not reach back across the newline and absorb this def's
           RHS, silently turning `def a() = 5.0` into `def a() = (5.0 as meter)` — changing a's type from a
           number to Qty(meter) on a mere line break. The `as` operator landed (8e73fdce) without the
           statement-boundary guard the quantity sugar got (f57c4a53); this pins the intended value. The
           s-expr surface has no `as` sugar, so this reads `a`'s RHS as the plain 5.0 it is; the ML
           printer->reader round-trip (which emits `as` for a `Unit.in` conversion) exercises the boundary.")
  (input  (do (def (a) 5.0) (def (main) (a)) (export main)))
  (call   main)
  (output (: 5.0 Float64)))
