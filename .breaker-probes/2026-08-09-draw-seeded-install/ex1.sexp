(case "ex1 a draw SEEDS a mid-body handler install for a different effect — the install expression itself performs"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (effect Q (op get (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (handle Q (* 100 (E.next))
                     ((get () t (resume t t)))
                     (Q.get))
                   (* 10 (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 310 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -190 Int64)))
