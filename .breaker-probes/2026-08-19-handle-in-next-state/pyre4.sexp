(case "pyre4 an IF-WRAPPED handle in next-state GATES the miscompile to the handle branch — the positive seed selects the closed pure handle (which should thread its value 42 but re-splices) while the zero seed selects a pure constant and threads correctly, isolating the bug to the handle-valued arm of the same next-state slot" (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (+ (resume (* s 10)
                   (if (> s 0)
                       (handle E (: 40 Int64) ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2))
                       (: 3 Int64)))
           (* 1000 s))))
      (+ (E.tick) (* 10 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 47210 Int64))
  (call   main (: 0 Int64)) (output (: 3300 Int64)))
