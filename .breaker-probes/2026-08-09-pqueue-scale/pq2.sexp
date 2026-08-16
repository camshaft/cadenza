(case "pq2 drain-and-refill: pop-min then re-insert at a LATER time keeps the queue coherent"
  (input  (do
        (def (insort (: q (List (Tuple Int64 Int64))) (: e (Tuple Int64 Int64)))
          (match q
            ((list) (List.prepend (list) e))
            ((list h .. t)
              (if (<= (. e 0) (. h 0))
                  (List.prepend q e)
                  (List.prepend (insort t e) h)))))
        (def (step (: q (List (Tuple Int64 Int64))) (: rounds Int64) (: acc Int64))
          (if (= rounds 0) (tuple acc (List.len q))
              (match q
                ((list) (tuple acc 0))
                ((list h .. t)
                  (step (insort t (tuple (+ (. h 0) 10) (. h 1))) (- rounds 1) (+ (* 10 acc) (. h 1)))))))
        (def (main (: k Int64))
          (do
            (def q0 (insort (insort (insort (list) (tuple 3 1)) (tuple 1 2)) (tuple 2 3)))
            (match (step q0 k 0)
              ((tuple acc len) (+ (* 10 acc) len)))))
        (export main)))
  (call   main (: 5 Int64)) (output (: 231233 Int64)))
