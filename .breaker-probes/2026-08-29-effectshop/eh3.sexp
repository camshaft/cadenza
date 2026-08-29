(do (effect E (op get (-> Unit Int64)))
    (def (loop (: k Int64)) (if (<= k 0) 0 (+ (E.get) (loop (- k 1)))))
    (def (main (: n Int64)) (handle E 10 ((get (_u) s (resume s (+ s 1)))) (loop n)))
    (export main))
