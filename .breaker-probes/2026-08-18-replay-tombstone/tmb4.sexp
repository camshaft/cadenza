(case "tmb4 a FOREIGN LEVY FEEDS THE DISCARDED REPLAY'S ANSWER — the arm resumes with an outer levy as the answer then discards the replay and answers a tombstone, the levy still fires and advances the outer thread even though its value flowed only into abandoned work, and the outer body's later levy reads five higher proving the discarded dataflow's effect landed"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5))))
                (+ (* 100 (handle E (: 1 Int64)
                            ((tick () s
                              (do (resume (T.levy) (+ s 1))
                                  (+ (* s 10) 7))))
                            (E.tick)))
                   (T.levy))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1706 Int64))
  (call   main (: 0 Int64)) (output (: 1705 Int64)))
