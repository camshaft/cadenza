(case "cs1 a CLOSURE handler state captures the enclosing function's parameter and applies per dispatch"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (* x n))
                ((next (u) f (resume (f 10) f)))
                (St.next)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
