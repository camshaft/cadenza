(case "dh2 five-deep stack where the INNERMOST abort tunnels its value out through all four outer handles"
  (input  (do
            (effect E1 (op o1 (-> Unit Int64)))
            (effect E2 (op o2 (-> Unit Int64)))
            (effect E3 (op o3 (-> Unit Int64)))
            (effect E4 (op o4 (-> Unit Int64)))
            (effect E5 (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (+ 1 (handle E1 0
                ((o1 (u) s (resume s s)))
                (+ 10 (handle E2 0
                  ((o2 (u) s (resume s s)))
                  (+ 100 (handle E3 0
                    ((o3 (u) s (resume s s)))
                    (+ 1000 (handle E4 0
                      ((o4 (u) s (resume s s)))
                      (+ 10000 (handle E5 7
                        ((bail (u) s (* 2 s)))
                        (+ 100000 (E5.bail)))))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11125 Int64)))
