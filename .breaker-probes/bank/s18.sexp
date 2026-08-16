(case "s18 SUM payload holding the closure + direct call"
  (input  (do
            (type H (Mk (-> Int64 Int64)))
            (def (main (: d Int64))
              (let ((k 100))
                (let ((f1 (fn ((: v Int64)) (+ k v))))
                  (let ((h (Mk f1)))
                    (+ (f1 d) (match h ((Mk g) (g 1))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 206 Int64)))
