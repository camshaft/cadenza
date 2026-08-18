(case "pyx1 a PASSIVE PEEK READS THE SURVIVING REPLAY'S STATE — the tick double-replays with the discarded replay adding one and the survivor TRIPLING the state, a second op then reads the state without advancing it, and the peek must see the survivor's tripled thread rather than the discarded increment or the pre-replay value"
  (input  (do
            (effect E (op tick (-> Int64)) (op peek (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (* s 3))))
                 (peek () s (resume s s)))
                (+ (E.tick) (* 100 (E.peek)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 311 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
