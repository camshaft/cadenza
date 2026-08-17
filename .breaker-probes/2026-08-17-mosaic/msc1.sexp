(case "msc1 a MOSAIC bench with tile nipping — laying takes whole tiles when the tray covers the course else lays the REMNANT recording the shortfall as waste (an eight-hundred row with the gap), nipping adds tiles and chips a third into waste, the read packs tray laid and waste, and the seed's tray covers every course on one bench while the other runs SHORT on the final course so the waste ledgers split"
  (input  (do
            (effect M
              (op lay (-> Int64 Int64))
              (op nip (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle M (tuple (+ (: 5 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((lay (k) st
                  (match st
                    ((tuple tiles laid waste)
                      (if (>= tiles k)
                          (resume (+ (* (+ laid k) 10) (% (- tiles k) 10))
                                  (tuple (- tiles k) (+ laid k) waste))
                          (resume (+ (: 800 Int64) (- k tiles))
                                  (tuple (: 0 Int64) (+ laid tiles) (+ waste (- k tiles))))))))
                 (nip (k) st
                  (match st
                    ((tuple tiles laid waste)
                      (resume (+ (* (+ tiles k) 10) (% (+ waste (/ k 3)) 10))
                              (tuple (+ tiles k) laid (+ waste (/ k 3)))))))
                 (read () st
                  (match st
                    ((tuple tiles laid waste)
                      (resume (+ (* tiles 100) (+ (* laid 10) waste)) st)))))
                (let ((a (M.lay (: 4 Int64))))
                  (let ((b (M.nip (: 6 Int64))))
                    (let ((c (M.lay (: 7 Int64))))
                      (let ((d (M.lay (: 3 Int64))))
                        (let ((f (M.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 441021131400142 Int64))
  (call   main (: 0 Int64)) (output (: 410721108030115 Int64)))
