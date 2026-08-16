(case "es2 a closure performing a HANDLED (non-delegated) effect does NOT trip the escape reject"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr 5 ((tick (u) s (resume s (+ s 1))))
                ((fn (x) (+ x (Ctr.tick))) k)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 15 Int64)))
