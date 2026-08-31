(case "ef2 the RESERVOIR-pick idiom: state keeps the LAST value satisfying a perform-checked predicate"
  (input  (do
            (effect St (op check (-> Int64 Int64)) (op best (-> Unit Int64)))
            (def (walk (: xs (List Int64)))
              (match xs
                ((list) 0)
                ((list h .. t) (+ (* 0 (St.check h)) (walk t)))))
            (def (main (: n Int64))
              (handle St -1
                ((check (v) s (resume 0 (if (= (% v 3) 0) v s)))
                 (best (u) s (resume s s)))
                (+ (* 0 (walk (list 4 6 7 9 11))) (St.best))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 9 Int64)))
