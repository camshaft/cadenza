(case "a guard expression containing its own LET binding evaluates with both scopes visible"
  (doc    "A guard whose body is itself a binder: `(guard (list a b) (let ((s (+ a b))) (> s 15)))` —
           the guard's LET reads the PATTERN's bindings (a, b) and introduces its own (s), so the
           guard evaluates under a two-layer scope: pattern binders visible to the let initializer,
           the let binder visible to the comparison. n=6 → 6+12=18 > 15 → arm fires (1); n=5 → 15 not
           > 15 → falls through to the unguarded twin arm (0). The guarded-scalar desugar rewrites the
           guard into a synthesized arm — a rewrite that hoisted the let out of the pattern scope
           (orphaning a/b, the finding-#46 class) or leaked s into the arm body's scope breaks a face.
           The guard-with-own-binder companion of the guard-reads-payload pins.")
  (input  (do
            (def (main (: n Int64))
              (match (list n (* n 2))
                ((guard (list a b) (let ((s (+ a b))) (> s 15))) 1)
                ((list a b) 0)
                (_ -1)))
            (export main)))
  (call   main (: 6 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: 0 Int64)))
