(case "dbr6 the two replays thread DIFFERENT STATES — the discarded first replay advances the state additively while the surviving second replay DOUBLES it, the next dispatch answers from whichever state its replay actually threaded, and a lowering that reuses the first replay's state thread for the second shifts the later dispatch's whole answer"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (* s 2)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 131 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
