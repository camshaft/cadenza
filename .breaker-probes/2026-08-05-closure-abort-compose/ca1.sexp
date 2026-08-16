(case "ca1 an ABORTING arm applies the CLOSURE STATE for its final answer (continuation discarded)"
  (input  (do
            (effect St (op fire (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ 1000
                (handle St (fn ((: x Int64)) (* x 7))
                  ((fire (v) f (f v)))
                  (+ 500 (St.fire n)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 1042 Int64)))
