(case "dt1 a THUNK built in the body wrapping a perform, forced twice (each force re-performs)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (do
                  (def th (fn ((: u Int64)) (St.a)))
                  (+ (* 10 (th 0)) (th 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
