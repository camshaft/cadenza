(case "pyx5 BOTH ARMS TOLLED IN A MULTI-OP EFFECT with an additive poke between tolled ticks — the tick tolls a thousandfold of its state and the poke a hundred-thousandfold of its argument, three dispatches stack tick-poke-tick tolls unwinding innermost-first, and the poke's state shift feeds the last tick's answer and toll both"
  (input  (do
            (effect E (op tick (-> Int64)) (op poke (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume (* s 10) (+ s 1)) (* 1000 s)))
                 (poke (v) s (+ (resume (+ s v) (+ s v)) (* 100000 v))))
                (+ (E.tick)
                   (+ (* 10 (E.poke 7))
                      (* 100 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 719100 Int64))
  (call   main (: 0 Int64)) (output (: 716080 Int64)))
