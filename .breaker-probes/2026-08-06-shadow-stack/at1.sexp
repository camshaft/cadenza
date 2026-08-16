(case "at1 SAME-effect re-perform through TWO stacked handlers of one effect reaches the OUTER"
  (input  (do
            (effect E (op e (-> Unit Int64)))
            (def (main (: k Int64))
              (handle E 100 ((e (u) s (resume s s)))
                (handle E 7 ((e (u) s (resume (* 10 (E.e)) s)))
                  (E.e))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1000 Int64)))
