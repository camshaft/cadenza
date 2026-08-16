(case "cc2 a closure that ITSELF performs, called twice from the handle body (state advances between calls)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (do
                  (def f (fn ((: k Int64)) (* k (Cnt.bump))))
                  (+ (f 10) (f 100)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 650 Int64)))
