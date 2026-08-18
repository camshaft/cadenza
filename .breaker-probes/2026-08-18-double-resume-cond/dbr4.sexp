(case "dbr4 CONDITIONAL DOUBLE REPLAY — a positive state replays the tail twice with the second replay winning while a zero state resumes once, the seed places the single-replay frame at different depths so one run double-replays at BOTH dispatches and the other threads a single first dispatch into a doubled second, mixing the one-shot and multi-shot paths in a single machine"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (if (> s 0)
                      (do (resume s (+ s 1))
                          (resume (+ s 10) (+ s 2)))
                      (resume s (+ s 1)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 141 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
