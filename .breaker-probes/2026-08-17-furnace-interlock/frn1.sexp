(case "frn1 a FURNACE interlock composing BOOL-RETURNING OPS under and or and not — hot answers whether the temperature clears the ignition line advancing one degree, cold answers the temperature's evenness advancing two, the and skips its right draw when hot reports low, the or skips its right draw when cold reports even, the not inverts a lone hot draw, every skip is visible in the tally's call count, and the seed's starting temperature makes one run short-circuit where the other evaluates both sides"
  (input  (do
            (effect L
              (op hot (-> Bool))
              (op cold (-> Bool))
              (op tally (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (% n 3) (: 0 Int64))
                ((hot () st
                  (match st
                    ((tuple t calls)
                      (resume (>= t 2) (tuple (+ t 1) (+ calls 1))))))
                 (cold () st
                  (match st
                    ((tuple t calls)
                      (resume (= (% t 2) 0) (tuple (+ t 2) (+ calls 1))))))
                 (tally () st
                  (match st
                    ((tuple t calls)
                      (resume (+ (* t 10) calls) st)))))
                (let ((a (if (and (L.hot) (L.cold)) (: 1 Int64) (: 2 Int64))))
                  (let ((p (L.tally)))
                    (let ((b (if (or (L.cold) (L.hot)) (: 3 Int64) (: 4 Int64))))
                      (let ((q (L.tally)))
                        (let ((c (if (not (L.hot)) (: 5 Int64) (: 6 Int64))))
                          (let ((r (L.tally)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) p)) b)) q)) c)) r)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 22103420653 Int64))
  (call   main (: 0 Int64)) (output (: 21103430654 Int64)))
