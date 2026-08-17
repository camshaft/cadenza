(case "lom1 a LOOM shuttling weft rows — each weave FLIPS the shuttle and a leftward return completes a row (an even completed row answers a seven-hundred pattern row, a rightward pass answers plain), a mend counts a break and unpicks one row floored at zero, the read packs row shuttle and breaks, and the seed's starting shuttle direction PHASE-SHIFTS which weaves complete rows so the pattern row fires on one run only"
  (input  (do
            (effect W
              (op weave (-> Int64))
              (op mend (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (: 0 Int64) (if (> (% n 3) 0) 1 0) (: 0 Int64))
                ((weave () st
                  (match st
                    ((tuple row d br)
                      (if (= d 1)
                          (if (= (% (+ row 1) 2) 0)
                              (resume (+ (: 700 Int64) (* (+ row 1) 10)) (tuple (+ row 1) (: 0 Int64) br))
                              (resume (* (+ row 1) 10) (tuple (+ row 1) (: 0 Int64) br)))
                          (resume (+ (* row 10) 1) (tuple row (: 1 Int64) br))))))
                 (mend () st
                  (match st
                    ((tuple row d br)
                      (if (> row 0)
                          (resume (+ (* (+ br 1) 10) (- row 1)) (tuple (- row 1) d (+ br 1)))
                          (resume (* (+ br 1) 10) (tuple row d (+ br 1)))))))
                 (read () st
                  (match st
                    ((tuple row d br)
                      (resume (+ (* row 100) (+ (* d 10) br)) st)))))
                (let ((a (W.weave)))
                  (let ((b (W.weave)))
                    (let ((c (W.weave)))
                      (let ((m (W.mend)))
                        (let ((f (W.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) m)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10011720011101 Int64))
  (call   main (: 0 Int64)) (output (: 1010011010011 Int64)))
