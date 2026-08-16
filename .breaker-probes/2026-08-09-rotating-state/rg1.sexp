(case "rg1 the arm ROTATES the tuple state (a b c)->(b c a) per dispatch and returns the evicted head — four weighted reads wrap the ring"
  (input  (do
            (effect E (op pop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((pop () st (match st
                              ((tuple a b c) (resume a (tuple b c a))))))
                (+ (E.pop)
                   (+ (* 2 (E.pop))
                      (+ (* 3 (E.pop))
                         (* 4 (E.pop)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 345 Int64))
  (call   main (: 0 Int64)) (output (: 320 Int64))
  (call   main (: -3 Int64)) (output (: 305 Int64)))
