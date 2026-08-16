(case "cg1 two recursive performers IDENTICAL except the effect they perform — the congruence must not merge across effect identity"
  (input  (do
            (effect A (op get (-> Int64)))
            (effect B (op get (-> Int64)))
            (def (wa (: k Int64))
              (if (< k 1) 0 (+ (A.get) (wa (- k 1)))))
            (def (wb (: k Int64))
              (if (< k 1) 0 (+ (B.get) (wb (- k 1)))))
            (def (main (: n Int64))
              (handle A 100
                ((get () s (resume s (+ s 1))))
                (handle B 500
                  ((get () t (resume t (+ t 10))))
                  (+ (wa n) (wb n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1833 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
