(case "pyc1 the RESUME CALL INSIDE A TUPLE CONSTRUCTOR — the arm builds a pair of the resumed rest-of-body value and a state increment then destructures BOTH through a tuple pattern whose binders feed a doubled-plus-witness combine, the resume value passing through construction and pattern-binding rather than any direct binder, and the frames disagree on both tuple fields"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (tuple (resume s (+ s 3)) (+ s 1))
                    ((tuple r w) (+ (* r 2) w)))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 176 Int64))
  (call   main (: 0 Int64)) (output (: 129 Int64)))
