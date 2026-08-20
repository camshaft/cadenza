(case "pymt2 probe: a THREE-WAY data-dependent resume — the arm branches on (% s 3) via nested if and resumes differently in each of three branches (s%3=0 -> (* s 100) thread +1; =1 -> (* s 10) thread +2; else -> s thread +3); four dispatches walk a state-dependent path across all three resume sites, so the tail fold reconverges three distinct resume calls each with its own answer AND state advance"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 4)
      ((tick () s (if (= (% s 3) (: 0 Int64)) (resume (* s 100) (+ s 1))
                    (if (= (% s 3) (: 1 Int64)) (resume (* s 10) (+ s 2))
                      (resume s (+ s 3))))))
      (+ (E.tick) (+ (E.tick) (+ (E.tick) (E.tick))))))
  (export main)))
  (call   main (: 1 Int64)) (output (: 950 Int64))
  (call   main (: 0 Int64)) (output (: 350 Int64)))
