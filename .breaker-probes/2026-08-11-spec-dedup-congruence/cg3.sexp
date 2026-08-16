(case "cg3 TRULY congruent twin performers — the dedup may merge them, both call sites stay exact"
  (input  (do
            (effect A (op get (-> Int64)))
            (def (wa (: k Int64))
              (if (< k 1) 0 (+ (A.get) (wa (- k 1)))))
            (def (wb (: k Int64))
              (if (< k 1) 0 (+ (A.get) (wb (- k 1)))))
            (def (main (: n Int64))
              (handle A 10
                ((get () s (resume s (+ s 1))))
                (+ (wa n) (* 1000 (wb n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 42033 Int64)))
