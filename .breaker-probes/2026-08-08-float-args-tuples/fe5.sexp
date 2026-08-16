(case "fe5 Float64 arguments HALVED in the arm — exact binary fractions cross dispatch both directions, count folds in"
  (input  (do
            (effect E (op halve (-> Float64 Float64)) (op count (-> Float64)))
            (def (main (: u Float64))
              (handle E 0.0
                ((halve (x) s (resume (* x 0.5) (+ s 1.0)))
                 (count () s (resume s s)))
                (+ (E.halve 3.0) (+ (E.halve 0.25) (E.count)))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 3.625 Float64)))
