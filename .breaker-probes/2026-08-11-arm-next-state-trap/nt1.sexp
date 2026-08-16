(case "nst1 the arm's NEXT-STATE expression divides by a state-derived quantity — both signs thread, the zero seed traps"
  (input  (do
            (effect St (op step (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((step () s (resume s (/ 100 (- s 4)))))
                (+ (* 10 (St.step)) (St.step))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (trap "divide by zero")
  (call   main (: 2 Int64)) (output (: -30 Int64)))
