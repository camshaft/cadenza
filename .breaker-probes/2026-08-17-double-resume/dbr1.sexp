(case "dbr1 SEQUENTIAL DOUBLE RESUME in one arm — the arm resumes once discarding the outcome then resumes AGAIN with a shifted answer and state, a single-perform body so each resume replays just the tail, probing whether continuations are one-shot (second resume must be a defined error) or multi-shot (the second replay's value wins)"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 1))
                      (resume (+ s 10) (+ s 2)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
