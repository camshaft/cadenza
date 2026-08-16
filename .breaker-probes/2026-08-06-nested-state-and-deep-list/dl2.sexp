(case "dl2 a 40-element list as op ARGUMENT — the arm folds a multi-leaf RRB payload"
  (input  (do
            (effect St (op total (-> (List Int64) Int64)))
            (def (build (: i Int64) (: k Int64) (: acc (List Int64)))
              (if (> i k) acc (build (+ i 1) k (List.push acc i))))
            (def (sum-l (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-l xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((total (xs) s (resume (sum-l xs 0 0) s)))
                (St.total (build 1 (* n 8) (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 820 Int64)))
