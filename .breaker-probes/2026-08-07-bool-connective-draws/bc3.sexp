(case "bc3 a nested and-of-or short-circuit tree over draws — each row exercises a distinct skip pattern"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (if (and (or (> (St.next) 8) (> (St.next) 2)) (> (St.next) 5))
                    (St.next)
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 11 Int64))
  (call   main (: 4 Int64)) (output (: 7 Int64))
  (call   main (: 1 Int64)) (output (: -3 Int64))
  (call   main (: 0 Int64)) (output (: -2 Int64)))
