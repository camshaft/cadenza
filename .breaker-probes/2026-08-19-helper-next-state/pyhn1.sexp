(case "pyhn1 probe: the resume NEXT-STATE is computed by a top-level pure HELPER function (nxt s = 2s+1) called from the arm — each dispatch answers ten-times the state and threads nxt(s), so the state follows a 2x+1 recurrence across three dispatches and the cross-function next-state call must thread correctly"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (nxt (: x Int64)) (+ (* x 2) 1))
  (def (main (: n Int64))
    (handle E (+ (% n 3) (: 1 Int64))
      ((tick () s (resume (* s 10) (nxt s))))
      (+ (E.tick) (+ (* 100 (E.tick)) (* 10000 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1105020 Int64))
  (call   main (: 0 Int64)) (output (: 703010 Int64)))
