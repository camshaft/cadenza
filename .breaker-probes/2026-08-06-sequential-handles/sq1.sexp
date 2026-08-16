(case "sq1 TWO sequential handles of the same effect — the second starts fresh, no state bleed"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (run (: seed Int64))
              (handle St seed
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (+ (* 100 (run n)) (run 10)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1121 Int64)))
