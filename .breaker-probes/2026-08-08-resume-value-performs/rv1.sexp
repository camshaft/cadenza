(case "rv1 the inner handler ARM resumes with an OUTER draw — the resume VALUE expression performs against the enclosing handler"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((ask () t (resume (O.next) t)))
                  (+ (* 10 (I.ask)) (I.ask)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
