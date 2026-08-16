(case "st1 the SAME effect handled at THREE depths — each draw resolves to the innermost open frame, outer frames resume as inner ones close"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle E 50
                     ((next () s (resume s (+ s 5))))
                     (+ (handle E 700
                          ((next () s (resume s (+ s 7))))
                          (E.next))
                        (* 10 (E.next))))
                   (* 100 (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1700 Int64))
  (call   main (: 0 Int64)) (output (: 1200 Int64))
  (call   main (: -3 Int64)) (output (: 900 Int64)))
