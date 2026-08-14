(case "vwp1 a VOLUME-WEIGHTED average price tracker — trade accrues (price*qty, qty) answering the running notional, vwap answers the truncated notional-over-volume or -1 before any trade, and the LEADING -1 row drives the whole packed total negative while the seed shifts one trade's price through both later readouts"
  (input  (do
            (effect V
              (op trade (-> Int64 Int64 Int64))
              (op vwap (-> Int64)))
            (def (main (: n Int64))
              (handle V (tuple (: 0 Int64) (: 0 Int64))
                ((trade (p qty) st
                  (match st
                    ((tuple pq q)
                      (resume (+ pq (* p qty)) (tuple (+ pq (* p qty)) (+ q qty))))))
                 (vwap () st
                  (match st
                    ((tuple pq q)
                      (if (= q 0)
                          (resume -1 st)
                          (resume (/ pq q) st))))))
                (let ((a (V.vwap)))
                  (let ((b (V.trade (+ n 5) 2)))
                    (let ((c (V.trade 8 3)))
                      (let ((d (V.vwap)))
                        (let ((e (V.trade 12 5)))
                          (let ((f (V.vwap)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -969945989885989 Int64))
  (call   main (: 0 Int64)) (output (: -989965993905991 Int64)))
