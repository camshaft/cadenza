(case "ov1 an if CONDITION from LET-bound draws — two draws feed a comparison that routes the branch"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (if (> (+ a b) 5) 100 200)
                       (* 10 (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 120 Int64))
  (call   main (: 0 Int64)) (output (: 220 Int64))
  (call   main (: 2 Int64)) (output (: 220 Int64)))
