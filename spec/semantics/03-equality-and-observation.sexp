; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program Terminates In Exactly One Terminal Condition. Results are (: <value> <Type>);
; observation of ordered host calls uses (host-calls ...); resource-measure exhaustion uses (exhausted).

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

(case "a program that makes a host call has that call in its observable behavior"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module imports and declares a unit-returning host function `log`, so it is bound
           (host-interface-binding.md #A Host Import Is A WIT-Typed Function The Manifest Enumerates);
           the run makes one host call and returns the unit value — the normal-termination value of a
           program evaluated only for its effect (core-semantics.md #An Expression Evaluated Only For
           Its Effect Yields The Unit Value). The (output …) primary clause pins the terminal
           condition; the (host-calls …) observation pins the call sequence.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (log "hello"))))
  (output (: unit Unit))
  (host-calls (call log (: "hello" String))))

(case "host calls are observed in the order they were made"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior and
           #A Sequencing Block Evaluates Its Forms In Order (3rd sentence: an earlier form's host call is
           observed before a later form's): the two host calls are sequenced by a (do …) block, so
           \"first\" is observed before \"second\". The run terminates normally with the unit value
           (core-semantics.md #An Expression Evaluated Only For Its Effect Yields The Unit Value); the
           (output …) clause pins that terminal condition and the (host-calls …) observation pins the order.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (do
                (log "first")
                (log "second")))))
  (output (: unit Unit))
  (host-calls (call log (: "first" String))
              (call log (: "second" String))))

(case "a program halts by exhausting the deterministic resource measure"
  (doc    "Witnesses core-semantics.md #Evaluation Is Bounded and #A Program Terminates In Exactly One
           Terminal Condition (the third terminal condition). Unbounded self-recursion consumes the
           resource measure (determinism-and-fuel.md §Resource Accounting) and halts at a defined
           point rather than running forever.")
  (input  (module m
            (def (loop n) (loop n))
            (def (main) (loop 0))))
  (exhausted))
