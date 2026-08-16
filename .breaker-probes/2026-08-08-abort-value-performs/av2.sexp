(case "av2 the abort value SUBTRACTS two outer draws under a DOUBLING outer state — antisymmetry pins order inside the aborting arm"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (* 2 s))))
                (+ (* 100 (handle Bail 0
                            ((out () t (- (O.next) (O.next))))
                            (+ (Bail.out) 999)))
                   (O.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -480 Int64))
  (call   main (: 1 Int64)) (output (: -96 Int64))
  (call   main (: -3 Int64)) (output (: 288 Int64)))
