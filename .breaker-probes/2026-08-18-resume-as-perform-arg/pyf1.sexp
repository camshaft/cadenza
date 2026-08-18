(case "pyf1 the RESUME VALUE FED TO A FOREIGN PERFORM — each inner arm hands whatever the resumed rest-of-body returned to the outer scaler which doubles it plus its own advancing state, the two scalings compose innermost-first during the unwind so the outer handler transforms the pyramid twice with different offsets, and the answer is the twice-scaled fold"
  (input  (do
            (effect T (op scale (-> Int64 Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((scale (v) t (resume (+ (* v 2) t) (+ t 1))))
                (handle E (: 1 Int64)
                  ((tick () s
                    (T.scale (resume s (+ s 1)))))
                  (let ((a (E.tick)))
                    (let ((b (E.tick)))
                      (+ a (* 10 b)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 88 Int64))
  (call   main (: 0 Int64)) (output (: 85 Int64)))
