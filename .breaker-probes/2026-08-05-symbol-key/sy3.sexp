(case "sy3 Symbols order by BYTE-lexicographic content across ASCII/multibyte boundaries in to-list"
  (input  (do
            (def (main (: k Int64))
              (match (Set.to-list (Set.of (list (Symbol.of "z") (Symbol.of (String.concat "é" "")) (Symbol.of "a"))))
                ((list x y z2) (+ (* 100 (if (= x (Symbol.of "a")) 1 0))
                                  (+ (* 10 (if (= y (Symbol.of "z")) 1 0))
                                     (if (= z2 (Symbol.of "é")) 1 0))))
                (_other -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
