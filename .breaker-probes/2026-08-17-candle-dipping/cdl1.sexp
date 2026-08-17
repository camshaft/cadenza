(case "cdl1 a CANDLE dipping rig — each dip grows the taper by six minus half its thickness FLOORED at one (thin candles gain fastest, the growth in the answer with the new thickness's low digit), trimming a taper over eight DRIPS three off (counted, seven-hundred row) else answers eight-hundred plus the standing thickness, the read packs thickness dips and drips, and the seed's bare-vs-primed wick walks different growth curves to the same drip"
  (input  (do
            (effect W
              (op dip (-> Int64))
              (op trim (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (* (% n 3) 4) (: 0 Int64) (: 0 Int64))
                ((dip () st
                  (match st
                    ((tuple th dips drips)
                      (if (< (- 6 (/ th 2)) 1)
                          (resume (+ (: 10 Int64) (% (+ th 1) 10))
                                  (tuple (+ th 1) (+ dips 1) drips))
                          (resume (+ (* (- 6 (/ th 2)) 10) (% (+ th (- 6 (/ th 2))) 10))
                                  (tuple (+ th (- 6 (/ th 2))) (+ dips 1) drips))))))
                 (trim () st
                  (match st
                    ((tuple th dips drips)
                      (if (> th 8)
                          (resume (+ (: 700 Int64) (+ drips 1))
                                  (tuple (- th 3) dips (+ drips 1)))
                          (resume (+ (: 800 Int64) th) st)))))
                 (read () st
                  (match st
                    ((tuple th dips drips)
                      (resume (+ (* th 100) (+ (* dips 10) drips)) st)))))
                (let ((a (W.dip)))
                  (let ((b (W.trim)))
                    (let ((c (W.dip)))
                      (let ((d (W.trim)))
                        (let ((f (W.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 488080207010721 Int64))
  (call   main (: 0 Int64)) (output (: 668060397010621 Int64)))
