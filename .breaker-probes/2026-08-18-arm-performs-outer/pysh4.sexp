(case "pysh4 the SHADOWING ARM PERFORMS THE EFFECT IT HANDLES — the inner arm's own body draws E while building its answer and that draw routes to the OUTER arm because a handler arm runs OUTSIDE its own region, the outer's decorated answer folds into the inner's, and an arm that captured its own region instead would recurse on itself"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (resume (+ (* s 10) 1) (+ s 1))))
                (handle E (: 50 Int64)
                  ((tick () s (resume (+ s (E.tick)) (+ s 1))))
                  (E.tick))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 61 Int64))
  (call   main (: 0 Int64)) (output (: 51 Int64)))
