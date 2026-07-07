; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program That Terminates Ends In One Of Two Terminal Conditions. Results are
; (: <value> <Type>); observation of ordered host calls uses (host-calls ...).

(case "structural equality holds component-wise"
  (doc    "Witnesses core-semantics.md #Equality Is Structural.")
  (input  (= 3 3))
  (output (: true Bool)))

(case "negative zero is not equal to positive zero"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           -0.0 and 0.0 have distinct canonical byte forms, so they are not equal.")
  (input  (= -0.0 0.0))
  (output (: false Bool)))

(case "every not-a-number value is equal to every not-a-number value"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           all NaN values share one canonical byte form, so they compare equal. `nan` denotes the
           canonical not-a-number value (options/code-shape/, deterministic-value-form.md).")
  (input  (= nan nan))
  (output (: true Bool)))

; --- Float equality follows the canonical byte form RECURSIVELY, inside compound values --
; #Equality Is Structural: "Two values MUST be equal when they have the same type and their contents
; are equal component-wise" — and each float COMPONENT is compared by #Floating-Point Equality Follows
; The Canonical Byte Form (every NaN equal to every NaN; -0.0 distinct from 0.0). The scalar cases
; above pin the float rule at top level; these pin that structural equality applies the SAME rule to a
; float NESTED in a tuple/list/record/sum, not a naive f64.eq. This is the sharpest adversarial float
; case: a lowering that recurses into a compound with wasm's f64.eq gives the OPPOSITE answer for both
; NaN (f64.eq says nan≠nan → the tuples wrongly unequal) and -0.0 (f64.eq says -0.0=0.0 → wrongly
; equal). The seed's `cval_eq` recurses through `float_canonical_eq`, so it must match the scalar rule.

(case "a NaN nested in a tuple compares equal under the canonical byte form"
  (doc    "`(= (tuple nan) (tuple nan))` = true: structural equality compares the tuples component-wise
           (core-semantics.md #Equality Is Structural), and the float component follows the
           canonical-byte-form rule where every NaN equals every NaN — exactly as the scalar
           `(= nan nan)` does. A recursion using wasm's f64.eq would answer false (nan ≠ nan); this pins
           the canonical-byte-form rule holds for a float INSIDE a compound.")
  (input  (= (tuple nan) (tuple nan)))
  (output (: true Bool)))

(case "a negative zero nested in a tuple is distinct from positive zero"
  (doc    "`(= (tuple -0.0) (tuple 0.0))` = false: the float components -0.0 and 0.0 have distinct
           canonical byte forms, so the tuples are unequal — the compound companion of the scalar
           `(= -0.0 0.0)` = false. A recursion using wasm's f64.eq would answer true (-0.0 = 0.0),
           silently collapsing the distinction the canonical byte form preserves.")
  (input  (= (tuple -0.0) (tuple 0.0)))
  (output (: false Bool)))

(case "identical negative zeros nested in a tuple compare equal"
  (doc    "The control the case above pairs with: `(= (tuple -0.0) (tuple -0.0))` = true — two -0.0
           components share one canonical byte form, so the tuples are equal. Confirms the nested
           comparison is a genuine value test (true for matching -0.0, false against 0.0), not a
           blanket answer.")
  (input  (= (tuple -0.0) (tuple -0.0)))
  (output (: true Bool)))

(case "a NaN nested in a list compares equal under the canonical byte form"
  (doc    "The list companion: `(= (list nan 1.0) (list nan 1.0))` = true — element-wise equality
           compares nan against nan (equal, canonical byte form) and 1.0 against 1.0 (equal), so the
           lists are equal. Pins that the canonical-byte-form float rule recurses through list elements
           too, alongside an ordinary equal float element.")
  (needs  collections)
  (input  (= (list nan 1.0) (list nan 1.0)))
  (output (: true Bool)))

(case "a NaN nested in a sum payload compares equal under the canonical byte form"
  (doc    "The sum companion: `(= (Some nan) (Some nan))` = true — the variant tags match (both Some)
           and the payloads compare by the canonical-byte-form rule where nan equals nan. Pins that
           structural equality applies the float rule to a Sum's payload, not only to tuple/list
           elements.")
  (input  (= (Some nan) (Some nan)))
  (output (: true Bool)))

(case "a negative zero in a record field is distinct from positive zero"
  (doc    "The record companion of the nested -0.0 case: `(= (record (x -0.0)) (record (x 0.0)))` =
           false — the field `x` holds -0.0 in one record and 0.0 in the other, distinct canonical byte
           forms, so the records are unequal. Pins the canonical-byte-form float distinction through a
           record field, the field-access analogue of the tuple-element case.")
  (needs  collections)
  (input  (= (record (x -0.0)) (record (x 0.0))))
  (output (: false Bool)))

; Float64 equality is a REALIZED seed capability (options/realized-capability-set/: "Float64
; literals/equality"), so it must hold for a RUNTIME float operand — one from a function parameter,
; a call, an if — not only for two compile-time-constant literals. The cases above compare constant
; floats; these compare a runtime float against a constant. The seed emits only the CONSTANT float
; equality (folded at compile time) and declines a runtime one ("non-constant float equality
; (canonical byte form) not yet emitted") — a not-yet-emitted runtime path within a realized
; capability. The value itself is carried correctly (a runtime float identity `(f 3.5)` → 3.5); only
; the equality comparison of a runtime float is missing.

(case "runtime float equality compares by canonical byte form"
  (doc    "`f` takes a Float64 parameter and compares it to the literal 3.5; f(3.5) is true. Float
           equality is realized (options/realized-capability-set/), so it must apply to a runtime
           float operand, matching the canonical-byte-form comparison the constant cases above use.
           The seed declines (\"non-constant float equality … not yet emitted\") — it folds constant
           float equality but has not emitted the runtime comparison.")
  (input  (module m
            (def (f x) (= x 3.5))
            (def (main) (f 3.5))))
  (output (: true Bool)))

(case "runtime float inequality compares by canonical byte form"
  (doc    "The companion with an unequal runtime operand: f(2.5) compares 2.5 to 3.5 and is false.
           Confirms the runtime float comparison is a genuine value test (true for 3.5, false for
           2.5), not a constant fold. The seed declines the same way.")
  (input  (module m
            (def (f x) (= x 3.5))
            (def (main) (f 2.5))))
  (output (: false Bool)))

(case "an offered ordering is total and deterministic"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total: Int64 offers a total order.")
  (input  (< 2 3))
  (output (: true Bool)))

(case "an entrypoint returning a comparison presents a Bool result at the boundary"
  (doc    "Type-directed emission at the component boundary: a nullary `main` whose body is an Int64
           comparison has result type Bool, so the `run` export is framed at the Bool boundary valtype,
           not the s64 an arithmetic result would use. `(lt 20 22)` is true. The companion below returns
           the arithmetic i64 (42) through the SAME entrypoint shape, so the pair pins that the boundary
           result type tracks the program's RESULT TYPE — a comparison crosses as Bool, an arithmetic
           expression as Int64 — rather than a fixed valtype. This is the observable of a compiler that
           reads a program's result kind and frames `run` accordingly; the result kind is one of a fixed
           set (Int64 / Bool), selected by the operator that produces the result (a comparison yields
           Bool, `+`/`-`/`*`/`/`/`%` yield Int64).")
  (input  (module m
            (def (lt a b) (< a b))
            (def (main)   (lt 20 22))))
  (output (: true Bool)))

(case "an entrypoint returning arithmetic presents an Int64 result at the boundary"
  (doc    "The Int64 companion to the Bool-boundary case above: the same nullary-`main`-calls-a-helper
           shape, but the body is an arithmetic expression whose result type is Int64, so `run` is framed
           at the Int64 boundary valtype and `(add 20 22)` crosses as 42. Together the two cases pin that
           the entrypoint's boundary result type is type-directed — Bool for a comparison, Int64 for
           arithmetic — the same program shape emitting a different boundary type from its result type
           alone.")
  (input  (module m
            (def (add a b) (+ a b))
            (def (main)    (add 20 22))))
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

(case "false is less than true"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total (Bool clause): `(< false
           true)` is true because false is the lesser of the two Bool values — the direction of the
           Bool order.")
  (input  (< false true))
  (output (: true Bool)))

(case "true is not less than false"
  (doc    "The complement fixing the order's direction: `(< true false)` is false, because true is
           not below false. Together with the case above this pins false < true rather than the
           reverse ranking, so the order is antisymmetric in the specified direction.")
  (input  (< true false))
  (output (: false Bool)))

(case "true is greater than false"
  (doc    "The `>` companion: `(> true false)` is true, the mirror of `(< false true)`. Pins that the
           strict greater-than operator observes the same Bool order.")
  (input  (> true false))
  (output (: true Bool)))

(case "a boolean is less-than-or-equal to itself"
  (doc    "`(<= false false)` is true: the inclusive ordering operator is reflexive on Bool, as a
           total order requires. Pins `<=` on equal Bool operands.")
  (input  (<= false false))
  (output (: true Bool)))

(case "a boolean is greater-than-or-equal to itself"
  (doc    "`(>= true true)` is true: `>=` is reflexive on Bool. Completes the four ordering operators
           over the Bool order.")
  (input  (>= true true))
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
; canonical element order Set/Map serialize in. Tagged `(needs ordering)` — a FRESH capability the
; seed does not realize (NOT `collections`; the seed would otherwise RUN these and reject the unbound
; `Ordering`/`compare` names with a coded diagnostic — a gate FAIL — rather than skip). A later
; generation realizes `compare`; until then the seed's behavior gate SKIPS these.

(case "comparing a lesser value to a greater yields Less"
  (doc    "`(compare 1 2)` is `(Ordering.Less unit)` — the three-way comparison reports that 1 is less
           than 2 as the `Less` variant of the Ordering sum, not a boolean (core-semantics.md #A Total
           Order Is Observed Through A Three-Way Comparison). Pins the Less arm of the three-way result.")
  (needs  ordering)
  (input  (compare 1 2))
  (output (: (Ordering.Less unit) Ordering)))

(case "comparing equal values yields Equal"
  (doc    "`(compare 2 2)` is `(Ordering.Equal unit)` — the middle variant, distinct from both Less and
           Greater. Pins that the three-way comparison reports equality as its own variant rather than
           collapsing it into one of the strict relations.")
  (needs  ordering)
  (input  (compare 2 2))
  (output (: (Ordering.Equal unit) Ordering)))

(case "comparing a greater value to a lesser yields Greater"
  (doc    "`(compare 3 2)` is `(Ordering.Greater unit)` — the Greater variant. Together with the Less and
           Equal cases this pins all three variants of the Ordering result are reachable and correctly
           discriminated by the value relation.")
  (needs  ordering)
  (input  (compare 3 2))
  (output (: (Ordering.Greater unit) Ordering)))

(case "the three-way comparison is deconstructed by an exhaustive match"
  (doc    "An Ordering value is an ordinary closed sum, so it is matched with the uniform `(Ctor _)`
           patterns over its three variants (core-semantics.md #A Total Order Is Observed Through A
           Three-Way Comparison, 2nd sentence): matching `(compare 1 2)` selects the `Less` arm, yielding
           -1. Pins that a comparison result dispatches through the same exhaustive match as any other
           sum, so every consumer handles all three cases.")
  (needs  ordering)
  (input  (match (compare 1 2)
            ((Ordering.Less _)    -1)
            ((Ordering.Equal _)   0)
            ((Ordering.Greater _) 1)))
  (output (: -1 Int64)))

(case "the boolean less-than operator agrees with the three-way comparison"
  (doc    "core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison (3rd sentence: the
           boolean ordering operators MUST agree with the three-way comparison): `(< 1 2)` is true
           exactly when `(compare 1 2)` is `(Ordering.Less unit)`. This case pins that agreement — `(< 1
           2)` is true and the compare above is Less, so a type's one order is surfaced two ways that
           cannot diverge.")
  (needs  ordering)
  (input  (< 1 2))
  (output (: true Bool)))

(case "the three-way comparison orders strings lexicographically"
  (doc    "`(compare \"a\" \"b\")` is `(Ordering.Less unit)` — String offers a total order (the
           lexicographic order of its Unicode scalar values, collections-and-text.md #String Comparison
           Is Defined On Scalar Values), so compare works over it exactly as over Int64. Pins that the
           three-way comparison is offered by every type with a total order, not only the numeric types.")
  (needs  ordering)
  (input  (compare "a" "b"))
  (output (: (Ordering.Less unit) Ordering)))

(case "a program that makes a host call has that call in its observable behavior"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module declares a unit-returning effect `log` and the entrypoint delegates it to the host,
           so its operation `log.emit` is bound (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates); the run makes one host call and returns the unit value — the
           normal-termination value of a program evaluated only for its effect (core-semantics.md #An
           Expression Evaluated Only For Its Effect Yields The Unit Value). The (output …) primary clause
           pins the terminal condition; the (host-calls …) observation pins the call sequence.")
  (needs  effects)
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "hello")))))
  (output (: unit Unit))
  (host-calls (call log.emit (: "hello" String))))

(case "host calls are observed in the order they were made"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior and
           #A Sequencing Block Evaluates Its Forms In Order (3rd sentence: an earlier form's host call is
           observed before a later form's): the two host calls are sequenced by a (do …) block, so
           \"first\" is observed before \"second\". The run terminates normally with the unit value
           (core-semantics.md #An Expression Evaluated Only For Its Effect Yields The Unit Value); the
           (output …) clause pins that terminal condition and the (host-calls …) observation pins the order.")
  (needs  effects)
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (do
                  (log.emit "first")
                  (log.emit "second"))))))
  (output (: unit Unit))
  (host-calls (call log.emit (: "first" String))
              (call log.emit (: "second" String))))
