(case "flk2 the FLOCK sensor at three pings — alignment within two recruits and drifts the heading one step toward the scout, the rogue is counted and echoed, the sub-quorum read packs heading followers and defectors, and the seeds disagree on WHICH scout is the rogue so the drifted headings differ"
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
                          (resume (+ (* heading 100) (+ (* fol 10) dfc)) st))))))
                (let ((a (S.ping (: 7 Int64))))
                  (let ((b (S.ping (: 4 Int64))))
                    (let ((c (S.ping (: 9 Int64))))
                      (let ((f (S.quorum)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 71904082821 Int64))
  (call   main (: 0 Int64)) (output (: 61052909521 Int64)))
