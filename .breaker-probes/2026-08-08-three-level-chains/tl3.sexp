(case "tl3 the MIDDLE frame's arm resumes with an OUTERMOST draw — rv-face at depth, dispatched from under a third live frame"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (O.next) m)))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (* 100 (M.grab)) (+ (* 10 (M.grab)) (I.pick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64))
  (call   main (: -2 Int64)) (output (: -203 Int64)))
