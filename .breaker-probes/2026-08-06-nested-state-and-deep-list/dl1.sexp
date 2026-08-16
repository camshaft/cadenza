(case "dl1 a 40-element list op RESULT crosses resume — a multi-leaf RRB payload survives the marshal"
  (input  (do
            (effect St (op range (-> Int64 (List Int64))))
            (def (build (: i Int64) (: k Int64) (: acc (List Int64)))
              (if (> i k) acc (build (+ i 1) k (List.push acc i))))
            (def (main (: n Int64))
              (handle St 0
                ((range (k) s (resume (build 1 k (list)) s)))
                (let ((xs (St.range (* n 8))))
                  (+ (* 100 (List.len xs))
                     (match (List.at xs 35) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4036 Int64)))
