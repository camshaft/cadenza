(case "pyb1 a BOOL-NEGATING POST-RESUME arm under a SHORT-CIRCUIT body — each frame NEGATES whatever the resumed rest-of-body returns and the or-body's short circuit decides HOW MANY negating frames stack, one seed answers true immediately stacking one negation while the other draws twice stacking two, and the double negation restores what the single negation flips"
  (input  (do
            (effect E (op probe (-> Bool)))
            (def (main (: n Int64))
              (if (handle E (% n 3)
                    ((probe () s
                      (not (resume (> s 0) (+ s 2)))))
                    (or (E.probe) (E.probe)))
                  (: 1 Int64) (: 2 Int64)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
