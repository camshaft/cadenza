(case "ng1 the arm NEGATES alternate draws — a Bool flip in a tuple state signs the rising thread (+,-,+)"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n false)
                ((next () s (match s
                              ((tuple v flip)
                                (resume (if flip (- 0 v) v)
                                        (tuple (+ v 2) (not flip)))))))
                (+ (* 100 (E.next)) (+ (* 10 (E.next)) (E.next)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 257 Int64))
  (call   main (: 0 Int64)) (output (: -16 Int64))
  (call   main (: -2 Int64)) (output (: -198 Int64)))
