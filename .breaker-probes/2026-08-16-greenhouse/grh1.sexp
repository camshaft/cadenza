(case "grh1 a GREENHOUSE auto-vent — sunshine heats the house and crossing thirty with the vents SHUT trips them open (an eight-hundred alarm row with the temperature's low digits), misting adds three humidity but an open vent bleeds one back (a nine-tagged row), the read packs temperature humidity and vents, and the seed's starting warmth trips the auto-vent on the FIRST sun for one run and the SECOND for the other so every later humidity row shifts"
  (input  (do
            (effect G
              (op sun (-> Int64 Int64))
              (op mist (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle G (tuple (+ (: 24 Int64) (* (% n 3) 4)) (: 5 Int64) (: 0 Int64))
                ((sun (d) st
                  (match st
                    ((tuple t h v)
                      (if (if (> (+ t d) 30) (= v 0) false)
                          (resume (+ (: 800 Int64) (% (+ t d) 100)) (tuple (+ t d) h (: 1 Int64)))
                          (resume (+ (* (+ t d) 10) v) (tuple (+ t d) h v))))))
                 (mist () st
                  (match st
                    ((tuple t h v)
                      (if (= v 1)
                          (resume (+ (* (+ h 2) 10) 9) (tuple t (+ h 2) v))
                          (resume (* (+ h 3) 10) (tuple t (+ h 3) v))))))
                 (read () st
                  (match st
                    ((tuple t h v)
                      (resume (+ (* t 100) (+ (* h 10) v)) st)))))
                (let ((a (G.sun (: 4 Int64))))
                  (let ((b (G.mist)))
                    (let ((c (G.sun (: 3 Int64))))
                      (let ((d (G.mist)))
                        (let ((f (G.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 8320793510993591 Int64))
  (call   main (: 0 Int64)) (output (: 2800808311093201 Int64)))
