(case "rf1 let-bound perform results stored in record fields"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((b (St.next)))
                    (let ((r (record (lo a) (hi b))))
                      (+ (* 100 (. r lo)) (. r hi)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))
