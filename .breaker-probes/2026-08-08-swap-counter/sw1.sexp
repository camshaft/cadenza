(case "sw1 a three-slot state SWAPS its pair while a counter ticks — the encoding exposes position, order, and dispatch count at once"
  (input  (do
            (effect E (op swap (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (+ n 1) 0)
                ((swap () s (match s
                              ((tuple a b k)
                                (resume (+ (* 100 a) (+ (* 10 b) k))
                                        (tuple b a (+ k 1)))))))
                (+ (E.swap) (+ (E.swap) (E.swap)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 783 Int64))
  (call   main (: 0 Int64)) (output (: 123 Int64))
  (call   main (: -3 Int64)) (output (: -867 Int64)))
