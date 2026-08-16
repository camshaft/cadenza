(case "ae9 RECORD literal of two draws read back by PROJECTION — field-init order in the literal drives the state thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((r (record (a (E.next)) (b (E.next)))))
                  (+ (* 10 (. r a)) (. r b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))
