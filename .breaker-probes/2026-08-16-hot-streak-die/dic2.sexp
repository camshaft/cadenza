(case "dic2 the HOT-STREAK die at three rolls — LCG state, face five-or-higher deepens the streak scoring face times depth, reset scores plain, and one seed opens with a DOUBLE streak the other breaks immediately"
  (input  (do
            (effect D
              (op roll (-> Int64))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle D (tuple (* (% n 3) 7) (: 0 Int64) (: 0 Int64))
                ((roll () st
                  (match st
                    ((tuple s streak score)
                      (if (>= (+ (% (% (+ (* s 7) 5) 31) 6) 1) 5)
                          (resume (+ (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) 100)
                                     (+ (* (+ streak 1) 10)
                                        (% (+ score (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) (+ streak 1))) 10)))
                                  (tuple (% (+ (* s 7) 5) 31)
                                         (+ streak 1)
                                         (+ score (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) (+ streak 1)))))
                          (resume (+ (* (+ (% (% (+ (* s 7) 5) 31) 6) 1) 100)
                                     (% (+ score (+ (% (% (+ (* s 7) 5) 31) 6) 1)) 10))
                                  (tuple (% (+ (* s 7) 5) 31)
                                         (: 0 Int64)
                                         (+ score (+ (% (% (+ (* s 7) 5) 31) 6) 1))))))))
                 (tally () st
                  (match st
                    ((tuple s streak score) (resume (+ (* score 10) streak) st)))))
                (let ((a (D.roll)))
                  (let ((b (D.roll)))
                    (let ((c (D.roll)))
                      (let ((f (D.tally)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 616628301210 Int64))
  (call   main (: 0 Int64)) (output (: 616400101110 Int64)))
