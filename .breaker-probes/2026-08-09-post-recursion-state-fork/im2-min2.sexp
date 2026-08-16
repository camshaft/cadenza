(case "im2min2 walk result LET-BOUND before the probe"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (walk (: steps Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0)
                    (* 100 d)
                    (walk (+ steps 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((w (walk 0)))
                  (+ w (- (E.probe) n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 703 Int64))
  (call   main (: 6 Int64)) (output (: 702 Int64)))
