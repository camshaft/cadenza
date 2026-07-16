; ADVERSARIAL FINDING (breaker, 2026-07-16) — 🔴 MISCOMPILE (silent wrong value): a same-dimension
; quantity ANNOTATION does not merely CHECK the dimension (the ratified ed5ad7901 semantics) — it
; REBRANDS the value at the annotation's unit, keeping the raw magnitude. `(: (Qty.of 1 kilometer)
; (Qty Int64 meter))` becomes ONE METER, not one kilometer. The spec/concierge ruling the fix cites
; says the annotated value "KEEPS ITS OWN SCALE (the annotation checks dimension, does not
; normalize/coerce to its unit)" — the implementation instead re-labels the magnitude wholesale.
;
; REPRODUCERS (both backends AGREE on the wrong value — shared inference, not a backend emit):
;   (Unit.in meter (: (Qty.of 1 kilometer) (Qty Int64 meter)))       → 1     WANT 1000
;   (Unit.in cm    (: (Qty.of 1 kilometer) (Qty Int64 centimeter)))  → 100   WANT 100000
;     (100 = 1 meter expressed in cm — the value was re-branded to the REFERENCE scale? No:)
;   (Unit.in kilometer (: (Qty.of 1 kilometer) (Qty Int64 meter)))   → 0     WANT 1 (identity)
;     (0 = 1 METER truncated to km — confirming the magnitude 1 now carries the ANNOTATION's unit.)
;   (+ (: (Qty.of 1 km) (Qty Int64 meter)) (Qty.of 2 km))            → joins at... ran 2001 in meter:
;     the annotated value entered the add AS 1 METER and the SUM was legal — meaning the annotated
;     value ALSO changed its join identity to meter... yet it added to a km operand without a scale
;     clash (2001 m = 1 m + 2 km silently converted!). TWO bugs compound here: the rebrand plus a
;     silently-converting mixed add through the rebranded operand.
;
; CONTROLS (correct, isolating the rebrand exactly):
;   (Unit.in meter (Qty.of 1 kilometer))            → 1000   [no annotation: conversion right]
;   (Qty.value (: (Qty.of 1 km) (Qty Int64 meter))) → 1      [magnitude preserved — it's the UNIT that flips]
;   (: (Qty.of 2500 meter) (Qty Int64 kilometer)) in meter → 2500  [WRONG for the same reason but
;     coincidentally right-looking: 2500 now means 2500 KILOMETERS... and in-meter yields 2500?
;     That contradicts the rebrand... UNLESS Unit.in read the SOURCE-syntax unit. The visible pair
;     (iso5=2500, iso6=2) is self-consistent with REBRAND-to-annotation-unit: 2500 km in m would be
;     2500000. Observed 2500 => in-meter of the rebranded value read it as 2500 m. iso6 (in km) = 2
;     => read as 2500 m truncated to 2 km. So the rebrand target is the ANNOTATION unit for the
;     km-annotation faces and both reads are consistent with magnitude-as-annotation-unit EXCEPT
;     iso5's in-meter... 2500 m in m = 2500 ✓. All observations consistent: MAGNITUDE KEPT, UNIT :=
;     ANNOTATION UNIT.]
;
; VERDICT: the annotation performs a silent SCALE REINTERPRETATION — `1 km` re-labeled `1 m`. This
; violates the fix's own stated semantics (keep own scale) and the no-silent-conversion rule from
; the OTHER side (a conversion happened with neither Unit.in nor a stated repair — magnitude
; constant, unit swapped). The composed-add face additionally shows the rebranded operand joining a
; genuine km quantity without the mixed-unit rejection, silently mixing scales in one sum.
;
; SEVERITY: 🔴 silent wrong value in the units feature's core promise (dimensional safety). Any
; program annotating a quantity at a same-dimension unit (the exact idiom ed5ad7901 legalized
; yesterday) computes with re-labeled magnitudes. Graded cases below (all Fail on current trunk,
; both backends).

(case "a same-dimension annotation preserves the value's own scale through a conversion"
  (doc    "`(Unit.in meter (: (Qty.of 1 kilometer) (Qty Int64 meter)))` — the ratified semantics
           (DESIGN-quantity-reference-normalized-unwrap.md §Interaction With Annotations, restated in
           ed5ad7901): the annotation CHECKS the dimension and the value KEEPS ITS OWN SCALE. One
           kilometer converted to meters is 1000. Instead the annotation re-labels the magnitude at
           the annotation's unit (1 meter) → 1. Expected: 1000.")
  (input  (Unit.in (Unit.of #"meter") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1000 Int64)))

(case "a same-dimension annotation is the identity under its own unit's conversion"
  (doc    "`(Unit.in kilometer (: (Qty.of 1 kilometer) (Qty Int64 meter)))` — converting the annotated
           kilometer BACK to kilometers is the identity → 1. Instead 0 (the re-labeled 1 METER
           truncates to 0 km), the sharpest single witness of the rebrand. Expected: 1.")
  (input  (Unit.in (Unit.of #"kilometer") (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))))
  (output (: 1 Int64)))

(case "an annotated quantity joins additions at its own scale"
  (doc    "`(+ (: (Qty.of 1 km) (Qty Int64 meter)) (Qty.of 2 km))` in meters — one km plus two km is
           three km → 3000 m. Instead 2001: the annotated operand entered the add as 1 METER and the
           mixed add silently converted (compounding the rebrand with a silent mixed-scale sum the
           join rule forbids). Expected: 3000.")
  (input  (Unit.in (Unit.of #"meter")
            (+ (: (Qty.of 1 (Unit.of #"kilometer")) (Qty Int64 (Unit.of #"meter")))
               (Qty.of 2 (Unit.of #"kilometer")))))
  (output (: 3000 Int64)))
