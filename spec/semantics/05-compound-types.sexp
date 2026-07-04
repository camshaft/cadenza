; Compound types — record/sum/list/map construction, field read, structural equality, matching, and
; the list-index trap. The primary clause is the dynamic interpreter (the oracle); where a typed
; generation rejects a program the interpreter still runs (non-exhaustive match CDZ0210,
; nominal/structural mismatch CDZ0202), the divergence is annotated inline with (compiler …)
; (constitution VII bootstrap carve-out; options/realized-capability-set/). Results are (: <value>
; <Type>); a runtime halt records a trap.

(case "a record is constructed and a field is read"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field. The dotted
           display `p.x` is sugar for the canonical member-access form (. p x); a reader
           expands one to the other (options/code-shape/), so both denote the same tree.")
  (input  (let ((p (record (x 1) (y 2)))) p.x))
  (output (: 1 Int64)))

(case "member access is written explicitly as the dot form in the canonical tree"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field: the canonical
           form is (. <record> <key>); `p.y` is its display sugar. This case writes the
           canonical form directly to pin that the tree carries (. …), not a dotted atom.")
  (input  (let ((p (record (x 1) (y 2)))) (. p y)))
  (output (: 2 Int64)))

(case "member access on a non-record traps"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field (2nd sentence):
           projecting a field of a non-record value has no defined result for the dynamic
           interpreter, so it traps rather than producing an unspecified value.")
  (input  (. 5 x))
  (trap   "member access on a non-record"))

(case "member access of a missing field traps"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field (3rd sentence):
           projecting a field the record does not contain traps rather than producing an
           unspecified value.")
  (input  (let ((p (record (x 1)))) (. p z)))
  (trap   "no such field"))

(case "two structural values of the same shape are equal"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural (2nd
           sentence) at the value level — the seed compares by structure without static types: two
           records of the same shape and contents compare equal.")
  (input  (= (record (x 1) (y 2)) (record (x 1) (y 2))))
  (output (: true Bool)))

(case "a sum-type value is constructed through a variant"
  (doc    "Sign is declared where used as (Neg | Zero | Pos) (options/code-shape/); a value is one
           variant. Construction is via application: Sign.Pos is a Constructor (function), and
           (Sign.Pos unit) applies it to unit, producing the Sum value.")
  (input  (let ((s (Sign.Pos unit))) s))
  (output (: Sign.Pos Sign)))

(case "a sum-type value is deconstructed by an exhaustive match"
  (doc    "Patterns are uniform: `(Ctor _)` for nullary constructors. The binder `_` matches the
           unit payload. Consistent with unary constructor patterns like `(Some x)`.")
  (input  (match (Sign.Zero unit)
            ((Sign.Neg _)  -1)
            ((Sign.Zero _) 0)
            ((Sign.Pos _)  1)))
  (output (: 0 Int64)))

(case "lists are equal by elements in order"
  (doc    "Witnesses collections-and-text.md #A List Is An Ordered Homogeneous Sequence (equality).
           Needs the primitive collections the seed realizes to build an AST (list/map/record).")
  (needs collections)
  (input  (= (list 1 2 3) (list 1 2 3)))
  (output (: true Bool)))

(case "indexing a list out of bounds traps"
  (doc    "Witnesses collections-and-text.md #List Operations Are Total Or Trap. The seed traps at
           runtime — a total-or-trap operation, not a static check.")
  (needs collections)
  (input  (List.at (list 1 2 3) 5))
  (trap   "list index out of bounds"))

(case "map equality is independent of insertion order"
  (doc    "Witnesses collections-and-text.md #A Map Associates Keys With Values.")
  (needs collections)
  (input  (= (map (a 1) (b 2)) (map (b 2) (a 1))))
  (output (: true Bool)))

(case "a match not covering the scrutinee is a runtime trap for the interpreter, a compile-time rejection for the compiler"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected. The dynamic interpreter
           (oracle) traps at runtime when the scrutinee (Sign.Zero unit) hits no branch — only Neg
           and Pos patterns are present. A typed generation, knowing the variant set statically,
           rejects the non-exhaustive match at compile time (CDZ0210) before it runs.")
  (input    (match (Sign.Zero unit)
              ((Sign.Neg _) -1)
              ((Sign.Pos _)  1)))
  (trap     "no matching pattern")
  (compiler (error CDZ0210)))

(case "a nullary constructor is a single-arity function taking unit"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (2nd sentence): a 'nullary' variant is a constructor whose argument type
           is Unit. Construction is uniform: (Sign.Zero unit) applies the constructor to unit and
           produces the Sum value. No pre-applied Sums in the prelude — the constructor is the value
           bound to Sign.Zero, not the already-constructed variant.")
  (input  (Sign.Zero unit))
  (output (: Sign.Zero Sign)))

(case "a unary constructor is a single-arity function"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (1st sentence): Some is a single-arity constructor. Applied to an
           argument, it produces a Sum tagged 'Some' carrying that argument as payload.")
  (input  (Some 42))
  (output (: (Some 42) (Option Int64))))

(case "nullary constructor patterns are uniform with unary"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (4th sentence): patterns are uniform `(Ctor binder)`. A nullary variant
           pattern is `(Sign.Zero _)` (the binder matches unit), not a bare `Sign.Zero`. Uniformity:
           no arity-based special case — all constructor patterns have the same syntactic form.")
  (input  (match (Sign.Zero unit)
            ((Sign.Zero _) 1)
            ((Sign.Pos _)  2)))
  (output (: 1 Int64)))

(case "constructor pattern binders capture the payload uniformly"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant: the pattern `(Some x)` binds `x` to the payload (42); the pattern
           `(None u)` binds `u` to the payload (unit). Both are single-arity patterns — uniform
           handling, no nullary special case.")
  (input  (match (Some 42)
            ((Some x) x)
            ((None _) 0)))
  (output (: 42 Int64)))

(case "a match over multiple nullary constructors with uniform patterns"
  (doc    "Witnesses core-semantics.md #The Pattern Matcher MUST Handle All Constructor Patterns
           Uniformly: Sign has three nullary variants (Neg, Zero, Pos); each pattern is `(Ctor _)`.
           The pattern matcher does not special-case them vs unary constructors — all constructor
           patterns have identical structure.")
  (input  (match (Sign.Neg unit)
            ((Sign.Neg _)  -1)
            ((Sign.Zero _) 0)
            ((Sign.Pos _)  1)))
  (output (: -1 Int64)))

(case "mixing nullary and unary constructors in one match"
  (doc    "Witnesses core-semantics.md #The Pattern Matcher MUST NOT Special-Case Nullary Vs Unary
           Constructors: a match over Option (which has Some taking a payload and None taking unit)
           uses uniform `(Ctor binder)` syntax for both. No branch distinguishes nullary from unary.")
  (input  (match (Some 5)
            ((Some n)  n)
            ((None _)  0)))
  (output (: 5 Int64)))

(case "the prelude binds Constructor values not pre-applied Sums"
  (doc    "Witnesses core-semantics.md #The Prelude MUST Bind Constructor Values Only: the name
           `None` resolves to a Constructor (a function value), not to a pre-constructed Sum. You
           cannot use bare `None` as a value; you must apply it: `(None unit)`. The bound value is
           the constructor, not the variant.")
  (input  (let ((ctor None)) (ctor unit)))
  (output (: None (Option Any))))

(case "same-shape nominal types are distinct to the compiler, structural to the dynamic interpreter"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural. Point and
           Vector share a shape; a typed generation tracks nominal identity and rejects comparing them
           (CDZ0202). The dynamic interpreter has no nominal tags, so it compares by structure and the
           run yields true — the oracle for a dynamic generation.")
  (input    (= (Point (x 0) (y 0)) (Vector (x 0) (y 0))))
  (output   (: true Bool))
  (compiler (error CDZ0202)))
