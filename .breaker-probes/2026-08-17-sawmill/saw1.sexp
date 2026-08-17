(case "saw1 a SAWMILL with a dulling blade — each cut yields the length MINUS half the blade wear (integer division of the PRE-cut wear) dulling the blade by two, a blade at six or duller JAMS (counted, resharpened to zero, nothing cut), the read packs planks blade and jams, and the seed pre-dulls the blade so the JAM lands mid-run on one seed and on the LAST cut on the other with every yield shifted between"
  (input  (do
            (effect M
              (op cut (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (* (% n 3) 3) (: 0 Int64) (: 0 Int64))
                ((cut (ln) st
                  (match st
                    ((tuple blade planks jams)
                      (if (>= blade 6)
                          (resume (+ (: 900 Int64) (+ jams 1))
                                  (tuple (: 0 Int64) planks (+ jams 1)))
                          (resume (+ (* (- ln (/ blade 2)) 10) (% (+ blade 2) 10))
                                  (tuple (+ blade 2) (+ planks (- ln (/ blade 2))) jams))))))
                 (read () st
                  (match st
                    ((tuple blade planks jams)
                      (resume (+ (* planks 100) (+ (* blade 10) jams)) st)))))
                (let ((a (M.cut (: 5 Int64))))
                  (let ((b (M.cut (: 4 Int64))))
                    (let ((c (M.cut (: 6 Int64))))
                      (let ((d (M.cut (: 3 Int64))))
                        (let ((f (M.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 450279010320921 Int64))
  (call   main (: 0 Int64)) (output (: 520340469011201 Int64)))
