(case "dbr2 DOUBLE RESUME over a TWO-PERFORM body — each of the two dispatches replays its tail twice so the body runs through FOUR leaf executions, the multi-shot second-replay-wins rule composes multiplicatively (the surviving answer threads the second replay at BOTH depths), and the seed shifts the surviving pair together"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (+ s 2)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 141 Int64))
  (call   main (: 0 Int64)) (output (: 130 Int64)))
