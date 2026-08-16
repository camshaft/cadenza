(case "flk1 a FLOCK quorum sensor — each scout ping within two of the flock heading recruits a follower and DRIFTS the heading one step toward the scout, a rogue ping is counted and echoed back unchanged, quorum fires at three followers answering the settled heading, and the seed's initial heading decides WHICH scout reads as the rogue while both flocks still reach quorum"
  (input  (do
            (effect S
              (op ping (-> Int64 Int64))
              (op quorum (-> Int64)))
            (def (sgn (: d Int64))
              (if (> d 0) 1 (if (< d 0) (: -1 Int64) 0)))
            (def (main (: n Int64))
              (handle S (tuple (+ (: 5 Int64) (% n 3)) (: 0 Int64) (: 0 Int64))
                ((ping (h) st
                  (match st
                    ((tuple heading fol dfc)
                      (if (if (<= (- h heading) 2) (<= (- heading h) 2) false)
                          (resume (+ (* (+ heading (sgn (- h heading))) 10) (+ fol 1))
                                  (tuple (+ heading (sgn (- h heading))) (+ fol 1) dfc))
                          (resume (+ (: 900 Int64) h)
                                  (tuple heading fol (+ dfc 1)))))))
                 (quorum () st
                  (match st
                    ((tuple heading fol dfc)
                      (if (>= fol 3)
                          (resume (+ (* heading 100) 1) st)
                          (resume (+ (: 400 Int64) (+ (* fol 10) dfc)) st))))))
                (let ((a (S.ping (: 7 Int64))))
                  (let ((b (S.ping (: 4 Int64))))
                    (let ((c (S.ping (: 9 Int64))))
                      (let ((d (S.ping (: 6 Int64))))
                        (let ((f (S.quorum)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 71904082073701 Int64))
  (call   main (: 0 Int64)) (output (: 61052909063601 Int64)))
