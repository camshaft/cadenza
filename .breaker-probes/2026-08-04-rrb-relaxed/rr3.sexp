(case "rr3 List.push onto a concat-built relaxed spine appends past the last seam"
  (input  (do
            (def (build (: n Int64) (: acc (List Int64)))
              (if (> n 0) (build (- n 1) (List.push acc n)) acc))
            (def (main (: k Int64))
              (let ((joined (List.concat (List.concat (build 37 (list)) (build 45 (list))) (build 29 (list)))))
                (let ((grown (List.push joined (+ 999 k))))
                  (+ (* 10000 (List.len grown))
                     (+ (* 10 (match (List.at grown 111) ((Some v) v) ((None _u) -1)))
                        (match (List.at grown 36) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1129991 Int64)))
