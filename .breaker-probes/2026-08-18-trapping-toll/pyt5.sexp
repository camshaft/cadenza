(case "pyt5 the BODY TRAPS UNDER A PENDING TOLL — the replayed rest-of-body divides by the drawn answer so the zero seed traps INSIDE the resumed continuation while the frame's thousandfold toll is still pending, the trap wins over the pending post-resume work, and the nonzero seed completes the body and pays the toll"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (let ((a (E.tick)))
                  (let ((b (/ 60 a)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2601 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))
