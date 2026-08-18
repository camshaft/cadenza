(case "dbr3 BOTH REPLAY VALUES CONSUMED — the arm SUMS two sequential resumes so neither replay is discarded, the answer is the two replays' body values added (each replay runs the whole tail with its own answer), and the multi-shot second-replay-wins rule from the discard shape gives way to both-contribute when the arm keeps both"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (resume s (+ s 1))
                     (resume (+ s 10) (+ s 2)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
