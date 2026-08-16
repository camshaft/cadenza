(case "sy2 Symbol round-trips through to-string/of across a MULTIBYTE rope"
  (input  (do
            (def (main (: k Int64))
              (do
                (def s (Symbol.of (String.concat "é日" (if (> k 0) "x" "y"))))
                (+ (* 10 (if (= (Symbol.of (Symbol.to-string s)) s) 1 0))
                   (Set.len (Set.of (list s (Symbol.of "é日x")))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
