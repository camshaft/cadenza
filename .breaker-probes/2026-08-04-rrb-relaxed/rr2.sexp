(case "rr2 List.update through a concat-built relaxed spine touches all three segments, original intact"
  (input  (do
            (def (build (: n Int64) (: acc (List Int64)))
              (if (> n 0) (build (- n 1) (List.push acc n)) acc))
            (def (main (: k Int64))
              (let ((joined (List.concat (List.concat (build 37 (list)) (build 45 (list))) (build 29 (list)))))
                (let ((upd (List.update (List.update (List.update joined 5 101) 50 102) 100 103)))
                  (+ (* 1000000 (match (List.at upd 5) ((Some v) v) ((None _u) -1)))
                     (+ (* 10000 (match (List.at upd 50) ((Some v) v) ((None _u) -1)))
                        (+ (* 100 (match (List.at upd 100) ((Some v) v) ((None _u) -1)))
                           (match (List.at joined (+ 5 k)) ((Some v) v) ((None _u) -1))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 102030332 Int64)))
