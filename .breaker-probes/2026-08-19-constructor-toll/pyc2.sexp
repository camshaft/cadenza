(case "pyc2 the CONSTRUCTED PAIR CARRIES BOTH THE REPLAY AND A CAPTURE — the arm builds a tuple of the resumed rest-of-body value and a state increment then the pattern binders feed a doubling and a thousandfold weight, the doubling COMPOUNDS across frames while the weights stack linearly, and the two digit ranges separate replay-flow errors from capture errors"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (tuple (resume (* s 10) (+ s 1)) (+ s 1))
                    ((tuple r w) (+ (* r 2) (* 1000 w))))))
                (+ (E.tick) (* 10 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 8840 Int64))
  (call   main (: 0 Int64)) (output (: 5400 Int64)))
