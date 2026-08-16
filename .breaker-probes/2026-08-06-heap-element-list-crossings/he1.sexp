(case "he1 a LIST OF STRINGS op result — the body indexes and measures rope elements after the marshal"
  (input  (do
            (effect St (op names (-> Int64 (List String))))
            (def (main (: n Int64))
              (handle St 0
                ((names (k) s (resume (list (String.concat "al" "pha") (if (> k 0) "beta" "x") "gamma") s)))
                (let ((xs (St.names n)))
                  (+ (* 100 (List.len xs))
                     (+ (* 10 (match (List.at xs 0) ((Some a) (String.byte-len a)) ((None _u) -1)))
                        (match (List.at xs 1) ((Some b) (String.byte-len b)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 354 Int64)))
