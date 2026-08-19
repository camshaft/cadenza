(case "pyab1 probe: resume answer BRANCHES on op arg vs captured state (if v>s then 100v else v+s); with v=1 the branch FLIPS by seed — only seed 0 crosses the threshold, so the first dispatch scales by 100 for n=0 but adds for higher seeds"
  (input (do
  (effect E (op tick (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick (v) s
        (resume (if (> v s) (* v 100) (+ v s)) (+ s 1))))
      (let ((a (E.tick 1)))
        (let ((b (E.tick 1)))
          (+ (* 100 a) b)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 203 Int64))
  (call   main (: 0 Int64)) (output (: 10002 Int64)))
