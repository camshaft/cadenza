(do (effect E (op get (-> Unit Int64)))
    (def (f) (+ (E.get) 1))
    (def (main (: n Int64)) (handle E n ((get (_u) s (resume s (+ s 1)))) (f)))
    (export main))
