(case "p6 minimal: bare bool lhs, repeated shielded division in and-rhs"
  (input  (do
            (def (main (: b Bool) (: d Int64))
              (if (and b (= (/ 10 d) (/ 10 d)))
                  1
                  0))
            (export main)))
  (call   main (: false Bool) (: 0 Int64)) (output (: 0 Int64))
  (call   main (: true Bool) (: 5 Int64)) (output (: 1 Int64)))
