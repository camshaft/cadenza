(case "ae6 short-circuit OR with DRAWS on both sides — the right draw fires only when the left is false"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((c (if (or (> (E.next) 0) (> (E.next) 0)) 100 200)))
                  (+ c (* 10 (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 160 Int64))
  (call   main (: -3 Int64)) (output (: 190 Int64)))
