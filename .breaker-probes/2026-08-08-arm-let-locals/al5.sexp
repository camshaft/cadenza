(case "al5 an arm-LOCAL named the same as a body-side binder — hygiene keeps the two w's separate across dispatches"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (let ((w (* v 10)))
                            (resume (+ w s) (+ s 1)))))
                (let ((w 7000))
                  (+ (E.f 2) (+ w (E.f 3))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7061 Int64))
  (call   main (: 0 Int64)) (output (: 7051 Int64)))
