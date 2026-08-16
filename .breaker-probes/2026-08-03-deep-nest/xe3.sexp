(case "xe3 a deep heterogeneous nest crosses the host boundary as a value form"
  (input  (do
            (def (main (: k Int64))
              (tuple (list (record (x k)) (record (x (+ k 1))))
                     (Set.of (list k))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: (tuple (list (record (x 5)) (record (x 6))) ((. Set of) (list 5)))
             (Tuple (List (record (x Int64))) (Set Int64)))))
