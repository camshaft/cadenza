(case "dd5 a do-def BINDS a whole cross-effect handle region — the region's seed draws from the outer thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Q (op get (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (def r (handle Q (E.next)
                             ((get () t (resume t t)))
                             (Q.get)))
                    (+ (* 100 r) (E.next)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 304 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -403 Int64)))
