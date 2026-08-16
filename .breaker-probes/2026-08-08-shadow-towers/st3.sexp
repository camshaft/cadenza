(case "st3 the SHADOWING frame's arm draws the SAME effect — its dispatch escapes its own extent and lands on the frame it shadows"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle E 999
                  ((next () m (resume (E.next) m)))
                  (+ (* 10 (E.next)) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))
