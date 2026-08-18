(case "pyt7 the INIT TRAPS IN VALUE POSITION — the handler's starting value divides by the seed's residue so the zero seed traps BEFORE any frame installs or any dispatch runs, completing the trap-position triple: init traps eagerly, a discarded pure trap is elided, and a toll trap fires only at unwind"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (/ 60 (% n 3))
                ((tick () s (+ (resume s (+ s 1)) (* 1000 s))))
                (let ((a (E.tick)))
                  (+ a (* 10 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 121670 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))
