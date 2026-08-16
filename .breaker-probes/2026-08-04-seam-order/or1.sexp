(case "or1 Set.to-list orders numerically across the fixnum/boxed representation seam"
  (input  (do
            (def (main (: z Int64))
              (match (Set.to-list (Set.of (list (+ z 536870920) (+ z 100) (- 0 (+ z 536870915)))))
                ((list a b c) (+ (* 100 (if (< a b) 1 0))
                                 (+ (* 10 (if (< b c) 1 0))
                                    (if (= a (- 0 (+ z 536870915))) 1 0))))
                (_other -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 111 Int64)))
