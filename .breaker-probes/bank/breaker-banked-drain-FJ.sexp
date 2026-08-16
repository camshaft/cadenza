(case "eval of a quoted nested list-of-lists construction folds to the runtime nested list"
  (doc    "The NESTED-construction face of eval (the pinned :2945 evals a FLAT (list 1 2 3)): a quoted
           `(list (list 1 2) (list 3 4 5))` — the reconstructor must fold the INNER list constructors
           into heap list values AND assemble them as the outer list's elements, a two-level
           reconstruct where each inner (list …) is itself a construction form the eval walk recurses
           into. Read back outer len 2, inner lens 2 and 3 → 223. An eval that flattened the nest, or
           folded only the outer constructor leaving the inners as unreconstructed Ast, drifts a
           len. Extends the eval data-construction family to compound-element depth.")
  (input  (do
            (def (main)
              (let ((v (eval (quote (list (list 1 2) (list 3 4 5))))))
                (+ (* 100 (List.len v))
                   (+ (* 10 (List.len (Option.expect (List.at v 0) "r0")))
                      (List.len (Option.expect (List.at v 1) "r1"))))))
            (export main)))
  (call   main) (output (: 223 Int64)))
