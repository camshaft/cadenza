(case "ksc1 the KAPREKAR walk for three-digit numbers — step applies ninety-nine-times-the-digit-spread once (answering the step ordinal; a converged 495 answers the sticky forty-nine), where reads the current value over ten, and the seeds converge in four versus five steps so the same where probes catch one orbit mid-flight and the other already fixed"
  (input  (do
            (effect K
              (op step (-> Int64))
              (op where (-> Int64)))
            (def (dmax (: a Int64) (: b Int64) (: c Int64))
              (if (< a b) (if (< b c) c b) (if (< a c) c a)))
            (def (dmin (: a Int64) (: b Int64) (: c Int64))
              (if (< b a) (if (< c b) c b) (if (< c a) c a)))
            (def (kap (: v Int64))
              (* 99 (- (dmax (/ v 100) (% (/ v 10) 10) (% v 10))
                       (dmin (/ v 100) (% (/ v 10) 10) (% v 10)))))
            (def (main (: n Int64))
              (handle K (tuple (+ 100 (* n 21)) (: 0 Int64))
                ((step () st
                  (match st
                    ((tuple v steps)
                      (if (= v 495)
                          (resume 49 st)
                          (resume (+ steps 1) (tuple (kap v) (+ steps 1)))))))
                 (where () st
                  (match st ((tuple v steps) (resume (/ v 10) st)))))
                (let ((a (K.step)))
                  (let ((b (K.step)))
                    (let ((c (K.where)))
                      (let ((d (K.step)))
                        (let ((e (K.step)))
                          (let ((f (K.where)))
                            (let ((g (K.step)))
                              (let ((h (K.where)))
                                (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h)))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 102690304494949 Int64))
  (call   main (: 0 Int64)) (output (: 102890304690559 Int64)))
