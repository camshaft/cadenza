(case "pyr3 the RESUME RESULT LET-BOUND and BRANCHED ON — each arm binds what the resumed rest-of-body returned then an if on that value picks between tripling-plus-state and adding a hundredfold state, the outer frame's branch decision depends on what the INNER frame's toll produced, and the seed flips the inner frame between the two branches so the runs unwind along different arithmetic paths entirely"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((r (resume s (+ s 3))))
                    (if (> r 35)
                        (+ (* r 3) s)
                        (+ r (* 100 s))))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 382 Int64))
  (call   main (: 0 Int64)) (output (: 990 Int64)))
