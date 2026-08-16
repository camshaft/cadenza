(case "rr1 one draw consumed THREE times (squared, scaled, summed) — a single dispatch, the binder multiply-read"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((d (St.next)))
                  (+ (* d d) (+ (* 10 d) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 81 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -23 Int64)))
