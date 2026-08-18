(case "pyr8 a POST-RESUME TOLL over a DESTRUCTURED TUPLE STATE — the arm matches the pair out of state then adds a thousandfold toll packing BOTH fields as captured at its own dispatch, the two frames' tolls unwind innermost-first with different field pairs, and a toll reading the post-resume tuple or the other frame's binding misprices both digits at once"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (% n 3) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple v k)
                      (+ (resume v (tuple (+ v 2) (+ k 1)))
                         (* 1000 (+ (* v 10) k)))))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 41031 Int64))
  (call   main (: 0 Int64)) (output (: 21020 Int64)))
