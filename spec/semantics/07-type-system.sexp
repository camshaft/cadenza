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
