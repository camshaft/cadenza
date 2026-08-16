(case "cs1 three-way compare over runtime SETS orders by canonical content"
  (input  (do
            (def (main (: k Int64))
              (+ (* 100 (match (compare (Set.of (list 1 k)) (Set.of (list 1 3)))
                          ((Ordering.Less _u) 1) ((Ordering.Equal _v) 2) ((Ordering.Greater _w) 3)))
                 (match (compare (Set.of (list k 1)) (Set.of (list 1 2)))
                   ((Ordering.Less _x) 1) ((Ordering.Equal _y) 2) ((Ordering.Greater _z) 3))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 102 Int64)))
