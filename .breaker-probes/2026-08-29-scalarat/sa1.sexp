(do (def (at (: s String) (: i Int64)) (match (String.scalar-at s i) ((Some c) (Char.to-int c)) ((None _u) -1)))
    (def (main (: n Int64)) (do (def s (String.concat "ab" (if (> n 0) "cé" "z!"))) (+ (* 1000 (at s 0)) (+ (* 10 (at s 2)) (at s 9)))))
    (export main))
