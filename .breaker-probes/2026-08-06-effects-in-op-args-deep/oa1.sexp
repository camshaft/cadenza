(case "oa1 a perform's ARGUMENT is another perform's result (same op, nested call)"
  (input  (do
            (effect St (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((dbl (v) s (resume (* v 2) (+ s 1))))
                (St.dbl (St.dbl (St.dbl n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 40 Int64)))
