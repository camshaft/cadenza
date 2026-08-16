(case "dn2b an abort under FOUR resumptive frames — the unwind abandons all four pending sums (extends the three-frame pin)"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (effect C (op c (-> Int64)))
            (effect D (op d (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail 0
                ((bail (v) s v))
                (handle A 1 ((a () s (resume s (+ s 1))))
                  (handle B 10 ((b () s (resume s (+ s 1))))
                    (handle C 100 ((c () s (resume s (+ s 1))))
                      (handle D 1000 ((d () s (resume s (+ s 1))))
                        (+ (A.a) (+ (B.b) (+ (C.c) (+ (D.d) (Bail.bail 7)))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
