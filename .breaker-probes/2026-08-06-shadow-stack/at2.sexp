(case "at2 THREE stacked same-effect handlers: each arm re-performs outward one level"
  (input  (do
            (effect E (op e (-> Unit Int64)))
            (def (main (: k Int64))
              (handle E 5 ((e (u) s (resume s s)))
                (handle E 0 ((e (u) s (resume (* 10 (E.e)) s)))
                  (handle E 0 ((e (u) s (resume (* 100 (E.e)) s)))
                    (+ (E.e) k)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 5003 Int64)))
