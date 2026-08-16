(case "hc6 to-list over a collision node yields the colliding keys in value order"
  (input  (do
            (def (main (: z Int64))
              (match (Set.to-list (Set.of (list (+ z 530337572) (+ z 0) (+ z 162287980))))
                ((list a b c) (+ (* 100 (if (< a b) 1 0))
                                 (+ (* 10 (if (< b c) 1 0))
                                    (if (= a 1) 1 0))))
                (_other -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 111 Int64)))
