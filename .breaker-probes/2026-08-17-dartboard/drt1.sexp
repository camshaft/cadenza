(case "drt1 a DARTBOARD countdown leg — every throw counts a dart, an overshoot BUSTS (counted, remaining untouched), hitting the EXACT remainder CHECKS OUT with a triple-seven row, an undershoot subtracts and packs the remainder with the dart count's low digit, the read packs remaining darts and busts, and the seed's starting score checks out mid-run on one leg (busting the follow-up) and on the LAST dart of the other (busting mid-run instead)"
  (input  (do
            (effect T
              (op throw (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (+ (: 20 Int64) (* (% n 3) 15)) (: 0 Int64) (: 0 Int64))
                ((throw (v) st
                  (match st
                    ((tuple rem darts busts)
                      (if (> v rem)
                          (resume (+ (: 900 Int64) (+ busts 1))
                                  (tuple rem (+ darts 1) (+ busts 1)))
                          (if (= v rem)
                              (resume (: 777 Int64)
                                      (tuple (: 0 Int64) (+ darts 1) busts))
                              (resume (+ (* (- rem v) 10) (% (+ darts 1) 10))
                                      (tuple (- rem v) (+ darts 1) busts)))))))
                 (read () st
                  (match st
                    ((tuple rem darts busts)
                      (resume (+ (* rem 100) (+ (* darts 10) busts)) st)))))
                (let ((a (T.throw (: 15 Int64))))
                  (let ((b (T.throw (: 12 Int64))))
                    (let ((c (T.throw (: 8 Int64))))
                      (let ((d (T.throw (: 5 Int64))))
                        (let ((f (T.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2010827779010041 Int64))
  (call   main (: 0 Int64)) (output (: 519019027770042 Int64)))
