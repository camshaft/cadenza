(case "ra5 recursion drawing the SAME op TWICE per round, trailing draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((a (E.next)))
                (let ((b (E.next)))
                  (if (< a 20) (walk (+ k 1)) k))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (let ((steps (walk 0)))
                  (+ (* 100 steps) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 235 Int64))
  (call   main (: 1 Int64)) (output (: 231 Int64))
  (call   main (: -4 Int64)) (output (: 336 Int64)))
