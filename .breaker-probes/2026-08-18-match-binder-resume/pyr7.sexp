(case "pyr7 a MATCH BINDER on the resume result — the arm matches the resumed rest-of-body value against a zero literal falling through to a BINDER arm that reuses the value doubled-plus-state, the binder arm is the match twin of the let form the two-hole refold folds, and the runs disagree at both frames through the binder path"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (resume s (+ s 3))
                    (0 (+ 100 s))
                    (r (+ (* r 2) s)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 173 Int64))
  (call   main (: 0 Int64)) (output (: 126 Int64)))
