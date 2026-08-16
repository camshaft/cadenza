(case "ha2 a THREE-layer adapter chain: each layer transforms the layer below's answer (+1, *10, +100)"
  (input  (do
            (effect L1 (op q (-> Unit Int64)))
            (effect L2 (op q (-> Unit Int64)))
            (effect L3 (op q (-> Unit Int64)))
            (def (main (: n Int64))
              (handle L1 n
                ((q (u) s (resume (+ s 1) s)))
                (handle L2 0
                  ((q (u) t (resume (* (L1.q) 10) t)))
                  (handle L3 0
                    ((q (u) w (resume (+ (L2.q) 100) w)))
                    (L3.q)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 160 Int64)))
