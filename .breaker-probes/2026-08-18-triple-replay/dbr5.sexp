(case "dbr5 TRIPLE SEQUENTIAL REPLAY — three resumes in one do with the first two outcomes discarded, the LAST replay's value wins extending the second-wins law to n-th-wins, each replay shifting the answer by ten so a lowering that stops at two replays or returns any earlier replay's value is off by a fixed decade"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (+ s 2))
                      (resume (+ s 20) (+ s 3)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 21 Int64))
  (call   main (: 0 Int64)) (output (: 20 Int64)))
