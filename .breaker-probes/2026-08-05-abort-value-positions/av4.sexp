(case "av4 an abort value flowing into a MATCH SCRUTINEE position outside the handle"
  (input  (do
            (effect St (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (match (handle St n ((bail (u) s (* s 2))) (+ 999 (St.bail)))
                (10 100)
                (12 200)
                (_ -1)))
            (export main)))
  (call   main (: 6 Int64)) (output (: 200 Int64)))
