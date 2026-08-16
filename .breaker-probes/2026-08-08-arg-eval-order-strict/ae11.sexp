(case "ae11 a DO-block as an argument — its DISCARDED interior draw still advances the state before the block's value draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (E.next) (do (E.next) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
