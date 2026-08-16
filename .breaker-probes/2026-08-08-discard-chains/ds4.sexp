(case "ds4 a DISCARDED if whose arms draw DIFFERENT counts — the taken branch's advances survive the discard"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (if (> (E.next) 0) (do (E.next) (E.next)) (E.next))
                    (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64))
  (call   main (: -3 Int64)) (output (: -1 Int64)))
