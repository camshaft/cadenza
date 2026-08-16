(case "pal1 PARITY-ALTERNATING two-effect draws — one recursive driver picks which effect to draw per hop, both threads advance interleaved"
  (input  (do
            (effect A (op get (-> Int64)))
            (effect B (op get (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (< k 1) acc
                (walk (- k 1) (+ (* 10 acc) (if (= (% k 2) 0) (A.get) (B.get))))))
            (def (main (: n Int64))
              (handle A n
                ((get () s (resume s (+ s 1))))
                (handle B 50
                  ((get () t (resume t (+ t 2))))
                  (walk 4 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 6072 Int64))
  (call   main (: 8 Int64)) (output (: 13142 Int64)))
