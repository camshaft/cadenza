(case "ha2 the same in-program handle OUTSIDE any host block on rust-async (control)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1))))
                (+ (St.bump) (St.bump))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
