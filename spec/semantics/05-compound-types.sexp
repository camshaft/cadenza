; Compound types — record/sum/list/map construction, field read, structural equality, matching, and
; the list-index trap. The primary clause is the dynamic interpreter (the oracle); where a typed
; generation rejects a program the interpreter still runs (non-exhaustive match CDZ0210,
; nominal/structural mismatch CDZ0202), the divergence is annotated inline with (compiler …)
; (constitution VII bootstrap carve-out; options/realized-capability-set/). Results are (: <value>
; <Type>); a runtime halt records a trap.

(case "a record is constructed and a field is read"
  (input  (let ((p (record (x 1) (y 2)))) p.x))
  (output (: 1 Int64)))

(case "two structural values of the same shape are equal"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural (2nd
           sentence) at the value level — the seed compares by structure without static types: two
           records of the same shape and contents compare equal.")
  (input  (= (record (x 1) (y 2)) (record (x 1) (y 2))))
  (output (: true Bool)))

(case "a sum-type value is constructed through a variant"
  (doc    "Sign is declared where used as (Neg | Zero | Pos) (options/code-shape/); a value is one
           variant.")
  (input  (let ((s Sign.Pos)) s))
  (output (: Sign.Pos Sign)))

(case "a sum-type value is deconstructed by an exhaustive match"
  (input  (match Sign.Zero
            (Sign.Neg  -1)
            (Sign.Zero 0)
            (Sign.Pos  1)))
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
           (oracle) traps at runtime when the scrutinee Sign.Zero hits no branch; a typed generation,
           knowing the variant set statically, rejects the non-exhaustive match at compile time
           (CDZ0210) before it runs.")
  (input    (match Sign.Zero
              (Sign.Neg -1)
              (Sign.Pos  1)))
  (trap     "no matching pattern")
  (compiler (error CDZ0210)))

(case "same-shape nominal types are distinct to the compiler, structural to the dynamic interpreter"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural. Point and
           Vector share a shape; a typed generation tracks nominal identity and rejects comparing them
           (CDZ0202). The dynamic interpreter has no nominal tags, so it compares by structure and the
           run yields true — the oracle for a dynamic generation.")
  (input    (= (Point (x 0) (y 0)) (Vector (x 0) (y 0))))
  (output   (: true Bool))
  (compiler (error CDZ0202)))
