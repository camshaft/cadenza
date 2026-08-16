(case "rw8 mutual-SCC THREE-way cycle with a branch perform in each member"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (wa (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (wb (- n 1)))))
            (def (wb (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (wc (- n 1)))))
            (def (wc (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (wa (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (wa 3)))
            (export main)))
  (output (: 6 Int64)))
