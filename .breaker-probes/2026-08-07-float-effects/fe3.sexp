(case "fe3 an Int64 handler and a Float64 handler INTERLEAVED — integer ticks and exact float halving thread independently"
  (input  (do
            (effect I (op tick (-> Int64)))
            (effect F (op half (-> Float64)))
            (def (main (: n Int64))
              (handle I n
                ((tick () s (resume s (+ s 1))))
                (handle F 8.0
                  ((half () t (resume t (* t 0.5))))
                  (let ((i1 (I.tick)))
                    (let ((f1 (F.half)))
                      (let ((i2 (I.tick)))
                        (let ((f2 (F.half)))
                          (if (= (+ f1 f2) 12.0) (+ (* 10 i1) i2) -1))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
