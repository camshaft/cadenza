(case "pyt4 a POST-RESUME TOLL THAT TRAPS — each frame's toll divides a hundred by its captured pre-resume state so the zero seed's FIRST frame traps at unwind AFTER the whole body already ran to completion, the nonzero seed pays both quotient tolls cleanly, and a lowering that evaluates the toll before the resume would trap before the body ran at all"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (/ 100 s))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 171 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))
