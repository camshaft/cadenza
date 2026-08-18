(case "pyk3 a FIBONACCI-ROTATING TUPLE STATE UNDER TOLLS — each dispatch answers a positional pack of both fields then rotates the pair Fibonacci-style while its toll charges a thousandfold of the FIRST field as captured, the rotation means each frame's captured first field is the previous frame's second, and a toll reading the rotated tuple instead of the captured one shifts the thousands by the rotation"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (% n 3) (: 1 Int64))
                ((tick () st
                  (match st
                    ((tuple a b)
                      (+ (resume (+ (* a 10) b) (tuple b (+ a b)))
                         (* 1000 a))))))
                (+ (E.tick) (* 100 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3211 Int64))
  (call   main (: 0 Int64)) (output (: 2101 Int64)))
