(case "zpl1 a ZIPLINE with a brake zone — gliding advances the position by speed times the hang time and gravity feeds the speed capped at nine, a brake in the zone past ten sheds four speed floored at one (a clean one-tag) while an early brake SCRAPES (counted, shedding one), the read packs position speed and scrapes, and the seed's launch speed reaches the zone before the brake on one line while the other scrapes short of it"
  (input  (do
            (effect Z
              (op glide (-> Int64 Int64))
              (op brake (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle Z (tuple (+ (: 2 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((glide (t) st
                  (match st
                    ((tuple speed pos sc)
                      (if (> (+ speed 2) 9)
                          (resume (+ (* (% (+ pos (* speed t)) 100) 10) 9)
                                  (tuple (: 9 Int64) (+ pos (* speed t)) sc))
                          (resume (+ (* (% (+ pos (* speed t)) 100) 10) (% (+ speed 2) 10))
                                  (tuple (+ speed 2) (+ pos (* speed t)) sc))))))
                 (brake () st
                  (match st
                    ((tuple speed pos sc)
                      (if (>= pos 10)
                          (if (< (- speed 4) 1)
                              (resume (: 11 Int64) (tuple (: 1 Int64) pos sc))
                              (resume (+ (* (- speed 4) 10) 1) (tuple (- speed 4) pos sc)))
                          (if (< (- speed 1) 1)
                              (resume (+ (: 900 Int64) (+ sc 1)) (tuple (: 1 Int64) pos (+ sc 1)))
                              (resume (+ (: 900 Int64) (+ sc 1)) (tuple (- speed 1) pos (+ sc 1))))))))
                 (read () st
                  (match st
                    ((tuple speed pos sc)
                      (resume (+ (* pos 100) (+ (* speed 10) sc)) st)))))
                (let ((a (Z.glide (: 2 Int64))))
                  (let ((b (Z.brake)))
                    (let ((c (Z.glide (: 1 Int64))))
                      (let ((f (Z.read)))
                        (+ (* 10000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 869011371371 Int64))
  (call   main (: 0 Int64)) (output (: 449010750751 Int64)))
