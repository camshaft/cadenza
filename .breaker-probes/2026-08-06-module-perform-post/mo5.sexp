(case "mo5 TWO modules' recursive performers interleaved under one handler"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module ma
              (def (wa (: n Int64) (: acc Int64))
                (if (= n 0) acc (wa (- n 1) (+ acc (Ctr.next unit)))))
              (export wa))
            (module mb
              (def (wb (: n Int64) (: acc Int64))
                (if (= n 0) acc (wb (- n 1) (+ acc (* 100 (Ctr.next unit))))))
              (export wb))
            (def (main (: k Int64))
              (handle Ctr 1 ((next (u) s (resume s (+ s 1))))
                (+ ((. ma wa) k 0) ((. mb wb) k 0))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 703 Int64)))
