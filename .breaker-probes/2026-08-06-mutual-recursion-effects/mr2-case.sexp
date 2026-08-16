(case "mr2c a single MUTUAL-recursion chain performs at its base — the cross-function fold serves it"
  (input  (do
            (effect St (op count (-> Unit Int64)))
            (def (ev (: k Int64))
              (if (= k 0) (St.count) (od (- k 1))))
            (def (od (: k Int64))
              (if (= k 0) (+ 100 (St.count)) (ev (- k 1))))
            (def (main (: n Int64))
              (handle St 0
                ((count (u) s (resume s (+ s 1))))
                (ev 4)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64)))
