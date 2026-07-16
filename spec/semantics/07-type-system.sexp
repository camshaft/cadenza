; Type system — witnesses type-system.md. The seed is a COMPILER that realizes the static-typing floor
; incrementally (constitution VII; Amendment 0.4.0): an ill-typed program's recorded outcome IS its
; rejection — (error <CODE>) is the primary clause, because an ill-typed program has no run and therefore
; no terminal value. For a type rule a generation does not yet cover it DECLINES rather than running the
; program (reject-don't-miscompile); the gate scores a decline as todo, not disagreement. Diagnostic
; codes are from options/diagnostics-schema/.

(case "a type annotation consistent with the value is transparent"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict: an annotation agreeing
           with the value changes nothing and the program evaluates to the annotated value.")
  (input  (: 42 Int64))
  (output (: 42 Int64)))

(case "an annotation that contradicts the value is rejected"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict: `(: 42 Bool)` annotates
           an Int64 value with Bool — a contradiction the compiler rejects (CDZ0203). The rejection is
           the program's outcome; there is no value, because the program does not run.")
  (input  (: 42 Bool))
  (error  CDZ0203))

; The TYPE OPERAND of an annotation `(: expr T)` must itself DENOTE A TYPE — validating what stands in
; type position is the dual of checking it against the value. A non-type there (an unbound name, an
; integer/compound VALUE, an arbitrary expression, a non-constructor type applied to arguments) is
; MEANINGLESS and MUST be REJECTED, not silently accepted-and-ignored (which would let a typo'd or
; garbage annotation pass — the opposite of the reject-don't-accept-garbage discipline the checker
; applies everywhere else). An UNBOUND NAME rejects the same CDZ0101 it gets in value position (`(+ foo
; 1)`); a well-formed non-type rejects CDZ0203 ("expected a type"). This holds for a PARAMETER annotation
; `(: name T)` too, not only a value annotation.

(case "an unbound name in a type annotation's type position is rejected"
  (doc    "`(: 5 foo)` puts the unbound name `foo` in TYPE position. `foo` names no type, and the same
           `foo` in VALUE position is a hard CDZ0101 'unbound name', so it must reject identically here —
           an annotation whose type is a typo or a non-type is meaningless (type-system.md #Annotations
           Constrain, Never Contradict). A generation that resolved the operand, found it not a type, and
           dropped the annotation ACCEPTED this and ran to 5; the type position must reject a non-type.")
  (input  (do (def (main) (: 5 foo)) (export main)))
  (error  CDZ0101))

(case "an unbound uppercase name in a type annotation's type position is rejected"
  (doc    "`(: 5 Foo)` puts the unbound UPPERCASE name `Foo` in type position — a missing or typo'd
           CONCRETE type (unlike the lowercase `foo` above, which reads as an ML-style type variable). Both
           reject CDZ0101 (the name is unbound either way), but they are distinct diagnostic branches: a
           lowercase name gets an actionable 'generic route' hint (Cadenza's polymorphism comes from an
           UNANNOTATED parameter, not a `∀`-binder in an annotation), while an uppercase name — read as a
           concrete type that does not exist — keeps the plain 'unbound name' message. Pins the uppercase
           branch (a missing concrete type), the case-distinct companion of the lowercase-`foo` case.")
  (input  (do (def (main) (: 5 Foo)) (export main)))
  (error  CDZ0101))

(case "a lowercase type variable in a USER-GENERIC parameter annotation is rejected"
  (doc    "`(def (next (: it (Iter a))) …)` annotates a parameter with a USER-GENERIC type applied to a
           lowercase `a` — the ML/Haskell reflex for a generic signature `next : Iter a -> …`. But Cadenza
           has NO `∀`-binder in an annotation: generics are type-valued parameters (type-system.md
           #Generics Are Type-Valued Parameters), so `a` in the annotation names no type and is a hard
           CDZ0101 'unbound name'. The nested lowercase leaf inside a user generic `(Iter a)` gets the same
           reject the bare `(: x a)` does — the actionable route is to drop the annotation (an UNANNOTATED
           parameter is already polymorphic) or take the element type as an explicit `(: t Type)` parameter.
           Pins that a lowercase type var in a user-generic constructor position rejects, not silently
           binds a fresh variable.")
  (input  (do (type Iter (FromList (List a)))
              (def (next (: it (Iter a))) it)
              (def (main) 0)
              (export main)))
  (error  CDZ0101))

(case "an integer literal in a type annotation's type position is rejected"
  (doc    "`(: 5 42)` puts the integer literal `42` — a VALUE, not a type — in type position. A value is
           not a type, so the annotation is meaningless and rejects (CDZ0203, 'expected a type'). Pins
           the non-name facet of the same missing validation: any non-type operand rejects, not just an
           unbound name. Accepted-and-ignored (ran to 5) before the type-operand check.")
  (input  (do (def (main) (: 5 42)) (export main)))
  (error  CDZ0203))

(case "a non-constructor type applied to arguments in type position is rejected"
  (doc    "`(: true (Int64 Int64))` applies `Int64` — which is NOT a type constructor (it takes no
           arguments) — to an argument, a malformed type expression. Were the operand simply `Int64`, the
           annotation would reject the Bool value `true`; instead the malformed application resolved to a
           non-type and was silently dropped, so `(: true (Int64 Int64))` ran to true. A non-constructor
           type applied to arguments must reject (CDZ0203, 'expected a type'). (An over/under-applied
           GENERIC type rejects via unification when the value forces it; this is the non-generic case.)")
  (input  (do (def (main) (: true (Int64 Int64))) (export main)))
  (error  CDZ0203))

(case "a monomorphic user sum applied to a type argument in an annotation is rejected"
  (doc    "The common sum-annotation slip: `(: t (T Int64))` where `(type T (Leaf Int64) (Node Int64))` is
           MONOMORPHIC — it takes no type parameters, so `(T Int64)` over-applies it. The reader parses
           `(T Int64)` as applying `T` to `Int64`; `T` reduces to a type-value with zero declared params,
           so it is over-applied and rejects CDZ0203. The bare `T` is the correct annotation (a monomorphic
           sum's type is just its name). Pins that a monomorphic USER SUM applied to a type argument is a
           coded rejection (the sum companion of the `(Int64 Int64)` case above — a non-generic type
           applied to arguments), NOT silently accepted; the diagnostic names the fix (`T`, not `(T …)`).")
  (input  (do
            (type T (Leaf Int64) (Node Int64))
            (def (f (: t (T Int64))) (match t ((T.Leaf n) n) ((T.Node n) n)))
            (def (main) (f (T.Leaf 5)))
            (export main)))
  (error  CDZ0203))

; The two cases above over-apply a NON-generic type to a TYPE argument (arity is wrong). The dual slip
; is a legitimately GENERIC type constructor — one that DOES take a type parameter — applied to a VALUE
; where a type belongs: `(Option 5)`, `(List 5)`. Here the arity is right (Option/List take one
; parameter) but the argument's KIND is wrong (a value, not a type). This is a distinct diagnostic
; (CDZ0203) that names the type-argument position — "the type argument must be a type, but a value
; appears here" — rather than the "not a type constructor" / "takes no type parameters" arity messages,
; so an author who wrote a value where the element/payload TYPE goes is told to write a type.

(case "a generic sum type constructor applied to a value is rejected"
  (doc    "`(: 1 (Option 5))` uses the generic type constructor `Option` — which correctly takes one type
           parameter — but supplies the VALUE `5` where a TYPE belongs. Unlike the `(Int64 Int64)` and `(T
           Int64)` cases above (a NON-generic type over-applied), the arity is right; the fault is the
           argument's kind. Rejected (CDZ0203), the diagnostic naming the type-argument position (`(Option
           <Type>)`, e.g. `(Option Int64)`). Pins that a generic type's argument must itself be a type,
           not a value.")
  (input  (do (def (main) (: 1 (Option 5))) (export main)))
  (error  CDZ0203))

(case "a list type whose element position holds a value is rejected"
  (doc    "`(: 1 (List 5))` puts the value `5` in `List`'s element-TYPE position. `List` is a generic type
           constructor whose one argument is the element TYPE — a value there is ill-formed (CDZ0203, 'the
           element type must be a type, but this is a value'). The built-in-collection companion of the
           `(Option 5)` case: a value where a type is required in a generic type's argument.")
  (input  (do (def (main) (: 1 (List 5))) (export main)))
  (error  CDZ0203))

(case "a set type whose element position holds a value is rejected"
  (doc    "`(: 1 (Set 5))` — the `Set` sibling of the list case: the element-TYPE position holds the value
           `5`, rejected (CDZ0203). Pins that the generic-argument-must-be-a-type check covers `Set` as
           well as `List`, so no built-in generic collection accepts a value in its type-argument slot.")
  (input  (do (def (main) (: 1 (Set 5))) (export main)))
  (error  CDZ0203))

(case "an unbound name as a parameter's annotation type is rejected"
  (doc    "The PARAMETER-annotation companion: `(def (f (: x foo)) x)` annotates the parameter `x` with
           the unbound `foo` in type position. A parameter's type operand must denote a type exactly as a
           value annotation's does, so the unbound `foo` rejects CDZ0101 (was accepted, `(f 7)` ran to 7
           — the garbage parameter type silently typed `x` as unconstrained). Pins that the type-operand
           validation covers a signature parameter, not only a value annotation.")
  (input  (do (def (f (: x foo)) x) (def (main) (f 7)) (export main)))
  (error  CDZ0101))

(case "a well-formed annotation still checks and accepts a matching type (the control)"
  (doc    "The control pinning the rejects above are about VALIDATING the type operand, not annotations
           in general: `(: 5 Int64)` matches the value's type and is accepted (5); a mismatch `(: 5 Bool)`
           still rejects CDZ0203; and a real parameter annotation `(: n Int64)` compiles. So the
           annotation machinery works for real types — the gap was specifically a NON-type operand
           accepted-and-ignored.")
  (input  (do (def (main) (: 5 Int64)) (export main)))
  (call   main)
  (output (: 5 Int64)))

; A value that ESCAPES to the host must have a FULLY DETERMINED type — a value whose payload/element
; type is an unresolved variable (a bare `(None)` : `(Option ?0)`) has no defined serialization. Such an
; escape is rejected for its AMBIGUOUS TYPE (CDZ0203, the type-determination fault — annotate to resolve
; it), NOT for its export SHAPE: `(def (main) (None)) (export main)` IS a single nullary export (the
; escape path's shape is satisfied), so a shape-restriction message would misdiagnose. The ambiguity
; bites ONLY at an unannotated escape — a CONSUMED bare `None` (matched, or passed to a typed parameter)
; constrains the payload and type-checks fine, and an ANNOTATED escape resolves the variable and crosses.

(case "an escaped value with an unresolved payload type is rejected as ambiguous, not for its export shape"
  (doc    "`(def (main) (None)) (export main)` returns a bare `None`, whose type is `(Option ?0)` — the
           payload is a free variable nothing constrains, so the escaped value has no defined
           serialization and is rejected (CDZ0203). The program IS a single nullary export, so the reject
           must name the UNRESOLVED TYPE and the annotation fix, not an export-shape restriction (the
           prior message wrongly said the sum 'crosses only as a single nullary export's result' — which
           it already is). An annotated `(: (None) (Option Int64))` escapes fine, and a consumed bare
           `None` type-checks — the ambiguity is escape-only.")
  (input  (do (def (main) (None)) (export main)))
  (error  CDZ0203))

(case "an annotated escaped None renders its canonical nullary-variant form (the control)"
  (doc    "The control pinning the reject above is ONLY the missing payload type: annotating the bare
           `None` to `(Option Int64)` fully determines the type, and it escapes as the program result,
           rendering the canonical `(None unit)` form. Same shape (a single nullary export returning a
           sum) as the rejected case — the only difference is the annotation resolves `?0`. Pins the
           escape path works once the payload type is known.")
  (input  (do (def (main) (: (None) (Option Int64))) (export main)))
  (output (: (None unit) (Option Int64))))

(case "a consumed bare None type-checks without annotation (ambiguity is escape-only)"
  (doc    "`(match (None) ((Some x) x) ((None) 42))` consumes a bare `None`: the match arms constrain the
           payload type variable, so no annotation is needed and the None arm yields 42. Pins that the
           unconstrained-payload rejection is specific to an unannotated ESCAPE — a consumed bare `None`
           is fine, triangulating that the escape reject is a payload-type-ambiguity condition, not an
           export-shape one.")
  (input  (do (def (main) (match (None) ((Some x) x) ((None) 42))) (export main)))
  (output (: 42 Int64)))

; The COLLECTION analogue of the bare-`None` escape above — with a DIFFERENT grounding. An empty `(list)`
; escaping to the host has type `(List Any)`: an empty collection has no element to constrain, so its
; element type GROUNDS to `Ty::Any` rather than staying a free variable like `None`'s `(Option ?0)`. It is
; the SAME undetermined-serialization fault (a value crosses the boundary with an element type no use
; fixed), so it MUST reject identically (CDZ0203, annotate). Before, the escape check tested only for a
; free `Var`, so the `Any`-grounded empty list SLIPPED PAST `cdz check` (exit 0) and hit an uncoded emit
; decline that misdescribed it as a runtime-collection-walker limitation — a check≡emit gap this closes by
; treating an `Any` element/payload as undetermined exactly as a free `Var` is. The ambiguity is
; escape-only: a CONSUMED empty list (`List.len (list)` → a scalar) and an ANNOTATED / determined-element
; list cross fine (their controls follow).

(case "an empty list escaping to the host is rejected as undetermined, like a bare None"
  (doc    "`(def (main) (list)) (export main)` returns an empty list of type `(List Any)` — the element
           type is undetermined (no element constrains it, so it grounds to `Any`, the collection analogue
           of bare `None`'s free-variable payload). The escaped value has no defined serialization and is
           rejected (CDZ0203, annotate — e.g. `(: (list) (List Int64))`). Pins that the undetermined-escape
           reject catches the `Any` grounding, not only a free `Var`; a `Set.of (list)` is the same fault.")
  (input  (do (def (main) (list)) (export main)))
  (error  CDZ0203))

(case "an annotated empty list escapes fine (the undetermined-empty-list control)"
  (doc    "The control pinning the reject above is ONLY the undetermined element type: annotating the empty
           list to `(List Int64)` fully determines it, and it escapes as the program result. Same shape (a
           single nullary export returning a list) as the rejected case — the annotation resolves the
           element. Pins the escape path works once the element type is known, mirroring the annotated-None
           control.")
  (input  (do (def (main) (: (list) (List Int64))) (export main)))
  (output (: (list) (List Int64))))

(case "a consumed empty list type-checks without annotation (ambiguity is escape-only)"
  (doc    "`(List.len (list))` consumes an empty list to a scalar `Int64` (0): the result that escapes is a
           determined scalar, not the undetermined `(List Any)`, so no annotation is needed. Pins that the
           undetermined-`Any` rejection is specific to an unannotated ESCAPE of the collection itself — a
           consumed empty list is fine — the collection analogue of the consumed-bare-None case.")
  (input  (do (def (main) (List.len (list))) (export main)))
  (output (: 0 Int64)))

; The annotation-contradiction check must hold for a COMPOUND value too, not only a scalar. A tuple /
; sum / record / list is not a scalar type, so annotating one with a scalar type (Int64, Bool, …)
; contradicts the value's type and MUST be rejected (CDZ0203, type-system.md #Annotations Constrain,
; Never Contradict).

(case "a tuple annotated as a scalar type is rejected"
  (doc    "`(: (tuple 1 2) Int64)` annotates a tuple with the scalar type Int64 — a contradiction (a
           tuple is not an Int64), so the compiler rejects it (CDZ0203), or declines if it does not yet
           cover the compound-vs-scalar annotation rule (reject-don't-miscompile).")
  (input  (: (tuple 1 2) Int64))
  (error  CDZ0203))

(case "a sum value annotated as a scalar type is rejected"
  (doc    "The sum companion: `(: (Some 5) Bool)` annotates an Option value with the scalar type Bool
           — a contradiction (CDZ0203). Pins that the annotation check covers a compound value on the
           value side, not only a scalar.")
  (input  (: (Some 5) Bool))
  (error  CDZ0203))

; The annotation check must also see a mismatch in the PARAMETER of a compound type, not only at the
; head. `(Some true)` has type `Option Bool`, which cannot unify with `Option Int64` — the head
; constructor `Option` agrees but the payload type does not, so the annotation contradicts the value's
; type and MUST be rejected (CDZ0203, type-system.md #Annotations Constrain, Never Contradict: "A
; program whose annotation cannot be unified with the type inference determines MUST be rejected").
; An annotation checker that unifies only the head constructor and ignores the type parameter would
; ACCEPT this ill-typed program and run it, returning `(Some true)` under a declared `Option Int64` —
; the silent annotation-replaces-inference the section forbids. A generation that does not yet cover
; the payload-level check DECLINES (reject-don't-miscompile); accepting the program is the failure.

(case "an option value annotated with the wrong payload type is rejected"
  (doc    "`(: (Some true) (Option Int64))` annotates a `Some true` (type `Option Bool`) as `Option
           Int64`: the head `Option` matches but the payload `Bool` cannot unify with `Int64`, a
           contradiction (CDZ0203). Pins that the annotation check descends into a compound type's
           PARAMETER, not only its head constructor — a checker that stops at the head silently accepts
           the ill-typed program and runs it, returning `(Some true)` under a wrong declared type
           (type-system.md #Annotations Constrain, Never Contradict). A generation that does not yet
           cover the payload-level check declines rather than accepting (reject-don't-miscompile).")
  (input  (: (Some true) (Option Int64)))
  (error  CDZ0203))

; The payload-parameter check must RECURSE, at every nesting depth, not only one level down. `(Some (Some
; 5))` has type `Option (Option Int64)`; annotated `Option (Option Bool)`, the outer `Option` and the
; inner `Option` heads agree but the innermost payload `Int64` cannot unify with `Bool` — a contradiction
; two levels deep. It is the same rule as the one-level `(: (Some true) (Option Int64))` case above, so it
; MUST be rejected (CDZ0203). A checker that descends ONE level into the type parameter but compares the
; nested payload only by coarse kind (both are `Option`) accepts the ill-typed program and runs it — the
; deeper-nesting analogue of the head-only gap the one-level case closed. A generation that does not yet
; recurse into the nested parameter declines rather than accepting (reject-don't-miscompile).

(case "a nested option value annotated with the wrong inner payload type is rejected"
  (doc    "`(: (Some (Some 5)) (Option (Option Bool)))` annotates a value of type `Option (Option Int64)`
           as `Option (Option Bool)`: both `Option` heads agree, but the innermost payload `Int64` cannot
           unify with `Bool` — a contradiction two levels deep (CDZ0203), the same rule as the one-level
           `(: (Some true) (Option Int64))` case above. Pins that the annotation's payload check RECURSES
           to any depth, not only one level — a checker that stops after one descent silently accepts the
           ill-typed program and runs it, returning `(Some (Some 5))` under a wrong declared inner type. A
           generation that does not yet recurse into the nested parameter declines rather than accepting.")
  (input  (: (Some (Some 5)) (Option (Option Bool))))
  (error  CDZ0203))

; Type-checking a DEEPLY-nested generic-sum VALUE must not blow up superlinearly. Each enclosing `(Some x)`
; unifies its payload variable against the (growing) `Option^k Int64` type below it, and the HM occurs-check
; run on that unification used to re-apply the whole substitution at every node — O(size²) per check,
; O(N³) over the N-deep chain (depth 400 = 2.5s, extrapolating to a compile hang around depth ~1500). Walking
; the type through the substitution in place (the standard union-find resolve) makes the occurs-check O(size),
; so the whole nested value is ~quadratic and a linear-size program compiles in linear-ish time. This case
; pins the VALUE compiles to the right answer at a depth (60) that the cubic version already handled but that
; anchors the shape; the pathology it guards is the GROWTH RATE, not this one point. A deep type ANNOTATION
; and a deep nested TUPLE value were already linear — the blowup was specific to the generic-sum constructor.

(case "a deeply-nested generic-sum value type-checks and matches its outermost variant"
  (doc    "A `(Some (Some … (Some 5)))` chain nested 60 deep, matched on its outermost `Some` (returning 1).
           The emitted program is tiny, but type-checking the nested generic-sum constructor applications was
           O(N³) (the HM occurs-check re-applied the full substitution at every node, O(size²) per check, over
           N levels), so a deeper chain hung the compiler. Walking variables through the substitution in place
           makes the occurs-check O(size) and the whole value ~quadratic. A deep type annotation alone and a
           deep nested tuple value were already linear, so the blowup was specific to the generic-sum value.
           The outer match returns 1; the point is that PRODUCING the deep value must not be superlinear.")
  (input  (do
            (def (main)
              (match
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some
                (Some (Some (Some (Some (Some (Some (Some (Some (Some (Some 5))))))))))
                )))))))))) )))))))))) )))))))))) )))))))))) ))))))))))
                ((Some inner) 1)
                ((None)       0)))
            (export main)))
  (call   main)
  (output (: 1 Int64)))

; The parameter check applies to a LIST's element type too, not only a sum's payload. `(list 1 2)` has
; type `List Int64`; annotated `List Bool`, the head `List` agrees but the element type `Int64` cannot
; unify with `Bool` — a contradiction (CDZ0203), the list analogue of the `Option` payload case. A checker
; that verifies only the head `List` and ignores the element parameter accepts the ill-typed program and
; runs it, returning `(list 1 2)` under a declared `List Bool`. (A list's elements share one type — the
; homogeneity rule — so a single provable element type suffices to contradict the annotation.)

(case "a list annotated with the wrong element type is rejected"
  (doc    "`(: (list 1 2) (List Bool))` annotates a `List Int64` as `List Bool`: the head `List` matches
           but the element type `Int64` cannot unify with `Bool`, a contradiction (CDZ0203), the list
           companion of the option-payload case above. Pins that the annotation's parameter check covers a
           list's element type, not only a sum's payload — a checker that stops at the head `List` silently
           accepts the ill-typed program and runs it. A generation that does not yet check the element
           parameter declines rather than accepting (reject-don't-miscompile).")
  (input  (: (list 1 2) (List Bool)))
  (error  CDZ0203))

; The parameter check applies to a RECORD's field type too, not only a sum's payload or a list's
; element. `(record (a 1))` has type `(Record (a Int64))`; annotated `(Record (a Bool))`, the head
; `Record` and the field name `a` agree but the field's type `Int64` cannot unify with `Bool` — a
; contradiction (CDZ0203), the record analogue of the list-element and option-payload cases above. A
; record's fields are the third structural type (type-system.md #The Structural Types Are Record, Tuple,
; And Sum) beside the tuple's positions and the sum's payload; the annotation-parameter check the cases
; above pin for a tuple position, a sum payload, and a list element MUST also cover a record field, or a
; checker that verifies only the head `Record` and the field NAMES silently accepts the ill-typed
; program and runs it, returning `(record (a 1))` under a declared `(Record (a Bool))` — the same
; annotation-replaces-inference the section forbids. A generation that does not yet check a record
; field's type parameter declines rather than accepting (reject-don't-miscompile).

(case "a record annotated with the wrong field type is rejected"
  (doc    "`(: (record (a 1)) (Record (a Bool)))` annotates a `(Record (a Int64))` as `(Record (a Bool))`:
           the head `Record` and the field name `a` match but the field's type `Int64` cannot unify with
           `Bool`, a contradiction (CDZ0203), the record companion of the list-element and option-payload
           cases above. Pins that the annotation's parameter check covers a record's field type — the
           third structural type beside a tuple's positions and a sum's payload — not only a sum's payload
           or a list's element. A checker that stops at the head `Record` and the field names silently
           accepts the ill-typed program and runs it, returning `(record (a 1))` under a declared
           `(Record (a Bool))` (type-system.md #Annotations Constrain, Never Contradict). A generation
           that does not yet check a record field's type declines rather than accepting
           (reject-don't-miscompile).")
  (input  (: (record (a 1)) (Record (a Bool))))
  (error  CDZ0203))

; --- A record's field SET must match the annotation, not only each field's type ------------
; The case above matches the field NAMES and rejects a wrong field TYPE. The dual failure is a field-SET
; mismatch: the value's set of field names differs from the annotation's — a MISSING field (the value
; lacks one the annotation names) or an EXTRA field (the value carries one the annotation does not). A
; record's shape is its fixed set of named fields (type-system.md #A Record Has A Fixed Set Of Named
; Fields), so `(Record (a Int64))` and `(Record (a Int64) (b Int64))` are DIFFERENT types — the check is
; over the whole field set, not a subset/superset relaxation (that widening is row polymorphism, a
; separate opt-in — 15-rows-and-open-sums). Each mismatch is CDZ0203 and the diagnostic names the
; offending field (missing `b` / no such field `c`), the actionable add-missing / delete-extra repair.

(case "a record missing a field the annotation names is rejected"
  (doc    "`(: (record (a 1)) (Record (a Int64) (b Int64)))` annotates a one-field record as a two-field
           type — the value is MISSING field `b`. The field sets differ, so the value's type `(Record (a
           Int64))` does not match the annotation `(Record (a Int64) (b Int64))` (CDZ0203, naming the
           missing `b`). A record type is not satisfied by a value carrying a subset of its fields — field
           presence is static (the row-poly widening that would accept this is a separate opt-in). The
           field-SET companion of the wrong-field-TYPE case above.")
  (input  (: (record (a 1)) (Record (a Int64) (b Int64))))
  (error  CDZ0203))

(case "a record carrying a field the annotation does not name is rejected"
  (doc    "The dual: `(: (record (a 1) (b 2) (c 3)) (Record (a Int64) (b Int64)))` carries an EXTRA field
           `c` the annotation does not name. The field sets differ, so it is rejected (CDZ0203, 'no such
           field `c` on the expected record'). A record value is not accepted against a type with FEWER
           fields — the extra field is not silently dropped. Pins the superset direction of the field-set
           check (the value has more fields than the type).")
  (input  (: (record (a 1) (b 2) (c 3)) (Record (a Int64) (b Int64))))
  (error  CDZ0203))

(case "a record whose field is misnamed is both missing and extra"
  (doc    "`(: (record (a 1) (x 2)) (Record (a Int64) (b Int64)))` names its second field `x` where the
           annotation expects `b` — so relative to the annotation the value is simultaneously MISSING `b`
           and carrying an EXTRA `x`. Rejected (CDZ0203) with both faults named ('missing field `b`; no
           such field `x`'). Pins that a single misnamed field surfaces as the combined field-set
           mismatch, the shape a field-name typo takes.")
  (input  (: (record (a 1) (x 2)) (Record (a Int64) (b Int64))))
  (error  CDZ0203))

; The variant-payload TYPE check must fire wherever the constructor appears — including as the direct
; scrutinee of a `match`. A sum's shape is "its variant names with their payload types" (type-system.md
; #The Structural Types Are Record, Tuple, And Sum), and "a value of a sum type MUST be constructed
; through one of its variants" (§A Value Of A Sum Type Is Constructed Through A Variant), so constructing
; `(I true)` under `(type N (I Int64) (J Int64))` — where `I`'s payload type is Int64 and `true` is Bool
; — is ill-typed and MUST be rejected (CDZ0201), no matter the surrounding context. The seed rejects it
; in EVERY position — bare `(I true)`, let-bound `(let ((n (I true))) n)`, as a function argument, when
; annotated `(: (I true) N)`, and over-applied `(I 5 6)` — EXCEPT when the constructor is the direct
; scrutinee of a match: `(match (I true) ((I x) x) ((J y) y))` type-checks the constructor's payload
; NOT AT ALL and runs, binding `x` to the Bool `true` and returning it — an ill-typed value (`true`
; where the arm's Int64 payload is expected) crossing the run boundary. It is a wrong VALUE, not merely
; a missed rejection: `x` is the payload of `I Int64`, so the arm's result is Int64, yet the program
; returns Bool `true`. The match's scrutinee position suppresses the payload check that every other
; position performs — the master-pattern gap (a check proven on operand/arg/let/annotation positions
; not carried to the match-scrutinee position). A generation that checks the scrutinee's payload declines
; the ill-typed program rather than running it and returning a Bool where an Int64 is required.
(case "a variant with a wrong-type payload as a direct match scrutinee is a type error"
  (doc    "`(match (I true) ((I x) x) ((J y) y))` under `(type N (I Int64) (J Int64))` matches a
           constructor `(I true)` whose payload `true` is Bool where `I`'s declared payload type is Int64
           — ill-typed exactly as the bare `(I true)` is (rejected in every other position: bare,
           let-bound, as an argument, annotated, over-applied). MUST be rejected (CDZ0201). Pins that the
           variant-payload type check fires in the DIRECT match-scrutinee position too, not only in
           construction/binding positions. The seed suppresses the check here and runs the program,
           binding `x` (declared payload type Int64) to the Bool `true` and returning it — an ill-typed
           Bool crossing the run boundary where the arm's Int64 payload is required, a wrong value. The
           companion `(let ((n (I true))) (match n …))` (a let-bound, then matched, scrutinee) IS rejected;
           only the constructor written directly in scrutinee position slips. A generation that type-checks
           the scrutinee constructor's payload declines rather than running the mistyped program.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (main) (match (I true) ((I x) x) ((J y) y))) (export main)))
  (error  CDZ0201))

; The annotation check must also catch a tuple's ARITY mismatch, not only its element TYPES. A tuple is
; a fixed-size positional value "whose length is part of its type" (type-system.md #A Tuple Is Reshaped
; Positionally …), and a structural type's shape is "a tuple's element types in order" (#The Structural
; Types Are Record, Tuple, And Sum) — so a two-element tuple has type `(Tuple Int64 Int64)`, which cannot
; unify with a three-element `(Tuple Int64 Int64 Int64)` any more than `(Tuple Int64 Bool)` (a wrong
; element type) can. `(: (tuple 1 2) (Tuple Int64 Int64 Int64))` is therefore a contradiction the compiler
; MUST reject (CDZ0203, #Annotations Constrain, Never Contradict). A checker that walks the annotation's
; element types POSITIONALLY against the value's elements but never compares the two ARITIES silently
; accepts the ill-typed program and runs it, returning `(tuple 1 2)` under a declared three-element type —
; the arity companion of the wrong-element-type gap the list/record/sum cases close. (The element-type
; check already fires: `(: (tuple 1 2) (Tuple Int64 Bool))` is rejected "annotation's parameter type
; contradicts the value"; the arity check must reach the same annotation the element check does.)

(case "a tuple annotated with the wrong arity is rejected"
  (doc    "`(: (tuple 1 2) (Tuple Int64 Int64 Int64))` annotates a two-element tuple (type `(Tuple Int64
           Int64)`) as a THREE-element tuple type: a tuple's length is part of its type (type-system.md
           #A Tuple Is Reshaped Positionally …, #The Structural Types Are Record, Tuple, And Sum), so the
           two arities cannot unify — a contradiction (CDZ0203), the arity companion of the wrong-element-
           type cases above. Pins that the annotation check compares a tuple's ARITY, not only its element
           types positionally — a checker that walks the shared positions and ignores the length silently
           accepts the ill-typed program and runs it, returning `(tuple 1 2)` under a declared three-
           element type. The element-type check already fires (`(: (tuple 1 2) (Tuple Int64 Bool))` is
           rejected), so the arity check must reach the same annotation. A generation that does not yet
           check tuple arity declines rather than accepting (reject-don't-miscompile).")
  (input  (: (tuple 1 2) (Tuple Int64 Int64 Int64)))
  (error  CDZ0203))

(case "a tuple annotated with too many elements is rejected"
  (doc    "The other arity direction: `(: (tuple 1 2 3) (Tuple Int64 Int64))` annotates a THREE-element
           tuple as a two-element type — the value has MORE positions than the annotation. Rejected
           (CDZ0203, 'expected a tuple with 2 elements, but this one has 3'), the too-many companion of the
           too-few case above. Pins that the arity check catches a surplus element as well as a missing
           one (the tuple analog of the record extra-field case), so a tuple's length must match exactly in
           both directions — not merely be at-least or at-most the annotation's.")
  (input  (: (tuple 1 2 3) (Tuple Int64 Int64)))
  (error  CDZ0203))

(case "an unannotated program with a valid typing type-checks and runs"
  (doc    "Witnesses type-system.md #An Unannotated Program Is Accepted When It Has A Valid Typing: a
           valid typing need not be written by the author; the program type-checks and evaluates to 3.")
  (input  (let ((x 1)) (+ x 2)))
  (output (: 3 Int64)))

(case "an operation on mismatched types is rejected at compile time"
  (doc    "Witnesses type-system.md #A Well-Typed Program Does Not Go Wrong via its contrapositive:
           the ill-typed `(+ 1 \"two\")` is caught and rejected (CDZ0201) rather than run.")
  (input  (+ 1 "two"))
  (error  CDZ0201))

; --- Arithmetic is not defined on a non-numeric type, even when both operands SHARE it ------
; The case above mixes DIFFERENT kinds (Int64 vs String). Arithmetic is also rejected when BOTH
; operands are the SAME non-numeric type — `(+ s t)` on two Strings, `(+ xs ys)` on two Lists, `(+ r
; q)` on two Records/Sets: the numeric operators offer arithmetic over the NUMERIC types only
; (numeric-model.md #Numeric ...), and Cadenza never coerces text or a compound to a number. This is a
; distinct path from the cross-kind mismatch — the two operands unify (they are one type), so the
; cross-kind guard does not fire; the operator's numeric requirement is what rejects it (CDZ0201,
; "arithmetic is not defined on <T>"). It is a genuine type error, not a phantom-`Int64` internal clash
; misattributed to the second operand. (`+` on a type with a total concatenation — String/List/Bytes —
; additionally carries a `((. List concat) …)` fix suggestion; the behavior pinned here is the
; rejection, whichever repair the diagnostic offers.)

(case "addition of two strings is rejected — text is not a number"
  (doc    "`(+ \"ab\" \"cd\")` adds two Strings; arithmetic is not defined on text (Cadenza never coerces
           a String to a number), so the compiler rejects it (CDZ0201). Unlike `(+ 1 \"two\")` the two
           operands are the SAME type — they unify, so this is not the cross-kind mismatch but the
           operator's numeric-operand requirement. Pins the honest 'not defined on String' rejection (the
           `+`-means-concat reflex is a fix suggestion, not an accepted meaning).")
  (input  (+ "ab" "cd"))
  (error  CDZ0201))

(case "subtraction of two strings is rejected"
  (doc    "`(- \"ab\" \"cd\")` — a non-`+` arithmetic operator on two Strings has no concatenation reading
           at all and is rejected (CDZ0201), the same 'arithmetic is not defined on String'. Pins that the
           rejection covers the whole arithmetic family on text, not only `+` (which merely additionally
           offers a concat fix).")
  (input  (- "ab" "cd"))
  (error  CDZ0201))

(case "addition of two lists is rejected — a list is not a number"
  (doc    "`(+ (list 1 2) (list 3 4))` adds two Lists of the same type; arithmetic is not defined on a
           List (CDZ0201). Both operands share the type `(List Int64)`, so this is the same-type path, not
           a cross-kind mismatch. Pins that a compound collection is not silently a number — the author
           who meant concatenation is offered the `List.concat` rewrite, not an implicit `+`.")
  (input  (+ (list 1 2) (list 3 4)))
  (error  CDZ0201))

(case "addition of two records is rejected — a record is not a number"
  (doc    "`(+ r q)` on two same-typed Records is rejected (CDZ0201, 'arithmetic is not defined on
           (Record (a Int64))'). A Record has no concatenation, so — unlike String/List — no fix is
           offered, only the honest message. Pins that the numeric-operand requirement rejects a compound
           record, the companion of the list case with no concat rewrite. Runtime operands (parameters),
           so the fault is the operator's, not a fold.")
  (input  (do
            (def (f (: r (Record (a Int64))) (: q (Record (a Int64)))) (+ r q))
            (def (main) (f (record (a 1)) (record (a 2)))) (export main)))
  (error  CDZ0201))

(case "addition of two sets is rejected — a set is not a number"
  (doc    "`(+ r q)` on two same-typed Sets is rejected (CDZ0201, 'arithmetic is not defined on (Set
           Int64)'). A set's combination is `Set.union`, not `+`; the numeric operator does not accept it.
           Pins the compound-collection rejection for Set (the sibling of the list and record cases), so
           `+` is never quietly overloaded to a set operation.")
  (input  (do
            (def (f (: r (Set Int64)) (: q (Set Int64))) (+ r q))
            (def (main) (f (Set.of (list 1)) (Set.of (list 2)))) (export main)))
  (error  CDZ0201))

(case "addition of two symbols is rejected — a symbol is not a number"
  (doc    "`(+ (Symbol.of \"a\") (Symbol.of \"b\"))` adds two Symbols; a Symbol is an interned name, not a
           number, so arithmetic is not defined on it (CDZ0201). The two operands share the type `Symbol`,
           so this is the same-type path, not a cross-kind mismatch — and a Symbol has no concatenation, so
           the honest message stands with no fix. Pins that the nominal name type is excluded from
           arithmetic exactly as the collections are.")
  (input  (+ (Symbol.of "a") (Symbol.of "b")))
  (error  CDZ0201))

(case "subtraction of two symbols is rejected"
  (doc    "`(- (Symbol.of \"a\") (Symbol.of \"b\"))` — the non-`+` arithmetic operator on two Symbols is
           rejected the same way (CDZ0201). Pins that the whole arithmetic family, not only `+`, refuses a
           Symbol operand (the Symbol companion of the two-strings pair above).")
  (input  (- (Symbol.of "a") (Symbol.of "b")))
  (error  CDZ0201))

; --- Arithmetic on a MISMATCHED non-numeric pair names BOTH real types -----------------------
; The cases above add two operands of the SAME non-numeric type. When the two operands are DIFFERENT
; non-numeric types — `(+ "ab" (list 1 2))`, a String and a List — the diagnostic names BOTH real types
; ("arithmetic is not defined on String and (List Int64)"), not a phantom `Int64` misattributed to one
; side. This is a THIRD path, distinct both from the same-type case (which names one type) and from the
; cross-KIND mismatch `(+ 1 "two")` above (which has a NUMERIC operand and reports "different types
; across that kind boundary"): here NEITHER operand is numeric, so the fault is squarely the operator's
; numeric requirement over an unrelated pair. The message is order-sensitive (it lists the operands as
; written), so both orders are pinned.

(case "addition of a string and a list names both non-numeric types"
  (doc    "`(+ \"ab\" (list 1 2))` adds a String and a List — two DIFFERENT non-numeric types. The
           compiler rejects it (CDZ0201) and names both real types rather than inventing a phantom
           `Int64` for one side; neither operand is a number, so the operator's numeric requirement is
           what fails. Pins the mismatched-pair diagnostic (distinct from the same-type cases and from the
           `(+ 1 \"two\")` cross-kind case, which has a numeric side).")
  (input  (+ "ab" (list 1 2)))
  (error  CDZ0201))

(case "the mismatched-pair rejection is order-independent"
  (doc    "`(+ (list 1 2) \"ab\")` — the operands of the case above flipped — is the same rejection
           (CDZ0201). The diagnostic lists the operands as written (List then String), but the fault does
           not depend on which non-numeric type is on which side. Pins that the mismatched-non-numeric-pair
           check is symmetric.")
  (input  (+ (list 1 2) "ab"))
  (error  CDZ0201))

(case "addition of a string and a byte sequence names both text types"
  (doc    "`(+ \"ab\" (Bytes.of (list 1)))` adds a String and a Bytes — two different text-ish types,
           both non-numeric, rejected (CDZ0201) naming both. Pins that the mismatched-pair diagnostic
           covers a text/text pair (String vs Bytes) as well as a text/collection pair, so no non-numeric
           combination is silently treated as arithmetic.")
  (input  (+ "ab" (Bytes.of (list 1))))
  (error  CDZ0201))

; --- Equality type-checks its operands: no cross-type comparison ---------------------------
; `=` offers structural equality over ONE type's values (type-system.md #Structural Values Are
; Comparable Only When Their Shapes Match), so comparing two DIFFERENT types is a type error — the
; same operand-typing rule the ordering operators and arithmetic obey. Two different NUMERIC types
; (Int64 vs Float64) is the silent-promotion the numeric model forbids (numeric-model.md #Numeric
; Types Do Not Silently Promote), rejected CDZ0301 exactly as `(+ 5 2.0)` and `(< 5 2.0)` are; Int64
; vs Bool is a number-vs-boolean error (CDZ0203); Int64 vs String (either order) is a general cross-
; kind error (CDZ0201). These pin the equality companions the ordering-operator section below cites
; by name — `=` is the operator those cases are measured against, so it must itself be pinned.

(case "equality of an integer and a float is rejected, not silently promoted"
  (doc    "`(= 5 2.0)` compares an Int64 and a Float64 — the numeric no-promotion rule (numeric-model.md
           #Numeric Types Do Not Silently Promote) applies to `=` as to `+` and `<`, so the compiler
           rejects it (CDZ0301) rather than promoting 5 to 5.0 and answering. The companion the ordering
           cases below cite as `(= 5 2.0)` → CDZ0301.")
  (input  (= 5 2.0))
  (error  CDZ0301))

(case "equality of an integer and a boolean is a type error"
  (doc    "`(= 1 true)` compares an Int64 with a Bool — a number and a boolean, unrelated kinds with no
           shared value space, rejected (CDZ0203). The equality companion of `(< 1 true)` → CDZ0203; `=`
           is not a coercion to a common type.")
  (input  (= 1 true))
  (error  CDZ0203))

(case "equality of an integer and a string is a type error"
  (doc    "`(= 1 \"x\")` compares an Int64 with a String — two different types across a kind boundary,
           rejected (CDZ0201). The equality companion of `(< 1 \"x\")` → CDZ0201; `=` never silently
           compares representations across types.")
  (input  (= 1 "x"))
  (error  CDZ0201))

(case "equality across types is rejected regardless of operand order"
  (doc    "The order-flipped companion: `(= \"x\" 1)` is the same cross-type comparison (String vs Int64)
           and rejected (CDZ0201), mirroring the flipped ordering case `(> \"x\" 1)`. Pins that the
           operand-type check does not depend on which side carries which type.")
  (input  (= "x" 1))
  (error  CDZ0201))

; --- Equality of two SAME-KIND compounds of different STRUCTURE names the structural delta -----
; The cases above compare across different KINDS (Int64 vs String). Two compounds of the SAME kind but
; a DIFFERENT structure — two records with different field SETS, two tuples of different ARITY — are also
; different types (type-system.md #Structural Values Are Comparable Only When Their Shapes Match: a
; record's shape is its field set, a tuple's is its arity), so `=` over them is a type error (CDZ0203).
; The diagnostic names the structural DELTA (a missing/extra field, an arity difference), not a raw
; 'these two Records differ' — the same minimal-conflict hint the annotation field-set cases carry, at
; the operator-argument position. These pin the same-kind-different-shape facet of equality's operand
; typing, the companion of the cross-kind cases above.

(case "equality of two records with different field sets is a type error"
  (doc    "`(= (record (x 1)) (record (y 2)))` compares a `(Record (x Int64))` with a `(Record (y Int64))`
           — same KIND (both records) but different field SETS, so different types (a record's shape is its
           field set). Rejected CDZ0203, the diagnostic naming the delta (missing `x`; no such field `y`).
           Pins that `=` requires matching field sets, not merely that both operands are records — the
           structural companion of the cross-kind `(= 1 \"x\")` case.")
  (input  (= (record (x 1)) (record (y 2))))
  (error  CDZ0203))

(case "equality of two tuples of different arity is a type error"
  (doc    "`(= (tuple 1 2) (tuple 1 2 3))` compares a 2-tuple with a 3-tuple — same kind, different arity,
           so different types (a tuple's arity is part of its type). Rejected CDZ0203, naming the arity
           delta (expected 2 elements, has 3). Pins that `=` over tuples requires equal arity, the tuple
           companion of the record-field-set case.")
  (input  (= (tuple 1 2) (tuple 1 2 3)))
  (error  CDZ0203))

(case "equality of a record with a subset of another's fields is a type error"
  (doc    "`(= (record (x 1)) (record (x 1) (y 2)))` — one record's fields are a SUBSET of the other's, but
           a subset is still a DIFFERENT field set (row polymorphism, which would relate them, is a
           separate opt-in — 15-rows-and-open-sums). Rejected CDZ0203 (no such field `y` on the smaller
           record's type). Pins that `=` is not silently widened to ignore the extra field, the subset
           facet of the field-set check.")
  (input  (= (record (x 1)) (record (x 1) (y 2))))
  (error  CDZ0203))

; --- The comparison operators type-check their operands exactly as = and + do -------------
; An ordering comparison (`<` `>` `<=` `>=`) offers a total order over ONE type's values
; (core-semantics.md #Ordering Where Offered Is Total; type-system.md #Structural Values Are
; Comparable Only When Their Shapes Match). Comparing two DIFFERENT numeric types is the same
; silent-promotion the arithmetic operators forbid (numeric-model.md #Numeric Types Do Not
; Silently Promote), so `(< 5 2.0)` is rejected (CDZ0301) exactly as `(+ 5 2.0)` and `(= 5 2.0)`
; are — an ordering is not a licence to promote Int64 to Float64 where + may not. Comparing two
; UNRELATED kinds has no shared order at all: Int64 vs Bool is a number-vs-boolean error (CDZ0203,
; exactly as `(= 1 true)` is), Int64 vs String is a general cross-kind error (CDZ0201, as `(= 1 "x")`
; is). These pin that the ordering operators are held to
; the SAME operand-typing rule as equality and arithmetic — a comparison must not be the one
; arithmetic-shaped operator that silently accepts a cross-type pair (the compiler either rejects
; with the code below or, for a rule it does not yet cover, declines rather than comparing across
; types — reject-don't-miscompile).

(case "ordering an integer against a float is rejected, not silently promoted"
  (doc    "`(< 5 2.0)` compares an Int64 and a Float64 — the numeric no-promotion rule the
           arithmetic operators obey applies to the ordering operators too, so the compiler rejects
           it (CDZ0301) rather than promoting 5 to 5.0 and answering. The passing companions are
           `(+ 5 2.0)` → CDZ0301 and `(= 5 2.0)` → CDZ0301; `<` must be held to the same rule.")
  (input  (< 5 2.0))
  (error  CDZ0301))

(case "greater-than of an integer and a float is rejected"
  (doc    "The `>` companion: `(> 5 2.0)` mixes Int64 and Float64, rejected (CDZ0301) like `<`.
           Pins that the no-promotion check covers `>`, not only `<`.")
  (input  (> 5 2.0))
  (error  CDZ0301))

(case "less-than-or-equal of an integer and a float is rejected"
  (doc    "The `<=` companion: `(<= 5 2.0)` mixes two numeric types, rejected (CDZ0301). Pins the
           check for the inclusive ordering operator.")
  (input  (<= 5 2.0))
  (error  CDZ0301))

(case "greater-than-or-equal of an integer and a float is rejected"
  (doc    "The `>=` companion: `(>= 5 2.0)` mixes two numeric types, rejected (CDZ0301). Completes
           the four ordering operators against the no-promotion rule.")
  (input  (>= 5 2.0))
  (error  CDZ0301))

(case "ordering an integer against a boolean is a type error"
  (doc    "`(< 1 true)` compares an Int64 with a Bool — unrelated kinds with no shared order, a
           general type error the compiler rejects (CDZ0203), exactly as `(= 1 true)` is. An
           ordering operator is not a coercion to a common type; a Bool has no position in Int64's
           order.")
  (input  (< 1 true))
  (error  CDZ0203))

(case "ordering an integer against a string is a type error"
  (doc    "`(< 1 \"x\")` compares an Int64 with a String — two different types, rejected (CDZ0201)
           like the equality companion `(= 1 \"x\")`. Pins that the ordering operators reject a
           cross-kind comparison rather than declining silently or comparing representations.")
  (input  (< 1 "x"))
  (error  CDZ0201))

(case "ordering a string against an integer is a type error regardless of operand order"
  (doc    "The order-flipped companion: `(> \"x\" 1)` is the same cross-type comparison (String vs
           Int64) and rejected (CDZ0201). Pins that the operand-type check does not depend on which
           side carries which type.")
  (input  (> "x" 1))
  (error  CDZ0201))

(case "Type is a first-class value"
  (doc    "Witnesses core-semantics.md #Types Are First-Class Values (1st sentence): a Type can be
           bound to a name, passed as an argument, returned from a function. A Type is an ordinary
           first-class value whose type is the type of types (type-system.md #Types Are First-Class
           Values Whose Type Is The Type Of Types). Here `Int64` is bound to `t` and RETURNED — the
           value the program produces is that type-value, which crosses the boundary as `(: Int64 Type)`
           (the type of a type-value is `Type`). A type-value is fully compile-time-known, so its
           boundary form is baked from the reduced type — it flows OUT of a nullary export directly,
           never from runtime data (a parameterized or not-fully-determined type has no boundary form
           and is rejected).")
  (input  (let ((t Int64)) t))
  (output (: Int64 Type)))

(case "a consistent annotation type-checks against the inferred type"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict and #A Well-Typed Program
           Does Not Go Wrong: `(: (+ 1 2) Int64)` type-checks because inference determines the
           expression's type is Int64 and the annotation unifies with it, so the program compiles and
           evaluates to 3. The passing companion to the CDZ0203 rejections above.")
  (input  (: (+ 1 2) Int64))
  (output (: 3 Int64)))

; --- The compiler never crashes: a malformed core form is rejected, not a panic ----------
; A core special form applied with the wrong number of operands (`(if true)`, `(= 5)`, a `let` binding
; with no value, an empty `(quote)`, a bare tuple accessor) is not a program the compiler can compile —
; but it is still INPUT the compiler is handed, and the compiler MUST NOT crash on it
; (self-hosting-and-bootstrap.md §"An Unsupported Construct Is Declined, Not Miscompiled" — the compiler
; declines or rejects; it never panics; the self-hosting fixpoint requires the compiler to be a total
; function over its input bytes). An ill-formed program's outcome is a rejection with the general
; ill-formed-program code CDZ0201 — never a crash, and never a value.

(case "a conditional with a missing branch is rejected, not a crash"
  (doc    "`(if <cond> <then>)` with no else branch is ill-formed: `if` requires condition, then, and
           else. The compiler rejects it (CDZ0201), never panicking while reaching for the absent third
           operand.")
  (input  (if true 1))
  (error  CDZ0201))

(case "a bare conditional keyword is rejected, not a crash"
  (doc    "`(if)` with no operands at all is ill-formed. The compiler rejects it, never indexing past
           the end of the operand list.")
  (input  (if))
  (error  CDZ0201))

(case "equality applied to one operand is rejected, not a crash"
  (doc    "`(= 5)` supplies one operand to a two-operand equality. The compiler rejects it (CDZ0201),
           never panicking reaching for the missing second operand.")
  (input  (= 5))
  (error  CDZ0201))

(case "a bare equality keyword is rejected, not a crash"
  (doc    "`(=)` with no operands is ill-formed. Rejected (CDZ0201), never a crash.")
  (input  (=))
  (error  CDZ0201))

(case "an arithmetic operator with a single operand is rejected, not a crash"
  (doc    "`(+ 5)` supplies one operand to the two-operand `+`. The compiler rejects it (CDZ0201), never
           panicking reaching for the missing second operand — the arithmetic-operator companion of the
           `(= 5)` equality-arity case above.")
  (input  (+ 5))
  (error  CDZ0201))

(case "a bare arithmetic keyword is rejected, not a crash"
  (doc    "`(+)` with no operands is ill-formed. Rejected (CDZ0201), never a crash — the `+` companion
           of the bare `(=)` case.")
  (input  (+))
  (error  CDZ0201))

(case "an ordering operator with a single operand is rejected, not a crash"
  (doc    "`(< 5)` supplies one operand to the two-operand `<`. Rejected (CDZ0201), never a crash. Pins
           that the arity check covers the ordering operators too, not only `=`/`+`.")
  (input  (< 5))
  (error  CDZ0201))

(case "a conditional with too many operands is rejected, not a crash"
  (doc    "`(if true 1 2 3)` supplies a fourth operand to `if`, which takes exactly three (condition,
           then, else). The compiler rejects it (CDZ0201), never silently ignoring the extra operand nor
           crashing — the over-application companion of the missing-branch `(if true 1)` case above.")
  (input  (if true 1 2 3))
  (error  CDZ0201))

(case "a member access with no field operand is rejected, not a crash"
  (doc    "`(. 5)` supplies the record operand but no key: member access `(. <operand> <key>)` takes
           exactly two operands — a NAME key projects a record field, an INTEGER key projects a
           positional tuple element (`(. t 0)`), so this one form serves both. With no key it is
           ill-formed; the compiler rejects it (CDZ0201), never panicking reaching for the absent key
           node.")
  (input  (. 5))
  (error  CDZ0201))

(case "a bare binding form with no bindings and no body is rejected, not a crash"
  (doc    "`(let)` supplies neither a binding list nor a body: `let` is `(let (<binding>…) <body>)`. The
           compiler rejects it (CDZ0201), never panicking reaching for the absent binding list or body
           node — the binding-form companion of the bare-keyword `(=)`/`(if)` cases.")
  (input  (let))
  (error  CDZ0201))

(case "a binding form with bindings but no body is rejected, not a crash"
  (doc    "`(let ((x 1)))` supplies a well-formed binding list but no body form to evaluate in its
           scope. Ill-formed — `let` requires a body — so the compiler rejects it (CDZ0201), never
           panicking reaching for the absent body node. Distinct from `(let ((x)) x)` above (a binding
           with no VALUE); this is a `let` with no BODY.")
  (input  (let ((x 1))))
  (error  CDZ0201))

(case "a let binding with no value expression is rejected, not a crash"
  (doc    "A binding `(x)` names `x` but supplies no value expression: `(let ((x)) x)` is ill-formed.
           The compiler rejects it (CDZ0201), never panicking reaching for the absent value node.")
  (input  (let ((x)) x))
  (error  CDZ0201))

(case "an empty quote is rejected, not a crash"
  (doc    "`(quote)` with nothing to quote is ill-formed: quote requires exactly one operand — the form
           it denotes. The compiler rejects it (CDZ0201), never panicking reaching for the absent
           quoted node.")
  (input  (quote))
  (error  CDZ0201))

(case "a record field with no value expression is rejected, not a crash"
  (doc    "A record entry `(a)` names the field `a` but supplies no value: `(record (a))` is ill-formed
           — a record entry is a `(name value)` pair. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Same never-crash class as the `(let ((x)) x)`
           binding-with-no-value case above, for a record entry.")
  (input  (record (a)))
  (error  CDZ0201))

(case "a map entry with no value expression is rejected, not a crash"
  (doc    "The map companion: `(map (a))` names the key `a` but supplies no value — a map entry is a
           `(key value)` pair, so this is ill-formed. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Pins that both the `record` and `map`
           construction paths bounds-check an entry before indexing its value.")
  (input  (map ("a")))
  (error  CDZ0201))

; --- Never — the empty sum, the dual of Unit, the type of a diverging expression ---------------
; type-system.md #Never Is The Empty Sum: the type universe includes the sum with ZERO variants, the
; dual of Unit (the empty tuple / zero-field product). Never is UNINHABITED — it has no constructor and
; no value — so it is only ever a TYPE, never a value a program builds. The type of an expression that
; DIVERGES rather than producing a value — a `(trap …)`, or `expect` on an absent optional — is Never,
; and Never UNIFIES WITH ANY EXPECTED TYPE (there is no value to be of the wrong type). The seed already
; carries this mechanism internally (a divergent expression's kind unifies with any expected kind, so a
; whole-body-trap function type-checks in any result position); these cases pin the SURFACE property.
; `never` is a FRESH capability the seed does not surface by name — so the seed DECLINES them,
; pinning the contract a later generation binds (the `Never` prelude name and the zero-arm
; exhaustive match) rather than forcing the seed to run them.

(case "a diverging expression unifies with an integer position"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum (3rd sentence: the type of a diverging
           expression is Never, which unifies with any expected type). In `(if b 1 (trap \"unreachable\"))`
           the then-branch is Int64 and the else-branch diverges (type Never); the two branches unify to
           Int64 because Never unifies with any type. With b=true the program yields 1; the else-branch
           never runs but must TYPE-CHECK. A generation without the Never-unifies rule would reject the
           branch-type mismatch. Pins that a divergent branch does not spoil a well-typed conditional.")
  (input  (do
            (def (f b) (if b 1 (trap "unreachable")))
            (def (main) (f true)) (export main)))
  (output (: 1 Int64)))

(case "a function whose body always diverges has result type Never"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum: `bomb` always traps, so its body has type
           Never; calling it at a use site that expects an Int64 type-checks because Never unifies with
           any expected type. The call diverges at run time (the trap), so the program's terminal
           condition is the trap, not a value. Pins that a Never-returning function is callable in a
           typed position — the honest type for a function that never returns normally.")
  (input  (do
            (def (bomb) (trap "unreachable"))
            (def (main) (+ 1 (bomb))) (export main)))
  (trap   "unreachable"))

(case "a match on an uninhabited scrutinee is exhaustive with zero arms"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum (4th sentence: a match on a Never-typed
           scrutinee is exhaustive with zero arms). `never-returns` has result type Never, so matching
           its result needs NO arms to cover every variant — there are none — and the zero-arm match is
           the degenerate BASE CASE of the exhaustiveness rule (core-semantics.md #Matching Is Exhaustive
           Or Rejected), NOT a CDZ0210 non-exhaustive rejection. The scrutinee diverges before the match,
           so the program traps. Pins that the empty sum makes a zero-arm match vacuously exhaustive
           rather than an error.")
  (input  (do
            (def (never-returns) (trap "unreachable"))
            (def (main) (match (never-returns))) (export main)))
  (trap   "unreachable"))

; TYPE REFLECTION — `(Type.of e)` reduces at compile time to the type-VALUE of `e`'s inferred type,
; realizing type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary (a type is
; a first-class value the compiler can compute). It is a COMPILE-TIME operation: a `Type` value is
; erased before the boundary (types-are-erased), so `Type.of` is used in TYPE positions — an annotation
; `(: x (Type.of y))` gives `x` the same type as `y` — never returned at runtime. Attaching a unit or a
; reflected type never changes the value's byte form, so an agreeing `(: x (Type.of y))` is transparent.

(case "Type.of reflects a value's type for use as an annotation"
  (doc    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary: a
           type is a first-class value the compiler computes. `(Type.of y)` reduces to `y`'s type-value
           (here Int64), so `(: 100 (Type.of y))` annotates 100 with that reflected type — an agreeing
           annotation, transparent, evaluating to 100. The reflected type is consumed in type position
           and erased; nothing about it survives to runtime.")
  (input  (let ((y 42)) (: 100 (Type.of y))))
  (output (: 100 Int64)))

(case "an annotation by a reflected type that contradicts the value is rejected"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict, over a REFLECTED type:
           `(Type.of y)` is Int64 (y is 42), so `(: true (Type.of y))` annotates a Bool value with the
           reflected Int64 — a contradiction rejected CDZ0203, exactly as a written `(: true Int64)` is.
           Reflection does not weaken the check: the computed type constrains the value like any
           annotation.")
  (input  (let ((y 42)) (: true (Type.of y))))
  (error  CDZ0203))

(case "Type.of carries a quantity's unit into a same-type annotation"
  (doc    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary
           over a unit-indexed type: `(Type.of y)` where `y : (Qty Float64 meter)` reflects the whole
           quantity type — inner numeric AND unit — so `(: (Qty.of 9.0 meter) (Type.of y))` agrees and
           the quantity erases to 9.0. Pins that reflection captures the full type, dimension included,
           for reuse as `make another quantity of the same type as this one`.")
  (input  (let ((y (Qty.of 3.0 (Unit.base #"meter"))))
            (Qty.value (: (Qty.of 9.0 (Unit.base #"meter")) (Type.of y)))))
  (output (: 9.0 Float64)))

(case "a reflected quantity type rejects a value of a different dimension"
  (doc    "The dimensional companion of the reflection annotation: `(Type.of y)` is `(Qty Float64 meter)`
           (y is a length), so annotating a TIME quantity `(: (Qty.of 9.0 second) (Type.of y))` is a
           dimensional mismatch, CDZ0501 — reflection carries the unit into the check exactly as a
           written `(Qty Float64 meter)` annotation would. A reflected type is a real type, checked in
           full.")
  (input  (let ((y (Qty.of 3.0 (Unit.base #"meter"))))
            (Qty.value (: (Qty.of 9.0 (Unit.base #"second")) (Type.of y)))))
  (error  CDZ0501))

(case "Type.of reflects a runtime parameter's type at compile time"
  (doc    "Witnesses that reflection reads the STATIC type, not a runtime value: `(Type.of n)` for a
           parameter `n : Int64` reduces to Int64 at compile time regardless of `n`'s runtime value, so
           `(: 100 (Type.of n))` is an agreeing Int64 annotation and `main 7` returns 100. The reflected
           type depends only on `n`'s inferred type, and is erased — `n`'s value is never consulted.")
  (input  (do
            (def (main (: n Int64)) (: 100 (Type.of n)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 100 Int64)))

; (A reflected `Type` value is compile-time-only and cannot cross the component boundary — exporting a
; definition whose result IS a `Type.of` value is rejected by the erasure fence, exactly as a bare unit
; value is. That rejection is currently an UNCODED decline, so it is not pinned as an `(error CODE)`
; case here; giving the erasure fence a diagnostic code is a separate increment.)

; COMPILE-TIME TYPE EQUALITY — `(Type.eq a b)` folds to the constant `Bool` of two type-values' EXACT
; structural equality (`Int64` ≠ `Int32`; a quantity's unit is part of its type, so `meter` ≠ `second`).
; The arguments are type-values: a `(Type.of e)` result OR a written type (`Int64`, `(Qty Float64
; meter)`). Because the result is a compile-time CONSTANT, `(if (Type.eq …) then else)` selects a branch
; at compile time — a program branches on types. (The two branches must still share a type: the checker
; unifies both arms before the constant condition prunes the dead one; branching to DIFFERENT result
; types is a later, larger step. The result `Bool` is an ordinary runtime value — only the comparison is
; compile-time.)

(case "Type.eq is true for two values of the same type"
  (doc    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary:
           types are first-class values the compiler compares. `(Type.eq (Type.of 5) (Type.of 6))` — both
           Int64 — folds to the constant `true`. The comparison is exact structural type equality decided
           at compile time; the produced `Bool` is an ordinary value.")
  (input  (Type.eq (Type.of 5) (Type.of 6)))
  (output (: true Bool)))

(case "Type.eq is false for values of different types"
  (doc    "`(Type.eq (Type.of 5) (Type.of true))` compares Int64 with Bool — distinct types — folding to
           the constant `false`. Pins that type equality is a real, decidable comparison, not always
           true: two differently-typed values are observably unequal at the type level.")
  (input  (Type.eq (Type.of 5) (Type.of true)))
  (output (: false Bool)))

(case "Type.eq carries an integer type's full width and signedness"
  (doc    "An integer type is identified by BOTH its width and its signedness, and `Type.eq` compares the
           whole thing. `(Type.of (Int8.of 5))` is `Int8`: equal to the written `Int8` (→ true), distinct
           from `Int64` (different WIDTH → false), and distinct from `UInt8` (same width, different SIGN →
           false). `1 + 0 + 0 = 1`. Pins that a numeric type-value carries the exact `(sign, width)` — the
           numeric-model no-silent-promotion rule at the type-value level, so a program can branch on a
           value's exact integer type.")
  (input  (do
            (def (main) (+ (if (Type.eq (Type.of (Int8.of 5)) Int8) 1 0)
                           (+ (if (Type.eq (Type.of (Int8.of 5)) Int64) 10 0)
                              (if (Type.eq (Type.of (Int8.of 5)) (Type.of (UInt8.of 5))) 100 0))))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq compares a reflected type against a written type"
  (doc    "`(Type.eq (Type.of 5) Int64)` compares a reflected type with a WRITTEN one — both are
           type-values, so the operation is symmetric over reflection and syntax — and is `true`. Pins
           that a written type and `Type.of` produce the same kind of value, composably comparable.")
  (input  (Type.eq (Type.of 5) Int64))
  (output (: true Bool)))

(case "Type.eq distinguishes quantities by their unit"
  (doc    "A quantity's UNIT is part of its type, so `(Type.eq (Type.of (Qty.of 1.0 meter)) (Type.of
           (Qty.of 1.0 second)))` is `false` — meter and second are different dimensions hence different
           types — while the same unit compares `true` regardless of magnitude. Pins that type equality
           carries the full unit-indexed type (units-of-measure.md #Dimensional Mismatch Is An Error, at
           the type-value level).")
  (input  (Type.eq (Type.of (Qty.of 1.0 (Unit.base #"meter")))
                   (Type.of (Qty.of 1.0 (Unit.base #"second")))))
  (output (: false Bool)))

(case "Type.eq on a constructed generic sum compares the full instantiated type"
  (doc    "`Type.of` on a value of a CONSTRUCTED generic sum reflects its full instantiated type, including
           the type ARGUMENT. `(Type.eq (Type.of (Iter.Cons 1 Iter.Nil)) (Type.of (Iter.Cons 2 Iter.Nil)))`
           — both are `Iter Int64` — folds to `true`. Pins that a generic type CONSTRUCTOR's type-value
           carries its arg (a `Ty::Sum{decl, args}`, compared structurally), so reflection over a
           user-generic value sees the same instantiated type two equal-typed values share.")
  (input  (do
            (type Iter (Nil) (Cons a (Iter a)))
            (def (main) (if (Type.eq (Type.of (Iter.Cons 1 (Iter.Nil)))
                                     (Type.of (Iter.Cons 2 (Iter.Nil)))) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq distinguishes a constructed generic sum by its type argument"
  (doc    "The type ARGUMENT is part of a generic sum's type, so `(Type.of (Iter.Cons 1 Iter.Nil))` (an
           `Iter Int64`) and `(Type.of (Iter.Cons true Iter.Nil))` (an `Iter Bool`) compare `false` — same
           `decl`, different element arg. The generic-sum analogue of the quantity-unit case: type equality
           carries the full instantiated `Ty::Sum{decl, args}`, not just the decl.")
  (input  (do
            (type Iter (Nil) (Cons a (Iter a)))
            (def (main) (if (Type.eq (Type.of (Iter.Cons 1 (Iter.Nil)))
                                     (Type.of (Iter.Cons true (Iter.Nil)))) 1 0))
            (export main)))
  (output (: 0 Int64)))

(case "Type.of on a bare nullary variant reflects its element as UNDETERMINED"
  (doc    "`Type.of` reflects a value's INFERRED type, so a bare nullary variant — carrying no element —
           reflects an UNDETERMINED element. `(None)` is `Option ?a`, distinct from a concrete `Option
           Int64` (`(Some 1)`): `(Type.eq (Type.of (None)) (Type.of (Some 1)))` is `false` — the `?a` is not
           yet `Int64`. Pins that reflection over a polymorphic value sees the type as inferred (undetermined
           here), NOT eagerly grounded — the type-value analogue of a bare `None`'s open element type. (See
           the next case: CONTEXT that constrains the element makes them equal.)")
  (input  (do
            (def (main) (if (Type.eq (Type.of (None)) (Type.of (Some 1))) 1 0))
            (export main)))
  (output (: 0 Int64)))

(case "context that constrains a nullary variant's element makes Type.of concrete"
  (doc    "The complement of the bare case above: when CONTEXT pins a nullary variant's element, `Type.of`
           reflects the concrete instantiation. `pick : (Option Int64) -> (Option Int64)` forces its `(None)`
           argument to `Option Int64`, so `(Type.eq (Type.of (pick (None))) (Type.of (Some 1)))` is `true` —
           both `Option Int64`. Pins that `Type.of` tracks the element the surrounding constraints DETERMINE,
           so the same `(None)` reflects `Option ?a` bare (false above) but `Option Int64` in a typed
           position (true here) — reflection is over the SOLVED type, not the syntactic constructor.")
  (input  (do
            (def (pick (: o (Option Int64))) o)
            (def (main) (if (Type.eq (Type.of (pick (None))) (Type.of (Some 1))) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq compares a TUPLE type-value structurally, by element types"
  (doc    "`Type.of` on a tuple value reflects its structural `Ty::Tuple` type, compared element-wise.
           `(Type.of (tuple 1 \"a\"))` is `(Tuple Int64 String)`: equal to another `(Tuple Int64 String)`
           regardless of the element VALUES (→ true), but distinct from `(Tuple Int64 Int64)` (a differing
           element type → false). `1 + 0 = 1`. Pins that type equality over a tuple type-value is by the
           element TYPES, not the values — the tuple analogue of the record/sum structural comparison.")
  (input  (do
            (def (main) (+ (if (Type.eq (Type.of (tuple 1 "a")) (Type.of (tuple 2 "b"))) 1 0)
                           (if (Type.eq (Type.of (tuple 1 "a")) (Type.of (tuple 1 2))) 10 0)))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq compares a RECORD type-value structurally, by field types"
  (doc    "`Type.of` on a record value reflects its structural `Ty::Record` type, compared by field name +
           type. `(Type.of (record (x 1) (y \"a\")))` is `(Record (x Int64) (y String))`: equal to another
           record of the same field-name-and-type set regardless of values (→ true), but distinct when a
           field's TYPE differs — `(y String)` vs `(y Int64)` → false. `1 + 0 = 1`. Pins that a record
           type-value's equality carries each field's type (the record analogue of the tuple case above).")
  (input  (do
            (def (main) (+ (if (Type.eq (Type.of (record (x 1) (y "a")))
                                        (Type.of (record (x 2) (y "b")))) 1 0)
                           (if (Type.eq (Type.of (record (x 1) (y "a")))
                                        (Type.of (record (x 1) (y 2)))) 10 0)))
            (export main)))
  (output (: 1 Int64)))

(case "an if on Type.eq selects a branch at compile time"
  (doc    "The headline of compile-time reflection: `(if (Type.eq (Type.of 5) Int64) 100 200)` folds the
           condition to the constant `true`, so the whole `if` is `100`. A program BRANCHES on types at
           compile time — the type comparison decides control flow with no runtime cost (the condition is
           a constant, not an emitted test).")
  (input  (if (Type.eq (Type.of 5) Int64) 100 200))
  (output (: 100 Int64)))

(case "a compile-time type branch reads a runtime parameter's static type"
  (doc    "`(if (Type.eq (Type.of n) Int64) (+ n 1) 0)` for a parameter `n : Int64` branches on `n`'s
           STATIC type (Int64), folding the condition to `true` at compile time, so `main 7` returns 8.
           Pins that the branch is decided by the parameter's inferred type — not its runtime value — yet
           the selected branch runs on the actual value.")
  (input  (do
            (def (main (: n Int64))
              (if (Type.eq (Type.of n) Int64) (+ n 1) 0))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 8 Int64)))

(case "Type.eq branches on a type-valued parameter, monomorphized per passed type"
  (doc    "`Type.eq` accepts a TYPE-VALUED PARAMETER `t` as an operand: `(def (is-int (: t Type) (: x
           Int64)) (if (Type.eq t Int64) 1 0))`. At each instantiation `t` is a concrete compile-time
           type-value (monomorphization substitutes it), so `(Type.eq t Int64)` folds to a constant per
           call — `(is-int Int64 5)` folds `true` → 1, `(is-int Bool 5)` folds `false` → 0, so their sum
           is 1. Pins that a type-valued parameter is a first-class `Type.eq` operand (types-as-values:
           a program branches on a passed type), not only a written type or a `Type.of` result — the
           operand in a VALUE position reduces through its `Type`-kinded annotation to the parameter's
           substituted type-value.")
  (input  (do
            (def (is-int (: t Type) (: x Int64)) (if (Type.eq t Int64) 1 0))
            (def (main) (+ (is-int Int64 5) (is-int Bool 5)))
            (export main)))
  (output (: 1 Int64)))

; Type.eq on a type-valued parameter NEIGHBORS (breaker): the case above compares ONE parameter against a
; WRITTEN type (Int64). These pin the operand positions it doesn't: BOTH operands type-valued parameters
; (t vs u), a COMPOUND type operand (List Int64 — structural, not a leaf), a type-value THREADED through a
; second call before the Type.eq (monomorphization must carry it), and a parameter against ITSELF (always
; true regardless of the instantiation). All fold per-call at compile time, both backends.

(case "Type.eq compares two type-valued parameters to each other"
  (doc    "Both operands are type-valued PARAMETERS, not one against a written type: `(Type.eq t u)`. Each
           instantiation substitutes concrete type-values, so it folds per call — `(same Int64 Int64)` folds
           true → 1, `(same Int64 Bool)` folds false → 0, sum 1. Pins that a type-valued parameter is a valid
           operand on BOTH sides of Type.eq, not only the left.")
  (input  (do
            (def (same (: t Type) (: u Type)) (if (Type.eq t u) 1 0))
            (def (main) (+ (same Int64 Int64) (same Int64 Bool)))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq on a type-valued parameter against a COMPOUND type is structural"
  (doc    "The operand may be a COMPOUND written type, not only a leaf: `(Type.eq t (List Int64))`. Type
           equality is structural, so `(List Int64)` ≠ `(List Bool)` — `(is-list-int (List Int64))` folds true
           → 1, `(is-list-int (List Bool))` folds false → 0, sum 1. Pins that Type.eq on a type-valued
           parameter compares compound types by structure (the element type distinguishes them), not by a
           head-only tag.")
  (input  (do
            (def (is-list-int (: t Type)) (if (Type.eq t (List Int64)) 1 0))
            (def (main) (+ (is-list-int (List Int64)) (is-list-int (List Bool))))
            (export main)))
  (output (: 1 Int64)))

(case "a type-valued parameter threaded through a second call reaches Type.eq with its type intact"
  (doc    "Monomorphization must carry the type-value across a call boundary: `relay` passes its `(: t Type)`
           to `check`, which does the `Type.eq t Int64`. `(relay Int64)` folds true → 1, `(relay Bool)` folds
           false → 0, sum 1. Pins that a type-valued parameter substituted at `relay`'s instantiation flows
           into `check`'s Type.eq — the type-value is not lost when passed to another function.")
  (input  (do
            (def (check (: t Type)) (if (Type.eq t Int64) 1 0))
            (def (relay (: t Type)) (check t))
            (def (main) (+ (relay Int64) (relay Bool)))
            (export main)))
  (output (: 1 Int64)))

(case "Type.eq of a type-valued parameter against itself is always true"
  (doc    "`(Type.eq t t)` — a type-valued parameter compared to ITSELF — is true at EVERY instantiation,
           whatever type is passed: `(refl Int64 5)` → 1 and `(refl Bool 5)` → 1, sum 2. Pins the reflexivity
           of Type.eq on a type-valued operand independent of the substituted type (a fold that only matched
           against a WRITTEN type would miss the param-vs-same-param case).")
  (input  (do
            (def (refl (: t Type) (: x Int64)) (if (Type.eq t t) 1 0))
            (def (main) (+ (refl Int64 5) (refl Bool 5)))
            (export main)))
  (output (: 2 Int64)))

; A TYPE-VALUE is compile-time-only (`type-system.md §A Type Parameter Is Resolvable At Compile Time`: a
; type-value never flows from runtime data into a position that determines a type). So a value that would
; carry a type-value into RUNTIME data — a compound storing a type, returned across the component boundary
; — is rejected at compile time. A bare type export (`(def (main) Int64)`) is already rejected; this pins
; the NESTED case: a type stored in a tuple result is ONE coded CDZ0201, naming the compound, not a cascade
; of internal no-runtime-form declines.

(case "a type stored in a compound result cannot cross the boundary"
  (doc    "`(def (main) (tuple Int64 5))` returns `(Tuple Type Int64)` — a tuple carrying a TYPE-value in
           its first slot. A type-value is compile-time only and has no runtime form, so a compound
           carrying one cannot cross the component boundary. The compiler reports ONE coded CDZ0201 naming
           the compound (not the four uncoded no-runtime-form declines the emit path would otherwise leak).
           The rejection is the program's outcome; there is no value.")
  (input  (do (def (main) (tuple Int64 5)) (export main)))
  (error  CDZ0201))
