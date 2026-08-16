(case "fam3 a let-bound List.concat result read three ways"
  (input  (do
            (def (main (: k Int64))
              (let ((a (list k (+ k 1)))
                    (b (list (+ k 2))))
                (let ((joined (List.concat a b)))
                  (+ (List.len joined)
                     (+ (* 10 (match (List.at joined 2) ((Some v) v) ((None _u) -1)))
                        (* 1000 (if (= joined (List.concat a b)) 1 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1073 Int64)))
