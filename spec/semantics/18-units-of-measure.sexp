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

(case "a RUNTIME quantity is returned WITH its unit (the unit label is injected at compile time)"
  (doc    "A parameterized `(def (main (: v Int64)) (Qty.of v (Unit.base #\"meter\")))` returns a RUNTIME
           quantity: `main 7` renders `(: (Qty.of 7 meter) (Qty Int64 meter))` — the unit crosses the host
           boundary WITH the erased runtime magnitude. Units are COMPILE-TIME-ONLY (units-of-measure.md
           §Dimensions Are Checked Then Erased): the Qty erases to its bare inner scalar at run time (zero
           runtime cost, no runtime unit tracking), and the boundary formatter injects the unit LABEL — a
           compile-time constant from the statically-known `Qty` type — alongside the scalar. So a computed
           quantity is returnable/printable without an explicit `Qty.value`, the same value form a CONSTANT
           quantity crosses as, only the magnitude is a runtime hole.")
  (input  (do (def (main (: v Int64)) (Qty.of v (Unit.base #"meter"))) (export main)))
  (call   main (: 7 Int64)) (output (: (Qty.of 7 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter"))))
  (call   main (: 42 Int64)) (output (: (Qty.of 42 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

; `Qty`'s SECOND argument is a UNIT, not a type. A bare unbound name there — `(Qty Int64 meter)` — used to
; draw the type-oriented guidance (lowercase → "not a type variable"; uppercase → "unknown type, declare it
; with `(type …)`"), both NONSENSE for a unit position. It now NAMES the unit misuse ("`Qty`'s second argument
; is a UNIT") and spells the real form `(Unit.base #"…")` as a replace fix (the name IS the intended base-unit
; name). Both a lowercase and uppercase bare name, at a parameter site AND a value-annotation site, get the
; unit message; the INNER (first) Qty argument stays a type position (its own type guidance — the both-bad
; cross-diagnostic case and the valid `(Unit.base …)` control stay a rust/existing-corpus residual). (Migrated
; from rcdzc a_bare_name_in_a_qty_unit_position_names_it_a_unit_not_a_type.)
(case "a bare lowercase name in the Qty unit position names it a unit misuse, not a type variable"
  (input  (do (def (g (: q (Qty Int64 meter))) q) (export g)))
  (error  CDZ0101 (message "`Qty`'s second argument is a UNIT") (message "(Unit.base") (not "type variable") (fix (kind replace) (replacement "(Unit.base #\"meter\")") (unverified))))

(case "a bare uppercase name in the Qty unit position names it a unit misuse, not an unknown type"
  (input  (do (def (g (: q (Qty Int64 Meter))) q) (export g)))
  (error  CDZ0101 (message "`Qty`'s second argument is a UNIT") (not "declare it with `(type") (fix (kind replace) (replacement "(Unit.base #\"Meter\")") (unverified))))

(case "the Qty unit-position misuse fires at a value-annotation site too"
  (input  (do (def (main) (: 5 (Qty Int64 meter))) (export main)))
  (error  CDZ0101 (message "`Qty`'s second argument is a UNIT") (fix (kind replace) (replacement "(Unit.base #\"meter\")") (unverified))))

(case "a Qty with a bad inner TYPE and a bad unit gets position-aware guidance — type guidance inner, unit guidance outer"
  (doc    "The cross-diagnostic position-awareness (migrated from rcdzc
           a_bare_name_in_a_qty_unit_position_names_it_a_unit_not_a_type): `(Qty widget meter)` has BOTH a bad
           inner TYPE argument (`widget`) and a bad unit (`meter`). Each position gets its OWN guidance in the
           same program — the inner type position keeps the TYPE-oriented message (`widget` is not a type
           variable), and the unit position gets the UNIT-oriented `not a unit` guidance — they do NOT
           cross-contaminate into both-as-units. Pins that the unit-position redirect is scoped to the second
           argument and does not swallow a genuine inner-type fault.")
  (input  (do (def (g (: q (Qty widget meter))) q) (export g)))
  (error  CDZ0101 (message "not a type variable") (message "not a unit")))

; The bare-SYMBOL twin of the bare-name unit slip above: `(Qty Float64 #"meter")` writes the unit's NAME
; directly (as a Symbol) where a unit EXPRESSION belongs. It used to fall through the bare-name check (a
; symbol is not a name) to the generic "requires a type, but found a non-type" — misleading (the position is
; a UNIT) with no repair. It now names the misuse ("is a UNIT expression, not a bare symbol") and, because the
; symbol text is in hand, carries the exact `(Unit.base #"…")` wrap fix. Fires at a parameter site, a
; value-annotation site, and NESTED inside a `(List …)`; the generic type message is superseded. (Migrated
; from rcdzc a_bare_symbol_in_a_qty_unit_position_names_it_a_unit_and_offers_the_wrap_fix.)
(case "a bare SYMBOL in the Qty unit position names a unit expression, not a bare symbol, with a wrap fix"
  (input  (do (def (g (: q (Qty Float64 #"meter"))) q) (def (main) 0) (export main)))
  (error  CDZ0201 (message "is a UNIT expression, not a bare symbol") (not "requires a type, but found a non-type") (fix (kind replace) (replacement-contains "(Unit.base") (unverified))))

(case "the bare-symbol Qty unit misuse fires at a value-annotation site"
  (input  (do (def (main) (: 5 (Qty Int64 #"sec"))) (export main)))
  (error  CDZ0201 (message "is a UNIT expression, not a bare symbol") (fix (kind replace) (replacement-contains "(Unit.base") (unverified))))

(case "the bare-symbol Qty unit misuse fires nested inside a List type"
  (input  (do (def (g (: xs (List (Qty Float64 #"kg")))) xs) (def (main) 0) (export main)))
  (error  CDZ0201 (message "is a UNIT expression, not a bare symbol") (fix (kind replace) (replacement-contains "(Unit.base") (unverified))))

; The VALUE-expression twin: a bare identifier in the SYMBOL-NAME argument of a unit BUILDER — `(Unit.base
; foot)`, `(Unit.of foot)`, `(Unit.define furlong …)` — is the author writing the unit's name as an
; identifier where a `#"…"` SYMBOL belongs. It used to resolve as a MISLEADING "unbound name `foot`" (with a
; did-you-mean to some near value); it now names the symbol requirement ("names its unit with a SYMBOL") and
; carries the `#"foot"` quote fix. An ORDINARY unbound name (not a unit-builder arg) keeps the plain "unbound
; name", not the redirect. (Migrated from rcdzc
; a_bare_name_in_a_unit_builder_names_the_symbol_and_offers_the_hash_quote_fix.)
(case "a bare name in Unit.base names the symbol requirement with a hash-quote fix"
  (input  (do (def (main) (Qty.of 1.0 (Unit.base foot))) (export main)))
  (error  CDZ0201 (message "is not a unit name here") (message "names its unit with a SYMBOL") (fix (kind replace) (replacement "#\"foot\"") (unverified))))

(case "a bare name in Unit.of names the symbol requirement with a hash-quote fix"
  (input  (do (def (main) (Qty.of 1.0 (Unit.of foot))) (export main)))
  (error  CDZ0201 (message "is not a unit name here") (fix (kind replace) (replacement "#\"foot\"") (unverified))))

(case "a bare name in Unit.define names the symbol requirement with a hash-quote fix"
  (input  (do (def (main) (Qty.of 1.0 (Unit.define furlong (Unit.base #"m") 201 1))) (export main)))
  (error  CDZ0201 (message "is not a unit name here") (fix (kind replace) (replacement "#\"furlong\"") (unverified))))

(case "an ordinary unbound name (not a unit-builder arg) keeps the plain unbound message, not the redirect"
  (input  (do (def (main) (bar 5)) (export main)))
  (error  CDZ0101 (message "unbound") (not "is not a unit name here")))

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

; A MALFORMED unit BUILDER in `Qty.of`'s unit position must be REJECTED at check, not leak to the opaque
; "function return type has no machine representation" at compile. `check_unit_composition` names the
; composition faults (a non-unit factor, a non-int exponent); these pin the LEAF-builder faults it also
; covers — a `Unit.of` named with a non-symbol, and a `Unit.^`/`Unit.*` at the wrong arity — which
; previously slipped `cdz check` (the operation is CONSUMED as a unit, so the partial-arity check does not
; fire) and surfaced only as the no-machine-representation compile error. (github-liaison/Copilot PR#506.)

(case "a Unit.of named with a non-symbol value is rejected"
  (doc    "`(Qty.of 5 (Unit.of 42))` — a unit builder names its unit with a `#\"…\"` SYMBOL, but `42` is an
           Int64. `unit_of` declines it, so it is not a real unit; the check names the symbol requirement
           (CDZ0201) rather than letting it leak to `function return type has no machine representation` at
           compile. A bare-NAME arg `(Unit.of foot)` keeps its richer `#\"foot\"`-fix message; this is the
           non-name non-symbol case.")
  (input  (do (def (main) (Qty.of 5 (Unit.of 42))) (export main)))
  (error  CDZ0201))

(case "a Unit.^ at the wrong arity is rejected"
  (doc    "`(Qty.of 5 (Unit.^ (Unit.base #\"m\")))` applies `Unit.^` (which raises a unit to an integer
           power) to ONE operand. `unit_of` declines the under-applied builder, and because the form is
           CONSUMED as a unit the partial-builtin-arity check does not fire — so it leaked past `cdz check`.
           The unit-composition check now names the arity (CDZ0201): `Unit.^` takes 2 operands.")
  (input  (do (def (main) (Qty.of 5 (Unit.^ (Unit.base #"m")))) (export main)))
  (error  CDZ0201))

(case "a Unit.* at the wrong arity is rejected"
  (doc    "`(Qty.of 5 (Unit.* (Unit.base #\"m\")))` composes with `Unit.*` (a binary product of two units)
           given only ONE operand — the arity twin of the `Unit.^` case. Rejected CDZ0201 (`Unit.*` takes 2
           operands) rather than leaking to the compile-time no-machine-representation error.")
  (input  (do (def (main) (Qty.of 5 (Unit.* (Unit.base #"m")))) (export main)))
  (error  CDZ0201))

; `Qty.of <value> <unit>` requires its SECOND argument to be a UNIT: a non-unit second arg (a bare Int, a
; String, a tuple) made `eval::unit_of` return None and `type_of`'s `Qty.of` arm silently fall through to
; `Any`, so `cdz check` passed a quantity with no real unit. Now CDZ0201 naming the unit forms. Migrated from
; rcdzc a_non_unit_second_argument_to_qty_of_is_rejected. The no-double contrast — an UNBOUND unit name
; surfaces its own CDZ0101, not ALSO the not-a-unit reject — is the (not …) case at the end of this cluster.
(case "a Qty.of second argument that is a bare integer is rejected as not a unit"
  (input  (do (def (main) (Qty.of 5 5)) (export main)))
  (error  CDZ0201 (message "`Qty.of`'s second argument must be a UNIT")))

(case "a Qty.of second argument that is a string is rejected as not a unit"
  (input  (do (def (main) (Qty.of 5 "s")) (export main)))
  (error  CDZ0201 (message "`Qty.of`'s second argument must be a UNIT")))

(case "a Qty.of second argument that is a tuple is rejected as not a unit"
  (input  (do (def (main) (Qty.value (Qty.of 5 #tuple(1 2)))) (export main)))
  (error  CDZ0201 (message "`Qty.of`'s second argument must be a UNIT")))

(case "an UNBOUND unit name in Qty.of's second-arg position surfaces its own unbound error, not ALSO the not-a-unit reject"
  (doc    "The no-double contrast (migrated from rcdzc a_non_unit_qty_of_arg_unbound_unit_is_not_a_double_report):
           `(Qty.of 5 meter)` with `meter` undefined surfaces its OWN CDZ0101 unbound-name error and NOT ALSO
           the `Qty.of`'s-second-argument-must-be-a-UNIT reject. The not-a-unit check is guarded on the arg
           being otherwise fault-free, so an unbound arg reports only the unbound name — the two checks do not
           double-report one root cause.")
  (input  (do (def (main) (Qty.of 5 meter)) (export main)))
  (error  CDZ0101 (message "unbound name `meter`"))
  (no-diagnostic "second argument must be a UNIT"))

; A `Unit.*`/`Unit./`/`Unit.^` COMPOSITION with a MALFORMED operand — a non-unit factor, a non-integer
; exponent, a non-unit base — made `eval::unit_of` return None. `Qty.of`'s not-a-unit check SKIPPED it (the
; arg IS a unit-builder form, so it deferred to the builder's own validation), but the builder had NONE: it
; silently reduced to `Any`, `check` passed, and `compile` leaked "function return type has no machine
; representation". The composition is now WALKED to NAME the offending operand (CDZ0201). Valid compositions
; (a product of two units, an integer-exponent power, a nested composition) are unaffected — covered by the
; run cases below. (Migrated from rcdzc a_malformed_unit_composition_operand_is_named_not_silently_shipped.)
(case "a non-unit factor in a Unit.* product is named, not silently shipped"
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.* (Unit.base #"m") 5)))) (export main)))
  (error  CDZ0201 (message "`Unit.*` composes two UNITS, but this operand is not a unit")))

(case "a non-unit factor in a Unit./ quotient is named, not silently shipped"
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit./ (Unit.base #"m") 5)))) (export main)))
  (error  CDZ0201 (message "`Unit./` composes two UNITS, but this operand is not a unit")))

(case "a non-integer exponent in a Unit.^ power is named, not silently shipped"
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.^ (Unit.base #"m") 2.5)))) (export main)))
  (error  CDZ0201 (message "`Unit.^`'s exponent must be a compile-time integer")))

(case "a non-unit base in a Unit.^ power is named, not silently shipped"
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.^ 5 2)))) (export main)))
  (error  CDZ0201 (message "`Unit.^` raises a UNIT to a power, but this base is not a unit")))

(case "a valid base-unit Qty.of is accepted and Qty.value recovers the magnitude (no over-rejection)"
  (input  (do (def (main) (Qty.value (Qty.of 5 (Unit.base #"meter")))) (export main)))
  (call   main) (output (: 5 Int64)))

(case "a valid Unit.one Qty.of is accepted and Qty.value recovers the magnitude"
  (input  (do (def (main) (Qty.value (Qty.of 5 Unit.one))) (export main)))
  (call   main) (output (: 5 Int64)))

; `Qty.of <value> <unit>` requires its FIRST argument (the MAGNITUDE) to be a NUMERIC scalar — Int, Float,
; Rational, or BigInt. `Qty.of`'s scheme `∀a. a → Unit → (Qty a u)` does not constrain the magnitude, and
; `type_of`'s `Qty.of` arm wraps whatever type the value has, so a NON-numeric magnitude (a tuple, a bool, a
; string) silently passed `cdz check` and reached emit — where `Qty.pow`/scale assume a scalar and mis-width
; the boxed compound (an i32/i64 mismatch → invalid wasm, a checker-accepts-illtyped miscompile). Now
; CDZ0201 at check, naming the numeric requirement (reject-don't-miscompile) — the magnitude twin of the
; not-a-unit second-arg reject above. A CONCRETE non-numeric type only; an unsolved value is not pre-judged.
(case "a Qty.of magnitude that is a tuple is rejected as not numeric"
  (input  (do (def (main) (Qty.value (Qty.pow (Qty.of #tuple(true) (Unit.base #"gram")) 2))) (export main)))
  (error  CDZ0201 (message "a quantity's magnitude must be a numeric value")))

(case "a Qty.of magnitude that is a string is rejected as not numeric"
  (input  (do (def (main) (Qty.value (Qty.of "x" (Unit.base #"gram")))) (export main)))
  (error  CDZ0201 (message "a quantity's magnitude must be a numeric value")))

(case "a Qty.of magnitude that is a bool is rejected as not numeric"
  (input  (do (def (main) (Qty.value (Qty.of true (Unit.base #"gram")))) (export main)))
  (error  CDZ0201 (message "a quantity's magnitude must be a numeric value")))

(case "a valid Float magnitude Qty.of is accepted (no over-rejection of the numeric case)"
  (input  (do (def (main) (Qty.value (Qty.of 5.0 (Unit.base #"gram")))) (export main)))
  (call   main) (output (: 5.0 Float64)))

; A BARE number where a `(Qty …)` is expected gets a `(Qty.of <n> <unit>)` WRAP fix — the unit read from
; the EXPECTED quantity type — wherever the bare number meets a quantity: an ARGUMENT position and a
; LET-BINDER both carry the verified-shape wrap. A DIRECT value annotation `(: 5 (Qty …))` is message-only
; (its wrap payload's nested `(Unit.base …)` mis-splices the parse-based fix builder), but the message still
; names the `Qty.of` repair. A numeric MIX (a Float into a `Qty Int64`) is NOT the wrap shape — it still
; mismatches on the inner numeric type after wrapping, so it names the ordinary arg-type mismatch instead.
; (Migrated from rcdzc a_bare_number_where_a_quantity_is_expected_offers_the_qty_of_wrap.)
(case "a bare number to a Qty parameter offers the Qty.of wrap fix"
  (input  (do (def (g (: q (Qty Int64 (Unit.base #"meter")))) q) (def (main) (g 5)) (export main)))
  (error  CDZ0203 (fix (kind wrap) (replacement "(Qty.of … (Unit.base #\"meter\"))"))))

(case "a bare number bound to a Qty let-binder offers the Qty.of wrap fix"
  (input  (do (def (main) (let (((: x (Qty Int64 (Unit.base #"meter"))) 5)) x)) (export main)))
  (error  CDZ0203 (fix (kind wrap) (replacement "(Qty.of … (Unit.base #\"meter\"))"))))

(case "a bare number directly annotated a Qty names the Qty.of repair (message-only)"
  (input  (do (def (main) (: 5 (Qty Int64 (Unit.base #"meter")))) (export main)))
  (error  CDZ0203 (message "give the number the required unit") (message "(Qty.of") (no-fix)))

(case "a numeric-mix bare value into a Qty Int64 names the arg-type mismatch, not the Qty.of wrap"
  (input  (do (def (g (: q (Qty Int64 (Unit.base #"meter")))) q) (def (main) (g 5.0)) (export main)))
  (error  CDZ0203 (message "this argument is a Float64, but a value of type (Qty Int64")))

; A quantity in a NON-REFERENCE unit DISPLAYS at its dimension's reference unit with the magnitude
; SCALED to that reference — the same normalize-to-reference the mixed-unit combine runs, so a single
; quantity and a homogeneous combine render identically. This is the fix for the calc relabel bug
; (a bare `5 kilometer` printed `5 meter`: the render took the base-dimension NAME but dropped the ×1000
; SCALE, so the number and unit disagreed). Scaling is a DISPLAY concern — construction stores the value
; exactly and `Unit.in` converts by the exact direct ratio; the render applies the scale in the inner
; numeric type (Float rounds, Rational exact, Int truncates on a non-whole ratio — the numeric core's rule).

(case "a prefixed quantity displays scaled to its reference unit (Float)"
  (doc    "`5 kilometer` = `(Qty.of 5.0 (Unit.prefix kilo meter))` DISPLAYS as `5000.0 meter`: the render
           normalizes to the reference `meter`, applying the ×1000 scale to the magnitude so the number
           and unit AGREE. Pins the calc-relabel-bug fix — the OLD render showed `5.0 meter` (base name,
           scale dropped). Same value form a homogeneous combine (`5 km + 0 m`) produces (one source of
           truth).")
  (input  (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter"))))
  (output (: (Qty.of 5000.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "a standard unit abbreviation resolves to its canonical unit in a converting sum"
  (doc    "A terse SI/metric abbreviation resolves to the SAME family unit as its canonical spelling —
           `km` = `kilometer`, `m` = `meter` — so `(Unit.of #\"km\")` and `(Unit.of #\"m\")` name real units
           and `1.0 km + 500.0 m` converts both to the meter reference and sums: 1000 + 500 = 1500 m. Pins
           that the abbreviation surface a calculator user reaches for (`5 km`, `100 m`) resolves, not just
           the canonical long spelling. `Qty.value` unwraps the summed quantity to its bare Float64 magnitude
           at the reference.")
  (input  (do (def (main) (Qty.value (+ (Qty.of 1.0 (Unit.of #"km")) (Qty.of 500.0 (Unit.of #"m"))))) (export main)))
  (call   main)
  (output (: 1500.0 Float64)))

(case "a NEGATIVE prefixed quantity carries its sign through the reference scale-fold"
  (doc    "`-5 kilometer` = `(Qty.of -5.0 (Unit.prefix kilo meter))` DISPLAYS as `-5000.0 meter`: the
           reference scale-fold (×1000) applies to a NEGATIVE magnitude with the sign preserved — the scale
           multiply is sign-transparent, so -5 km → -5000 m, not +5000. The sibling of the positive `5 km →
           5000 m` display pin; isolates that scaling a stored quantity to its reference does not drop or
           flip the sign.")
  (input  (Qty.of -5.0 (Unit.prefix kilo (Unit.base #"meter"))))
  (output (: (Qty.of -5000.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "a quantity whose reference-scaled magnitude overflows its inner Int is rejected"
  (doc    "`(Qty.of 9223372036854776 kilometer)` : `(Qty Int64 …)` scales to `9223372036854776 × 1000` at
           its reference `meter` = 9.2e18, which EXCEEDS Int64's max (9223372036854775807). A quantity
           displays scaled to its reference, but the scaled magnitude must FIT the inner numeric type —
           the value form cannot render an out-of-range Int (the OLD render emitted 9223372036854776000, a
           wrong-VALUE miscompile). Per the overflow policy (numeric-model.md §Overflow Is Defined): a
           STATICALLY-KNOWN scaled magnitude that overflows DECLINES at compile time (CDZ0304), the constant
           twin of the runtime scale-multiply's trap-on-overflow — so the constant and runtime paths agree.
           A value whose scaled magnitude FITS (`5 km` → 5000 m) renders normally.")
  (input  (do (def (main) (Qty.of 9223372036854776 (Unit.prefix kilo (Unit.base #"meter")))) (export main)))
  (error  CDZ0304))

; The two BOUNDARY controls of the scaled-display overflow gate: it must fire ONLY when the scale-multiply
; actually overflows, never a step early and never when there is no scale to apply. (Migrated from rcdzc
; a_quantity_whose_reference_scaled_magnitude_overflows_its_inner_int_declines.)
(case "a prefixed-unit magnitude whose reference-scaled value JUST fits Int64 is not rejected"
  (doc    "The just-under-overflow control of the CDZ0304 scaled-display reject above: `9223372036854775 km`
           × 1000 = 9223372036854775000 = 9.223e18 < Int64 max (9223372036854775807), so the scaled magnitude
           FITS and the quantity renders normally at its reference `meter` — the overflow gate is not
           off-by-one and does not reject a value one step below the ceiling.")
  (input  (Qty.of 9223372036854775 (Unit.prefix kilo (Unit.base #"meter"))))
  (output (: (Qty.of 9223372036854775000 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

(case "a reference-unit magnitude at Int64 max is not rejected (no scale to overflow)"
  (doc    "The no-scale control: `(Qty.of 9223372036854775807 meter)` is already in the dimension's REFERENCE
           unit (scale 1/1), so the display applies NO scale multiply — an Int64-max magnitude has nothing to
           overflow and must render as-is. Pins that the scaled-display overflow gate fires on the SCALE
           multiply, not on the raw magnitude, so a reference-unit max value is never false-rejected.")
  (input  (Qty.of 9223372036854775807 (Unit.base #"meter")))
  (output (: (Qty.of 9223372036854775807 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

(case "a narrow-width quantity whose reference-scaled magnitude overflows its inner type is rejected"
  (doc    "The NARROW-WIDTH twin of the Int64 scaled-display overflow above: `(Qty.of (Int8.of 5) kilometer)`
           scales to `5 × 1000` = 5000 at its reference `meter`, which EXCEEDS Int8's max (127). The
           scaled-magnitude fit check peels `Ty::Qty` AND reads the INNER numeric type's actual width — not
           just Int64 — so a narrow inner (Int8) overflows on display exactly as Int64 does, DECLINING at
           compile time (CDZ0304). Confirms the display-scale overflow gate is width-aware for every integer
           inner, not hard-coded to 64 bits.")
  (input  (do (def (main) (Qty.of (Int8.of 5) (Unit.prefix kilo (Unit.base #"meter")))) (export main)))
  (error  CDZ0304))

; The narrow-inner width check also covers ARITHMETIC overflow, not just the scaled-display magnitude above:
; a quantity's `+` runs the ERASED inner numeric op, so a `(Qty Int8 m)` add must overflow-trap like a bare
; Int8. `(+ (Qty.of (Int8.of 100) m) (Qty.of (Int8.of 100) m))` = 200 overflows Int8 — a compile-provable
; overflow is CDZ0304 (a constant OPERATION with no value), the SAME code the bare Int8 add gets, NOT CDZ0302
; ("literal does not fit its width": each 100 FITS Int8, it is the SUM that overflows). The width check peels
; `Ty::Qty` to the inner Int8 width; without the peel the over-range constant slipped to a BACKEND CDZ0302
; `cdz check` never saw (a check-vs-compile gap). (Migrated from rcdzc
; a_narrow_width_int_quantity_overflow_is_cdz0304_not_backend_cdz0302.)
(case "a narrow-width quantity ADD whose sum overflows the inner type is rejected (same-unit arith path)"
  (input  (do (def (main) (Qty.value (+ (Qty.of (Int8.of 100) (Unit.base #"meter")) (Qty.of (Int8.of 100) (Unit.base #"meter"))))) (export main)))
  (error  CDZ0304))

(case "a mixed-scale narrow-width quantity combine whose reference-converted sum overflows is rejected"
  (doc    "The reference-CONVERTING arith path honors the inner width too: `1 km` → `1000 m`, then
           `1000 + 50` = 1050 overflows UInt8 (max 255) → CDZ0304, folded inside the quantity-combine Int arm
           after the scale conversion — the mixed-scale twin of the same-unit add overflow above.")
  (input  (do (def (main) (Qty.value (+ (Qty.of (UInt8.of 1) (Unit.prefix kilo (Unit.base #"meter"))) (Qty.of (UInt8.of 50) (Unit.base #"meter"))))) (export main)))
  (error  CDZ0304))

(case "a narrow-width quantity add whose sum FITS the inner type runs normally (no spurious overflow trap)"
  (doc    "The control of the narrow-width arith overflow pair: `50 + 50` = 100 fits Int8 (max 127), so the
           same-dimension add runs and `Qty.value` reads back 100 — the overflow gate does not over-reject a
           fitting narrow-width sum.")
  (input  (do (def (main) (Qty.value (+ (Qty.of (Int8.of 50) (Unit.base #"meter")) (Qty.of (Int8.of 50) (Unit.base #"meter"))))) (export main)))
  (call   main) (output (: 100 Int64)))

(case "a magnitude literal that overflows the annotated Qty INNER width is rejected"
  (doc    "The ANNOTATION face of Qty magnitude width (distinct from the scaled-display CDZ0304 pair above —
           no unit scaling here, the bare literal itself does not fit): `(: (Qty.of 999 meter) (Qty Int8
           meter))` — the annotation's INNER type `Int8` grounds the magnitude literal `999`, which overflows
           (-128..=127) → CDZ0302 'does not fit', exactly as the bare `(: 999 Int8)`. Pins that the width
           fit-check peels `Ty::Qty` on the ANNOTATION path and grounds the magnitude at the inner type —
           the Qty arm of the compound-payload width descent.")
  (input  (: (Qty.of 999 (Unit.base #"meter")) (Qty Int8 (Unit.base #"meter"))))
  (error  CDZ0302))

(case "a Float32-overflowing magnitude literal under a Float32 Qty inner type is rejected"
  (doc    "The float twin: `(: (Qty.of 1.0e300 meter) (Qty Float32 meter))` — `1.0e300` is finite at Float64
           but overflows binary32, and the annotation's inner `Float32` grounds the magnitude → CDZ0302.
           With the fitting control below, pins that the Qty inner-width grounding covers FLOAT widths and
           does not over-reject.")
  (input  (: (Qty.of 1.0e300 (Unit.base #"meter")) (Qty Float32 (Unit.base #"meter"))))
  (error  CDZ0302))

(case "a fitting Float32 Qty magnitude computes at the narrow inner width"
  (doc    "The no-over-reject control: `(Qty.of 1.5 meter)` under `(Qty Float32 meter)` — 1.5 is exactly
           representable in binary32, so the magnitude grounds at Float32 and `Qty.value` reads it back →
           1.5 at Float32. Guards the Qty inner-width grounding.")
  (input  (do
            (def (main)
              (Qty.value (: (Qty.of 1.5 (Unit.base #"meter")) (Qty Float32 (Unit.base #"meter")))))
            (export main)))
  (call   main) (output (: 1.5 Float32)))

(case "a quantity in a TUPLE inside a LIST element renders scaled (nested composition)"
  (doc    "The DEPTH-2 composition of the collection-element scale notes: the Qty sits in a tuple which
           itself is a LIST element — `(list (tuple 1 (Qty.of 5.0 km)))` renders `(list (tuple 1 (Qty.of
           5000.0 meter)))`. The per-element scale path must compose the list's `.*` segment with the
           tuple's positional segment to reach the Qty leaf; the whole-LIST and whole-MAP faces are pinned
           by the direct cases — this pins that the note-path descent NESTS (a scale-path machinery that
           handled only a top-level collection element would render the tuple-nested Qty raw at 5.0).")
  (input  #list(#tuple(1 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter"))))))
  (output (: #list(#tuple(1 (Qty.of 5000.0 (Unit.base #"meter"))))
             (List (Tuple Int64 (Qty Float64 (Unit.base #"meter")))))))

(case "a quantity in an OPTION inside a MAP value renders scaled (nested composition)"
  (doc    "The sum-wrapper composition: the Qty sits in an Option payload which itself is a MAP value —
           `{1 ↦ (Some (Qty.of 5.0 km))}` renders `(map (1 (Some (Qty.of 5000.0 meter))))`. The map's `!v`
           scale-path segment must compose with the Option payload segment to reach the Qty. With the
           tuple-in-list case above, pins both wrapper KINDS (positional compound + sum payload) nesting
           inside both collection positions that carry per-element notes.")
  (input  (Map.insert Map.empty 1 (Some (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter"))))))
  (output (: #map((= 1 (Some (Qty.of 5000.0 (Unit.base #"meter")))))
             (Map Int64 (Option (Qty Float64 (Unit.base #"meter")))))))

(case "a family quantity displays scaled exactly to its reference (Rational)"
  (doc    "`5 mile` = `(Qty.of (Rational.of 5 1) (Unit.of #\"mile\"))` DISPLAYS as `201168/25 meter`
           EXACTLY: mile = 201168/125 m, so 5 mile = 5·201168/125 = 201168/25 m, scaled at the reference
           with no rounding (the magnitude is Rational). Pins that a family unit's display scale is exact
           over Rational — the `5 mile → 5 meter` relabel bug is fixed with the correct scaled value.")
  (input  (Qty.of (Rational.of 5 1) (Unit.of #"mile")))
  (output (: (Qty.of 201168/25 (Unit.base #"meter")) (Qty Rational (Unit.base #"meter")))))

(case "an IEC-prefixed quantity displays scaled to its reference over Int64"
  (doc    "`1 KiB` = `(Qty.of 1 (Unit.prefix kibi byte))` DISPLAYS as `1024 byte`: the kibi scale (1024,
           a whole ratio) applies exactly over Int64 at the reference `byte`. Pins the display scaling
           over an integer magnitude at a whole-ratio prefix — number and unit agree (`1 byte` would be
           the relabel bug).")
  (input  (Qty.of 1 (Unit.prefix kibi (Unit.base #"byte"))))
  (output (: (Qty.of 1024 (Unit.base #"byte")) (Qty Int64 (Unit.base #"byte")))))

(case "a prefixed quantity displays scaled to its reference over Float32"
  (doc    "The Float32-inner companion of the Float64/Rational/Int64 scaled-display pins: `5 kilometer`
           = `(Qty.of (Float32.of 5.0) (Unit.prefix kilo meter))` DISPLAYS as `5000.0 meter` typed
           `(Qty Float32 meter)` — the ×1000 kilo scale applies to the Float32 magnitude at the reference
           `meter`. Pins that the reference display-scale fires over EVERY float width, not just Float64
           (the narrow-float inner threads the same const_value_ast_scaled path).")
  (input  (Qty.of (Float32.of 5.0) (Unit.prefix kilo (Unit.base #"meter"))))
  (output (: (Qty.of 5000.0 (Unit.base #"meter")) (Qty Float32 (Unit.base #"meter")))))

(case "a prefixed quantity displays scaled to its reference over BigInt (exact, no truncation)"
  (doc    "The BigInt-inner companion: `5 kilometer` = `(Qty.of (BigInt.of 5) (Unit.prefix kilo meter))`
           DISPLAYS as `5000 meter` typed `(Qty BigInt meter)` — the ×1000 whole-ratio scale applies EXACTLY
           over the arbitrary-precision BigInt at the reference `meter`, no truncation (contrast the Int64
           `5 foot` non-whole-ratio truncation; a whole-ratio prefix like kilo is exact over any integer
           inner). Pins the reference display-scale over a HEAP-numeric inner (a BigInt erases to a handle;
           the scale-multiply runs in the bignum path), completing the display matrix {Float64, Float32,
           Rational, Int64, BigInt}.")
  (input  (Qty.of (BigInt.of 5) (Unit.prefix kilo (Unit.base #"meter"))))
  (output (: (Qty.of 5000 (Unit.base #"meter")) (Qty BigInt (Unit.base #"meter")))))

(case "a family quantity displays scaled to its reference over Int64, truncating a non-whole ratio"
  (doc    "`5 foot` = `(Qty.of 5 (Unit.of #\"foot\"))` DISPLAYS as `1 meter` over Int64: foot = 381/1250 m,
           so 5 foot = 1905/1250 = 1.524 m, and the reference-normalized DISPLAY truncates toward zero to
           1 (the numeric core's Int rule — `Int truncates on a non-whole ratio`, contrast the exact
           Rational `5 mile` above and the whole-ratio Int64 `1 KiB` above). The truncation is a DISPLAY
           concern ONLY — the stored magnitude is kept exactly (`Qty.value` reads back 5, the foot-unit
           magnitude) and an explicit `Unit.in` converts by the exact direct ratio (5 foot in inches = 60,
           no rounding), so no value is lost at construction or conversion — only the Int reference render
           rounds, exactly as the same 1.524 over Float would display 1.524 and over Rational 1905/1250.")
  (input  (Qty.of 5 (Unit.of #"foot")))
  (output (: (Qty.of 1 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

(case "the display truncation does not lose the stored magnitude (Qty.value reads it back)"
  (doc    "`(Qty.value (Qty.of 5 (Unit.of #\"foot\")))` = 5 : Int64 — the value recovered is the stored
           foot-unit magnitude 5, NOT the truncated reference render `1` from the display case above. Pins
           that reference-normalization is a DISPLAY concern only: the stored magnitude is the number the
           source wrote, in the unit the source named, and `Qty.value` (the explicit exit) returns it
           unchanged — the Int display truncation never reaches back into the stored value.")
  (input  (Qty.value (Qty.of 5 (Unit.of #"foot"))))
  (output (: 5 Int64)))

(case "an explicit conversion off the truncating-display quantity is still exact (5 foot in inches = 60)"
  (doc    "`(Unit.in (Unit.of #\"inch\") (Qty.of 5 (Unit.of #\"foot\")))` = 60 : Int64 — 5 foot is exactly
           60 inches (foot = 12 inch), and the explicit conversion computes it by the exact direct ratio
           off the stored 5, with NO intermediate truncation to the `1 meter` reference display. Pins that
           the lossy Int reference DISPLAY (the case above) does not corrupt an EXACT conversion — the two
           are independent: display normalizes to reference and truncates, `Unit.in` converts by the
           direct source-to-target ratio and here stays whole.")
  (input  (Unit.in (Unit.of #"inch") (Qty.of 5 (Unit.of #"foot"))))
  (output (: 60 Int64)))

; A same-DIMENSION quantity annotation is a PURE DIMENSION CHECK — it must NOT re-label the value's unit
; (breaker's adv-annotation-rebrands-quantity-scale repro): `(: (Qty.of 1 kilometer) (Qty Int64 meter))`
; stays 1 KM downstream, NOT silently reinterpreted as 1 meter. The annotation names the dimension; it does
; not normalize/coerce the magnitude to the annotation's unit. (A rebrand inverted the units-safety promise:
; 1 km read as 1 m — a silent wrong-value miscompile. `type_of`'s Annot arm keeps the EXPRESSION's type.)
(case "a same-dimension quantity annotation does not rebrand: annotated 1 km converts to 1000 m"
  (doc    "`(: (Qty.of 1 kilometer) (Qty Int64 meter))` is 1 km annotated at the meter dimension; converting
           it to meters is 1000 (not the rebranded 1). Pins that the annotation checks the dimension without
           re-labelling the value's km unit as meter.")
  (input  (Unit.in (Unit.of #"meter") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1000 Int64)))

(case "a same-dimension-annotated quantity converts back to its own unit as the identity"
  (doc    "The sharpest no-rebrand witness: `(: (Qty.of 1 kilometer) (Qty Int64 meter))` converted BACK to
           kilometer is 1 (the identity) — a rebrand to `1 meter` would give 0 (1 m in km truncates). Pins
           that the annotated value retains its km scale.")
  (input  (Unit.in (Unit.of #"kilometer") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1 Int64)))

(case "a same-dimension-annotated quantity combines at its own scale: 1 km + 2 km = 3000 m"
  (doc    "The annotated `(: (Qty.of 1 kilometer) (Qty Int64 meter))` combined with a real `2 km` sums at km
           scale: 1 km + 2 km = 3 km = 3000 m. A rebrand (annotated entering as 1 m via a silent mixed-scale
           bypass) gave 2001; the correct no-rebrand result is 3000.")
  (input  (Unit.in (Unit.of #"meter")
            (+ (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))
               (Qty.of 2 (Unit.of #"kilometer")))))
  (output (: 3000 Int64)))

(case "recovering two quantities' magnitudes takes their remainder as bare numbers"
  (doc    "`%` (remainder) is not defined on quantity operands (the units surface has no `%` rule; it
           declines — that clean decline is pinned in rcdzc). The suggested repair is to recover each
           quantity's magnitude with `Qty.value` and take the remainder of the bare numbers:
           `(% (Qty.value (Qty.of 7 meter)) (Qty.value (Qty.of 3 meter)))` = 7 % 3 = 1. Pins that the
           documented repair works.")
  (input  (% (Qty.value (Qty.of 7 (Unit.base #"meter"))) (Qty.value (Qty.of 3 (Unit.base #"meter")))))
  (output (: 1 Int64)))

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

(case "adding a length and a time is a dimensional mismatch naming the operation and bare units"
  (doc    "L1-2: adding a length directly to a time is a DIMENSIONAL mismatch — CDZ0501 (units-of-measure.md
           §A Dimensional Mismatch Is An Error), a compile-time rejection (units erase before the program
           runs, never a runtime trap). The message names the OPERATION (adding) and the two bare UNITS
           (meter, second) and states the equal-dimensions rule — the rustc-gold form, not a full `(Qty …)`
           type dump — the message must NOT contain `(Qty`, pinned by the `(not …)` message-absence clause.
           (Migrated from rcdzc combining_quantities_of_incompatible_dimension_is_cdz0501.)")
  (input  (do (def (main) (+ (Qty.of 1.0 (Unit.base #"meter")) (Qty.of 1.0 (Unit.base #"second")))) (export main)))
  (error  CDZ0501 (message "adding") (message "meter") (message "second") (not "(Qty")))

; MIXED-MAGNITUDE-WIDTH under quantity arithmetic — the magnitudes UNIFY WITH THEIR CONSTRUCTION (operator
; seq-32: "types unify with their construction — the literal adopts the one width"). A BARE literal magnitude
; adopts its arith sibling quantity's concretely-fixed magnitude width (a length + a length share ONE `(Qty T
; u)`, so the two magnitudes share one width `T`), exactly as a bare literal adopts a fixed sibling in plain
; `(+ <lit> n)` — NOT a promotion. Before this the bare magnitude grounded to the Int64 DEFAULT while the
; sibling was `UInt32`, so the quantity arith emitted an i64 op over the i32 magnitude — invalid wasm with no
; diagnostic (fuzzer rcdzc-wasm-qty-add-mixed-magnitude-width). A genuine TWO-FIXED-width magnitude clash
; (an ANNOTATED `(: 5 Int64)` magnitude beside a `UInt32` one) still REJECTS CDZ0301 — no silent widening.
(case "a bare literal magnitude adopts its arith sibling's fixed integer width (unify with construction)"
  (doc    "`(+ (Qty.of 5 meter) (Qty.of v0 meter))` over `v0 : UInt32`: the bare `5` adopts the sibling
           quantity's `UInt32` magnitude (the arith unifies both quantities to one `(Qty UInt32 meter)`), so
           `main 3` returns `(: (Qty.of 8 meter) (Qty UInt32 meter))` — the same bare-literal-adopts-a-fixed-
           peer rule as `(+ 5 n)`, NOT a promotion. Formerly the bare `5` grounded to Int64 and the quantity
           arith emitted an i64 op over the i32 magnitude → invalid wasm with no diagnostic.")
  (input  (do (def (main (: v0 UInt32)) (Qty.value (+ (Qty.of 5 (Unit.base #"meter")) (Qty.of v0 (Unit.base #"meter"))))) (export main)))
  (call   main (: 3 UInt32)) (output (: 8 UInt32))
  (call   main (: 100 UInt32)) (output (: 105 UInt32)))

(case "a two-fixed-width magnitude clash under quantity arithmetic is CDZ0301 (no silent widening)"
  (doc    "`(+ (Qty.of (: 5 Int64) meter) (Qty.of v0 meter))` over `v0 : UInt32`: the LEFT magnitude is an
           ANNOTATED `Int64`, the RIGHT a `UInt32` — two CONCRETELY-fixed widths that do NOT unify, so the
           quantity add is a numeric-type contradiction, CDZ0301 (Cadenza never silently promotes). The
           annotation makes `5` fixed Int64 (the literal's parent is the `(: …)`, not the `Qty.of`), so the
           bare-literal adopt-the-peer rule does NOT apply — this is the genuine type error the adaptation is
           carefully NOT.")
  (input  (do (def (main (: v0 UInt32)) (+ (Qty.of (: 5 Int64) (Unit.base #"meter")) (Qty.of v0 (Unit.base #"meter")))) (export main)))
  (error  CDZ0301))

; Adding a quantity and a plain number — `(+ (Qty.of 5 (Unit.of #"meter")) 3)` — is CDZ0501 (no implicit
; dimensionless coercion), and carries a same-unit WRAP fix: give the bare number the SAME unit as the
; quantity operand, `(Qty.of <n> (Unit.base #"meter"))` (the unit is recoverable from the quantity operand),
; on whichever side is the plain number. The wrapped sum then type-checks. (Migrated from rcdzc
; adding_a_quantity_and_a_bare_number_offers_the_same_unit_wrap_fix.)
(case "adding a quantity and a bare number offers the same-unit Qty.of wrap fix"
  (input  (do (def (g) (+ (Qty.of 5 (Unit.of #"meter")) 3)) (export g)))
  (error  CDZ0501 (fix (kind wrap) (replacement "(Qty.of … (Unit.base #\"meter\"))"))))

(case "a bare number on the LEFT of a quantity add is wrapped in the quantity's unit"
  (input  (do (def (g) (+ 3 (Qty.of 5 (Unit.of #"meter")))) (export g)))
  (error  CDZ0501 (fix (kind wrap) (replacement "(Qty.of … (Unit.base #\"meter\"))"))))

(case "the same-unit wrap resolves the dimension fault — the wrapped quantity sum runs"
  (input  (do (def (main) (Qty.value (+ (Qty.of 5 (Unit.of #"meter")) (Qty.of 3 (Unit.base #"meter")))))
              (export main)))
  (call   main) (output (: 8 Int64)))

; The "a plain number" CDZ0501 message + its `(Qty.of …)` wrap fix are meaningful ONLY when the
; non-quantity operand is actually a NUMBER. A quantity added to a NON-numeric value — an `(Option (Qty …))`
; (the common `List.at`/`Map.get` result), a tuple, a string — is NOT a dimension slip; mislabeling it "a
; plain number" and offering a `(Qty.of …)` wrap would be nonsense. The additive-check arm is gated on the
; non-quantity operand being numeric; otherwise it falls through to the generic scheme-unify, which reports
; the accurate CDZ0203 for the real type clash (naming the `(Option …)`), NOT CDZ0501. The corpus grade of
; the PRIMARY as CDZ0203 is itself the regression guard against the CDZ0501 mislabel. (Migrated from rcdzc
; adding_a_quantity_to_a_non_numeric_operand_is_cdz0203_not_the_plain_number_cdz0501; the not-among-any-diag
; CDZ0501-"a plain number" message-ABSENCE negative is the inexpressible remainder kept white-box.)
(case "adding a quantity to a non-numeric Option operand is a plain CDZ0203 type mismatch, not a dimension slip"
  (doc    "A quantity added to a non-numeric `(Option (Qty …))` (a `List.at`/`Map.get` result) is the plain
           type mismatch CDZ0203, NOT the CDZ0501 'a plain number' dimension-slip (that arm is gated on the
           non-quantity operand being NUMERIC). `(no-other-errors)` pins that CDZ0203 is the SOLE error — so
           no CDZ0501 'a plain number' mislabel leaks alongside it, the guard the source test made across
           ALL diagnostics.")
  (input  (do (def (main) (Qty.value (+ (Qty.of 5 (Unit.base #"meter")) (List.at #list((Qty.of 1 (Unit.base #"meter"))) 0)))) (export main)))
  (error  CDZ0203) (no-other-errors))

(case "with the Option operand on the LEFT, the quantity-add clash names the (Option (Qty …)) type"
  (input  (do (def (main) (Qty.value (+ (List.at #list((Qty.of 1 (Unit.base #"meter"))) 0) (Qty.of 5 (Unit.base #"meter"))))) (export main)))
  (error  CDZ0203 (message "Option")))

; `*`/`/` on a quantity is dimensionally always well-formed, but the INNER numeric types must still agree
; (no silent promotion): a Float64 quantity scaled by a bare Int64 (`(* (Qty.of 5.0 …) 1)`) is CDZ0301 with
; the `1` -> `1.0` widening fix — matching the quantity's Float inner (without the check the mismatch reached
; lowering and emitted an i64 into an f64 multiply = invalid wasm). Likewise two same-dimension quantities
; whose INNER numerics differ (`(Qty.of 5 m) + (Qty.of 3.0 m)`) offer the SAME retype on the offending inner
; value (`5` -> `5.0`). Same-inner-type operations are clean. (Migrated from rcdzc
; scaling_a_float_quantity_by_a_bare_integer_is_a_numeric_mismatch_not_a_miscompile +
; a_numeric_inner_mismatch_under_a_unit_offers_the_same_coercion_fix_as_a_bare_number.)
(case "a Float64 quantity scaled by a bare Int64 is a numeric mismatch with the widening fix"
  (input  (do (def (g) (* (Qty.of 5.0 (Unit.base #"meter")) 1)) (export g)))
  (error  CDZ0301 (fix (kind replace) (replacement "1.0"))))

(case "a Float64 quantity scaled by a bare Float64 is well-formed and runs"
  (input  (do (def (main) (Qty.value (* (Qty.of 5.0 (Unit.base #"meter")) 2.0))) (export main)))
  (call   main) (output (: 10.0 Float64)))

(case "two same-dimension quantities with differing inner numerics retype the inner literal"
  (input  (do (def (g) (+ (Qty.of 5 (Unit.of #"meter")) (Qty.of 3.0 (Unit.of #"meter")))) (export g)))
  (error  CDZ0301 (fix (kind replace) (replacement "5.0"))))

(case "same-inner-type quantities of one dimension add cleanly and run"
  (input  (do (def (main) (Qty.value (+ (Qty.of 5 (Unit.of #"meter")) (Qty.of 3 (Unit.of #"meter")))))
              (export main)))
  (call   main) (output (: 8 Int64)))

; A `(Unit.of #"name")` naming a unit that is neither a built-in nor a user `Unit.define` is CDZ0201 naming
; the unknown unit. A NEAR-MISS of a real unit (`metre`/`secnd`) gets a did-you-mean + a Replace fix on the
; NAME literal that PRESERVES the argument's delimiter — a `#"…"` symbol → `#"meter"`, a plain string
; `"metre"` → `"meter"` — so the applied fix re-renders a valid argument. A name with NO confident neighbour
; (`mph`) gets ACTIONABLE compose/declare guidance instead of a misleading closest-matches list. An unknown
; base inside a `Unit.define` is caught too. (Migrated from rcdzc
; an_unknown_unit_in_a_quantity_literal_is_named_with_a_suggestion.)
(case "an unknown unit near a real one suggests it with a delimiter-preserving symbol rename fix"
  (input  (do (def (main) (Qty.of 5 (Unit.of #"metre"))) (export main)))
  (error  CDZ0201 (message "unknown unit `metre`") (message "did you mean `meter`?")
                  (fix (kind replace) (replacement "#\"meter\"") (unverified))))

(case "another unknown-unit near-miss suggests the near unit"
  (input  (do (def (main) (Qty.of 5 (Unit.of #"secnd"))) (export main)))
  (error  CDZ0201 (message "did you mean `second`?")
                  (fix (kind replace) (replacement "#\"second\"") (unverified))))

(case "an unknown unit with no confident neighbour gets compose/declare guidance, not closest-matches"
  (input  (do (def (main) (Qty.of 45 (Unit.of #"mph"))) (export main)))
  (error  CDZ0201 (message "compose a compound unit") (message "(Unit.define #\"mph\"")))

(case "applying the unknown-unit rename fix clears the fault — the corrected unit runs"
  (input  (do (def (main) (Qty.value (Qty.of 5 (Unit.of #"meter")))) (export main)))
  (call   main) (output (: 5 Int64)))

(case "an unknown BASE unit inside a Unit.define is caught and named"
  (input  (do (Unit.define #"furlong" (Unit.of #"zorks") 660 1) (def (main) 1) (export main)))
  (error  CDZ0201 (message "`zorks`")))

(case "a prefixed unit of a different dimension still rejects CDZ0501 (prefix scales within a dimension, never across)"
  (doc    "A PREFIX scales WITHIN a dimension, never across: `km + second` is still CDZ0501. Pins that the
           family/prefix relaxation (auto-convert within a dimension) does not weaken the dimensional
           safety the layer exists for. (migrated from rcdzc
           a_prefixed_unit_of_a_different_dimension_still_rejects_cdz0501.)")
  (input  (do (def (main) (+ ((. Qty of) 1.0 ((. Unit prefix) kilo ((. Unit base) #"meter")))
                             ((. Qty of) 1.0 ((. Unit base) #"second")))) (export main)))
  (error  CDZ0501))

; A `Unit.define` declares a derived unit `(Unit.define <symbol-name> <base-unit> <scale-int> <offset-int>)`
; — exactly four args, a SYMBOL name, INTEGER scale/offset. A wrong arity, a non-symbol name (a string), or
; a non-integer scale (a float) is a malformed declaration, CDZ0201 "a `Unit.define` is …". A well-formed
; `Unit.define` + its use is the satisfying family elsewhere in this file. (migrated from rcdzc
; a_malformed_unit_define_is_cdz0201.)
(case "a Unit.define with the wrong arity is rejected"
  (input  (do (Unit.define #"furlong" (Unit.of #"foot") 660) (def (main) 1) (export main)))
  (error  CDZ0201 (message "a `Unit.define` is")))

(case "a Unit.define with a non-integer scale is rejected"
  (input  (do (Unit.define #"furlong" (Unit.of #"foot") 660.5 1) (def (main) 1) (export main)))
  (error  CDZ0201 (message "a `Unit.define` is")))

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

(case "a runtime mixed-unit sum emits the scale conversion before the add"
  (doc    "The mixed-unit companion of the same-unit runtime sum: `(+ (Qty.of v kilometer) (Qty.of 500
           meter))` with `v` a runtime Int64. The kilometer operand must be scaled to the reference meter
           by a x1000 multiply, and because `v` is not a constant that scale is EMITTED as real arithmetic
           (lower_runtime_combine synthesizes `(+ (* v 1000) 500)`) rather than folded. v=1 gives 1 km +
           500 m = 1500 m; v=2 gives 2500 m — the scale multiply tracks the runtime magnitude. Qty.value
           recovers the erased sum. Distinct from the same-unit case, which emits no conversion.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                            (Qty.of 500 (Unit.base #"meter")))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1500 Int64))
  (call   main (: 2 Int64)) (output (: 2500 Int64)))

(case "a recursive fold SUMS a list of same-unit quantities threading a Qty accumulator"
  (doc    "The COLLECTION face of the same-unit sum above (one binop there; a recursive walk here): a
           fold over a `(List (Qty Int64 meter))` threads a Qty ACCUMULATOR through `(+ acc h)` per step
           — the unit layer erases per-element, leaving the plain integer fold (n+2+30 = 42 at n=10). A
           runtime element keeps the list out of the constant fold. Pins that the erased-magnitude
           arithmetic composes with the recursive fold spine and the accumulator's Qty type survives the
           recursion (a per-step re-wrap that lost or double-applied a scale would drift).")
  (input  (do
            (def (sum-q (: xs (List (Qty Int64 (Unit.base #"meter")))) (: acc (Qty Int64 (Unit.base #"meter"))))
              (match xs
                (#list() acc)
                (#list(h .. t) (sum-q t (+ acc h)))))
            (def (main (: n Int64))
              (Qty.value (sum-q #list((Qty.of n (Unit.base #"meter")) (Qty.of 2 (Unit.base #"meter")) (Qty.of 30 (Unit.base #"meter"))) (Qty.of 0 (Unit.base #"meter")))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case "a MAX-fold over quantities compares through the erased unit wrapper per step"
  (doc    "The comparison-accumulator companion: the fold's step is `(if (> h best) h best)` — a Qty
           COMPARISON deciding which Qty to thread. The `>` peels the unit layer to the erased Int64
           (equal dimensions), so the winner is by magnitude: n=42 dominates {5, n, 7} → 42; n=1 leaves
           7 the max. Pins ordering + selection + recursion over the wrapper in one shape (the running-max
           genre from 05-compound, lifted to quantities).")
  (input  (do
            (def (max-q (: xs (List (Qty Int64 (Unit.base #"meter")))) (: best (Qty Int64 (Unit.base #"meter"))))
              (match xs
                (#list() best)
                (#list(h .. t) (max-q t (if (> h best) h best)))))
            (def (main (: n Int64))
              (Qty.value (max-q #list((Qty.of 5 (Unit.base #"meter")) (Qty.of n (Unit.base #"meter")) (Qty.of 7 (Unit.base #"meter"))) (Qty.of 0 (Unit.base #"meter")))))
            (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64))
  (call   main (: 1 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case "a quantity over a BigInt magnitude runs unbounded arithmetic on the erased handles"
  (doc    "A `(Qty BigInt meter)` — a quantity whose inner numeric is the UNBOUNDED BigInt — runs bigint
           arithmetic on the erased inner handles, exactly as a bare `BigInt` `+` does. `(+ (Qty.of
           (BigInt.of n) meter) (Qty.of (BigInt.of 100) meter))`, `Qty.value` recovering the sum: n=5 →
           105, and a MASSIVE constant (10^12) → 1000000000005 (beyond Int64, the point of BigInt). Pins
           the fix for a MISCOMPILE: a BigInt inner is a heap HANDLE (i32), but the quantity `+` dispatch
           and the constant-materialize/ownership sites keyed on `Ty::BigInt` MISSED a `(Qty BigInt u)`
           (its type is `Ty::Qty { inner: BigInt }`), so it fell to the fixnum integer path and emitted an
           i64 where an i32 handle was expected → invalid wasm. The dispatch + materialize now peel the
           quantity to see the BigInt inner.")
  (input  (do (def (main (: n Int64))
                (Qty.value (+ (Qty.of (BigInt.of n) (Unit.base #"meter"))
                              (Qty.of (BigInt.of 100) (Unit.base #"meter"))))) (export main)))
  (call   main (: 5 Int64)) (output (: 105 BigInt))
  (call   main (: 1000000000000 Int64)) (output (: 1000000000100 BigInt))
  (live-objects known-leak))

(case "a quantity over a BigInt magnitude compares by its exact value"
  (doc    "A `(Qty BigInt meter)` COMPARISON routes to the exact bigint compare (`bigint-cmp`) on the
           erased inner handles, exactly as a bare `BigInt` `<` does — `(< (Qty.of (BigInt.of n) meter)
           (Qty.of (BigInt.of 100) meter))`: n=5 → true (1), n=200 → false (0). Pins the comparison
           companion of the bigint-quantity arithmetic fix: `bigint_operand` (read by `lower_comparison`)
           now peels `Ty::Qty` to see the BigInt inner, so a quantity comparison no longer declines
           ('comparison of a compound value needs a heap walk').")
  (input  (do (def (main (: n Int64))
                (if (< (Qty.of (BigInt.of n) (Unit.base #"meter"))
                       (Qty.of (BigInt.of 100) (Unit.base #"meter"))) 1 0)) (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 200 Int64)) (output (: 0 Int64)))

(case "a quantity over a Rational magnitude compares by its exact value"
  (doc    "The Rational companion: `(< (Qty (Rational 1/40) meter) (Qty (Rational 127/5000) meter))` — 25
           mm vs 1 inch expressed as exact reference-meter rationals — compares exactly (1/40 = 125/5000 <
           127/5000) → true. `rational_operand` peels `Ty::Qty` so a rational-quantity comparison folds
           through the exact cross-multiply path rather than declining as a compound compare.")
  (input  (< (Qty.of (Rational.of 1 40) (Unit.base #"meter"))
             (Qty.of (Rational.of 127 5000) (Unit.base #"meter"))))
  (output (: true Bool)))

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

(case "adding a quantity to a NON-numeric value is a plain type mismatch, not a dimension slip"
  (doc    "The quantity/non-quantity additive reject reports CDZ0501 'a quantity and a plain number' — with
           a `(Qty.of <n> <unit>)` repair — ONLY when the non-quantity operand is actually a NUMBER (the
           companion case: a quantity + a bare Int). But a quantity added to a NON-numeric value is not a
           dimension slip: here `(List.at xs 0)` has type `(Option (Qty Int64 meter))` (a lookup may miss),
           and adding a `(Qty …)` to an `Option` is an ordinary type clash, reported as CDZ0203 — NOT
           mislabeled 'a plain number' with a nonsensical `Qty.of` wrap. The additive-quantity arm is gated
           on the non-quantity operand being numeric; otherwise it falls through to the generic scheme-unify
           path, which names the actual `Option` mismatch (and, for an Option, even guides matching the
           `None` case). Pins that a quantity combined with a non-number gets the accurate diagnostic.")
  (input  (do
            (def (main)
              (Qty.value (+ (Qty.of 5 (Unit.base #"meter"))
                            (List.at #list((Qty.of 1 (Unit.base #"meter"))) 0))))
            (export main)))
  (error  CDZ0203))

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

(case "an acceleration dimension renders as a quotient with a squared denominator"
  (doc    "`(/ (Qty.of 10.0 meter) (Qty.pow (Qty.of 2.0 second) 2))` derives meter/second² — an acceleration
           — value 2.5. Beyond the simple `m/s` quotient, this pins a COMPOSITE derived dimension: the
           denominator is itself a `(Unit.^ second 2)`, so the value-form renderer nests a power inside the
           quotient — `(Unit./ (Unit.base meter) (Unit.^ (Unit.base second) 2))`. The `Qty.pow` on the second
           builds the squared denominator dimension; the group quotient then divides. Pins the nested
           quotient-of-power render (a distinct value-form shape from the flat `m/s`).")
  (input  (/ (Qty.of 10.0 (Unit.base #"meter")) (Qty.pow (Qty.of 2.0 (Unit.base #"second")) 2)))
  (output (: (Qty.of 2.5 (Unit./ (Unit.base #"meter") (Unit.^ (Unit.base #"second") 2)))
             (Qty Float64 (Unit./ (Unit.base #"meter") (Unit.^ (Unit.base #"second") 2))))))

(case "a reciprocal dimension renders as Unit.one over the unit"
  (doc    "`(/ (Qty.of 1.0 Unit.one) (Qty.of 4.0 second))` derives 1/second — a reciprocal (frequency-like)
           dimension — value 0.25. The dimensionless `Unit.one` numerator divided by a unit yields a quotient
           whose numerator is `Unit.one`: `(Unit./ (Unit.one) (Unit.base second))`. Pins the reciprocal
           value-form render (a `Unit.one` numerator, distinct from a base-unit numerator).")
  (input  (/ (Qty.of 1.0 Unit.one) (Qty.of 4.0 (Unit.base #"second"))))
  (output (: (Qty.of 0.25 (Unit./ Unit.one (Unit.base #"second")))
             (Qty Float64 (Unit./ Unit.one (Unit.base #"second"))))))

(case "a cube dimension renders as Unit.^ with exponent 3"
  (doc    "`(* (* (Qty.of 2.0 meter) (Qty.of 3.0 meter)) (Qty.of 4.0 meter))` derives meter³ — a volume —
           value 24.0. Beyond the meter² area pin, this pins a HIGHER power: three meter factors compose to a
           single `(Unit.^ (Unit.base meter) 3)` (the exponent accumulates, not a nested `Unit.* (Unit.^ … 2)
           meter`). Pins the cube value-form render + exponent accumulation across a chained product.")
  (input  (* (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))
             (Qty.of 4.0 (Unit.base #"meter"))))
  (output (: (Qty.of 24.0 (Unit.^ (Unit.base #"meter") 3))
             (Qty Float64 (Unit.^ (Unit.base #"meter") 3)))))

(case "a product of two DIFFERENT base dimensions renders as Unit.*"
  (doc    "`(* (Qty.of 3.0 meter) (Qty.of 2.0 second))` composes two DIFFERENT base dimensions — meter and
           second — into a product dimension `(Unit.* (Unit.base meter) (Unit.base second))`, value 6.0. The
           multiply rule never requires equal dimensions; two distinct bases stay a `Unit.*` product (unlike
           two equal bases, which accumulate into a `Unit.^`). Pins the `Unit.*` product value-form render
           for distinct bases.")
  (input  (* (Qty.of 3.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"second"))))
  (output (: (Qty.of 6.0 (Unit.* (Unit.base #"meter") (Unit.base #"second")))
             (Qty Float64 (Unit.* (Unit.base #"meter") (Unit.base #"second"))))))

(case "a scaled derived quantity displays with BOTH scales folded into the magnitude"
  (doc    "`100 km/h` = `(Qty.of 100.0 (Unit./ kilometer hour))` DISPLAYS as `27.77… meter/second`: the
           render normalizes a DERIVED (quotient) dimension to its reference units AND folds BOTH scales
           into the magnitude — the numerator's kilo prefix (×1000) and the denominator's hour→second scale
           (÷3600) compose to ×1000/3600 = ×5/18, so 100 → 27.777…. Beyond the flat scaled-prefix display
           (`5 km` → `5000 m`, a single base dimension) and the unscaled derived renders (`m/s`, `m·s`,
           `m/s²`, `m³`), this pins the COMPOSITION: a scaled numerator over a scaled denominator both
           reduce to reference, and the group magnitude carries the combined ratio. The rendered unit is the
           reference quotient `(Unit./ (Unit.base meter) (Unit.base second))`, number and unit AGREE.")
  (input  (Qty.of 100.0 (Unit./ (Unit.of #"kilometer") (Unit.of #"hour"))))
  (output (: (Qty.of 27.77777777777778 (Unit./ (Unit.base #"meter") (Unit.base #"second")))
             (Qty Float64 (Unit./ (Unit.base #"meter") (Unit.base #"second"))))))

(case "a scaled cube dimension raises its prefix scale to the power (km³ → ×1e9 meter³)"
  (doc    "`(Qty.of 2.0 (Unit.^ kilometer 3))` — a scaled base raised to a power — DISPLAYS as
           `2000000000.0 meter³`: the render normalizes to the reference `meter³` and the kilo prefix scale
           is RAISED TO THE EXPONENT, not applied linearly — ×(1000³) = ×1e9, so 2 → 2e9. Beyond the linear
           scale-fold of the flat scaled derived pin (`100 km/h` → `27.77… m/s`, scale ×5/18 applied once),
           this pins that a POWERED scaled unit folds the prefix scale RAISED to its exponent: `km³` is
           `(1000 m)³` = `1e9 m³`, NOT `1000 m³`. The rendered unit is the reference power
           `(Unit.^ (Unit.base meter) 3)`, number and unit AGREE.")
  (input  (Qty.of 2.0 (Unit.^ (Unit.of #"kilometer") 3)))
  (output (: (Qty.of 2000000000.0 (Unit.^ (Unit.base #"meter") 3))
             (Qty Float64 (Unit.^ (Unit.base #"meter") 3)))))

(case "a scaled reciprocal dimension folds the denominator prefix scale (1/ms → ×1000 per second)"
  (doc    "`(Qty.of 1.0 (Unit./ Unit.one millisecond))` — a reciprocal over a SCALED denominator —
           DISPLAYS as `1000.0 (Unit.one/second)`: the render normalizes `millisecond` to the reference
           `second` (ms = 1/1000 s), and because the scaled unit is in the DENOMINATOR its scale INVERTS —
           ÷(1/1000) = ×1000, so 1 → 1000. Pins the scaled-reciprocal render (a `Unit.one` numerator over a
           scaled base): the companion of the flat reciprocal pin (`1/s`, unscaled) and the scaled quotient
           pin (`km/h`), isolating the denominator-scale inversion. Number and unit AGREE at the reference
           `(Unit./ Unit.one (Unit.base second))`.")
  (input  (Qty.of 1.0 (Unit./ Unit.one (Unit.of #"millisecond"))))
  (output (: (Qty.of 1000.0 (Unit./ Unit.one (Unit.base #"second")))
             (Qty Float64 (Unit./ Unit.one (Unit.base #"second"))))))

(case "a bare TUPLE of quantities renders each element scaled to its reference (mixed inner types)"
  (doc    "A compound VALUE — a `(Tuple (Qty …) (Qty …))` literal — crossing the boundary renders each Qty
           element scaled to its reference in the value form, INDEPENDENTLY and with the element's own inner
           numeric type: `(Qty.of 5.0 kilometer, Qty.of 5 mile)` → `(tuple (Qty.of 5000.0 meter) (Qty.of
           201168/25 meter))` — the Float `5 km` scales to `5000.0 m` and the Rational `5 mile` scales EXACTLY
           to `201168/25 m`, both at the reference `meter`. Distinct from the collection-element/key matrix
           (those DECODE a quantity from a heap collection): this pins the const/value-form bake of a compound
           LITERAL of quantities, confirming the per-element scale-fold recurses into a tuple's holes and
           respects each element's inner type. Number and unit AGREE in every element.")
  (input  #tuple((Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))
                 (Qty.of (Rational.of 5 1) (Unit.of #"mile"))))
  (output (: #tuple((Qty.of 5000.0 (Unit.base #"meter")) (Qty.of 201168/25 (Unit.base #"meter")))
             (Tuple (Qty Float64 (Unit.base #"meter")) (Qty Rational (Unit.base #"meter"))))))

(case "an OPTION payload quantity renders scaled to its reference in the value form"
  (doc    "A `(Some (Qty …))` literal crossing the boundary renders its payload quantity scaled to its
           reference: `(Some (Qty.of 5.0 kilometer))` → `(Some (Qty.of 5000.0 meter))`. Distinct from the
           collection cases where `(List.at xs i)` RETURNS a `(Some (Qty …))` decoded from a heap collection
           and is then unwrapped with `Qty.value`: this pins the const/value-form bake of a `Some(Qty)`
           LITERAL, confirming the per-element scale-fold recurses into a SUM payload's hole exactly as it
           does into a tuple hole. Number and unit AGREE in the payload.")
  (input  (Some (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))))
  (output (: (Some (Qty.of 5000.0 (Unit.base #"meter")))
             (Option (Qty Float64 (Unit.base #"meter"))))))

(case "a NESTED tuple of quantities renders every element scaled at depth"
  (doc    "The recursion companion of the flat tuple-of-quantities pin: a `(Tuple (Tuple (Qty …)) (Tuple
           (Qty …)))` — quantities nested two tuple layers deep — renders EVERY element scaled to its
           reference: `((5.0 km,), (2.0 m,))` → `((5000.0 meter,), (2.0 meter,))`. Pins that the value-form
           scale-fold recurses through nested compound holes to arbitrary depth, not just the outermost
           tuple's direct elements — every Qty leaf, however deep, is normalized to its reference.")
  (input  #tuple(#tuple((Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter"))))
                 #tuple((Qty.of 2.0 (Unit.base #"meter")))))
  (output (: #tuple(#tuple((Qty.of 5000.0 (Unit.base #"meter"))) #tuple((Qty.of 2.0 (Unit.base #"meter"))))
             (Tuple (Tuple (Qty Float64 (Unit.base #"meter"))) (Tuple (Qty Float64 (Unit.base #"meter")))))))

(case "a MIXED-shape compound scales its quantity leaf beside a non-quantity element"
  (doc    "The heterogeneous companion of the uniform tuple/nested pins: a compound whose leaves are a MIX of
           a quantity-through-an-Option and a bare non-quantity — `(tuple (Some (Qty.of 5.0 kilometer)) 7)` —
           scales ONLY the Qty leaf (through the Option payload hole) to its reference while leaving the bare
           `7` untouched: → `(tuple (Some (Qty.of 5000.0 meter)) 7)` typed `(Tuple (Option (Qty Float64
           meter)) Int64)`. Pins that the value-form scale-fold is per-LEAF and shape-directed — it descends
           an Option nested inside a tuple to reach the Qty, and does not touch a sibling non-quantity element
           (no spurious scaling of the Int).")
  (input  #tuple((Some (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))) 7))
  (output (: #tuple((Some (Qty.of 5000.0 (Unit.base #"meter"))) 7)
             (Tuple (Option (Qty Float64 (Unit.base #"meter"))) Int64))))

(case "a velocity multiplied by a time recovers the distance dimension"
  (doc    "The multiply-direction inverse of the velocity quotient: `(* (Qty.of 6.0 (meter/second))
           (Qty.of 2.0 second))` composes `(meter·second⁻¹)·second` = meter — the `second` cancels in the
           group product — with value 12.0. Pins that multiplying a DERIVED (quotient) dimension by another
           quantity cancels exponents correctly back to a base dimension (the companion of `m·s / s = m`,
           which cancels through a DIVIDE; this cancels through a MULTIPLY). `Qty.value` recovers 12.0.")
  (input  (Qty.value (* (Qty.of 6.0 (Unit.* (Unit.base #"meter") (Unit.^ (Unit.base #"second") -1)))
                        (Qty.of 2.0 (Unit.base #"second")))))
  (output (: 12.0 Float64)))

(case "dividing two same-dimension quantities cancels to a dimensionless number"
  (doc    "`(/ (Qty.of 6.0 meter) (Qty.of 2.0 meter))` divides a length by a length — the meter exponents
           cancel (1 − 1 = 0) to the DIMENSIONLESS unit, value 3.0. The quotient of two quantities of the
           SAME dimension is a pure ratio (units-of-measure.md: a dimension divided by itself is the group
           identity). The companion of `unit · unit⁻¹ = dimensionless` (a cancellation through MULTIPLY);
           this cancels through a DIVIDE of same-dimension operands. `Qty.value` of the dimensionless
           result recovers the bare ratio 3.0.")
  (input  (Qty.value (/ (Qty.of 6.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter")))))
  (output (: 3.0 Float64)))

; DATA-RATE composite units (information ÷ time) — the desugared s-expr form of the ML-surface rate literal
; `59 GiB/s` (postfix-unit sugar → this shape; the sugar is ML-only, so the corpus pins the desugared form
; that carries the TYPING). Distinct from the length/time composites above (km/h, m/s²): this exercises the
; INFORMATION dimension with a BINARY prefix (GiB = ×2³⁰) and an INT magnitude, and pins rate ARITHMETIC
; (same-dimension add) + the cross-dimension guard on a COMPOSITE dimension. `Unit./` of two registered
; atomic units composes the derived `[(byte,1),(second,-1)]` dimension automatically — no rate registry row
; needed (a bare `bps`/`mbps` NAME would use one, but a `GiB/s` composite does not).
(case "a data-rate quantity converts a binary-prefix composite unit to its byte/second reference"
  (doc    "`59 GiB/s` (desugared `(Qty.of 59 (Unit./ (Unit.of GiB) (Unit.of s)))`) normalizes to the
           byte/second reference: the numerator's binary prefix GiB = 2³⁰ = 1073741824 bytes folds into the
           magnitude (the denominator `s` is already the reference), so 59 → 59·1073741824 = 63350767616,
           unit `(Unit./ byte second)`. Pins the DERIVED data-rate dimension (information÷time) with a binary
           prefix + Int magnitude — the composite of two atomic units types + converts with no registry row.")
  (input  (Qty.of 59 (Unit./ (Unit.of #"GiB") (Unit.of #"s"))))
  (output (: (Qty.of 63350767616 (Unit./ (Unit.base #"byte") (Unit.base #"second")))
             (Qty Int64 (Unit./ (Unit.base #"byte") (Unit.base #"second"))))))

(case "adding two data-rate quantities of the same composite dimension is exact"
  (doc    "`2 GiB/s + 1 GiB/s` = `3 GiB/s`: adding two quantities of the SAME derived data-rate dimension
           composes exactly — both normalize to byte/second (each GiB → ×1073741824) and the magnitudes add,
           2·1073741824 + 1·1073741824 = 3221225472 byte/second. Pins that same-dimension addition works over
           a COMPOSITE (derived) dimension, not only atomic units — the group-add is dimension-general.")
  (input  (+ (Qty.of 2 (Unit./ (Unit.of #"GiB") (Unit.of #"s")))
             (Qty.of 1 (Unit./ (Unit.of #"GiB") (Unit.of #"s")))))
  (output (: (Qty.of 3221225472 (Unit./ (Unit.base #"byte") (Unit.base #"second")))
             (Qty Int64 (Unit./ (Unit.base #"byte") (Unit.base #"second"))))))

(case "adding a data-rate to a length is a compile-time dimension error over a composite dimension"
  (doc    "`(GiB/s) + meter` rejects CDZ0501: a data-rate (byte·second⁻¹) and a length (meter) are
           incompatible dimensions, so their addition is a compile-time error — units are never silently
           converted across dimensions. Pins that the dimension-safety guard fires on a COMPOSITE/derived
           dimension (byte·second⁻¹), not only on atomic units — the companion of the atomic-unit mismatch
           cases above, isolating the derived-dimension side.")
  (input  (+ (Qty.of 2 (Unit./ (Unit.of #"GiB") (Unit.of #"s"))) (Qty.of 1 (Unit.of #"meter"))))
  (error  CDZ0501))

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

; Runtime cancellation companions (breaker). The cancel-through-DIVIDE runtime case above (`m·s / s = m`)
; has three siblings still unpinned on runtime magnitudes: cancelling through a MULTIPLY (a stored
; velocity times a time — the derived m·s⁻¹ QUOTIENT flowing on into a product), the same-dimension
; quotient collapsing to a DIMENSIONLESS value that then participates in BARE integer arithmetic (the
; erasure boundary: a fully-cancelled quantity's value is a plain number), and a derived dimension
; crossing a DEF boundary through explicitly-annotated `(Qty Int64 (Unit.base …))` parameters (the
; dimension algebra composing across a call, not only within one expression).

(case "a runtime velocity multiplied by a time cancels back to the base dimension"
  (doc    "`(* (/ dist time) time)` over runtime magnitudes: `(/ (Qty.of d meter) (Qty.of t second))`
           derives m·s⁻¹ at compile time; multiplying by the same `time` cancels the s⁻¹·s to meter. The
           erased arithmetic is `(d / t) * t` — with d=100, t=5 the checked integer ops give 20·5 = 100.
           The runtime companion of the constant velocity-times-time case: the QUOTIENT-derived dimension
           flows through a let into a later product and cancels correctly there, with all magnitudes
           runtime parameters (nothing folds).")
  (input  (do (def (main (: d Int64) (: t Int64))
                (let ((dist (Qty.of d (Unit.base #"meter")))
                      (time (Qty.of t (Unit.base #"second"))))
                  (let ((speed (/ dist time)))
                    (Qty.value (* speed time))))) (export main)))
  (call   main (: 100 Int64) (: 5 Int64)) (output (: 100 Int64))
  (call   main (: 9 Int64) (: 2 Int64)) (output (: 8 Int64)))

(case "a runtime same-dimension multiply derives the SQUARED dimension and computes the area"
  (doc    "The runtime companion of the const dimension-multiply pin (:508 folds): two boundary-parameter
           lengths multiply to a meter² AREA — the dimension composition happens at check time while the
           erased 6·7 = 42 runs at run time. Completes the runtime dimension-algebra trio: quotient
           (velocity), cancel-back (velocity·time), and now the same-dimension PRODUCT (a lowering
           confusing the squared result's erasure with a plain meter would still compute 42, but the
           check-side derivation is what this witnesses — the add-mismatch pin at :814 guards misuse).")
  (input  (do
            (def (main (: w Int64) (: h Int64))
              (Qty.value (* (Qty.of w (Unit.base #"meter")) (Qty.of h (Unit.base #"meter")))))
            (export main)))
  (call   main (: 6 Int64) (: 7 Int64))
  (output (: 42 Int64)))

(case "a runtime same-dimension quotient is dimensionless and its value joins bare integer arithmetic"
  (doc    "`(/ (Qty.of a meter) (Qty.of a meter))` over a runtime magnitude cancels to the dimensionless
           unit; `Qty.value` of the fully-cancelled quantity is a PLAIN Int64 that participates in bare
           arithmetic (`+ 41` = 42 at a=9, where 9/9=1). The erasure boundary of the group identity: once
           every exponent cancels, the value re-enters the ordinary numeric world with no unit residue.
           (The 9/2 call pins the truncating integer quotient under the unit layer too: 4+41 = 45.)")
  (input  (do (def (main (: a Int64) (: b Int64))
                (let ((x (Qty.of a (Unit.base #"meter")))
                      (y (Qty.of b (Unit.base #"meter"))))
                  (+ (Qty.value (/ x y)) 41))) (export main)))
  (call   main (: 9 Int64) (: 9 Int64)) (output (: 42 Int64))
  (call   main (: 9 Int64) (: 2 Int64)) (output (: 45 Int64)))

(case "a derived dimension composes across a def boundary through annotated Qty parameters"
  (doc    "`area` takes two explicitly-annotated `(Qty Int64 (Unit.base #\"meter\"))` parameters and
           returns their product — the derived meter² crosses the CALL boundary as the def's result
           dimension. Two areas from different call sites then ADD (same derived dimension, well-formed)
           and `Qty.value` recovers the erased 3·4 + 5·5 = 37. Pins that dimension derivation is not
           expression-local: a def's annotated Qty parameters participate in the group algebra, the
           derived result dimension survives the return, and same-derived-dimension results from separate
           calls are addable.")
  (input  (do (def (area (: w (Qty Int64 (Unit.base #"meter"))) (: h (Qty Int64 (Unit.base #"meter"))))
                (* w h))
              (def (main (: a Int64) (: b Int64) (: c Int64))
                (Qty.value (+ (area (Qty.of a (Unit.base #"meter")) (Qty.of b (Unit.base #"meter")))
                              (area (Qty.of c (Unit.base #"meter")) (Qty.of c (Unit.base #"meter"))))))
              (export main)))
  (call   main (: 3 Int64) (: 4 Int64) (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64) (: 7 Int64) (: 2 Int64)) (output (: 4 Int64)))

; SAME-BASE POWER dimension distinctions (breaker): the mismatch case above uses a MIXED product
; (meter·second) vs meter. These pin the subtler SAME-BASE-EXPONENT distinction: meter² (from meter·meter)
; is a DIFFERENT dimension from meter¹, and meter³ from meter², so adding across exponents is a mismatch —
; while adding two SAME-exponent quantities is well-dimensioned. A dimension check that merged same-base
; units without tracking the exponent (meter·meter → meter, not meter²) would wrongly accept meter²+meter.

(case "adding a squared length to a plain length is a dimension mismatch"
  (doc    "`(+ (* (Qty.of 2.0 meter) (Qty.of 3.0 meter)) (Qty.of 1.0 meter))` adds meter² (the product's
           derived dimension) to a plain meter — DIFFERENT dimensions (exponent 2 vs 1), so the compiler
           rejects it (CDZ0501). The same-base-power companion of the mixed meter·second mismatch above: it
           pins that meter·meter composes the EXPONENT (meter²), not collapsing to meter, so the mismatch
           check sees them as distinct. A dimension-multiply that merged same-base units to exponent 1 would
           wrongly accept this.")
  (input  (+ (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))
             (Qty.of 1.0 (Unit.base #"meter"))))
  (error  CDZ0501))

(case "adding two squared-length quantities is well-dimensioned"
  (doc    "The positive companion: `(+ meter² meter²)` — two areas of the SAME derived dimension — is valid,
           its magnitudes add. `(2·3) + (1·4)` = 6.0 + 4.0 = 10.0. Pins that same-exponent derived dimensions
           combine like any matching dimension, so the mismatch above is about the EXPONENT differing, not
           about derived dimensions being uncombinable in general.")
  (input  (Qty.value (+ (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))
                        (* (Qty.of 1.0 (Unit.base #"meter")) (Qty.of 4.0 (Unit.base #"meter"))))))
  (output (: 10.0 Float64)))

(case "adding a cubed length to a squared length is a dimension mismatch"
  (doc    "`(+ meter³ meter²)` — meter·meter·meter (exponent 3) vs meter·meter (exponent 2) — is a mismatch
           (CDZ0501). Pins the exponent distinction ONE step further than meter²-vs-meter¹: the check
           compares the full exponent, so consecutive powers are distinct dimensions, not just power-vs-linear.")
  (input  (+ (* (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter")))
                (Qty.of 2.0 (Unit.base #"meter")))
             (* (Qty.of 3.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))))
  (error  CDZ0501))

(case "Qty.pow and repeated multiplication produce the same dimension for a matching add"
  (doc    "`(+ (Qty.pow (Qty.of 2.0 meter) 2) (* (Qty.of 1.0 meter) (Qty.of 1.0 meter)))` adds a `Qty.pow`-2
           area to a `*`-derived area — both meter², so the add is well-dimensioned: 4.0 + 1.0 = 5.0. The
           add-position companion of the existing `(= (Qty.pow q 2) (* q q))` equality: not only do they
           compare equal, they combine as the SAME dimension under `+`.")
  (input  (Qty.value (+ (Qty.pow (Qty.of 2.0 (Unit.base #"meter")) 2)
                        (* (Qty.of 1.0 (Unit.base #"meter")) (Qty.of 1.0 (Unit.base #"meter"))))))
  (output (: 5.0 Float64)))

; The same-base power distinctions above go up to exponent 3 via repeated multiplication. These pin the
; exponent tracking at SCALE through `Qty.pow` — a LARGE exponent (meter^100) must still track the exact
; exponent (so meter^100 + meter^100 joins but meter^100 + meter^101 is a mismatch), and a high-power
; product must cancel exactly (meter^100 · meter^-100 = dimensionless). A dimension exponent stored in a
; too-narrow int, or a cancellation that didn't carry the full exponent, would pass at exponent 3 yet
; overflow, saturate, or mis-cancel at 100 — these witness the exponent map holds a wide range exactly.

(case "adding two equal high-power quantities joins (meter^100 + meter^100)"
  (doc    "`(+ (Qty.pow q 100) (Qty.pow q 100))` over meter: two meter^100 quantities share the exact same
           dimension, so the add joins — 1.0 + 1.0 = 2.0. Pins the dimension exponent tracks meter^100
           precisely (not saturated or wrapped), so equal high exponents are recognized as one dimension.")
  (input  (Qty.value (+ (Qty.pow (Qty.of 1.0 (Unit.base #"meter")) 100)
                        (Qty.pow (Qty.of 1.0 (Unit.base #"meter")) 100))))
  (output (: 2.0 Float64)))

(case "adding adjacent high-power quantities is a mismatch (meter^100 + meter^101)"
  (doc    "`(+ (Qty.pow q 100) (Qty.pow q 101))` — meter^100 vs meter^101 — are DISTINCT dimensions one
           exponent apart at scale, so the compiler rejects it (CDZ0501). Pins the exponent distinction
           holds far above the exponent-3 cases: the check compares the full exponent even at 100, so a
           representation that saturated or collided adjacent large exponents would wrongly accept this.")
  (input  (+ (Qty.pow (Qty.of 1.0 (Unit.base #"meter")) 100)
             (Qty.pow (Qty.of 1.0 (Unit.base #"meter")) 101)))
  (error  CDZ0501))

(case "a high-power product cancels exactly to dimensionless (meter^100 * meter^-100)"
  (doc    "`(* (Qty.pow q 100) (Qty.pow q -100))` over meter: exponent 100 + (-100) = 0, so the product is
           dimensionless and `Qty.value` reads the bare magnitude 3.0^100 · 3.0^-100 = 1.0. Pins that a
           high positive exponent and its negative cancel EXACTLY through the exponent map (the dimension
           returns to Unit.one), the extreme-exponent companion of the unit·inverse cancellation.")
  (input  (Qty.value (* (Qty.pow (Qty.of 3.0 (Unit.base #"meter")) 100)
                        (Qty.pow (Qty.of 3.0 (Unit.base #"meter")) -100))))
  (output (: 1.0 Float64)))

(case "scaling a quantity by a dimensionless quantity keeps its dimension"
  (doc    "`(* (Qty.of 2.0 meter) (Qty.of 3.0 Unit.one))` multiplies a length by a dimensionless scalar:
           meter·one = meter, value 6.0. Pins that `Unit.one` is the group identity — multiplying by it
           leaves the dimension unchanged — so scaling by a constant does not change a quantity's
           dimension.")
  (input  (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 Unit.one)))
  (output (: (Qty.of 6.0 (Unit.base #"meter")) (Qty Float64 (Unit.base #"meter")))))

(case "scaling a Float64 quantity by a bare integer is a numeric-type mismatch"
  (doc    "`(* (Qty.of 5.0 meter) 1)` scales a `(Qty Float64 meter)` by a bare `Int64` `1` — the SAME
           no-silent-promotion error a bare `(* 5.0 1)` gets (CDZ0301, numeric-model.md), NOT a silent
           success: a quantity's inner numeric type and a bare scaling factor must agree, exactly as two
           bare numbers must. Pins the fix for a miscompile where this was accepted and lowered the `1` as
           an i64 into an f64 multiply (invalid wasm); it must be a compile-time rejection with the `1` →
           `1.0` coercion the bare mismatch offers.")
  (input  (* (Qty.of 5.0 (Unit.base #"meter")) 1))
  (error  CDZ0301))

(case "scaling a quantity by a bare number of its own numeric type keeps its dimension"
  (doc    "`(Qty.value (* (Qty.of 2.0 meter) 3.0))` = 6.0: multiplying a `(Qty Float64 meter)` by a bare
           dimensionless Float64 keeps the dimension — the bare operand contributes `Unit.one`, so the
           result is a `(Qty Float64 meter)` and `Qty.value` recovers 6.0. Pins that a `(Qty T u) * <bare
           T>` is well-formed scaling that preserves the unit; before the apply_type reorder the Float
           operand-type arm preempted the quantity arm and the result mis-inferred as a bare `Float64`
           (the unit silently dropped, so `Qty.value` of it declined).")
  (input  (Qty.value (* (Qty.of 2.0 (Unit.base #"meter")) 3.0)))
  (output (: 6.0 Float64)))

(case "scaling an integer quantity by a bare integer of its own type keeps its dimension"
  (doc    "`(Qty.value (* (Qty.of 5 meter) 2))` = 10: the Int64 companion — a `(Qty Int64 meter)` scaled
           by a bare `Int64` stays a `(Qty Int64 meter)`, value 10. Pins the same unit-preserving scaling
           over the integer numeric type.")
  (input  (Qty.value (* (Qty.of 5 (Unit.base #"meter")) 2)))
  (output (: 10 Int64)))

(case "multiplying a scaled-unit quantity by the literal one keeps its dimension and displays scaled"
  (doc    "`(* (Qty.of 5 (Unit.prefix kilo meter)) 1)` — the calc `5 kilometer * 1` case: multiplying by
           the bare integer `1` keeps the dimension (the `* 1` does NOT drop the unit) and the result
           DISPLAYS at the reference `meter` with the magnitude scaled — `(Qty.of 5000 (Unit.base
           #\"meter\"))`. Pins both fixes together: the apply_type reorder (a `(Qty T u) * <bare T>` stays
           a quantity, not a bare number) and the reference-normalized display (5 km renders 5000 m). This
           closes the calc relabel bug's last item (`5 kilometer * 1` used to drop the unit entirely).")
  (input  (* (Qty.of 5 (Unit.prefix kilo (Unit.base #"meter"))) 1))
  (output (: (Qty.of 5000 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))

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

(case "raising a BigInt-magnitude quantity to a power runs the unbounded multiply"
  (doc    "`(Qty.pow (Qty.of (BigInt.of n) meter) 2)` over a BigInt inner: `Qty.pow` lowers to a repeated
           multiply of the erased magnitude (`value·value`), which for a BigInt runs the runtime `bigint-*`
           op on the heap handle — NOT the fixnum path. `Qty.value` recovers the squared magnitude: n=5 →
           25. Pins that `Qty.pow`'s repeated-multiply reaches the bigint arithmetic through the quantity
           (the `Qty.pow` companion of the bigint-inner-quantity arithmetic fix), staying valid + exact.")
  (input  (do (def (main (: n Int64))
                (Qty.value (Qty.pow (Qty.of (BigInt.of n) (Unit.base #"meter")) 2))) (export main)))
  (call   main (: 5 Int64)) (output (: 25 BigInt))
  (call   main (: 1000000 Int64)) (output (: 1000000000000 BigInt))
  (live-objects known-leak))

(case "the power form derives the same dimension as repeated multiplication"
  (doc    "`(= (Qty.pow (Qty.of 2.0 meter) 2) (* (Qty.of 2.0 meter) (Qty.of 2.0 meter)))` is true: raising
           to the 2nd power and multiplying twice derive the SAME dimension (meter²) AND the same value
           (4.0), so the equality is well-dimensioned and holds. Pins that `Qty.pow n` is definitionally
           the n-fold product — the unit exponents compose identically, decided by the canonical map.")
  (input  (= (Qty.pow (Qty.of 2.0 (Unit.base #"meter")) 2)
             (* (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 2.0 (Unit.base #"meter")))))
  (output (: true Bool)))

(case "raising a COMPUTED (derived) quantity to a power squares its magnitude and dimension"
  (doc    "`(Qty.pow (/ (Qty.of 6.0 meter) (Qty.of 2.0 second)) 2)` — raising a COMPUTED velocity (a
           `/`-derived `(Qty Float64 meter/second)`, NOT a directly-written `Qty.of`) to the 2nd power:
           (3.0 m/s)² = 9.0 (m²/s²). Pins that `Qty.pow` works over ANY quantity expression, not only a
           literal `Qty.of` — it previously declined ('Qty.pow over a non-Qty.of magnitude') because it
           read the value via the literal-only `qty_value_occ`; now it falls back to `Qty.value` (the
           erased magnitude) for a computed/let-bound quantity. `Qty.value` recovers the squared magnitude.")
  (input  (Qty.value (Qty.pow (/ (Qty.of 6.0 (Unit.base #"meter"))
                                 (Qty.of 2.0 (Unit.base #"second"))) 2)))
  (output (: 9.0 Float64)))

(case "raising a runtime computed quantity to a power squares the runtime magnitude"
  (doc    "The runtime companion: `(Qty.pow (/ (Qty.of n meter) (Qty.of 1 second)) 2)` with `n` a boundary
           parameter — squaring a computed velocity built from a runtime magnitude. n=4 → (4 m/s)² = 16.
           Pins that the `Qty.value` fallback for a non-literal `Qty.pow` argument also emits over a
           runtime magnitude, not only a constant.")
  (input  (do
            (def (main (: n Int64))
              (Qty.value (Qty.pow (/ (Qty.of n (Unit.base #"meter"))
                                     (Qty.of 1 (Unit.base #"second"))) 2)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 16 Int64)))

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

(case "a negative power over a Rational magnitude is the EXACT reciprocal"
  (doc    "`(Qty.pow (Qty.of (Rational.of 2 3) meter) -2)` over a Rational inner: the unit is meter⁻² and the
           magnitude is the EXACT reciprocal of (2/3)² = 4/9, i.e. 9/4 — no rounding, because a Rational
           carries its own denominator. The reciprocal `1 / value²` needs its numerator `1` in the INNER
           numeric type (`(Rational.of 1 1)`, NOT a bare Int64 `1`): a bare Int over a Rational value is a
           numeric mismatch that used to slip past the check inside the quantity and surface as a backend
           ownership error on the reciprocal divide. `1` is built in-type, so the exact rational reciprocal
           folds. The Rational companion of the Int64-truncating negative-power case above.")
  (input  (do (def (main)
                (Qty.value (Qty.pow (Qty.of (Rational.of 2 3) (Unit.base #"meter")) -2))) (export main)))
  (output (: 9/4 Rational)))

(case "a negative power over a BigInt magnitude truncates the reciprocal exactly"
  (doc    "`(Qty.pow (Qty.of (BigInt.of 4) meter) -1)` over a BigInt inner: the unit is meter⁻¹ and the
           reciprocal 1/4 is computed by BigInt division, which truncates toward zero to 0 — a BigInt has no
           fractions (the Rational above is the exact one). The reciprocal's numerator `1` is `(BigInt.of 1)`
           (the inner type), not a bare Int64 `1`; building it in-type is what clears the backend ownership
           error the mixed Int64/BigInt divide raised. Pins the BigInt companion of the negative-power
           reciprocal — the arithmetic is over the heap-BigInt handle, exact truncation, no fractions.")
  (input  (do (def (main)
                (Qty.value (Qty.pow (Qty.of (BigInt.of 4) (Unit.base #"meter")) -1))) (export main)))
  (output (: 0 BigInt))
  (live-objects known-leak))

; ============================================================================================
; Comparison — same dimension required (the ordering/equality obligation)
; ============================================================================================

(case "comparing two quantities of the same dimension yields a Bool"
  (doc    "`(< (Qty.of 2.0 meter) (Qty.of 3.0 meter))` compares two lengths and is true — comparison
           requires EQUAL dimensions (you can order two lengths) and yields a bare Bool. The underlying
           Float64 comparison runs unchanged on the erased values.")
  (input  (< (Qty.of 2.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter"))))
  (output (: true Bool)))

(case "comparing two same-unit Float-inner quantities at RUNTIME rides the scalar float compare"
  (doc    "`(< (Qty.of x meter) (Qty.of 5.0 meter))` with `x` a RUNTIME Float64: a same-unit Float-inner
           quantity comparison erases to a plain scalar float compare (the `(Qty Float64 meter)` erases to
           its f64), so it runs the runtime IEEE compare — x=3.0 → true (1), x=7.0 → false (0). Pins that a
           same-unit Float quantity comparison routes to the scalar float path (`float_operand` peels
           `Ty::Qty` to see the inner Float), NOT the compound-heap-walk decline it hit before — a gap
           masked until runtime float ordering landed (before that the bare float compare declined too).")
  (input  (do (def (main (: x Float64))
                (if (< (Qty.of x (Unit.base #"meter")) (Qty.of 5.0 (Unit.base #"meter"))) 1 0)) (export main)))
  (call   main (: 3.0 Float64)) (output (: 1 Int64))
  (call   main (: 7.0 Float64)) (output (: 0 Int64)))

(case "comparing two same-unit scalar-Int quantities at RUNTIME rides the scalar integer compare"
  (doc    "The Int-inner analogue of the Float case above. `(< (Qty.of x meter) (Qty.of 5 meter))` with `x`
           a RUNTIME Int64: a same-unit scalar-Int quantity comparison erases to a plain integer compare
           (the `(Qty Int64 meter)` erases to its i64). The heap-inner quantity comparisons (BigInt/Rational/
           Float) route via their own `Ty::Qty`-peeling operand predicates, but a SCALAR-Int inner fell to
           the generic `is_scalar` gate, which does NOT peel `Ty::Qty` — so a RUNTIME same-unit Int-quantity
           comparison (a parameter, or a `q` bound from a `Map.lookup`/`List.at` Option arm) declined
           'comparison of a compound value needs a heap walk', while a bare Int and a CONSTANT quantity
           compare fine. A same-unit quantity-comparison arm now rewrites `(op a b)` → `(op (Qty.value a)
           (Qty.value b))` (erasing both units to their inner numerics) and re-lowers. GUARDED on same-unit:
           a DIFFERENT-scale pair still routes through the mixed-scale CONVERSION arm (never a raw
           cross-scale compare). x=3 → true (1), x=7 → false (0).")
  (input  (do (def (main (: x Int64))
                (if (< (Qty.of x (Unit.base #"meter")) (Qty.of 5 (Unit.base #"meter"))) 1 0)) (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64))
  (call   main (: 7 Int64)) (output (: 0 Int64)))

; The SOUNDNESS guard on the same-unit scalar-Int compare-rewrite above: the rewrite `(op a b)` → `(op
; (Qty.value a) (Qty.value b))` fires ONLY for a same-unit pair. A DIFFERENT-scale pair must route through
; the mixed-scale CONVERSION arm instead — never a raw magnitude compare. The constant mixed-scale
; comparisons elsewhere fold at compile time; this pins the RUNTIME scalar-Int arm's guard (the scalar-Int
; companion of the runtime BigInt mixed-scale comparison — here the inner is a plain i64, so the wrong path
; would be a bare scalar compare of the unscaled magnitudes).
(case "a runtime cross-scale scalar-Int quantity comparison converts before comparing, not a raw magnitude compare"
  (doc    "`(< (Qty.of a kilometer) (Qty.of b meter))` with RUNTIME a, b: a km operand converts to the
           reference meter (×1000) before the compare. At a=1 km, b=999 m the compare is 1000 m < 999 m =
           FALSE (0), NOT a raw 1 < 999 (which would be TRUE) — the same-unit fast path's guard never fires
           across scales. At a=1 km, b=1001 m it is 1000 < 1001 = TRUE (1), confirming the result is the
           converted ordering and not an always-false collapse.")
  (input  (do (def (main (: a Int64) (: b Int64))
                (if (< (Qty.of a (Unit.prefix kilo (Unit.base #"meter")))
                       (Qty.of b (Unit.base #"meter"))) 1 0)) (export main)))
  (call   main (: 1 Int64) (: 999 Int64))  (output (: 0 Int64))
  (call   main (: 1 Int64) (: 1001 Int64)) (output (: 1 Int64)))

(case "a runtime Float quantity comparison against NaN is the IEEE partial order (false)"
  (doc    "`(< (Qty.of nan meter) (Qty.of 5.0 meter))` — a runtime NaN magnitude compares FALSE under the
           IEEE partial order (NaN is unordered), exactly as the bare `nan < 5.0` does: the quantity's Float
           inner rides the same runtime float compare, and units don't change the numeric ordering. x=nan →
           0. Pins that the Float-quantity comparison inherits the numeric core's IEEE partial-order
           semantics for NaN (the dimensional layer is erased before the compare).")
  (input  (do (def (main (: x Float64))
                (if (< (Qty.of x (Unit.base #"meter")) (Qty.of 5.0 (Unit.base #"meter"))) 1 0)) (export main)))
  (call   main (: nan Float64)) (output (: 0 Int64)))

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

; A mixed-scale comparison converts each operand to the reference in the INNER numeric type, so over Int
; the conversion TRUNCATES (the numeric core's rule, `Int truncates on a non-whole ratio`) — and two
; sub-reference-unit quantities can truncate to the SAME reference value and compare EQUAL even when their
; exact values differ. This is a documented consequence of opting into integer math (contrast the exact
; Rational comparison, which keeps the fractional scale). These pin that a mixed-scale comparison converts
; in the inner type — lossy over Int, exact over Rational — so the surprising Int result can't silently
; change and the Rational contrast documents the exact answer.

(case "a mixed-scale Int comparison truncates each operand to the reference (lossy, can compare equal)"
  (doc    "`(= (Qty.of 30 centimeter) (Qty.of 1 foot))` over Int64: each converts to the reference `meter`
           by its scale — 30 cm = 30/100 m and 1 foot = 381/1250 m — but INTEGER conversion truncates both
           to 0 m (each is < 1 m), so they compare EQUAL (1) even though 0.30 m ≠ 0.3048 m exactly. The
           documented Int-truncates rule (opting into integer math) applied to a comparison: the conversion
           happens in the inner type before comparing, so a sub-reference-unit Int quantity loses its
           fractional part. Contrast the exact Rational case below. Not a miscompile — the same truncation
           `Unit.in` over Int gives (30 cm in meter = 0, 1 foot in meter = 0).")
  (input  (if (= (Qty.of 30 (Unit.of #"centimeter")) (Qty.of 1 (Unit.of #"foot"))) 1 0))
  (output (: 1 Int64)))

(case "the same mixed-scale comparison is EXACT over Rational (keeps the fractional scale)"
  (doc    "`(< (Qty.of 30/1 centimeter) (Qty.of 1/1 foot))` over Rational: each converts to the reference
           EXACTLY — 30 cm = 3/10 = 375/1250 m, 1 foot = 381/1250 m — so 375/1250 < 381/1250 is true (1),
           the correct answer the Int case above truncates away. Pins that a Rational mixed-scale
           comparison keeps the fractional scale and gives the exact ordering (the reason exact rationals
           are load-bearing for units — the numeric type decides precision, not the dimensional layer).")
  (input  (if (< (Qty.of (Rational.of 30 1) (Unit.of #"centimeter"))
                 (Qty.of (Rational.of 1 1) (Unit.of #"foot"))) 1 0))
  (output (: 1 Int64)))

(case "a mixed-scale equality finds GENUINE equality after a whole-ratio conversion (5000 m = 5 km)"
  (doc    "`(= (Qty.of 5000.0 meter) (Qty.of 5.0 kilometer))` — a mixed-scale EQUALITY where the two values
           are genuinely equal after conversion: 5 km = 5×1000 = 5000 m, so 5000 m = 5 km is TRUE (1). Unlike
           the Int-truncation case above (30 cm = 1 foot compares equal by both truncating to 0 m — a
           coincidence of lossy conversion), this pins that a mixed-scale `=` over a WHOLE-ratio prefix (kilo
           = 1000/1) converts EXACTLY and finds real equality, not a truncation artifact. The companion
           `(= (Qty.of 5000.0 meter) (Qty.of 6.0 kilometer))` is FALSE (0) — 5000 m ≠ 6000 m — confirming the
           equality discriminates on the converted value, not an always-true collapse.")
  (input  (if (= (Qty.of 5000.0 (Unit.base #"meter"))
                 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))) 1 0))
  (output (: 1 Int64)))

(case "a mixed-scale equality is FALSE when the converted values differ (5000 m ≠ 6 km)"
  (doc    "The negative companion of the whole-ratio equality: `(= (Qty.of 5000.0 meter) (Qty.of 6.0
           kilometer))` converts 6 km = 6000 m, so 5000 m = 6000 m is FALSE (0). Pins that a mixed-scale
           equality returns false for genuinely unequal converted values — the discrimination the always-
           equal-by-truncation Int case cannot show.")
  (input  (if (= (Qty.of 5000.0 (Unit.base #"meter"))
                 (Qty.of 6.0 (Unit.prefix kilo (Unit.base #"meter")))) 1 0))
  (output (: 0 Int64)))

; ============================================================================================
; Join sites — an if/match/list of quantities must share ONE quantity type (unit AND scale)
; ============================================================================================
; A quantity join (the two branches of an `if`, the arms of a `match`, the elements of a `list`) must
; agree on the WHOLE `(Qty T u)` type, including the unit's SCALE — a quantity does NOT auto-normalize
; to the reference at a join (the no-silent-promotion rule; a conversion is explicit via `in`/`as`). Two
; same-dimension quantities at DIFFERENT units (km vs m) are DIFFERENT types, so a join rejects them
; (CDZ0203) — the diagnostic names the scale difference (both render to the reference-unit name, so the
; generic "same name, different type" hint would misread it as a shadowed declaration). SAME-unit
; branches join normally.

(case "an if over two same-dimension quantities at different units is a type mismatch (no auto-convert)"
  (doc    "`(if b (Qty.of 1 kilometer) (Qty.of 500 meter))` — the two branches are the SAME dimension
           (length) but DIFFERENT units (km vs m), which are DIFFERENT `(Qty T u)` types (their unit's
           scale to the reference differs: km is 1000/1, m is 1/1). A quantity join does not auto-convert
           to the reference — a unit conversion is explicit (`in`/`as`, the no-silent-promotion rule) —
           so the join is rejected CDZ0203. (Both branches RENDER to `(Qty Int64 (Unit.base #\"meter\"))` —
           the reference-unit name with the scale dropped — so the diagnostic must name the SCALE
           difference, not misread it as a shadowed-declaration same-name clash.)")
  (input  (do (def (main (: b Bool))
                (Qty.value (if b (Qty.of 1 (Unit.prefix kilo (Unit.base #"meter")))
                                 (Qty.of 500 (Unit.base #"meter"))))) (export main)))
  (error  CDZ0203))

(case "an if over two quantities at the SAME unit joins normally"
  (doc    "`(if b (Qty.of 1000 meter) (Qty.of 500 meter))` — both branches are the SAME `(Qty Int64
           meter)` type, so the join is well-typed and runs: b=true yields 1000. The control beside the
           different-unit rejection above — a same-unit quantity join is an ordinary well-typed join, no
           conversion needed.")
  (input  (do (def (main (: b Bool))
                (Qty.value (if b (Qty.of 1000 (Unit.base #"meter"))
                                 (Qty.of 500 (Unit.base #"meter"))))) (export main)))
  (call   main (: true Bool))
  (output (: 1000 Int64)))

(case "a list of two same-dimension quantities at different units names the scale difference (CDZ0201)"
  (doc    "The LIST-element peer-join sibling of the if-join scale-mismatch case: `(list (Qty.of 5.0
           kilometer) (Qty.of 2.0 meter))` — two SAME-dimension DIFFERENT-unit quantities — breaks list
           homogeneity (km and m are distinct `(Qty T u)` types; no auto-convert), rejected CDZ0201. Both
           elements RENDER to `(Qty Float64 (Unit.base #\"meter\"))` (the reference-unit name, scale
           dropped), so the diagnostic must name the SCALE difference (same dimension, different units,
           convert with `in`/`as`) rather than the confusing bare 'must share one type: (Qty … meter) and
           (Qty … meter)' — two identical-looking types. Pins the list-element join to fix-parity with the
           if/match join sites (qty_scale_mismatch_hint routed through peer_type_delta_hint).")
  (input  (do (def (main)
                (Qty.value (List.at #list((Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))
                                          (Qty.of 2.0 (Unit.base #"meter"))) 0))) (export main)))
  (error  CDZ0201))

; ============================================================================================
; Remainder (%) on same-dimension quantities is a SAME-DIMENSION INTEGER operation (operator ruling)
; ============================================================================================
; The units surface enumerates `+`/`-`/`*`/`/`/comparison AND `%` (remainder). A `%` of two same-dimension
; INTEGER quantities is well-formed — `7m % 3m = 1m` — mirroring `+`/`-` dimensionally: same dimension in,
; SAME unit out (a remainder does not compose units the way `*`/`/` do). Defined only for an integer/bigint
; inner numeric type (a float/rational has no remainder — exact division is total, so a float-quantity `%`
; declines like the bare float `%`). A cross-DIMENSION remainder (`7m % 3s`) is CDZ0501; a quantity mixed
; with a bare number (`7m % 3`) is CDZ0501 (no dimensionless coercion). (Operator ruling 2026-08-28:
; same-dimension mod makes sense; the earlier clean-decline was superseded by this fold.)

(case "remainder (%) on same-dimension integer quantities keeps the unit (7m % 3m = 1m)"
  (doc    "`(% (Qty.of 7 meter) (Qty.of 3 meter))` — remainder on same-dimension integer quantity operands —
           yields `1 meter`, so `Qty.value` of it is 1. `%` on quantities is same-in/same-out like `+`/`-`
           (SAME unit out, unlike `*`/`/` which compose units): the compiler checks the dimensions match
           (cross-dimension is CDZ0501), runs the remainder on the erased magnitudes (7 % 3 = 1), and
           recovers the unit from the solved `(Qty Int64 meter)`. Defined only for an integer inner (a
           float/rational quantity `%` declines — no remainder on exact/floating arithmetic). Operator
           ruling 2026-08-28: same-dimension mod is well-formed (superseded the prior clean-decline).")
  (input  (Qty.value (% (Qty.of 7 (Unit.base #"meter")) (Qty.of 3 (Unit.base #"meter")))))
  (output (: 1 Int64)))

(case "remainder (%) of quantities of incompatible dimension is CDZ0501"
  (doc    "`(% (Qty.of 7 meter) (Qty.of 3 second))` — a cross-DIMENSION remainder — is a dimensional error
           (CDZ0501), exactly like `+`/`-` across dimensions: a remainder requires equal dimensions (units
           are never silently converted across dimensions). The dimension pin for the new same-dimension `%`.")
  (input  (Qty.value (% (Qty.of 7 (Unit.base #"meter")) (Qty.of 3 (Unit.base #"second")))))
  (error  CDZ0501 (message "incompatible dimension")))

(case "remainder (%) on a floating-point quantity is CDZ0301 (no float remainder)"
  (doc    "`(% (Qty.of 7.0 meter) (Qty.of 3.0 meter))` — a remainder on a FLOAT-inner quantity — is rejected
           CDZ0301, the SAME code a bare float `%` gets: a remainder is an integer operation (exact/floating
           arithmetic has no remainder), so a float quantity `%` is rejected exactly as the bare float `%` is.
           Pins that the same-dimension `%` fold is INTEGER-only; the repair is an integer quantity, or
           recover the value with `Qty.value` first.")
  (input  (Qty.value (% (Qty.of 7.0 (Unit.base #"meter")) (Qty.of 3.0 (Unit.base #"meter")))))
  (error  CDZ0301 (message "floating-point or rational quantity")))

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

; An annotation checks the DIMENSION, not the scale — a unit is construction sugar for a magnitude at
; the dimension's reference (units-of-measure.md; DESIGN §Interaction With Annotations: "accept any unit
; of the right dimension; scale is construction sugar"). So annotating a quantity at a SAME-DIMENSION
; DIFFERENT-unit type is accepted, and the annotated value KEEPS ITS OWN SCALE — the annotation checks
; the dimension, it does NOT normalize/coerce the value to its unit. A cross-DIMENSION annotation is
; still CDZ0501; a same-dimension annotation whose INNER NUMERIC type differs is still CDZ0203.

(case "annotating a quantity at a same-dimension DIFFERENT unit is accepted (dimension checked, not scale)"
  (doc    "`(: (Qty.of 1 kilometer) (Qty Int64 meter))` annotates a kilometer quantity at meter — the SAME
           dimension (length) at a different scale. The annotation checks the DIMENSION, not the unit's
           scale (a unit is construction sugar), so it is ACCEPTED, and the value KEEPS ITS OWN SCALE: it
           stays 1 km, so `Qty.value` reads back 1 (the km magnitude), NOT a coerced-to-meter 1000. The
           annotation constrains the dimension without normalizing the value to its unit.")
  (input  (Qty.value (: (Qty.of 1 (Unit.prefix kilo (Unit.base #"meter"))) (Qty Int64 (Unit.base #"meter")))))
  (output (: 1 Int64)))

; The value keeping its own scale must survive a CONVERSION and a COMBINATION — the annotation is a pure
; dimension CHECK, it must NOT rebrand the value's unit (a landed regression re-labeled 1 km as 1 meter
; downstream: `Unit.in` read the wrong scale and a combine with real km silently bypassed the mixed-unit
; conversion — breaker's high-severity miscompile, adv-annotation-rebrands-quantity-scale-silent-magnitude
; -reinterpret.sexp). These three pin the no-rebrand invariant at each downstream use.

(case "a same-dimension annotation preserves the value's own scale through a conversion"
  (doc    "`(Unit.in meter (: (Qty.of 1 kilometer) (Qty Int64 meter)))` — the annotation CHECKS the
           dimension and the value KEEPS ITS OWN SCALE (still 1 km), so converting it to meters is 1000,
           NOT 1. A landed regression re-labeled the magnitude at the annotation's unit (1 km → 1 meter),
           making this 1 — the sharpest conversion witness of the rebrand. The annotation names the
           dimension; it does not normalize/coerce the magnitude to its unit.")
  (input  (Unit.in (Unit.of #"meter") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1000 Int64)))

(case "a same-dimension annotation is the identity under its own unit's conversion"
  (doc    "`(Unit.in kilometer (: (Qty.of 1 kilometer) (Qty Int64 meter)))` — converting the annotated
           value BACK to kilometers is the identity → 1 (the value is still 1 km). The rebrand regression
           gave 0 (a re-labeled 1 meter truncates to 0 km) — the single sharpest witness that the magnitude
           had been re-labeled at the annotation's unit rather than keeping its own.")
  (input  (Unit.in (Unit.of #"kilometer") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1 Int64)))

(case "an annotated quantity joins additions at its own scale, no silent mixed-scale bypass"
  (doc    "`(+ (: (Qty.of 1 km) (Qty Int64 meter)) (Qty.of 2 km))` in meters — the annotated operand is
           still 1 km, so one km plus two km is three km = 3000 m. The rebrand regression made the
           annotated operand enter the add as 1 METER, and the mixed add silently converted the 2 km
           without the guard firing → 2001 (compounding the rebrand with a silent mixed-scale sum the join
           rule forbids). Pins that a same-dimension annotation does not corrupt a downstream COMBINE.")
  (input  (Unit.in (Unit.of #"meter")
            (+ (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))
               (Qty.of 2 (Unit.of #"kilometer")))))
  (output (: 3000 Int64)))

(case "a same-dimension quantity annotation whose inner numeric type differs is still an error"
  (doc    "`(: (Qty.of 1 kilometer) (Qty Float64 meter))` shares the dimension (length) but the value's
           inner numeric type is Int64 while the annotation says Float64 — a genuine numeric-type conflict,
           CDZ0203. The dimension agreeing does NOT excuse a numeric mismatch: an annotation is an
           additional constraint on the WHOLE type, and the inner numeric types must still unify (the same
           no-silent-promotion the bare `(: 5 Float64)` enforces), even when the units share a dimension.")
  (input  (Qty.value (: (Qty.of 1 (Unit.prefix kilo (Unit.base #"meter"))) (Qty Float64 (Unit.base #"meter")))))
  (error  CDZ0203))

; A quantity annotation grounds + RANGE-CHECKS the inner numeric type exactly as a bare `(: 300 UInt8)`
; does — the annotation checks the dimension (scale is construction sugar) but still constrains the inner
; width/sign, so an out-of-range magnitude is CDZ0302, not silently accepted. Covers both a same-unit and
; a same-dimension different-scale annotation (the check drills the quantity's magnitude against the
; annotation's inner type, at the same choke point the compound-payload cases use).

(case "a quantity annotation range-checks the inner width — an out-of-range magnitude is rejected"
  (doc    "`(: (Qty.of 300 kilometer) (Qty UInt8 meter))` — the annotation checks the dimension (km and
           meter are both length, accepted) but STILL range-checks the inner numeric type: 300 does not fit
           UInt8 (0..=255), so CDZ0302, exactly as the bare `(: 300 UInt8)` is rejected. A quantity
           annotation grounds + checks the inner width like any annotation; the dimension-not-scale rule
           does not excuse an out-of-range magnitude (it previously slipped the inner check entirely).")
  (input  (Qty.value (: (Qty.of 300 (Unit.of #"kilometer")) (Qty UInt8 (Unit.base #"meter")))))
  (error  CDZ0302))

(case "a quantity annotation at a same-unit out-of-range magnitude is rejected"
  (doc    "`(: (Qty.of 300 meter) (Qty UInt8 meter))` — a SAME-UNIT annotation (the magnitude unit equals the
           annotation's inner unit) still range-checks the inner width: 300 does not fit UInt8 (0..=255) →
           CDZ0302. The same-unit companion of the same-dimension different-scale case above; pins the inner
           check fires regardless of the unit relationship (it drills the magnitude, not the units).")
  (input  (Qty.value (: (Qty.of 300 (Unit.base #"meter")) (Qty UInt8 (Unit.base #"meter")))))
  (error  CDZ0302))

(case "a quantity annotation rejects a negative magnitude at an unsigned inner width"
  (doc    "`(: (Qty.of -1 meter) (Qty UInt8 meter))` — a same-UNIT annotation whose inner width is unsigned:
           -1 does not fit UInt8 (0..=255), CDZ0302. Pins that the inner range-check also enforces SIGN (a
           negative into an unsigned width), the same as the bare `(: -1 UInt8)`, and that the check fires
           for a same-unit annotation too (it drills the magnitude regardless of the unit relationship).")
  (input  (Qty.value (: (Qty.of -1 (Unit.base #"meter")) (Qty UInt8 (Unit.base #"meter")))))
  (error  CDZ0302))

(case "a quantity annotation at an in-range narrow width grounds and accepts"
  (doc    "`(: (Qty.of 5 kilometer) (Qty UInt8 meter))` — the control: 5 fits UInt8 (0..=255), so the
           annotation grounds the inner to UInt8 and accepts, keeping the km unit (value stays 1·5 = 5 km).
           `Qty.value` reads back 5. Pins that the inner range-check rejects ONLY a genuine out-of-range
           magnitude — an in-range same-dimension annotation still grounds + accepts, unit preserved.")
  (input  (Qty.value (: (Qty.of 5 (Unit.of #"kilometer")) (Qty UInt8 (Unit.base #"meter")))))
  (output (: 5 UInt8)))

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
           (units-of-measure.md #A Unit Carries An Exact Scale To Its Dimension's Reference). `Unit.in`
           UNWRAPS: the result is the bare dimensionless number 127/5000 : Rational (the *count* of
           meters), NOT a `(Qty Rational meter)` — `as`/`in` is the deliberate exit from the units world
           (DESIGN-quantity-reference-normalized-unwrap.md §1b). Pins that a within-dimension conversion
           yields the exact scale the family declares, as a plain number.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of (Rational.of 1 1) (Unit.of #"inch"))))
  (output (: 127/5000 Rational)))

(case "a chained Unit.in re-wrapped with Qty.of converts inch to cm exactly (127/50)"
  (doc    "The two-step within-dimension conversion the chained-Unit.in CDZ0501 repair suggests: because
           `Unit.in` UNWRAPS to a bare number, chaining needs a `Qty.of` RE-WRAP between steps. `(Unit.in cm
           (Qty.of (Unit.in mm (Qty.of 1/1 inch)) mm))` converts 1 inch → mm (127/5), re-wraps as mm, → cm =
           127/50 (1 inch = 2.54 cm exactly). The exact-rational composition companion of the single-step
           conversion above. Relocated from rcdzc
           chaining_two_unit_in_conversions_is_a_clean_cdz0501_not_a_terse_runtime_decline — its CDZ0501
           chained-Unit.in reject + Qty.of-rewrap repair diagnostic stays in rcdzc.")
  (input  (Unit.in (Unit.of #"centimeter")
            (Qty.of (Unit.in (Unit.of #"millimeter")
                      (Qty.of (Rational.of 1 1) (Unit.of #"inch")))
                    (Unit.of #"millimeter"))))
  (output (: 127/50 Rational)))

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
           `Unit.in` UNWRAPS to the bare number 3000/1 : Rational. Pins that a prefixed unit is a unit of
           the same dimension differing by the exact prefix factor, converted to a plain count.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of (Rational.of 3 1) (Unit.prefix kilo (Unit.of #"meter")))))
  (output (: 3000/1 Rational)))

(case "a negative-power SI prefix is an exact rational scale"
  (doc    "`(Unit.in (Unit.of #\"second\") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #\"second\"))))`
           converts 5 ms to seconds: `milli` = 10⁻³ = 1/1000, so 5 ms = 5/1000 = 1/200 s. `Unit.in`
           UNWRAPS to the bare number 1/200 : Rational. Pins that negative-power prefixes are exact
           `Rational` scales — the second reason exact rationals are load-bearing for units (a
           milli/micro/nano factor has no exact float or integer form).")
  (input  (Unit.in (Unit.of #"second") (Qty.of (Rational.of 5 1) (Unit.prefix milli (Unit.of #"second")))))
  (output (: 1/200 Rational)))

(case "an IEC binary prefix scales a unit by a power of two"
  (doc    "`(Unit.in (Unit.of #\"byte\") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #\"byte\"))))`
           converts 1 MiB to bytes: `mebi` = 2²⁰ = 1048576, so 1 MiB = 1048576 byte. `Unit.in` UNWRAPS to
           the bare number 1048576/1 : Rational. Pins the binary prefix family (kibi/mebi/gibi) alongside
           the decimal one — distinct scales for `information`.")
  (input  (Unit.in (Unit.of #"byte") (Qty.of (Rational.of 1 1) (Unit.prefix mebi (Unit.of #"byte")))))
  (output (: 1048576/1 Rational)))

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
  (input  (Unit.in (Unit.of #"second") (Qty.of 5.0 (Unit.prefix milli (Unit.of #"second")))))
  (output (: 0.005 Float64)))

(case "an IEC binary prefix converts exactly over Int64"
  (doc    "`(Unit.in (Unit.of #\"byte\") (Qty.of 1 (Unit.prefix mebi (Unit.of #\"byte\"))))` converts 1 MiB
           to bytes over Int64: `mebi` = 2²⁰ = 1048576, so 1 MiB = 1048576 byte. The exact-`Rational` case
           above pins the same magnitude; this pins the binary prefix (kibi/mebi/gibi) converting over the
           integer numeric the seed has — the whole scale is an exact integer multiply, so no precision is
           lost. Pairs with the decimal-prefix Float case to cover both prefix systems over concrete
           numerics via explicit `Unit.in`.")
  (input  (Unit.in (Unit.of #"byte") (Qty.of 1 (Unit.prefix mebi (Unit.of #"byte")))))
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

(case "a plural family-unit spelling names the same unit as its singular"
  (doc    "The ML quantity-literal surface reads for natural language (`4.0 feet`, `1.0 meters`), so a
           common English PLURAL spelling names the SAME family unit as its canonical singular: `feet`
           is `foot`, `meters` is `meter`. Adding one meter to four feet therefore converts feet to the
           meter reference (foot = 381/1250 m) and adds: 1 + 4 * 0.3048 = 2.2192 m — the plural resolves
           and converts exactly as the singular would, rather than failing as an unknown unit.")
  (input  (Qty.value (+ (Qty.of 1.0 (Unit.of #"meters"))
                        (Qty.of 4.0 (Unit.of #"feet")))))
  (output (: 2.2192 Float64)))

; ============================================================================================
; Unit.in — EXPLICIT conversion to a chosen unit over concrete numerics. `(Unit.in TARGET q)` converts
; q's magnitude from its unit to TARGET (result `(Qty T TARGET)`), the way a program pins a specific
; result unit rather than the auto-chosen reference (units-of-measure.md #A Unit Conversion Is The
; Arithmetic The Source Denotes). The `(Rational.of …)` Unit.in cases above pin the exact-magnitude form
; (realized when Rational lands); these pin the same conversions over Float/Int.

(case "Unit.in converts a quantity to a chosen larger unit (Float)"
  (doc    "`(Unit.in meter (Qty.of 3.0 kilometer))` converts 3 km to meters and UNWRAPS: `Unit.in`/`as`
           yields the bare dimensionless number of the chosen unit, not a quantity
           (units-of-measure.md #An Explicit Conversion Unwraps To A Bare Number). 3 km is stored at the
           reference `meter` as 3000.0 (eager normalization), and converting it to `meter` is an identity
           that unwraps to the bare `3000.0 : Float64` — an ordinary number, no longer dimension-checked.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"kilometer"))))
  (output (: 3000.0 Float64)))

(case "Unit.in converts a quantity to a chosen smaller unit exactly (Int)"
  (doc    "`(Unit.in kilometer (Qty.of 2000 meter))` converts 2000 m to kilometers and UNWRAPS to the
           bare number `2 : Int64`: 2000 / 1000 = 2 km, exact integer arithmetic (the ratio divides).
           `Unit.in`/`as` strips the quantity wrapper, so the result is a plain Int64, not a
           `(Qty Int64 kilometer)`. Pins that Unit.in over Int64 is exact when the conversion is whole;
           a non-dividing ratio truncates (opting into integer math).")
  (input  (Unit.in (Unit.of #"kilometer") (Qty.of 2000 (Unit.of #"meter"))))
  (output (: 2 Int64)))

(case "Unit.in to a unit of a different dimension is a compile-time error"
  (doc    "`(Unit.in meter (Qty.of 3.0 second))` asks to convert a time to a length — different
           dimensions — so it is CDZ0501. Unit.in converts WITHIN a dimension (meter↔km), never ACROSS
           one; there is no scale relating a length to a time.")
  (input  (Unit.in (Unit.of #"meter") (Qty.of 3.0 (Unit.of #"second"))))
  (error  CDZ0501 (message "second") (message "meter")))

(case "chaining two Unit.in conversions is a compile-time error — the inner one already unwrapped"
  (doc    "`(Unit.in centimeter (Unit.in millimeter (Qty.of 1 inch)))` — the INNER `Unit.in` UNWRAPS to a
           bare number (Q3), so the OUTER `Unit.in` receives a plain number, not a quantity. `Unit.in`/`as`
           converts a QUANTITY, so this is CDZ0501 at COMPILE time (not the terse backend 'Unit.in of a
           non-quantity' at lowering). The message explains the unwrap and names the repair: re-wrap the
           intermediate with `Qty.of` if it should carry a unit. This is the deliberate exit-from-units
           semantic surfacing as a clean type error rather than a runtime failure.")
  (input  (Unit.in (Unit.of #"centimeter")
            (Unit.in (Unit.of #"millimeter") (Qty.of (Rational.of 1 1) (Unit.of #"inch")))))
  (error  CDZ0501
          (message "converts a QUANTITY")
          (message "which is not a quantity")
          (message "Qty.of")
          (not "of a non-quantity")))

(case "Qty.value of a conversion result is a compile-time error — the conversion already unwrapped"
  (doc    "`(Qty.value (Unit.in inch (Qty.of 5 foot)))` — the `Unit.in` UNWRAPS to a bare number (Q3: 60,
           the inches count), so `Qty.value` is applied to a PLAIN Int64, not a quantity. `Qty.value`
           recovers a QUANTITY's number, so this is CDZ0501 at COMPILE time. Previously the `Qty.value`-of-
           a-non-quantity type arm returned `Ty::Any` ('faulted elsewhere') but NOTHING faulted it, so the
           un-representable `Any` result slipped past `cdz check` and declined only at the backend
           ('function return type has no machine representation') — a check-vs-compile gap. Now a coded
           reject names the operand's type + the repair: a conversion result is already the bare number, so
           drop the `Qty.value`. (The convert-alone `(Unit.in inch (Qty.of 5 foot))` = 60 and the
           extract-alone `(Qty.value (Qty.of 5 foot))` = 5 both compile; only the redundant composition was
           the gap.) The bare-number sibling of the chained-`Unit.in` reject above.")
  (input  (Qty.value (Unit.in (Unit.of #"inch") (Qty.of 5 (Unit.of #"foot")))))
  (error  CDZ0501 (message "recovers a quantity") (message "which is not a quantity")
          (message "already UNWRAPS to a bare number") (not "no machine representation")))

(case "Unit.in of a NON-NUMERIC operand names the type, not a self-contradictory plain number"
  (doc    "`(Unit.in meter true)` applies the unit conversion to a Bool — not a quantity. CDZ0501 names the
           real operand type ('a Bool … which is not a quantity'), NOT the self-contradictory hardcoded 'a
           plain number, not a quantity' (a Bool is not a plain number). And because the operand is
           non-numeric (not the bare-number result of a chained Unit.in unwrap), it must NOT append the
           numeric-only 'conversion unwrapped it — re-wrap with Qty.of' hint. (migrated from rcdzc
           unit_in_of_a_non_numeric_operand_names_the_type_without_the_self_contradictory_plain_number.)")
  (input  (Unit.in (Unit.of #"meter") true))
  (error  CDZ0501 (message "a Bool") (message "which is not a quantity") (not "plain number") (not "Qty.of")))

(case "Qty.value of a NON-NUMERIC operand names the type, not a self-contradictory plain number"
  (doc    "`(Qty.value true)` extracts a magnitude from a Bool — not a quantity. CDZ0501 names the real
           operand type ('a Bool … which is not a quantity'), NOT the hardcoded 'a plain number, not a
           quantity'; and being a plain non-numeric (not an unwrapped conversion result), it must NOT append
           the 'conversion UNWRAPS it' chain hint. The Qty.value sibling of the Unit.in non-numeric case.
           (migrated from rcdzc qty_value_of_a_non_numeric_operand_names_the_type_without_the_self_contradictory_plain_number.)")
  (input  (Qty.value true))
  (error  CDZ0501 (message "a Bool") (message "which is not a quantity") (not "plain number") (not "UNWRAPS")))

(case "re-wrapping the intermediate with Qty.of makes a two-step conversion well-formed"
  (doc    "The repair for the chained-`Unit.in` error: wrap the first conversion's bare result back into a
           `Qty.of` at the unit it was converted to, so the second `Unit.in` sees a quantity again. `1 inch
           → mm` = 127/5 mm (a bare number after the unwrap), re-wrapped as `(Qty.of mm-value millimeter)`,
           then `→ cm` = 127/50 cm (1 inch = 2.54 cm exactly). Pins that the Qty.of-rewrap idiom the
           diagnostic suggests actually type-checks and computes the exact chained conversion. The
           intermediate is a `let`-bound so each conversion step is a flat form (the same computation, one
           `Unit.in` per step).")
  (input  (let ((mm (Unit.in (Unit.of #"millimeter") (Qty.of (Rational.of 1 1) (Unit.of #"inch")))))
            (Unit.in (Unit.of #"centimeter") (Qty.of mm (Unit.of #"millimeter")))))
  (output (: 127/50 Rational)))

(case "an unwrapped conversion result is a bare number under ordinary numeric rules"
  (doc    "`Unit.in` UNWRAPS: its result is a bare dimensionless number, so ordinary numeric arithmetic
           applies with NO dimension checking (DESIGN-quantity-reference-normalized-unwrap.md §1b —
           the deliberate exit from the units world). `(+ (Unit.in meter (Qty.of 2000 kilometer)) 5)`
           converts 2000 km to 2000000 m, unwraps to the bare Int64 2000000, then adds 5 as plain
           integer arithmetic → 2000005. Pins that after `as`/`in` you hold a number, not a quantity.")
  (input  (+ (Unit.in (Unit.of #"meter") (Qty.of 2000 (Unit.of #"kilometer"))) 5))
  (output (: 2000005 Int64)))

(case "an unwrapped length adds to an unwrapped time with no dimension error"
  (doc    "The unwrap is a REAL exit from dimensional checking: two `Unit.in` results of DIFFERENT
           dimensions add without CDZ0501, because each is already a bare number (the unit was
           intentionally dropped by `in`). `(+ (Unit.in meter (Qty.of 3 kilometer)) (Unit.in second
           (Qty.of 2 minute)))` = 3000 (meters) + 120 (seconds) = 3120 as plain integers — nonsensical
           dimensionally, but that is the user's choice once they unwrap. Contrast `(+ (Qty 3 km) (Qty 2
           minute))` which IS CDZ0501. Pins the headline unwrap semantic: `as`/`in` leaves the units
           world entirely.")
  (input  (+ (Unit.in (Unit.of #"meter") (Qty.of 3 (Unit.of #"kilometer")))
             (Unit.in (Unit.of #"second") (Qty.of 2 (Unit.of #"minute")))))
  (output (: 3120 Int64)))

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

(case "a runtime MIXED-SCALE BigInt-magnitude sum converts to the reference in unbounded arithmetic"
  (doc    "`(+ (Qty.of (BigInt.of v) kilometer) (Qty.of (BigInt.of 500) meter))` — a MIXED-SCALE combine
           (km + m) over a BigInt inner: each operand converts to the reference `meter` by its exact scale
           in UNBOUNDED bigint arithmetic (v km → v*1000 m), then adds. v=2 → 2000 + 500 = 2500. Pins the
           BigInt arm of `lower_quantity_combine` — a mixed-scale BigInt combine previously declined
           ('ownership cannot prove'), routing to the fixnum runtime path with bare-int scale factors; now
           it synthesizes `value * (BigInt.of scale)` so the conversion runs the runtime bigint ops.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of (BigInt.of v) (Unit.prefix kilo (Unit.base #"meter")))
                            (Qty.of (BigInt.of 500) (Unit.base #"meter")))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2500 BigInt))
  (live-objects known-leak))

(case "a scaled-unit PARAMETER annotation keeps its scale across the type round-trip (mixed-scale combine)"
  (doc    "A parameter annotated at a NON-reference unit — `(: a (Qty Int64 kilometer))` — must keep its
           scale (1000/1) so a mixed-scale combine with it CONVERTS. A `Ty::Qty`'s unit carries a `scale`
           ratio to the dimension reference, but the type-value ENCODE/DECODE round-trip a parameter
           annotation takes (`eval::encode_ty` → `resolve::decode_ty`) dropped it: `encode_ty` emitted only
           the `(base NAME EXP)` dimension triples, so decode rebuilt the unit at scale 1/1 (the reference).
           A scaled-unit param then silently became its REFERENCE unit, and `(+ a b)` with `a : Qty km`,
           `b : Qty m` saw EQUAL scales → added the RAW magnitudes with no conversion (a silent miscompile:
           `f(2, 500)` gave 502, not 2500). The fix encodes a `(scale NUM DEN)` item for a non-1/1 unit and
           decode restores it. Now `2 km + 500 m` = 2500 m (2 km converts to 2000 m, + 500). CONTRAST the
           CONSTANT/let-bound forms, which never lost the scale (no encode round-trip) — this pins the
           PARAMETER path. `cdz check` passed the mis-scaled program, so this is a check-vs-run pin: it must
           RUN to the converted magnitude, not a raw-added one.")
  (input  (do
            (def (main (: a (Qty Int64 (Unit.prefix kilo (Unit.base #"meter"))))
                       (: b (Qty Int64 (Unit.base #"meter"))))
              (Qty.value (+ a b)))
            (export main)))
  (call   main (: 2 Int64) (: 500 Int64)) (output (: 2500 Int64)))

; A mixed-scale combine converts each operand's magnitude to the reference — and that magnitude may come
; from a COMPUTED quantity (a `*`/`/`-derived one, a let-bound one), not only a directly-written `Qty.of`.
; The converter reads the magnitude via `qty_magnitude_occ`: a literal `Qty.of`'s value occurrence, else
; `(Qty.value operand)` (the explicit unwrap). Previously the converters used the literal-only occurrence
; and DECLINED a computed operand ('runtime mixed-unit … combine over a non-Qty.of operand not yet
; emitted'); these pin that a computed operand now combines across scales.

(case "a runtime mixed-scale combine converts a COMPUTED BigInt quantity operand"
  (doc    "`(+ (* (Qty.of (BigInt.of n) kilometer) (BigInt.of 2)) (Qty.of (BigInt.of 500) meter))` — the
           LEFT operand is a COMPUTED quantity (a scaled `n km × 2`), not a literal `Qty.of`, combined
           across scales with a meter quantity. Its magnitude is recovered via `(Qty.value …)` (the
           qty_magnitude_occ fallback), converted to the reference (×1000), and added: n=3 → 3·1000·2 + 500
           = 6500. Pins that a runtime mixed-scale combine works over a computed operand, not only a
           literal `Qty.of` (it previously declined 'non-Qty.of operand not yet emitted').")
  (input  (do
            (def (main (: n Int64))
              (Qty.value (+ (* (Qty.of (BigInt.of n) (Unit.prefix kilo (Unit.base #"meter"))) (BigInt.of 2))
                            (Qty.of (BigInt.of 500) (Unit.base #"meter")))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6500 BigInt))
  (live-objects known-leak))

(case "a runtime mixed-scale combine converts a COMPUTED Float quantity operand"
  (doc    "The Float companion: `(+ (* (Qty.of n kilometer) 2.0) (Qty.of 500.0 meter))` with a runtime
           Float64 `n` — the computed left operand's magnitude is recovered via `(Qty.value …)`, scaled to
           the reference meter (×1000), and added. n=3.0 → 3.0·1000·2.0 + 500.0 = 6500.0. Pins the Float
           arm of the computed-operand fallback (the same qty_magnitude_occ path the BigInt/Int/Rational
           arms use).")
  (input  (do
            (def (main (: n Float64))
              (Qty.value (+ (* (Qty.of n (Unit.prefix kilo (Unit.base #"meter"))) 2.0)
                            (Qty.of 500.0 (Unit.base #"meter")))))
            (export main)))
  (call   main (: 3.0 Float64)) (output (: 6500.0 Float64)))

(case "a runtime MIXED-SCALE BigInt comparison converts to the reference before comparing"
  (doc    "`(< (Qty.of (BigInt.of v) kilometer) (Qty.of (BigInt.of 5000) meter))` — a mixed-scale BigInt
           COMPARISON: v km converts to meters (×1000) before comparing. v=2 → 2000 m < 5000 m → true (1);
           v=6 → 6000 m < 5000 m → false (0). Pins that a mixed-scale BigInt comparison converts through
           the bigint conversion + `bigint-cmp`, not the declining fixnum path.")
  (input  (do
            (def (main (: v Int64))
              (if (< (Qty.of (BigInt.of v) (Unit.prefix kilo (Unit.base #"meter")))
                     (Qty.of (BigInt.of 5000) (Unit.base #"meter"))) 1 0))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1 Int64))
  (call   main (: 6 Int64)) (output (: 0 Int64)))

(case "a runtime MIXED-SCALE Rational-magnitude sum converts to the reference EXACTLY"
  (doc    "`(+ (Qty.of (Rational.of v 1) kilometer) (Qty.of (Rational.of 500 1) meter))` with a RUNTIME
           `v`: a mixed-scale combine (km + m) over a Rational inner converts each operand to the
           reference `meter` by an EXACT rational multiply (v km × 1000/1) then adds — v=2 → 2500/1. Pins
           the runtime Rational arm of `lower_quantity_combine` (it previously declined 'runtime mixed-unit
           Rational combine not yet emitted', folding only a constant pair). The Rational companion of the
           runtime mixed-scale BigInt/Int sums; exact, no rounding.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of (Rational.of v 1) (Unit.prefix kilo (Unit.base #"meter")))
                            (Qty.of (Rational.of 500 1) (Unit.base #"meter")))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2500/1 Rational))
  (live-objects known-leak))

(case "a runtime exact mixing of inch and millimeter keeps the fractional scale"
  (doc    "THE exact-mixing case at RUNTIME: `(+ (Qty.of (Rational.of v 1) inch) (Qty.of (Rational.of 1 1)
           millimeter))` — v inch + 1 mm — converts each to the reference meter by its exact fractional
           scale (inch = 127/5000, mm = 1/1000) and adds EXACTLY. v=1 → 127/5000 + 1/1000 = 132/5000 =
           33/1250 m, no rounding. Pins that the runtime Rational mixed-scale combine keeps a FRACTIONAL
           scale exact (the whole reason exact rationals are load-bearing for units) — the runtime analogue
           of the constant `1 inch + 1 mm` mixing case.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (+ (Qty.of (Rational.of v 1) (Unit.of #"inch"))
                            (Qty.of (Rational.of 1 1) (Unit.of #"millimeter")))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 33/1250 Rational))
  (live-objects known-leak))

(case "a runtime Unit.in conversion emits the scale multiply (Int)"
  (doc    "`(Unit.in meter (Qty.of v kilometer))` with `v` a runtime Int64: converts v km to meters by
           *1000 at run time, so v=3 → 3000 m. The explicit-conversion companion of the runtime mixed
           sum.")
  (input  (do
            (def (main (: v Int64))
              (Unit.in (Unit.of #"meter") (Qty.of v (Unit.of #"kilometer"))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 3000 Int64)))

(case "Unit.in over a BigInt-magnitude quantity converts in unbounded arithmetic and unwraps"
  (doc    "`(Unit.in meter (Qty.of (BigInt.of v) kilometer))` — an explicit conversion of a `(Qty BigInt
           kilometer)` — converts `value * 1000` in UNBOUNDED bigint arithmetic (the scale factors are
           materialized as `BigInt.of` so the `*`/`/` run the runtime bigint ops), and UNWRAPS to the bare
           BigInt count. v=3 → 3000; v=10^12 → 10^15 (beyond Int64, the point of BigInt). Pins the BigInt
           arm of `lower_unit_in` (it previously declined — 'ownership cannot prove' — having only
           float/rational arms).")
  (input  (do
            (def (main (: v Int64))
              (Unit.in (Unit.of #"meter") (Qty.of (BigInt.of v) (Unit.of #"kilometer"))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3000 BigInt))
  (call   main (: 1000000000000 Int64)) (output (: 1000000000000000 BigInt))
  (live-objects known-leak))

(case "Unit.in over a BigInt quantity truncates a non-dividing ratio"
  (doc    "`(Unit.in kilometer (Qty.of (BigInt.of v) meter))` — 2500 m in km, 2500/1000 does not divide,
           so the bigint division TRUNCATES toward zero → 2 (BigInt). Pins that a BigInt-quantity
           conversion uses integer/bigint division semantics (the same opt-into-integer-math rule the
           fixed-Int Unit.in uses), not a promotion.")
  (input  (do
            (def (main (: v Int64))
              (Unit.in (Unit.of #"kilometer") (Qty.of (BigInt.of v) (Unit.of #"meter"))))
            (export main)))
  (call   main (: 2500 Int64)) (output (: 2 BigInt))
  (live-objects known-leak))

(case "Unit.in over a runtime Rational-magnitude quantity converts exactly and unwraps"
  (doc    "`(Unit.in meter (Qty.of (Rational.of v 1) kilometer))` with a RUNTIME `v`: converts the
           Rational magnitude to meters by the exact scale (×1000) and UNWRAPS to a bare Rational. v=3 →
           3000/1. Runs the runtime `rational-mul` on the erased handle (the value × `(Rational.of 1000
           1)`), NOT a fold. Pins the runtime Rational arm of `lower_unit_in` — it previously declined
           ('runtime Rational magnitude not yet emitted'), having only a constant-fold path. The Rational
           companion of the runtime BigInt Unit.in.")
  (input  (do
            (def (main (: v Int64))
              (Unit.in (Unit.of #"meter") (Qty.of (Rational.of v 1) (Unit.of #"kilometer"))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3000/1 Rational))
  (live-objects known-leak))

(case "Unit.in over a runtime Rational quantity keeps a fractional scale exact"
  (doc    "`(Unit.in meter (Qty.of (Rational.of v 1) inch))` with a runtime `v`: inch = 127/5000 m, so
           v=1 → 127/5000 EXACTLY — the rational multiply keeps the fractional scale with no rounding
           (unlike the Int/BigInt arms, which truncate a non-whole ratio). Pins that a runtime Rational
           conversion is exact even when the scale is fractional, the whole point of a Rational magnitude.")
  (input  (do
            (def (main (: v Int64))
              (Unit.in (Unit.of #"meter") (Qty.of (Rational.of v 1) (Unit.of #"inch"))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 127/5000 Rational))
  (live-objects known-leak))

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
  (input  (Unit.in (Unit.of #"byte-per-second") (Qty.of 1.0 (Unit.of #"mbps"))))
  (output (: 125000.0 Float64)))

(case "a rate derived by division mixes with a named rate unit of the same dimension"
  (doc    "`(bytes / seconds)` derives the dimension `byte/second` — the SAME dimension `mbps` names — so
           a computed rate and an `mbps` quantity combine and convert: (250000 byte / 1 s) + 1 mbps =
           250000 + 125000 = 375000 byte/s. Pins that a NAMED derived-dimension unit and a DERIVED-by-
           arithmetic dimension are one free-abelian-group element, mixing and converting freely.")
  (input  (Unit.in (Unit.of #"byte-per-second")
                       (+ (/ (Qty.of 250000.0 (Unit.of #"byte")) (Qty.of 1.0 (Unit.of #"second")))
                          (Qty.of 1.0 (Unit.of #"mbps")))))
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
            (def (main) (Unit.in (Unit.of #"meter") (Qty.of 1.0 (Unit.of #"furlong"))))
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

(case "an inline Unit.define conflicting with a built-in conversion is rejected wherever it occurs"
  (doc    "The name->conversion-uniqueness rule (#A Named Unit's Conversion Is Unique) holds wherever a
           `Unit.define` occurs, not only at the TOP LEVEL: an INLINE `(Unit.define #\"foot\" (Unit.base
           #\"meter\") 2 1)` in a `Qty.of` unit position redeclares the built-in `foot` (381/1250 m) as 2 m
           — a conflicting conversion — so it is CDZ0502, exactly like the top-level form. Before the scan
           walked every arena node (not just top items), an inline define BYPASSED the uniqueness table: it
           silently evaluated at the fake 2 m ratio (`(+ (Qty.of 1.0 fake-foot) (Qty.of 1.0 meter))` ran to
           3.0 instead of rejecting), the classic silent-wrong-physics the layer exists to prevent.")
  (input  (do
            (def (main (: a Float64))
              (Qty.value (+ (Qty.of a (Unit.define #"foot" (Unit.base #"meter") 2 1))
                            (Qty.of a (Unit.base #"meter")))))
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
            (def (main) (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"foot"))))
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
            (def (main) (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"foot"))))
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
            (def (main) (Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"span"))))
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

; --- Unwrap edges: runtime magnitude, the truncation promise, identity, and re-entry ---------------
; The Q3 unwrap cases above pin the headline (a bare number exits the units world) over CONSTANT
; magnitudes and exact ratios. These pin the edges: a runtime magnitude through the scale multiply,
; the truncating non-dividing ratio the exact-Int case's doc PROMISES but never grades, the identity
; conversion (scale 1 — a rewrite that special-cases same-unit must still unwrap), and re-entering
; the units world by wrapping an unwrapped number.

(case "a runtime magnitude unwraps through a unit conversion"
  (doc    "`(Unit.in inch (Qty.of n foot))` with n a boundary PARAMETER: the constant cases fold at
           compile time; this exercises the EMITTED scale multiply (×12) followed by the unwrap on a
           genuinely runtime magnitude. n = 2 → the bare Int64 24. Pins that the unwrap semantics and
           the runtime conversion path agree with the folded one (const-vs-runtime discipline).")
  (input  (do
            (def (main (: n Int64))
              (Unit.in (Unit.of #"inch") (Qty.of n (Unit.of #"foot"))))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 24 Int64)))

(case "a non-dividing conversion ratio truncates on an Int magnitude"
  (doc    "`(Unit.in kilometer (Qty.of 2500 meter))` — 2500/1000 does not divide, and the magnitude
           type is Int64, so the conversion TRUNCATES toward zero → the bare 2. The exact-Int case's
           doc promises this ('a non-dividing ratio truncates — opting into integer math'); this
           GRADES it. An Int magnitude means integer division semantics (the same `/` the numeric
           model pins), not a silent promotion to Float or Rational and not a trap: choosing Int64 as
           the magnitude type is the opt-in.")
  (input  (Unit.in (Unit.of #"kilometer") (Qty.of 2500 (Unit.of #"meter"))))
  (output (: 2 Int64)))

(case "an identity conversion unwraps the magnitude unchanged"
  (doc    "`(Unit.in meter (Qty.of 5 meter))` — source and target are the SAME unit, scale ratio 1:
           the conversion is the identity on the magnitude and the unwrap yields the bare 5. Pins the
           degenerate ratio (an emit that skips the multiply entirely must still UNWRAP — returning
           the quantity unchanged would leak a `(Qty Int64 meter)` where a bare Int64 is promised).")
  (input  (Unit.in (Unit.of #"meter") (Qty.of 5 (Unit.of #"meter"))))
  (output (: 5 Int64)))

(case "wrapping an unwrapped number re-enters the units world at the new unit"
  (doc    "The unwrap-then-rewrap round trip: `(Unit.in meter (Qty.of 3 kilometer))` exits with the
           bare 3000; `(Qty.of 3000 foot)` re-enters as three thousand FEET (the bare number carries
           no memory of having been meters — re-attachment is purely nominal); `(Unit.in foot …)` of
           that quantity unwraps 3000 unchanged (identity conversion). Pins that unwrap genuinely
           erases the unit: the re-entered quantity answers to its NEW unit only, with no residual
           meter scale applied anywhere in the chain.")
  (input  (Unit.in (Unit.of #"foot")
            (Qty.of (Unit.in (Unit.of #"meter") (Qty.of 3 (Unit.of #"kilometer")))
                    (Unit.of #"foot"))))
  (output (: 3000 Int64)))

; --- Bare-number scaling neighbors: operand side, division, and the squared-dimension guard --------
; The apply_type-reorder cases above pin `qty × bare` keeping its dimension for both magnitude
; types. These pin the neighbors of the same arm-ordering hazard: the bare number on the LEFT (the
; commuted form goes through the same operand-type arms in the other order), DIVISION by a bare
; number (the other multiplicative op the `is_multiplicative && any_qty` gate covers), and the
; dimension-COMPOSITION guard (qty × qty is a new dimension that must not flow into a linear add).

(case "a bare number on the left scales the quantity and keeps its dimension"
  (doc    "`(* 3 (Qty.of 2 kilometer))` — the commuted form: the BARE operand is examined first, so an
           operand-type arm keyed on 'left is a bare Int' fires before any quantity check unless gated
           on any_qty. The product is 6 km; `(Unit.in meter …)` unwraps → 6000. The left-operand
           companion of the landed qty-on-the-left cases.")
  (input  (Unit.in (Unit.of #"meter") (* 3 (Qty.of 2 (Unit.of #"kilometer")))))
  (output (: 6000 Int64)))

(case "dividing a quantity by a bare number keeps its dimension"
  (doc    "`(/ (Qty.of 6 kilometer) 3)` = 2 km — division is the other multiplicative op the
           quantity path covers (a qty ÷ bare scales the magnitude down, dimension unchanged).
           `(Unit.in meter …)` → 2000. A reorder regression on the divide arm would drop the unit
           exactly as the multiply arm did.")
  (input  (Unit.in (Unit.of #"meter") (/ (Qty.of 6 (Unit.of #"kilometer")) 3)))
  (output (: 2000 Int64)))

(case "the magnitude of a bare-scaled quantity reads back"
  (doc    "`(Qty.value (* (Qty.of 2 kilometer) 3))` = 6 — the exact expression shape the apply_type
           bug DECLINED ('function return type has no machine representation': the product mis-typed
           as bare Float/Int, so Qty.value had no quantity to unwrap). Pins the observation op over a
           scaled quantity end to end.")
  (input  (Qty.value (* (Qty.of 2 (Unit.of #"kilometer")) 3)))
  (output (: 6 Int64)))

(case "a squared quantity does not add to a linear quantity"
  (doc    "`(* (Qty.of 2 meter) (Qty.of 3 meter))` composes dimensions (m²) — the qty×qty arm, NOT
           the bare-scaling arm. Adding the m² product to a linear `(Qty.of 1 meter)` is a
           cross-dimension add → CDZ0501. Pins the two multiplicative paths stay distinct: a reorder
           that sent qty×qty through the bare-operand arm would type the product linear and wrongly
           ACCEPT this add.")
  (input  (+ (* (Qty.of 2 (Unit.of #"meter")) (Qty.of 3 (Unit.of #"meter"))) (Qty.of 1 (Unit.of #"meter"))))
  (error  CDZ0501))

; --- BigInt-inner quantity dispatch: the neighbor sites -------------------------------------------
; 36ed35673 routes a `(Qty BigInt u)`'s +,-,*,/ to the bigint path (a BigInt inner is a heap HANDLE;
; the fixnum path emitted an i64 where the i32 handle was expected — invalid wasm). Each dispatch
; site is a separate predicate, so each neighbor is a separately-missable face. These pin two that
; work today (bare-BigInt scaling, unbounded growth) and the exactness control.

(case "a BigInt-inner quantity scales by a bare BigInt on the bigint path"
  (doc    "`(* (Qty.of (BigInt.of 5) meter) (BigInt.of 3))` — a BigInt-inner quantity times a BARE
           BigInt: the bare-scaling arm must route to the bigint multiply exactly as the qty+qty add
           does (the fix's case). `Qty.value` reads back 15 : BigInt. A bare-scaling arm keyed on the
           fixnum default emits the i64/i32 mismatch on the handle operand.")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (* (Qty.of (BigInt.of v) (Unit.of #"meter")) (BigInt.of 3))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 15 BigInt))
  (live-objects known-leak))

(case "a BigInt-inner quantity grows past Int64.max without trapping"
  (doc    "`(* (Qty.of (BigInt.of Int64.max) meter) (BigInt.of 2))` = 18446744073709551614 : BigInt —
           the whole point of a BigInt magnitude: the product exceeds every fixnum and must GROW, not
           trap. A dispatch that reached the checked fixnum multiply would trap 'integer overflow'
           here; the bigint path is unbounded (numeric-model.md #An Arbitrary-Precision Integer Has
           Unbounded Range, composed through the quantity wrapper).")
  (input  (do
            (def (main (: v Int64))
              (Qty.value (* (Qty.of (BigInt.of 9223372036854775807) (Unit.of #"meter"))
                            (BigInt.of 2))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 18446744073709551614 BigInt))
  (live-objects known-leak))

(case "a BigInt-inner quantity ADDITION past Int64.max stays exact through the wrapper"
  (doc    "The additive companion of the multiplicative grow-past-Int64.max case above: `(+ (Qty.of
           (BigInt.of Int64.max) meter) (Qty.of (BigInt.of Int64.max) meter))` = 2·(2^63-1) = 2^64-2, a
           magnitude past every fixnum. `Qty.value` reads back the exact `18446744073709551614 : BigInt`.
           Pins that a same-unit `+` over two BigInt-inner quantities runs the unbounded bigint add through
           the quantity wrapper (not a checked fixnum add that would trap), exactly as the `*` case does.")
  (input  (do
            (def (main (: n Int64))
              (= (Qty.value (+ (Qty.of (BigInt.of n) (Unit.base #"meter"))
                               (Qty.of (BigInt.of n) (Unit.base #"meter"))))
                 (: 18446744073709551614 BigInt)))
            (export main)))
  (call   main (: 9223372036854775807 Int64))
  (output (: true Bool)))

(case "a squared BigInt-inner quantity added to a linear one is a dimension mismatch"
  (doc    "The same-base power dimension distinction, through a BigInt inner: `(+ (Qty.pow (Qty.of (BigInt.of
           n) meter) 2) (Qty.of (BigInt.of n) meter))` adds meter² (from the pow) to a plain meter — DIFFERENT
           dimensions (exponent 2 vs 1) — so it rejects CDZ0501, exactly as the Float64-magnitude power-dim
           cases do. Pins that the dimension check composes the exponent correctly OVER a BigInt magnitude
           (the dimension is on the unit, independent of the inner numeric type), not just over Float64 —
           the intersection of the power-dimension distinction and the BigInt-inner quantity.")
  (input  (do
            (def (main (: n Int64))
              (Qty.value (+ (Qty.pow (Qty.of (BigInt.of n) (Unit.base #"meter")) 2)
                            (Qty.of (BigInt.of n) (Unit.base #"meter")))))
            (export main)))
  (call   main (: 5 Int64))
  (error  CDZ0501))

(case "a Rational-inner quantity addition stays exact through the quantity wrapper"
  (doc    "`(+ (Qty.of 1/3 meter) (Qty.of 1/6 meter))` → `Qty.value` reads back 1/2 : Rational — the
           pre-existing `quantity_inner_is_rational` predicate's happy path, pinned as the control
           beside the new bigint arm: each inner-type predicate routes to ITS arithmetic (exact
           rational here), and adding the bigint arm must not have perturbed the rational dispatch.")
  (input  (Qty.value (+ (Qty.of (Rational.of 1 3) (Unit.of #"meter"))
                        (Qty.of (Rational.of 1 6) (Unit.of #"meter")))))
  (output (: 1/2 Rational)))

; --- narrow-width Int-inner quantity arithmetic obeys the inner type's overflow rule ---------------
; A quantity's arithmetic runs the ERASED inner numeric type's operation, so a `(Qty Int8 u)` + / * /
; must overflow-trap exactly as a bare Int8 does — a compile-provable narrow overflow is CDZ0304 (a
; constant OPERATION with no value), the SAME code the bare `(+ (Int8.of 100) (Int8.of 100))` gets, NOT
; the backend CDZ0302 "literal does not fit its width" (100 fits Int8; it is the SUM 200 that overflows).
; The width-check reads the quantity's INNER Int type (`Ty::Qty { inner: Int … }`): the same-unit case
; falls through to the generic arith path (equal scales → not a mixed-unit combine), the mixed-scale case
; folds inside the reference-converting combine — both peel the quantity to read the inner width. A
; non-overflowing narrow-width quantity arithmetic runs normally (the control).

(case "a narrow-width Int quantity add that overflows the inner width traps at compile time"
  (doc    "`(+ (Qty.of (Int8.of 100) meter) (Qty.of (Int8.of 100) meter))` — a same-unit add over an
           Int8 inner: 100 + 100 = 200 overflows Int8 (max 127). Units are erased, so the arithmetic
           obeys the inner Int8 type's overflow rule: a compile-provable overflow is CDZ0304 (a constant
           operation with no value), the SAME code the bare `(+ (Int8.of 100) (Int8.of 100))` gets — NOT
           CDZ0302 (each 100 literal FITS Int8; it is the sum that overflows). The width-check reads the
           quantity's INNER Int8 type; without peeling the `Ty::Qty` the overflow slipped through to a
           backend CDZ0302 that `cdz check` never saw.")
  (input  (Qty.value (+ (Qty.of (Int8.of 100) (Unit.base #"meter"))
                        (Qty.of (Int8.of 100) (Unit.base #"meter")))))
  (error  CDZ0304))

(case "a RUNTIME erased Qty add is still CHECKED at the inner width (overflow traps at run time)"
  (doc    "The RUNTIME twin of the compile-time CDZ0304 quantity-overflow pins around it, closing the
           'constant and runtime paths agree' claim (§overflow policy) with an actual runtime witness:
           `(+ q q)` over a runtime-magnitude `(Qty Int64 meter)` erases to a bare Int64 add, and that
           erased add must still be the CHECKED `+` — at v = 2^62 the sum 2^63 overflows Int64 and TRAPS
           'integer overflow' (a backend that emitted a wrapping add for the erased quantity would return
           Int64.min silently). v = 21 → 42, the in-range control. Pins that unit ERASURE does not erase
           the inner type's overflow discipline on the runtime path.")
  (input  (do
            (def (main (: v Int64))
              (let ((q (Qty.of v (Unit.base #"meter"))))
                (Qty.value (+ q q))))
            (export main)))
  (call   main (: 4611686018427387904 Int64)) (trap "integer overflow")
  (call   main (: 21 Int64)) (output (: 42 Int64)))

(case "a narrow-width Int quantity add that fits the inner width runs normally"
  (doc    "`(+ (Qty.of (Int8.of 50) meter) (Qty.of (Int8.of 50) meter))` = 100, which FITS Int8 — the
           control beside the overflowing case: a narrow-width quantity arithmetic whose result is in
           range runs exactly as the bare Int8 add does, no spurious trap. Pins that the inner-width
           overflow check rejects ONLY a genuine overflow.")
  (input  (Qty.value (+ (Qty.of (Int8.of 50) (Unit.base #"meter"))
                        (Qty.of (Int8.of 50) (Unit.base #"meter")))))
  (output (: 100 Int8)))

(case "a mixed-scale narrow-width Int quantity combine that overflows the inner width traps"
  (doc    "`(+ (Qty.of (UInt8.of 1) kilometer) (Qty.of (UInt8.of 50) meter))` — a MIXED-scale combine
           over a UInt8 inner: 1 km converts to 1000 m at the reference, then 1000 + 50 = 1050 overflows
           UInt8 (max 255). The reference-converting combine folds the result and range-checks it against
           the inner UInt8 width — a compile-provable overflow is CDZ0304, exactly as the same-unit case
           above. Pins that the mixed-scale (reference-converting) path honors the inner width too, not
           only the same-unit generic-arith path.")
  (input  (Qty.value (+ (Qty.of (UInt8.of 1) (Unit.prefix kilo (Unit.base #"meter")))
                        (Qty.of (UInt8.of 50) (Unit.base #"meter")))))
  (error  CDZ0304))

(case "a narrow-width Int quantity stored as a map value round-trips through Map.lookup"
  (doc    "A `(Qty Int8 meter)` stored as a MAP VALUE, read back via `Map.lookup` (→ `Option`), and let the
           retrieved quantity ESCAPE the Option match AS A QTY (bound, then `Qty.value`-unwrapped OUTSIDE the
           arm). A quantity over a NARROW int erases to its inner narrow int's i32 machine slot, but the heap
           boxes/reads an integer through an i64 cell (`box-int`/`get-int`), so a narrow value needs an
           i32→i64 EXTEND before `box-int` and an i64→i32 NARROW after `get-int`. Both `is_narrow_int` (the
           extend/narrow decision) and `int_ty_of` (the `ConstI32`-vs-`ConstI64` literal-width decision) read
           the node's solved type and MUST peel `Ty::Qty` to see the narrow inner — WITHOUT the peel a
           `(Qty Int8 u)` map value mis-lowered: the magnitude emitted as an i64 constant while the read
           applied the i64→i32 narrow (and vice-versa), leaving an i64 where the i32 narrow-int slot was
           expected → an INVALID module (`expected i32, found i64`) that `cdz check` did NOT catch. Here the
           stored `100 meter` reads back and unwraps to 100. Pins the narrow-Qty heap value-decode round-trip.")
  (input  (do
            (def (main)
              (Qty.value
                (match (Map.lookup (Map.insert (Map.empty) 1 (Qty.of (Int8.of 100) (Unit.base #"meter"))) 1)
                  ((Some q) q)
                  ((None) (Qty.of (Int8.of 0) (Unit.base #"meter"))))))
            (export main)))
  (output (: 100 Int8))
  (live-objects 0))

(case "Qty map VALUES unwrap, ADD, and COMPARE in one chain preserving the unit through the collection"
  (doc    "The WORKING-chain face of quantities in collections (the case above round-trips ONE value):
           two lookups Option-unwrap through a helper whose miss face returns the 0m identity, the
           same-unit Qty ADD runs over COLLECTION-sourced operands, and the Qty COMPARE against a
           threshold exercises the scalar-Int Ty::Qty-peeling arm on exactly the map-sourced operand
           shape its doc names — then Qty.value out. k=2: 10m+25m = 35m > 30m → 351; k=9: miss →
           10m+0m = 10m, not > 30m → 100.")
  (input  (do
            (def (getq (: m (Map Int64 (Qty Int64 (Unit.base #"meter")))) (: k Int64))
              (match (Map.lookup m k)
                ((Some q) q)
                ((None _u) (Qty.of 0 (Unit.base #"meter")))))
            (def (main (: k Int64))
              (do
                (def m (Map.insert (Map.insert Map.empty 1 (Qty.of 10 (Unit.base #"meter"))) 2 (Qty.of 25 (Unit.base #"meter"))))
                (def s (+ (getq m 1) (getq m k)))
                (+ (* (Qty.value s) 10)
                   (if (> s (Qty.of 30 (Unit.base #"meter"))) 1 0))))
            (export main)))
  (call main (: 2 Int64)) (output (: 351 Int64))
  (call main (: 9 Int64)) (output (: 100 Int64))
  (live-objects 0))

(case "a tuple with a Qty leaf as a map key hits by magnitude-and-unit content"
  (doc    "The COMPOUND-key face (the bare-Qty-key cluster below pins Int/BigInt/Rational inners
           directly as keys): a Qty INSIDE a tuple key — the CHAMP compound descent must erase the
           Qty to its inner numeric at the LEAF position, completing the tuple-leaf-kind matrix
           (float/BigInt/Rational/Symbol leaves are pinned in 05-compound/17-symbols; this is the
           Qty leaf). Keyed at runtime v, hit by a rebuilt (tuple 10m 3) at v=10, miss at v=11.")
  (input  (do
            (def (main (: v Int64))
              (do
                (def m (Map.insert Map.empty #tuple((Qty.of v (Unit.base #"meter")) 3) 42))
                (match (Map.lookup m #tuple((Qty.of 10 (Unit.base #"meter")) 3))
                  ((Some x) x)
                  ((None _u) -1))))
            (export main)))
  (call main (: 10 Int64)) (output (: 42 Int64))
  (call main (: 11 Int64)) (output (: -1 Int64)))

(case "a quantity read from a map COMBINES with a fresh same-dimension quantity"
  (doc    "The arithmetic face of the Qty-in-collection round-trip (the round-trip pins above read and
           UNWRAP; this one reads and COMPUTES): a runtime-magnitude `(Qty.of n meter)` stored as a map
           value, looked up, and ADDED to a fresh `(Qty.of 5 meter)` inside the Some arm — the looked-up
           quantity must carry its dimension through the heap round-trip so the homogeneous `+` type-checks
           and computes 10+5 = 15. A decode that returned a bare magnitude (dimension dropped) would either
           reject the `+` or mis-scale. The collection-read companion of the direct Qty arithmetic pins.")
  (input  (do
            (def (main (: n Int64))
              (let ((m (Map.insert Map.empty 1 (Qty.of n (Unit.base #"meter")))))
                (Qty.value (match (Map.lookup m 1)
                  ((Some q) (+ q (Qty.of 5 (Unit.base #"meter"))))
                  ((None u) (Qty.of 0 (Unit.base #"meter")))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 15 Int64))
  (live-objects 0))

(case "a Float32 quantity stored as a map value round-trips through Map.lookup"
  (doc    "The Float32 analogue of the narrow-Int map-value case. A `(Qty Float32 meter)` stored as a MAP
           VALUE, read back via `Map.lookup`, and unwrapped. A quantity over a Float32 erases to its inner
           f32 machine slot (distinct from the f64 default), boxed/read through `box-float32`/`get-float32`.
           The `ConstFloat`/`ConstFloatNan` emit reads the node's solved type to pick `f32.const` vs
           `f64.const` and MUST peel `Ty::Qty` — WITHOUT the peel a `(Qty Float32)` magnitude emitted an
           `f64.const` while the box op is `box-float32` (f32) → an INVALID module (`expected f32, found
           f64`) that `cdz check` did NOT catch; the rust backend emitted `f64::from_bits` into an `f32` map
           slot (E0308). The float twin of the narrow-Int `int_ty_of` peel, fixed on BOTH backends. The
           stored `2.5 meter` reads back and unwraps to 2.5.")
  (input  (do
            (def (main)
              (Qty.value
                (match (Map.lookup (Map.insert (Map.empty) 1 (Qty.of (Float32.of 2.5) (Unit.base #"meter"))) 1)
                  ((Some q) q)
                  ((None) (Qty.of (Float32.of 0.0) (Unit.base #"meter"))))))
            (export main)))
  (output (: 2.5 Float32))
  (live-objects 0))

(case "a nominal newtype over a Float32 quantity stored as a map value round-trips"
  (doc    "The nominal-newtype layer over the Float32-quantity map case. `(type Len (Q (Qty Float32 meter)))`
           — an erased single-variant newtype over a `(Qty Float32 meter)` — stored as a MAP VALUE. The
           newtype erases to the SAME f32 slot as the inner Float32 quantity, so the `ConstFloat` width
           reader must STRIP the nominal wrapper AND peel `Ty::Qty` to reach the inner Float32. WITHOUT the
           outer strip, `peel_qty_ty` saw `Nominal(Len, Qty{Float32})`, missed the `Ty::Qty` arm, and fell
           to the f64 default → an `f64.const` where `box-float32` wanted f32 → INVALID wasm (`expected f32,
           found f64`); the rust backend's `float_width_of` had the same gap (v-rust-backend's twin). The
           reader now does strip_nominal → peel Ty::Qty → strip_nominal (the strip_nominal lockstep the
           integer `int_ty_of` already maintains). Constructs a `Len` wrapping `2.5 m`, stores + looks it up,
           returns 1 on the hit.")
  (input  (do
            (type Len (Q (Qty Float32 (Unit.base #"meter"))))
            (def (main)
              (match (Map.lookup (Map.insert (Map.empty) 1 (Len.Q (Qty.of (Float32.of 2.5) (Unit.base #"meter")))) 1)
                ((Some _) 1)
                ((None) 0)))
            (export main)))
  (output (: 1 Int64))
  (live-objects 0))

; --- Quantity as a Map KEY: content-address equality over the erased magnitude + unit --------------
; The map cases above store a quantity as a map VALUE (the decode/read-back path). These pin the
; complementary KEY path: a runtime quantity used as a Map key must hash + compare by its CONTENT
; (the erased magnitude in its slot, plus the scaled unit), so a key rebuilt independently from the
; SAME runtime magnitude HITS, and one built from a DIFFERENT magnitude MISSES. This exercises the
; value comparator/hasher on a quantity key — a distinct runtime path from the value-decode cases.

(case "a runtime quantity used as a Map key hits when a separately-built equal key is looked up"
  (doc    "Insert under key `(Qty.of v kilometer)` with `v` a runtime Int64, then look up a key built
           INDEPENDENTLY from the same `v` (`(Qty.of v kilometer)` again). Content-address equality over
           the quantity key — same erased magnitude in the same unit slot — so the lookup HITS and returns
           the stored value 42. Pins that a quantity is a first-class heap key, not only a heap value:
           the key hasher/comparator sees the erased magnitude and matches a separately-constructed equal
           quantity (mirrors the compound-key content-equality the Map corpus pins for tuples/lists).")
  (input  (do
            (def (main (: v Int64))
              (let ((k (Qty.of v (Unit.prefix kilo (Unit.base #"meter"))))
                    (m (Map.insert (Map.empty) k 42)))
                (match (Map.lookup m (Qty.of v (Unit.prefix kilo (Unit.base #"meter"))))
                  ((Some found) found)
                  ((None) 0))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a runtime quantity Map key MISSES when the looked-up magnitude differs"
  (doc    "The negative twin of the quantity-key hit: insert under `(Qty.of v kilometer)`, then look up
           `(Qty.of (+ v 1) kilometer)` — a DIFFERENT magnitude, hence a distinct key by content — so the
           lookup MISSES and the `None` arm returns 0. Pins that the quantity key comparator discriminates
           on the erased magnitude (a different number is a different key), so equality is real content
           equality, not a spurious always-hit.")
  (input  (do
            (def (main (: v Int64))
              (let ((k (Qty.of v (Unit.prefix kilo (Unit.base #"meter"))))
                    (m (Map.insert (Map.empty) k 42)))
                (match (Map.lookup m (Qty.of (+ v 1) (Unit.prefix kilo (Unit.base #"meter"))))
                  ((Some found) found)
                  ((None) 0))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 0 Int64)))

(case "a DERIVED-dimension (velocity) quantity used as a Map key hits by content"
  (doc    "The prior key cases use a BASE-dimension quantity key; this pins a DERIVED (quotient) dimension —
           a velocity `(/ (Qty.of v meter) (Qty.of 2 second))` = `v/2 m/s`. A derived-dimension quantity
           still erases to its inner scalar with a composite (quotient) unit, so it is a first-class heap key:
           insert under the velocity key, look up a key built INDEPENDENTLY from the same runtime `v` (a
           velocity of the same magnitude AND the same m/s dimension) — content equality HITS and returns 42.
           Confirms the key hasher/comparator handles a composite-unit quantity key exactly as a base-unit
           one (the unit is compile-time-only; the runtime key is the erased magnitude in a m/s-shaped slot).")
  (input  (do
            (def (main (: v Int64))
              (let ((k (/ (Qty.of v (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second"))))
                    (m (Map.insert (Map.empty) k 42)))
                (match (Map.lookup m (/ (Qty.of v (Unit.base #"meter")) (Qty.of 2 (Unit.base #"second"))))
                  ((Some found) found)
                  ((None) 0))))
            (export main)))
  (call   main (: 8 Int64))
  (output (: 42 Int64)))

; --- Quantity over a HEAP-NUMERIC inner (BigInt/Rational) as a Map key -----------------------------
; The fixnum Map-key cases above erase to an immediate scalar in the key. These pin the heap-numeric
; inner: a `(Qty BigInt u)` / `(Qty Rational u)` key erases to a heap HANDLE, so the map key comparator
; must compare the pointed-to bignum/rational by CONTENT (not the handle), and — for Rational — the
; construction-time canonicalization means equal VALUES with different spellings are the same key.

(case "a runtime BigInt-inner quantity used as a Map key hits, and a different magnitude misses"
  (doc    "`(Qty (BigInt.of v) meter)` as a Map key with `v` a runtime Int64: a BigInt inner erases to a
           heap handle, so the key comparator compares the pointed-to bignum by content. A separately-built
           equal key HITS (42); `(+ v 1)` MISSES (0). Pins that a heap-numeric quantity key compares by the
           bignum's value, not the handle identity — the same content equality the fixnum key cases pin,
           through the BigInt heap path.")
  (input  (do
            (def (main (: v Int64))
              (let ((k (Qty.of (BigInt.of v) (Unit.base #"meter")))
                    (m (Map.insert (Map.empty) k 42)))
                (+ (match (Map.lookup m (Qty.of (BigInt.of v) (Unit.base #"meter")))
                     ((Some found) found) ((None) 0))
                   (match (Map.lookup m (Qty.of (BigInt.of (+ v 1)) (Unit.base #"meter")))
                     ((Some found) found) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a runtime Rational-inner quantity used as a Map key hits, and a different magnitude misses"
  (doc    "The Rational twin of the BigInt-key case: `(Qty (Rational.of v 2) meter)` as a Map key. A Rational
           inner erases to a heap handle, so the key comparator compares the pointed-to rational by content.
           A separately-built `v/2` key HITS (42); `(v+1)/2` MISSES (0). Pins content equality of a
           Rational-inner quantity key through the rational heap path.")
  (input  (do
            (def (main (: v Int64))
              (let ((k (Qty.of (Rational.of v 2) (Unit.base #"meter")))
                    (m (Map.insert (Map.empty) k 42)))
                (+ (match (Map.lookup m (Qty.of (Rational.of v 2) (Unit.base #"meter")))
                     ((Some found) found) ((None) 0))
                   (match (Map.lookup m (Qty.of (Rational.of (+ v 1) 2) (Unit.base #"meter")))
                     ((Some found) found) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a Rational-inner quantity Map key matches an equal value written with a different spelling"
  (doc    "Rational canonicalizes at construction, so a `(Qty (Rational.of v 2) meter)` key is looked up by
           `(Qty (Rational.of (* v 2) 4) meter)` — `v/2` vs `2v/4`, the SAME rational value in a different
           spelling — and HITS (42). Pins that a Rational-inner quantity key compares by the CANONICAL value,
           not the written numerator/denominator, so equality is real value equality across spellings.")
  (input  (do
            (def (main (: v Int64))
              (let ((k (Qty.of (Rational.of v 2) (Unit.base #"meter")))
                    (m (Map.insert (Map.empty) k 42)))
                (match (Map.lookup m (Qty.of (Rational.of (* v 2) 4) (Unit.base #"meter")))
                  ((Some found) found)
                  ((None) 0))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a trie of 30 QUANTITY keys resolves a cross-normalized magnitude lookup at depth"
  (doc    "The Qty-key rows above run on single-key maps; this pins a POPULATED trie of Rational-inner
           quantity keys: 30 keys `i/2 meter` fill the trie, and a lookup spelled `10/4` must hit the
           `5/2` slot — Rational normalization inside the Qty magnitude composing with the CHAMP descent
           at depth (·10 on len + the hit value → 305). A key path that normalized a bare Rational but
           read a Qty magnitude's spelling literally would miss among 30 neighbors.")
  (input  (do
            (def (fill (: i Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Qty.of (Rational.of i 2) (Unit.base #"meter")) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Qty.of (Rational.of 10 4) (Unit.base #"meter"))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 305 Int64)))

(case "a Qty-keyed trie churned with differently-normalized magnitudes equals the direct build"
  (doc    "The normalization-identity churn for quantity keys: 24 keys (i = 1..n-1 at n = 25) INSERTED as `2i/4 meter` and
           REMOVED as `i/2 meter` — differently-written spellings of the same magnitude — so every
           removal must land on its insert's slot through the canonical form inside the Qty. The
           surviving seed (stored `999/1`) must leave the map EQUAL to the direct build by canonical
           `=` (10) and resolve when probed as `1998/2` (+1 → 11). Three spellings per value across
           insert/remove/probe, all converging on one slot — the Qty face of the normalized-key churn
           family (the Rational twin is in 03-equality).")
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (Qty.of (Rational.of (* i 2) 4) (Unit.base #"meter")) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map (Qty Rational (Unit.base #"meter")) Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (Qty.of (Rational.of i 2) (Unit.base #"meter"))))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert Map.empty (Qty.of (Rational.of 999 1) (Unit.base #"meter")) 50))
                (def churned (shrink 1 n (grow 1 n direct)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned (Qty.of (Rational.of 1998 2) (Unit.base #"meter"))) ((Some v) (if (= v 50) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 11 Int64)))

; --- Quantity as a Set element: content-address dedup + membership --------------------------------
; The Map-key cases above pin a quantity on the key side of a Map. These pin the Set analogue: a
; quantity as a SET element is deduplicated + tested for membership by CONTENT (the erased magnitude
; in its unit slot), so two equal runtime quantities collapse to one element, distinct magnitudes
; stay separate, and `Set.contains` finds a separately-built equal quantity but not a non-member.
; Exercises the same value comparator/hasher on a quantity through the Set dedup + membership path.

(case "two equal runtime quantities in a Set dedup to one element"
  (doc    "`(Set.of (list (Qty.of v kilometer) (Qty.of v kilometer)))` with `v` a runtime Int64: both
           elements are the SAME quantity by content (same erased magnitude, same unit), so the Set
           deduplicates them to a single element and `Set.len` is 1. Pins a quantity as a first-class Set
           element whose identity is content, not object identity — the same content-address equality the
           Map-key cases pin, exercised through Set construction/dedup.")
  (input  (do
            (def (main (: v Int64))
              (Set.len #set((Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                                     (Qty.of v (Unit.prefix kilo (Unit.base #"meter"))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64)))

(case "distinct-magnitude runtime quantities in a Set stay separate"
  (doc    "The negative twin of the dedup case: `(Set.of (list (Qty.of v kilometer) (Qty.of (+ v 1)
           kilometer)))` holds two quantities with DIFFERENT magnitudes, hence distinct by content, so the
           Set keeps both and `Set.len` is 2. Pins that the element comparator discriminates on the erased
           magnitude (a different number is a different element), so dedup is real content equality.")
  (input  (do
            (def (main (: v Int64))
              (Set.len #set((Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                                     (Qty.of (+ v 1) (Unit.prefix kilo (Unit.base #"meter"))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 2 Int64)))

(case "Set.contains finds a separately-built equal quantity but not a non-member"
  (doc    "`Set.contains` over a Set of runtime quantities: a query quantity built INDEPENDENTLY from the
           same magnitude as a member HITS (content equality), while one built from a magnitude not in the
           Set MISSES. Here the Set holds `v km` and `(v+1) km`; querying `v km` (a separately-constructed
           equal quantity) returns 1, and `(v+2) km` (absent) returns 0. Pins the membership side of the
           quantity content-address comparator, complementing the dedup case above.")
  (input  (do
            (def (main (: v Int64))
              (let ((s #set((Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                                     (Qty.of (+ v 1) (Unit.prefix kilo (Unit.base #"meter"))))))
                (if (Set.contains s (Qty.of v (Unit.prefix kilo (Unit.base #"meter")))) 1 0)))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64)))

; --- Quantity as a List element: heap round-trip through List.at ----------------------------------
; The Map-value cases pin a quantity read back from a Map; these pin the List analogue — a quantity
; stored in a runtime List and read back via `List.at` (→ Option), then unwrapped. A quantity erases
; to its inner scalar, so a List of quantities boxes/reads each element through the heap list cells,
; and the retrieved quantity must decode back to the stored magnitude. Covers the general runtime
; path (a runtime-parameter magnitude) and the narrow-Int inner (the i32↔i64 box extend/narrow).

(case "a runtime quantity stored in a List reads back through List.at and unwraps"
  (doc    "`(List.at (list (Qty.of v km) (Qty.of (+ v 1) km)) 1)` with `v` a runtime Int64: the list holds
           two runtime quantities, `List.at 1` returns `(Some (Qty …))`, and `Qty.value` on the bound
           quantity reads the stored magnitude. v=5 → index 1 is `(v+1) km` → 6. Pins a quantity as a
           first-class List element that heap-round-trips (contrast the CONSTANT single-element list above,
           which folds); the magnitude is a runtime parameter so the list decode runs, not a constant fold.")
  (input  (do
            (def (main (: v Int64))
              (match (List.at #list((Qty.of v (Unit.prefix kilo (Unit.base #"meter")))
                                    (Qty.of (+ v 1) (Unit.prefix kilo (Unit.base #"meter")))) 1)
                ((Some q) (Qty.value q))
                ((None) 0)))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

(case "a narrow-Int quantity stored in a List reads back through List.at and unwraps"
  (doc    "The narrow-inner twin of the List round-trip: a `(Qty Int8 meter)` List element, read back via
           `List.at 0` and unwrapped. A quantity over a narrow int erases to its inner i32 machine slot, but
           the heap list boxes/reads an integer through an i64 cell, so a narrow value needs an i32→i64
           EXTEND before the box and an i64→i32 NARROW after the read — the same peel-`Ty::Qty` the map-value
           narrow case pins, exercised through the List element decode. The stored `100 meter` reads back and
           unwraps to 100.")
  (input  (do
            (def (main)
              (Qty.value
                (match (List.at #list((Qty.of (Int8.of 100) (Unit.base #"meter"))
                                      (Qty.of (Int8.of 50) (Unit.base #"meter"))) 0)
                  ((Some q) q)
                  ((None) (Qty.of (Int8.of 0) (Unit.base #"meter"))))))
            (export main)))
  (output (: 100 Int8)))

; --- Quantity over a HEAP-NUMERIC inner (BigInt/Rational) in a Set / List -------------------------
; The Set/List cases above erase to an immediate scalar element. These pin the heap-numeric inner
; through the SAME collection paths: a `(Qty BigInt u)` / `(Qty Rational u)` element erases to a heap
; HANDLE, so Set dedup + List decode must compare/copy the pointed-to bignum/rational by CONTENT (and,
; for Rational, the construction-time canonicalization normalizes the value). Mirrors the heap-numeric
; Map-key pins on the Set/List side.

(case "a BigInt-inner quantity Set dedups equal elements and keeps distinct ones"
  (doc    "`(Qty (BigInt.of v) meter)` as a Set element with `v` a runtime Int64. A BigInt inner erases to a
           heap handle, so Set dedup compares the pointed-to bignum by content: two equal elements collapse
           to `Set.len` 1, and a distinct magnitude keeps `Set.len` 2. Combined as `dedup + 10*distinct` =
           1 + 20 = 21. Pins content-address dedup for a heap-numeric quantity element.")
  (input  (do
            (def (main (: v Int64))
              (+ (Set.len #set((Qty.of (BigInt.of v) (Unit.base #"meter"))
                                        (Qty.of (BigInt.of v) (Unit.base #"meter"))))
                 (* 10 (Set.len #set((Qty.of (BigInt.of v) (Unit.base #"meter"))
                                              (Qty.of (BigInt.of (+ v 1)) (Unit.base #"meter")))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 21 Int64)))

(case "a Rational-inner quantity Set dedups equal elements and keeps distinct ones"
  (doc    "The Rational twin of the BigInt Set case: `(Qty (Rational.of v 2) meter)` elements. A Rational
           inner erases to a heap handle, so Set dedup compares the pointed-to rational by content: `v/2` and
           `v/2` collapse to `Set.len` 1, `v/2` and `(v+1)/2` stay `Set.len` 2. Combined `dedup + 10*distinct`
           = 21. Pins content-address dedup for a Rational-inner quantity element.")
  (input  (do
            (def (main (: v Int64))
              (+ (Set.len #set((Qty.of (Rational.of v 2) (Unit.base #"meter"))
                                        (Qty.of (Rational.of v 2) (Unit.base #"meter"))))
                 (* 10 (Set.len #set((Qty.of (Rational.of v 2) (Unit.base #"meter"))
                                              (Qty.of (Rational.of (+ v 1) 2) (Unit.base #"meter")))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 21 Int64)))

(case "a BigInt-inner quantity stored in a List reads back through List.at"
  (doc    "`(List.at (list (Qty (BigInt v) m) (Qty (BigInt v+1) m)) 1)` with `v` a runtime Int64: the list
           holds two BigInt-inner quantities (each a heap handle), `List.at 1` returns `(Some (Qty …))`, and
           `Qty.value` reads the stored bignum. v=5 → index 1 is `(v+1) m` → BigInt 6. Pins that a
           heap-numeric quantity List element round-trips through the list decode with its handle intact.")
  (input  (do
            (def (main (: v Int64))
              (match (List.at #list((Qty.of (BigInt.of v) (Unit.base #"meter"))
                                    (Qty.of (BigInt.of (+ v 1)) (Unit.base #"meter"))) 1)
                ((Some q) (Qty.value q))
                ((None) (BigInt.of 0))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 6 BigInt))
  (live-objects known-leak))

(case "a Rational-inner quantity stored in a List reads back canonicalized through List.at"
  (doc    "The Rational twin of the BigInt List case: `(List.at (list (Qty (Rational v 2) m) …) 0)`. A
           Rational inner erases to a heap handle, so the List element round-trips the rational by content,
           canonicalized at construction. v=5 → `List.at 0` unwraps to `5/2`. Pins a Rational-inner quantity
           List element decode (the canonical value survives the round-trip).")
  (input  (do
            (def (main (: v Int64))
              (match (List.at #list((Qty.of (Rational.of v 2) (Unit.base #"meter"))
                                    (Qty.of (Rational.of (+ v 1) 2) (Unit.base #"meter"))) 0)
                ((Some q) (Qty.value q))
                ((None) (Rational.of 0 1))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5/2 Rational))
  (live-objects known-leak))

; --- Quantity inside a COMPOUND key (list-of-Qty / tuple-of-Qty): key canonicalization -------------
; A list-typed or list-CONTAINING Map/Set key is CANONICALIZED at the key site (value-canonicalize into
; the correct CHAMP slot), which bakes the key type's shape descriptor. A quantity element erases to its
; inner scalar, so the shape builder must peel `Ty::Qty` to the inner — without the peel a compound key
; that CONTAINS a quantity (a `(List (Qty …))` / `(Tuple (Qty …) …)` key) DECLINED to compile
; ("list-key canonicalization: key type has no bakeable shape descriptor"). These pin the peel.

(case "a list-of-quantities used as a Map key canonicalizes and hits"
  (doc    "`(list (Qty v m) (Qty (+ v 1) m))` as a Map key: a list-CONTAINING key is canonicalized at the
           key site, which bakes the key's shape descriptor. The quantity elements erase to their inner
           scalars, so the shape builder peels `Ty::Qty` to the inner (a list-of-Qty hashes exactly as a
           list-of-Int). A separately-built equal list-of-Qty key HITS (42); one differing in a quantity
           element MISSES (0). Combined = 42. Pins that a quantity is a valid COMPOUND-key element (before
           the peel this DECLINED at compile time).")
  (input  (do
            (def (main (: v Int64))
              (let ((k #list((Qty.of v (Unit.base #"meter")) (Qty.of (+ v 1) (Unit.base #"meter"))))
                    (m (Map.insert (Map.empty) k 42)))
                (+ (match (Map.lookup m #list((Qty.of v (Unit.base #"meter"))
                                              (Qty.of (+ v 1) (Unit.base #"meter"))))
                     ((Some found) found) ((None) 0))
                   (match (Map.lookup m #list((Qty.of v (Unit.base #"meter"))
                                              (Qty.of (+ v 2) (Unit.base #"meter"))))
                     ((Some found) found) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a Set of lists-of-quantities dedups equal members and keeps distinct ones"
  (doc    "The Set twin, on the canonicalization path: `Set.of` over two `(list (Qty …))` members. Two equal
           lists-of-quantities canonicalize to the same CHAMP slot and dedup to `Set.len` 1; lists differing
           in a quantity element stay `Set.len` 2. Combined `dedup + 10*distinct` = 21. Pins the compound-key
           quantity-element peel through Set canonicalization.")
  (input  (do
            (def (main (: v Int64))
              (+ (Set.len #set(#list((Qty.of v (Unit.base #"meter")))
                                        #list((Qty.of v (Unit.base #"meter")))))
                 (* 10 (Set.len #set(#list((Qty.of v (Unit.base #"meter")))
                                              #list((Qty.of (+ v 1) (Unit.base #"meter"))))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 21 Int64)))

(case "a NESTED list-of-lists-of-quantities used as a Map key canonicalizes and hits"
  (doc    "The quantity-element shape peel recurses through nesting: a `(list (list (Qty …)))` Map key — a
           quantity two list levels deep — canonicalizes and hashes by content. A separately-built equal
           nested key HITS (42); one differing in the inner quantity MISSES (0). Combined = 42. Pins that the
           `shape_of` `Ty::Qty` peel composes with the recursive List shape builder (not just a one-level
           list-of-Qty key).")
  (input  (do
            (def (main (: v Int64))
              (let ((k #list(#list((Qty.of v (Unit.base #"meter")))))
                    (m (Map.insert (Map.empty) k 42)))
                (+ (match (Map.lookup m #list(#list((Qty.of v (Unit.base #"meter")))))
                     ((Some x) x) ((None) 0))
                   (match (Map.lookup m #list(#list((Qty.of (+ v 1) (Unit.base #"meter")))))
                     ((Some x) x) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a Set of tuples-containing-a-quantity dedups equal members and keeps distinct ones"
  (doc    "The tuple analogue of the list-of-Qty Set case: `Set.of` over `(tuple (Qty …) v)` members. The
           tuple shape builder recurses into the quantity element (peeled to its inner), so two equal
           tuples-with-a-quantity dedup to `Set.len` 1 and tuples differing in the quantity stay 2. Combined
           `dedup + 10*distinct` = 21. Pins the quantity-element peel through a TUPLE (not list) compound key.")
  (input  (do
            (def (main (: v Int64))
              (+ (Set.len #set(#tuple((Qty.of v (Unit.base #"meter")) v)
                                        #tuple((Qty.of v (Unit.base #"meter")) v)))
                 (* 10 (Set.len #set(#tuple((Qty.of v (Unit.base #"meter")) v)
                                              #tuple((Qty.of (+ v 1) (Unit.base #"meter")) v))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 21 Int64)))

(case "a Map VALUE that is a list-of-quantities round-trips through Map.lookup and List.at"
  (doc    "A quantity nested in a compound Map VALUE (not key): the map holds `1 → (list (Qty v m) (Qty (v+1)
           m))`, and `Map.lookup 1` returns the list, `List.at 1` its second quantity, `Qty.value` its
           magnitude. v=5 → 6. Pins that a list-of-quantities survives as a Map value through the decode +
           list-index path (the value-side complement of the compound-KEY cases above).")
  (input  (do
            (def (main (: v Int64))
              (let ((m (Map.insert (Map.empty) 1 #list((Qty.of v (Unit.base #"meter"))
                                                       (Qty.of (+ v 1) (Unit.base #"meter"))))))
                (match (Map.lookup m 1)
                  ((Some xs) (match (List.at xs 1) ((Some q) (Qty.value q)) ((None) 0)))
                  ((None) 0))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64))
  (live-objects known-leak))

(case "a whole MAP holding a quantity VALUE renders it scaled to reference in the value form"
  (doc    "The value-form RENDER of a whole Map whose VALUE is a quantity (the render companion of the
           lookup-round-trip case above): `(Map.insert (Map.empty) 1 (Qty.of 5.0 kilometer))` returned WHOLE
           renders `(map (1 (Qty.of 5000.0 meter)))` typed `(Map Int64 (Qty Float64 meter))` — the per-value
           reference scale-fold recurses into the Map's value slot (×1000 kilo → 5000 at reference), just as
           it does into tuple / sum-payload / record holes. Pins that a Map VALUE quantity is display-scaled
           in the whole-collection value form, not only when decoded via `Map.lookup` + `Qty.value`.")
  (input  (Map.insert (Map.empty) 1 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))))
  (output (: #map((= 1 (Qty.of 5000.0 (Unit.base #"meter"))))
             (Map Int64 (Qty Float64 (Unit.base #"meter"))))))

(case "a whole LIST of quantities renders every element scaled to reference in the value form"
  (doc    "The list sibling of the whole-Map-value render: a `(List (Qty …))` returned WHOLE renders every
           element scaled to its reference — `(list (Qty.of 5.0 kilometer) (Qty.of 2.0 kilometer))` →
           `(list (Qty.of 5000.0 meter) (Qty.of 2000.0 meter))` typed `(List (Qty Float64 meter))`. Pins the
           per-element reference scale-fold recursing into a LIST's elements in the whole-collection value
           form, not only when a single element is decoded via `List.at` + `Qty.value`. Companion of the
           whole-Map-value render above — both exercise a quantity inside a heap collection's element slot.")
  (input  #list((Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))
                (Qty.of 2.0 (Unit.prefix kilo (Unit.base #"meter")))))
  (output (: #list((Qty.of 5000.0 (Unit.base #"meter")) (Qty.of 2000.0 (Unit.base #"meter")))
             (List (Qty Float64 (Unit.base #"meter"))))))

; --- A NOMINAL newtype over a quantity as a (compound) key: strip_nominal ∘ peel-Ty::Qty -----------
; A single-variant nominal newtype `(type Len (Q (Qty Int64 meter)))` erases to the SAME machine slot as
; its inner quantity, which erases to its inner scalar — so the key shape builder must strip the nominal
; wrapper AND peel `Ty::Qty` to reach the scalar. These pin that composition on the KEY path: a
; nominal-over-Qty is a valid Map key / Set element, and one nested in a list key too.

(case "a nominal newtype over a quantity is a Map key that hits by content"
  (doc    "`(type Len (Q (Qty Int64 meter)))` — a nominal newtype over a quantity — used as a Map key. The
           newtype erases to the inner quantity's slot, which erases to the inner scalar, so the key
           comparator strips the nominal wrapper then peels `Ty::Qty`. A separately-built equal `Len` key
           HITS (42); one wrapping a different magnitude MISSES (0). Combined = 42. Pins strip_nominal ∘
           peel-Ty::Qty on the key path.")
  (input  (do
            (type Len (Q (Qty Int64 (Unit.base #"meter"))))
            (def (main (: v Int64))
              (let ((m (Map.insert (Map.empty) (Len.Q (Qty.of v (Unit.base #"meter"))) 42)))
                (+ (match (Map.lookup m (Len.Q (Qty.of v (Unit.base #"meter"))))
                     ((Some x) x) ((None) 0))
                   (match (Map.lookup m (Len.Q (Qty.of (+ v 1) (Unit.base #"meter"))))
                     ((Some x) x) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

(case "a Set of nominal-over-quantity newtypes dedups equal members"
  (doc    "The Set twin: `Set.of` over `(Len.Q (Qty …))` members. Two equal nominal-over-Qty values dedup to
           `Set.len` 1; distinct magnitudes stay 2. Combined `dedup + 10*distinct` = 21. Pins the
           strip_nominal ∘ peel-Ty::Qty composition through Set canonicalization.")
  (input  (do
            (type Len (Q (Qty Int64 (Unit.base #"meter"))))
            (def (main (: v Int64))
              (+ (Set.len #set((Len.Q (Qty.of v (Unit.base #"meter")))
                                        (Len.Q (Qty.of v (Unit.base #"meter")))))
                 (* 10 (Set.len #set((Len.Q (Qty.of v (Unit.base #"meter")))
                                              (Len.Q (Qty.of (+ v 1) (Unit.base #"meter"))))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 21 Int64)))

(case "a list of nominal-over-quantity newtypes is a compound Map key"
  (doc    "The compound-key composition: a `(list (Len.Q (Qty …)))` Map key — a nominal-over-Qty nested in a
           list key. The list-key canonicalization shape builder recurses into the element, strips the
           nominal wrapper, and peels `Ty::Qty` to the scalar. A separately-built equal list key HITS (42);
           one differing in the wrapped magnitude MISSES (0). Combined = 42. Pins the full recursive
           strip_nominal ∘ peel-Ty::Qty on a compound key.")
  (input  (do
            (type Len (Q (Qty Int64 (Unit.base #"meter"))))
            (def (main (: v Int64))
              (let ((k #list((Len.Q (Qty.of v (Unit.base #"meter")))))
                    (m (Map.insert (Map.empty) k 42)))
                (+ (match (Map.lookup m #list((Len.Q (Qty.of v (Unit.base #"meter")))))
                     ((Some x) x) ((None) 0))
                   (match (Map.lookup m #list((Len.Q (Qty.of (+ v 1) (Unit.base #"meter")))))
                     ((Some x) x) ((None) 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 42 Int64)))

; --- Quantity joins: the same-unit flow and the explicit-conversion repair --------------------------
; 806e45ba9 fixed the mixed-unit join DIAGNOSTIC (a scale clash, not a shadowed declaration). These
; pin the join semantics around it, promoted from passing breaker probes: a same-unit join is ONE
; type and flows; the repair for a mixed join is the explicit conversion (no silent unification).

(case "a same-unit quantity join flows and its magnitude reads back"
  (doc    "`(if (> b 0) (Qty.of 1 kilometer) (Qty.of 5 kilometer))` — BOTH branches are `(Qty Int64
           kilometer)`, one type, so the join flows; `Qty.value` reads the selected magnitude (1).
           The positive control beside the mixed-unit rejection: unit identity, not mere dimension
           agreement, is what joins (the conversion of a JOINED quantity is a separate, currently
           declining capability — this pins the join itself).")
  (input  (do
            (def (main (: b Int64))
              (Qty.value (if (> b 0) (Qty.of 1 (Unit.of #"kilometer")) (Qty.of 5 (Unit.of #"kilometer")))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1 Int64)))

(case "an explicit conversion repairs a mixed-unit join"
  (doc    "The REPAIR the scale-clash diagnostic names: convert one branch explicitly so both are
           meters — `(if b (Qty.of 1000 meter) (Qty.of (Unit.in meter (Qty.of 1 kilometer)) meter))`
           — and the join flows; both directions unwrap to 1000. Pins the no-silent-conversion rule's
           constructive half: the program states the scale change, and the two spellings of one
           kilometer agree exactly.")
  (input  (do
            (def (main (: b Int64))
              (Unit.in (Unit.of #"meter")
                (if (> b 0)
                    (Qty.of 1000 (Unit.of #"meter"))
                    (Qty.of (Unit.in (Unit.of #"meter") (Qty.of 1 (Unit.of #"kilometer"))) (Unit.of #"meter")))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 1000 Int64))
  (call   main (: 0 Int64))
  (output (: 1000 Int64)))

; --- Annotation-scale preservation: the composition faces beyond the rebrand fix's pins ------------
; ad4097530's pins grade the direct conversion/identity/join faces. These pin the compositions,
; promoted from passing breaker probes.

(case "an annotated argument joins a callee-side quantity at its own scale"
  (doc    "`(f (: (Qty.of 2 kilometer) (Qty Int64 meter)))` where f adds one kilometer — the
           annotation crosses a CALL boundary as the param's declared type, and the value still
           carries its own km scale inside the callee: 2 km + 1 km = 3 km → 3000 m. The
           param-annotation face of scale preservation (a rebrand at the call seam re-labels the
           argument exactly as the fixed inline annotation did).")
  (input  (do
            (def (f (: q (Qty Int64 (Unit.of #"meter"))))
              (+ q (Qty.of 1 (Unit.of #"kilometer"))))
            (def (main (: d Int64))
              (Unit.in (Unit.of #"meter") (f (: (Qty.of 2 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter"))))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 3000 Int64)))

(case "a double same-dimension annotation preserves the original scale"
  (doc    "`(: (: (Qty.of 1 kilometer) (Qty Int64 meter)) (Qty Int64 centimeter))` — TWO stacked
           same-dimension annotations at different units: each checks the dimension, neither touches
           the scale, so the value is still one kilometer → 1000 m. Pins idempotence of the
           check-not-coerce semantics under composition (a rebrand applied per-annotation would
           yield 1 cm → 0 m... or 1 m depending on order).")
  (input  (Unit.in (Unit.of #"meter")
            (: (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter"))) (Qty Int64 (Unit.of #"centimeter")))))
  (output (: 1000 Int64)))

; --- A MALFORMED unit COMPOSITION operand is named, not silently shipped (CDZ0201) ---------------
; `Unit.*`/`Unit./` compose two UNITS; `Unit.^` raises a unit to a compile-time INTEGER. A malformed
; operand — a non-unit factor (`(Unit.* (Unit.base #"m") 5)`), a non-unit power base, or a non-integer
; exponent (`(Unit.^ u 2.5)`) — is CDZ0201, naming the offending operand. Before this the `Qty.of`
; not-a-unit check SKIPPED a unit-builder-headed arg (deferring to the builder's own validation, which
; had NONE): the composition silently reduced to a unitless `Any`, `cdz check` PASSED, and `cdz compile`
; leaked "function return type has no machine representation" — a check-miss the reject now closes. A
; VALID composition is unaffected (the control cases in the body of the file exercise those).

(case "a non-unit factor in a Unit.* composition is rejected"
  (doc    "`(Unit.* (Unit.base #\"meter\") 5)` multiplies a unit by the integer `5` — `5` is not a unit, so
           the composition is malformed and rejected CDZ0201, naming the non-unit operand. Pins that a
           `Unit.*` factor must itself be a unit; before, this slipped `check` (the composition reduced to
           a unitless value) and only surfaced as a leaked 'no machine representation' at compile.")
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.* (Unit.base #"meter") 5)))) (export main)))
  (error  CDZ0201))

(case "a non-unit factor in a Unit./ composition is rejected"
  (doc    "`(Unit./ (Unit.base #\"meter\") 5)` — the quotient sibling: a non-unit divisor `5` is rejected
           CDZ0201, exactly as the product case. Pins the whole two-operand composer family refuses a
           non-unit operand.")
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit./ (Unit.base #"meter") 5)))) (export main)))
  (error  CDZ0201))

(case "a non-integer exponent in a Unit.^ power is rejected"
  (doc    "`(Unit.^ (Unit.base #\"meter\") 2.5)` raises a unit to `2.5` — a unit power's exponent must be a
           compile-time INTEGER (a fractional power has no unit meaning), so it is rejected CDZ0201, naming
           the exponent requirement. The power sibling of the non-unit-factor cases.")
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.^ (Unit.base #"meter") 2.5)))) (export main)))
  (error  CDZ0201))

(case "a non-unit base in a Unit.^ power is rejected"
  (doc    "`(Unit.^ 5 2)` raises the integer `5` to a power — the BASE of a unit power must be a unit, so a
           non-unit base is rejected CDZ0201. Completes the malformed-composition family: both operands of
           `Unit.^` (base and exponent) are validated, like both factors of `Unit.*`/`Unit./`.")
  (input  (do (def (main) (Qty.value (Qty.of 1.0 (Unit.^ 5 2)))) (export main)))
  (error  CDZ0201))

(case "a Qty-typed variant payload keeps its type non-generic and matches back"
  (doc    "A `(type Holder (H (Qty Rational (Unit.base #\"meter\"))))` — a variant whose payload is a Qty with
           a CONCRETE unit. The unit expression's leaf names (`base`, `meter`) are compile-time UNIT bases,
           NOT type variables, so `Holder` must stay a NULLARY (non-generic) type: `collect_type_params`
           descends only into a `(Qty T u)`'s inner type `T`, never the unit `u`. Previously the free
           lowercase `base` in the unit leaked into the type's parameter binder, making `Holder` spuriously
           generic `(Holder base)` — a bare `Holder` annotation then failed CDZ0203 and the match arm was
           `not a variant of Holder`. Here the constructor builds the Qty, a bare `Holder` sig resolves, and
           the arm reads the payload back; `Qty.value` of the `7/2 m` quantity floors' numerator is exercised
           via `Rational.value`. Runs to the rational magnitude 7/2.")
  (input  (do
            (type Holder (H (Qty Rational (Unit.base #"meter"))))
            (def (mk) (Holder.H (Qty.of (Rational.of 7 2) (Unit.base #"meter"))))
            (def (unwrap (: h Holder)) (match h ((Holder.H q) (Qty.value q))))
            (def (main) (unwrap (mk)))
            (export main)))
  (output (: 7/2 Rational)))

(case "a product type of three Qty fields with concrete units constructs and projects"
  (doc    "The units-carrying product a CAD `Vec3` needs: `(type V3q (V3 (Qty Rational m) (Qty Rational m)
           (Qty Rational m)))`, three Qty fields each with a concrete `meter` unit. Each unit expression's
           leaf names are unit bases, not type parameters, so `V3q` stays non-generic (three concrete
           payloads) — the same `(Qty T u)` unit-arg skip as the single-payload case, now across a
           multi-field product. Builds a `V3q` from a `5 m` quantity, projects the first field, reads its
           magnitude — 5/1. Pins that a Qty-typed PRODUCT field (not just a def-param annotation) is a
           first-class carrier of units.")
  (input  (do
            (type V3q (V3 (Qty Rational (Unit.base #"meter"))
                          (Qty Rational (Unit.base #"meter"))
                          (Qty Rational (Unit.base #"meter"))))
            (def (mkv (: x (Qty Rational (Unit.base #"meter")))) (V3q.V3 x x x))
            (def (getx (: v V3q)) (match v ((V3q.V3 a b c) a)))
            (def (main) (Qty.value (getx (mkv (Qty.of (Rational.of 5 1) (Unit.base #"meter"))))))
            (export main)))
  (output (: 5/1 Rational)))

(case "a record of quantities RETURNED as a value renders every field scaled to its reference"
  (doc    "The value-form RENDER companion of the construct-and-project case above: a GENERIC single-
           constructor record `(type V3q (V3 a a a))` instantiated at a prefixed quantity and RETURNED WHOLE
           renders each field scaled to its reference — `(V3q.V3 (5.0 km) (2.0 km) (3.0 km))` (all three at
           the SAME unit `kilometer`, so the generic `a` unifies cleanly — a DIFFERENT-scale field would be a
           correctly-rejected mismatch, no auto-convert) → `(tuple (Qty.of 5000.0 meter) (Qty.of 2000.0
           meter) (Qty.of 3000.0 meter))` typed `V3q`. Unlike the project-then-Qty.value case (which reads a
           raw stored magnitude), this pins the per-FIELD reference scale-fold recursing into a named
           record's payload holes at the boundary — the record twin of the bare-tuple-of-quantities render,
           the same fix path the rust backend needed for compound Qty leaves. Number and unit AGREE in every field.")
  (input  (do
            (type V3q (V3 a a a))
            (def (main) (V3q.V3 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))
                                (Qty.of 2.0 (Unit.prefix kilo (Unit.base #"meter")))
                                (Qty.of 3.0 (Unit.prefix kilo (Unit.base #"meter")))))
            (export main)))
  (output (: #tuple((Qty.of 5000.0 (Unit.base #"meter")) (Qty.of 2000.0 (Unit.base #"meter"))
                    (Qty.of 3000.0 (Unit.base #"meter"))) V3q)))

(case "a USER-DEFINED multi-variant sum payload quantity renders scaled to its reference"
  (doc    "A quantity in a USER-DEFINED multi-variant sum's payload displays scaled to its reference when the
           sum is returned as a value: `(type Shape (Circle (Qty Float64 kilometer)) (Sq Int64))` with
           `(Circle (Qty.of 3.0 kilometer))` renders `(Circle (Qty.of 3000.0 meter))` — the ×1000 kilo scale
           folds into the payload magnitude, the unit shows at the reference `meter`. Extends the compound
           value-form scale-fold (tuple / Option / Result / record) to a USER sum's variant-constructor
           payload hole — the render descends the matched variant and scales its Qty leaf. The payload's
           declared unit equals the applied value's unit (kilometer = kilometer; a quantity does not
           auto-convert at construction). Number and unit AGREE in the payload.")
  (input  (do
            (type Shape (Circle (Qty Float64 (Unit.prefix kilo (Unit.base #"meter")))) (Sq Int64))
            (def (main) (Shape.Circle (Qty.of 3.0 (Unit.prefix kilo (Unit.base #"meter")))))
            (export main)))
  (output (: (Circle (Qty.of 3000.0 (Unit.base #"meter"))) Shape)))

(case "a Qty payload's INNER type stays a real type parameter — the unit-arg skip does not over-skip"
  (doc    "The companion of the two cases above, guarding the OTHER edge of the `(Qty T u)` type-parameter
           skip. `collect_type_params` descends ONLY into a `(Qty T u)`'s inner type `T` and skips the unit
           `u` — but it must NOT over-skip: a type VARIABLE sitting in the inner (`T`) position is still a
           real generic parameter. `(type Box (B (Qty a (Unit.base #\"meter\"))))` has `a` in the inner-type
           position, so `Box` is GENERIC over `a` (`children[1]` is harvested), while the unit-leaf `base`
           is still NOT a parameter. So `(Box Rational)` resolves and a BARE `Box` correctly needs a type
           argument (CDZ0203, its own reject case would fire) — exactly the genericity the fix must preserve.
           Construct a `(Box Rational)` from a `7/2 m` quantity, project the payload, read its magnitude — the
           inner-type parameter flows through construct → match → `Qty.value`. Runs to 7/2, proving the skip
           preserved the inner-type generic (a spurious over-skip would have made `Box` nullary and rejected
           `(Box Rational)`).")
  (input  (do
            (type Box (B (Qty a (Unit.base #"meter"))))
            (def (mk (: x (Qty Rational (Unit.base #"meter")))) (Box.B x))
            (def (unwrap (: b (Box Rational))) (match b ((Box.B q) (Qty.value q))))
            (def (main) (unwrap (mk (Qty.of (Rational.of 7 2) (Unit.base #"meter")))))
            (export main)))
  (output (: 7/2 Rational)))

(case "a Qty payload with a GENERIC inner type rejects a bare use with no type argument"
  (doc    "The reject twin of the genericity pin above: since `(type Box (B (Qty a (Unit.base #\"meter\"))))`
           is generic over its inner-type parameter `a` (harvested from the `(Qty T u)` inner position, unit
           skipped), a BARE `Box` annotation with no type argument is CDZ0203 — the same as any generic type
           used without its argument. This pins that the inner-type parameter is genuinely required (the skip
           does not silently drop it, which would make `Box` nullary and wrongly ACCEPT a bare `Box`).")
  (input  (do
            (type Box (B (Qty a (Unit.base #"meter"))))
            (def (mk (: x (Qty Rational (Unit.base #"meter")))) (Box.B x))
            (def (unwrap (: b Box)) (match b ((Box.B q) (Qty.value q))))
            (def (main) (unwrap (mk (Qty.of (Rational.of 7 2) (Unit.base #"meter")))))
            (export main)))
  (error  CDZ0203))

(case "angle units radian and degree are first-class and exact within their own dimension"
  (doc    "ANGLE units — `radian` and `degree`, first-class built-ins for CAD revolve/rotate angles (the
           operator ruling). They are SEPARATE base dimensions (NOT one angle dimension): rad↔deg is
           IRRATIONAL (180° = π rad, π has no exact Rational), and every family unit keys to an EXACT
           rational ratio to its dimension reference, so one shared dimension would break the exact-Rational
           invariant. As distinct dimensions each is EXACT within itself: `5 degree + 90 degree = 95 degree`
           (the magnitude sums exactly, no π ever enters). Mixing them is a CDZ0501 dimension mismatch — the
           companion case below — never a silent irrational conversion. Pins that the angle family composes
           exactly within one unit; a program crossing rad↔deg does so explicitly at the f64/sin-cos boundary.")
  (input  (Qty.value (+ (Qty.of 5 (Unit.of #"degree")) (Qty.of 90 (Unit.of #"degree")))))
  (output (: 95 Int64)))

(case "adding a degree quantity to a radian quantity is a dimension mismatch"
  (doc    "The honest cross-dimension reject: `degree` and `radian` are DISTINCT base dimensions (their
           conversion is irrational — see above), so adding them is CDZ0501 (incompatible dimension),
           exactly as `meter + second` is. There is no silent rad↔deg conversion — the angle family keeps
           each unit exact within itself, and a genuine conversion must be explicit at the approximate f64
           boundary. Pins that the two angle dimensions do NOT interconvert implicitly.")
  (input  (+ (Qty.of 1 (Unit.of #"degree")) (Qty.of 1 (Unit.of #"radian"))))
  (error  CDZ0501))

; A Quantity stored in a RECURSIVE sum's payload — the list-of-quantities shape a measurement log
; takes. The Qty erases to its inner scalar in the variant payload slot, and the recursive walk
; (construct N cells, then match-fold them) must carry the erased magnitudes with the units held by
; the STATIC type alone; the rust backend's per-payload display-scale walk additionally needs its
; recursive-sum CYCLE GUARD here (a naive type-directed payload walk on QList recurses forever).

(case "a quantity in a recursive sum payload folds through construction and match"
  (doc    "`(type QList (QNil) (QCons (Qty Int64 meter) QList))` — a recursive sum whose payload is a
           QUANTITY. `total` folds the list by match, summing `Qty.value` of each cell: `(QCons a·m
           (QCons 5m QNil))` at a=3 → 8. Pins that a Qty rides a recursive sum's payload slot as its
           erased scalar (construct → boxed variant payload → match-bind → Qty.value), and that the
           type-level recursion (QList inside QList) does not trip the payload walk — the corpus face
           of the rust backend's recursive-sum cycle guard on the display-scale walk. Expected: 8.")
  (input  (do
            (type QList (QNil) (QCons (Qty Int64 (Unit.base #"meter")) QList))
            (def (total (: l QList))
              (match l
                ((QNil) 0)
                ((QCons q rest) (+ (Qty.value q) (total rest)))))
            (def (main (: a Int64))
              (total (QCons (Qty.of a (Unit.base #"meter"))
                     (QCons (Qty.of 5 (Unit.base #"meter")) (QNil)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64))
  (live-objects known-leak))

; --- Qty through program structure (module exports, CHAMP values, closure envs, extract/compute/
; reinsert) with the dimension checks holding at each boundary; the #44 workaround perimeter
; (arm-local def + value/re-wrap — the inline resume-slot spelling is the held finding); and the
; free-abelian-group exponent laws. ---

(case "a Qty-typed export carries its unit frame across the module boundary into caller algebra"
  (doc    "Units × modules: `double-len : (Qty Int64 meter) -> ...` is exported, and the IMPORTER's
           algebra composes its result with a fresh meter quantity — the unit frame in the export
           SIGNATURE must survive the import so the caller-side `+` type-checks as same-unit
           (2k+1 → 11 at k=5, 1 at k=0). An import that erased the signature to bare Int64 would
           let a seconds operand through the caller's add (dimension-safety hole); one that
           re-keyed the unit fails the same-unit check falsely.")
  (input  (do
        (import "geo" (double-len))
        (def (main (: k Int64))
          (Qty.value (+ (double-len (Qty.of k (Unit.base #"meter")))
                        (Qty.of 1 (Unit.base #"meter")))))
        (export main)))
  (module "geo"
    (do
      (def (double-len (: d (Qty Int64 (Unit.base #"meter"))))
        (+ d d))
      (export double-len)))
  (call   main (: 5 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "a seconds quantity is rejected by a meter-typed import at the call site"
  (doc    "The dimension-reject twin of the Qty-export pin: the importer hands
           `(Qty Int64 second)` to the meter-typed `double-len` — rejected CDZ0501 AT THE CALL
           SITE, proving the unit frame crossed the boundary with teeth (an import that erased
           the signature would accept and double the seconds silently). The cross-module
           dimension check is what makes Qty-typed libraries safe to publish.")
  (input  (do
        (import "geo" (double-len))
        (def (main (: k Int64))
          (Qty.value (double-len (Qty.of k (Unit.base #"second")))))
        (export main)))
  (module "geo"
    (do
      (def (double-len (: d (Qty Int64 (Unit.base #"meter"))))
        (+ d d))
      (export double-len)))
  (error  CDZ0501))

(case "a map of meter quantities rejects a second-dimension value insert"
  (doc    "The dimension-safety face of the Qty collection cycle: `(Map Int64 (Qty Int64 meter))`
           refuses a `(Qty Int64 second)` value insert — CDZ0201 naming BOTH Qty types (the unit is
           part of the VALUE type, so cross-dimension pollution is a compile reject, not a runtime
           surprise; there is no runtime unit tag to catch it later). The compute-cycle pin above
           shows same-unit arithmetic flowing through the map; this pins the boundary that makes
           that flow safe.")
  (input  (do
        (def (main (: k Int64))
          (do
            (def m (Map.insert Map.empty 1 (Qty.of 30 (Unit.base #"meter"))))
            (def m2 (Map.insert m 2 (Qty.of k (Unit.base #"second"))))
            (Map.len m2)))
        (export main)))
  (error  CDZ0201))

(case "a quantity captured in a closure env adds against per-call quantities"
  (doc    "Units × captures: the factory's `(Qty Int64 meter)` param rides the returned closure's
           ENV (the erased magnitude in a capture cell, the unit in the closure's TYPE frame) and
           each application adds a fresh same-unit quantity (100+k then 100+1 → 1151 at k=5, 1101
           at k=0). A capture that stored the Qty as a boxed compound (rep mismatch with the erased
           scalar) or an env type that dropped the unit (letting a seconds arg unify later) breaks
           the add or the safety. The closure consumer completes the Qty surface: collections,
           arithmetic, keys, params, and now envs.")
  (input  (do
        (def (mk (: base (Qty Int64 (Unit.base #"meter"))))
          (fn ((: n Int64)) (Qty.value (+ base (Qty.of n (Unit.base #"meter"))))))
        (def (main (: k Int64))
          (do
            (def f (mk (Qty.of 100 (Unit.base #"meter"))))
            (+ (* 10 (f k)) (f 1))))
        (export main)))
  (call   main (: 5 Int64)) (output (: 1151 Int64))
  (call   main (: 0 Int64)) (output (: 1101 Int64)))

(case "unit arithmetic on a map-extracted quantity re-enters the map typed"
  (doc    "The extract-compute-reinsert cycle for UNIT-typed values (the 18-units collection pins
           are identity/dedupe): a `(Qty Int64 meter)` comes OUT of a map, ADDS a fresh same-unit
           quantity (the static unit check crossing the Option/lookup boundary), and the SUM
           re-enters under a new key — read back via Qty.value (350+2 → 352 at k=5, 302 at k=0).
           A lookup that erased the value to a BARE Int64 (losing the unit frame) would let a
           dimension-mixing bug through the later add; the typed round-trip pins that the frame
           survives the collection.")
  (input  (do
        (def (main (: k Int64))
          (do
            (def m (Map.insert Map.empty 1 (Qty.of 30 (Unit.base #"meter"))))
            (def d (Option.expect (Map.lookup m 1) "p"))
            (def m2 (Map.insert m 2 (+ d (Qty.of k (Unit.base #"meter")))))
            (+ (* 10 (Qty.value (Option.expect (Map.lookup m2 2) "p")))
               (Map.len m2))))
        (export main)))
  (call   main (: 5 Int64)) (output (: 352 Int64))
  (call   main (: 0 Int64)) (output (: 302 Int64)))

; FINDING #44 (breaker): a Qty+Qty arithmetic expression INLINE in a handler's resume slot
; (either the VALUE slot or the NEXT-STATE slot) is falsely typed as the ERASED inner scalar
; (Int64) and rejected — while the semantically identical expression bound via an arm-local
; `def` first type-checks AND runs correctly. False reject + workaround inconsistency.
;
;   (handle Acc (Qty.of a meter)
;     ((step (_u) s (resume s (+ s s))))          ; REJECTS: "next-state of type Int64 but state
;                                                 ;  type is (Qty Int64 meter)" — but s IS the Qty
;     ...)
;   ((step (_u) s (resume (+ s s) s)))            ; REJECTS: "resumes with a value of type Int64
;                                                 ;  but the operation's result type is (Qty ...)"
;   ((step (_u) s (do (def t (+ s s)) (resume t s))))  ; ACCEPTS and runs → 42 at a=21
;
; The state binder `s` seeded with a Qty seems to lose its Qty type exactly when consumed by an
; arithmetic op INSIDE the resume-slot expression — the checker types (+ s s) at the erased inner
; scalar (Int64) and then the slot check compares Int64 vs (Qty ...). Control: (+ q q) over a Qty
; in a PLAIN fn types fine; (resume s s) pass-through is fine; Qty.value/re-wrap in the slot is fine.
; Lane guess: v-inference (handler-arm slot typing runs before/without the Qty layer's op typing?)
; or the effects fold's arm typing. Probed on trunk fc2b91731. (Finding #44 is now CLOSED and its
; witnesses live in 14-effects; this note is retained here only as Qty-layer provenance.)
;
; --- The free-abelian exponent laws (drain AA) -------------------------------------------------
(case "a nested unit power multiplies exponents — (m^2)^2 is the same dimension as m^4"
  (doc    "The free-abelian-group laws beyond product/quotient cancellation: NESTED power must MULTIPLY exponents ((Unit.^ (Unit.^ m 2) 2) = m^4 — adding or concatenating gives m^2/m^6 and the same-dimension divide rejects); verified by a dimensionless divide (5).")
  (input  (do
            (def m2 (Unit.^ (Unit.base #"meter") 2))
            (def m4 (Unit.^ m2 2))
            (def (main (: a Int64))
              (Qty.value (/ (Qty.of a m4) (Qty.of 2 (Unit.^ (Unit.base #"meter") 4)))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 5 Int64)))

(case "a unit raised to the ZEROTH power is the dimensionless identity Unit.one"
  (doc    "u^0 must BE Unit.one — the zero-exponent entry drops from the canonical map (a map keeping meter->0 fails the same-dimension + with a Unit.one quantity).")
  (input  (do
            (def m0 (Unit.^ (Unit.base #"meter") 0))
            (def (main (: a Int64))
              (+ (Qty.value (Qty.of a m0)) (Qty.value (+ (Qty.of a m0) (Qty.of 1 Unit.one)))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 9 Int64)))

(case "a negative-exponent unit cancels its base through a multiply to dimensionless"
  (doc    "A negative exponent as a def-bound first-class unit VALUE (hz = s^-1) cancelling through a runtime multiply to dimensionless — the existing neg-exp case is inline and Float-inner; this is def-bound with an Int64 inner.")
  (input  (do
            (def hz (Unit.^ (Unit.base #"second") -1))
            (def (main (: a Int64))
              (Qty.value (* (Qty.of a hz) (Qty.of 3 (Unit.base #"second")))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 21 Int64)))


; --- A derived-dimension quantity flowing through a call boundary. ---

(case "a helper RETURNS a derived-dimension (m squared) quantity and same-dimension results add"
  (doc    "The derived-dimension flow face (the value-form pins render m2/m3 in OUTPUT annotations only): a helper's INFERRED return type carries (Unit.^ meter 2) and two results add as same-dimension m2 — the composed-dimension frame flows through a call boundary. (Spelled with a def-bound unit + inferred return: a derived unit in an INPUT-position type annotation trips an ML-printer re-parse gap, reported to v-syntax.)")
  (input  (do
            (def m2 (Unit.^ (Unit.base #"meter") 2))
            (def (area (: v Int64)) (Qty.of (* v v) m2))
            (def (main (: k Int64))
              (Qty.value (+ (area k) (area 3))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 25 Int64)))

; --- The input-position derived-unit annotation (type_ref infix witness). ---

(case "a derived-unit type annotation in INPUT position round-trips and the param computes"
  (doc    "The corpus witness for the type_ref infix fix (bd6a1bafd): a (Qty Int64 (Unit./ meter (Unit.^ second 2))) PARAM annotation — a meter/second^2 acceleration unit that combines BOTH infix operators, pinning the /-vs-^ precedence (^ binds tighter, so it parses as meter/(second^2), not (meter/second)^2). Before the fix the ML type parser choked on the infix exponent ('expected ,'; only OUTPUT-position annotations existed in corpus so the ml_surface path never saw one). Exercises print->re-parse of the combined derived unit AND the annotated param computing (16 at k=4; Qty.value reads the scalar, so the unit shape does not change the value).")
  (input  (do
            (def (f (: q (Qty Int64 (Unit./ (Unit.base #"meter") (Unit.^ (Unit.base #"second") 2))))) (Qty.value q))
            (def (main (: k Int64)) (f (Qty.of (* k k) (Unit./ (Unit.base #"meter") (Unit.^ (Unit.base #"second") 2)))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 16 Int64)))

; --- Construction-path equality on the unit lattice (the collection/string/bytes companions
; live in 05/19/13/10): a Qty whose unit was REACHED via arithmetic must equal one whose unit
; was directly COMPOSED, independent of written operand order. ---

(case "a product-REACHED quantity equals the directly-composed unit in either written order"
  (doc    "Construction-path equality on the unit lattice: `(* 2m 3s)` REACHES the unit m·s through Qty
           multiplication; the right-hand sides COMPOSE it directly with Unit.* — written m·s (tens digit)
           and s·m (ones digit). Both must equal 6 at the same canonical unit → 11. A unit product that
           preserved operand order (m·s ≠ s·m) or an arithmetic path that composed a structurally
           different unit term than Unit.* breaks a leg.")
  (input  (do
            (def (main (: n Int64))
              (+ (* 10 (if (= (* (Qty.of n (Unit.base #"m")) (Qty.of 3 (Unit.base #"s")))
                             (Qty.of (* n 3) (Unit.* (Unit.base #"m") (Unit.base #"s")))) 1 0))
                 (if (= (* (Qty.of n (Unit.base #"m")) (Qty.of 3 (Unit.base #"s")))
                        (Qty.of (* n 3) (Unit.* (Unit.base #"s") (Unit.base #"m")))) 1 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 11 Int64)))

(case "a division-REACHED quantity equals the inverse-composed unit and cancels to Unit.one"
  (doc    "The division face: `(/ 6m 3s)` reaches m/s = m·s⁻¹; the direct composition spells it
           `(Unit.* m (Unit.^ s -1))` — equal at value 2 (tens digit). And the full-cancellation face:
           `(/ nm 1m)` must land exactly on the dimensionless `Unit.one` the literal composes (ones
           digit) → 11. A division that left a residual m/m term (structurally present, exponent zero)
           instead of erasing it breaks the cancel leg.")
  (input  (do
            (def (main (: n Int64))
              (+ (* 10 (if (= (/ (Qty.of (* n 3) (Unit.base #"m")) (Qty.of 3 (Unit.base #"s")))
                             (Qty.of n (Unit.* (Unit.base #"m") (Unit.^ (Unit.base #"s") -1)))) 1 0))
                 (if (= (/ (Qty.of n (Unit.base #"m")) (Qty.of 1 (Unit.base #"m")))
                        (Qty.of n Unit.one)) 1 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 11 Int64)))

(case "a mixed-scale combine over a computed BigInt quantity operand converts across scales"
  (doc    "`(Qty.value (+ (* (Qty.of (BigInt.of n) km) 2) (Qty.of 500 m)))`: a COMPUTED (2x-scaled) km
           quantity operand combines with a metre quantity across scales — n=3 → 3km*2=6000m + 500m = 6500m.")
  (input (do
    (def (main (: n Int64))
      ((. Qty value) (+ (* ((. Qty of) ((. BigInt of) n) ((. Unit prefix) kilo ((. Unit base) #"meter")))
                           ((. BigInt of) 2))
                        ((. Qty of) ((. BigInt of) 500) ((. Unit base) #"meter")))))
    (export main)))
  (call main (: 3 Int64)) (output (: 6500 BigInt))
  (live-objects known-leak))

; ── breaker batch 553: quantities as CHAMP KEYS (the hash/eq agreement pattern on a type whose
; `=` is scale-converting). The design forecloses the divergence: same-unit keys round-trip with
; value discrimination (qkm1); a CROSS-scale probe is a compile-time type error with the teaching
; convert-with-in/as diagnostic (qkm2 — the mixed-scale `=` conversion rule deliberately does NOT
; extend to collection ops); the explicit route (Unit.in + re-wrap) hits correctly (qkm3).

(case "qkm1 a same-unit Float quantity Map key round-trips with value discrimination"
  (input (do (def (main (: n Int64))
  (let ((m (Map.insert (Map.empty) (Qty.of (Float64.of-int (* n 5000)) (Unit.base #"meter")) 42)))
    (+ (* 100 (match (Map.lookup m (Qty.of 5000.0 (Unit.base #"meter"))) ((Some v) v) ((None u) -1)))
       (match (Map.lookup m (Qty.of 6000.0 (Unit.base #"meter"))) ((Some v) v) ((None u) -1)))))
(export main)))
  (call main (: 1 Int64))
  (output (: 4199 Int64))
  (live-objects 0))

(case "qkm2 a CROSS-scale Map probe is a compile-time type error (the mixed-scale = conversion does not extend to collection ops)"
  (input (do (def (main (: n Int64))
  (match (Map.lookup (Map.insert (Map.empty) (Qty.of 5000.0 (Unit.base #"meter")) 42) (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))) ((Some v) v) ((None u) -1)))
(export main)))
  (error CDZ0203))

(case "qkm3 an explicitly converted cross-scale probe (Unit.in + re-wrap) hits the same-unit key"
  (input (do (def (main (: n Int64))
  (let ((m (Map.insert (Map.empty) (Qty.of (Float64.of-int (* n 5000)) (Unit.base #"meter")) 42)))
    (match (Map.lookup m (Qty.of (Unit.in (Unit.base #"meter") (Qty.of 5.0 (Unit.prefix kilo (Unit.base #"meter")))) (Unit.base #"meter"))) ((Some v) v) ((None u) -1))))
(export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (live-objects 0))

; Two SAME-dimension quantities at DIFFERENT units (km vs m) are distinct `(Qty T u)` types (the unit carries
; the scale), so an if-join (CDZ0203) or a list-element join (CDZ0201 homogeneity) rejects them — but BOTH
; render to `(Qty … (Unit.base #"meter"))` (reference-unit name, scale dropped), so the bare message reads as
; a contradiction / would wrongly blame "a declaration shadows a built-in". The quantity-scale hint fires
; first and names the REAL cause: "SAME dimension at DIFFERENT units" (convert with in/as). A cross-DIMENSION
; clash (meter vs second) is a plain distinguishable mismatch — no scale tail. (Migrated from rcdzc
; an_if_join_over_different_unit_quantities_names_the_scale_not_a_shadowed_declaration +
; a_list_of_different_unit_quantities_names_the_scale_not_two_identical_looking_types.)
(case "an if-join over different-unit same-dimension quantities names the scale, not a shadowed declaration"
  (input  (do (def (main (: b Bool)) ((. Qty value) (if b ((. Qty of) 1 ((. Unit prefix) kilo ((. Unit base) #"meter"))) ((. Qty of) 500 ((. Unit base) #"meter"))))) (export main)))
  (error  CDZ0203 (message "SAME dimension at DIFFERENT units") (not "shadows a built-in")))

(case "a list-element join over different-unit same-dimension quantities names the scale"
  (input  (do (def (main) ((. Qty value) ((. List at) #list(((. Qty of) 5.0 ((. Unit prefix) kilo ((. Unit base) #"meter"))) ((. Qty of) 2.0 ((. Unit base) #"meter"))) 0))) (export main)))
  (error  CDZ0201 (message "SAME dimension at DIFFERENT units")))

(case "a cross-dimension if-join clash stays a plain mismatch (no same-dimension scale tail)"
  (input  (do (def (main (: b Bool)) ((. Qty value) (if b ((. Qty of) 1 ((. Unit base) #"meter")) ((. Qty of) 2 ((. Unit base) #"second"))))) (export main)))
  (error  CDZ0203 (message "meter") (not "SAME dimension at DIFFERENT units")))

(case "a cross-dimension list-join clash stays a plain mismatch (no same-dimension scale tail)"
  (input  (do (def (main) ((. Qty value) ((. List at) #list(((. Qty of) 5.0 ((. Unit base) #"meter")) ((. Qty of) 2.0 ((. Unit base) #"second"))) 0))) (export main)))
  (error  CDZ0201 (message "meter") (not "SAME dimension at DIFFERENT units")))
