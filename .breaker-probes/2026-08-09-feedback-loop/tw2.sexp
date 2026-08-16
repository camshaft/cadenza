(case "tw2 an explicit FEEDBACK loop — each call feeds the previous result back as the op's first argument beside a fresh draw"
  (input  (do
            (effect E (op mix (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix (r) s (resume (* 2 (+ r s)) (+ s 2)))
                 (probe () s (resume s s)))
                (+ (* 10 (E.mix (E.mix (E.mix 1))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 666 Int64))
  (call   main (: 0 Int64)) (output (: 246 Int64))
  (call   main (: -5 Int64)) (output (: -454 Int64)))
