(case "pyj1 a PURE HELPER CALL AS THE TOLL — each frame's post-resume toll is a squaring def applied to the captured state so the toll routes through a real function call during the unwind, the two frames square different offsets, and the call rung joins the toll side of the ladder as pyh1 joined its consumer side"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (sq (: x Int64)) (* x x))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (sq (+ s 2)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 46 Int64))
  (call   main (: 0 Int64)) (output (: 23 Int64)))
