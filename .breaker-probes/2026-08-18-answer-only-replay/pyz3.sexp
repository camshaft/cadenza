(case "pyz3 REPLAYS DIVERGING ONLY IN THE ANSWER — both resumes thread the SAME next-state but answer one apart so the second replay's plus-one signature must appear at BOTH dispatch depths of the two-perform body, and a lowering that collapses same-state replays into one or returns the first answer drops the low bit at each depth"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume (* s 10) (+ s 1))
                      (resume (+ (* s 10) 1) (+ s 1)))))
                (+ (E.tick) (* 100 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2111 Int64))
  (call   main (: 0 Int64)) (output (: 1101 Int64)))
