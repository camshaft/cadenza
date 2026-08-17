(case "chn1 a BUTTER churn — churning turns a QUARTER of the cream to butter floored at one whole scoop (the empty churn answering nine hundred), pouring past ten SPILLS the excess (an eight-hundred row with the excess and the running spill's low digit), the read packs butter cream and spills, and the seed's cream quarters to different scoops every churn with one pail spilling its pour and the other taking it whole"
  (input  (do
            (effect B
              (op pour (-> Int64 Int64))
              (op churn (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (+ (: 3 Int64) (* (% n 3) 6)) (: 0 Int64) (: 0 Int64))
                ((pour (k) st
                  (match st
                    ((tuple cream butter sp)
                      (if (> (+ cream k) 10)
                          (resume (+ (: 800 Int64)
                                     (+ (* (- (+ cream k) 10) 10)
                                        (% (+ sp (- (+ cream k) 10)) 10)))
                                  (tuple (: 10 Int64) butter (+ sp (- (+ cream k) 10))))
                          (resume (+ (* (+ cream k) 10) (% k 10))
                                  (tuple (+ cream k) butter sp))))))
                 (churn () st
                  (match st
                    ((tuple cream butter sp)
                      (if (= cream 0)
                          (resume (: 900 Int64) st)
                          (if (< (/ cream 4) 1)
                              (resume (+ (: 10 Int64) (% (- cream 1) 10))
                                      (tuple (- cream 1) (+ butter 1) sp))
                              (resume (+ (* (/ cream 4) 10) (% (- cream (/ cream 4)) 10))
                                      (tuple (- cream (/ cream 4)) (+ butter (/ cream 4)) sp)))))))
                 (read () st
                  (match st
                    ((tuple cream butter sp)
                      (resume (+ (* butter 100) (+ (* cream 10) sp)) st)))))
                (let ((a (B.churn)))
                  (let ((b (B.pour (: 4 Int64))))
                    (let ((c (B.churn)))
                      (let ((f (B.read)))
                        (+ (* 10000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 278110280481 Int64))
  (call   main (: 0 Int64)) (output (: 120640150250 Int64)))
