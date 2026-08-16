(case "lc1 SELF-CONCAT sharing — the same list appears twice in one concat, then the original grows and is re-read unchanged"
  (input  (do
            (def (main (: n Int64))
              (let ((xs (list n (+ n 1))))
                (let ((doubled (List.concat xs xs)))
                  (let ((grown (List.push xs 99)))
                    (+ (* 100000 (List.len doubled))
                       (+ (* 1000 (match (List.at doubled 2) ((Some v) v) ((None _u) -1)))
                          (+ (* 10 (List.len grown))
                             (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 403034 Int64))
  (call   main (: 0 Int64)) (output (: 400031 Int64)))
