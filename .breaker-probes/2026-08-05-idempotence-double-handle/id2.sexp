(case "id2 the inner handle's RESULT re-performs into the OUTER same-effect handler (post-inner escape)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 100
                ((a (u) s (resume (* s 2) s)))
                (+ (handle St n
                     ((a (u) s (resume s (+ s 1))))
                     (St.a))
                   (St.a))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 205 Int64)))
