(case "pyr6 the RESUME RESULT LET-BOUND into a PURE COMBINE — no branch at all, each arm binds the resumed rest-of-body value and answers twice-it-plus-state, the minimal let-bound-resume shape isolating the binder itself from any downstream control flow, doubling per frame so the unwind order is pinned by magnitude"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((r (resume s (+ s 1))))
                    (+ (* 2 r) s))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 89 Int64))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
