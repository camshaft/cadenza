(case "ra7 a NON-recursive helper performing TWO draws, called once, trailing draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (two)
              (let ((a (E.next)))
                (let ((b (E.next)))
                  (+ a b))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (+ (two) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64))
  (call   main (: 1 Int64)) (output (: 18 Int64)))
