(case "dc1 a DEEP left-leaning concat chain — ten single-element concats build an 11-deep rope, random-access reads stay exact"
  (input  (do
            (def (at-or (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None _u) -1)))
            (def (grow (: k Int64) (: acc (List Int64)))
              (if (< k 1) acc (grow (- k 1) (List.concat acc (list k)))))
            (def (main (: n Int64))
              (let ((deep (grow 10 (list n))))
                (+ (* 100000 (List.len deep))
                   (+ (* 10000 (at-or deep 0))
                      (+ (* 100 (at-or deep 5)) (at-or deep 10))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1170601 Int64))
  (call   main (: 0 Int64)) (output (: 1100601 Int64)))
