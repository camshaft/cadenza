(case "rw2 control: BARE perform in the recursive body threads correctly"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64)) (if (= n 0) 0 (+ (St.get) (walk (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 6 Int64)))
