(case "c1 a heap-param @requires guard reads the list without consuming it — caller re-reads after the call"
  (input  (do
            (@ (requires (>= (List.len xs) 1)) (def (head-or (: xs (List Int64)))
              (match (List.at xs 0) ((Some v) v) ((None _u) -1))))
            (def (main (: k Int64))
              (let ((xs (list k (+ k 1))))
                (+ (head-or xs)
                   (* 10 (List.len xs)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 27 Int64)))
