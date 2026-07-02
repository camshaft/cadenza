; Compound types — witnesses the declarable type universe (type-system.md #The Declarable
; Type Universe), structural equality (core-semantics.md #Equality And Ordering), and the
; collections semantics (collections-and-text.md). Results are (: <value> <Type>); a
; rejected program records a diagnostic code; a runtime halt records a trap.

(case "a record is constructed and a field is read"
  (input  (let ((p (record (x 1) (y 2)))) p.x))
  (output (: 1 Int64)))

(case "records are structurally equal by contents"
  (input  (= (record (x 1) (y 2)) (record (x 1) (y 2))))
  (output (: true Bool)))

(case "a sum-type value is constructed through a variant"
  (doc    "Sign is declared elsewhere as (Neg | Zero | Pos); a value is one variant.")
  (input  (let ((s Sign.Pos)) s))
  (output (: Sign.Pos Sign)))

(case "a sum-type value is deconstructed by an exhaustive match"
  (input  (match Sign.Zero
            (Sign.Neg  -1)
            (Sign.Zero 0)
            (Sign.Pos  1)))
  (output (: 0 Int64)))

(case "a non-exhaustive match is rejected at compile time"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected.")
  (input  (match Sign.Zero
            (Sign.Neg -1)
            (Sign.Pos  1)))
  (error  CDZ0210))

(case "lists are equal by elements in order"
  (input  (= (list 1 2 3) (list 1 2 3)))
  (output (: true Bool)))

(case "indexing a list out of bounds traps"
  (doc    "Witnesses collections-and-text.md #List Operations Are Total Or Trap.")
  (input  (List.at (list 1 2 3) 5))
  (trap   "list index out of bounds"))

(case "map equality is independent of insertion order"
  (doc    "Witnesses collections-and-text.md #A Map Associates Keys With Values.")
  (input  (= (map (a 1) (b 2)) (map (b 2) (a 1))))
  (output (: true Bool)))

(case "structural and nominal typing differ"
  (doc    "Point and Vector are distinct nominal types with the same shape
           (type-system.md #User Types Are Declarable As Nominal Or Structural).")
  (input  (= (Point (x 0) (y 0)) (Vector (x 0) (y 0))))
  (error  CDZ0202))
