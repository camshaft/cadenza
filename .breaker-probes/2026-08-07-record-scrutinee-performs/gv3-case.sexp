(case "gv3 the std-sum (Option) literal scrutinee — single eval, payload binds by position"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (match (Some (St.next))
                            ((Some x) (* x 10))
                            ((None _u) -1)))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5006 Int64)))
