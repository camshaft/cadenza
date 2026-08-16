(case "lu1 an updated-back RRB list equals the never-updated build (update history-independence)"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (churn (: i Int64) (: xs (List Int64)))
              (if (= i 0) xs (churn (- i 1) (List.update xs 20 777))))
            (def (main (: n Int64))
              (do
                (def base (build n (list)))
                (def restored (List.update (churn 30 base) 20 (match (List.at base 20) ((Some v) v) ((None _u) -1))))
                (+ (* 10 (if (= restored base) 1 0))
                   (if (= (List.len restored) n) 1 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 11 Int64)))
