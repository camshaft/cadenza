(case "rw4 escalation: the branch perform in a MUTUALLY-recursive performer pair"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (even-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (odd-w (- n 1)))))
            (def (odd-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (even-w (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (even-w 3)))
            (export main)))
  (output (: 6 Int64)))
