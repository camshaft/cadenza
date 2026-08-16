(case "ac1 an ASCENT counter — the tuple state carries the previous payload, each dispatch compares and conditionally bumps"
  (input  (do
            (effect W (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle W (tuple n 0)
                ((feed (v) st
                  (match st
                    ((tuple prev hits)
                      (let ((nh (if (< prev v) (+ hits 1) hits)))
                        (resume nh (tuple v nh)))))))
                (+ (W.feed 5) (+ (* 10 (W.feed 3)) (+ (* 100 (W.feed 9)) (* 1000 (W.feed 9)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2211 Int64))
  (call   main (: 8 Int64)) (output (: 1100 Int64)))
