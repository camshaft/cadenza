(case "pyr10 the LET-BOUND REPLAY VALUE ROUTES AND FEEDS BOTH BRANCHES — each arm binds the rest-of-body value then an if picks between adding the state or doubling the value plus tenfold state, the seeds split the INNER frame's branch while the outer frame holds steady, and both the branch decision and the surviving arithmetic reuse the same binder across the suspend"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((r (resume s (+ s 1))))
                    (if (> r 15)
                        (+ r s)
                        (+ (* 2 r) (* 10 s))))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 24 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64)))
