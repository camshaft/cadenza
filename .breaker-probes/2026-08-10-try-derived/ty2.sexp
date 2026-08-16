(case "ty2 the in-bounds twin over the derived list computes"
  (input  (do
            (def (get2 (: xs (List Int64)))
              (: (let ((a (try (List.at xs 0))))
                   (let ((b (try (List.at xs 3))))
                     (Some (+ a b))))
                 (Option Int64)))
            (def (main (: n Int64))
              (do
                (def joined (List.concat (list n 2) (list 3 4)))
                (match (get2 joined) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 5 Int64))
  (call   main (: 10 Int64)) (output (: 14 Int64)))
