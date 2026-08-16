(case "p4 control: repeated division inside an if branch stays shielded"
  (input  (do
            (def (main (: x Int64))
              (if (not (= x 0))
                  (if (= (+ (/ 100 x) (/ 100 x)) 2) 1 0)
                  0))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
