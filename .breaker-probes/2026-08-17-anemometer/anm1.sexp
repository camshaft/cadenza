(case "anm1 an ANEMOMETER with a running average — each gust averages the new reading into the speed (integer halving of the sum) and a fresh peak tags the answer, a locked read echoes speed and peak without touching either (only counting itself), the read packs peak speed and locks, and the seed's starting wind makes the LAST gust a fresh peak on one mast but not the... both mast peaks land at different heights with the peak-tag pattern differing"
  (input  (do
            (effect A
              (op gust (-> Int64 Int64))
              (op lock (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle A (tuple (+ (: 4 Int64) (* (% n 3) 6)) (: 0 Int64) (: 0 Int64))
                ((gust (v) st
                  (match st
                    ((tuple speed peak lk)
                      (if (> (/ (+ speed v) 2) peak)
                          (resume (+ (* (/ (+ speed v) 2) 10) 1)
                                  (tuple (/ (+ speed v) 2) (/ (+ speed v) 2) lk))
                          (resume (* (/ (+ speed v) 2) 10)
                                  (tuple (/ (+ speed v) 2) peak lk))))))
                 (lock () st
                  (match st
                    ((tuple speed peak lk)
                      (resume (+ (* speed 10) (% peak 10)) (tuple speed peak (+ lk 1))))))
                 (read () st
                  (match st
                    ((tuple speed peak lk)
                      (resume (+ (* peak 100) (+ (* speed 10) lk)) st)))))
                (let ((a (A.gust (: 8 Int64))))
                  (let ((b (A.lock)))
                    (let ((c (A.gust (: 2 Int64))))
                      (let ((d (A.gust (: 12 Int64))))
                        (let ((f (A.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 910990500800981 Int64))
  (call   main (: 0 Int64)) (output (: 610660400810881 Int64)))
