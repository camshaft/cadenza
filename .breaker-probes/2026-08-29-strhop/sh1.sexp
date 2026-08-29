(do (def (sl (: s String) (: a Int64) (: b Int64)) (match (String.slice s a b) ((Some t) (String.byte-len t)) ((None _u) -1)))
    (def (main (: n Int64)) (do (def s (String.concat "hello" (if (> n 0) "world" "!"))) (+ (* 10 (sl s 2 6)) (sl s 2 (+ 20 n)))))
    (export main))
