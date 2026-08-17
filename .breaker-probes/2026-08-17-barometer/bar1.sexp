(case "bar1 a BAROMETER with trend and storm flags — each sample signs the delta against the standing pressure (the sgn seeding the trend field), a FALLING trend below the seed-shifted storm line counts a storm (nine-hundred row), otherwise the answer packs the reading with the trend offset by one, the read packs pressure storms and trend, and the same four readings storm TWICE on the low threshold but once on the high with the second storm's row downgraded to a plain fall"
  (input  (do
            (effect B
              (op sample (-> Int64 Int64))
              (op read (-> Int64)))
            (def (sgn (: d Int64))
              (if (> d 0) 1 (if (< d 0) (: -1 Int64) 0)))
            (def (main (: n Int64))
              (handle B (tuple (: 30 Int64) (: 0 Int64) (: 0 Int64))
                ((sample (p) st
                  (match st
                    ((tuple pressure trend storms)
                      (if (if (= (sgn (- p pressure)) (: -1 Int64)) (< p (+ (: 27 Int64) (% n 3))) false)
                          (resume (+ (: 900 Int64) (+ storms 1))
                                  (tuple p (sgn (- p pressure)) (+ storms 1)))
                          (resume (+ (* p 10) (+ (sgn (- p pressure)) 1))
                                  (tuple p (sgn (- p pressure)) storms))))))
                 (read () st
                  (match st
                    ((tuple pressure trend storms)
                      (resume (+ (* pressure 100) (+ (* storms 10) (+ trend 1))) st)))))
                (let ((a (B.sample (: 28 Int64))))
                  (let ((b (B.sample (: 27 Int64))))
                    (let ((c (B.sample (: 29 Int64))))
                      (let ((d (B.sample (: 26 Int64))))
                        (let ((f (B.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2809012929022620 Int64))
  (call   main (: 0 Int64)) (output (: 2802702929012610 Int64)))
