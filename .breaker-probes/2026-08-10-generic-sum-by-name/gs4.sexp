(case "gs4 the applied generic wraps a HEAP payload — (Container (List Int64)) in the annotation, the list summed after unwrap"
  (input  (do
            (type (Container a) (Full a))
            (def (sum-at (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-at xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (unwrap-sum (: b (Container (List Int64))))
              (match b ((Full xs) (sum-at xs 0 0))))
            (def (main (: k Int64)) (unwrap-sum (Full (list k 7 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64))
  (call   main (: -9 Int64)) (output (: -1 Int64)))
