(case "rnk1 an ICE RINK with wear feedback — skaters degrade the ice at DOUBLE rate once quality drops below five (the multiplier reads the field it degrades), the zamboni clears the ice and resurfaces by six CAPPED at ten counting the pass and echoing the cleared headcount, the read packs quality skaters and passes, and the seed's starting ice fires the fast-wear feedback immediately on one run (grinding to the floor) while the other wears slow and resurfaces to the cap"
  (input  (do
            (effect R
              (op skate (-> Int64 Int64))
              (op zamboni (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle R (tuple (+ (: 4 Int64) (* (% n 3) 5)) (: 0 Int64) (: 0 Int64))
                ((skate (k) st
                  (match st
                    ((tuple q sk ps)
                      (if (< q 5)
                          (if (> (- q (* k 2)) 0)
                              (resume (+ (* (- q (* k 2)) 10) (% (+ sk k) 10))
                                      (tuple (- q (* k 2)) (+ sk k) ps))
                              (resume (% (+ sk k) 10)
                                      (tuple (: 0 Int64) (+ sk k) ps)))
                          (if (> (- q k) 0)
                              (resume (+ (* (- q k) 10) (% (+ sk k) 10))
                                      (tuple (- q k) (+ sk k) ps))
                              (resume (% (+ sk k) 10)
                                      (tuple (: 0 Int64) (+ sk k) ps)))))))
                 (zamboni () st
                  (match st
                    ((tuple q sk ps)
                      (if (> (+ q 6) 10)
                          (resume (+ (: 700 Int64) (+ (* (+ ps 1) 10) (% sk 10)))
                                  (tuple (: 10 Int64) (: 0 Int64) (+ ps 1)))
                          (resume (+ (: 700 Int64) (+ (* (+ ps 1) 10) (% sk 10)))
                                  (tuple (+ q 6) (: 0 Int64) (+ ps 1)))))))
                 (read () st
                  (match st
                    ((tuple q sk ps)
                      (resume (+ (* q 100) (+ (* sk 10) ps)) st)))))
                (let ((a (R.skate (: 3 Int64))))
                  (let ((c (R.zamboni)))
                    (let ((d (R.skate (: 4 Int64))))
                      (let ((f (R.read)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) c)) d)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 63713064641 Int64))
  (call   main (: 0 Int64)) (output (: 3713024241 Int64)))
