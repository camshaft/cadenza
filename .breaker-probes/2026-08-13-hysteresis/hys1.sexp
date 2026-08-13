(case "hys1 a HYSTERESIS gate — the output turns on at eight and off at three, HOLDING between the thresholds; the same mid-band feed answers differently depending on which side entered it"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S 0
                ((feed (v) out
                  (let ((o2 (if (>= v 8) 1 (if (<= v 3) 0 out))))
                    (resume (+ (* o2 10) (if (and (> v 3) (< v 8)) 1 0)) o2))))
                (let ((a (S.feed n)))
                  (let ((b (S.feed 5)))
                    (let ((c (S.feed 2)))
                      (let ((d (S.feed 6)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 10110001 Int64))
  (call   main (: 5 Int64)) (output (: 1010001 Int64)))
