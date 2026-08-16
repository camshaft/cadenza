(case "ne1 an OPTION built from a draw crosses dispatch as an op ARGUMENT — the arm matches it against the live state"
  (input  (do
            (effect E (op next (-> Int64)) (op score (-> (Option Int64) Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (score (o) s (resume (match o
                                        ((Some v) (+ (* 10 v) s))
                                        ((None) s))
                                      (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (* 10 (E.score (if (> d 0) (Some d) (None))))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 342 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -4 Int64)) (output (: -28 Int64)))
