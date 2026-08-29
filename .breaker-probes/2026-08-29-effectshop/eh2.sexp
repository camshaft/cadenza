(do (effect St (op next (-> Unit Int64)))
    (def (sum3) (+ (St.next) (+ (St.next) (St.next))))
    (def (main (: n Int64)) (handle St n ((next (_u) s (resume s (+ s 1)))) (sum3)))
    (export main))
