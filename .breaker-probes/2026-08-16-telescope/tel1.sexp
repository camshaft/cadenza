(case "tel1 a TELESCOPE tracker with a slew-rate limit — track names a target locking within five degrees (else a nine-hundred miss with the gap's low digits), slew moves the azimuth toward the target by at most SEVEN degrees (a two-sided inline clamp of the signed difference) tagging an exact arrival, the read packs azimuth lock and arrival, and one seed starts within locking range so every row down to the read disagrees with the far-parked run"
  (input  (do
            (effect T
              (op track (-> Int64 Int64))
              (op slew (-> Int64))
              (op read (-> Int64)))
            (def (dist (: a Int64) (: b Int64))
              (if (> a b) (- a b) (- b a)))
            (def (main (: n Int64))
              (handle T (tuple (+ (: 10 Int64) (* (% n 3) 20)) (: 0 Int64) (: 0 Int64))
                ((track (t2) st
                  (match st
                    ((tuple az tg lk)
                      (if (<= (dist az t2) 5)
                          (resume (+ (: 100 Int64) (dist az t2)) (tuple az t2 (: 1 Int64)))
                          (resume (+ (: 900 Int64) (% (dist az t2) 100)) (tuple az t2 (: 0 Int64)))))))
                 (slew () st
                  (match st
                    ((tuple az tg lk)
                      (if (> (- tg az) 7)
                          (resume (* (+ az 7) 10) (tuple (+ az 7) tg lk))
                          (if (< (- tg az) -7)
                              (resume (* (- az 7) 10) (tuple (- az 7) tg lk))
                              (resume (+ (* tg 10) 1) (tuple tg tg lk)))))))
                 (read () st
                  (match st
                    ((tuple az tg lk)
                      (resume (+ (* az 100) (+ (* lk 10) (if (= az tg) 1 0))) st)))))
                (let ((a (T.track (: 27 Int64))))
                  (let ((b (T.slew)))
                    (let ((c (T.track (: 30 Int64))))
                      (let ((d (T.slew)))
                        (let ((f (T.read)))
                          (+ (* 10000 (+ (* 10000 (+ (* 10000 (+ (* 10000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1030271010303013011 Int64))
  (call   main (: 0 Int64)) (output (: 9170170091302402400 Int64)))
