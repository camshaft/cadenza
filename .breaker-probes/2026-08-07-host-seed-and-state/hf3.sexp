(case "hf3 a host call SANDWICHED between two in-program dispatches — state survives the boundary crossing"
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (host (ask)
                (handle St 0
                  ((next (u) s (resume s (+ s 1))))
                  (let ((a (St.next)))
                    (let ((h (ask.ask)))
                      (let ((b (St.next)))
                        (+ (* 100 a) (+ (* 10 h) b))))))))
            (export main)))
  (call   main (: 0 Int64))
  (host-responses (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask))
  (output (: 71 Int64)))
