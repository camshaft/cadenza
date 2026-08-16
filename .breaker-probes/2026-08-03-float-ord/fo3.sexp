(case "fo3 float-sum keys ORDER through Set.to-list: discriminant-first then float payload"
  (input  (do
            (type Reading (Temp Float64) (Missing))
            (def (rank (: r Reading))
              (match r ((Temp f) (if (< f 2.0) 1 2)) ((Missing) 9)))
            (def (main (: x Float64))
              (let ((sorted (Set.to-list (Set.of (list (Missing) (Temp x) (Temp 1.5))))))
                (match sorted
                  ((list a b c) (+ (rank a) (+ (* 10 (rank b)) (* 100 (rank c)))))
                  (_ -1))))
            (export main)))
  (call   main (: 2.5 Float64)) (output (: 921 Int64)))
