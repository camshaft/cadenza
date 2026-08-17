(case "ovn1 a BAKERY OVEN with burn risk — a bake's doneness is minutes times temperature over ten (integer), OVER twelve burns the loaf (counted, the open door dropping the temperature two), eight to twelve is PERFECT (loaf counted, one-tagged), under eight answers plain with nothing changed, heating raises the oven, the read packs loaves temperature and burnt, and the seed's starting oven makes the same schedule bake one PERFECT loaf and one BURNT on one run against two clean loaves on the other"
  (input  (do
            (effect B
              (op bake (-> Int64 Int64))
              (op heat (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle B (tuple (+ (: 8 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((bake (t) st
                  (match st
                    ((tuple temp loaves burnt)
                      (if (> (/ (* t temp) 10) 12)
                          (resume (+ (: 900 Int64) (+ burnt 1))
                                  (tuple (- temp 2) loaves (+ burnt 1)))
                          (if (>= (/ (* t temp) 10) 8)
                              (resume (+ (* (/ (* t temp) 10) 10) 1)
                                      (tuple temp (+ loaves 1) burnt))
                              (resume (* (/ (* t temp) 10) 10) st))))))
                 (heat (d) st
                  (match st
                    ((tuple temp loaves burnt)
                      (resume (* (+ temp d) 10) (tuple (+ temp d) loaves burnt)))))
                 (read () st
                  (match st
                    ((tuple temp loaves burnt)
                      (resume (+ (* loaves 100) (+ (* temp 10) burnt)) st)))))
                (let ((a (B.bake (: 9 Int64))))
                  (let ((b (B.heat (: 3 Int64))))
                    (let ((c (B.bake (: 11 Int64))))
                      (let ((d (B.bake (: 7 Int64))))
                        (let ((f (B.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 91130901070211 Int64))
  (call   main (: 0 Int64)) (output (: 70110121070210 Int64)))
