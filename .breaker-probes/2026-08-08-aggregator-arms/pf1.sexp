(case "pf1 an AGGREGATOR arm — the op returns the running sum of every value fed to it, three feeds pin the accumulation"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E 0
                ((feed (x) s (resume (+ s x) (+ s x))))
                (let ((r1 (E.feed n)))
                  (let ((r2 (E.feed 7)))
                    (let ((r3 (E.feed n)))
                      (+ (* 100 r1) (+ (* 10 r2) r3)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 413 Int64))
  (call   main (: 0 Int64)) (output (: 77 Int64))
  (call   main (: -2 Int64)) (output (: -147 Int64)))
