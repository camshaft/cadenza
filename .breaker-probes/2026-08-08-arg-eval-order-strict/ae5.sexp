(case "ae5 short-circuit AND with DRAWS on both sides — the skipped right draw leaves the state thread untouched"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((c (if (and (> (E.next) 0) (> (E.next) 0)) 100 200)))
                  (+ c (* 10 (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 170 Int64))
  (call   main (: -3 Int64)) (output (: 180 Int64)))
