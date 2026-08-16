(case "as5 as-class radius: SAME-handler perform in the next-state slot ((resume t (+ t (B.other))))"
  (input  (do
            (effect B (op step (-> Unit Int64)) (op other (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B n
                ((step (u) t (resume t (+ t (B.other))))
                 (other (u) t (resume 1 t)))
                (+ (* 10 (B.step)) (B.step))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
