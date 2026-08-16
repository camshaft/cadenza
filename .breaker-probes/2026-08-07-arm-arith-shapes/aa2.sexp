(case "aa2 a QUADRATIC next-state (s^2+1) — three dispatches, the state squaring away from the seed"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume s (+ (* s s) 1))))
                (+ (E.g) (+ (E.g) (E.g)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 33 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64))
  (call   main (: -3 Int64)) (output (: 108 Int64)))
