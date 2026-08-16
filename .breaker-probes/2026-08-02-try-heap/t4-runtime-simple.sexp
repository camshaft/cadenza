(case "t4 runtime chained try over List.at with success and cut paths"
  (input  (do
            (def (grab (: xs (List Int64)) (: i Int64))
              (let ((a (try (List.at xs i))))
                (let ((b (try (List.at xs (+ i 1)))))
                  (Some (+ (* 100 a) b)))))
            (def (main (: i Int64))
              (match (grab (list 7 8 9) i)
                ((Some v) v)
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 708 Int64))
  (call   main (: 2 Int64)) (output (: -1 Int64)))
