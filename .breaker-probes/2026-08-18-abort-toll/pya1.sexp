(case "pya1 ABORT-OR-TOLL — the arm either answers WITHOUT resuming (a thousandfold abort tag) or resumes and adds a hundredfold toll, an abort deep in the pyramid still returns THROUGH the pending outer frames' tolls, and the seed picks which dispatch aborts so one run tolls-then-aborts while the other completes both tolls"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (if (> s 1)
                      (+ (* s 1000) 9)
                      (+ (resume s (+ s 1)) (* 100 s)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2109 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
