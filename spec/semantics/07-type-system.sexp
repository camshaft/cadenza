; Type system — witnesses type-system.md. The seed is a COMPILER that realizes the static-typing floor
; incrementally (constitution VII; Amendment 0.4.0): an ill-typed program's recorded outcome IS its
; rejection — (error <CODE>) is the primary clause, because an ill-typed program has no run and therefore
; no terminal value. For a type rule a generation does not yet cover it DECLINES rather than running the
; program (reject-don't-miscompile); the gate scores a decline as todo, not disagreement. Diagnostic
; codes are from options/diagnostics-schema/.
(case
  "a type annotation consistent with the value is transparent"
  (doc
    "Witnesses type-system.md #Annotations Constrain, Never Contradict: an annotation agreeing
           with the value changes nothing and the program evaluates to the annotated value.")
  (input (: 42 Int64))
  (output (: 42 Int64)))

(case
  "a Bool annotation on a Bool value is transparent"
  (doc
    "The Bool companion of the transparency rule above: `(: true Bool)` — the annotation matches the
           value, so it is transparent and the program evaluates to `true`. Exercises the Bool boundary
           specifically (i1), not only the Int64 case. Relocated from the in-crate rcdzc
           `a_bool_annotation_on_a_bool_is_transparent`.")
  (input (: true Bool))
  (output (: true Bool)))

(case
  "an annotation whose type is a reduced (Int 64) constructor grounds the value"
  (doc
    "The annotation's type side is a full type EXPRESSION the evaluator reduces: `(: 5 (Int 64))` uses
           the `(Int 64)` type-constructor application (not the `Int64` alias) as the annotation type. It
           grounds the literal at the 64-bit signed width and is transparent → 5. Pins that a constructor-form
           type annotation reduces + grounds, the positive companion of the ctor-form width rejects in 06.
           Relocated from the in-crate rcdzc `an_annotation_grounds_a_width_via_int_ctor`.")
  (input (: 5 (Int 64)))
  (output (: 5 Int64)))

(case
  "an annotation that contradicts the value is rejected"
  (doc
    "Witnesses type-system.md #Annotations Constrain, Never Contradict: `(: 42 Bool)` annotates
           an Int64 value with Bool — a contradiction the compiler rejects (CDZ0203). The rejection is
           the program's outcome; there is no value, because the program does not run.")
  (input (: 42 Bool))
  (error CDZ0203))

(case
  "an annotation that contradicts the value is rejected — the mirror direction"
  (doc
    "The mirror of the `(: 42 Bool)` case: `(: true Int64)` annotates a BOOL value with Int64. Unifying
           the annotation type against the value's type fails either direction, so the contradiction rejects
           CDZ0203 the same way — the disambiguation force turned against a genuine conflict. (Migrated from
           rcdzc an_annotation_conflicting_with_the_value_rejects.)")
  (input (: true Int64))
  (error CDZ0203))

; The `(: <expression> <type>)` annotation FORM is exactly two operands; a MALFORMED arity — too few `(: 5)`,
; too many `(: 5 Int64 foo)`, empty `(:)` — is CDZ0201 naming the canonical form AND the actual part count
; (rustc-style) so the author sees exactly what is wrong. The `(not "takes exactly")` pins a regression guard:
; an earlier wording collided with the emit-path operator-arity dedup filter (EMIT_OPERAND_ARITY_MARKER) and
; was SILENTLY DROPPED for the 0- and 3-operand cases. (Migrated from rcdzc
; a_wrong_arity_type_annotation_names_the_operand_count_at_every_arity.)
(case
  "a type annotation with too few operands names the one part present"
  (input (do (def x (: 5)) (export x)))
  (error
    CDZ0201
    (message "a type annotation is written")
    (message "1 part is here")
    (not "takes exactly")))

(case
  "a type annotation with too many operands names the three parts present"
  (input (do (def x (: 5 Int64 foo)) (export x)))
  (error CDZ0201 (message "3 parts are here") (not "takes exactly")))

(case
  "an empty type annotation names the zero parts present"
  (input (do (def x (:)) (export x)))
  (error CDZ0201 (message "0 parts are here") (not "takes exactly")))

(case
  "a well-formed two-operand annotation does not false-positive as a malformed-arity reject"
  (input (do (def (main) (: 5 Int64)) (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a contradictory ARROW annotation on a function value is rejected"
  (doc
    "The function-value facet of #Annotations Constrain, Never Contradict: `h x = x + 1` has type
           `(-> Int64 Int64)` (the `+` body-solves the domain to `Int64`), so annotating it `(: h (-> Bool
           Int64))` is a contradiction and rejects CDZ0203 — the domains disagree. This slipped through when
           the annotation check read `h`'s type bottom-up as `(-> Any Int64)` (an unannotated parameter is
           `Any`, and `Any` unifies with `Bool`, masking the mismatch); the check now body-solves the
           function's domain (matching how the value lowers + reflects), so a contradictory domain is caught.
           A result contradiction `(: h (-> Int64 Bool))` and an annotated-domain function were already
           rejected — only the un-annotated DOMAIN slipped.")
  (input (do (def (h x) (+ x 1)) (def (main) (: h (-> Bool Int64))) (export main)))
  (error CDZ0203))

; The contradictory-arrow cases reject; the AGREEING complement must be ACCEPTED and transparent — the
; positive half of #Annotations Constrain, Never Contradict for a function value. A direct arrow
; annotation on a lambda whose body-solved type MATCHES the annotation changes nothing: the annotated
; lambda is the same function and applies normally. Pins that the annotation-check's body-solve (which
; catches the contradictions above) does not spuriously reject an agreeing arrow — a check that compared
; the annotation against the bottom-up `(-> Any _)` instead of the body-solved domain could over-reject
; an agreeing `(-> Int64 _)` as "Any ≠ Int64". Runtime application, both backends.
(case
  "an agreeing arrow annotation on a lambda is accepted and the function applies normally"
  (doc
    "The positive complement of the contradictory-arrow rejects: `(: (fn (x) (+ x 1)) (-> Int64
           Int64))` annotates a lambda whose body solves its domain to `Int64` — the annotation AGREES, so
           it is transparent and the function applies: `f(n) = n + 1`, run(5) = 6. And the 2-arg twin
           `(: (fn (x y) (* x y)) (-> Int64 Int64 Int64))` agrees and applies: `g(3,4) = 12`. Pins that an
           arrow annotation matching the body-solved type is accepted (never over-rejected), the accept
           side of the constrain-never-contradict rule for function values.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((f (: (fn ((: x Int64)) (+ x 1)) (-> Int64 Int64)))
            (g (: (fn ((: x Int64) (: y Int64)) (* x y)) (-> Int64 Int64 Int64))))
          (+ (f a) (g a b))))
      (export main)))
  (call main (: 5 Int64) (: 4 Int64))
  (output (: 26 Int64))
  (call main (: 3 Int64) (: 4 Int64))
  (output (: 16 Int64)))

(case
  "a contradictory arrow annotation on a function INSIDE a compound is rejected"
  (doc
    "The bare-function domain check above, extended to a function stored INSIDE the annotated
           compound: `(: (tuple h 0) (Tuple (-> Bool Int64) Int64))` where `h : Int64 -> Int64` is a
           contradiction — the tuple's element function has domain `Int64`, not the annotated `Bool`. It
           slipped through because the compound's bottom-up type rendered its fn element as `(-> Any Int64)`
           (the unannotated domain leaks `Any` through the container, and `Any` unifies with `Bool`); the
           annotation check now grounds a fn element's domain from its body wherever it sits — through
           tuple/list/record/map elements and sum-variant payloads — so the nested contradiction is caught.
           A non-fn element mismatch and a fn RESULT mismatch through a compound were already caught (the
           element/result types are concrete); only the nested un-annotated DOMAIN slipped. CDZ0203.")
  (input
    (do
      (def (h x) (+ x 1))
      (def (main) (: #tuple(h 0) (Tuple (-> Bool Int64) Int64)))
      (export main)))
  (error CDZ0203))

(case
  "a contradictory arrow annotation on a function in a SUM payload is rejected"
  (doc
    "The sum-payload sibling of the compound-element annotation check: `(: (Some h) (Option (-> Bool
           Int64)))` with `h : Int64 -> Int64` is a contradiction — the payload function's domain is `Int64`,
           not `Bool`. Same root as the tuple case (the payload's fn domain leaked `Any` through the `Option`
           type argument, masking the mismatch); the grounded annotation check catches it. CDZ0203.")
  (input (do (def (h x) (+ x 1)) (def (main) (: (Some h) (Option (-> Bool Int64)))) (export main)))
  (error CDZ0203))

(case
  "a contradictory arrow annotation on a function stored as a Map KEY is rejected"
  (doc
    "The Map-key companion (annotation-check companion to the fn-domain reflection cases): a function
           is a reachable Map key, so a contradictory arrow annotation on the KEY must also be caught.
           `(: (Map.insert Map.empty h 1) (Map (-> Bool Int64) Int64))` with `h : Int64 -> Int64` annotates
           the key function's domain as `Bool` — a contradiction. The grounded annotation check grounds the
           key fn's domain too (both k and v via the same `Prim::MapInsert` gate + reflected_ty grounding),
           so it rejects. CDZ0203.")
  (input
    (do
      (def (h x) (+ x 1))
      (def (main) (: (Map.insert Map.empty h 1) (Map (-> Bool Int64) Int64)))
      (export main)))
  (error CDZ0203))

(case
  "a contradictory arrow annotation on a function in a runtime Map value is rejected"
  (doc
    "The runtime-Map-builder sibling of the compound/sum annotation-check cases: `(: (Map.insert
           Map.empty 1 h) (Map Int64 (-> Bool Int64)))` with `h : Int64 -> Int64` is a contradiction — the
           map value function's domain is `Int64`, not the annotated `Bool`. Like the sum-payload case, the
           value fn's domain leaked `Any` through the `Map` type argument (`Map.insert`'s result type comes
           from the op scheme, read bottom-up), so the `Any` absorbed the annotated `Bool` and the mismatch
           was silently ACCEPTED — the check-side twin of the Map reflection leak (Option/Tuple already
           rejected the same). The annotation check now grounds a fn domain in the Map value (and key)
           position, matching the reflection fix. CDZ0203.")
  (input
    (do
      (def (h x) (+ x 1))
      (def (main) (: (Map.insert Map.empty 1 h) (Map Int64 (-> Bool Int64))))
      (export main)))
  (error CDZ0203))

; The TYPE OPERAND of an annotation `(: expr T)` must itself DENOTE A TYPE — validating what stands in
; type position is the dual of checking it against the value. A non-type there (an unbound name, an
; integer/compound VALUE, an arbitrary expression, a non-constructor type applied to arguments) is
; MEANINGLESS and MUST be REJECTED, not silently accepted-and-ignored (which would let a typo'd or
; garbage annotation pass — the opposite of the reject-don't-accept-garbage discipline the checker
; applies everywhere else). An UNBOUND NAME rejects the same CDZ0101 it gets in value position (`(+ foo
; 1)`); a well-formed non-type rejects CDZ0203 ("expected a type"). This holds for a PARAMETER annotation
; `(: name T)` too, not only a value annotation.
(case
  "an unbound name in a type annotation's type position is rejected"
  (doc
    "`(: 5 foo)` puts the unbound name `foo` in TYPE position. `foo` names no type, and the same
           `foo` in VALUE position is a hard CDZ0101 'unbound name', so it must reject identically here —
           an annotation whose type is a typo or a non-type is meaningless (type-system.md #Annotations
           Constrain, Never Contradict). A generation that resolved the operand, found it not a type, and
           dropped the annotation ACCEPTED this and ran to 5; the type position must reject a non-type.")
  (input (do (def (main) (: 5 foo)) (export main)))
  (error CDZ0101))

(case
  "an unbound uppercase name in a type annotation's type position is rejected"
  (doc
    "`(: 5 Foo)` puts the unbound UPPERCASE name `Foo` in type position — a missing or typo'd
           CONCRETE type (unlike the lowercase `foo` above, which reads as an ML-style type variable). Both
           reject CDZ0101 (the name is unbound either way), but they are distinct diagnostic branches: a
           lowercase name gets an actionable 'generic route' hint (Cadenza's polymorphism comes from an
           UNANNOTATED parameter, not a `∀`-binder in an annotation), while an uppercase name — read as a
           concrete type that does not exist — keeps the plain 'unbound name' message. Pins the uppercase
           branch (a missing concrete type), the case-distinct companion of the lowercase-`foo` case. The
           (message ..) pins the actionable lead — the diagnostic names the repair ('declare it with `(type
           Foo …)`'), not a dead-end 'unbound name' — so a wording degrade flips this case.")
  (input (do (def (main) (: 5 Foo)) (export main)))
  (error CDZ0101 (message "declare it with")))

(case
  "a lowercase type variable in a USER-GENERIC parameter annotation is rejected"
  (doc
    "`(def (next (: it (Iter a))) …)` annotates a parameter with a USER-GENERIC type applied to a
           lowercase `a` — the ML/Haskell reflex for a generic signature `next : Iter a -> …`. But Cadenza
           has NO `∀`-binder in an annotation: generics are type-valued parameters (type-system.md
           #Generics Are Type-Valued Parameters), so `a` in the annotation names no type and is a hard
           CDZ0101 'unbound name'. The nested lowercase leaf inside a user generic `(Iter a)` gets the same
           reject the bare `(: x a)` does — the actionable route is to drop the annotation (an UNANNOTATED
           parameter is already polymorphic) or take the element type as an explicit `(: t Type)` parameter.
           Pins that a lowercase type var in a user-generic constructor position rejects, not silently
           binds a fresh variable.")
  (input
    (do
      (type Iter (FromList (List a)))
      (def (next (: it (Iter a))) it)
      (def (main) 0)
      (export main)))
  (error CDZ0101))

(case
  "an integer literal in a type annotation's type position is rejected"
  (doc
    "`(: 5 42)` puts the integer literal `42` — a VALUE, not a type — in type position. A value is
           not a type, so the annotation is meaningless and rejects (CDZ0203, 'expected a type'). Pins
           the non-name facet of the same missing validation: any non-type operand rejects, not just an
           unbound name. Accepted-and-ignored (ran to 5) before the type-operand check. A LITERAL has no
           name to blame, so it keeps the GENERIC `found a non-type` phrasing — not the `X is a value, not a
           type` naming a bound value name earns (below).")
  (input (do (def (main) (: 5 42)) (export main)))
  (error CDZ0203 (message "found a non-type")))

(case
  "a value compound in a type annotation's type position is rejected"
  (doc
    "`(: 5 (tuple 1 2))` puts a VALUE tuple — not a type — in type position, the compound-value
           companion of the bare-literal `(: 5 42)` case above. A value compound is not a type, so the
           annotation is meaningless and rejects CDZ0203 ('expected a type'), rather than being resolved as
           a value and silently dropped. (migrated from rcdzc a_non_type_in_a_type_annotation_position.)")
  (input (do (def (main) (: 5 #tuple(1 2))) (export main)))
  (error CDZ0203))

; A type-CONSTRUCTOR form with a well-formed NON-TYPE in a type-argument position — `(List 5)`, `(Tuple Int64
; 5)`, `(-> Int64 5)`, `(Map 5 Int64)` — used to read as the flat "requires a type, but found a non-type" over
; the WHOLE form, naming neither WHICH element is wrong nor anchoring at it. It now names the specific POSITION
; (element / element-index / key / value / result / parameter-index) and anchors at the offending element. An
; UNBOUND name in a type-argument position keeps its own CDZ0101; a nested WRONG-ARITY ctor keeps its arity
; message; a valid type raises nothing. (Migrated from rcdzc
; a_non_type_argument_in_a_type_constructor_names_the_position — the per-case node-anchor check is covered by
; the dedicated anchor tests.)
(case
  "a non-type in a List element position names the element type"
  (input (do (def (g (: x (List 5))) x) (export g)))
  (error CDZ0203 (message "the element type must be a type, but this is a value")))

(case
  "a non-type in a Set element position names the element type"
  (input (do (def (g (: x (Set 5))) x) (export g)))
  (error CDZ0203 (message "the element type must be a type, but this is a value")))

(case
  "a non-type in a Tuple element position names the element index"
  (input (do (def (g (: x (Tuple Int64 5))) x) (export g)))
  (error CDZ0203 (message "element 1's type must be a type, but this is a value")))

(case
  "a non-type in a Map key position names the key type"
  (input (do (def (g (: m (Map 5 Int64))) m) (export g)))
  (error CDZ0203 (message "the key type must be a type, but this is a value")))

(case
  "a non-type in a Map value position names the value type"
  (input (do (def (g (: m (Map Int64 5))) m) (export g)))
  (error CDZ0203 (message "the value type must be a type, but this is a value")))

(case
  "a non-type in a function result position names the result type"
  (input (do (def (g (: f (-> Int64 5))) f) (export g)))
  (error CDZ0203 (message "the result type must be a type, but this is a value")))

(case
  "a non-type in a function parameter position names the parameter index"
  (input (do (def (g (: f (-> 5 Int64))) f) (export g)))
  (error CDZ0203 (message "parameter 0's type must be a type, but this is a value")))

(case
  "an unbound name in a type-argument position keeps CDZ0101, not the non-type message"
  (input (do (def (g (: x (List Nonesuch))) x) (export g)))
  (error CDZ0101 (message "Nonesuch") (not "must be a type, but this is a value")))

(case
  "a nested wrong-arity type constructor keeps its own arity message"
  (input (do (def (g (: x (List (Map Int64)))) x) (export g)))
  (error CDZ0203 (message "`Map` takes 2 type arguments")))

(case
  "a non-constructor type applied to arguments in type position is rejected"
  (doc
    "`(: true (Int64 Int64))` applies `Int64` — which is NOT a type constructor (it takes no
           arguments) — to an argument, a malformed type expression. Were the operand simply `Int64`, the
           annotation would reject the Bool value `true`; instead the malformed application resolved to a
           non-type and was silently dropped, so `(: true (Int64 Int64))` ran to true. A non-constructor
           type applied to arguments must reject (CDZ0203, 'expected a type'). (An over/under-applied
           GENERIC type rejects via unification when the value forces it; this is the non-generic case.)")
  (input (do (def (main) (: true (Int64 Int64))) (export main)))
  (error CDZ0203))

(case
  "a monomorphic user sum applied to a type argument in an annotation is rejected"
  (doc
    "The common sum-annotation slip: `(: t (T Int64))` where `(type T (Leaf Int64) (Node Int64))` is
           MONOMORPHIC — it takes no type parameters, so `(T Int64)` over-applies it. The reader parses
           `(T Int64)` as applying `T` to `Int64`; `T` reduces to a type-value with zero declared params,
           so it is over-applied and rejects CDZ0203. The bare `T` is the correct annotation (a monomorphic
           sum's type is just its name). Pins that a monomorphic USER SUM applied to a type argument is a
           coded rejection (the sum companion of the `(Int64 Int64)` case above — a non-generic type
           applied to arguments), NOT silently accepted; the diagnostic names the fix (`T`, not `(T …)`).")
  (input
    (do
      (type T (Leaf Int64) (Node Int64))
      (def (f (: t (T Int64))) (match t ((T.Leaf n) n) ((T.Node n) n)))
      (def (main) (f (T.Leaf 5)))
      (export main)))
  (error CDZ0203))

; The monomorphic case above rejects an OVER-application; its positive counterpart is a legitimately
; GENERIC user sum applied to a CONCRETE type in an annotation — this must RESOLVE by name exactly as a
; built-in generic `(Option Int64)` / `(List Int64)` does. That path was silently broken (a user generic's
; name missed the type-annotation resolve — bare or applied — while monomorphic user sums and built-in
; generics resolved fine), so no annotation case existed to catch it; the two below pin both faces.
(case
  "a user-declared generic sum resolves by name in a parameter type annotation"
  (doc
    "The positive counterpart of the monomorphic over-application reject above: a user-declared
           GENERIC sum `(type (Container a) (Full a))` applied to a CONCRETE type in a parameter annotation
           `(: b (Container Int64))` must RESOLVE by name and check the argument, exactly as the built-in
           `(: x (Option Int64))` / `(: x (List Int64))` do (type-system.md #Generics Are Type-Valued
           Parameters). A user generic's type NAME previously missed the type-annotation resolve entirely —
           `(Container Int64)` reported CDZ0101 'unbound name Container' — so a program could not annotate a
           parameter with its own generic type (workaround was to drop the annotation and lean on inference).
           `(unwrap (Full 7))` at `(Container Int64)` recovers the payload `7`. Pins that a user generic
           resolves in a type-expression position like a built-in generic does, closing the annotation gap.")
  (input
    (do
      (type (Container a) (Full a))
      (def (unwrap (: b (Container Int64))) (match b ((Full v) v)))
      (def (main (: k Int64)) (unwrap (Full k)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64)))

(case
  "a user generic sum named BARE (unapplied) in an annotation needs a type argument"
  (doc
    "The generic sibling of the monomorphic over-application: naming the generic `Container` BARE in a
           parameter annotation `(: b Container)` — no type argument — is under-applied, exactly the reject a
           bare built-in `(: b Option)` gets. The diagnostic is the type-constructor-needs-an-argument branch
           (CDZ0203), NOT the old 'unknown type Container' (CDZ0101): the name now resolves to the type
           constructor, and the fault is the missing argument. Pins that a user generic behaves as a proper
           type constructor in an annotation — resolvable by name, but requiring its argument.")
  (input
    (do
      (type (Container a) (Full a))
      (def (unwrap (: b Container)) (match b ((Full v) v)))
      (def (main (: k Int64)) (unwrap (Full k)))
      (export main)))
  (error CDZ0203))

; The bare-under-applied reject above is a PARAMETER annotation. A VALUE annotation `(: <value> Name)` with
; a bare generic ctor must give the SAME needs-an-argument message — it used to give a CONFUSING mismatch
; ('annotation type Box does not match value type Box') because a bare generic REDUCES to a Ty::Sum with a
; fresh var, so the value-vs-annotation unify fired a mismatch whose two sides rendered identically. The
; value-annotation path now runs the same bare-ctor check the parameter path does → CDZ0203, for a user
; generic and a built-in alike; the two faces below pin that a value annotation reads like a param annotation.
(case
  "a bare user generic ctor in a VALUE annotation needs a type argument"
  (doc
    "`(: (Mk 1) Box)` annotates a VALUE with the bare generic `(type (Box a) (Mk a))` — no type
           argument. This must give the type-constructor-needs-an-argument reject (CDZ0203), the same the
           bare PARAMETER annotation `(: b Container)` above gives, NOT the old confusing 'annotation type Box
           does not match value type Box — wrap the value in Mk' (both sides rendered `Box`, reading as a
           self-contradiction) that fired because the bare generic reduced to a `Ty::Sum` with a fresh var so
           the value-vs-annotation unify mismatched. Pins the value-annotation bare-ctor face aligns with the
           parameter path's clear diagnostic.")
  (input (do (type (Box a) (Mk a)) (def (main) (: (Mk 1) Box)) (export main)))
  (error CDZ0203))

(case
  "a bare built-in generic in a VALUE annotation needs a type argument"
  (doc
    "The built-in companion: `(: 5 List)` annotates a value with the bare generic `List` — no element
           type. CDZ0203 needs-an-argument, matching the user-generic value-annotation case above and the
           bare built-in in a parameter position. Pins the value-annotation bare-ctor check is uniform for a
           built-in generic and a user one.")
  (input (do (def (main) (: 5 List)) (export main)))
  (error CDZ0203))

; The DECLARATION-position face: a bare generic ctor with NO argument in a variant PAYLOAD or an effect-op
; arrow type must give the SAME needs-an-argument reject the annotation positions do. `validate_type_position`
; used to wave a bare user generic through (typeval_of succeeds on it, reducing to a Ty::Sum with a fresh
; var → early-return); it now runs the bare-ctor check first, so a declaration position agrees with an
; annotation: CDZ0203 "`<ctor>` is a type constructor — it needs a type argument here". (migrated from rcdzc
; a_bare_generic_ctor_missing_its_argument_in_a_declaration_type_position_is_cdz0203.)
(case
  "a bare user generic ctor in a variant payload needs a type argument"
  (input (do (type (Box a) (Mk a)) (type W (Wrap Box)) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` is a type constructor — it needs a type argument")))

(case
  "a bare built-in generic in a variant payload needs a type argument"
  (input (do (type W (Wrap Option)) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Option` is a type constructor — it needs a type argument")))

(case
  "a bare user generic ctor in an effect-operation type needs a type argument"
  (input
    (do (type (Box a) (Mk a)) (effect E (op emit (-> Box Int64))) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` is a type constructor — it needs a type argument")))

(case
  "a bare MONOMORPHIC type in a variant payload is valid (not a missing-argument ctor)"
  (input (do (type Color (Red) (Green)) (type W (Wrap Color)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; A bare PRELUDE type constructor (`List`/`Set`/`Map`/`Qty`) in a PARAMETER type annotation — `(: x List)`
; — is CDZ0203, but the message must NOT call it "a value, not a type" (it IS a type, a constructor): it
; names the missing type argument and spells the applied form (`(List Elem)`, `(Map Key Value)`, `(Qty T u)`),
; the bare-name twin of the wrong-arity `(List Int64 Int64)` message. A bare USER generic echoes its own
; parameters (`(Box a)`, `(Pair a b)`); a genuine VALUE misused as a type keeps "is a value, not a type".
; (Migrated from rcdzc a_bare_type_constructor_in_type_position_names_the_missing_argument.)
(case
  "a bare List constructor in a parameter type names the missing element argument"
  (input (do (def (f (: x List)) x) (export f)))
  (error
    CDZ0203
    (message "`List` is a type constructor")
    (message "`(List Elem)`")
    (not "is a value")))

(case
  "a bare Set constructor in a parameter type names the missing element argument"
  (input (do (def (f (: x Set)) x) (export f)))
  (error CDZ0203 (message "`Set` is a type constructor") (message "`(Set Elem)`")))

(case
  "a bare Map constructor in a parameter type names its two missing arguments"
  (input (do (def (f (: x Map)) x) (export f)))
  (error CDZ0203 (message "`Map` is a type constructor") (message "`(Map Key Value)`")))

(case
  "a bare Qty constructor in a parameter type names its missing arguments"
  (input (do (def (f (: x Qty)) x) (export f)))
  (error CDZ0203 (message "`Qty` is a type constructor") (message "`(Qty T u)`")))

(case
  "a bare user generic in a parameter type echoes its own parameter"
  (input (do (type Box (W a)) (def (f (: b Box)) b) (export f)))
  (error CDZ0203 (message "`Box` is a type constructor") (message "`(Box a)`")))

(case
  "a bare 2-parameter user generic echoes both parameters"
  (input (do (type Pair (P a b)) (def (f (: x Pair)) x) (export f)))
  (error CDZ0203 (message "`Pair` is a type constructor") (message "`(Pair a b)`")))

(case
  "a genuine value misused as a parameter type keeps the is-a-value message (not the constructor one)"
  (input (do (def helper 5) (def (f (: x helper)) x) (export helper)))
  (error CDZ0203 (message "`helper` is a value, not a type")))

; `Int`/`UInt`/`Float` are the WIDTH-FAMILY value constructors — they build a sized type from a width literal
; (`(Int 64)` ≡ `Int64`), so a bare `(: a Int)` uses a VALUE as a type (the near-universal newcomer reflex, as
; `int`/`float` name a type in most languages). CDZ0203, but the message names the concrete sized DEFAULT the
; author likely meant (`use Int64`) + another admitted width, and carries a one-shot Replace fix to the default
; (the rustc "perhaps you meant `i32`" analogue) — NOT the opaque "is a value, not a type". Applying the fix
; (`Int` → `Int64`) type-checks clean. (Migrated from rcdzc
; a_bare_width_ctor_in_type_position_suggests_the_sized_default_with_a_fix.)
(case
  "a bare Int width-constructor names the sized default with a replace fix"
  (input (do (def (f (: a Int)) a) (export f)))
  (error
    CDZ0203
    (message "`Int` is a width constructor")
    (message "use `Int64`")
    (message "Int32")
    (not "is a value")
    (fix (kind replace) (replacement "Int64"))))

(case
  "a bare UInt width-constructor names the sized default with a replace fix"
  (input (do (def (f (: a UInt)) a) (export f)))
  (error
    CDZ0203
    (message "`UInt` is a width constructor")
    (message "use `UInt64`")
    (message "UInt8")
    (fix (kind replace) (replacement "UInt64"))))

(case
  "a bare Float width-constructor names the sized default with a replace fix"
  (input (do (def (f (: a Float)) a) (export f)))
  (error
    CDZ0203
    (message "`Float` is a width constructor")
    (message "use `Float64`")
    (message "Float32")
    (fix (kind replace) (replacement "Float64"))))

; APPLYING a type name in EXPRESSION position (where a function was expected) — the value-position twin of the
; type-position "needs a type argument" family above. The head reduces to a type-value, so the generic
; typeval discriminator recognizes it (no hard-coded name list). The message DIVERGES by whether the type is
; GENERIC: a NON-generic prelude type (`Int64`, no declared params) reads as a type misplaced where a function
; belongs and points at the annotation form `(: value Int64)`; a GENERIC type ctor (`Option`, ≥1 param) whose
; type-ARGUMENT position wants a type names that a value appears where a type belongs. (Migrated from rcdzc
; applying_a_type_name_names_it_a_type_and_points_at_annotation_position.)
(case
  "applying a non-generic prelude type to a value names it a type, not a function"
  (doc
    "`(Int64 5)` applies the type `Int64` as if it were a function. `Int64` is a prelude type with no
           declared params, so the message names the category — `Int64` is a type, not a function — and points
           at the annotation form `(: value Int64)` where a type legitimately appears, rather than the opaque
           'cannot apply a value of type Int64'.")
  (input (do (def (main) (Int64 5)) (export main)))
  (error CDZ0203 (message "is a type, not a function")))

; The ARGUMENT-position twin of the type-in-head case above: a value juxtaposed with a type WITHOUT the
; colon — `(5 Int64)` for `(: 5 Int64)`. A plain value applied to one arg that resolves as a TYPE reads as
; applying a non-function; rather than the opaque "cannot apply a value of type Int64", CDZ0201 names the
; missing-colon repair + carries a heuristic add-`:` fix. Compound-type has no name to splice (no fix); a
; non-type argument stays the generic message (no false positive). (migrated from rcdzc
; a_value_juxtaposed_with_a_type_names_the_missing_colon_annotation.)
(case
  "a value juxtaposed with a type without the colon names the missing-colon annotation with an add-: fix"
  (input (do (def (main) (5 Int64)) (export main)))
  (error
    CDZ0201
    (message "`(: <value> <Type>)`")
    (message "leading `:`")
    (message "juxtaposed with a type")
    (fix (kind replace) (replacement "(: 5 Int64)") (unverified))))

(case
  "a value juxtaposed with a COMPOUND type names the shape but carries no fix"
  (input (do (def (main) (5 (List Int64))) (export main)))
  (error CDZ0201 (message "juxtaposed with a type") (no-fix)))

(case
  "applying a value to a NON-type value keeps the generic not-a-function message, not missing-colon"
  (doc
    "`(5 6)` applies a value to a non-type value — the generic not-a-function reject (CDZ0201 'cannot
           apply a value of type Int64 — it is not a function'), NOT the missing-colon repair (which is only
           offered when the argument resolves as a TYPE). Also pins the DEDUP (migrated from rcdzc
           applying_a_non_function_reports_one_error_not_a_shadowing_decline): applying a non-function is
           EXACTLY ONE error — the coded not-a-function reject — NOT that reject PLUS the emit path's uncoded
           'value is not applyable' decline for the same node (both would surface as error:, reading as two);
           `dedup_faults` drops the weaker decline when the coded reject is present, so the count is 1.")
  (input (do (def (main) (5 6)) (export main)))
  (error
    CDZ0201
    (message "cannot apply a value of type")
    (message "it is not a function")
    (not "juxtaposed")
    (count 1)))

(case
  "applying a generic type constructor to a value names the type-argument position"
  (doc
    "`(Option 5)` applies the GENERIC type constructor `Option` to a value. Its type-argument position
           wants a TYPE, so the message names that — `Option` is a type constructor, its type argument must be
           a type, but a value appears here — the sum twin of List/Set's 'the element type must be a type'.")
  (input (do (def (main) (Option 5)) (export main)))
  (error CDZ0203 (message "its type argument must be a type")))

; ── OVER/WRONG-arity generic type application (migrated from rcdzc
; a_wrong_arity_generic_type_application_in_an_annotation_is_cdz0203_for_user_and_builtin) ──
; A generic type applied with the WRONG NUMBER of type arguments — over-supplied, under-supplied, or bare —
; rejects CDZ0203 with an ACTIONABLE message naming the true arity (correct singular/plural grammar) and the
; canonical name, UNIFORMLY for a USER generic and a BUILT-IN (the #1683 user-generic-by-name path agrees with
; the built-in one and does not panic / silently accept).
(case
  "a built-in generic Option over-applied with two type arguments is rejected with the arity"
  (input (do (def (f (: x (Option Int64 Bool))) 0) (def (main) (f (Some 1))) (export main)))
  (error CDZ0203 (message "`Option` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a user generic Box over-applied with two type arguments is rejected with the same arity shape"
  (input (do (type (Box a) (Mk a)) (def (f (: x (Box Int64 Bool))) 0) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a multi-parameter user generic Pair under-applied with one type argument names the plural arity"
  (input
    (do (type (Pair a b) (Both a b)) (def (f (: x (Pair Int64))) 0) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Pair` takes 2 type arguments") (message "but 1 was supplied")))

(case
  "a user generic Box applied with ZERO type arguments names the arity and the zero supplied"
  (input (do (type (Box a) (Mk a)) (def (f (: x (Box))) 0) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` takes 1 type argument") (message "but 0 were supplied")))

; The same wrong-arity check fires in a VARIANT-PAYLOAD type position at the DECLARATION, not only in an
; annotation: `(type W (Wrap (Box Int64 Bool)))` over 1-arg `(Box a)` was SILENTLY ACCEPTED (a user generic
; REDUCES to a Ty::Sum dropping the extra arg, so validate_type_position's typeval_of early-return waved it
; through), and the mis-arity surfaced only LATER as a confusing construction-site CDZ0201. Now the arity
; check runs BEFORE the typeval_of return, rejecting CDZ0203 at the declaration with the same actionable
; message. Migrated from rcdzc a_wrong_arity_generic_in_a_variant_payload_is_cdz0203_at_the_declaration.
(case
  "a wrong-arity user generic in a variant payload is rejected at the declaration"
  (input (do (type (Box a) (Mk a)) (type W (Wrap (Box Int64 Bool))) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a wrong-arity built-in generic in a variant payload is rejected at the declaration"
  (input (do (type W (Wrap (Option Int64 Bool))) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Option` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a RIGHT-arity generic variant payload constructs and matches (no false wrong-arity)"
  (input
    (do
      (type (Box a) (Mk a))
      (type W (Wrap (Box Int64)))
      (def (main) (match (Wrap (Mk 5)) ((Wrap b) (match b ((Mk v) v)))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a PARAM-parameterized generic payload (Box a) inside another generic is valid, not wrong-arity"
  (input (do (type (Box a) (Mk a)) (type (Pair a) (P (Box a))) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; ── PRELUDE type-constructor wrong-arity (migrated from rcdzc
; a_prelude_type_constructor_with_the_wrong_arity_names_its_expected_argument_count) ──
; A PRELUDE type constructor applied to the wrong number of type arguments reduces to NO type-value, so it
; used to read as the generic "a parameter's annotation requires a type, but found a non-type" — misleading,
; since List/Map/Set/Int/UInt/Qty/-> ARE type constructors, just misapplied. Each now names the constructor +
; its expected-vs-supplied arity (correct singular/plural) and spells the fix (rustc's "this type takes N
; generic arguments but M were supplied"). The correct arities raise no fault (covered by the working
; List/Int/Qty/arrow cases across the corpus); a genuine non-type keeps the generic "requires a type" message.
(case
  "a prelude List over-applied with two type arguments names its arity"
  (input (do (def (g (: xs (List Int64 Int64))) xs) (export g)))
  (error CDZ0203 (message "`List` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a prelude Map under-applied with one type argument names its two-argument arity"
  (input (do (def (g (: mp (Map Int64))) mp) (export g)))
  (error CDZ0203 (message "`Map` takes 2 type arguments") (message "but 1 was supplied")))

(case
  "a prelude Set over-applied with two type arguments names its one-argument arity"
  (input (do (def (g (: s (Set Int64 Bool))) s) (export g)))
  (error CDZ0203 (message "`Set` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a wrong-arity prelude ctor in a VALUE annotation routes through the same arity helper"
  (input (do (def (g) (: 5 (List Int64 Int64))) (export g)))
  (error CDZ0203 (message "`List` takes 1 type argument") (message "but 2 were supplied")))

(case
  "the WIDTH-indexed Int constructor with zero width arguments names the width arity"
  (input (do (def (main) (: 5 (Int))) (export main)))
  (error
    CDZ0203
    (message "`Int` is a WIDTH-indexed type constructor")
    (message "but 0 arguments were supplied")))

(case
  "the WIDTH-indexed UInt constructor with zero width arguments names the width arity"
  (input (do (def (main) (: 5 (UInt))) (export main)))
  (error
    CDZ0203
    (message "`UInt` is a WIDTH-indexed type constructor")
    (message "but 0 arguments were supplied")))

(case
  "the WIDTH-indexed Int constructor with two width arguments names the width arity"
  (input (do (def (main) (: 5 (Int 32 64))) (export main)))
  (error
    CDZ0203
    (message "`Int` is a WIDTH-indexed type constructor")
    (message "but 2 arguments were supplied")))

(case
  "a prelude Qty under-applied with one type argument names its two-argument arity"
  (input (do (def (g (: q (Qty Int64))) q) (export g)))
  (error CDZ0203 (message "`Qty` takes 2 type arguments") (message "but 1 was supplied")))

(case
  "a prelude Qty with zero type arguments names its two-argument arity"
  (input (do (def (g (: q (Qty))) q) (export g)))
  (error CDZ0203 (message "`Qty` takes 2 type arguments") (message "but 0 were supplied")))

(case
  "the arrow type constructor with zero arguments names the arrow shape and its minimum"
  (input (do (def (g (: h (->))) 0) (export g)))
  (error CDZ0203 (message "an arrow type is") (message "it needs at least a result type")))

; ── Applying a MONOMORPHIC sum type to arguments (migrated from rcdzc
; applying_a_monomorphic_sum_type_to_arguments_says_it_takes_no_type_parameters) ──
; `(: t (T Int64))` where `(type T …)` is MONOMORPHIC (zero declared params) parses as applying `T` to
; `Int64`; since `T` reduces to a type-value with ZERO params, the message names the exact fix — write `T`,
; not `(T …)` — and carries a REPLACE fix stripping the spurious args (heuristic: right in annotation
; position, so unverified). A user `(Color 5)` in value call position takes the same precise message + fix.
(case
  "annotating with a monomorphic sum applied to a type argument says it takes no type parameters"
  (input
    (do
      (type T (Leaf Int64) (Node Int64))
      (def (f (: t (T Int64))) (match t ((T.Leaf n) n) ((T.Node n) n)))
      (def (main) (f (T.Leaf 5)))
      (export main)))
  (error
    CDZ0203
    (message "is a type that takes no type parameters")
    (fix (kind replace) (replacement "T") (unverified))))

(case
  "applying a monomorphic sum in value position says it takes no type parameters"
  (input (do (type Color R G B) (def (main) (Color 5)) (export main)))
  (error
    CDZ0203
    (message "is a type that takes no type parameters")
    (fix (kind replace) (replacement "Color") (unverified))))

; ── USER generic sum wrong-arity (migrated from rcdzc
; a_user_generic_sum_with_the_wrong_type_arg_count_names_its_expected_arity) — the user-sum twin of the
; prelude-ctor arity block above. A user generic sum applied to the wrong number of type args REDUCES to a
; Ty::Sum (silently dropping/defaulting args), so it once compiled clean; the arity is now checked off the
; sum's declared param count and names the fix, echoing the sum's own parameter names. ──
(case
  "a user generic sum over-applied with two type arguments names its one-argument arity"
  (input (do (type Box (W a) (E)) (def (g (: b (Box Int64 Bool))) b) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Box` takes 1 type argument") (message "but 2 were supplied")))

(case
  "a multi-parameter user generic sum under-applied names its two-argument arity"
  (input (do (type Pair (P a b)) (def (g (: p (Pair Int64))) p) (def (main) 0) (export main)))
  (error CDZ0203 (message "`Pair` takes 2 type arguments") (message "but 1 was supplied")))

(case
  "a wrong-arity user generic sum in a VALUE annotation routes through the same arity check"
  (input (do (type Box (W a) (E)) (def (g) (: 5 (Box Int64 Bool))) (export g)))
  (error CDZ0203 (message "`Box` takes 1 type argument") (message "but 2 were supplied")))

; The single-param applied case above covers the flat one-argument face; the resolve path also handles
; MULTI-parameter generics and NESTED type-arguments (a user generic inside a built-in generic, or inside
; another user generic). Each was equally CDZ0101-unresolvable under the parenthesized-head `""`-name bug
; (#1683/#1700) and now resolves like a built-in — the three faces below pin that the head-param collect
; handles arity > 1 and the applied-ctor reduction recurses into a nested type-argument.
(case
  "a MULTI-parameter user generic resolves by name in a type annotation applied with two arguments"
  (doc
    "The arity-greater-than-1 face of the parenthesized-head resolve: a two-parameter generic `(type (Pair a b)
           (Both a b))` applied with two concrete arguments in an annotation `(: p (Pair Int64 Bool))` must
           resolve by name — the head-param collect harvests BOTH `a` and `b`, and the applied-ctor
           reduction binds both type arguments (the single-param `(Container Int64)` case above exercised
           only arity 1). `(fst (Both 9 true))` projects the first field `9`. Pins that a multi-parameter
           user generic is resolvable and correctly instantiated in an annotation, not just a single-param one.")
  (input
    (do
      (type (Pair a b) (Both a b))
      (def (fst (: p (Pair Int64 Bool))) (match p ((Both x y) x)))
      (def (main (: k Int64)) (fst (Both k true)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a user generic NESTED as the type argument of a built-in generic resolves in an annotation"
  (doc
    "The nested-in-built-in face: a user generic `(Box a)` appears as the type ARGUMENT of the
           built-in `Option` in an annotation `(: b (Option (Box Int64)))`. The applied-ctor reduction must
           recurse into the nested position and resolve the user generic by name there, exactly as it does
           at top level — a nested user generic was equally unresolvable under the `\"\"`-name bug. `(unwrap
           (Some (Mk 4)))` peels the `Option` then the `Box` to recover `4`. Pins that the resolve reaches a
           user generic nested inside a built-in generic's argument.")
  (input
    (do
      (type (Box a) (Mk a))
      (def (unwrap (: b (Option (Box Int64)))) (match b ((Some x) (match x ((Mk v) v))) (None 0)))
      (def (main (: k Int64)) (unwrap (Some (Mk k))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64)))

(case
  "a user generic NESTED as the type argument of another user generic resolves in an annotation"
  (doc
    "The nested-in-user face: the user generic `(Box a)` is instantiated at `(Box Int64)` and that
           itself is the type argument to `Box` again — `(: b (Box (Box Int64)))`. Both the outer and the
           nested occurrence resolve the same user generic by name (the reduction recurses through a user
           generic's own argument, not only a built-in's). `(unwrap (Mk (Mk 6)))` peels both `Box` layers to
           `6`. Pins the doubly-nested user-generic annotation, the all-user-sum companion of the
           user-in-built-in case above.")
  (input
    (do
      (type (Box a) (Mk a))
      (def (unwrap (: b (Box (Box Int64)))) (match b ((Mk x) (match x ((Mk v) v)))))
      (def (main (: k Int64)) (unwrap (Mk (Mk k))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 6 Int64)))

; The resolve faces above pin CORRECT-arity user-generic applications. The WRONG-arity user-generic
; applications must reject CDZ0203 with the true arity — exactly as a built-in generic does — now that a
; user generic resolves by name (#1683): over-supplied (a 1-param generic given 2 args) and under-supplied
; (a 2-param generic given 1). Distinct from the MONOMORPHIC over-application above (`(T Int64)`, which
; takes ZERO params): here the ctor IS generic but the arg COUNT is wrong. The two faces below pin the
; user-generic arity check agrees with the built-in's and doesn't panic or silently accept.
(case
  "a user generic sum OVER-applied with too many type arguments in an annotation is rejected"
  (doc
    "`(: x (Box Int64 Bool))` applies the one-parameter user generic `(type (Box a) (Mk a))` to TWO
           type arguments — over-supplied. Now that a user generic resolves by name in a type position
           (#1683), its arity check must fire like a built-in's: `(Option Int64 Bool)` rejects CDZ0203, and
           so must `(Box Int64 Bool)` (naming `Box`'s true arity of 1). Pins the over-supplied user-generic
           face — the ctor is genuinely generic (unlike the monomorphic `(T Int64)` above) but given the
           wrong COUNT; must reject, not truncate the extra arg or panic.")
  (input (do (type (Box a) (Mk a)) (def (f (: x (Box Int64 Bool))) 0) (def (main) 0) (export main)))
  (error CDZ0203))

(case
  "a multi-parameter user generic UNDER-applied in an annotation is rejected"
  (doc
    "`(: x (Pair Int64))` applies the two-parameter user generic `(type (Pair a b) (Both a b))` to only
           ONE type argument — under-supplied. The arity check must reject CDZ0203 naming `Pair`'s true arity
           of 2 (the under-supplied companion of the over-supplied `Box` case, and the wrong-COUNT companion
           of the correct-arity `(Pair Int64 Bool)` resolve case above). Pins that a partially-applied user
           generic in an annotation is a coded reject, not a silently-accepted or defaulted instantiation.")
  (input
    (do (type (Pair a b) (Both a b)) (def (f (: x (Pair Int64))) 0) (def (main) 0) (export main)))
  (error CDZ0203))

; A bare under-applied generic NESTED as another generic's type argument — `(: x (Option Box))` where
; `(Box a)` needs an arg — is a harder sibling of the flat under-application above: it bound a CYCLIC
; substitution `?v := (Option ?v)` that bypassed unify's occurs-check, and the universal type-read then
; chased the cycle forever and CRASHED the compiler (stack overflow, not a catchable decline) when the
; annotated value was consumed. It must reject CDZ0203 cleanly. The two cases below pin the clean reject +
; a valid deeply-nested control (a resolved cycle would break the valid case too).
(case
  "a bare under-applied generic nested as a type argument declines, does not stack-overflow the compiler"
  (doc
    "`(: x (Option Box))` annotates a CONSUMED parameter with the built-in `Option` whose type argument
           is the BARE under-applied user generic `Box` (`(type (Box a) (Mk a))` needs one arg). This bound a
           cyclic substitution `?v := (Option ?v)` bypassing the occurs-check; `Subst::apply` (every type
           read funnels through it) then chased the cycle and STACK-OVERFLOWED rcdzc — a compiler crash, the
           worst class (not even a catchable panic), triggered when `x` is matched + the fn called. The fix
           caps the apply var-chain/descent so a cycle breaks to `Ty::Any` and the fault walk reports a clean
           CDZ0203. Pins that a nested under-applied generic DECLINES, never crashes the compiler — the
           nested (cycle-inducing) sibling of the flat `(: b Container)` bare-under-application above.")
  (input
    (do
      (type (Box a) (Mk a))
      (def (f (: x (Option Box))) (match x ((Some b) (match b ((Mk v) v))) ((None) 0)))
      (def (main) (f (Some (Mk 5))))
      (export main)))
  (error CDZ0203))

(case
  "a nullary generic producer argument with no element source declines with a nullary-specific message"
  (doc
    "`(count (empty))` feeds a NULLARY generic producer `empty : forall a. GIter a` into a generic
           recursive consumer — nothing determines the element type, so it cannot monomorphize (CDZ0201).
           The message names the NULLARY-PRODUCER shape (its workaround is to annotate THAT argument), not
           the generic three-shape 'annotate a nested argument' advice which does not apply to a producer
           that takes no argument.")
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def (empty) (GIter.Nil))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons h rest) (+ 1 (count rest)))))
      (def (main) (count (empty)))
      (export main)))
  (error CDZ0201 (message "nullary generic producer")))

(case
  "annotating a nullary generic producer's result grounds the element and it runs"
  (doc
    "The workaround the message above names: annotating the producer call's result
           `(: (empty) (GIter Int64))` grounds the element type, so the program compiles and runs —
           `count` of the empty GIter is 0. The passing companion of the nullary-producer decline.")
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def (empty) (GIter.Nil))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons h rest) (+ 1 (count rest)))))
      (def (main) (count (: (empty) (GIter Int64))))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a valid deeply-nested generic type argument still resolves and runs (the crash-guard control)"
  (doc
    "The passing control for the cyclic-substitution guard above: the SAME shape but the nested generic
           is FULLY applied — `(: x (Option (Box Int64)))` — so no cycle forms and the annotation resolves
           normally. `(f (Some (Mk 9)))` peels the `Option` then the `Box` to recover 9. Pins that the
           apply-depth cap (which breaks a genuine cycle to `Ty::Any`) does NOT over-decline a well-formed
           deeply-nested generic — a real program never approaches the limit.")
  (input
    (do
      (type (Box a) (Mk a))
      (def (f (: x (Option (Box Int64)))) (match x ((Some b) (match b ((Mk v) v))) ((None) 0)))
      (def (main) (f (Some (Mk 9))))
      (export main)))
  (call main)
  (output (: 9 Int64)))

; A generic type parameter that appears inside a NESTED payload shape (not just a bare `(Mk a)`) must
; thread through the inner container and let the value compute: a param inside a built-in List payload, a
; param used TWICE in one variant (both positions unify at one type), and a param inside a Tuple payload.
(case
  "a generic type parameter inside a List payload resolves and the value computes"
  (doc
    "`(type (Box a) (Mk (List a)))` puts the parameter `a` inside a built-in List; `(Mk (list 1 2))`
           instantiates `a = Int64` and `(match … ((Mk xs) (List.len xs)))` reads the list back = 2.")
  (input
    (do
      (type (Box a) (Mk (List a)))
      (def (main) (match (Mk #list(1 2)) ((Mk xs) (List.len xs))))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a generic type parameter used twice in one variant unifies both fields"
  (doc
    "`(type (Pair a) (P a a))` uses the parameter `a` in BOTH payload positions; `(P 1 2)` unifies
           both at Int64 and `(match … ((P x y) (+ x y)))` reads them back = 3.")
  (input (do (type (Pair a) (P a a)) (def (main) (match (P 1 2) ((P x y) (+ x y)))) (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a generic type parameter inside a Tuple payload threads and the value computes"
  (doc
    "`(type (T a) (W (Tuple a a)))` threads the parameter `a` into a Tuple payload; `(W (tuple 1 2))`
           instantiates `a = Int64` and `(match … ((W t) (+ (. t 0) (. t 1))))` projects both = 3.")
  (input
    (do
      (type (T a) (W (Tuple a a)))
      (def (main) (match (W #tuple(1 2)) ((W t) (+ (. t 0) (. t 1)))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

; A `(type …)` declaration may be NESTED in a `do` block (a LOCAL sum, not a top-level item): the nested
; declaration is gathered and its sum record synthesized, the do-form walks skip a `(type …)` form as a
; declaration (like a `def`) so its `type` head is not resolved as an unbound value, and the type + variant
; names resolve program-wide by nominal identity. A do-block ending in a `(type …)` (nothing to yield) is
; malformed, like a trailing `def`.
(case
  "a nullary sum type declared inside a do block resolves and matches"
  (doc
    "`(do (type C R G B) (match R (R 1) (G 2) (B 3)))` declares the enum `C` LOCALLY inside `main`'s
           `do`; `R`/`G`/`B` resolve although the type is not top-level. Constant scrutinee `R` folds to 1.")
  (input (do (def (main) (do (type C R G B) (match R (R 1) (G 2) (B 3)))) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a payload sum type declared inside a do block resolves and matches its payload"
  (doc
    "`(do (type Box (Bx Int64)) (match (Bx 5) ((Bx x) x)))` declares the payload-variant type locally;
           `Bx` constructs and destructures although declared inside the `do`. Folds to 5.")
  (input (do (def (main) (do (type Box (Bx Int64)) (match (Bx 5) ((Bx x) x)))) (export main)))
  (call main)
  (output (: 5 Int64)))

; The arity rejects above (a non-generic type over-applied, and a generic type over-/under-supplied) are
; about the NUMBER of type arguments. The dual slip is a legitimately GENERIC type constructor — one that
; DOES take a type parameter — applied to a VALUE where a type belongs: `(Option 5)`, `(List 5)`. Here the
; arity is right (Option/List take one parameter) but the argument's KIND is wrong (a value, not a type).
; This is a distinct diagnostic
; (CDZ0203) that names the type-argument position — "the type argument must be a type, but a value
; appears here" — rather than the "not a type constructor" / "takes no type parameters" arity messages,
; so an author who wrote a value where the element/payload TYPE goes is told to write a type.
(case
  "a generic sum type constructor applied to a value is rejected"
  (doc
    "`(: 1 (Option 5))` uses the generic type constructor `Option` — which correctly takes one type
           parameter — but supplies the VALUE `5` where a TYPE belongs. Unlike the `(Int64 Int64)` and `(T
           Int64)` cases above (a NON-generic type over-applied), the arity is right; the fault is the
           argument's kind. Rejected (CDZ0203), the diagnostic naming the type-argument position (`(Option
           <Type>)`, e.g. `(Option Int64)`). Pins that a generic type's argument must itself be a type,
           not a value.")
  (input (do (def (main) (: 1 (Option 5))) (export main)))
  (error CDZ0203))

(case
  "a list type whose element position holds a value is rejected"
  (doc
    "`(: 1 (List 5))` puts the value `5` in `List`'s element-TYPE position. `List` is a generic type
           constructor whose one argument is the element TYPE — a value there is ill-formed (CDZ0203, 'the
           element type must be a type, but this is a value'). The built-in-collection companion of the
           `(Option 5)` case: a value where a type is required in a generic type's argument.")
  (input (do (def (main) (: 1 (List 5))) (export main)))
  (error CDZ0203))

(case
  "a set type whose element position holds a value is rejected"
  (doc
    "`(: 1 (Set 5))` — the `Set` sibling of the list case: the element-TYPE position holds the value
           `5`, rejected (CDZ0203). Pins that the generic-argument-must-be-a-type check covers `Set` as
           well as `List`, so no built-in generic collection accepts a value in its type-argument slot.")
  (input (do (def (main) (: 1 (Set 5))) (export main)))
  (error CDZ0203))

(case
  "an unbound name as a parameter's annotation type is rejected"
  (doc
    "The PARAMETER-annotation companion: `(def (f (: x foo)) x)` annotates the parameter `x` with
           the unbound `foo` in type position. A parameter's type operand must denote a type exactly as a
           value annotation's does, so the unbound `foo` rejects CDZ0101 (was accepted, `(f 7)` ran to 7
           — the garbage parameter type silently typed `x` as unconstrained). Pins that the type-operand
           validation covers a signature parameter, not only a value annotation.")
  (input (do (def (f (: x foo)) x) (def (main) (f 7)) (export main)))
  (error CDZ0101))

(case
  "a literal as a parameter's annotation type is rejected"
  (doc
    "The parameter-annotation companion of the value-annotation `(: 5 42)` literal case: `(def (f (:
           x 42)) x)` annotates the parameter `x` with the VALUE `42` in type position. A literal is not a
           type, so it rejects CDZ0203 ('expected a type') just as it does in a value annotation — the
           type-operand validation is uniform across both annotation forms. (migrated from rcdzc
           a_non_type_in_a_type_annotation_position.)")
  (input (do (def (f (: x 42)) x) (def (main) (f 7)) (export main)))
  (error CDZ0203))

; A BOUND VALUE name misused as a type — `(: x helper)` where `helper` is a value `def`, distinct from an
; UNBOUND name (`foo` → CDZ0101 above) and a LITERAL (`42` → the generic "expected a type"). It IS bound, but
; to a value, not a type, so CDZ0203 NAMES it: "`helper` is a value, not a type" (the type-position analogue
; of the apply-position category message), across all THREE annotation sites — a parameter annotation, a
; value annotation, and a let-binder annotation (three parallel producers that must share the naming).
; (Migrated from rcdzc a_value_name_in_type_position_is_named_a_value_not_a_generic_non_type.)
(case
  "a bound value name as a parameter's annotation type is named a value, not a type"
  (input (do (def helper 5) (def (f (: x helper)) x) (export helper)))
  (error CDZ0203 (message "`helper` is a value, not a type")))

(case
  "a bound value name as a value's annotation type is named a value, not a type"
  (input (do (def helper 5) (def (main) (: 3 helper)) (export main)))
  (error CDZ0203 (message "`helper` is a value, not a type")))

(case
  "a bound value name as a let-binder annotation type is named a value, not a type"
  (input (do (def helper 5) (def (main) (let (((: x helper) 3)) x)) (export main)))
  (error CDZ0203 (message "`helper` is a value, not a type")))

(case
  "a genuine type mismatch in an annotation still rejects (no over-rejection)"
  (doc
    "The no-over-rejection control's REJECT half as a standalone case: `(: 5 Bool)` names a real type
           `Bool` that CONTRADICTS the Int64 value `5`, so it rejects CDZ0203 — the validation of the type
           OPERAND (is it a type?) does not swallow the ordinary value-vs-type mismatch check. Pins that
           annotating with a well-formed but wrong type still faults. (migrated from rcdzc
           a_non_type_in_a_type_annotation_position.)")
  (input (do (def (main) (: 5 Bool)) (export main)))
  (error CDZ0203))

; A value used where a SUM is expected, whose type matches a variant's PAYLOAD, is CDZ0203 with a
; wrap-in-variant fix — "wrap the value in `Some`" + a WRAP replace `(Some …)` (the `…` marks where the
; original goes). This applies at an ANNOTATION `(: n Option)` and at a CALL SITE (a mistyped argument to a
; sum parameter). The wrap is offered only when the variant is FORCED: `(Result Int64 Int64)` given an
; Int64 could be `Ok` OR `Err` (ambiguous) → NO fix; `(Result Int64 String)` given an Int64 → only `Ok`
; fits → `(Ok …)`. (Migrated from rcdzc an_annotation_mismatch_to_a_sum_offers_a_wrap_in_variant_fix +
; a_mistyped_argument_to_a_sum_parameter_offers_a_wrap_in_variant_fix.)
(case
  "an annotation mismatch to a sum offers a wrap-in-variant fix"
  (input (do (type Option (Some Int64) None) (def (f (: n Int64)) (: n Option)) (export f)))
  (error
    CDZ0203
    (message "wrap the value in `Some`")
    (fix (kind wrap) (replacement "(Some …)") (unverified))))

(case
  "a mistyped argument to a sum parameter offers a wrap-in-variant fix at the call site"
  (input (do (def (f (: o (Option Int64))) o) (def (main) (f 5)) (export main)))
  (error CDZ0203 (fix (kind wrap) (replacement "(Some …)") (unverified))))

(case
  "an ambiguous wrap (both Result arms fit the payload) offers no fix"
  (input (do (def (f (: r (Result Int64 Int64))) r) (def (main) (f 5)) (export main)))
  (error CDZ0203 (no-fix)))

(case
  "a forced-choice wrap into the only fitting Result arm offers that arm"
  (input (do (def (f (: r (Result Int64 String))) r) (def (main) (f 5)) (export main)))
  (error CDZ0203 (fix (kind wrap) (replacement "(Ok …)") (unverified))))

(case
  "an arrow parameter annotation compiles and applies (a positive control)"
  (doc
    "A positive control proving the type-operand validation does not over-reject a WELL-FORMED arrow
           type: an arrow-typed parameter `(: g (-> Int64 Int64))` applied to a lambda compiles and runs.
           (migrated from rcdzc a_non_type_in_a_type_annotation_position.)")
  (input
    (do (def (f (: g (-> Int64 Int64))) (g 1)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a well-formed annotation still checks and accepts a matching type (the control)"
  (doc
    "The control pinning the rejects above are about VALIDATING the type operand, not annotations
           in general: `(: 5 Int64)` matches the value's type and is accepted (5); a mismatch `(: 5 Bool)`
           still rejects CDZ0203; and a real parameter annotation `(: n Int64)` compiles. So the
           annotation machinery works for real types — the gap was specifically a NON-type operand
           accepted-and-ignored.")
  (input (do (def (main) (: 5 Int64)) (export main)))
  (call main)
  (output (: 5 Int64)))

; A value that ESCAPES to the host must have a FULLY DETERMINED type — a value whose payload/element
; type is an unresolved variable (a bare `(None)` : `(Option ?0)`) has no defined serialization. Such an
; escape is rejected for its AMBIGUOUS TYPE (CDZ0203, the type-determination fault — annotate to resolve
; it), NOT for its export SHAPE: `(def (main) (None)) (export main)` IS a single nullary export (the
; escape path's shape is satisfied), so a shape-restriction message would misdiagnose. The ambiguity
; bites ONLY at an unannotated escape — a CONSUMED bare `None` (matched, or passed to a typed parameter)
; constrains the payload and type-checks fine, and an ANNOTATED escape resolves the variable and crosses.
(case
  "an escaped value with an unresolved payload type is rejected as ambiguous, not for its export shape"
  (doc
    "`(def (main) (None)) (export main)` returns a bare `None`, whose type is `(Option ?0)` — the
           payload is a free variable nothing constrains, so the escaped value has no defined
           serialization and is rejected (CDZ0203). The program IS a single nullary export, so the reject
           must name the UNRESOLVED TYPE and the annotation fix, not an export-shape restriction (the
           prior message wrongly said the sum 'crosses only as a single nullary export's result' — which
           it already is). An annotated `(: (None) (Option Int64))` escapes fine, and a consumed bare
           `None` type-checks — the ambiguity is escape-only.")
  (input (do (def (main) (None)) (export main)))
  (error CDZ0203))

(case
  "an annotated escaped None renders its canonical nullary-variant form (the control)"
  (doc
    "The control pinning the reject above is ONLY the missing payload type: annotating the bare
           `None` to `(Option Int64)` fully determines the type, and it escapes as the program result,
           rendering the canonical `(None unit)` form. Same shape (a single nullary export returning a
           sum) as the rejected case — the only difference is the annotation resolves `?0`. Pins the
           escape path works once the payload type is known.")
  (input (do (def (main) (: (None) (Option Int64))) (export main)))
  (output (: (None unit) (Option Int64))))

; An EXPORTED function with an unannotated parameter and NO grounding call site leaves the parameter
; type unconstrained at the export boundary — a rejection naming the ambiguity / the annotate fix.
; Contrast the polymorphic-instantiation cases (a call site grounds `id`'s param); a direct export has
; nothing to infer from. (Migrated from rcdzc an_unannotated_exported_parameter_declines.)
(case
  "an exported function with an unannotated parameter is rejected as ambiguous"
  (input (do (def (id x) x) (export id)))
  (error CDZ0201 (message "ambiguous")))

(case
  "a USER-declared monomorphic sum's Some variant escapes to the host rendering its bare name"
  (doc
    "A user `(type Option (Some Int64) None)` (monomorphic, shadowing the prelude generic Option) —
           `(Option.Some 5)` escapes as the program result, a compile-time CONSTANT whose bytes are baked, and
           renders `(: (Some 5) Option)` (the variant by its BARE name, the type as the user's monomorphic
           `Option`). Relocated from rcdzc a_nullary_sum_export_escapes_to_the_host (its constant-baked
           no-value-heap-import compile pin stays in rcdzc).")
  (input (do (type Option (Some Int64) None) (def (main) (Option.Some 5)) (export main)))
  (output (: (Some 5) Option)))

(case
  "a USER-declared monomorphic sum's nullary None variant escapes as its unit payload"
  (doc
    "The nullary arm: over the same user `(type Option (Some Int64) None)`, `Option.None` escapes and
           renders `(: (None unit) Option)` — a nullary variant carries the unit value and renders by its bare
           name. A compile-time constant (bytes baked). Relocated from rcdzc
           a_nullary_variant_export_escapes_as_unit_payload.")
  (input (do (type Option (Some Int64) None) (def (main) Option.None) (export main)))
  (output (: (None unit) Option)))

(case
  "a consumed bare None type-checks without annotation (ambiguity is escape-only)"
  (doc
    "`(match (None) ((Some x) x) ((None) 42))` consumes a bare `None`: the match arms constrain the
           payload type variable, so no annotation is needed and the None arm yields 42. Pins that the
           unconstrained-payload rejection is specific to an unannotated ESCAPE — a consumed bare `None`
           is fine, triangulating that the escape reject is a payload-type-ambiguity condition, not an
           export-shape one.")
  (input (do (def (main) (match (None) ((Some x) x) ((None) 42))) (export main)))
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
(case
  "an empty list escaping to the host is rejected as undetermined, like a bare None"
  (doc
    "`(def (main) (list)) (export main)` returns an empty list of type `(List Any)` — the element
           type is undetermined (no element constrains it, so it grounds to `Any`, the collection analogue
           of bare `None`'s free-variable payload). The escaped value has no defined serialization and is
           rejected (CDZ0203, annotate — e.g. `(: (list) (List Int64))`). Pins that the undetermined-escape
           reject catches the `Any` grounding, not only a free `Var`; a `Set.of (list)` is the same fault.
           The message is ACTIONABLE (#7739): it names the unconstrained part + the annotation template,
           pinned below via the `(message …)` substrings — distinct from the bare not-fully-determined the
           Set-path cases pin, this is the escape-result path via an empty list.")
  (input (do (def (main) #list()) (export main)))
  (error CDZ0203 (message "not fully determined") (message "unconstrained part shown above")))

(case
  "an annotated empty list escapes fine (the undetermined-empty-list control)"
  (doc
    "The control pinning the reject above is ONLY the undetermined element type: annotating the empty
           list to `(List Int64)` fully determines it, and it escapes as the program result. Same shape (a
           single nullary export returning a list) as the rejected case — the annotation resolves the
           element. Pins the escape path works once the element type is known, mirroring the annotated-None
           control.")
  (input (do (def (main) (: #list() (List Int64))) (export main)))
  (output (: #list() (List Int64))))

(case
  "a consumed empty list type-checks without annotation (ambiguity is escape-only)"
  (doc
    "`(List.len (list))` consumes an empty list to a scalar `Int64` (0): the result that escapes is a
           determined scalar, not the undetermined `(List Any)`, so no annotation is needed. Pins that the
           undetermined-`Any` rejection is specific to an unannotated ESCAPE of the collection itself — a
           consumed empty list is fine — the collection analogue of the consumed-bare-None case.")
  (input (do (def (main) (List.len #list())) (export main)))
  (output (: 0 Int64)))

; The annotation-contradiction check must hold for a COMPOUND value too, not only a scalar. A tuple /
; sum / record / list is not a scalar type, so annotating one with a scalar type (Int64, Bool, …)
; contradicts the value's type and MUST be rejected (CDZ0203, type-system.md #Annotations Constrain,
; Never Contradict).
(case
  "a tuple annotated as a scalar type is rejected"
  (doc
    "`(: (tuple 1 2) Int64)` annotates a tuple with the scalar type Int64 — a contradiction (a
           tuple is not an Int64), so the compiler rejects it (CDZ0203), or declines if it does not yet
           cover the compound-vs-scalar annotation rule (reject-don't-miscompile).")
  (input (: #tuple(1 2) Int64))
  (error CDZ0203))

(case
  "a sum value annotated as a scalar type is rejected"
  (doc
    "The sum companion: `(: (Some 5) Bool)` annotates an Option value with the scalar type Bool
           — a contradiction (CDZ0203). Pins that the annotation check covers a compound value on the
           value side, not only a scalar.")
  (input (: (Some 5) Bool))
  (error CDZ0203))

; The annotation check must also see a mismatch in the PARAMETER of a compound type, not only at the
; head. `(Some true)` has type `Option Bool`, which cannot unify with `Option Int64` — the head
; constructor `Option` agrees but the payload type does not, so the annotation contradicts the value's
; type and MUST be rejected (CDZ0203, type-system.md #Annotations Constrain, Never Contradict: "A
; program whose annotation cannot be unified with the type inference determines MUST be rejected").
; An annotation checker that unifies only the head constructor and ignores the type parameter would
; ACCEPT this ill-typed program and run it, returning `(Some true)` under a declared `Option Int64` —
; the silent annotation-replaces-inference the section forbids. A generation that does not yet cover
; the payload-level check DECLINES (reject-don't-miscompile); accepting the program is the failure.
(case
  "an option value annotated with the wrong payload type is rejected"
  (doc
    "`(: (Some true) (Option Int64))` annotates a `Some true` (type `Option Bool`) as `Option
           Int64`: the head `Option` matches but the payload `Bool` cannot unify with `Int64`, a
           contradiction (CDZ0203). Pins that the annotation check descends into a compound type's
           PARAMETER, not only its head constructor — a checker that stops at the head silently accepts
           the ill-typed program and runs it, returning `(Some true)` under a wrong declared type
           (type-system.md #Annotations Constrain, Never Contradict). A generation that does not yet
           cover the payload-level check declines rather than accepting (reject-don't-miscompile).")
  (input (: (Some true) (Option Int64)))
  (error CDZ0203))

; The payload-parameter check must RECURSE, at every nesting depth, not only one level down. `(Some (Some
; 5))` has type `Option (Option Int64)`; annotated `Option (Option Bool)`, the outer `Option` and the
; inner `Option` heads agree but the innermost payload `Int64` cannot unify with `Bool` — a contradiction
; two levels deep. It is the same rule as the one-level `(: (Some true) (Option Int64))` case above, so it
; MUST be rejected (CDZ0203). A checker that descends ONE level into the type parameter but compares the
; nested payload only by coarse kind (both are `Option`) accepts the ill-typed program and runs it — the
; deeper-nesting analogue of the head-only gap the one-level case closed. A generation that does not yet
; recurse into the nested parameter declines rather than accepting (reject-don't-miscompile).
(case
  "a nested option value annotated with the wrong inner payload type is rejected"
  (doc
    "`(: (Some (Some 5)) (Option (Option Bool)))` annotates a value of type `Option (Option Int64)`
           as `Option (Option Bool)`: both `Option` heads agree, but the innermost payload `Int64` cannot
           unify with `Bool` — a contradiction two levels deep (CDZ0203), the same rule as the one-level
           `(: (Some true) (Option Int64))` case above. Pins that the annotation's payload check RECURSES
           to any depth, not only one level — a checker that stops after one descent silently accepts the
           ill-typed program and runs it, returning `(Some (Some 5))` under a wrong declared inner type. A
           generation that does not yet recurse into the nested parameter declines rather than accepting.")
  (input (: (Some (Some 5)) (Option (Option Bool))))
  (error CDZ0203))

; Type-checking a DEEPLY-nested generic-sum VALUE must not blow up superlinearly. Each enclosing `(Some x)`
; unifies its payload variable against the (growing) `Option^k Int64` type below it, and the HM occurs-check
; run on that unification used to re-apply the whole substitution at every node — O(size²) per check,
; O(N³) over the N-deep chain (depth 400 = 2.5s, extrapolating to a compile hang around depth ~1500). Walking
; the type through the substitution in place (the standard union-find resolve) makes the occurs-check O(size),
; so the whole nested value is ~quadratic and a linear-size program compiles in linear-ish time. This case
; pins the VALUE compiles to the right answer at a depth (60) that the cubic version already handled but that
; anchors the shape; the pathology it guards is the GROWTH RATE, not this one point. A deep type ANNOTATION
; and a deep nested TUPLE value were already linear — the blowup was specific to the generic-sum constructor.
(case
  "a deeply-nested generic-sum value type-checks and matches its outermost variant"
  (doc
    "A `(Some (Some … (Some 5)))` chain nested 60 deep, matched on its outermost `Some` (returning 1).
           The emitted program is tiny, but type-checking the nested generic-sum constructor applications was
           O(N³) (the HM occurs-check re-applied the full substitution at every node, O(size²) per check, over
           N levels), so a deeper chain hung the compiler. Walking variables through the substitution in place
           makes the occurs-check O(size) and the whole value ~quadratic. A deep type annotation alone and a
           deep nested tuple value were already linear, so the blowup was specific to the generic-sum value.
           The outer match returns 1; the point is that PRODUCING the deep value must not be superlinear.")
  (input
    (do
      (def
        (main)
        (match
          (Some
            (Some
              (Some
                (Some
                  (Some
                    (Some
                      (Some
                        (Some
                          (Some
                            (Some
                              (Some
                                (Some
                                  (Some
                                    (Some
                                      (Some
                                        (Some
                                          (Some
                                            (Some
                                              (Some
                                                (Some
                                                  (Some
                                                    (Some
                                                      (Some
                                                        (Some
                                                          (Some
                                                            (Some
                                                              (Some
                                                                (Some
                                                                  (Some
                                                                    (Some
                                                                      (Some
                                                                        (Some
                                                                          (Some
                                                                            (Some
                                                                              (Some
                                                                                (Some
                                                                                  (Some
                                                                                    (Some
                                                                                      (Some
                                                                                        (Some
                                                                                          (Some
                                                                                            (Some
                                                                                              (Some
                                                                                                (Some
                                                                                                  (Some
                                                                                                    (Some
                                                                                                      (Some
                                                                                                        (Some
                                                                                                          (Some
                                                                                                            (Some
                                                                                                              (Some
                                                                                                                (Some
                                                                                                                  (Some
                                                                                                                    (Some
                                                                                                                      (Some
                                                                                                                        (Some
                                                                                                                          (Some
                                                                                                                            (Some
                                                                                                                              (Some
                                                                                                                                (Some
                                                                                                                                  5))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))
          ((Some inner) 1)
          ((None) 0)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

; The parameter check applies to a LIST's element type too, not only a sum's payload. `(list 1 2)` has
; type `List Int64`; annotated `List Bool`, the head `List` agrees but the element type `Int64` cannot
; unify with `Bool` — a contradiction (CDZ0203), the list analogue of the `Option` payload case. A checker
; that verifies only the head `List` and ignores the element parameter accepts the ill-typed program and
; runs it, returning `(list 1 2)` under a declared `List Bool`. (A list's elements share one type — the
; homogeneity rule — so a single provable element type suffices to contradict the annotation.)
(case
  "a list annotated with the wrong element type is rejected"
  (doc
    "`(: (list 1 2) (List Bool))` annotates a `List Int64` as `List Bool`: the head `List` matches
           but the element type `Int64` cannot unify with `Bool`, a contradiction (CDZ0203), the list
           companion of the option-payload case above. Pins that the annotation's parameter check covers a
           list's element type, not only a sum's payload — a checker that stops at the head `List` silently
           accepts the ill-typed program and runs it. A generation that does not yet check the element
           parameter declines rather than accepting (reject-don't-miscompile).")
  (input (: #list(1 2) (List Bool)))
  (error CDZ0203))

; The parameter check applies to a RECORD's field type too, not only a sum's payload or a list's
; element. `(record (a 1))` has type `(Record (: a Int64))`; annotated `(Record (: a Bool))`, the head
; `Record` and the field name `a` agree but the field's type `Int64` cannot unify with `Bool` — a
; contradiction (CDZ0203), the record analogue of the list-element and option-payload cases above. A
; record's fields are the third structural type (type-system.md #The Structural Types Are Record, Tuple,
; And Sum) beside the tuple's positions and the sum's payload; the annotation-parameter check the cases
; above pin for a tuple position, a sum payload, and a list element MUST also cover a record field, or a
; checker that verifies only the head `Record` and the field NAMES silently accepts the ill-typed
; program and runs it, returning `(record (a 1))` under a declared `(Record (: a Bool))` — the same
; annotation-replaces-inference the section forbids. A generation that does not yet check a record
; field's type parameter declines rather than accepting (reject-don't-miscompile).
(case
  "a record annotated with the wrong field type is rejected"
  (doc
    "`(: (record (a 1)) (Record (: a Bool)))` annotates a `(Record (: a Int64))` as `(Record (: a Bool))`:
           the head `Record` and the field name `a` match but the field's type `Int64` cannot unify with
           `Bool`, a contradiction (CDZ0203), the record companion of the list-element and option-payload
           cases above. Pins that the annotation's parameter check covers a record's field type — the
           third structural type beside a tuple's positions and a sum's payload — not only a sum's payload
           or a list's element. A checker that stops at the head `Record` and the field names silently
           accepts the ill-typed program and runs it, returning `(record (a 1))` under a declared
           `(Record (: a Bool))` (type-system.md #Annotations Constrain, Never Contradict). A generation
           that does not yet check a record field's type declines rather than accepting
           (reject-don't-miscompile).")
  (input (: #record((= a 1)) (Record (: a Bool))))
  (error CDZ0203))

; --- A record's field SET must match the annotation, not only each field's type ------------
; The case above matches the field NAMES and rejects a wrong field TYPE. The dual failure is a field-SET
; mismatch: the value's set of field names differs from the annotation's — a MISSING field (the value
; lacks one the annotation names) or an EXTRA field (the value carries one the annotation does not). A
; record's shape is its fixed set of named fields (type-system.md #A Record Has A Fixed Set Of Named
; Fields), so `(Record (: a Int64))` and `(Record (: a Int64) (: b Int64))` are DIFFERENT types — the check is
; over the whole field set, not a subset/superset relaxation (that widening is row polymorphism, a
; separate opt-in — 15-rows-and-open-sums). Each mismatch is CDZ0203 and the diagnostic names the
; offending field (missing `b` / no such field `c`), the actionable add-missing / delete-extra repair.
(case
  "a record missing a field the annotation names is rejected"
  (doc
    "`(: (record (a 1)) (Record (: a Int64) (: b Int64)))` annotates a one-field record as a two-field
           type — the value is MISSING field `b`. The field sets differ, so the value's type `(Record (: a Int64))` does not match the annotation `(Record (: a Int64) (: b Int64))` (CDZ0203, naming the
           missing `b`). A record type is not satisfied by a value carrying a subset of its fields — field
           presence is static (the row-poly widening that would accept this is a separate opt-in). The
           field-SET companion of the wrong-field-TYPE case above.")
  (input (: #record((= a 1)) (Record (: a Int64) (: b Int64))))
  (error CDZ0203))

(case
  "a record carrying a field the annotation does not name is rejected"
  (doc
    "The dual: `(: (record (a 1) (b 2) (c 3)) (Record (: a Int64) (: b Int64)))` carries an EXTRA field
           `c` the annotation does not name. The field sets differ, so it is rejected (CDZ0203, 'no such
           field `c` on the expected record'). A record value is not accepted against a type with FEWER
           fields — the extra field is not silently dropped. Pins the superset direction of the field-set
           check (the value has more fields than the type).")
  (input (: #record((= a 1) (= b 2) (= c 3)) (Record (: a Int64) (: b Int64))))
  (error CDZ0203))

(case
  "a record whose field is misnamed is both missing and extra"
  (doc
    "`(: (record (a 1) (x 2)) (Record (: a Int64) (: b Int64)))` names its second field `x` where the
           annotation expects `b` — so relative to the annotation the value is simultaneously MISSING `b`
           and carrying an EXTRA `x`. Rejected (CDZ0203) with both faults named ('missing field `b`; no
           such field `x`'). Pins that a single misnamed field surfaces as the combined field-set
           mismatch, the shape a field-name typo takes.")
  (input (: #record((= a 1) (= x 2)) (Record (: a Int64) (: b Int64))))
  (error CDZ0203))

; A record TYPE in a mismatch message renders CAPITALIZED with `(: name T)` ascription fields — the
; type-constructor head an author writes in an annotation (`(Record (: a Bool))`), consistent with
; `Tuple`/`List`/`Map`/`Set` — NOT the lowercase VALUE-constructor spelling `(record …)` (which a type
; annotation rejects "not a type"). So the rendered type ROUND-TRIPS: a reader can copy it straight into an
; annotation and it compiles. (Migrated from rcdzc a_record_type_renders_capitalized_matching_its_annotation_
; spelling — the reject render + the acceptance round-trip.)
(case
  "a record-type mismatch message renders the type capitalized, matching the annotation spelling"
  (input (do (def y (: #record((= a 1)) (Record (: a Bool)))) (export y)))
  (error CDZ0203 (message "(Record (: a Bool))") (message "(Record (: a Int64))") (not "(record (")))

(case
  "the rendered capitalized record type round-trips as a valid annotation and runs"
  (input
    (do (def (f (: r (Record (: a Bool)))) r.a) (def (main) (f #record((= a true)))) (export main)))
  (output (: true Bool)))

; The field-set mismatch above (annotation position, bare) carries an ACTIONABLE repair wherever a record
; literal meets a `(Record …)` type — a function ARGUMENT, a LET-BINDER, or NESTED inside a shared field. A
; single key that is a plausible TYPO of an expected field (`fooo` for `foo`) surfaces as simultaneously
; missing + extra and offers a heuristic RENAME fix on the misspelled key (the same repair a `(. r fooo)`
; access typo gets), drilling into the inner literal when the typo is nested. A genuinely-MISSING field (not a
; typo of a supplied one) is an ADD, not a rename: an insert fix appending `(= <f> (trap "TODO"))` (`trap`
; inhabits any field type, clearing the fault in one shot). No false rename for a genuinely-missing nested
; field. (Migrated from rcdzc a_misspelled_field_in_a_record_argument_offers_a_rename.)
(case
  "a misspelled field in a record ARGUMENT names the field-diff and offers a rename fix"
  (input
    (do
      (def (g (: r (Record (: foo Int64)))) r.foo)
      (def (main) (g #record((= fooo 1))))
      (export main)))
  (error
    CDZ0203
    (message "missing field `foo`")
    (message "no such field `fooo`")
    (fix (kind replace) (replacement-contains "foo"))))

(case
  "a genuinely-missing field in a record ARGUMENT carries an add (insert) fix, not a rename"
  (input
    (do
      (def (g (: r (Record (: x Int64) (: y Int64)))) r.x)
      (def (main) (g #record((= x 1))))
      (export main)))
  (error
    CDZ0203
    (message "missing field `y`")
    (fix (kind insert-into) (replacement-contains "(= y (trap \"TODO\"))"))))

(case
  "a misspelled field in a record LET-BINDER carries the same field rename fix"
  (input
    (do (def (main) (let (((: r (Record (: foo Int64))) #record((= fooo 1)))) 0)) (export main)))
  (error CDZ0203 (fix (kind replace) (replacement-contains "foo"))))

(case
  "a misspelled field in a SHARED nested record drills the rename into the inner literal"
  (input
    (do
      (def (g (: r (Record (: inner (Record (: foo Int64)))))) r.inner)
      (def (main) (g #record((= inner #record((= fooo 1))))))
      (export main)))
  (error CDZ0203 (fix (kind replace) (replacement-contains "foo"))))

(case
  "a genuinely-missing nested field gets no rename fix"
  (input
    (do
      (def (g (: r (Record (: inner (Record (: a Int64) (: b Int64)))))) r.inner)
      (def (main) (g #record((= inner #record((= a 1))))))
      (export main)))
  (error CDZ0203 (no-fix)))

; A field-set mismatch that is a pure OMISSION or a lone SURPLUS (not a typo — no rename applies) carries the
; construction analogue of rustc's "add the missing field" / "no field `z`" edits (diagnostics.md §A Diagnostic
; Carries A Route To A Fix). MISSING fields → an INSERT fix appending a `(= <f> (trap "TODO"))` placeholder per
; missing field (`trap : ∀a. String → a` inhabits any field type, clearing the fault in one shot); a lone
; SURPLUS field → a DELETE fix removing the extra entry. The direct value-annotation site carries the same
; fixes. A field set that is SIMULTANEOUSLY missing and extra with no confident near-miss is ambiguous — the
; message guides, but no mechanical fix. (Migrated from rcdzc
; a_record_field_set_mismatch_offers_add_missing_or_delete_extra_fields.)
(case
  "a record ARGUMENT missing fields carries an add (insert) fix with a placeholder per missing field"
  (input
    (do
      (def (f (: r (Record (: x Int64) (: y Int64) (: z Int64)))) r)
      (def (main) (f #record((= x 1))))
      (export main)))
  (error
    CDZ0203
    (message "missing fields `y`, `z`")
    (fix (kind insert-into) (replacement-contains "(= y (trap \"TODO\"))"))))

(case
  "a record ARGUMENT with a lone surplus field carries a delete fix"
  (input
    (do
      (def (f (: r (Record (: x Int64)))) r)
      (def (main) (f #record((= x 1) (= y 2))))
      (export main)))
  (error CDZ0203 (message "no such field `y`") (fix (kind delete))))

(case
  "the direct value-annotation site also carries the add fix for a missing record field"
  (input (do (def (main) (: #record((= x 1)) (Record (: x Int64) (: y Int64)))) (export main)))
  (error CDZ0203 (fix (kind insert-into) (replacement-contains "(= y (trap \"TODO\"))"))))

(case
  "an ambiguous missing-and-extra record field set (not a typo) gets no mechanical fix"
  (input
    (do
      (def (f (: r (Record (: x Int64) (: y Int64)))) r)
      (def (main) (f #record((= x 1) (= zzzzzz 2))))
      (export main)))
  (error CDZ0203 (no-fix)))

; The TUPLE analogue of the record field-set add/delete: a tuple literal with the wrong ARITY names the gap
; ("expected a tuple with N elements, but this one has M") AND carries the POSITIONAL repair — too FEW gets a
; `(trap "TODO")` placeholder appended per missing trailing position; ONE too many gets the trailing element
; deleted. The value-annotation site carries the same. TWO too many is not one clean delete → no fix. A
; SAME-ARITY per-position TYPE mismatch keeps its own element message with no arity add/delete. (Migrated from
; rcdzc a_tuple_arity_mismatch_offers_add_missing_or_delete_extra_elements.)
(case
  "a tuple ARGUMENT with too few elements carries an add (insert) fix appending a placeholder"
  (input
    (do (def (f (: t (Tuple Int64 Int64 Int64))) t) (def (main) (f #tuple(1 2))) (export main)))
  (error
    CDZ0203
    (message "expected a tuple with 3 elements, but this one has 2")
    (fix (kind insert-into) (replacement-contains "(trap \"TODO\")"))))

(case
  "a tuple ARGUMENT with one too many elements carries a delete fix on the trailing element"
  (input (do (def (f (: t (Tuple Int64 Int64))) t) (def (main) (f #tuple(1 2 3))) (export main)))
  (error
    CDZ0203
    (message "expected a tuple with 2 elements, but this one has 3")
    (fix (kind delete))))

(case
  "the direct value-annotation site also carries the tuple add fix for too few elements"
  (input (do (def (main) (: #tuple(1 2) (Tuple Int64 Int64 Int64))) (export main)))
  (error CDZ0203 (fix (kind insert-into) (replacement-contains "(trap \"TODO\")"))))

(case
  "a tuple ARGUMENT with two too many elements is not one clean delete — no mechanical fix"
  (input (do (def (f (: t (Tuple Int64))) t) (def (main) (f #tuple(1 2 3))) (export main)))
  (error CDZ0203 (no-fix)))

(case
  "a same-arity per-position tuple type mismatch keeps its element message with no add or delete fix"
  (input (do (def (f (: t (Tuple Int64 Int64))) t) (def (main) (f #tuple(1 true))) (export main)))
  (error CDZ0203 (message "element 1 should be Int64") (no-fix)))

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
(case
  "a variant with a wrong-type payload as a direct match scrutinee is a type error"
  (doc
    "`(match (I true) ((I x) x) ((J y) y))` under `(type N (I Int64) (J Int64))` matches a
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
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (main) (match (I true) ((I x) x) ((J y) y)))
      (export main)))
  (error CDZ0201))

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
(case
  "a tuple annotated with the wrong arity is rejected"
  (doc
    "`(: (tuple 1 2) (Tuple Int64 Int64 Int64))` annotates a two-element tuple (type `(Tuple Int64
           Int64)`) as a THREE-element tuple type: a tuple's length is part of its type (type-system.md
           #A Tuple Is Reshaped Positionally …, #The Structural Types Are Record, Tuple, And Sum), so the
           two arities cannot unify — a contradiction (CDZ0203), the arity companion of the wrong-element-
           type cases above. Pins that the annotation check compares a tuple's ARITY, not only its element
           types positionally — a checker that walks the shared positions and ignores the length silently
           accepts the ill-typed program and runs it, returning `(tuple 1 2)` under a declared three-
           element type. The element-type check already fires (`(: (tuple 1 2) (Tuple Int64 Bool))` is
           rejected), so the arity check must reach the same annotation. A generation that does not yet
           check tuple arity declines rather than accepting (reject-don't-miscompile).")
  (input (: #tuple(1 2) (Tuple Int64 Int64 Int64)))
  (error CDZ0203))

(case
  "a tuple annotated with too many elements is rejected"
  (doc
    "The other arity direction: `(: (tuple 1 2 3) (Tuple Int64 Int64))` annotates a THREE-element
           tuple as a two-element type — the value has MORE positions than the annotation. Rejected
           (CDZ0203, 'expected a tuple with 2 elements, but this one has 3'), the too-many companion of the
           too-few case above. Pins that the arity check catches a surplus element as well as a missing
           one (the tuple analog of the record extra-field case), so a tuple's length must match exactly in
           both directions — not merely be at-least or at-most the annotation's.")
  (input (: #tuple(1 2 3) (Tuple Int64 Int64)))
  (error CDZ0203))

(case
  "an unannotated program with a valid typing type-checks and runs"
  (doc
    "Witnesses type-system.md #An Unannotated Program Is Accepted When It Has A Valid Typing: a
           valid typing need not be written by the author; the program type-checks and evaluates to 3.")
  (input (let ((x 1)) (+ x 2)))
  (output (: 3 Int64)))

(case
  "an operation on mismatched types is rejected at compile time"
  (doc
    "Witnesses type-system.md #A Well-Typed Program Does Not Go Wrong via its contrapositive:
           the ill-typed `(+ 1 \"two\")` is caught and rejected (CDZ0201) rather than run.")
  (input (+ 1 "two"))
  (error CDZ0201))

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
(case
  "addition of two strings is rejected — text is not a number"
  (doc
    "`(+ \"ab\" \"cd\")` adds two Strings; arithmetic is not defined on text (Cadenza never coerces
           a String to a number), so the compiler rejects it (CDZ0201). Unlike `(+ 1 \"two\")` the two
           operands are the SAME type — they unify, so this is not the cross-kind mismatch but the
           operator's numeric-operand requirement. Pins the honest 'not defined on String' rejection (the
           `+`-means-concat reflex is a fix suggestion, not an accepted meaning). The message NAMES the real
           type (`arithmetic is not defined on String`), and — because String has a total concatenation op —
           `+` carries a REPLACE fix rewriting the operator head to `(. String concat)`. (fix + message
           migrated from rcdzc arithmetic_on_two_same_typed_non_numeric_operands_names_the_real_type_and_plus_offers_concat.)")
  (input (+ "ab" "cd"))
  (error
    CDZ0201
    (message "arithmetic is not defined on String")
    (fix (kind replace) (replacement "(. String concat)"))))

(case
  "subtraction of two strings is rejected"
  (doc
    "`(- \"ab\" \"cd\")` — a non-`+` arithmetic operator on two Strings has no concatenation reading
           at all and is rejected (CDZ0201), the same 'arithmetic is not defined on String'. Pins that the
           rejection covers the whole arithmetic family on text, not only `+` (which merely additionally
           offers a concat fix) — so `-` names the type but carries NO (mis)concat fix.")
  (input (- "ab" "cd"))
  (error CDZ0201 (message "arithmetic is not defined on String") (no-fix)))

(case
  "addition of two bytes is rejected — offers the Bytes.concat rewrite"
  (doc
    "The Bytes companion of the String `+` case: `(+ a b)` on two `Bytes` is CDZ0201 and — Bytes
           having a total concatenation — carries the `(. Bytes concat)` rewrite. From rcdzc
           arithmetic_on_two_same_typed_non_numeric_operands_names_the_real_type_and_plus_offers_concat.")
  (input (do (def (f (: a Bytes) (: b Bytes)) (+ a b)) (export f)))
  (error CDZ0201 (fix (kind replace) (replacement "(. Bytes concat)"))))

(case
  "addition of two lists is rejected — a list is not a number"
  (doc
    "`(+ (list 1 2) (list 3 4))` adds two Lists of the same type; arithmetic is not defined on a
           List (CDZ0201). Both operands share the type `(List Int64)`, so this is the same-type path, not
           a cross-kind mismatch. Pins that a compound collection is not silently a number — the author
           who meant concatenation is offered the `List.concat` rewrite (`(. List concat)`), not an implicit `+`.")
  (input (+ #list(1 2) #list(3 4)))
  (error
    CDZ0201
    (message "arithmetic is not defined on (List Int64)")
    (fix (kind replace) (replacement "(. List concat)"))))

(case
  "addition of two records is rejected — a record is not a number"
  (doc
    "`(+ r q)` on two same-typed Records is rejected (CDZ0201, 'arithmetic is not defined on
           (Record (: a Int64))'). A Record has no concatenation, so — unlike String/List — no fix is
           offered, only the honest message. Pins that the numeric-operand requirement rejects a compound
           record, the companion of the list case with no concat rewrite. Runtime operands (parameters),
           so the fault is the operator's, not a fold.")
  (input
    (do
      (def (f (: r (Record (: a Int64))) (: q (Record (: a Int64)))) (+ r q))
      (def (main) (f #record((= a 1)) #record((= a 2))))
      (export main)))
  (error CDZ0201 (message "arithmetic is not defined on") (no-fix)))

(case
  "addition of two sets is rejected — a set is not a number"
  (doc
    "`(+ r q)` on two same-typed Sets is rejected (CDZ0201, 'arithmetic is not defined on (Set
           Int64)'). A set's combination is `Set.union`, not `+`; the numeric operator does not accept it.
           Pins the compound-collection rejection for Set (the sibling of the list and record cases), so
           `+` is never quietly overloaded to a set operation.")
  (input
    (do
      (def (f (: r (Set Int64)) (: q (Set Int64))) (+ r q))
      (def (main) (f #set(1) #set(2)))
      (export main)))
  (error CDZ0201))

(case
  "addition of two symbols is rejected — a symbol is not a number"
  (doc
    "`(+ (Symbol.of \"a\") (Symbol.of \"b\"))` adds two Symbols; a Symbol is an interned name, not a
           number, so arithmetic is not defined on it (CDZ0201). The two operands share the type `Symbol`,
           so this is the same-type path, not a cross-kind mismatch — and a Symbol has no concatenation, so
           the honest message stands with no fix. Pins that the nominal name type is excluded from
           arithmetic exactly as the collections are.")
  (input (+ (Symbol.of "a") (Symbol.of "b")))
  (error CDZ0201))

(case
  "subtraction of two symbols is rejected"
  (doc
    "`(- (Symbol.of \"a\") (Symbol.of \"b\"))` — the non-`+` arithmetic operator on two Symbols is
           rejected the same way (CDZ0201). Pins that the whole arithmetic family, not only `+`, refuses a
           Symbol operand (the Symbol companion of the two-strings pair above).")
  (input (- (Symbol.of "a") (Symbol.of "b")))
  (error CDZ0201))

(case
  "remainder on two strings is rejected — the whole family, including %"
  (doc
    "`(% \"ab\" \"cd\")` — `%` (remainder) is integer arithmetic like `+`/`-`/`*`/`/`, so a String
           operand is rejected (CDZ0201, 'arithmetic is not defined on String') exactly as `(- \"ab\"
           \"cd\")` is. `%` was the LAST arithmetic operator to be brought into the non-numeric-operand
           message family (it had been omitted from the cross-kind operand lists, so a String `%` leaked a
           phantom `Int64`-and-String clash instead); this pins that `%` now names the numeric requirement
           like its siblings. The modulo completion of the whole-family rule the `-`-on-strings case
           introduces.")
  (input (% "ab" "cd"))
  (error CDZ0201))

; NO PHANTOM Int64 CLASH: a same-typed non-numeric arithmetic operand reaches the numeric-requirement
; rejection through the operator's own check — it must NOT leak the generic scheme-unify's phantom "type
; mismatch: Int64 and <T> must be the same type here", an `Int64` the author never wrote. These PARAMETER
; versions (where the scheme-unify path runs, unlike the const-folded literal cases above) pin the negative
; via the `(not …)` message-absence form. (Migrated from rcdzc
; arithmetic_on_a_non_numeric_operand_carries_no_phantom_int64_clash, which pre-dated `(not …)` and so had
; stayed white-box; #6146's message-absence sub-form now lets the corpus assert it.)
(case
  "same-typed String operands to + reject without a phantom Int64 clash"
  (input (do (def (f (: a String) (: b String)) (+ a b)) (export f)))
  (error CDZ0201 (message "arithmetic is not defined on String") (not "must be the same type here")))

(case
  "same-typed String operands to % reject without a phantom Int64 clash"
  (input (do (def (f (: a String) (: b String)) (% a b)) (export f)))
  (error CDZ0201 (message "arithmetic is not defined on String") (not "must be the same type here")))

(case
  "same-typed List operands to + reject without a phantom Int64 clash"
  (input (do (def (f (: a (List Int64)) (: b (List Int64))) (+ a b)) (export f)))
  (error CDZ0201 (message "arithmetic is not defined on") (not "must be the same type here")))

; --- Arithmetic on a MISMATCHED non-numeric pair names BOTH real types -----------------------
; The cases above add two operands of the SAME non-numeric type. When the two operands are DIFFERENT
; non-numeric types — `(+ "ab" (list 1 2))`, a String and a List — the diagnostic names BOTH real types
; ("arithmetic is not defined on String and (List Int64)"), not a phantom `Int64` misattributed to one
; side. This is a THIRD path, distinct both from the same-type case (which names one type) and from the
; cross-KIND mismatch `(+ 1 "two")` above (which has a NUMERIC operand and reports "different types
; across that kind boundary"): here NEITHER operand is numeric, so the fault is squarely the operator's
; numeric requirement over an unrelated pair. The message is order-sensitive (it lists the operands as
; written), so both orders are pinned.
(case
  "addition of a string and a list names both non-numeric types"
  (doc
    "`(+ \"ab\" (list 1 2))` adds a String and a List — two DIFFERENT non-numeric types. The
           compiler rejects it (CDZ0201) and names both real types rather than inventing a phantom
           `Int64` for one side; neither operand is a number, so the operator's numeric requirement is
           what fails. Pins the mismatched-pair diagnostic (distinct from the same-type cases and from the
           `(+ 1 \"two\")` cross-kind case, which has a numeric side).")
  (input (+ "ab" #list(1 2)))
  (error CDZ0201))

(case
  "the mismatched-pair rejection is order-independent"
  (doc
    "`(+ (list 1 2) \"ab\")` — the operands of the case above flipped — is the same rejection
           (CDZ0201). The diagnostic lists the operands as written (List then String), but the fault does
           not depend on which non-numeric type is on which side. Pins that the mismatched-non-numeric-pair
           check is symmetric.")
  (input (+ #list(1 2) "ab"))
  (error CDZ0201))

; A cross-kind arithmetic operand that is a COMPOUND (a tuple) against a numeric operand — `(* (tuple) 0)`
; multiplies the empty tuple by `0`. The tuple is not a number, so the operator's numeric requirement fails
; (CDZ0201, "a (Tuple) and an Int64 are different types … across that kind boundary"), the compound sibling
; of the `(+ 1 "two")` cross-kind reject. This was a `cdz-smith` fuzzer finding
; (`invalid-wasm-type-mismatch-expected-i-found-i-at-offset`, filed @ c9940747e): the generated program
; `(match ((fn (v0) (* (tuple) 0)) 0) (_ 0))` had SLIPPED PAST check and emitted INVALID wasm (a type
; mismatch i64/i32) — a check-vs-compile gap. It is now correctly rejected AT CHECK, no wasm emitted. Pinned
; so a future change to the arithmetic operand check can't silently reintroduce the miscompile.
(case
  "multiplying a compound (tuple) by a number is a cross-kind type error, not invalid wasm"
  (doc
    "`(* (tuple) 0)` multiplies an empty tuple by `0`. The tuple is not numeric, so the arithmetic
           operator's numeric-operand requirement fails: rejected CDZ0201 naming the mismatched kinds (a
           `(Tuple)` and an `Int64`). Regression pin for a cdz-smith invalid-wasm finding (the operation
           used to slip past check and emit an invalid component with a type mismatch); now caught at check,
           the compound-operand sibling of the `(+ 1 \"two\")` cross-kind arithmetic reject.")
  (input (* #tuple() 0))
  (error CDZ0201))

; The COMPARISON/ordering operators share the cross-kind reject: `(< 1 "x")` compares an Int64 and a String
; — different types, CDZ0201 "an Int64 and a String are different types … across that kind boundary". DEDUP
; guard: the ordering carve-out ALSO declines this on the emit path, and dedup_faults must drop that
; consequent decline so the reader sees exactly ONE coded fault (pinned `(count 1)`) AND does NOT leak the
; UNCODED "needs a heap walk (not yet built)" decline (pinned `(no-diagnostic …)`, the program-scoped
; cross-kind absence lever #6765 — `(not …)`/`(count)` can't see an uncoded sibling decline). Holds for the
; tuple↔list ordering and equality cross-kind pairs too. (Migrated from rcdzc
; a_mismatched_type_ordering_stays_a_single_coded_error_not_a_double_with_the_ordering_decline +
; a_mismatched_comparison_drops_the_uncoded_heap_walk_decline.)
(case
  "a cross-kind ordering comparison is a single coded type error, not a double with the ordering decline"
  (input (do (def (main) (if (< 1 "x") 1 0)) (export main)))
  (error CDZ0201 (message "different types") (count 1))
  (no-diagnostic "needs a heap walk"))

(case
  "a cross-kind ordering of a tuple against a list is one coded error with no uncoded heap-walk decline"
  (input (do (def (main) (if (< #tuple(1 2) #list(3)) 1 2)) (export main)))
  (error CDZ0201 (message "different types") (count 1))
  (no-diagnostic "needs a heap walk"))

(case
  "a cross-kind equality of a tuple against a list is one coded error with no uncoded heap-walk decline"
  (input (do (def (main) (if (= #tuple(1 2) #list(3)) 1 2)) (export main)))
  (error CDZ0201 (message "different types") (count 1))
  (no-diagnostic "needs a heap walk"))

(case
  "addition of a string and a byte sequence names both text types"
  (doc
    "`(+ \"ab\" (Bytes.of (list 1)))` adds a String and a Bytes — two different text-ish types,
           both non-numeric, rejected (CDZ0201) naming both. Pins that the mismatched-pair diagnostic
           covers a text/text pair (String vs Bytes) as well as a text/collection pair, so no non-numeric
           combination is silently treated as arithmetic.")
  (input (+ "ab" (Bytes.of #list(1))))
  (error CDZ0201))

; --- Equality type-checks its operands: no cross-type comparison ---------------------------
; `=` offers structural equality over ONE type's values (type-system.md #Structural Values Are
; Comparable Only When Their Shapes Match), so comparing two DIFFERENT types is a type error — the
; same operand-typing rule the ordering operators and arithmetic obey. Two different NUMERIC types
; (Int64 vs Float64) is the silent-promotion the numeric model forbids (numeric-model.md #Numeric
; Types Do Not Silently Promote), rejected CDZ0301 exactly as `(+ 5 2.0)` and `(< 5 2.0)` are; Int64
; vs Bool is a number-vs-boolean error (CDZ0203); Int64 vs String (either order) is a general cross-
; kind error (CDZ0201). These pin the equality companions the ordering-operator section below cites
; by name — `=` is the operator those cases are measured against, so it must itself be pinned.
(case
  "equality of an integer and a float is rejected, not silently promoted"
  (doc
    "`(= 5 2.0)` compares an Int64 and a Float64 — the numeric no-promotion rule (numeric-model.md
           #Numeric Types Do Not Silently Promote) applies to `=` as to `+` and `<`, so the compiler
           rejects it (CDZ0301) rather than promoting 5 to 5.0 and answering. The companion the ordering
           cases below cite as `(= 5 2.0)` → CDZ0301.")
  (input (= 5 2.0))
  (error CDZ0301))

(case
  "equality of an integer and a boolean is a type error"
  (doc
    "`(= 1 true)` compares an Int64 with a Bool — a number and a boolean, unrelated kinds with no
           shared value space, rejected (CDZ0203). The equality companion of `(< 1 true)` → CDZ0203; `=`
           is not a coercion to a common type.")
  (input (= 1 true))
  (error CDZ0203))

; A Bool operand against a DIFFERENT scalar kind — a number or a Char — names the scalar KIND BOUNDARY in
; the message ("a Bool and an Int64 are different types … this operation is not defined between a boolean and
; a number"), rather than the generic scheme-unify "Bool and Int64 must be the same type here" that reads
; like an internal clash. Two SCALARS, so the code stays CDZ0203 (a two-scalar clash, not the compound cases'
; CDZ0201). A Char-vs-number is NOT a dead-end boundary — `Char.to-int` is a total conversion, so it keeps a
; `(Char.to-int …)` WRAP fix (the boundary guard fires only when a Bool, with no numeric/char conversion, is
; one side). (Migrated from rcdzc a_bool_against_a_number_or_char_names_the_scalar_kind_boundary; the int-vs-
; float no-promotion CDZ0301 control is the "ordering an integer against a float" case below.)
(case
  "ordering a boolean against a number names the scalar kind boundary, not an internal clash"
  (input (do (def (g) (< true 5)) (export g)))
  (error
    CDZ0203
    (message "different types")
    (message "between a boolean and a number")
    (not "must be the same type here")))

(case
  "adding a boolean to a number names the scalar kind boundary"
  (input (do (def (g) (+ 1 true)) (export g)))
  (error CDZ0203 (message "different types") (message "between a number and a boolean")))

(case
  "comparing a boolean against a character names the scalar kind boundary"
  (input (do (def (g) (= true #\a)) (export g)))
  (error CDZ0203 (message "different types") (message "between a boolean and a character")))

(case
  "a Char against a number keeps its total-conversion wrap fix, not a dead-end boundary"
  (doc
    "`(+ #\\a 1)` mismatches a Char and an Int64 (CDZ0203), but — unlike a Bool — a Char has a TOTAL
           conversion to a number (`Char.to-int`), so the diagnostic offers a `(Char.to-int …)` WRAP fix
           rather than naming a dead-end kind boundary. Pins that the boundary guard is Bool-specific.")
  (input (do (def (main) (+ #\a 1)) (export main)))
  (error CDZ0203 (fix (kind wrap))))

(case
  "equality of an integer and a string is a type error"
  (doc
    "`(= 1 \"x\")` compares an Int64 with a String — two different types across a kind boundary,
           rejected (CDZ0201). The equality companion of `(< 1 \"x\")` → CDZ0201; `=` never silently
           compares representations across types. The message names BOTH operands with the grammatically
           correct indefinite article — `an Int64` (vowel sound), `a String` — not the ungrammatical
           `a Int64`.")
  (input (= 1 "x"))
  (error CDZ0201 (message "an Int64 and a String are different types")))

(case
  "a cross-kind clash message picks the indefinite article by SOUND, not letter (a UInt8)"
  (doc
    "The article is chosen by the leading SOUND, not the letter: `UInt8` starts with a `yoo` sound, so
           it keeps `a UInt8` (not `an UInt8`), while `Int64` takes `an`. `(< n \"x\")` over a UInt8 param
           vs a String is CDZ0201 naming `a UInt8 and a String`. Pins the sound-based article rule.")
  (input (do (def (g (: n UInt8)) (< n "x")) (export g)))
  (error CDZ0201 (message "a UInt8 and a String")))

(case
  "equality across types is rejected regardless of operand order"
  (doc
    "The order-flipped companion: `(= \"x\" 1)` is the same cross-type comparison (String vs Int64)
           and rejected (CDZ0201), mirroring the flipped ordering case `(> \"x\" 1)`. Pins that the
           operand-type check does not depend on which side carries which type.")
  (input (= "x" 1))
  (error CDZ0201))

; --- Equality of two SAME-KIND compounds of different STRUCTURE names the structural delta -----
; The cases above compare across different KINDS (Int64 vs String). Two compounds of the SAME kind but
; a DIFFERENT structure — two records with different field SETS, two tuples of different ARITY — are also
; different types (type-system.md #Structural Values Are Comparable Only When Their Shapes Match: a
; record's shape is its field set, a tuple's is its arity), so `=` over them is a type error (CDZ0203).
; The diagnostic names the structural DELTA (a missing/extra field, an arity difference), not a raw
; 'these two Records differ' — the same minimal-conflict hint the annotation field-set cases carry, at
; the operator-argument position. These pin the same-kind-different-shape facet of equality's operand
; typing, the companion of the cross-kind cases above.
(case
  "equality of two records with different field sets is a type error"
  (doc
    "`(= (record (x 1)) (record (y 2)))` compares a `(Record (: x Int64))` with a `(Record (: y Int64))`
           — same KIND (both records) but different field SETS, so different types (a record's shape is its
           field set). Rejected CDZ0203, the diagnostic naming the delta (missing `x`; no such field `y`).
           Pins that `=` requires matching field sets, not merely that both operands are records — the
           structural companion of the cross-kind `(= 1 \"x\")` case.")
  (input (= #record((= x 1)) #record((= y 2))))
  (error CDZ0203))

(case
  "equality of two tuples of different arity is a type error"
  (doc
    "`(= (tuple 1 2) (tuple 1 2 3))` compares a 2-tuple with a 3-tuple — same kind, different arity,
           so different types (a tuple's arity is part of its type). Rejected CDZ0203, naming the arity
           delta (expected 2 elements, has 3). Pins that `=` over tuples requires equal arity, the tuple
           companion of the record-field-set case.")
  (input (= #tuple(1 2) #tuple(1 2 3)))
  (error CDZ0203))

(case
  "equality of a record with a subset of another's fields is a type error"
  (doc
    "`(= (record (x 1)) (record (x 1) (y 2)))` — one record's fields are a SUBSET of the other's, but
           a subset is still a DIFFERENT field set (row polymorphism, which would relate them, is a
           separate opt-in — 15-rows-and-open-sums). Rejected CDZ0203 (no such field `y` on the smaller
           record's type). Pins that `=` is not silently widened to ignore the extra field, the subset
           facet of the field-set check.")
  (input (= #record((= x 1)) #record((= x 1) (= y 2))))
  (error CDZ0203))

; --- A COMPOUND / SUM / NOMINAL operand against a SCALAR names the KIND BOUNDARY ----------------
; The cross-kind cases above are scalar-vs-scalar (Int64 vs String). A COMPOUND value (record/tuple/list)
; or a USER SUM / NOMINAL held against a SCALAR or TEXT operand — `(= r 5)`, `(< t 5)`, `(+ c 1)` — is the
; same cross-kind clash: no shared value space, no shared arithmetic/order across the boundary. The message
; names the KIND BOUNDARY (CDZ0201) instead of the generic "type mismatch: (Record …) and Int64 must be the
; same type here" that reads like an internal unify clash, and it must NOT leak a phantom Int64 the author
; never wrote. Two DIFFERENT user sums, by contrast, share the sum "kind" tag, so they keep the generic
; same-kind mismatch (CDZ0203), and a same-sum comparison is valid. (Migrated from rcdzc
; a_compound_operand_against_a_scalar_names_the_kind_boundary.)
(case
  "comparing a record against an integer names the kind boundary"
  (input (do (def (g (: r (Record (: a Int64)))) (= r 5)) (export g)))
  (error CDZ0201 (message "different types") (message "kind boundary")))

(case
  "ordering a tuple against an integer names the kind boundary"
  (input (do (def (g (: t (Tuple Int64 Int64))) (< t 5)) (export g)))
  (error CDZ0201 (message "different types") (message "kind boundary")))

(case
  "comparing a list against an integer names the kind boundary"
  (input (do (def (g (: xs (List Int64))) (= xs 5)) (export g)))
  (error CDZ0201 (message "different types") (message "kind boundary")))

(case
  "adding an integer to a user sum names the kind boundary, not a phantom Int64 clash"
  (input (do (type Color (Red)) (def (g (: c Color)) (+ c 1)) (export g)))
  (error CDZ0201 (message "kind boundary") (not "Int64 and Color")))

(case
  "comparing a user sum against a string names the kind boundary"
  (input (do (type Color (Red)) (def (g (: c Color) (: s String)) (= c s)) (export g)))
  (error CDZ0201 (message "kind boundary")))

(case
  "adding an integer to a nominal newtype names the kind boundary"
  (input (do (type UserId (Mk Int64)) (def (g (: u UserId)) (+ u 1)) (export g)))
  (error CDZ0201 (message "kind boundary")))

; The COMPOUND-vs-COMPOUND cross-kind comparison companion of the scalar/compound-vs-scalar kind-boundary
; cases above: two compounds of DIFFERENT structural kinds — a tuple vs a list — are different types, so a
; comparison (`<`/`=`) across them is CDZ0201 "are different types" (not the generic same-type-here unify
; lead). A well-typed SAME-KIND compound comparison (two same-shape tuples) is NOT a mismatch — with blessed
; compound ordering it COMPILES and runs the lexicographic walk. Two SAME-kind compounds that differ only
; STRUCTURALLY (a tuple arity diff) name the readable arity delta (CDZ0203), not a kind boundary. (Migrated
; from rcdzc a_mismatched_comparison_drops_the_misleading_heap_walk_decline — the compound comparison facets;
; the uncoded heap-walk-decline dedup stays a rust residual.)
(case
  "ordering a tuple against a list is a cross-kind type error"
  (input (do (def (main) (if (< #tuple(1 2) #list(3)) 1 2)) (export main)))
  (error CDZ0201 (message "are different types")))

(case
  "comparing a tuple against a list for equality is a cross-kind type error"
  (input (do (def (main) (if (= #tuple(1 2) #list(3)) 1 2)) (export main)))
  (error CDZ0201 (message "are different types")))

(case
  "a well-typed same-shape tuple ordering compiles and runs the lexicographic walk"
  (input
    (do (def (mk (: n Int64)) #tuple(1 n)) (def (main) (if (< (mk 2) (mk 3)) 1 2)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "ordering two same-kind tuples of different arity names the arity delta, not a kind boundary"
  (input (do (def (g (: a (Tuple Int64)) (: b (Tuple Int64 Int64))) (if (< a b) 1 2)) (export g)))
  (error
    CDZ0203
    (message "expected a tuple with 1 element, but this one has 2")
    (not "kind boundary")))

; A first-class TYPE VALUE (a type name `Color`/`Int64`, a module) used as an arithmetic/comparison operand is
; a type error — there is no `+` on types, and type EQUALITY is the dedicated `Type.eq`, not the bare `=`/`+`.
; A naive generic scheme leaked a phantom "Int64 and Type" clash. Now a Type-VALUE vs a scalar names the KIND
; BOUNDARY (no phantom Int64), and TWO type-value operands in arithmetic name "arithmetic is not defined on
; Type". (Distinct from the cases above, where the operand is a VALUE of a user type; here it is the type
; itself.) (Migrated from rcdzc arithmetic_or_comparison_with_a_type_value_operand_names_it_not_a_phantom_int64
; — the reject-naming facets; the two-type bare-`=` no-relabel control + the spanless-decline dedup stay a
; rust residual.)
(case
  "an arithmetic op with a user-type VALUE operand and a scalar names the kind boundary, not a phantom Int64"
  (doc
    "Also pins the DEDUP (migrated from rcdzc): `(+ Color 1)` used to report the CDZ0201 kind-boundary
           AND a SPANLESS uncoded 'a type value has no runtime form' decline (lowering the type-valued
           operand) — two error: lines for one root cause. dedup_faults drops the spanless decline when the
           kind-boundary CDZ0201 is present, so it is EXACTLY ONE error. Hence (count 1).")
  (input (do (type Color (Red)) (def (main) (+ Color 1)) (export main)))
  (error CDZ0201 (message "kind boundary") (message "Type") (not "Int64 and Type") (count 1)))

(case
  "a bare `=` on two identical TYPE values is refused with a guided CDZ0203 — type equality is Type.eq"
  (doc
    "`(= Int64 Int64)` compares two TYPE values with the bare `=`. A type is ERASED at run time and is
           not data; bare `=` is a runtime structural comparison, so it is the wrong tool — the dedicated
           compile-time `Type.eq` (which folds two type-values to a constant Bool) is. Rejected with a guided
           CDZ0203 that names `Type.eq`, NOT relabeled a 'kind boundary' (both operands share the Type kind,
           so the cross-kind guard must NOT over-reach onto a same-kind Type-vs-Type comparison). Corpus-
           deprecation BUCKET-2: a correct-reject asserting the CODE, replacing the former uncoded decline.")
  (input (do (def (main) (if (= Int64 Int64) 1 0)) (export main)))
  (error CDZ0203 (message "type value") (message "Type.eq") (not "kind boundary")))

(case
  "two type-value operands in arithmetic name that arithmetic is not defined on Type"
  (input (do (type Color (Red)) (def (main) (+ Color Color)) (export main)))
  (error CDZ0201 (message "arithmetic is not defined on Type")))

(case
  "a prelude type name as an arithmetic operand names the kind boundary, not a phantom Int64"
  (input (do (def (main) (+ Int64 1)) (export main)))
  (error CDZ0201 (message "kind boundary") (not "Int64 and Type")))

; --- DEF-shadowing a prelude TYPE name is a plain rebind; the shadowed name as a payload still faults ---
;    (migrated from rcdzc shadowing_a_prelude_payload_type_name_is_a_plain_rebind_not_a_phantom_variant_fault)
; Defining a value named after a prelude type — `(def (Int64) 1)` — is a plain rebind, not a fault: it
; shadows the prelude type name in value/binding position and compiles cleanly. The variant-payload
; validation must NOT re-check the PRELUDE's own sum payloads against the user's now-shadowed namespace
; (a prelude payload typed `Int64` is not re-validated to find the name bound to a nullary function — that
; produced a spurious "variant payload requires a type" at the prelude payload node, which has no source
; span). The check is gated on user nodes, so it is NOT weakened: a USER variant that names the
; value-shadowed `Int64` as its payload DOES still fault CDZ0203 (a non-type payload), at the user's node.
(case
  "a def named after a prelude type name is a plain rebind and compiles"
  (input (do (def (Int64) 1) (def (main) (Int64)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a def shadowing the prelude String type name is likewise a plain rebind"
  (input (do (def (String) 1) (def (main) (String)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a USER variant naming a value-shadowed prelude type name as its payload still faults CDZ0203"
  (doc
    "The check is not weakened by gating on user nodes: with `Int64` shadowed by `(def (Int64) 1)`,
           a user variant `(A Int64)` names a VALUE (the def) as its payload type — a non-type payload —
           so it still faults CDZ0203, landing at the user's own payload node (a real span), not the
           prelude's spanless node.")
  (input (do (def (Int64) 1) (type C (A Int64)) (def (main) 0) (export main)))
  (error CDZ0203))

(case
  "comparing a user sum against a record (different compound kinds) names the kind boundary"
  (input
    (do (type Color (Red)) (def (g (: c Color) (: r (Record (: x Int64)))) (= c r)) (export g)))
  (error CDZ0201 (message "kind boundary")))

(case
  "two DIFFERENT user sums keep the generic same-kind mismatch, not the kind boundary"
  (doc
    "`Color` vs `Shape` share the sum KIND tag, so the cross-kind guard does NOT fire — they keep the
           generic same-kind type mismatch (CDZ0203), distinct from the compound/sum-vs-scalar kind-boundary
           CDZ0201 above. Pins that `different_compound_kinds` fires on a KIND difference, not merely on two
           distinct user types.")
  (input
    (do (type Color (Red)) (type Shape (Sq)) (def (g (: c Color) (: s Shape)) (= c s)) (export g)))
  (error CDZ0203))

(case
  "a same-sum comparison is valid (no false kind-boundary reject)"
  (doc
    "The positive control: `(= Red Blue)` compares two values of the SAME sum `Color` — a well-typed
           comparison that runs and yields false, so the kind-boundary/mismatch checks never over-reject a
           legitimate same-type comparison.")
  (input (do (type Color (Red) (Blue)) (def (main) (= Red Blue)) (export main)))
  (output (: false Bool)))

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
(case
  "ordering an integer against a float is rejected, not silently promoted"
  (doc
    "`(< 5 2.0)` compares an Int64 and a Float64 — the numeric no-promotion rule the
           arithmetic operators obey applies to the ordering operators too, so the compiler rejects
           it (CDZ0301) rather than promoting 5 to 5.0 and answering. The passing companions are
           `(+ 5 2.0)` → CDZ0301 and `(= 5 2.0)` → CDZ0301; `<` must be held to the same rule.")
  (input (< 5 2.0))
  (error CDZ0301))

(case
  "greater-than of an integer and a float is rejected"
  (doc
    "The `>` companion: `(> 5 2.0)` mixes Int64 and Float64, rejected (CDZ0301) like `<`.
           Pins that the no-promotion check covers `>`, not only `<`.")
  (input (> 5 2.0))
  (error CDZ0301))

(case
  "less-than-or-equal of an integer and a float is rejected"
  (doc
    "The `<=` companion: `(<= 5 2.0)` mixes two numeric types, rejected (CDZ0301). Pins the
           check for the inclusive ordering operator.")
  (input (<= 5 2.0))
  (error CDZ0301))

(case
  "greater-than-or-equal of an integer and a float is rejected"
  (doc
    "The `>=` companion: `(>= 5 2.0)` mixes two numeric types, rejected (CDZ0301). Completes
           the four ordering operators against the no-promotion rule.")
  (input (>= 5 2.0))
  (error CDZ0301))

; The SAME-DOMAIN width/sign axis of the no-promotion rule for ordering: the cases above mix numeric
; DOMAINS (Int vs Float). But two INTEGERS of different WIDTH — or the same width but different SIGN —
; are also distinct types the no-promotion rule forbids comparing without an explicit conversion, exactly
; as `(+ (: 5 Int8) (: 10 Int16))` is CDZ0301 (numeric-model.md #Numeric Types Do Not Silently Promote).
; A comparison relates two operands of ONE integer type (∀a. Int a → Int a → Bool); a mixed width or sign
; is CDZ0301, not a silent widen/reinterpret. These pin the ordering operators obey no-promotion on the
; WIDTH and SIGN axes too, not only across domains — the seed-side companion of the compiler-ml bin-type
; relational-arm fix (a mixed-width/sign `<` must unify its operands or reject, never run to a coerced bool).
(case
  "ordering two integers of different width is rejected, not silently widened"
  (doc
    "`(< (: 5 Int8) (: 10 Int16))` compares an Int8 and an Int16 — same domain, different WIDTH.
           The no-promotion rule the arithmetic operators obey (`(+ (: 5 Int8) (: 10 Int16))` → CDZ0301)
           applies to ordering too, so the compiler rejects it (CDZ0301) rather than widening the Int8 to
           Int16 and answering. A comparison relates two operands of one integer type; a width mismatch is
           not a licence to widen.")
  (input (< (: 5 Int8) (: 10 Int16)))
  (error CDZ0301))

(case
  "ordering two integers of different sign is rejected, not silently reinterpreted"
  (doc
    "The SIGN-axis companion: `(< (: 5 Int8) (: 10 UInt8))` compares an Int8 and a UInt8 — same
           width, different SIGN — rejected (CDZ0301), not reinterpreted across signedness. Pins that
           the ordering no-promotion rule covers the sign axis as well as the width axis; a signed and
           an unsigned integer are distinct types with no shared order without an explicit conversion.")
  (input (< (: 5 Int8) (: 10 UInt8)))
  (error CDZ0301))

(case
  "ordering an integer against a boolean is a type error"
  (doc
    "`(< 1 true)` compares an Int64 with a Bool — unrelated kinds with no shared order, a
           general type error the compiler rejects (CDZ0203), exactly as `(= 1 true)` is. An
           ordering operator is not a coercion to a common type; a Bool has no position in Int64's
           order.")
  (input (< 1 true))
  (error CDZ0203))

(case
  "ordering an integer against a string is a type error"
  (doc
    "`(< 1 \"x\")` compares an Int64 with a String — two different types, rejected (CDZ0201)
           like the equality companion `(= 1 \"x\")`. Pins that the ordering operators reject a
           cross-kind comparison rather than declining silently or comparing representations. Also pins the
           cross-message CONTRAST (migrated from rcdzc an_ast_operand_in_arithmetic_names_...): a non-Ast
           cross-type clash keeps the GENERIC 'different types' message, NOT the Ast-specific compile-time-
           metadata message that an Ast operand draws (corpus 12-metaprogramming) — the metadata wording is
           reserved for genuine Ast misuse and must not leak onto an ordinary cross-type reject.")
  (input (< 1 "x"))
  (error
    CDZ0201
    (message "an Int64 and a String are different types")
    (not "compile-time metadata")))

(case
  "ordering a string against an integer is a type error regardless of operand order"
  (doc
    "The order-flipped companion: `(> \"x\" 1)` is the same cross-type comparison (String vs
           Int64) and rejected (CDZ0201). Pins that the operand-type check does not depend on which
           side carries which type.")
  (input (> "x" 1))
  (error CDZ0201))

(case
  "Type is a first-class value"
  (doc
    "Witnesses core-semantics.md #Types Are First-Class Values (1st sentence): a Type can be
           bound to a name, passed as an argument, returned from a function. A Type is an ordinary
           first-class value whose type is the type of types (type-system.md #Types Are First-Class
           Values Whose Type Is The Type Of Types). Here `Int64` is bound to `t` and RETURNED — the
           value the program produces is that type-value, which crosses the boundary as `(: Int64 Type)`
           (the type of a type-value is `Type`). A type-value is fully compile-time-known, so its
           boundary form is baked from the reduced type — it flows OUT of a nullary export directly,
           never from runtime data (a parameterized or not-fully-determined type has no boundary form
           and is rejected).")
  (input (let ((t Int64)) t))
  (output (: Int64 Type)))

(case
  "a type-value flows through nested let bindings and crosses as its own type-of-types"
  (doc
    "The reduction companion of the first-class-Type case above: a type-value flows through TWO
           nested `let` bindings — `(let ((t String)) (let ((u t)) u))` binds `String` to `t`, rebinds `t`
           to `u`, and returns `u`. The compile-time type reducer follows the Let/Ref chain to the ground
           type `String`, so the program crosses the boundary as `(: String Type)` — a DIFFERENT type name
           than the Int64 case, confirming the baked boundary form is the reduced type's own name (not
           hard-wired to one type), and that the reduction descends through more than one binding.")
  (input (let ((t String)) (let ((u t)) u)))
  (output (: String Type)))

(case
  "a consistent annotation type-checks against the inferred type"
  (doc
    "Witnesses type-system.md #Annotations Constrain, Never Contradict and #A Well-Typed Program
           Does Not Go Wrong: `(: (+ 1 2) Int64)` type-checks because inference determines the
           expression's type is Int64 and the annotation unifies with it, so the program compiles and
           evaluates to 3. The passing companion to the CDZ0203 rejections above.")
  (input (: (+ 1 2) Int64))
  (output (: 3 Int64)))

(case
  "an annotation on an arithmetic operand is transparent"
  (doc
    "The annotation position variant of the case above: `(+ (: 2 Int64) 3)` annotates an OPERAND
           rather than the whole expression. The annotation agrees with the operand's type and erases, so
           the arithmetic folds normally to 5 (type-system.md #Annotations Constrain, Never Contradict — an
           agreeing annotation changes nothing, wherever it sits).")
  (input (+ (: 2 Int64) 3))
  (output (: 5 Int64)))

; --- The compiler never crashes: a malformed core form is rejected, not a panic ----------
; A core special form applied with the wrong number of operands (`(if true)`, `(= 5)`, a `let` binding
; with no value, an empty `(quote)`, a bare tuple accessor) is not a program the compiler can compile —
; but it is still INPUT the compiler is handed, and the compiler MUST NOT crash on it
; (self-hosting-and-bootstrap.md §"An Unsupported Construct Is Declined, Not Miscompiled" — the compiler
; declines or rejects; it never panics; the self-hosting fixpoint requires the compiler to be a total
; function over its input bytes). An ill-formed program's outcome is a rejection with the general
; ill-formed-program code CDZ0201 — never a crash, and never a value.
(case
  "a conditional with a missing branch is rejected, not a crash"
  (doc
    "`(if <cond> <then>)` with no else branch is ill-formed: `if` requires condition, then, and
           else. The compiler rejects it (CDZ0201), never panicking while reaching for the absent third
           operand.")
  (input (if true 1))
  (error CDZ0201))

(case
  "a bare conditional keyword is rejected, not a crash"
  (doc
    "`(if)` with no operands at all is ill-formed. The compiler rejects it, never indexing past
           the end of the operand list.")
  (input (if))
  (error CDZ0201))

(case
  "equality curries — applied to one operand it is a first-class predicate"
  (doc
    "Operators CURRY (operator ruling: \"operators should curry\"). `(= 5)` supplies the first of
           `=`'s two operands and yields the first-class function `\\b. 5 = b`. Bound then applied — `(let
           ((eq5 (= 5))) (eq5 5))` — it completes the equality: 5 = 5 → true → 1. The `let` binding forces
           `(= 5)` to be a VALUE (the resolver would otherwise flatten `((= 5) 5)` to the plain `(= 5 5)`).
           (A ZERO-operand `(=)` is still malformed — the bare-keyword case below.)")
  (input (do (def (main) (let ((eq5 (= 5))) (if (eq5 5) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "a bare equality keyword is rejected, not a crash"
  (doc "`(=)` with no operands is ill-formed. Rejected (CDZ0201), never a crash.")
  (input (=))
  (error CDZ0201))

(case
  "an arithmetic operator with a single operand curries into a partial-application closure"
  (doc
    "`(+ 5)` supplies the first of `+`'s two operands and CURRIES to `\\b. 5 + b` (operator ruling:
           operators curry) — the arithmetic companion of the `(= 5)` case above. Bound then applied —
           `(let ((add5 (+ 5))) (add5 3))` — completes the addition: 5 + 3 → 8. (A ZERO-operand `(+)` is
           still malformed — the bare-keyword case below.)")
  (input (do (def (main) (let ((add5 (+ 5))) (add5 3))) (export main)))
  (output (: 8 Int64)))

(case
  "a bare arithmetic keyword is rejected, not a crash"
  (doc
    "`(+)` with no operands is ill-formed. Rejected (CDZ0201), never a crash — the `+` companion
           of the bare `(=)` case.")
  (input (+))
  (error CDZ0201))

(case
  "an ordering operator with a single operand curries into a partial-application closure"
  (doc
    "`(< 5)` supplies the first of `<`'s two operands and CURRIES to `\\b. 5 < b` (operators curry),
           covering the ordering operators too, not only `=`/`+`. Bound then applied — `(let ((lt5 (< 5)))
           (lt5 3))` — completes the comparison: 5 < 3 → false → 0. (A ZERO-operand `(<)` is still
           malformed.)")
  (input (do (def (main) (let ((lt5 (< 5))) (if (lt5 3) 1 0))) (export main)))
  (output (: 0 Int64)))

(case
  "a conditional with too many operands is rejected, not a crash"
  (doc
    "`(if true 1 2 3)` supplies a fourth operand to `if`, which takes exactly three (condition,
           then, else). The compiler rejects it (CDZ0201), never silently ignoring the extra operand nor
           crashing — the over-application companion of the missing-branch `(if true 1)` case above.")
  (input (if true 1 2 3))
  (error CDZ0201))

(case
  "a member access with no field operand is rejected, not a crash"
  (doc
    "`(. 5)` supplies the record operand but no key: member access `(. <operand> <key>)` takes
           exactly two operands — a NAME key projects a record field, an INTEGER key projects a
           positional tuple element (`(. t 0)`), so this one form serves both. With no key it is
           ill-formed; the compiler rejects it (CDZ0201), never panicking reaching for the absent key
           node.")
  (input (. 5))
  (error CDZ0201))

(case
  "a bare binding form with no bindings and no body is rejected, not a crash"
  (doc
    "`(let)` supplies neither a binding list nor a body: `let` is `(let (<binding>…) <body>)`. The
           compiler rejects it (CDZ0201), never panicking reaching for the absent binding list or body
           node — the binding-form companion of the bare-keyword `(=)`/`(if)` cases.")
  (input (let))
  (error CDZ0201))

(case
  "a binding form with bindings but no body is rejected, not a crash"
  (doc
    "`(let ((x 1)))` supplies a well-formed binding list but no body form to evaluate in its
           scope. Ill-formed — `let` requires a body — so the compiler rejects it (CDZ0201), never
           panicking reaching for the absent body node. Distinct from `(let ((x)) x)` above (a binding
           with no VALUE); this is a `let` with no BODY.")
  (input (let ((x 1))))
  (error CDZ0201))

(case
  "a let binding with no value expression is rejected, not a crash"
  (doc
    "A binding `(x)` names `x` but supplies no value expression: `(let ((x)) x)` is ill-formed.
           The compiler rejects it (CDZ0201), never panicking reaching for the absent value node.")
  (input (let ((x)) x))
  (error CDZ0201))

(case
  "an empty quote is rejected, not a crash"
  (doc
    "`(quote)` with nothing to quote is ill-formed: quote requires exactly one operand — the form
           it denotes. The compiler rejects it (CDZ0201), never panicking reaching for the absent
           quoted node.")
  (input (quote))
  (error CDZ0201))

; The too-MANY-operand face: `quote`/`quasiquote` take EXACTLY one operand (the form they denote), so a
; surplus operand is CDZ0201 with a delete-the-surplus fix (the mechanical repair — delete the extra form),
; the same surplus-arg delete an over-applied operator gets. (The empty `(quote)` above carries no fix —
; nothing to delete.) (migrated from rcdzc a_quote_with_too_many_operands_offers_a_delete_the_surplus_fix.)
(case
  "a quote with too many operands is rejected with a delete-the-surplus fix"
  (input (do (def (main) (quote 1 2)) (export main)))
  (error CDZ0201 (message "takes exactly one operand") (fix (kind delete))))

(case
  "a quasiquote with too many operands is rejected with a delete-the-surplus fix"
  (input (do (def (main) (quasiquote 1 2)) (export main)))
  (error CDZ0201 (message "takes exactly one operand") (fix (kind delete))))

(case
  "a record field with no value expression is rejected, not a crash"
  (doc
    "A record entry `(= a)` names the field `a` but supplies no value: `#record((= a))` is ill-formed
           — a record entry is a `(= name value)` pair. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Same never-crash class as the `(let ((x)) x)`
           binding-with-no-value case above, for a record entry. (The prior `#record((= = a))` input was a
           nativization artifact — a well-formed `=`-keyed field whose value `a` is merely unbound, which
           tests CDZ0101 unbound, not the no-value shape this case is for; `#record((= a))` is the genuine
           missing-value field.)")
  (input #record((= a)))
  (error CDZ0201))

(case
  "a map entry with no value expression is rejected, not a crash"
  (doc
    "The map companion: `(map (a))` names the key `a` but supplies no value — a map entry is a
           `(key value)` pair, so this is ill-formed. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Pins that both the `record` and `map`
           construction paths bounds-check an entry before indexing its value.")
  (input (map ("a")))
  (error CDZ0201))

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
(case
  "a diverging expression unifies with an integer position"
  (doc
    "Witnesses type-system.md #Never Is The Empty Sum (3rd sentence: the type of a diverging
           expression is Never, which unifies with any expected type). In `(if b 1 (trap \"unreachable\"))`
           the then-branch is Int64 and the else-branch diverges (type Never); the two branches unify to
           Int64 because Never unifies with any type. With b=true the program yields 1; the else-branch
           never runs but must TYPE-CHECK. A generation without the Never-unifies rule would reject the
           branch-type mismatch. Pins that a divergent branch does not spoil a well-typed conditional.")
  (input (do (def (f b) (if b 1 (trap "unreachable"))) (def (main) (f true)) (export main)))
  (output (: 1 Int64)))

(case
  "a function whose body always diverges has result type Never"
  (doc
    "Witnesses type-system.md #Never Is The Empty Sum: `bomb` always traps, so its body has type
           Never; calling it at a use site that expects an Int64 type-checks because Never unifies with
           any expected type. The call diverges at run time (the trap), so the program's terminal
           condition is the trap, not a value. Pins that a Never-returning function is callable in a
           typed position — the honest type for a function that never returns normally.")
  (input (do (def (bomb) (trap "unreachable")) (def (main) (+ 1 (bomb))) (export main)))
  (trap "unreachable"))

(case
  "arithmetic on a let binding whose initializer diverges traps"
  (doc
    "`(let ((x (trap …))) (+ x 1))`: the binding's initializer is Never, so it TRAPS before the add
           ever runs — the arithmetic is dead. The value is the trap, not a number. Witnesses that a
           diverging sub-value aborts its enclosing computation (type-system.md #Never Is The Empty Sum:
           Never unifies with any position, and a diverging expression halts). The Rust backend must NOT
           emit the dead `(+ x 1)` as a method call on the `!`-typed binding (`x.checked_add(1)` — E0599, a
           method call on Never); it emits only the diverging trap, matching the wasm `unreachable`.")
  (input (do (def (main) (let ((x (trap "boom"))) (+ x 1))) (export main)))
  (trap "unreachable"))

(case
  "arithmetic reached via an inlined diverging call argument traps"
  (doc
    "`(f (trap …))` with `f x = (+ x 1)`: the argument diverges (Never), so evaluating it traps before
           `f`'s body runs. When `f` inlines, the Never argument substitutes for `x`, so the body's `(+ x 1)`
           becomes arithmetic on Never — the same shape as the let-binding case, reached by a call-arg
           substitution. Traps unreachable; the Rust backend emits only the trap, not a method call on `!`.")
  (input (do (def (f (: x Int64)) (+ x 1)) (def (main) (f (trap "boom"))) (export main)))
  (trap "unreachable"))

(case
  "a match whose all arms diverge is Never and always traps"
  (doc
    "Every arm of `(match n (0 (trap …)) (_ (trap …)))` diverges, so the match is Never — it produces
           no value on any path. Type-checks (Never), and running it traps. The backends emit the match with
           no result value (an empty/never block, not a decline for 'no machine representation'); with n=0
           the first arm traps unreachable.")
  (input (do (def (main (: n Int64)) (match n (0 (trap "zero")) (_ (trap "other")))) (export main)))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "an if whose both branches diverge is Never and always traps"
  (doc
    "Both arms of `(if b (trap …) (trap …))` diverge, so the `if` is Never — no value on any path. It
           type-checks (Never unifies with any position), and running EITHER branch traps. The backends emit
           the `if` with no result value (an empty/never block + a stack-polymorphic trailing unreachable),
           NOT a decline for 'result type has no machine representation'. With b=true the then-branch traps.")
  (input (do (def (main (: b Bool)) (if b (trap "then") (trap "else"))) (export main)))
  (call main (: true Bool))
  (trap "unreachable"))

(case
  "a both-diverge if nested in a value position yields the outer value"
  (doc
    "`(if b 1 (if c (trap …) (trap …)))`: the INNER if is Never (both arms trap); as the outer else-arm
           it unifies into the outer if's Int64 (Never unifies with any type), so the outer if is Int64. With
           b=true the concrete `1` is selected (the diverging inner if is never entered) → 1. Pins that a
           both-diverge if nested as a value subexpression does not spoil the enclosing typed conditional —
           the Never inner supplies a stack-polymorphic value the outer arm's type expects.")
  (input
    (do (def (main (: b Bool) (: c Bool)) (if b 1 (if c (trap "x") (trap "y")))) (export main)))
  (call main (: true Bool) (: false Bool))
  (output (: 1 Int64)))

(case
  "a both-diverge MATCH nested in a value position yields the outer value or traps"
  (doc
    "The match twin of the nested both-diverge `if` above: `(if (> n 0) 1 (match n (0 (trap …)) (_
           (trap …))))` — the all-diverge match is Never, and as the outer `if`'s Int64 else-arm its
           empty-block + trailing unreachable supplies the slot the outer arm's type expects (Never unifies
           with any type). n>0 selects the concrete 1 (the diverging match is never entered) -> 1; n<=0
           forces the all-diverge match, which traps (a raw `unreachable`, so the trap message is
           'unreachable', not the source arm string). Pins that an all-diverge match nested as a value
           subexpression does not spoil the enclosing typed conditional.")
  (input
    (do
      (def (main (: n Int64)) (if (> n 0) 1 (match n (0 (trap "z")) (_ (trap "o")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "a trap message that is not a String is rejected"
  (doc
    "`trap` aborts with a TEXT message, so its argument must be a String; `(trap 42)` supplies an
           Int64, which is rejected (CDZ0203). The diagnostic names the requirement — a trap message is
           text — and shows the shape `(trap \"reason\")`, rather than leaking a bare 'Int64 and String'
           clash from grounding the operator's argument type. The valid-message companion of the
           diverging-`(trap \"unreachable\")` cases above: those pin that a WELL-FORMED trap has type
           Never; this pins that a MALFORMED trap message is refused up front, not run.")
  (input (do (def (main) (trap 42)) (export main)))
  (error
    CDZ0203
    (message "`trap`'s message must be a String")
    (message "a value of type Int64 was given")
    (not "must be the same type here")))

; `trap : ∀a. String → a` — its polymorphic RESULT `a` made a naive scheme-unify ground the operand to
; String and leak the OPAQUE "String and <T> must be the same type here" clash. The reject now NAMES the real
; fault (the message must be a String, showing `(trap "reason")`) across operand types, and does NOT leak the
; internal same-type-here wording. A wrong-ARITY trap keeps its own over-application message (arity 1), not
; the message-type reject; a well-formed String-message trap in a polymorphic position compiles + runs clean.
; (Migrated from rcdzc a_non_string_trap_message_names_the_string_requirement_not_a_phantom_clash.)
(case
  "a non-String trap message names the String requirement (Bool operand)"
  (input (do (def (f) (trap true)) (export f)))
  (error
    CDZ0203
    (message "`trap`'s message must be a String")
    (message "a value of type Bool was given")
    (not "must be the same type here")))

(case
  "a non-String trap message names the String requirement (tuple operand)"
  (input (do (def (f) (trap #tuple(1 2))) (export f)))
  (error
    CDZ0203
    (message "`trap`'s message must be a String")
    (message "a value of type (Tuple Int64 Int64) was given")
    (not "must be the same type here")))

(case
  "a wrong-arity trap keeps the over-application message, not the message-type reject"
  (input (do (def (f) (trap "a" "b")) (export f)))
  (error CDZ0203 (message "function of arity 1") (not "message must be a String")))

(case
  "a well-formed String-message trap in a polymorphic position compiles and runs clean"
  (input (do (def (f (: x Bool)) (if x 1 (trap "no"))) (def (main) (f true)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a match on an uninhabited scrutinee is exhaustive with zero arms"
  (doc
    "Witnesses type-system.md #Never Is The Empty Sum (4th sentence: a match on a Never-typed
           scrutinee is exhaustive with zero arms). `never-returns` has result type Never, so matching
           its result needs NO arms to cover every variant — there are none — and the zero-arm match is
           the degenerate BASE CASE of the exhaustiveness rule (core-semantics.md #Matching Is Exhaustive
           Or Rejected), NOT a CDZ0210 non-exhaustive rejection. The scrutinee diverges before the match,
           so the program traps. Pins that the empty sum makes a zero-arm match vacuously exhaustive
           rather than an error.")
  (input
    (do
      (def (never-returns) (trap "unreachable"))
      (def (main) (match (never-returns)))
      (export main)))
  (trap "unreachable"))

(case
  "a zero-arm match on an inhabited scrutinee is non-exhaustive"
  (doc
    "The inhabited companion of the uninhabited zero-arm case: `(match n)` over an Int64 scrutinee
           has values (every integer) that no arm covers, so an empty arm list is genuinely NON-EXHAUSTIVE
           and rejected CDZ0210 — NOT the malformed no-arms rejection it once was, and NOT the vacuously
           exhaustive zero-arm match a Never-typed scrutinee admits. Pins that the zero-arm base case is
           exhaustive only when the scrutinee is uninhabited. A SCALAR scrutinee (no named variants) keeps
           the generic `zero-arm match is exhaustive only` message and carries an add-a-wildcard-arm INSERT
           fix — a single `(_ (trap \"TODO\"))` arm covers any scalar. (Enhanced from rcdzc
           a_zero_arm_match_on_an_inhabited_sum_offers_the_full_add_arms_fix scalar tail.)")
  (input (do (def (f (: n Int64)) (match n)) (export f)))
  (error
    CDZ0210
    (message "zero-arm match is exhaustive only")
    (fix (kind insert-into) (replacement "(_ (trap \"TODO\"))"))))

; A zero-arm scalar match in a CALLED def must report the CDZ0210 EXACTLY ONCE, not twice: the reduced
; (inlined) body re-runs the exhaustiveness check at a SYNTHESIZED match node, so the reject once leaked a
; second copy re-anchored to the call site. `dedup_faults` drops the copy whose add-a-wildcard fix targets a
; non-user (synthesized) node, keeping only the def-body copy that edits the real match. Two DISTINCT called
; zero-arm matches still report BOTH (each fix edits its own user node — neither is the dropped non-user
; copy). (Migrated from rcdzc a_called_defs_zero_arm_scalar_match_reports_cdz0210_once_not_at_the_call_site_too.)
(case
  "a called def's zero-arm scalar match reports the non-exhaustive reject exactly once, not at the call site too"
  (input (do (def (f (: x Int64)) (match x)) (def (main) (f 1)) (export main)))
  (error CDZ0210 (message "zero-arm match is exhaustive") (count 1)))

(case
  "two distinct called zero-arm scalar matches each report (no false-merge to one)"
  (input
    (do
      (def (f (: x Int64)) (match x))
      (def (g (: y Int64)) (match y))
      (def (main) (+ (f 1) (g 2)))
      (export main)))
  (error CDZ0210 (message "zero-arm match is exhaustive") (count 2)))

; TYPE REFLECTION — `(Type.of e)` reduces at compile time to the type-VALUE of `e`'s inferred type,
; realizing type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary (a type is
; a first-class value the compiler can compute). It is a COMPILE-TIME operation: a `Type` value is
; erased before the boundary (types-are-erased), so `Type.of` is used in TYPE positions — an annotation
; `(: x (Type.of y))` gives `x` the same type as `y` — never returned at runtime. Attaching a unit or a
; reflected type never changes the value's byte form, so an agreeing `(: x (Type.of y))` is transparent.
(case
  "Type.of reflects a value's type for use as an annotation"
  (doc
    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary: a
           type is a first-class value the compiler computes. `(Type.of y)` reduces to `y`'s type-value
           (here Int64), so `(: 100 (Type.of y))` annotates 100 with that reflected type — an agreeing
           annotation, transparent, evaluating to 100. The reflected type is consumed in type position
           and erased; nothing about it survives to runtime.")
  (input (let ((y 42)) (: 100 (Type.of y))))
  (output (: 100 Int64)))

(case
  "an annotation by a reflected type that contradicts the value is rejected"
  (doc
    "Witnesses type-system.md #Annotations Constrain, Never Contradict, over a REFLECTED type:
           `(Type.of y)` is Int64 (y is 42), so `(: true (Type.of y))` annotates a Bool value with the
           reflected Int64 — a contradiction rejected CDZ0203, exactly as a written `(: true Int64)` is.
           Reflection does not weaken the check: the computed type constrains the value like any
           annotation.")
  (input (let ((y 42)) (: true (Type.of y))))
  (error CDZ0203))

(case
  "Type.of carries a quantity's unit into a same-type annotation"
  (doc
    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary
           over a unit-indexed type: `(Type.of y)` where `y : (Qty Float64 meter)` reflects the whole
           quantity type — inner numeric AND unit — so `(: (Qty.of 9.0 meter) (Type.of y))` agrees and
           the quantity erases to 9.0. Pins that reflection captures the full type, dimension included,
           for reuse as `make another quantity of the same type as this one`.")
  (input
    (let
      ((y (Qty.of 3.0 (Unit.base #"meter"))))
      (Qty.value (: (Qty.of 9.0 (Unit.base #"meter")) (Type.of y)))))
  (output (: 9.0 Float64)))

(case
  "a reflected quantity type rejects a value of a different dimension"
  (doc
    "The dimensional companion of the reflection annotation: `(Type.of y)` is `(Qty Float64 meter)`
           (y is a length), so annotating a TIME quantity `(: (Qty.of 9.0 second) (Type.of y))` is a
           dimensional mismatch, CDZ0501 — reflection carries the unit into the check exactly as a
           written `(Qty Float64 meter)` annotation would. A reflected type is a real type, checked in
           full.")
  (input
    (let
      ((y (Qty.of 3.0 (Unit.base #"meter"))))
      (Qty.value (: (Qty.of 9.0 (Unit.base #"second")) (Type.of y)))))
  (error CDZ0501))

(case
  "Type.of reflects a runtime parameter's type at compile time"
  (doc
    "Witnesses that reflection reads the STATIC type, not a runtime value: `(Type.of n)` for a
           parameter `n : Int64` reduces to Int64 at compile time regardless of `n`'s runtime value, so
           `(: 100 (Type.of n))` is an agreeing Int64 annotation and `main 7` returns 100. The reflected
           type depends only on `n`'s inferred type, and is erased — `n`'s value is never consulted.")
  (input (do (def (main (: n Int64)) (: 100 (Type.of n))) (export main)))
  (call main (: 7 Int64))
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
(case
  "Type.eq is true for two values of the same type"
  (doc
    "Witnesses type-system.md #Inference And First-Class Types Meet At A Bidirectional Boundary:
           types are first-class values the compiler compares. `(Type.eq (Type.of 5) (Type.of 6))` — both
           Int64 — folds to the constant `true`. The comparison is exact structural type equality decided
           at compile time; the produced `Bool` is an ordinary value.")
  (input (Type.eq (Type.of 5) (Type.of 6)))
  (output (: true Bool)))

(case
  "Type.eq is false for values of different types"
  (doc
    "`(Type.eq (Type.of 5) (Type.of true))` compares Int64 with Bool — distinct types — folding to
           the constant `false`. Pins that type equality is a real, decidable comparison, not always
           true: two differently-typed values are observably unequal at the type level.")
  (input (Type.eq (Type.of 5) (Type.of true)))
  (output (: false Bool)))

(case
  "Type.eq carries an integer type's full width and signedness"
  (doc
    "An integer type is identified by BOTH its width and its signedness, and `Type.eq` compares the
           whole thing. `(Type.of (Int8.of 5))` is `Int8`: equal to the written `Int8` (→ true), distinct
           from `Int64` (different WIDTH → false), and distinct from `UInt8` (same width, different SIGN →
           false). `1 + 0 + 0 = 1`. Pins that a numeric type-value carries the exact `(sign, width)` — the
           numeric-model no-silent-promotion rule at the type-value level, so a program can branch on a
           value's exact integer type.")
  (input
    (do
      (def
        (main)
        (+
          (if (Type.eq (Type.of (Int8.of 5)) Int8) 1 0)
          (+
            (if (Type.eq (Type.of (Int8.of 5)) Int64) 10 0)
            (if (Type.eq (Type.of (Int8.of 5)) (Type.of (UInt8.of 5))) 100 0))))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.eq compares a reflected type against a written type"
  (doc
    "`(Type.eq (Type.of 5) Int64)` compares a reflected type with a WRITTEN one — both are
           type-values, so the operation is symmetric over reflection and syntax — and is `true`. Pins
           that a written type and `Type.of` produce the same kind of value, composably comparable.")
  (input (Type.eq (Type.of 5) Int64))
  (output (: true Bool)))

(case
  "Type.eq distinguishes quantities by their unit"
  (doc
    "A quantity's UNIT is part of its type, so `(Type.eq (Type.of (Qty.of 1.0 meter)) (Type.of
           (Qty.of 1.0 second)))` is `false` — meter and second are different dimensions hence different
           types — while the same unit compares `true` regardless of magnitude. Pins that type equality
           carries the full unit-indexed type (units-of-measure.md #Dimensional Mismatch Is An Error, at
           the type-value level).")
  (input
    (Type.eq
      (Type.of (Qty.of 1.0 (Unit.base #"meter")))
      (Type.of (Qty.of 1.0 (Unit.base #"second")))))
  (output (: false Bool)))

(case
  "Type.eq on a constructed generic sum compares the full instantiated type"
  (doc
    "`Type.of` on a value of a CONSTRUCTED generic sum reflects its full instantiated type, including
           the type ARGUMENT. `(Type.eq (Type.of (Iter.Cons 1 Iter.Nil)) (Type.of (Iter.Cons 2 Iter.Nil)))`
           — both are `Iter Int64` — folds to `true`. Pins that a generic type CONSTRUCTOR's type-value
           carries its arg (a `Ty::Sum{decl, args}`, compared structurally), so reflection over a
           user-generic value sees the same instantiated type two equal-typed values share.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (main)
        (if (Type.eq (Type.of (Iter.Cons 1 (Iter.Nil))) (Type.of (Iter.Cons 2 (Iter.Nil)))) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.eq distinguishes a constructed generic sum by its type argument"
  (doc
    "The type ARGUMENT is part of a generic sum's type, so `(Type.of (Iter.Cons 1 Iter.Nil))` (an
           `Iter Int64`) and `(Type.of (Iter.Cons true Iter.Nil))` (an `Iter Bool`) compare `false` — same
           `decl`, different element arg. The generic-sum analogue of the quantity-unit case: type equality
           carries the full instantiated `Ty::Sum{decl, args}`, not just the decl.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (main)
        (if (Type.eq (Type.of (Iter.Cons 1 (Iter.Nil))) (Type.of (Iter.Cons true (Iter.Nil)))) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "a same-name generic type applied in a variant payload denotes the type"
  (doc
    "A generic sum whose declared NAME coincides with its sole variant's name — `(type Box (Box a))`
           — used APPLIED in another type's variant payload `(type Holder (Holder (Box Int64)))`. In a
           TYPE position the applied `(Box Int64)` can only mean the TYPE `Box Int64`, but the head `Box`
           resolves to the VARIANT constructor (a same-name occurrence prefers the value binding), so a
           payload type-position reader that only recognized the type-constructor form rejected the
           well-formed program CDZ0203 (`a variant payload requires a type, but found a non-type`) — a
           bogus decline. The BARE same-name form (`(type Note (Note Pitch))` with monomorphic `Pitch`)
           already worked; only the APPLIED generic form hit the variant-head gap. Building `(Holder (Box
           7))`, projecting the `Box Int64` payload, and matching its inner `Int64` = 7 pins that the
           applied same-name generic in a payload denotes the type and its value round-trips. The reader
           recovers the owning declaration from the variant head (its params are non-empty ⇒ generic) and
           builds the sum type directly — the same `Ty` the type-constructor head produces.")
  (input
    (do
      (type Box (Box a))
      (type Holder (Holder (Box Int64)))
      (def (main) (match (Holder (Box 7)) ((Holder b) (match b ((Box n) n)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a same-name generic type at TWO instantiations in one payload each denotes its type"
  (doc
    "Extends the single-field same-name-generic pin above: the SAME generic sum `(type Box (Box a))`
           applied at TWO DISTINCT instantiations — `(Box Int64)` and `(Box Bool)` — as SIBLING fields of one
           `(type Pair (Pair (Box Int64) (Box Bool)))` payload. Each applied `(Box T)` sits in a type
           position where the head `Box` resolves to the VARIANT constructor (same-name prefers the value
           binding), so the payload type-position reader must recover the owning declaration from the variant
           head and rebuild the sum type at EACH instantiation independently — `Box Int64` and `Box Bool` are
           different types built from the same decl. Constructing `(Pair (Box 5) (Box b))`, matching both
           `Box` payloads in a nested pattern, and using the `Int64` inner (`x`) selected by the `Bool` inner
           (`b`) pins that the variant-head recovery is per-occurrence — the two instantiations do not collide
           on the shared decl. The `true`-branch selects the stored `Int64` (5); the `false`-branch twin
           (a separate program) yields 99.")
  (input
    (do
      (type Box (Box a))
      (type Pair (Pair (Box Int64) (Box Bool)))
      (def (main) (match (Pair (Box 5) (Box true)) ((Pair (Box x) (Box b)) (if b x 99))))
      (export main)))
  (output (: 5 Int64)))

; The correct-arity applied-generic-in-payload cases above denote the type. The WRONG-arity form in a
; variant payload must reject CDZ0203 AT THE DECLARATION — the payload-position sibling of the annotation
; wrong-arity rejects (a user generic OVER-applied, or a built-in given too many args). This used to be
; SILENTLY ACCEPTED: a user generic reduces to a Ty::Sum dropping the extra arg, so the payload type-reader
; waved `(Box Int64 Bool)` through, and the mis-arity surfaced only LATER as a confusing CDZ0201 at a
; construction site (identical renders because the extra arg was dropped). The check must fire at the decl.
(case
  "a wrong-arity user generic in a variant payload is rejected at the declaration"
  (doc
    "`(type W (Wrap (Box Int64 Bool)))` puts the one-parameter user generic `(type (Box a) (Mk a))`
           OVER-applied (two type args) in a variant PAYLOAD. The payload type-position arity check must fire
           at the DECLARATION — CDZ0203 naming `Box`'s true arity of 1 — exactly as the same over-application
           rejects in an annotation. Was silently accepted (the user generic reduced to a `Ty::Sum` dropping
           the extra `Bool`, so the payload reader's typeval early-return waved it through), surfacing only
           later as a confusing CDZ0201 at a construction site with identical `Box`-vs-`Box` renders. Pins the
           arity check reaches a variant-payload position, not only an annotation — the payload sibling of the
           over-supplied annotation case.")
  (input (do (type (Box a) (Mk a)) (type W (Wrap (Box Int64 Bool))) (def (main) 0) (export main)))
  (error CDZ0203))

(case
  "a wrong-arity built-in generic in a variant payload is rejected at the declaration"
  (doc
    "The built-in companion: `(type W (Wrap (Option Int64 Bool)))` over-applies `Option` (takes 1) in a
           variant payload → CDZ0203 at the declaration, matching the user-generic payload case above and the
           built-in annotation arity reject. Pins that the payload-position arity check is uniform for a
           built-in generic and a user one.")
  (input (do (type W (Wrap (Option Int64 Bool))) (def (main) 0) (export main)))
  (error CDZ0203))

; --- Type.try-as — the compile-time "view a value at the EXPECTED type" reflection op ---------
; DESIGN-variable-arity-functions.md §5: `Type.try-as : ∀a b. a → (Option b)` yields `Some x` iff x's
; type STRUCTURALLY matches the target `b`, else `None`. The target `b` is INFERRED from usage (the
; enclosing `(: … (Option T))` annotation grounds it, mirroring `Value.decode`); a value's type is
; grounded before the match (a bare literal `5` is `Int64`), so the check is exact but not tripped by a
; deferred literal width. Strict — no subtype widening. Folds at compile time (no runtime type tag), so
; the match on its result selects a branch statically. It underpins tuple-rest varargs type-branching.
(case
  "Type.try-as at a value's own type yields Some carrying the value"
  (doc
    "`(Type.try-as 5)` viewed at the expected `(Option Int64)` matches — `5` IS an `Int64` — so it is
           `(Some 5)`, and the match reads the value back as `5`. The deferred literal width grounds to its
           default `Int64` before the type check, so the own-type view succeeds.")
  (input (match (: (Type.try-as 5) (Option Int64)) ((Some n) n) ((None u) -1)))
  (output (: 5 Int64)))

(case
  "Type.try-as at a DIFFERENT type yields None"
  (doc
    "`(Type.try-as 5)` viewed at `(Option String)` does NOT match — an `Int64` is not a `String` — so it
           is `None` and the miss branch is taken. Pins the strict negative: no coercion of a number to a
           string, the view simply fails.")
  (input (match (: (Type.try-as 5) (Option String)) ((Some s) 1) ((None u) 0)))
  (output (: 0 Int64)))

(case
  "Type.try-as branches on which type a value has, folded at compile time"
  (doc
    "The type-branching idiom: try `7` at `Int64` first — it matches, so `(Some 7)` is taken and the
           `String` fallback is never reached, yielding `7 + 100 = 107`. Each arm ascribes the target it
           tests; the whole ladder folds at compile time because both the value's type and each target are
           static. This is the mechanism tuple-rest varargs use to branch on what types were passed.")
  (input
    (match (: (Type.try-as 7) (Option Int64))
      ((Some n) (+ n 100))
      ((None u)
        (match (: (Type.try-as 7) (Option String)) ((Some s) 0) ((None u) -1)))))
  (output (: 107 Int64)))

(case
  "Type.try-as views a RUNTIME value at its declared type"
  (doc
    "`(Type.try-as n)` for a boundary parameter `n : Int64` viewed at `(Option Int64)` matches on the
           declared type — the target is decided statically from the annotation, so it is `(Some n)` for
           every `n`, read back unchanged (7 → 7, -3 → -3). Pins that the view works on a runtime value, not
           only a literal (the type is what is checked, statically; the value flows through).")
  (input
    (do
      (def (main (: n Int64)) (match (: (Type.try-as n) (Option Int64)) ((Some m) m) ((None u) -1)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (output (: -3 Int64)))

(case
  "asserting a value has a type is Type.try-as fed to Option.expect"
  (doc
    "There is no dedicated assert-of-type primitive — the caller composes the existing pieces:
           `Option.expect((Type.try-as x : Option T), msg)` yields the value when its type matches `T` and
           traps with `msg` otherwise. `Option.expect((Type.try-as 5 : Option Int64), \"not an int\")` is
           `5` — `5` IS an `Int64`, so `Some 5` is unwrapped. Pins the composition as the assert-of-type
           idiom (the reason no `Type.assert-as` primitive exists).")
  (input (Option.expect (: (Type.try-as 5) (Option Int64)) "not an int"))
  (output (: 5 Int64)))

; ---- Same-name MONOMORPHIC constructor in a VALUE position: the ctor wins, in a helper AND multi-variant.
; The cases above are the TYPE-position complement (an applied same-name generic denotes the TYPE). These pin
; the VALUE-position rule for a MONOMORPHIC same-name sum: `(N a)` builds the VARIANT, not the type — direct
; in a body, in a CALLED helper (whose body is β-copied to a synth node at instantiation), and for a
; multi-variant sum whose FIRST variant shares the type name. All three used to falsely reject CDZ0203 ("`N`
; is a type that takes no type parameters") because the same-name ctor index was gated to single-variant OR
; the head-position rule skipped the synth β-copy node; the monomorphic-sum fix fires the ctor rule for a
; same-name variant regardless of variant count and through the β-copy. A GENERIC same-name ctor via a helper
; is a distinct harder case that STILL declines (pinned below).
(case
  "a same-name variant of a multi-variant sum constructs bare in a body"
  (doc
    "`(type N (N Int64) (J Int64))` — the FIRST variant shares the type's name. Bare `(N a)` DIRECT in
           main builds the VARIANT (not the type): `f(4)` matches the `N` arm → 4 + 1 = 5. The single-variant
           twin `(type Meters (Meters Int64))` already worked in this position; this pins that variant COUNT
           does not change whether the same-name constructor is visible bare (the ctor index is not gated to
           `variants.len()==1`). The qualified `N.N` works either way.")
  (input
    (do
      (type N (N Int64) (J Int64))
      (def (main (: a Int64)) (match (N a) ((N v) (+ v 1)) ((J w) w)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 5 Int64)))

(case
  "a same-name monomorphic constructor in a called helper resolves to the constructor"
  (doc
    "`(type Meters (Meters Int64))` gives the constructor the type's name; `(def (mk a) (Meters a))`
           uses the bare constructor in a helper `main` CALLS. At mk's instantiation its body is β-copied to a
           synth node, and the head-position ctor rule must still fire there (a monomorphic same-name sum has
           no `sum_applied` synth type-expr to confuse) — so `(Meters a)` builds the VARIANT, exactly as it
           does written directly in main. `mk(4)` → matches `Meters` → 4 + 1 = 5. Used to reject CDZ0203 at
           mk's call site (the β-copied `(Meters a)` synth node fell to the type binding) while a NEVER-CALLED
           helper compiled — the lazy-resolution tell. The qualified `Meters.Meters` in the helper always
           worked.")
  (input
    (do
      (type Meters (Meters Int64))
      (def (mk a) (Meters a))
      (def (main (: a Int64)) (match (mk a) ((Meters v) (+ v 1))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 5 Int64)))

(case
  "a same-name monomorphic constructor over a COMPOUND payload in a called helper constructs"
  (doc
    "FACE B of the same-name-ctor helper case above, with a COMPOUND (tuple) payload rather than a scalar: `(type Pair (Pair (Tuple Int64 Int64)))` names the constructor after the type, and `(def (mk a) (Pair (tuple a a)))` builds it in a helper `main` CALLS. The β-copied `(Pair (tuple a a))` synth node must fire the head-position ctor rule for a monomorphic same-name sum whose payload is a Tuple — not only for a scalar payload like `(Meters a)` above. `mk 5` builds `(Pair (tuple 5 5))`; the pop destructures the boxed tuple → 5 + 5 = 10.")
  (input
    (do
      (type Pair (Pair (Tuple Int64 Int64)))
      (def (mk (: a Int64)) (Pair #tuple(a a)))
      (def (main (: a Int64)) (match (mk a) ((Pair t) (match t (#tuple(x y) (+ x y))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a same-name multi-variant constructor in a called helper resolves to the constructor"
  (doc
    "The multi-variant twin of the helper case above: `(type N (N Int64) (J Int64))` with `(def (mk a)
           (N a))` called from main. The β-copied `(N a)` in mk's body resolves to the VARIANT for a
           monomorphic multi-variant sum too — `mk(4)` matches the `N` arm → 5. Pins that the synth-node
           head-position ctor rule is independent of variant count, closing the {single, multi} × {direct,
           helper} matrix for a monomorphic same-name sum.")
  (input
    (do
      (type N (N Int64) (J Int64))
      (def (mk a) (N a))
      (def (main (: a Int64)) (match (mk a) ((N v) (+ v 1)) ((J w) w)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 5 Int64)))

(case
  "a same-name GENERIC constructor via a called helper resolves to the constructor (adv-63)"
  (doc
    "The former residual boundary, now CLOSED (adv-63, deliberate flip): a GENERIC same-name sum `(type
           Box (Box a))` constructed via a called helper `(def (mk x) (Box x))` and INLINED into main now
           resolves the β-copied `(Box x)` head to the VARIANT CONSTRUCTOR — completing the {mono, generic} ×
           {direct, helper} matrix. Previously this DECLINED CDZ0203 because the synth-node ctor rule was
           monomorphic-gated (a generic sum has a confusable `sum_applied` type-expr synth `(Box a)` that must
           stay the type). The fix distinguishes the two synth kinds by β-copy PROVENANCE: an inlined VALUE
           construct traces (`source_of_synth`) back to the author's value-position `Box` outside any
           type-expression subtree, whereas the `sum_applied` synth has none — so the value construct fires the
           ctor while the generic type-expr path stays the type. `mk(4)` builds `(Box 4)`, matched → 5.")
  (input
    (do
      (type Box (Box a))
      (def (mk x) (Box x))
      (def (main (: a Int64)) (match (mk a) ((Box v) (+ v 1))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 5 Int64)))

(case
  "a same-name GENERIC ctor CONSTRUCTED AND MATCHED inside an inlined callee resolves (adv-63 b1)"
  (doc
    "The construct-and-consume-fully-inside-the-callee face of adv-63 (distinct from the case above,
           where the ctor value crosses OUT of the helper into the caller's match): here the WHOLE
           `(match (Box k) ((Box v) v))` lives inside `inner`, and `inner` is called from `main`. When
           `inner` β-inlines into `main`, its `(Box k)` head must still resolve to the VARIANT constructor
           (not the same-name generic type) at the inlined site — the provenance-based synth disambiguation
           must survive a full match-expression inline, not just a bare-ctor-value inline. `main 5` → inner
           builds `(Box 5)`, matched → 5. Runtime arg so nothing folds.")
  (input
    (do
      (type Box (Box a))
      (def (inner (: k Int64)) (match (Box k) ((Box v) v)))
      (def (main (: n Int64)) (inner n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a same-name GENERIC ctor in an inlined callee resolves with a CONST argument (adv-63 b2)"
  (doc
    "The const-argument twin of the inline-callee face above: `(inner 7)` passes a compile-time
           constant, so `inner` β-reduces with `k`=7 folded in. The inlined `(Box 7)` head must STILL
           resolve to the variant constructor through the β-reduction — the const-fold path must not
           re-route the same-name head to the type. `main` (any arg) → `(inner 7)` builds `(Box 7)`,
           matched → 7. Pins that the adv-63 resolve fix holds under const-argument inlining too, the
           complement of the runtime-arg face.")
  (input
    (do
      (type Box (Box a))
      (def (inner (: k Int64)) (match (Box k) ((Box v) v)))
      (def (main (: n Int64)) (inner 7))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "Type.of on a bare nullary variant reflects its element as UNDETERMINED"
  (doc
    "`Type.of` reflects a value's INFERRED type, so a bare nullary variant — carrying no element —
           reflects an UNDETERMINED element. `(None)` is `Option ?a`, distinct from a concrete `Option
           Int64` (`(Some 1)`): `(Type.eq (Type.of (None)) (Type.of (Some 1)))` is `false` — the `?a` is not
           yet `Int64`. Pins that reflection over a polymorphic value sees the type as inferred (undetermined
           here), NOT eagerly grounded — the type-value analogue of a bare `None`'s open element type. (See
           the next case: CONTEXT that constrains the element makes them equal.)")
  (input (do (def (main) (if (Type.eq (Type.of (None)) (Type.of (Some 1))) 1 0)) (export main)))
  (output (: 0 Int64)))

(case
  "context that constrains a nullary variant's element makes Type.of concrete"
  (doc
    "The complement of the bare case above: when CONTEXT pins a nullary variant's element, `Type.of`
           reflects the concrete instantiation. `pick : (Option Int64) -> (Option Int64)` forces its `(None)`
           argument to `Option Int64`, so `(Type.eq (Type.of (pick (None))) (Type.of (Some 1)))` is `true` —
           both `Option Int64`. Pins that `Type.of` tracks the element the surrounding constraints DETERMINE,
           so the same `(None)` reflects `Option ?a` bare (false above) but `Option Int64` in a typed
           position (true here) — reflection is over the SOLVED type, not the syntactic constructor.")
  (input
    (do
      (def (pick (: o (Option Int64))) o)
      (def (main) (if (Type.eq (Type.of (pick (None))) (Type.of (Some 1))) 1 0))
      (export main)))
  (output (: 1 Int64)))

; The two cases above pin bare-vs-concrete and context-constrains. These pin the NEIGHBORS: two bare
; undetermined values reflect the SAME type (the undetermined element canonicalizes, so `Type.of` is stable
; — not two distinct fresh vars that compare unequal), the same nullary variant constrained to DIFFERENT
; concrete types reflects different types, and the List analogue of the bare-None undetermined case.
(case
  "two bare nullary variants reflect the same undetermined type"
  (doc
    "`(Type.eq (Type.of (None)) (Type.of (None)))` is true — both bare `(None)`s reflect `Option ?a`
           with the SAME canonical undetermined element, so their reflected types are equal. Pins that the
           undetermined element is CANONICAL (stable) across two reflections, not two distinct fresh vars
           that would compare unequal — the type-level analogue of the value-level `(= (None) (None))` = true.
           Complements the bare-vs-concrete case (Option ?a ≠ Option Int64): undetermined = undetermined,
           undetermined ≠ concrete.")
  (input (do (def (main) (if (Type.eq (Type.of (None)) (Type.of (None))) 1 0)) (export main)))
  (output (: 1 Int64)))

(case
  "the same nullary variant constrained to different concrete types reflects different types"
  (doc
    "`(None)` forced to `Option Int64` by one context and `Option Bool` by another reflects DIFFERENT
           concrete types: `(Type.eq (Type.of (pi (None))) (Type.of (pb (None))))` is false. Pins that
           `Type.of` tracks the element each surrounding constraint determines — the same syntactic `(None)`
           reflects `Option Int64` in an Int64 context and `Option Bool` in a Bool context, so the two are
           unequal. The differing-constraint companion of the single-constraint case above.")
  (input
    (do
      (def (pi (: o (Option Int64))) o)
      (def (pb (: o (Option Bool))) o)
      (def (main) (if (Type.eq (Type.of (pi (None))) (Type.of (pb (None)))) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "a bare empty list reflects an undetermined element, distinct from a concrete list type"
  (doc
    "The List analogue of the bare-`None` undetermined case: `(Type.of (list))` is `List ?a` (an empty
           list carries no element to fix the type), distinct from the concrete `(Type.of (list 1))` = `List
           Int64`, so `Type.eq` is false. Pins that reflection over an empty polymorphic collection sees the
           element as undetermined, exactly as a bare nullary variant does — the collection form of the
           inferred-not-eagerly-grounded rule.")
  (input (do (def (main) (if (Type.eq (Type.of #list()) (Type.of #list(1))) 1 0)) (export main)))
  (output (: 0 Int64)))

(case
  "Type.eq compares a TUPLE type-value structurally, by element types"
  (doc
    "`Type.of` on a tuple value reflects its structural `Ty::Tuple` type, compared element-wise.
           `(Type.of (tuple 1 \"a\"))` is `(Tuple Int64 String)`: equal to another `(Tuple Int64 String)`
           regardless of the element VALUES (→ true), but distinct from `(Tuple Int64 Int64)` (a differing
           element type → false). `1 + 0 = 1`. Pins that type equality over a tuple type-value is by the
           element TYPES, not the values — the tuple analogue of the record/sum structural comparison.")
  (input
    (do
      (def
        (main)
        (+
          (if (Type.eq (Type.of #tuple(1 "a")) (Type.of #tuple(2 "b"))) 1 0)
          (if (Type.eq (Type.of #tuple(1 "a")) (Type.of #tuple(1 2))) 10 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.eq compares a RECORD type-value structurally, by field types"
  (doc
    "`Type.of` on a record value reflects its structural `Ty::Record` type, compared by field name +
           type. `(Type.of (record (x 1) (y \"a\")))` is `(Record (: x Int64) (: y String))`: equal to another
           record of the same field-name-and-type set regardless of values (→ true), but distinct when a
           field's TYPE differs — `(y String)` vs `(y Int64)` → false. `1 + 0 = 1`. Pins that a record
           type-value's equality carries each field's type (the record analogue of the tuple case above).")
  (input
    (do
      (def
        (main)
        (+
          (if
            (Type.eq (Type.of #record((= x 1) (= y "a"))) (Type.of #record((= x 2) (= y "b"))))
            1
            0)
          (if
            (Type.eq (Type.of #record((= x 1) (= y "a"))) (Type.of #record((= x 1) (= y 2))))
            10
            0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.of reflects a function's BODY-SOLVED domain, distinguishing different parameter types"
  (doc
    "`Type.of` on a FUNCTION value reflects its arrow type with each UNANNOTATED parameter solved from
           the body, not left undetermined. `f x = x + 1` has domain `Int64` (the `+` pins it) and `g b =
           if b 0 1` has domain `Bool` (the `if` condition pins it), both returning `Int64`. Their reflected
           arrows `(-> Int64 Int64)` and `(-> Bool Int64)` differ in the DOMAIN, so `Type.eq` is false —
           `0 + 0 = 0`. Guards the reflection-soundness fix: a bottom-up arrow left both domains `Any`, so
           two functions with genuinely different parameter types reflected the SAME `(-> Any Int64)` and
           `Type.eq` returned a WRONG `true` (a miscompiled reflection); the body-solve grounds the domain.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+ (if (Type.eq (Type.of f) (Type.of g)) 1 0) (if (Type.eq (Type.of g) (Type.of f)) 10 0)))
      (export main)))
  (output (: 0 Int64)))

(case
  "a function's reflected domain matches an equivalent explicitly-annotated signature"
  (doc
    "The dual of the domain-distinguishing case: an UNANNOTATED parameter solved from the body reflects
           the SAME type an explicit annotation would. `f x = x + 1` (domain solved `Int64`) and `h (: y
           Int64) = y + 1` (domain declared `Int64`) reflect the SAME `(-> Int64 Int64)`, so `Type.eq` is
           true — `1`. Guards the second facet of the same fix: the old bottom-up arrow reflected `f` as
           `(-> Any Int64)`, unequal to the annotated `(-> Int64 Int64)`, so an unannotated function was
           wrongly DISTINCT from its own explicitly-typed twin.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (h (: y Int64)) (+ y 1))
      (def (main) (if (Type.eq (Type.of f) (Type.of h)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.of reflects a RETURNED function's body-solved domain, at every currying level"
  (doc
    "Reflection solves the parameters of a RETURNED function too, not just the outer ones. `adder n =
           (fn (x) (+ x n))` is `(-> Int64 (-> Int64 Int64))` and `pick n = (fn (b) (if b n 0))` is
           `(-> Int64 (-> Bool Int64))` — same OUTER domain, but the returned function's domain differs
           (`Int64` vs `Bool`). Their reflected arrows differ there, so `Type.eq` is false — `0 + 0 = 0`.
           Guards the curried facet of the function-domain reflection fix: solving only the outer params left
           the returned lambda's domain `Any` (the bottom-up `type_of` again), so a curried `Int64->Int64->
           Int64` and `Int64->Bool->Int64` reflected the SAME `Int64->(-> Any Int64)` and `Type.eq` returned
           a wrong `true` — the reflection miscompile one currying level deeper.")
  (input
    (do
      (def (adder n) (fn (x) (+ x n)))
      (def (pick n) (fn (b) (if b n 0)))
      (def
        (main)
        (+
          (if (Type.eq (Type.of adder) (Type.of pick)) 1 0)
          (if (Type.eq (Type.of pick) (Type.of adder)) 10 0)))
      (export main)))
  (output (: 0 Int64)))

; The fix's cases pin that DIFFERENT body-solved arrows reflect DISTINCT types (and match an annotated
; signature). These pin the complements: two functions with the SAME body-solved arrow but different BODIES
; reflect EQUAL types (the domain-solve equates, doesn't spuriously distinguish by body), and the CODOMAIN
; is body-solved too (not just the domain) — a function whose result is inferred to Bool reflects a distinct
; arrow from one returning Int64, and two Int64→Bool functions reflect equal.
(case
  "two functions with the same body-solved arrow but different bodies reflect equal types"
  (doc
    "`f x = x + 1` and `h x = x * 2` are BOTH unannotated with body-solved domain `Int64` and result
           `Int64`, so their reflected arrows are the SAME `(-> Int64 Int64)` despite different bodies —
           `Type.eq` is true. The equality companion of the fix's distinguish-different-domains case: the
           domain-solve reflects the TYPE, not the body, so two genuinely same-typed functions equate (a
           reflection keyed on the body would wrongly distinguish them).")
  (input
    (do
      (def (f x) (+ x 1))
      (def (h x) (* x 2))
      (def (main) (if (Type.eq (Type.of f) (Type.of h)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a function's reflected CODOMAIN is body-solved, distinguishing an Int64 result from a Bool result"
  (doc
    "The fix's cases all return Int64; this pins the RESULT type is body-solved too. `f x = x + 1` is
           `(-> Int64 Int64)` and `p x = x > 0` is `(-> Int64 Bool)` (the `>` solves the result to Bool),
           so their reflected arrows differ in the CODOMAIN → `Type.eq` false. Pins that reflection solves
           both ends of the arrow, not just the domain — a codomain left `Any` would wrongly equate them.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (p x) (> x 0))
      (def (main) (if (Type.eq (Type.of f) (Type.of p)) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "two functions both body-solved to Int64->Bool reflect equal arrows"
  (doc
    "The equality companion at a body-solved CODOMAIN: `p x = x > 0` and `q x = x < 5` are both
           `(-> Int64 Bool)` (domain Int64 from the comparison, result Bool), so their reflected arrows are
           equal → `Type.eq` true. Together with the case above this pins that the codomain-solve equates
           same-result functions and distinguishes different-result ones — the arrow is solved end-to-end.")
  (input
    (do
      (def (p x) (> x 0))
      (def (q x) (< x 5))
      (def (main) (if (Type.eq (Type.of p) (Type.of q)) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.of grounds a function stored inside a compound value's element"
  (doc
    "Reflection solves a function's domain even when the function is an ELEMENT of a tuple/list/record,
           not just a bare operand. `(tuple f 0)` with `f x = x + 1` reflects `(Tuple (-> Int64 Int64)
           Int64)` and `(tuple g 0)` with `g b = if b 0 1` reflects `(Tuple (-> Bool Int64) Int64)` — the
           element function types differ, so `Type.eq` is false; but `(list f)` vs `(list f2)` (both `(-> Int64
           Int64)`) is true. `0 + 100 = 100`. Guards the compound-element facet of the function-domain
           reflection fix: the element type came from the bottom-up `type_of` (which leaves an unannotated fn
           element `(-> Any R)`), so two compounds whose element functions had genuinely different domains
           reflected the SAME type and `Type.eq` returned a wrong `true`. Reflection now recurses the compound
           and grounds each fn element from its body.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of #tuple(f 0)) (Type.of #tuple(g 0))) 1 0)
          (if (Type.eq (Type.of #list(f)) (Type.of #list(f2))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of grounds a function stored inside a SUM VARIANT payload"
  (doc
    "The sum-payload sibling of the compound-element case: a function wrapped in a variant payload
           `(Some f)` reflects its domain body-solved too, not just tuple/list/record elements. `f x = x + 1`
           is `(-> Int64 Int64)` and `g b = if b 0 1` is `(-> Bool Int64)`, so `(Some f)` reflects `(Option
           (-> Int64 Int64))` and `(Some g)` reflects `(Option (-> Bool Int64))` — distinct → `Type.eq`
           false; but `(Some f)` vs `(Some f2)` (both `(-> Int64 Int64)`) is true. `0 + 100 = 100`. Guards
           the sum-payload facet: the payload flows into the sum's type argument via the ctor scheme, which
           unified the fn's UNANNOTATED-domain `(-> Any Int64)`, so two different-domain wrapped functions
           reflected the SAME `(Option (-> Any Int64))` and `Type.eq` returned a wrong `true`. Reflection now
           re-runs the ctor scheme with the payload's grounded type. The codomain through the payload was
           already solved — only the domain leaked.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of (Some f)) (Type.of (Some g))) 1 0)
          (if (Type.eq (Type.of (Some f)) (Type.of (Some f2))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of grounds a function nested TWO containers deep — a sum payload inside a list"
  (doc
    "The nested facet of the function-domain reflection family: a function two containers deep — a
           variant payload inside a list, `(list (Some f))` — is grounded from its body just like the
           single-container element/payload cases. `f x = x + 1` is `(-> Int64 Int64)` and `g b = if b 0 1`
           is `(-> Bool Int64)`, so `(list (Some f))` reflects `(List (Option (-> Int64 Int64)))` and
           `(list (Some g))` reflects `(List (Option (-> Bool Int64)))` — distinct → `Type.eq` false; but
           `(list (Some f))` vs `(list (Some f2))` (both `(-> Int64 Int64)`) is true. `0 + 100 = 100`.
           Guards that reflection recurses through STACKED containers to ground the fn, not just one level.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of #list((Some f))) (Type.of #list((Some g)))) 1 0)
          (if (Type.eq (Type.of #list((Some f))) (Type.of #list((Some f2)))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of grounds a function nested TWO containers deep — a tuple inside a sum payload"
  (doc
    "The complementary nesting order: a function inside a tuple inside a variant payload,
           `(Some (tuple f 0))`. `f x = x + 1` vs `g b = if b 0 1` give distinct payload tuple types, so
           `(Some (tuple f 0))` reflects `(Option (Tuple (-> Int64 Int64) Int64))` and `(Some (tuple g 0))`
           reflects `(Option (Tuple (-> Bool Int64) Int64))` — `Type.eq` false; same domain (`f` vs `f2`)
           true. `0 + 100 = 100`. Pins that the ctor-scheme re-grounding (sum payload) composes with the
           compound-element re-grounding (tuple) when they are nested.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of (Some #tuple(f 0))) (Type.of (Some #tuple(g 0)))) 1 0)
          (if (Type.eq (Type.of (Some #tuple(f 0))) (Type.of (Some #tuple(f2 0)))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of grounds a RETURNED function's domain — a closure whose result is a function"
  (doc
    "A higher-order value: `(fn (a) f)` is a closure returning `f`, so its reflected type is
           `(-> Any (-> Int64 Int64))` grounded at the RETURNED function's domain. `f x = x + 1` vs
           `g b = if b 0 1`: `(fn (a) f)` and `(fn (a) g)` reflect arrows with different codomain-function
           domains → `Type.eq` false; `(fn (a) f)` vs `(fn (a) f2)` (both return `(-> Int64 Int64)`) is
           true. `0 + 100 = 100`. Guards that reflection grounds a function reachable through a RESULT
           position, not only through container elements/payloads.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of (fn (a) f)) (Type.of (fn (a) g))) 1 0)
          (if (Type.eq (Type.of (fn (a) f)) (Type.of (fn (a) f2))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of grounds a function stored in a RECORD FIELD"
  (doc
    "The record-field position of the function-domain reflection family (coverage companion to the
           reflected_ty domain-grounding fixes). A function in a record field `(record (fld f))` is grounded
           from its body like a tuple/list element: `f x = x + 1` is `(-> Int64 Int64)` and `g b = if b 0 1`
           is `(-> Bool Int64)`, so `(record (fld f))` reflects `(Record (: fld (-> Int64 Int64)))` and
           `(record (fld g))` reflects `(Record (: fld (-> Bool Int64)))` — distinct → `Type.eq` false; but
           `(record (fld f))` vs `(record (fld f2))` (both `(-> Int64 Int64)`) is true. `0 + 100 = 100`.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if (Type.eq (Type.of #record((= fld f))) (Type.of #record((= fld g)))) 1 0)
          (if (Type.eq (Type.of #record((= fld f))) (Type.of #record((= fld f2)))) 100 0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Type.of reflects equal for two functions of the SAME TYPE stored as a MAP VALUE"
  (doc
    "The map-value position (coverage companion to the reflected_ty domain-grounding fixes) — the
           same-TYPE half: `Map.insert(Map.empty, 1, f)` and the same with `f2` — two DISTINCT functions,
           both `(-> Int64 Int64)`, so they
           reflect the SAME map type, so `Type.eq` is true. `main` returns 1. The DIFFERENT-domain
           map-value/key case (a leak that once left a Map fn's domain `Any`) is now FIXED and pinned by the
           companion case below; this same-fn case is the already-correct control.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def
        (main)
        (if
          (Type.eq (Type.of (Map.insert Map.empty 1 f)) (Type.of (Map.insert Map.empty 1 f2)))
          1
          0))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.of grounds a function stored as a runtime Map value"
  (doc
    "The runtime-map-builder sibling of the sum-payload / compound-element cases: `(Map.insert
           Map.empty 1 f)` builds a map whose VALUE is a function. Unlike the `(map (k v) …)` literal (whose
           element nodes are read directly), a `Map.insert` result type comes from the op scheme, which read
           the value via bottom-up `type_of` — leaking the fn's domain as `Any` (`(Map Int64 (-> Any
           Int64))`), so two maps with different-domain value functions reflected the SAME type and `Type.eq`
           returned a wrong `true`. Reflection now rebuilds the `(Map k v)` from its grounded key + value.
           `f x = x + 1` (`Int64 -> Int64`) and `g b = if b 0 1` (`Bool -> Int64`) → distinct maps →
           `Type.eq` false; `f` vs `f2` (both `Int64 -> Int64`) → equal. `0 + 100 = 100`.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if
            (Type.eq (Type.of (Map.insert Map.empty 1 f)) (Type.of (Map.insert Map.empty 1 g)))
            1
            0)
          (if
            (Type.eq (Type.of (Map.insert Map.empty 1 f)) (Type.of (Map.insert Map.empty 1 f2)))
            100
            0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "a function stored as a runtime Map KEY is rejected (CDZ0216 — a function is not equatable/orderable)"
  (doc
    "A FUNCTION cannot be a Map KEY: a closure has no canonical identity, so it is neither equatable
           nor orderable, and structural key membership needs equality (collections-and-text §A Set Is A
           Collection Of Unique Elements). `(Map.insert Map.empty f 1)` keys the map by the function `f` →
           CDZ0216 (NotEquatable). Distinct from CDZ0202 (the abstract/nominal-BOUNDARY opacity code): this
           is INTRINSIC non-comparability, not a boundary issue (v-inference ruling, concierge-confirmed).
           (Formerly this case tested `Type.of` grounding of the key fn's DOMAIN and expected 100 — but a
           function key is now an outright reject, so key-domain reflection is moot; the VALUE-position
           companion above still pins the reflected_ty (Map k v) grounding for a legal fn Map VALUE.)")
  (input (do (def (f x) (+ x 1)) (def (main) (Map.len (Map.insert Map.empty f 1))) (export main)))
  (error CDZ0216))

(case
  "a function stored as a runtime SET element is rejected (CDZ0216 — the Set face of the Map-key reject)"
  (doc
    "The SET-element companion of the fn-Map-KEY reject above: a Set element needs the same equality
           a Map key does (membership is decided by the element's value under core-semantics.md §Equality Is
           Structural — collections-and-text §Keys Are Compared By Value, Not Representation for maps, §Set
           Membership Is Total for sets), and a function is neither equatable nor orderable (no canonical
           identity), so `(Set.of (list (fn (x) (+ x 1))))` → CDZ0216 (NotEquatable). Pins the Set face + an
           INLINE lambda (the Map-key case uses a named `def f`), so the reject fires on an anonymous `fn`,
           not only a named function. This closed a wasm-vs-rust DIVERGENCE breaker found (adv-50 residual):
           wasm formerly INVENTED a closure identity and computed, while rust E0277'd on `dyn Fn: Ord` — the
           uniform CDZ0216 reject at type-check is the ruled fix (v-inference, concierge-confirmed).")
  (input (do (def (main) (Set.len #set((fn (x) (+ x 1))))) (export main)))
  (error CDZ0216))

(case
  "a function in a NATIVE SET LITERAL is rejected (CDZ0216 — the native-literal face must not bypass the prim-app check)"
  (doc
    "The NATIVE `#set` LITERAL face of the fn-set-element reject above. `(\"set\" (fn (x) (+ x 1)))` — the
           first-class tagged set literal (ML `#(fn(x) => x + 1)`) — keys/hashes its elements exactly as
           `Set.of` does, so a function element is CDZ0216 here too. This closed an M2 SOUNDNESS HOLE: the
           key-hashability gate fired ONLY on set/map PRIM APPLICATIONS (`Set.of`/`Set.insert`/…), so the
           s-expr `(Set.of (list (fn …)))` above correctly declined, but the native `#set` literal resolved
           to `Resolved::Set` (never a prim app) and SAILED THROUGH — type-checking a set of functions (wasm
           would invent a closure identity). Pre-M2 there was no native literal so the prim-only gate was
           complete; the M2 printer's native literal reopened it. The fix runs the SAME element/key check on
           the `Resolved::Set`/`Resolved::Map` literal nodes. A `#list` of functions stays legal (a list does
           not hash its elements) — only the set/map literals are gated.")
  (input (do (def (main) (Set.len #set((fn (x) (+ x 1))))) (export main)))
  (error CDZ0216))

(case
  "a heterogeneous native #map literal NAMES the two clashing value types (diagnostic quality)"
  (doc
    "`#map((= 1 1) (= 2 \"b\"))` mixes an Int64 value with a String value -> CDZ0201, and the message
           NAMES both clashing types (\"the values differ: Int64 and String\"), matching the `map` name-alias
           (`Apply(MapNew)`) path — the native-literal (`Resolved::Map`) arm must not give a WORSE, type-name-
           less message than the alias for the identical fault. Pins the diagnostic quality the infer.rs
           submodule split (#6039) had regressed on this literal arm to the generic \"do not share a type\".")
  (input (do (def (main) (Map.len #map((= 1 1) (= 2 "b")))) (export main)))
  (error CDZ0201 (message "values differ") (message "Int64") (message "String")))

(case
  "a function KEY in a NATIVE MAP LITERAL is rejected (CDZ0216 — the #map sibling of the native #set bypass)"
  (doc
    "The NATIVE `#map` LITERAL face: `(map ((fn (x) x) 1))` — a map literal whose sole entry is keyed by
           a function — is CDZ0216, the same as `(Map.insert Map.empty f 1)`. Sibling of the native-`#set`
           case above; both close the M2 native-literal bypass of the key-hashability gate (a native literal
           resolves to `Resolved::Map`/`Set`, not a prim app, so the fault-walk must check the literal node
           itself). The map VALUE axis is unconstrained (a fn value is legal); only the KEY is hashed.")
  (input (do (def (main) (Map.len #map((= (fn (x) x) 1)))) (export main)))
  (error CDZ0216))

(case
  "a TUPLE containing a closure as a map key is rejected (CDZ0216 descends into compound keys)"
  (doc
    "The compound-wrapped face of the fn-Map-KEY reject above: the function is not the key itself
           but a COMPONENT of a tuple key — `(Map.insert Map.empty (tuple 1 f) 42)`. Keyability is
           decided over the WHOLE key type, so the check must descend into the tuple and find the
           un-equatable fn leaf → CDZ0216. A keyability check inspecting only the top-level constructor
           (Tuple is normally keyable) would admit the closure into the CHAMP hash path — the exact
           smuggle that re-opens the adv-50 wasm-invents-identity vs rust-E0277 divergence the bare
           reject closed. The closure CAPTURES a runtime binding (n) so nothing erases it.")
  (input
    (do
      (def
        (main (: n Int64))
        (do (def f (fn ((: x Int64)) (+ x n))) (Map.len (Map.insert Map.empty #tuple(1 f) 42))))
      (export main)))
  (error CDZ0216))

(case
  "a RECORD with a closure field as a map key is rejected (the record face of the compound descent)"
  (doc
    "The record companion of the tuple-wrapped fn-key reject: `(record (id 1) (cb f))` as a Map
           key — the keyability descent must walk record FIELDS (by the descriptor, not just the head
           constructor) and reject on the fn-typed `cb` → CDZ0216. Together with the tuple face this
           pins that both compound layouts route their component types through the same keyability
           check; a callback-registry record accidentally used as a key gets the coded reject, not a
           backend-divergent identity.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def f (fn ((: x Int64)) (+ x n)))
          (Map.len (Map.insert Map.empty #record((= id 1) (= cb f)) 42))))
      (export main)))
  (error CDZ0216))

(case
  "a LIST of closures as a set element is rejected (the collection-element face of the descent)"
  (doc
    "The third wrapper kind: the closure hides inside a LIST that is itself a Set ELEMENT —
           `(Set.of (list (list (fn ...))))`. The element-keyability check must descend through the
           collection's ELEMENT type (a (List (-> Int64 Int64)) is un-equatable because its element is)
           → CDZ0216. Completes the compound descent family: tuple component, record field, and
           collection element all reject uniformly on every backend (no wasm-invented closure identity
           at any nesting).")
  (input
    (do (def (main (: n Int64)) (Set.len #set(#list((fn ((: x Int64)) (+ x n)))))) (export main)))
  (error CDZ0216))

(case
  "Type.of grounds a Map-value function nested inside a tuple"
  (doc
    "The compounding facet: a runtime-Map-value function wrapped in an outer container —
           `(tuple (Map.insert Map.empty 1 f) 0)`. The tuple-element reflection recurses INTO the
           `Map.insert` node, so the (Map k v) grounding fix flows through the wrapper automatically: the
           tuples' first-element map types differ when the value functions differ (`f : Int64 -> Int64` vs
           `g : Bool -> Int64`) → `Type.eq` false; `f` vs `f2` → equal. `0 + 100 = 100`. Pins that the Map
           grounding composes with an outer container rather than being re-leaked at the wrapper.")
  (input
    (do
      (def (f x) (+ x 1))
      (def (f2 z) (+ z 9))
      (def (g b) (if b 0 1))
      (def
        (main)
        (+
          (if
            (Type.eq
              (Type.of #tuple((Map.insert Map.empty 1 f) 0))
              (Type.of #tuple((Map.insert Map.empty 1 g) 0)))
            1
            0)
          (if
            (Type.eq
              (Type.of #tuple((Map.insert Map.empty 1 f) 0))
              (Type.of #tuple((Map.insert Map.empty 1 f2) 0)))
            100
            0)))
      (export main)))
  (output (: 100 Int64)))

(case
  "an if on Type.eq selects a branch at compile time"
  (doc
    "The headline of compile-time reflection: `(if (Type.eq (Type.of 5) Int64) 100 200)` folds the
           condition to the constant `true`, so the whole `if` is `100`. A program BRANCHES on types at
           compile time — the type comparison decides control flow with no runtime cost (the condition is
           a constant, not an emitted test).")
  (input (if (Type.eq (Type.of 5) Int64) 100 200))
  (output (: 100 Int64)))

(case
  "a compile-time type branch reads a runtime parameter's static type"
  (doc
    "`(if (Type.eq (Type.of n) Int64) (+ n 1) 0)` for a parameter `n : Int64` branches on `n`'s
           STATIC type (Int64), folding the condition to `true` at compile time, so `main 7` returns 8.
           Pins that the branch is decided by the parameter's inferred type — not its runtime value — yet
           the selected branch runs on the actual value.")
  (input (do (def (main (: n Int64)) (if (Type.eq (Type.of n) Int64) (+ n 1) 0)) (export main)))
  (call main (: 7 Int64))
  (output (: 8 Int64)))

; A type-value is compile-time-only — it never flows from runtime data (type-system.md §226), so it has no
; boundary/runtime form. Where one WOULD need a runtime representation — a PARAMETERIZED type-valued export
; (its result would depend on a runtime arg), a `(: t Type)` parameter used in a VALUE position, a type-value
; nested in a compound RESULT — the compiler reports ONE coded reject (CDZ0201 "is a TYPE, not a runtime
; value", or CDZ0203 for a Bool-checked `if` condition) and DEDUPS the downstream no-runtime-form declines,
; so it is one clean error, not a 2–4-line cascade. `(no-other-errors)` pins the no-cascade (no other coded
; error accompanies the reject). A BAKEABLE type-value — a nullary `(: Int64 Type)` export that reduces to a
; fully compile-time-known type — DOES cross and is not rejected. (migrated from rcdzc
; a_non_bakeable_type_valued_export / a_type_valued_param_used_in_a_value_position /
; a_type_stored_in_a_compound_result, the one-coded-error-not-a-cascade family; enabled by the C1
; (no-other-errors) facet.)
(case
  "a parameterized type-valued export is one coded error, not a no-runtime-form cascade"
  (input (do (def (main (: n Int64)) Int64) (export main)))
  (error CDZ0201 (message "is a TYPE, not a runtime value"))
  (no-other-errors))

(case
  "a bakeable nullary type-value export crosses the boundary (the control)"
  (input (do (def (main) (: Int64 Type)) (export main)))
  (call main)
  (output (: Int64 Type)))

(case
  "a type-valued parameter in an arithmetic position is one coded error, no cascade"
  (input (do (def (f (: t Type)) (+ t 1)) (def (main) (f Int64)) (export main)))
  (error CDZ0201)
  (no-other-errors))

(case
  "a type-valued parameter as a Bool-checked if condition is one coded error, no cascade"
  (input (do (def (f (: t Type)) (if t 1 2)) (def (main) (f Int64)) (export main)))
  (error CDZ0203 (message "found Type"))
  (no-other-errors))

; A position that binds a type-valued parameter `(: t Type)` is a bidirectional-CHECKING boundary
; (type-system.md #Generics Are Type-Valued Parameters, line 60: "a type is CHECKED against an explicit
; annotation, RATHER THAN SOLVED BY UNIFICATION"). So passing a type VALUE for `t` CONSTRAINS a sibling
; `(: x t)`: a value arg of a DIFFERENT type is checked against the passed witness and REJECTED (CDZ0203),
; not silently unification-solved (which would leave the passed type DEAD — the over-accept the spec forbids,
; surfaced by the `forall a. a` sugar that desugars to `(: a Type) (: x a)`). The boundary CHECKS, it does
; NOT over-constrain: an arg whose type AGREES with the witness compiles and runs. The check fires whether or
; not the body references the param. (Migrated from rcdzc
; a_type_valued_param_is_a_checking_boundary_a_wrong_typed_sibling_arg_is_rejected.)
(case
  "a wrong-typed sibling arg against a type-valued parameter is rejected at the checking boundary"
  (input (do (def (f (: t Type) (: x t)) x) (def (main) (f Bool 41)) (export main)))
  (error CDZ0203))

(case
  "the type-valued-param checking boundary fires even when the param is unreferenced in the body"
  (input (do (def (f (: t Type) (: x t)) 0) (def (main) (f Bool 41)) (export main)))
  (error CDZ0203))

(case
  "a different type witness (String) still checks its sibling arg against it"
  (input (do (def (f (: t Type) (: x t)) x) (def (main) (f String 7)) (export main)))
  (error CDZ0203))

(case
  "a sibling arg agreeing with an Int64 type witness compiles and runs"
  (input (do (def (f (: t Type) (: x t)) x) (def (main) (f Int64 41)) (export main)))
  (call main)
  (output (: 41 Int64)))

(case
  "a sibling arg agreeing with a Bool type witness compiles and runs"
  (input (do (def (f (: t Type) (: x t)) x) (def (main) (f Bool true)) (export main)))
  (call main)
  (output (: true Bool)))

(case
  "a type-value nested in a compound result is one coded error, no cascade"
  (input (do (def (main) #tuple(Int64 5)) (export main)))
  (error CDZ0201 (message "is a TYPE, not a runtime value"))
  (no-other-errors))

; The effect / closure siblings of the type-value non-runtime-form rejects: exporting a bare EFFECT name
; leaked a 4-error cascade of internals (unknown intrinsic / effect-op / nullary-lambda-no-closure); an
; exported closure with an UNANNOTATED param `(fn (x) 1)` : `(-> Any Int64)` cannot cross the boundary and
; leaked a second "no machine representation" decline at the body. Both now report ONE coded CDZ0201 naming
; the concrete cause, dedup dropping the leaked internals → `(no-other-errors)`. (migrated from rcdzc
; an_effect_valued_export_reports_one_clean_error_not_a_leaked_cascade /
; an_unrepresentable_closure_export_reports_one_error_not_a_shadowing_decline.)
(case
  "an effect-valued export is one coded error, not a leaked internal cascade"
  (input (do (effect E (op f (-> Int64))) (def (main) E) (export main)))
  (error CDZ0201 (message "is an effect, not a runtime value"))
  (no-other-errors))

(case
  "an unrepresentable closure export is one coded boundary error, not a shadowing decline"
  (input (do (def (main) (fn (x) 1)) (export main)))
  (error CDZ0201 (message "cannot cross the component boundary"))
  (no-other-errors))

(case
  "Type.eq branches on a type-valued parameter, monomorphized per passed type"
  (doc
    "`Type.eq` accepts a TYPE-VALUED PARAMETER `t` as an operand: `(def (is-int (: t Type) (: x
           Int64)) (if (Type.eq t Int64) 1 0))`. At each instantiation `t` is a concrete compile-time
           type-value (monomorphization substitutes it), so `(Type.eq t Int64)` folds to a constant per
           call — `(is-int Int64 5)` folds `true` → 1, `(is-int Bool 5)` folds `false` → 0, so their sum
           is 1. Pins that a type-valued parameter is a first-class `Type.eq` operand (types-as-values:
           a program branches on a passed type), not only a written type or a `Type.of` result — the
           operand in a VALUE position reduces through its `Type`-kinded annotation to the parameter's
           substituted type-value.")
  (input
    (do
      (def (is-int (: t Type) (: x Int64)) (if (Type.eq t Int64) 1 0))
      (def (main) (+ (is-int Int64 5) (is-int Bool 5)))
      (export main)))
  (output (: 1 Int64)))

; Type.eq on a type-valued parameter NEIGHBORS (breaker): the case above compares ONE parameter against a
; WRITTEN type (Int64). These pin the operand positions it doesn't: BOTH operands type-valued parameters
; (t vs u), a COMPOUND type operand (List Int64 — structural, not a leaf), a type-value THREADED through a
; second call before the Type.eq (monomorphization must carry it), and a parameter against ITSELF (always
; true regardless of the instantiation). All fold per-call at compile time, both backends.
(case
  "Type.eq compares two type-valued parameters to each other"
  (doc
    "Both operands are type-valued PARAMETERS, not one against a written type: `(Type.eq t u)`. Each
           instantiation substitutes concrete type-values, so it folds per call — `(same Int64 Int64)` folds
           true → 1, `(same Int64 Bool)` folds false → 0, sum 1. Pins that a type-valued parameter is a valid
           operand on BOTH sides of Type.eq, not only the left.")
  (input
    (do
      (def (same (: t Type) (: u Type)) (if (Type.eq t u) 1 0))
      (def (main) (+ (same Int64 Int64) (same Int64 Bool)))
      (export main)))
  (output (: 1 Int64)))

(case
  "Type.eq on a type-valued parameter against a COMPOUND type is structural"
  (doc
    "The operand may be a COMPOUND written type, not only a leaf: `(Type.eq t (List Int64))`. Type
           equality is structural, so `(List Int64)` ≠ `(List Bool)` — `(is-list-int (List Int64))` folds true
           → 1, `(is-list-int (List Bool))` folds false → 0, sum 1. Pins that Type.eq on a type-valued
           parameter compares compound types by structure (the element type distinguishes them), not by a
           head-only tag.")
  (input
    (do
      (def (is-list-int (: t Type)) (if (Type.eq t (List Int64)) 1 0))
      (def (main) (+ (is-list-int (List Int64)) (is-list-int (List Bool))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a type-valued parameter threaded through a second call reaches Type.eq with its type intact"
  (doc
    "Monomorphization must carry the type-value across a call boundary: `relay` passes its `(: t Type)`
           to `check`, which does the `Type.eq t Int64`. `(relay Int64)` folds true → 1, `(relay Bool)` folds
           false → 0, sum 1. Pins that a type-valued parameter substituted at `relay`'s instantiation flows
           into `check`'s Type.eq — the type-value is not lost when passed to another function.")
  (input
    (do
      (def (check (: t Type)) (if (Type.eq t Int64) 1 0))
      (def (relay (: t Type)) (check t))
      (def (main) (+ (relay Int64) (relay Bool)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a type-valued parameter threads through TWO relay layers beside a runtime operand"
  (doc
    "The depth-2 companion of the one-relay case above, in a body that also computes a RUNTIME
           value: `relay2 → relay → check` carries `t` across two call boundaries before the `Type.eq`,
           and `main` adds the boundary parameter `n` so the def cannot fold whole — the type-value
           substitution and the runtime arithmetic coexist in one lowered body. `(relay2 Int64)` = 1,
           `(relay2 Bool)` = 0, +41 = 42. A monomorphizer that carried the type only one hop (or
           specialized `relay2` before `relay`'s substitution resolved) would break an inner Type.eq.")
  (input
    (do
      (def (check (: t Type)) (if (Type.eq t Int64) 1 0))
      (def (relay (: t Type)) (check t))
      (def (relay2 (: t Type)) (relay t))
      (def (main (: n Int64)) (+ (relay2 Int64) (+ (relay2 Bool) n)))
      (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))

(case
  "a Type.eq width dispatcher answers per WRITTEN type at two call sites in one body"
  (doc
    "The dispatch-table idiom over types: `width-of` chains `Type.eq` tests (Int64 → 64, Int8 → 8,
           else 0) and ONE body calls it at TWO different written types, combining the answers with a
           runtime operand — 64·100 + 8 + 1 = 6409. Each call site folds its own chain independently
           (the two instantiations must not share one resolved answer), and the runtime `+ n` keeps the
           body live. The multi-site companion of the single-comparison cases above.")
  (input
    (do
      (def (width-of (: t Type)) (if (Type.eq t Int64) 64 (if (Type.eq t Int8) 8 0)))
      (def (main (: n Int64)) (+ (* (width-of Int64) 100) (+ (width-of Int8) n)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6409 Int64)))

(case
  "Type.eq of a type-valued parameter against itself is always true"
  (doc
    "`(Type.eq t t)` — a type-valued parameter compared to ITSELF — is true at EVERY instantiation,
           whatever type is passed: `(refl Int64 5)` → 1 and `(refl Bool 5)` → 1, sum 2. Pins the reflexivity
           of Type.eq on a type-valued operand independent of the substituted type (a fold that only matched
           against a WRITTEN type would miss the param-vs-same-param case).")
  (input
    (do
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
(case
  "a type stored in a compound result cannot cross the boundary"
  (doc
    "`(def (main) (tuple Int64 5))` returns `(Tuple Type Int64)` — a tuple carrying a TYPE-value in
           its first slot. A type-value is compile-time only and has no runtime form, so a compound
           carrying one cannot cross the component boundary. The compiler reports ONE coded CDZ0201 naming
           the compound (not the four uncoded no-runtime-form declines the emit path would otherwise leak).
           The rejection is the program's outcome; there is no value.")
  (input (do (def (main) #tuple(Int64 5)) (export main)))
  (error CDZ0201))

; A GENERIC newtype whose single variant's payload is a STRUCTURAL RECORD mentioning the type PARAMETER —
; `(type Box (Box (Record (: v a) (: tag Int64))))` — must register `a` as a type parameter so the
; constructor is GENERIC (a `(fn (a) (-> (Record …) (Box a)))` type-lambda), not nullary. The param scan
; (`db::collect_type_params`) descends a record field's TYPE (skipping the name label), but originally only
; the 2-element `(name Type)` pair spelling; the ML record-type surface `{v: a, tag: Int64}` lowers each
; field to a 3-element `(: name Type)` annotation triple, so the param `a` nested in a field type was NOT
; collected → `decl.params` empty → no ctor type-lambda → the free `a` in the ctor arrow made
; `typeval_of` yield `None` → the ctor read NULLARY (CDZ0201 at construction). Building `(Box (record (v
; 42) (tag 7)))` and reading its `tag` field = 7 pins that a record-field type parameter is collected (the
; ctor is generic + constructs), the record companion of the `(Tuple a …)`/`(List a)` payload arms that
; already descend params.
(case
  "a generic newtype with a structural-record payload mentioning the type parameter constructs"
  (input
    (do
      (type Box (Box (Record (: v a) (: tag Int64))))
      (def (main) (match (Box #record((= v 42) (= tag 7))) ((Box r) r.tag)))
      (export main)))
  (output (: 7 Int64)))

; --- A chained generic instantiated at a MAP type. ---
(case
  "a chained generic instantiates at a MAP type and both tuple slots share the CHAMP"
  (doc
    "The heap-collection instantiation of the chained generic (the landed pin instantiates at
           Int64 + String): `dup` duplicates a MAP through two module hops — both tuple slots hold
           the SAME CHAMP handle, one read by lookup (k·10) and one by len (+1 → 51 at k=5, 1 at
           k=0). A specialization that deep-copied the map per slot still answers right here (the
           values agree) — what this pins is the TYPE-level instantiation at (Map Int64 Int64)
           resolving through the chain; a resolution that monomorphized only at scalar types
           rejects the call.")
  (input
    (do
      (import "mid" (dup))
      (def
        (main (: k Int64))
        (do
          (def p (dup (Map.insert Map.empty 1 k)))
          (+ (* 10 (match (Map.lookup (. p 0) 1) ((Some v) v) ((None _u) -1))) (Map.len (. p 1)))))
      (export main)))
  (module "base"
    (do (def (dup x) #tuple(x x)) (export dup)))
  (module "mid"
    (do (import "base" (dup)) (export dup)))
  (call main (: 5 Int64))
  (output (: 51 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; --- Type.of over a perform. ---
(case
  "Type.of over a PERFORM result reflects the op's declared result type"
  (doc
    "The Type.of operand family covers literals, params, constructions, generic sums, and PERFORM results: for a perform the reflected type is the op's DECLARED result type ((List Int64)), resolved statically through the handler frame, Type.eq-verified against a same-type construction.")
  (input
    (do
      (effect E (op get (-> Unit (List Int64))))
      (def
        (main (: k Int64))
        (handle
          E
          k
          ((get (_u) s (resume #list(s (+ s 1)) s)))
          (if (Type.eq (Type.of (E.get)) (Type.of #list(1 2))) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "an unbound TYPE name in an uncalled definition's annotation is still rejected"
  (doc
    "The TYPE-position sibling of the uncalled-def gap: the annotation's resolve is a different walk from the body's — an ML reachability skip may treat annotations differently. rcdzc rejects CDZ0101.")
  (input (do (def (unused (: x NoSuchType)) x) (def (main) 42) (export main)))
  (error CDZ0101))

; --- Constructor-position validation-walk probes (uncalled-def CONSTRUCTION faces; the ML
; differential classifies each — all three decline in ML today, coverage-not-yet). ---
(case
  "a bare undeclared capitalized ctor on the called path is rejected (an open sum does not sanction it)"
  (doc
    "OS1 load-bearing: the open-sum row variable does NOT sanction an undeclared LOCAL constructor
           name. A bare capitalized head that names no declared variant — `(Nope 5)` — still rejects
           CDZ0101 (unbound), NOT accepted-as-a-possibly-open-ctor. Open-ness is declared via the explicit
           `.. r` marker on a `(type …)`, never by treating any-undeclared-ctor-as-open. Pins that the
           marker did not open a hole for typo'd ctor names on the called path (the uncalled-def face is the
           case below). (migrated from rcdzc an_undeclared_capitalized_ctor_still_rejects_cdz0101.)")
  (input (do (def (main) (Nope 5)) (export main)))
  (error CDZ0101))

(case
  "an unbound CONSTRUCTOR applied in an uncalled def is rejected"
  (doc
    "The construction-position face of the uncalled-def scope walk: `(NoSuchCtor 1)` in a never-called def's body — a reachability skip that only validates ctor heads on the called path runs to 42. rcdzc rejects CDZ0101; the pattern-position twin (an unbound ctor as a match-arm HEAD) is pinned in 05-compound-types.")
  (input (do (def (unused) (NoSuchCtor 1)) (def (main) 42) (export main)))
  (error CDZ0101))

(case
  "a KNOWN type's UNKNOWN variant constructed in an uncalled def is rejected"
  (doc
    "Sharper than the bare-unbound-ctor case: a type T with variant Mk IS declared, and the uncalled def constructs `(NoSuchVariant 1)` — a walk that treats any capitalized head as a possibly-later-declared ctor (because SOME type exists) runs instead of rejecting. rcdzc rejects CDZ0101.")
  (input (do (type T (Mk Int64)) (def (unused) (NoSuchVariant 1)) (def (main) 42) (export main)))
  (error CDZ0101))

(case
  "an unbound name as an ARGUMENT to a known ctor in an uncalled def is rejected"
  (doc
    "The ctor-ARGUMENT face: the head `Mk` resolves (T is declared) but its operand `no-such-value` is unbound, inside an uncalled def — a walk that validates the ctor head then skips descending into its operands runs to 42. rcdzc rejects CDZ0101.")
  (input (do (type T (Mk Int64)) (def (unused) (Mk no-such-value)) (def (main) 42) (export main)))
  (error CDZ0101))

; --- Export-resolution walk (the multi-export gap-A family: the self-hosted front-end checked
; only the sequence-terminating export — b27c59b96 then the d3516ee2c residual; full order
; matrix pinned so neither face regresses). ---
(case
  "an EXPORT naming an unbound definition is rejected"
  (doc
    "The export-resolution walk: (export no-such-def) beside a valid main — a compiler resolving exports lazily (or tracing only from main) runs instead of rejecting. rcdzc rejects CDZ0101. The self-hosted front-end had exactly this gap TWICE (single-export b27c59b96, then the multi-export residual d3516ee2c — only the sequence-terminating export was checked); pinned so neither face regresses.")
  (input (do (def (main) 42) (export main) (export no-such-def)))
  (error CDZ0101))

(case
  "an unbound export BEFORE a valid main export is rejected"
  (doc
    "Order-matrix companion of the multi-export pin: the unbound export comes FIRST, then (export main) — a reader that only validates the final export accepts this mirror image. rcdzc rejects CDZ0101.")
  (input (do (def (main) 42) (export no-such-def) (export main)))
  (error CDZ0101))

(case
  "an unbound export after a valid HELPER export is rejected"
  (doc
    "The helper variant of the multi-export face: (export helper) resolves, the LATER (export no-such-def) must still be checked — the self-hosted reader stopped reading after any valid export (gap-A residual). rcdzc rejects CDZ0101.")
  (input (do (def (helper) 1) (def (main) 42) (export helper) (export no-such-def)))
  (error CDZ0101))

(case
  "an unbound PAYLOAD type in a never-constructed type declaration is still rejected"
  (doc
    "The decl-validation walk on an UNUSED type — the direct sibling of the two width-validation gaps in KNOWN_ML_DIFFS (exactly this class): a validator that only checks constructed types runs to 42. rcdzc rejects CDZ0101. The self-hosted front-end had exactly this gap (ran 42) until the Option-C narrow slice (38e7cdca8: uppercase non-builtin undeclared non-nested payload declines; type-vars/declared/forward-refs still accept); pinned so the registry doesn't regress.")
  (input (do (type Unused (Mk NoSuchPayload)) (def (main) 42) (export main)))
  (error CDZ0101))

(case
  "an unbound FIELD type in a never-constructed record type declaration is rejected"
  (doc
    "The record-decl face of the payload-type validation walk (the sum-ctor face is pinned above):
           the field type atom sits one paren level down inside `(Record (: field NoSuchField))`, so a
           validator that only checks top-level payload atoms misses it. rcdzc rejects CDZ0101. The
           self-hosted front-end had exactly this gap — its narrow slice checked flat sum-ctor payloads
           but skipped the record group as nested — until the one-level record descent (1d4aaee7d);
           pinned so the descent doesn't regress. (A record TYPE is the `(Record (: name Type))` form — the
           prior `#record((= : field …))` input was a nativization artifact: `#record(…)` is a record VALUE
           ctor, not a type, so it hit the malformed-variant CDZ0201 before the field-type check; the
           canonical `(Record (: field NoSuchField))` reaches the descent and rejects the unbound field type
           CDZ0101 as this case intends.)")
  (input (do (type R (Record (: field NoSuchField))) (def (main) 42) (export main)))
  (error CDZ0101))

; -- a GENERIC same-name constructor inlined into a caller resolves to the constructor, not the type
; (behavioral migration from rcdzc a_generic_same_name_ctor_in_an_inlined_callee_resolves_to_the_constructor
; + its const-arg and annotated-both companions, adv-63, 2026-08-27). A generic same-name sum `(type Box
; (Box a))` used bare in a helper `(match (Box k) ((Box v) v))` that a caller INLINES: the β-copied head
; `Box` in VALUE position must fire the CONSTRUCTOR (a regression mis-classified it as the type → spurious
; CDZ0203). Monomorphic same-name ctor resolution is pinned above (1540/1557); this is the GENERIC face.
(case
  "a generic same-name constructor inlined into a caller resolves to the constructor not the type"
  (doc
    "`inner` builds `(Box k)` and pops the payload; `main` INLINES `inner`. Three faces: a runtime arg
           `(inner n)`, a const arg `(inner 7)` (fully β-reduced at the call site), and an ANNOTATED
           construct `(: (Box k) (Box Int64))` where `Box` is in BOTH value (ctor) and type positions of one
           β-copied expression. Each must resolve the value-position head to the constructor and run.")
  (input
    (do
      (type Box (Box a))
      (def (inner (: k Int64)) (match (Box k) ((Box v) v)))
      (def (innerann (: k Int64)) (match (: (Box k) (Box Int64)) ((Box v) v)))
      (def (mrt (: n Int64)) (inner n))
      (def (mconst (: _n Int64)) (inner 7))
      (def (mann (: n Int64)) (innerann n))
      (export mrt)
      (export mconst)
      (export mann)))
  (call mrt (: 5 Int64))
  (output (: 5 Int64))
  (call mconst (: 5 Int64))
  (output (: 7 Int64))
  (call mann (: 5 Int64))
  (output (: 5 Int64)))

; -- a parenthesized-head generic type de-dups a repeated head parameter (behavioral migration from rcdzc
; a_parenthesized_head_type_decl_dedups_repeated_head_params, 2026-08-27): a degenerate but well-formed
; head with a repeated param must collect to arity ONE, so the ctor scheme reads the true arity and the
; type resolves + runs (an overcount read a higher-arity scheme and mis-typed).
(case
  "a parenthesized-head generic type de-dups a repeated head parameter to arity one and resolves"
  (doc
    "`(type (Box a a) (Mk a))` repeats the head param `a`; the head-param collect must DE-DUP to one
           param so `(Box Int64)` is a correctly-arity-1 application that resolves by name and runs.
           `(Mk k)` through `(Box Int64)` = k.")
  (input
    (do
      (type (Box a a) (Mk a))
      (def (u (: b (Box Int64))) (match b ((Mk v) v)))
      (def (main (: k Int64)) (u (Mk k)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

; -- a user-generic constructor PATTERN binds and computes through nesting, self-nesting, and a record
; field (behavioral migration from rcdzc a_nested_generic_ctor_pattern_binds_and_computes_through_the_inner_ctor
; + a_record_literal_field_holding_a_user_generic_ctor_projects_and_matches, 2026-08-27): a generic ctor
; pattern `(Mk …)` must resolve the inner binder through both ctor layers — wrapping a built-in `(Some/None)`,
; wrapping ITSELF `(Mk (Mk v))`, and reached by projecting an inline record field holding the generic value.
(case
  "a user-generic constructor pattern binds and computes through nesting, self-nesting, and a record field"
  (input
    (do
      (type (Box a) (Mk a))
      (def (nsome) (match (Mk (Some 5)) ((Mk (Some v)) v) ((Mk (None)) 0)))
      (def (nnone) (match (Mk (None)) ((Mk (Some v)) v) ((Mk (None)) 99)))
      (def (selfnest) (match (Mk (Mk 7)) ((Mk (Mk v)) v)))
      (def (recfield) (match (. #record((= b (Mk 7))) b) ((Mk v) v)))
      (def (recsome) (match (. #record((= b (Mk (Some 5)))) b) ((Mk (Some v)) v) ((Mk (None)) 0)))
      (def (recnone) (match (. #record((= b (Mk (None)))) b) ((Mk (Some v)) v) ((Mk (None)) 0)))
      (export nsome)
      (export nnone)
      (export selfnest)
      (export recfield)
      (export recsome)
      (export recnone)))
  (call nsome)
  (output (: 5 Int64))
  (call nnone)
  (output (: 99 Int64))
  (call selfnest)
  (output (: 7 Int64))
  (call recfield)
  (output (: 7 Int64))
  (call recsome)
  (output (: 5 Int64))
  (call recnone)
  (output (: 0 Int64)))

; ── breaker batch 584: generic monomorphization × heap payload census (07-type-system is
; census-BLIND: 2/193 clauses). A user-generic Box wraps then unwraps a runtime Option-of-list
; payload per frame; the value is exact (75) and the Box+Option shells + the list leak LINEARLY
; (~2.8/frame: 28@n10, 138@n50) — the generic-dispatch face of the sum-shell reclaim family. The
; monomorphized `unbox` extracts the payload but the discarded Box/Option shells are not dropped.
(case
  "gib1 a user-generic Box wrap/unwrap over a runtime heap payload is value-exact and leaks the shells linearly"
  (input
    (do
      (type Box (Box a))
      (def (unbox (: b (Box a))) (match b ((Box.Box v) v)))
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (match
              (unbox (Box.Box (Option.Some (bld (% k 4)))))
              ((Option.Some xs) (List.len xs))
              ((Option.None) -1))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 75 Int64))
  (live-objects known-leak))

; ── breaker batch 585: generic monomorphization at TWO distinct heap domains, TRI-TARGET. This is
; the exact shape the gtx transformer miscompiled (rust E0308, grounded elements to Unit) before
; #4319 — a plain user-generic Box instantiated at (List Int64) AND String in one program. Value
; correct on wasm AND rust AND rust-async (3003 = list-len 3 + string-len 3); leak-free (the unbox
; results feed len/byte-len directly). Tri-target rows: a rust-only monomorphization regression
; on the two-heap-domain path now reds --check.
(case
  "gid1 a user-generic Box at TWO distinct heap domains (List + String) in one program is correct on every backend"
  (input
    (do
      (type Box (Box a))
      (def (unbox (: b (Box a))) (match b ((Box.Box v) v)))
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (main (: n Int64))
        (+
          (* 1000 (List.len (unbox (Box.Box (bld n)))))
          (String.byte-len (unbox (Box.Box (String.concat "ab" (if (> n 0) "c" "")))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3003 Int64)))

; ── breaker batch 586: NESTED generic monomorphization census (Box-of-Pair, both generic, over a
; heap payload — the composition of gib1's Box + a generic Pair). Value exact (75); the nested
; shells leak ~5.5/frame (56@n10, 276@n50 — Box+Pair+tuple+list, deeper than gib1's ~2.8/frame
; single Box). rust correctness verified separately (the two-level unbox/fst extraction is
; value-correct cross-backend). Flips with the sum-shell reclaim.
(case
  "gng1 a NESTED generic Box-of-Pair over a runtime heap payload is value-exact and leaks the nested shells (~5.5/frame)"
  (input
    (do
      (type Box (Box a))
      (type Pair (Pair (Tuple a b)))
      (def (unbox (: x (Box a))) (match x ((Box.Box v) v)))
      (def (fst (: p (Pair a b))) (match p ((Pair.Pair #tuple(x y)) x)))
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (List.len (fst (unbox (Box.Box (Pair.Pair #tuple((bld (% k 4)) k))))))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 75 Int64))
  (live-objects known-leak))

; ── breaker batch 587: element-DISCARDING generic at two heap domains, TRI-TARGET. A Wrap
; rewrapped (payload threaded but never read) then peeked (payload discarded, returns 1),
; instantiated at (List Int64) AND String. This is the discarding-consumer shape adjacent to
; gtx3 (the transformer whose element-discarding single-domain case still declines) — but a plain
; Wrap threads it fine on wasm+rust+rust-async (2). The contrast pins that the gtx3 residual is
; TRANSFORMER-specific (closure-result element grounding), not a general discard-generic gap.
(case
  "gdc1 an element-discarding generic (rewrap + peek) at two heap domains is correct on every backend"
  (input
    (do
      (type Wrap (Wrap a))
      (def (rewrap (: w (Wrap a))) (match w ((Wrap.Wrap v) (Wrap.Wrap v))))
      (def (peek (: w (Wrap a))) (match w ((Wrap.Wrap _) 1)))
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (main (: n Int64))
        (+
          (peek (rewrap (Wrap.Wrap (bld n))))
          (peek (rewrap (Wrap.Wrap (String.concat "x" (if (> n 0) "y" "z")))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2 Int64)))

; ── breaker batch 596: exhaustiveness-with-GUARDS soundness (a guarded arm does NOT count as
; covering its variant — a failed guard leaves a gap). gex1: Some covered ONLY by a guarded arm
; (+ a None arm) is NON-EXHAUSTIVE, rejected CDZ0210. gex2: adding an UNGUARDED Some fall-through
; makes it exhaustive, and the guard-first arm ordering is honored (n=5 guard fails -> unguarded
; -> 5; n=200 guard holds -> *10 -> 2000). Pins that guards can NARROW an arm but never SATISFY
; exhaustiveness — a compiler that let gex1 compile would trap-or-fall-through at runtime.
(case
  "gex1 a sum variant covered ONLY by a guarded arm is non-exhaustive (rejected CDZ0210)"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (if (> n 0) (Option.Some n) (Option.None))
          ((guard (Option.Some v) (> v 100)) (* v 10))
          ((Option.None) -1)))
      (export main)))
  (error CDZ0210))

(case
  "gex2 a guarded arm plus an unguarded fall-through for the same variant IS exhaustive (guard-first ordering honored)"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (if (> n 0) (Option.Some n) (Option.None))
          ((guard (Option.Some v) (> v 100)) (* v 10))
          ((Option.Some v) v)
          ((Option.None) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 200 Int64))
  (output (: 2000 Int64)))

; ── let-binder annotation type-validation (migrated from rcdzc an_unknown_type_in_a_let_binder_annotation_is_rejected) ──
; A `(let (((: x T) v)) …)` binder annotation `T` is VALIDATED (it was once silently accepted — an
; unresolvable T "agreed" vacuously and typed x as Any). The reject half is backend-agnostic (compile-time),
; so it lives here; the KNOWN-type-mismatch M63 coercion-fix assertion (a structured .fix) stays as a
; corpus-inexpressible white-box pin in rcdzc.
(case
  "an unknown type in a let-binder annotation is rejected"
  (input (do (def (main) (let (((: x Nonesuch) 5)) x)) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "an unknown type nested in a let-binder List annotation is rejected"
  (input (do (def (main) (let (((: x (List Nonesuch)) #list(1))) x)) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "a well-formed non-type in a let-binder annotation is rejected"
  (input (do (def (main) (let (((: x 5) 3)) x)) (export main)))
  (error CDZ0203))

(case
  "a known-type let-binder annotation compiles and the binding reads back"
  (input (do (def (main) (let (((: x Int64) 5)) x)) (export main)))
  (call main)
  (output (: 5 Int64)))

; ── an unknown type in a RECORD-type annotation FIELD names the type, never the field LABEL ──
;    (migrated from rcdzc an_unknown_type_in_a_record_parameter_annotation_names_only_the_type_not_the_field_label)
; A record-type annotation `(Record (: x T) …)` validates each field's TYPE `T` but must NOT treat the
; field LABEL (`x`) as a value name. A bad field type is CDZ0101 naming the unknown type (`Nonesuch`),
; and the label is NEVER reported "unbound name `x`". This holds across every annotation SITE — a
; parameter annotation, a value annotation `(: value T)`, and a let-binder annotation — which share the
; record-aware type-position validator. Before, a naive value-`collect` fallback mis-resolved the label
; as an unbound value name alongside the real fault.
(case
  "an unknown type in a record parameter annotation names the type, not the field label"
  (input (do (def (g (: r (Record (: x Nonesuch)))) r) (export g)))
  (error CDZ0101 (message "Nonesuch") (not "unbound name `x`")))

(case
  "an unknown type in a NESTED record parameter annotation field names only the deep type"
  (input (do (def (g (: r (Record (: a (Record (: b Nonesuch)))))) r) (export g)))
  (error CDZ0101 (message "Nonesuch") (not "unbound name `a`") (not "unbound name `b`")))

(case
  "an unknown type in a record VALUE annotation names the type, not the field label"
  (input (do (def (main) (: 5 (Record (: x Nonesuch)))) (export main)))
  (error CDZ0101 (message "Nonesuch") (not "unbound name `x`")))

(case
  "an unknown type in a record LET-BINDER annotation names the type, not the field label"
  (input (do (def (main) (let (((: r (Record (: x Nonesuch))) #record((= x 5)))) r)) (export main)))
  (error CDZ0101 (message "Nonesuch") (not "unbound name `x`")))

(case
  "a well-formed record parameter annotation compiles and the field reads back"
  (input
    (do (def (g (: r (Record (: x Int64)))) r.x) (def (main) (g #record((= x 7)))) (export main)))
  (call main)
  (output (: 7 Int64)))

; ── let-binder annotation MISMATCH offers the SAME coercion fix as a value annotation / argument position
;    (migrated from rcdzc an_int_let_binder_annotation_mismatch_offers_an_of_conversion_fix +
;    a_known_type_let_binder_mismatch_keeps_its_coercion_fix) ──
; A `(let (((: x T) init)) …)` whose annotation T mismatches the INIT value's type is CDZ0203 ("a binder
; annotated T is bound to a value of type U") and carries the SAME numeric/text/sum coercion fix the value
; annotation `(: value T)` and the argument position give (the D33 lesson — one repair fires wherever the
; mismatch surfaces). No coercion (a Bool into Int64) → the bare reject, no fix.
(case
  "an int-width let-binder annotation mismatch offers the of-conversion wrap fix"
  (input (do (def (f (: n Int8)) (let (((: x Int64) n)) x)) (export f)))
  (error
    CDZ0203
    (message "value of type Int8")
    (fix (kind wrap) (replacement-contains "(Int64.of "))))

(case
  "a Bool let-binder init annotated Int64 has no coercion and carries no fix"
  (input (do (def (f) (let (((: x Int64) true)) x)) (export f)))
  (error CDZ0203 (message "value of type Bool") (no-fix)))

(case
  "an int LITERAL let-binder init annotated Float retypes to a float literal"
  (input (do (def (f) (let (((: x Float64) 3)) x)) (export f)))
  (error CDZ0203 (message "annotated Float64") (fix (kind replace) (replacement "3.0"))))

(case
  "a NON-literal int let-binder init annotated Float wraps in of-int"
  (input (do (def (f (: n Int64)) (let (((: x Float64) n)) x)) (export f)))
  (error CDZ0203 (fix (kind wrap) (replacement-contains "(Float64.of-int "))))

(case
  "an integer-valued float LITERAL let-binder init annotated Int drops the fraction"
  (input (do (def (f) (let (((: x Int64) 3.0)) x)) (export f)))
  (error CDZ0203 (message "value of type Float64") (fix (kind replace) (replacement "3"))))

; CONTEXT-AWARE suggestion in TYPE position: the type slot of an annotation `(: v T)` is a type expression,
; so only a TYPE name could be meant. The candidate pool DROPS non-type kinds (value defs, lexical binders,
; variant ctors) — suggesting one would fail the one-shot rule (`(: 5 flag)` → "annotation requires a type").
; So a value def `flag` one edit from a typo'd `flg` is NOT suggested (the diagnostic stays the honest plain
; unbound), while a real user/prelude TYPE one edit away (`Widgett`→`Widget`, `Booll`→`Bool`) IS suggested
; with an applyable rename fix. (Migrated from rcdzc a_type_position_typo_does_not_suggest_a_nearer_value +
; a_type_position_typo_still_suggests_a_real_type.)
(case
  "a type-position typo does not suggest a nearer VALUE def"
  (input (do (def (flag) true) (def (main) (: 5 flg)) (export main)))
  (error CDZ0101 (message "unbound") (not "flag")))

(case
  "a type-position typo still suggests a real user type with a rename fix"
  (input (do (type Widget (W Int64)) (def (main) (: 5 Widgett)) (export main)))
  (error
    CDZ0101
    (message "did you mean `Widget`?")
    (fix (kind replace) (replacement "Widget") (unverified))))

(case
  "a type-position typo still suggests a prelude type with a rename fix"
  (input (do (def (main) (: 5 Booll)) (export main)))
  (error
    CDZ0101
    (message "did you mean `Bool`?")
    (fix (kind replace) (replacement "Bool") (unverified))))

(case
  "a String let-binder init annotated Bytes wraps in to-bytes"
  (input (do (def (f (: s String)) (let (((: x Bytes) s)) x)) (export f)))
  (error
    CDZ0203
    (message "value of type String")
    (fix (kind wrap) (replacement-contains "(String.to-bytes "))))

(case
  "a payload-typed let-binder init annotated its sum wraps in the variant constructor"
  (input (do (def (f) (let (((: x (Option Int64)) 5)) x)) (export f)))
  (error
    CDZ0203
    (message "annotated (Option Int64)")
    (fix (kind wrap) (replacement-contains "(Some "))))

; ── malformed VARIANT POSITION in a type declaration (migrated from rcdzc
;    a_malformed_variant_position_in_a_type_declaration_is_rejected) ──
; A `(type …)` tail element is a VARIANT: a bare NAME (`Red`) or a `(Name payload…)` form. Anything else —
; a bare literal `(type T 5)`, a list headed by a non-name `(type T (5 Int64))`, an empty list `()`, a
; string — was SILENTLY DROPPED, so `(type T Red 5 Blue)` became the two-variant `{Red,Blue}` with `5`
; invisibly gone and a match on Red/Blue then wrongly type-checked as EXHAUSTIVE (a silent correctness
; hazard). Each malformed variant position now rejects CDZ0201 ("a variant … must be a name").
(case
  "a bare literal in a variant position of a type declaration is rejected"
  (input (do (type T 5) (def (main) 0) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "a literal amid valid variants is rejected, not silently dropped"
  (input (do (type T Red 5 Blue) (def (main) 0) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "a variant form headed by a non-name is rejected"
  (input (do (type T (5 Int64)) (def (main) 0) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "an empty list in a variant position is rejected"
  (input (do (type T ()) (def (main) 0) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "a string literal in a variant position is rejected"
  (input (do (type T (Red) "str") (def (main) 0) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "the dropped-variant exhaustiveness hazard is rejected at the declaration"
  (doc
    "`(type T Red 5 Blue)` with a match on Red/Blue no longer compiles as though exhaustive — the
           malformed `5` is a hard CDZ0201 at the declaration, closing the silent-drop → false-exhaustive
           correctness hazard.")
  (input
    (do (type T Red 5 Blue) (def (main) (match (T.Red) ((T.Red) 10) ((T.Blue) 20))) (export main)))
  (error CDZ0201 (message "must be a name")))

(case
  "valid variant shapes (nullary / generic / record payload) do not false-positive as malformed"
  (input
    (do
      (type Color Red Green Blue)
      (type Opt (Sm a) Nn)
      (type C (Mk (Record (: x Int64))))
      (def (main) 0)
      (export main)))
  (call main)
  (output (: 0 Int64)))

; ── variant-payload + effect-op type-position validation (migrated from rcdzc
;    an_unknown_type_in_a_variant_payload_is_rejected + an_unknown_type_in_an_effect_operation_type_is_rejected) ──
; The declaration-site record-aware type-position walk (validate_type_position) also guards sum-variant
; PAYLOADS and effect-operation arrow types — the same check as the let-binder annotation sites above. An
; unknown type in a payload/op position is CDZ0101 (naming the missing type + the `(type Nonesuch …)` declare
; fix); a well-formed non-type payload is CDZ0203. The reject+message halves are backend-agnostic (compile-time),
; so they live here; the structured did-you-mean enrichment on a NEAR typo stays a white-box rcdzc pin.
(case
  "an unknown type in a variant payload is rejected at the declaration"
  (input (do (type C (A Nonesuch)) (def (main) 0) (export main)))
  (error CDZ0101 (message "unknown type `Nonesuch`") (message "(type Nonesuch …)")))

(case
  "an unknown type nested in a List variant payload is rejected"
  (input (do (type C (A (List Nonesuch))) (def (main) 0) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "an unknown type in a record field inside a variant payload is rejected"
  (input (do (type Box (B (Record (: val Nonesuch))) N) (def (main) 0) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "a well-formed non-type in a variant payload is rejected CDZ0203"
  (input (do (type C (A 5)) (def (main) 0) (export main)))
  (error CDZ0203))

(case
  "a near typo of a real type in a variant payload keeps its did-you-mean"
  (input (do (type C (A Strng)) (def (main) 0) (export main)))
  (error CDZ0101 (message "did you mean `String`?")))

(case
  "a parametric variant payload does not false-positive as an unknown type"
  (input (do (type Opt (Some a) (Non)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a self-recursive variant payload does not false-positive as an unknown type"
  (input (do (type T (Nil) (Cons Int64 T)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "mutually-recursive variant payloads do not false-positive as unknown types"
  (input (do (type A (MkA B)) (type B (MkB A)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a record variant payload mentioning a type param does not false-positive"
  (input (do (type Box (B (Record (: v (Option a)))) N) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; ── variant-CONSTRUCTOR-APPLICATION payload VALUE mismatch offers the SAME coercion fix as an argument
;    position (migrated from rcdzc a_wrong_type_constructor_payload_offers_the_same_coercion_fix_as_an_argument) ──
; Distinct from the declaration-site payload TYPE-position validation above: here the payload type is well-formed
; and the fault is a wrong-typed VALUE APPLIED to the constructor — CDZ0201 "a variant constructor's payload has
; declared type T, but a value of type U was applied". It carries the same numeric/text coercion fix the argument
; position gives: an int-width source `(Int64.of …)` (wrap), an int-valued-float `3.0`→`3` (drop the fraction,
; replace), a `String`→`Bytes` `(String.to-bytes …)` (wrap). A source with NO coercion to the declared type (a
; Bool into Int64) carries the bare CDZ0201 with no fix — a false suggestion is worse than none.
(case
  "a variant constructor int-width payload mismatch offers the of-conversion wrap fix"
  (input (do (type P (Mk Int64)) (def (f (: a Int8)) (Mk a)) (export f)))
  (error CDZ0201 (message "Int8 was applied") (fix (kind wrap) (replacement-contains "(Int64.of "))))

(case
  "a variant constructor int-valued-float payload mismatch offers the drop-the-fraction fix"
  (input (do (type P (Mk Int64)) (def (f) (Mk 3.0)) (export f)))
  (error CDZ0201 (message "Float64 was applied") (fix (kind replace) (replacement "3"))))

(case
  "a variant constructor String-into-Bytes payload mismatch offers the to-bytes wrap fix"
  (input (do (type P (Mk Bytes)) (def (f (: s String)) (Mk s)) (export f)))
  (error
    CDZ0201
    (message "String was applied")
    (fix (kind wrap) (replacement-contains "(String.to-bytes "))))

(case
  "a variant constructor payload mismatch with no coercion carries the bare reject and no fix"
  (input (do (type P (Mk Int64)) (def (f) (Mk true)) (export f)))
  (error CDZ0201 (message "Bool was applied") (no-fix)))

(case
  "an unknown type in an effect operation arg is rejected at the declaration"
  (input (do (effect E (op e (-> Nonesuch Unit))) (def (main) 0) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "an unknown type in an effect operation result is rejected"
  (input (do (effect E (op e (-> Unit Nonesuch))) (def (main) 0) (export main)))
  (error CDZ0101 (message "Nonesuch")))

(case
  "an unknown type nested in an effect operation List arg is rejected"
  (input (do (effect E (op e (-> (List Zzz) Unit))) (def (main) 0) (export main)))
  (error CDZ0101 (message "Zzz")))

(case
  "a type-variable effect operation type does not false-positive as an unknown type"
  (input (do (effect E (op e (-> a a))) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a known generic effect operation type does not false-positive as an unknown type"
  (input (do (effect E (op e (-> (Option Int64) Unit))) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; (migrated from rcdzc a_wrong_typed_option_field_in_a_direct_record_arg_still_rejects — a soundness guard:
;  direct-arg reflection freshening must stop the shared-unsolved-var false reject WITHOUT masking a genuine
;  field-type mismatch. The reject is backend-agnostic; the freshening internals stay in rcdzc.)
(case
  "a wrong-typed Option field in a direct record arg still rejects"
  (input
    (do
      (type Outcome (Ok Int64) (Err Int64))
      (def (apply (: evt (Record (: b (Option Outcome)) (: c Int64)))) evt.c)
      (def (main) (apply #record((= b (Some 5)) (= c 9))))
      (export main)))
  (error CDZ0203))

; --- A type mismatch at an argument/annotation names the specific DELTA, not two full renders ----
; When a compound value is passed where a differently-shaped compound is expected, the diagnostic names the
; SPECIFIC differing facet — a tuple's arity delta or its differing element position, a collection's differing
; AXIS (element / key / value) with expected-vs-actual — rather than dumping both full type renders for the
; reader to diff. All CDZ0203. (Migrated from rcdzc a_tuple_arity_mismatch_names_the_element_counts +
; a_collection_element_mismatch_names_the_differing_axis.)
(case
  "a tuple passed with too FEW elements names the arity delta"
  (input
    (do (def (h (: t (Tuple Int64 Int64 Int64))) (. t 0)) (def (g) (h #tuple(1 2))) (export g)))
  (error CDZ0203 (message "expected a tuple with 3 elements, but this one has 2")))

(case
  "a tuple passed with too MANY elements names the arity delta the other direction"
  (input (do (def (h (: t (Tuple Int64 Int64))) (. t 0)) (def (g) (h #tuple(1 2 3))) (export g)))
  (error CDZ0203 (message "expected a tuple with 2 elements, but this one has 3")))

(case
  "a same-arity tuple element-type mismatch names the specific position, not an arity delta"
  (input (do (def (h (: t (Tuple Int64 Bool))) (. t 0)) (def (g) (h #tuple(1 2))) (export g)))
  (error
    CDZ0203
    (message "element 1 should be Bool, but this one is Int64")
    (not "expected a tuple with")))

(case
  "a list element-type mismatch names the element axis and offers no mechanical fix"
  (input (do (def (h (: xs (List Int64))) xs) (def (g) (h #list(true))) (export g)))
  (error CDZ0203 (message "its elements should be Int64, but these are Bool") (no-fix)))

(case
  "a map KEY-type mismatch names the key axis"
  (input (do (def (h (: mp (Map String Int64))) mp) (def (g) (h #map((= 1 2)))) (export g)))
  (error CDZ0203 (message "its keys should be String, but these are Int64")))

(case
  "a map VALUE-type mismatch names the value axis"
  (input (do (def (h (: mp (Map Int64 Int64))) mp) (def (g) (h #map((= 1 true)))) (export g)))
  (error CDZ0203 (message "its values should be Int64, but these are Bool")))

(case
  "a map with BOTH axes wrong reports the leftmost (key) axis deterministically"
  (input (do (def (h (: mp (Map String Int64))) mp) (def (g) (h #map((= 1 true)))) (export g)))
  (error CDZ0203 (message "its keys should be String") (not "its values should be")))

; The record + function analogues of the tuple/collection delta hints above. A same-field-set record whose
; field TYPE differs names the specific field ("field `x` should be Int64, but this one is Bool") and is NOT
; reported as a field-SET difference (nothing is missing/extra). Two curried function types name the specific
; difference — a RESULT-type mismatch or an ARITY mismatch — rather than two full arrow renders; a same-arity
; PARAMETER difference instead resolves at the inner argument position (no fn-signature tail). All CDZ0203.
; (Migrated from rcdzc a_record_type_mismatch_is_not_reported_as_a_field_set_difference +
; a_function_type_mismatch_names_the_differing_result_or_arity.)
(case
  "a same-field-set record with a differing field type names the field, not a field-set difference"
  (input (do (def (h (: p (Record (: x Int64)))) p.x) (def (g) (h #record((= x true)))) (export g)))
  (error
    CDZ0203
    (message "field `x` should be Int64, but this one is Bool")
    (not "missing field")
    (not "no such field")))

(case
  "a function argument with the wrong RESULT type names the result axis"
  (input
    (do
      (def (k (: f (-> Int64 Bool))) (f 1))
      (def (bad (: x Int64)) x)
      (def (g) (k bad))
      (export g)))
  (error CDZ0203 (message "its result should be Bool, but this one returns Int64")))

(case
  "a function argument of the wrong ARITY names the argument-count delta"
  (input
    (do
      (def (k (: f (-> Int64 Int64))) (f 1))
      (def (bad (: x Int64) (: y Int64)) x)
      (def (g) (k bad))
      (export g)))
  (error CDZ0203 (message "expected a function taking 1 argument, but this one takes 2")))

(case
  "a same-arity function PARAMETER difference resolves at the inner argument, no fn-signature tail"
  (input
    (do
      (def (k (: f (-> Int64 Int64))) (f 1))
      (def (bad (: x Bool)) 0)
      (def (g) (k bad))
      (export g)))
  (error
    CDZ0203
    (message "argument")
    (not "its result should be")
    (not "expected a function taking")))

(case
  "the function-signature delta hint also fires at a value-annotation site"
  (input (do (def (bad (: x Int64)) x) (def (g) (: bad (-> Int64 Bool))) (export g)))
  (error CDZ0203 (message "its result should be Bool, but this one returns Int64")))

(case
  "two IDENTICAL function types produce no fault (function argument type-checks clean)"
  (input
    (do
      (def (k (: f (-> Int64 Int64))) (f 1))
      (def (good (: x Int64)) x)
      (def (g) (k good))
      (export g)))
  (output (: 1 Int64)))

; --- Value-vs-sum / operator-arg mismatch: readable lead + wrap fix + "match it" hint --------------
; When a value appears where a SUM (Option/user sum) is expected, or two same-kind compounds are compared in
; an OPERATOR-argument position, the diagnostic reads at the argument site (naming the expected type / the
; structural delta) rather than leaking the raw naive-HM "type mismatch: A and B must be the same type here".
; A value that IS a sum's payload carries a "wrap in `(Ctor …)`" fix (the ctor derived from the DECLARATION,
; no-keys-outside-the-prelude); a sum with no fitting variant carries none. The INVERSE — an `(Option T)` used
; where the bare payload `T` is expected — has no total unwrap, so no mechanical fix, but the message says to
; MATCH it. (Migrated from rcdzc an_operator_arg_wrap_in_variant_uses_the_readable_lead_not_the_raw_unify_message
; + an_operator_arg_structural_mismatch_names_the_delta_not_the_raw_unify_message +
; the_wrap_variant_is_derived_generically_from_the_user_sum_not_hardcoded +
; an_annotation_mismatch_with_no_fitting_variant_carries_no_wrap + using_an_option_where_its_payload_is_expected_says_to_match_it.)
(case
  "comparing an Option to its payload reads at the argument site and carries a wrap-in-Some fix"
  (input (do (def (f (: o (Option Int64))) (= o 5)) (def (main) (f (Some 1))) (export main)))
  (error
    CDZ0203
    (message "this argument is an Int64, but a value of type (Option Int64) is expected here")
    (not "must be the same type here")
    (fix (kind wrap) (replacement "(Some …)"))))

(case
  "an operator-arg record field-SET mismatch names the delta, not the raw unify message"
  (input (do (def (main) (= #record((= x 1)) #record((= y 2)))) (export main)))
  (error CDZ0203 (message "missing field `x`") (not "must be the same type here")))

(case
  "an operator-arg record field-TYPE mismatch names the differing field's types"
  (input (do (def (main) (= #record((= x 1)) #record((= x true)))) (export main)))
  (error CDZ0203 (message "field `x` should be Int64, but this one is Bool")))

(case
  "an operator-arg tuple ARITY mismatch names the element counts"
  (input (do (def (main) (= #tuple(1 2) #tuple(1 2 3))) (export main)))
  (error CDZ0203 (message "expected a tuple with 2 elements, but this one has 3")))

(case
  "the wrap-in-variant fix's constructor is derived generically from the user sum, not hardcoded"
  (input (do (type Box (Wrap Int64)) (def (f (: n Int64)) (: n Box)) (export f)))
  (error CDZ0203 (fix (kind wrap) (replacement "(Wrap …)"))))

(case
  "a mismatch against a sum with no fitting variant carries no wrap suggestion"
  (input (do (type Flag On Off) (def (f (: n Int64)) (: n Flag)) (export f)))
  (error CDZ0203 (message "Flag") (not "wrap") (no-fix)))

(case
  "using an Option where its bare payload is expected says to match it, with no mechanical fix"
  (input (do (def (h (: n Int64)) n) (def (g (: o (Option Int64))) (h o)) (export g)))
  (error
    CDZ0203
    (message "the value is optional")
    (message "match it")
    (message "(Some x)")
    (no-fix)))

(case
  "the match-it hint also fires at a binop unify site over an optional read"
  (input (do (def (g (: xs (List Int64))) (+ (List.at xs 0) 1)) (export g)))
  (error CDZ0203 (message "the value is optional") (message "match it")))

(case
  "an unrelated-payload Option mismatch gets no match-it hint"
  (input (do (def (h (: b Bool)) b) (def (g (: o (Option Int64))) (h o)) (export g)))
  (error CDZ0203 (message "this argument") (message "expected here") (not "the value is optional")))

; --- The per-member structural-delta hints ALSO fire at JOIN sites (list literal / if branches / match arms) ---
; The same field / element-position / arity deltas above surface where two same-kind compounds meet at a join,
; instead of dumping two whole renders. A LIST literal join is a homogeneity fault (CDZ0201); an `if`/`match`
; branch join is a type mismatch (CDZ0203). A scalar clash at a join keeps its clean message (no delta tail) and
; its int-literal->float retype fix — that no-delta control stays a small rcdzc residual (its fix-only quality
; grade is a todo). (Migrated from rcdzc a_join_site_names_the_structural_delta_not_two_full_renders.)
(case
  "a list literal of records differing in one field type names the differing field"
  (input (do (def (g) #list(#record((= x 1)) #record((= x true)))) (export g)))
  (error CDZ0201 (message "field `x` should be Int64, but this one is Bool")))

(case
  "an if whose branch records differ in one field type names the differing field"
  (input (do (def (f (: b Bool)) (if b #record((= x 1)) #record((= x true)))) (export f)))
  (error CDZ0203 (message "field `x` should be Int64, but this one is Bool")))

(case
  "a match whose arm tuples differ in one position names the differing position"
  (input (do (def (f (: n Int64)) (match n (0 #tuple(1 2)) (_ #tuple(1 true)))) (export f)))
  (error CDZ0203 (message "element 1 should be Int64, but this one is Bool")))

(case
  "a list literal of tuples of different arity names the arity delta at the join"
  (input (do (def (g) #list(#tuple(1 2) #tuple(1 2 3))) (export g)))
  (error CDZ0201 (message "expected a tuple with 2 elements, but this one has 3")))

; The `if`-branch twins of the match-arm/list-literal joins above: two tuple branches of an `if` that
; disagree in ARITY or in one ELEMENT TYPE are a branch-join type mismatch (CDZ0203, not the CDZ0201
; homogeneity fault a LIST literal raises). The coarse "if branches differ" lead carries the SAME
; per-member structural delta the match-arm case names — an arity delta or an element-position delta —
; not two whole tuple renders. A cross-KIND disagreement (tuple vs scalar) is CDZ0203; two DISTINCT
; NUMERIC branches would instead be CDZ0201 (no silent promotion). (Migrated from rcdzc
; tuple_branches_of_different_arity_are_a_type_error + tuple_branches_of_different_element_type_are_a_type_error.)
(case
  "an if whose branch tuples differ in arity is a CDZ0203 branch-join mismatch naming the arity delta"
  (input (do (def (f (: b Bool)) (if b #tuple(1 2) #tuple(3 4 5))) (export f)))
  (error
    CDZ0203
    (message "if branches differ")
    (message "expected a tuple with 2 elements, but this one has 3")))

(case
  "an if whose branch tuples differ in one element type names the differing position at the join"
  (input (do (def (f (: b Bool)) (if b #tuple(1 2) #tuple(1 true))) (export f)))
  (error
    CDZ0203
    (message "if branches differ")
    (message "element 1 should be Int64, but this one is Bool")))

; Over-applying a bare variant CONSTRUCTOR names it + its arity (like the prelude-member-op over-application),
; not an anonymous "applied N arguments to a function of arity M". The constructor is named bare (`Mk`) or
; dotted at the member-access spelling (`P.Mk`), and the reject carries a delete-surplus fix. An ordinary
; over-applied user function keeps the anonymous arity message (it is not a constructor). (Migrated from rcdzc
; over_applying_a_bare_variant_constructor_names_it.)
(case
  "over-applying a bare variant constructor names the constructor and its arity"
  (input (do (type P (Mk Int64 Int64) (Z)) (def (g) (Mk 1 2 3)) (export g)))
  (error CDZ0203 (message "`Mk` takes 2 arguments, but 3 were given") (fix (kind delete))))

(case
  "the member-access spelling of an over-applied constructor names it dotted"
  (input (do (type P (Mk Int64 Int64) (Z)) (def (g) (P.Mk 1 2 3)) (export g)))
  (error CDZ0203 (message "`P.Mk` takes 2 arguments, but 3 were given")))

(case
  "an ordinary over-applied user function keeps the anonymous arity message, not a constructor phrasing"
  (input (do (def (h (: a Int64)) a) (def (g) (h 1 2)) (export g)))
  (error CDZ0203 (message "function of arity 1") (not "were given")))

; An UNAPPLIED / partially-applied function used where a non-function value is expected has type `(-> …)`; the
; generic "type mismatch: Int64 and (-> …)" never says the value is simply a function you FORGOT to call. Both
; the annotation/arg site and the binop-unify site append "hasn't been fully applied; apply it to N more
; argument(s)" (rustc's "you might have forgotten to call this function"), with a polished function-value lead
; at an operator arg (not the raw internal-clash unify wording). No mechanical fix (the missing arg values are
; unknown). The hint fires ONLY when applying the remaining args would yield the expected type — an applied
; result that still differs, or a fn-vs-fn mismatch, keeps the plain message. (Migrated from rcdzc
; an_unapplied_function_value_names_the_forgotten_call.)
(case
  "a partial application where a scalar is expected names the forgotten call"
  (input
    (do
      (def (h (: a Int64) (: b Int64)) (+ a b))
      (def (g (: x Int64)) x)
      (def (main) (g (h 1)))
      (export main)))
  (error
    CDZ0203
    (message "hasn't been fully applied; apply it to 1 more argument to get an Int64")
    (no-fix)))

(case
  "a partial application as an operator operand names the forgotten call with a polished function-value lead"
  (input (do (def (h (: a Int64) (: b Int64)) (+ a b)) (def (g) (+ (h 1) 2)) (export g)))
  (error
    CDZ0203
    (message "hasn't been fully applied; apply it to 1 more argument")
    (message "this operation is not defined on a function value")
    (not "must be the same type here")))

(case
  "a partial application still needing two arguments pluralizes the count"
  (input
    (do
      (def (h (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
      (def (g) (: (h 1) Int64))
      (export g)))
  (error CDZ0203 (message "apply it to 2 more arguments")))

(case
  "no forgotten-call hint when the fully-applied result would still differ from the expected type"
  (input (do (def (h (: a Int64) (: b Int64)) (+ a b)) (def (g) (: (h 1) Bool)) (export g)))
  (error CDZ0203 (message "Bool") (not "hasn't been fully applied")))

(case
  "no forgotten-call hint when the expected type is itself a function (fn-vs-fn mismatch)"
  (input
    (do
      (def (apply1 (: f (-> Int64 Int64)) (: x Int64)) (f x))
      (def (h (: a Int64) (: b Int64)) (+ a b))
      (def (g) (apply1 h 5))
      (export g)))
  (error
    CDZ0203
    (message "expected a function taking 1 argument, but this one takes 2")
    (not "hasn't been fully applied")))

; A wrong-type argument to a named prelude MEMBER OP names the OPERATION + its expected/actual types (like the
; effect-op perform message), not the generic symmetric unify clash. A List op's element disagreement is a
; malformed collection (CDZ0201) but keeps the same phrasing; a conversion op (`Int64.of`) is CDZ0203. A bare
; operator (`+`) is not a `.`-member head, so it keeps the generic message. A same-kind compound arg that
; differs structurally appends the field/element delta. Over-applying a member op names the op + its arity +
; a delete-surplus fix. (Migrated from rcdzc a_wrong_type_argument_to_a_prelude_member_op_names_the_operation
; + over_applying_a_prelude_member_op_names_the_operation_and_arity.)
(case
  "a wrong-element-type List.push names the operation and its expected/actual types"
  (input (do (def (g (: xs (List Int64))) (List.push xs true)) (export g)))
  (error CDZ0201 (message "`List.push` expects an argument of type Int64") (message "Bool")))

(case
  "a wrong-type argument to a conversion op names the operation"
  (input (do (def (g (: s String)) (Int64.of s)) (export g)))
  (error CDZ0203 (message "`Int64.of` expects an argument of type Int64")))

(case
  "a bare operator keeps the generic mismatch message, not a member-op phrasing"
  (input (do (def (g) (+ 1 true)) (export g)))
  (error CDZ0203 (message "Bool") (not "expects an argument of type")))

(case
  "a structurally-mismatched List.push element names the operation and the field-level delta"
  (input
    (do (def (g (: xs (List (Record (: x Int64))))) (List.push xs #record((= y 2)))) (export g)))
  (error CDZ0201 (message "`List.push` expects an argument of type") (message "field `x`")))

(case
  "over-applying a member op names the operation and its arity with a delete-surplus fix"
  (input (do (def (g (: xs (List Int64))) (List.push xs 1 2)) (export g)))
  (error CDZ0203 (message "`List.push` takes 2 arguments, but 3 were given") (fix (kind delete))))

(case
  "over-applying an arity-1 member op uses the singular argument"
  (input (do (def (main) (Map.len #map((= 1 2)) 99)) (export main)))
  (error CDZ0203 (message "`Map.len` takes 1 argument, but 2 were given")))

; Two DIFFERENT types that render with the SAME name (a user `(type Int64 …)` shadowing the prelude) get a
; disambiguating tail — "two DIFFERENT types printed with the same name … shadows a built-in" — so the
; message doesn't read as a contradiction ("an Int64 where an Int64 is expected"). An ordinary distinct-name
; mismatch (Int64 vs String) adds no such tail. (Migrated from rcdzc
; a_mismatch_between_two_same_named_distinct_types_disambiguates_the_shared_name.)
(case
  "a mismatch between two same-named distinct types disambiguates the shared name (argument site)"
  (input (do (type Int64 (A)) (def (f (: x Int64)) x) (def (main) (f 5)) (export main)))
  (error
    CDZ0203
    (message "two DIFFERENT types printed with the same name")
    (message "shadows a built-in")))

(case
  "the same-name disambiguation also fires at a value annotation"
  (input (do (type Int64 (A)) (def (main) (: 5 Int64)) (export main)))
  (error CDZ0203 (message "two DIFFERENT types printed with the same name")))

(case
  "an ordinary distinct-name mismatch adds no same-name disambiguation tail"
  (input (do (def (f (: x Int64)) x) (def (main) (f "s")) (export main)))
  (error CDZ0203 (message "String") (not "two DIFFERENT types printed with the same name")))

; An UNSOLVED type variable in a rendered type renders as `_` (rustc's placeholder for an unknown type), NOT
; the internal solver-assigned `?{n}` (a nondeterministic number that reads as a naive-HM leak). Checked at the
; sites an unsolved var reaches a user message: a list-element clash, a call argument, an if-branch join — the
; error type of a bare `(Ok 2)` is `(Result Int64 _)` (inference never pins the Err payload). (Migrated from
; rcdzc an_unsolved_type_variable_renders_as_underscore_not_an_internal_number.)
(case
  "an unsolved type variable renders as underscore in a list-element clash"
  (input (do (def (g) #list((Some 1) (Ok 2))) (export g)))
  (error CDZ0201 (message "(Result Int64 _)") (not "?")))

(case
  "an unsolved type variable renders as underscore in a call-argument mismatch"
  (input (do (def (g (: o (Option Int64))) o) (def (main) (g (Ok 2))) (export main)))
  (error CDZ0203 (message "(Result Int64 _)") (not "?")))

(case
  "an unsolved type variable renders as underscore in an if-branch join"
  (input (do (def (f (: b Bool)) (if b (Some 1) (Ok 2))) (export f)))
  (error CDZ0203 (message "(Result Int64 _)") (not "?")))

; When a differing record field / tuple position is itself a same-shape nested compound, the delta hint DRILLS
; through the shared structure to the deepest SCALAR leaf and names the dotted access PATH ("field `a.b.c`
; should be Int64, but this one is Bool") — a record field contributes its name, a tuple position its 0-based
; index — instead of re-rendering the whole sub-compound. The drill STOPS at a field-SET difference deeper
; down (naming the immediate field, not a misleading leaf path). (Migrated from rcdzc
; a_nested_compound_mismatch_drills_to_the_exact_leaf_path.)
(case
  "a two-level nested record mismatch drills to the dotted leaf path"
  (input
    (do
      (def (h (: r (Record (: inner (Record (: x Int64)))))) r.inner)
      (def (g) (h #record((= inner #record((= x true))))))
      (export g)))
  (error CDZ0203 (message "field `inner.x` should be Int64, but this one is Bool")))

(case
  "a three-level nested record mismatch grows the dotted path"
  (input
    (do
      (def (h (: r (Record (: a (Record (: b (Record (: c Int64)))))))) r.a)
      (def (g) (h #record((= a #record((= b #record((= c true))))))))
      (export g)))
  (error CDZ0203 (message "field `a.b.c` should be Int64, but this one is Bool")))

(case
  "a nested-path mismatch mixes a record field name and a tuple index"
  (input
    (do
      (def (h (: r (Record (: pt (Tuple Int64 Int64))))) r.pt)
      (def (g) (h #record((= pt #tuple(1 true)))))
      (export g)))
  (error CDZ0203 (message "field `pt.1` should be Int64, but this one is Bool")))

(case
  "a tuple element path drills into a nested record field"
  (input
    (do
      (def (h (: t (Tuple (Record (: x Int64)) Int64))) (. t 1))
      (def (g) (h #tuple(#record((= x true)) 2)))
      (export g)))
  (error CDZ0203 (message "element 0.x should be Int64, but this one is Bool")))

(case
  "the leaf-path drill stops at a deeper field-set difference, naming the immediate field"
  (input
    (do
      (def (h (: r (Record (: inner (Record (: x Int64) (: y Int64)))))) r.inner)
      (def (g) (h #record((= inner #record((= x 1))))))
      (export g)))
  (error CDZ0203 (message "field `inner` should be") (not "inner.")))

; Applying an EFFECT name as a function names the CATEGORY — "`E` is an effect, not a function" — the
; apply-position analogue of the "is a type, not a function" message. The effect's SYNTHESIZED record type is
; never leaked to the user (no `Record`/`Any` in the message). A non-name head keeps the type-named message
; (already covered by the applying-a-non-function cases). (Migrated from rcdzc
; applying_an_effect_name_names_the_category_not_the_leaked_record_type.)
(case
  "applying an effect name names the category, not the leaked synthesized record type"
  (input (do (effect E (op foo (-> Int64))) (def (main) (E 5)) (export main)))
  (error CDZ0201 (message "`E` is an effect, not a function") (not "Record") (not "Any")))
