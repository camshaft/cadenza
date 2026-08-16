(case "ce2 two crossed LISTS compare structurally in the arm — same content from different builders"
  (input  (do
            (effect St (op pair (-> (Tuple (List Int64) (List Int64)) Int64)))
            (def (build (: i Int64) (: k Int64) (: acc (List Int64)))
              (if (> i k) acc (build (+ i 1) k (List.push acc i))))
            (def (main (: n Int64))
              (handle St 0
                ((pair (p) s
                  (match p
                    ((tuple xs ys) (resume (if (= xs ys) 1 0) s)))))
                (+ (* 10 (St.pair (tuple (list 1 2 3) (build 1 3 (list)))))
                   (St.pair (tuple (list 1 2) (list 1 2 9))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
