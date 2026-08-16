(case "ae7 draw NESTED under a pure unary call in the first arg slot, second slot a bare draw — nesting must not reorder"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (dbl (: x Int64)) (* 2 x))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (dbl (E.next)) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
