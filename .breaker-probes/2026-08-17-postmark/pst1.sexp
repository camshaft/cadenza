(case "pst1 a POSTMARK desk where every stamp weight is a LET CHAIN COMPUTED AT THE PERFORM SITE — the second stamp's argument let-binds the first answer's tens digit doubled-plus-one, the third stamp's argument chains two lets folding the second answer's low digit with the audit's remainder before a mod-seven clamp, stamp folds the weight into the total counting stamps and echoing weight and count, audit reads the total mod one hundred, and the seed sets the first weight so every downstream let chain carries different bindings between the runs"
  (input  (do
            (effect L
              (op stamp (-> Int64 Int64))
              (op audit (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((stamp (w) st
                  (match st
                    ((tuple total stamps)
                      (resume (+ (* w 10) (% (+ stamps 1) 10))
                              (tuple (+ total w) (+ stamps 1))))))
                 (audit () st
                  (match st
                    ((tuple total stamps)
                      (resume (% total 100) st)))))
                (let ((a (L.stamp (+ (% n 3) 2))))
                  (let ((b (L.stamp (let ((h (/ a 10))) (+ (* h 2) 1)))))
                    (let ((c (L.audit)))
                      (let ((d (L.stamp (let ((u (% b 10)))
                                          (let ((v (+ u (% c 10))))
                                            (+ (% v 7) 1))))))
                        (let ((e (L.audit)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3172103313 Int64))
  (call   main (: 0 Int64)) (output (: 2152073310 Int64)))
