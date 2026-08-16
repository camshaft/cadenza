(case "rg2 the rotation DIRECTION flips on the evicted head's parity — even rotates left, odd rotates right, negative-odd exercises truncated mod"
  (input  (do
            (effect E (op pop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((pop () st (match st
                              ((tuple a b c)
                               (resume a (if (= (% a 2) 0)
                                             (tuple b c a)
                                             (tuple c a b)))))))
                (+ (E.pop) (+ (* 2 (E.pop)) (* 3 (E.pop))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 220 Int64))
  (call   main (: 0 Int64)) (output (: 320 Int64))
  (call   main (: -3 Int64)) (output (: 188 Int64)))
