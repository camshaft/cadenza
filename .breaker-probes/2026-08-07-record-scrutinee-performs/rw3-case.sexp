(case "rw3 dispatch-count witness: post-match read shows how many draws fired"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (match (record (a (St.next)) (b (St.next)))
                            ((record (a x) (b y)) (+ (* 10 x) y))))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1602 Int64)))
