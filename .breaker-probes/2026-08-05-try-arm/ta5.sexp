(case "ta5 try over a runtime if via a let-bound scrutinee"
  (input  (do
            (def (f (: s Int64))
              (: (let ((opt (if (> s 0) (Some s) (: (None unit) (Option Int64)))))
                   (let ((v (try opt))) (Some v)))
                 (Option Int64)))
            (def (main (: k Int64))
              (match (f k) ((Some v) v) ((None _u) -99)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: -99 Int64)))
