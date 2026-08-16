(case "rv2 the inner arm's resume value SUBTRACTS two outer draws — cross-handler order inside the arm, with a doubling outer state"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (* 2 s))))
                (handle I 0
                  ((ask () t (resume (- (O.next) (O.next)) t)))
                  (+ (I.ask) (* 10 (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 195 Int64))
  (call   main (: 1 Int64)) (output (: 39 Int64))
  (call   main (: -3 Int64)) (output (: -117 Int64)))
