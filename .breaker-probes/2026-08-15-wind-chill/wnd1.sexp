(case "wnd1 a WIND-CHILL stepper — gust sets the wind answering temp minus twice the wind CLAMPED at minus thirty (extremes counted), warm raises the temperature answering it, ext reads the extreme count, and only the cold seed drives one gust past the clamp so its rows ride deep negative while the warm seed never clamps"
  (input  (do
            (effect W
              (op gust (-> Int64 Int64))
              (op warm (-> Int64 Int64))
              (op ext (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (- n 5) (: 0 Int64))
                ((gust (v) st
                  (match st
                    ((tuple temp extremes)
                      (if (< (- temp (* v 2)) -30)
                          (resume -30 (tuple temp (+ extremes 1)))
                          (resume (- temp (* v 2)) st)))))
                 (warm (d) st
                  (match st
                    ((tuple temp extremes) (resume (+ temp d) (tuple (+ temp d) extremes)))))
                 (ext () st
                  (match st ((tuple temp extremes) (resume extremes st)))))
                (let ((a (W.gust 4)))
                  (let ((b (W.warm 6)))
                    (let ((c (W.gust 10)))
                      (let ((d (W.gust 16)))
                        (let ((e (W.warm 20)))
                          (let ((f (W.gust 16)))
                            (let ((g (W.ext)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -2890920690100 Int64))
  (call   main (: 0 Int64)) (output (: -12991929791099 Int64)))
