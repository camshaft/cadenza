(case "ra9 MUTUAL pair each drawing ONCE (two draws per CYCLE), trailing draw"
  (input  (do
            (effect E (op next (-> Int64)) (op tick (-> Int64)))
            (def (f (: k Int64))
              (let ((a (E.next)))
                (g k a)))
            (def (g (: k Int64) (: a Int64))
              (let ((b (E.tick)))
                (if (< a 20) (f (+ k 1)) k)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5)))
                 (tick () s (resume s (+ s 1))))
                (let ((steps (f 0)))
                  (+ (* 100 steps) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 329 Int64))
  (call   main (: 1 Int64)) (output (: 431 Int64)))
