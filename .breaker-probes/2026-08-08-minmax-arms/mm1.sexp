(case "mm1 a MIN/MAX tracking arm — three feeds tighten the (lo,hi) tuple state, readers project the final spread"
  (input  (do
            (effect E (op feed (-> Int64 Int64)) (op lo (-> Int64)) (op hi (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 1000 -1000)
                ((feed (x) s (match s
                               ((tuple l h) (resume x (tuple (if (< x l) x l)
                                                             (if (> x h) x h))))))
                 (lo () s (match s ((tuple l h) (resume l s))))
                 (hi () s (match s ((tuple l h) (resume h s)))))
                (do (E.feed n)
                    (E.feed 7)
                    (E.feed (- 0 n))
                    (+ (* 100 (E.lo)) (E.hi)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -293 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64))
  (call   main (: -9 Int64)) (output (: -891 Int64)))
