(case "ce2 identical PURE subterms around distinct performs — pure sharing must not merge dispatches"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* (+ n 1) (St.next)) (* (+ n 1) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 66 Int64)))
