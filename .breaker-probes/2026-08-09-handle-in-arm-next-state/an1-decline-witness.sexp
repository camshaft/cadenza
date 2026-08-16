(case "an1 a WHOLE nested handle computes the arm's NEXT-STATE expression"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s
                  (resume s
                    (handle B s
                      ((g (u) t (resume t (+ t 1))))
                      (+ (B.g) (B.g))))))
                (+ (E.next) (* 10 (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -4 Int64)) (output (: -74 Int64)))
