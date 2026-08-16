(case "rw10 match-arm face with a MULTI-ARM scrutinee selecting the performing arm at runtime"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0) 0
                (+ (match (% n 2) (0 (St.get)) (_ (St.get))) (walk (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 6 Int64)))
