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

; --- The comparison operators type-check their operands exactly as = and + do -------------
; An ordering comparison (`<` `>` `<=` `>=`) offers a total order over ONE type's values
; (core-semantics.md #Ordering Where Offered Is Total; type-system.md #Structural Values Are
; Comparable Only When Their Shapes Match). Comparing two DIFFERENT numeric types is the same
; silent-promotion the arithmetic operators forbid (numeric-model.md #Numeric Types Do Not
; Silently Promote), so `(< 5 2.0)` is rejected (CDZ0301) exactly as `(+ 5 2.0)` and `(= 5 2.0)`
; are — an ordering is not a licence to promote Int64 to Float64 where + may not. Comparing two
; UNRELATED kinds (Int64 vs Bool, Int64 vs String) has no shared order at all, a general type
; error (CDZ0201), exactly as `(= 1 true)` is. These pin that the ordering operators are held to
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
           general type error the compiler rejects (CDZ0201), exactly as `(= 1 true)` is. An
           ordering operator is not a coercion to a common type; a Bool has no position in Int64's
           order.")
  (input  (< 1 true))
  (error  CDZ0201))

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
           Values Whose Type Is The Type Of Types).")
  (needs  type-system)
  (input  (let ((t Int64)) t))
  (output (: Int64 Type)))

(case "a consistent annotation type-checks against the inferred type"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict and #A Well-Typed Program
           Does Not Go Wrong: `(: (+ 1 2) Int64)` type-checks because inference determines the
           expression's type is Int64 and the annotation unifies with it, so the program compiles and
           evaluates to 3. The passing companion to the CDZ0203 rejections above.")
  (needs  type-system)
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
  (doc    "`(. 5)` supplies the record operand but no field name: member access `(. <record> <field>)`
           takes exactly two operands. The compiler rejects it (CDZ0201), never panicking reaching for
           the absent field node — the member-access companion of the `(tuple.0)` accessor-with-no-operand
           case below.")
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

(case "a tuple accessor with no operand is rejected, not a crash"
  (doc    "`(tuple.0)` names a positional tuple accessor but supplies no tuple to project from.
           Ill-formed: the accessor takes exactly one operand. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent tuple argument.")
  (input  (tuple.0))
  (error  CDZ0201))

(case "a record field with no value expression is rejected, not a crash"
  (doc    "A record entry `(a)` names the field `a` but supplies no value: `(record (a))` is ill-formed
           — a record entry is a `(name value)` pair. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Same never-crash class as the `(let ((x)) x)`
           binding-with-no-value case above, for a record entry.")
  (needs  collections)
  (input  (record (a)))
  (error  CDZ0201))

(case "a map entry with no value expression is rejected, not a crash"
  (doc    "The map companion: `(map (a))` names the key `a` but supplies no value — a map entry is a
           `(key value)` pair, so this is ill-formed. The compiler rejects it (CDZ0201), never
           panicking reaching for the absent value node. Pins that both the `record` and `map`
           construction paths bounds-check an entry before indexing its value.")
  (needs  collections)
  (input  (map (a)))
  (error  CDZ0201))

; --- Never — the empty sum, the dual of Unit, the type of a diverging expression ---------------
; type-system.md #Never Is The Empty Sum: the type universe includes the sum with ZERO variants, the
; dual of Unit (the empty tuple / zero-field product). Never is UNINHABITED — it has no constructor and
; no value — so it is only ever a TYPE, never a value a program builds. The type of an expression that
; DIVERGES rather than producing a value — a `(trap …)`, or `expect` on an absent optional — is Never,
; and Never UNIFIES WITH ANY EXPECTED TYPE (there is no value to be of the wrong type). The seed already
; carries this mechanism internally (a divergent expression's kind unifies with any expected kind, so a
; whole-body-trap function type-checks in any result position); these cases pin the SURFACE property.
; Tagged `(needs never)` — a FRESH capability the seed does not surface by name — so the behavior gate
; SKIPS them, pinning the contract a later generation binds (the `Never` prelude name and the zero-arm
; exhaustive match) rather than forcing the seed to run them.

(case "a diverging expression unifies with an integer position"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum (3rd sentence: the type of a diverging
           expression is Never, which unifies with any expected type). In `(if b 1 (trap \"unreachable\"))`
           the then-branch is Int64 and the else-branch diverges (type Never); the two branches unify to
           Int64 because Never unifies with any type. With b=true the program yields 1; the else-branch
           never runs but must TYPE-CHECK. A generation without the Never-unifies rule would reject the
           branch-type mismatch. Pins that a divergent branch does not spoil a well-typed conditional.")
  (needs  never)
  (input  (module m
            (def (f b) (if b 1 (trap "unreachable")))
            (def (main) (f true))))
  (output (: 1 Int64)))

(case "a function whose body always diverges has result type Never"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum: `bomb` always traps, so its body has type
           Never; calling it at a use site that expects an Int64 type-checks because Never unifies with
           any expected type. The call diverges at run time (the trap), so the program's terminal
           condition is the trap, not a value. Pins that a Never-returning function is callable in a
           typed position — the honest type for a function that never returns normally.")
  (needs  never)
  (input  (module m
            (def (bomb) (trap "unreachable"))
            (def (main) (+ 1 (bomb)))))
  (trap   "unreachable"))

(case "a match on an uninhabited scrutinee is exhaustive with zero arms"
  (doc    "Witnesses type-system.md #Never Is The Empty Sum (4th sentence: a match on a Never-typed
           scrutinee is exhaustive with zero arms). `never-returns` has result type Never, so matching
           its result needs NO arms to cover every variant — there are none — and the zero-arm match is
           the degenerate BASE CASE of the exhaustiveness rule (core-semantics.md #Matching Is Exhaustive
           Or Rejected), NOT a CDZ0210 non-exhaustive rejection. The scrutinee diverges before the match,
           so the program traps. Pins that the empty sum makes a zero-arm match vacuously exhaustive
           rather than an error.")
  (needs  never)
  (input  (module m
            (def (never-returns) (trap "unreachable"))
            (def (main) (match (never-returns)))))
  (trap   "unreachable"))
