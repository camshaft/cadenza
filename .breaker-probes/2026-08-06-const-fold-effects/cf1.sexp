(case "cf1 constant conditions simplify around performs — kept branches dispatch, dropped ones do not"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (if true (St.next) 999) (if false 999 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
