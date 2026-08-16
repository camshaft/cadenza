(case "ra4 a recursion drawing TWO OPS of ONE effect per round, then a trailing draw — the one-effect-two-ops control for the two-effect fork"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (E.next)))
                (let ((b (E.probe)))
                  (if (< a 20) (walk (+ k 1)) k))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5)))
                 (probe () s (resume s (+ s 1))))
                (let ((steps (walk 0)))
                  (+ (* 100 steps) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 329 Int64))
  (call   main (: 1 Int64)) (output (: 431 Int64))
  (call   main (: -4 Int64)) (output (: 426 Int64)))
