(case "ema1 an INTEGER EMA state — each dispatch blends (3*ema + 100*v)/4 at 100x scale, convergence toward the fed value from both sides"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (* n 100)
                ((feed (v) ema
                  (let ((nema (/ (+ (* ema 3) (* v 100)) 4)))
                    (resume (/ nema 100) nema))))
                (let ((a (S.feed 8)))
                  (let ((b (S.feed 8)))
                    (let ((c (S.feed 8)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 20304 Int64))
  (call   main (: 16 Int64)) (output (: 141211 Int64)))
