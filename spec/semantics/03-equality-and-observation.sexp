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
  (input  (do
            (def (f x) (= x 3.5))
            (def (main) (f 3.5)) (export main)))
  (output (: true Bool)))

(case "runtime float inequality compares by canonical byte form"
  (doc    "The companion with an unequal runtime operand: f(2.5) compares 2.5 to 3.5 and is false.
           Confirms the runtime float comparison is a genuine value test (true for 3.5, false for
           2.5), not a constant fold. The seed declines the same way.")
  (input  (do
            (def (f x) (= x 3.5))
            (def (main) (f 2.5)) (export main)))
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

(case "two runtime strings compare equal by their contents"
  (doc    "`eq2` compares its two String PARAMETERS — both runtime values, neither a literal the
           compiler can fold against. `(eq2 \"foo\" \"foo\")` is true. String equality is realized
           (collections-and-text.md #String Equality Follows Normalized Contents), so it must hold when
           BOTH operands are runtime, not only when one side is a literal (which folds). The seed
           declines (\"runtime compound equality (heap walk) not yet emitted\"): it folds a literal-side
           comparison but has not emitted the two-runtime heap walk. A program comparing two names read
           from data takes this shape.")
  (input  (do
            (def (eq2 a b) (= a b))
            (def (main) (eq2 "foo" "foo")) (export main)))
  (output (: true Bool)))

(case "two unequal runtime strings compare false by their contents"
  (doc    "The companion with unequal runtime operands: `(eq2 \"foo\" \"bar\")` is false. Confirms the
           two-runtime string comparison is a genuine content test, not a constant fold (true for equal
           contents, false for different). The seed declines the same way as the equal case.")
  (input  (do
            (def (eq2 a b) (= a b))
            (def (main) (eq2 "foo" "bar")) (export main)))
  (output (: false Bool)))

(case "a runtime string compared against a literal folds against the literal side"
  (doc    "The control the two cases above must be distinguished from: when ONE operand is a literal,
           the comparison folds against that side and the seed compiles it. `f` compares its String
           parameter to the literal \"x\"; `(f \"x\")` is true. Pins that the runtime-string equality
           gap is specifically the BOTH-runtime case — a literal on either side is already emitted.")
  (input  (do
            (def (f s) (= s "x"))
            (def (main) (f "x")) (export main)))
  (output (: true Bool)))

(case "a runtime string bound from a sum payload compares equal to a string parameter"
  (doc    "The two-runtime-string case above compares two direct PARAMETERS; this compares a String bound
           from a SUM-VARIANT PAYLOAD (`s` from `(Wrap.Wrap s)`) against a String parameter (`name`) —
           still two runtime operands with no literal to fold, but one is now a heap value extracted from a
           constructor payload rather than a bare parameter. `(payload-is (Wrap.Wrap \"foo\") \"foo\")` is
           true by String equality (collections-and-text.md #String Equality Follows Normalized Contents).
           A generation that emits the two-runtime heap walk for bare parameters but not for a
           payload-extracted operand declines here (\"runtime compound equality (heap walk) not yet
           emitted\") — the payload/aliased-operand companion of the two-parameter case; a program that
           compares a name it destructured from a data node against an expected name takes exactly this
           shape.")
  (input  (do
            (type Wrap (Wrap String))
            (def (payload-is w name) (match w ((Wrap.Wrap s) (= s name))))
            (def (main) (payload-is (Wrap.Wrap "foo") "foo")) (export main)))
  (output (: true Bool)))

; --- Equality of two RUNTIME compound values (a heap walk over the value heap) -----------------
; core-semantics.md #Equality Is Structural: two values are equal when they have the same type and
; their contents are equal component-wise; #Values Are Equal … agrees with the canonical byte form. The
; component-wise cases above compare compound values built from LITERALS (folded at compile time). The
; demanding shape is two compound values BUILT AT RUN TIME — a sum/record/tuple whose contents come
; from a parameter or a call — so the comparison is a walk over two heap values, not a constant fold.
; The seed declines this ("runtime compound equality (heap walk) not yet emitted") — the same
; not-yet-emitted runtime path the two-runtime-string case above hits (a String is itself a
; Bytes-backed heap value, so the two declines share one root). A program that compares two runtime AST
; nodes / proof terms / records for structural equality hits this; the recorded oracle is what a
; generation emitting the heap-walk comparison reproduces. Until then a program routes around it with a
; hand-written recursive comparator (scalar `=` on the leaves, which IS emitted).

(case "two runtime sum values compare equal by a heap walk"
  (doc    "`mk` builds a runtime sum `(N.I n)` from its parameter, so both operands of `(= (mk 1) (mk
           1))` are heap values, not folded constants. Structural equality (core-semantics.md #Equality
           Is Structural) makes them equal, so the program is true. The seed declines (\"runtime
           compound equality (heap walk) not yet emitted\"): it folds equality of compile-time-known
           compounds but has not emitted the runtime heap walk. The runtime-compound companion of the
           runtime-float and two-runtime-string equality cases above — all three are the same
           not-yet-emitted runtime comparison. A generation emitting the heap walk reproduces true.")
  (needs  sum-type-declaration)
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (main) (if (= (mk 1) (mk 1)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "two differing runtime sum values compare unequal by a heap walk"
  (doc    "The companion with unequal runtime compounds: `(mk 1)` is `(N.I 1)` and `(mk2 2)` is `(N.I
           2)`, so the heap walk finds their payloads differ and the comparison is false → 0. Confirms
           the runtime compound comparison is a genuine structural test, not a constant fold. The seed
           declines the same way as the equal case.")
  (needs  sum-type-declaration)
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (main) (if (= (mk 1) (mk 2)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "two constant sums with the same payload but different variants are not equal"
  (doc    "Constant compound equality folds STRUCTURALLY (core-semantics.md #Equality Is Structural), and
           structural equality compares the VARIANT before the payload: `(= (Ok 1) (Err 1))` is FALSE even
           though both carry the payload 1, because `Ok` and `Err` are different variants. Pins the
           discriminant half of the fold — an implementation that compared only payloads (a heap walk that
           skipped the variant tag) would wrongly report true here, conflating `Ok 1` and `Err 1`. The
           companion of `(= (Ok 1) (Ok 1))` = true: same variant AND same payload.")
  (input  (= (Ok 1) (Err 1)))
  (output (: false Bool)))

(case "two constant records with the same fields in different written order are equal"
  (doc    "Constant record equality folds structurally and compares fields as a SET keyed by name, not by
           written order: `(= (record (x 1) (y 2)) (record (y 2) (x 1)))` is true — both denote the same
           value (a record's canonical form sorts its fields by key, deterministic-value-form.md #A Value
           Has One Canonical Byte Form). Pins that the equality fold normalizes field order before
           comparing, so the same record written two ways is one value — not a position-wise comparison
           that would call these unequal.")
  (input  (= (record (x 1) (y 2)) (record (y 2) (x 1))))
  (output (: true Bool)))

(case "a runtime compound structural equality is expressible as a hand-written recursive comparator"
  (doc    "The route around the not-yet-emitted heap walk, and the shape a program needing runtime
           compound equality writes today: an explicit recursive comparator that dispatches on each
           value's variant and compares the leaves with scalar `=` (which IS emitted for runtime
           scalars). `same` compares two `N` values by matching both and comparing the bound Int64
           payloads; `(same (mk 1) (mk 1))` is true → 1. Pins that structural equality of runtime
           compounds is ALREADY achievable by hand — the missing built-in `=` heap walk is a
           convenience over this, not a new expressive power — so a program (a proof kernel comparing
           terms, a compiler comparing AST nodes) is not blocked, only more verbose.")
  (needs  sum-type-declaration)
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (same a b)
              (match a
                ((N.I x) (match b ((N.I y) (= x y)) ((N.J _) false)))
                ((N.J x) (match b ((N.J y) (= x y)) ((N.I _) false)))))
            (def (main) (if (same (mk 1) (mk 1)) 1 0)) (export main)))
  (output (: 1 Int64)))

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
  (input  (do
            (def (lt a b) (< a b))
            (def (main)   (lt 20 22)) (export main)))
  (output (: true Bool)))

(case "an entrypoint returning arithmetic presents an Int64 result at the boundary"
  (doc    "The Int64 companion to the Bool-boundary case above: the same nullary-`main`-calls-a-helper
           shape, but the body is an arithmetic expression whose result type is Int64, so `run` is framed
           at the Int64 boundary valtype and `(add 20 22)` crosses as 42. Together the two cases pin that
           the entrypoint's boundary result type is type-directed — Bool for a comparison, Int64 for
           arithmetic — the same program shape emitting a different boundary type from its result type
           alone.")
  (input  (do
            (def (add a b) (+ a b))
            (def (main)    (add 20 22)) (export main)))
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
  (output (: (Less unit) Ordering)))

(case "comparing equal values yields Equal"
  (doc    "`(compare 2 2)` is `(Ordering.Equal unit)` — the middle variant, distinct from both Less and
           Greater. Pins that the three-way comparison reports equality as its own variant rather than
           collapsing it into one of the strict relations.")
  (needs  ordering)
  (input  (compare 2 2))
  (output (: (Equal unit) Ordering)))

(case "comparing a greater value to a lesser yields Greater"
  (doc    "`(compare 3 2)` is `(Ordering.Greater unit)` — the Greater variant. Together with the Less and
           Equal cases this pins all three variants of the Ordering result are reachable and correctly
           discriminated by the value relation.")
  (needs  ordering)
  (input  (compare 3 2))
  (output (: (Greater unit) Ordering)))

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
  (output (: (Less unit) Ordering)))

(case "a program that makes a host call has that call in its observable behavior"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module declares a unit-returning effect `log` and the entrypoint delegates it to the host,
           so its operation `log.emit` is bound (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates); the run makes one host call and returns the unit value — the
           normal-termination value of a program evaluated only for its effect (core-semantics.md #An
           Expression Evaluated Only For Its Effect Yields The Unit Value). The (output …) primary clause
           pins the terminal condition; the (host-calls …) observation pins the call sequence.")
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "hello"))) (export main)))
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
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (do
                  (log.emit "first")
                  (log.emit "second")))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "first" String))
              (call log.emit (: "second" String))))
