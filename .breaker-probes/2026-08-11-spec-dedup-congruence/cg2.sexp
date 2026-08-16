(case "cg2 near-congruent recursive performers — one coefficient differs, the dedup must keep both"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (wa (: k Int64))
              (if (< k 1) 0 (+ (* 2 (A.get)) (wa (- k 1)))))
            (def (wb (: k Int64))
              (if (< k 1) 0 (+ (* 3 (A.get)) (wb (- k 1)))))
            (def (main (: n Int64))
              (handle A 10
                ((get () s (resume s (+ s 1))))
                (+ (wa n) (* 100000 (wb n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 12600066 Int64)))
