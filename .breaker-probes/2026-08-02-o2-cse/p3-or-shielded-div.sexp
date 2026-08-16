(case "p3 or twin: repeated division in or-rhs must stay shielded when lhs is true"
  (input  (do
            (def (main (: x Int64))
              (if (or (= x 0) (= (+ (/ 100 x) (/ 100 x)) 2))
                  1
                  0))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
