(case "im1 one handler with BOTH abort and resume arms used in an alternating sequence across two handles"
  (input  (do
            (effect St (op go (-> Unit Int64)) (op stop (-> Unit Int64)))
            (def (round (: seed Int64))
              (handle St seed
                ((go (u) s (resume s (+ s 1)))
                 (stop (u) s (* 100 s)))
                (+ (* 0 (St.go)) (+ (* 0 (St.go)) (St.stop)))))
            (def (main (: n Int64))
              (+ (round n) (round (* n 10))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3700 Int64)))
