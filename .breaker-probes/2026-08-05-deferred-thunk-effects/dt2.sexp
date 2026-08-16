(case "dt2 a performing thunk passed OUT of the handle and forced outside declines or reroutes correctly"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def th (handle St n
                          ((a (u) s (resume s (+ s 1))))
                          (fn ((: u Int64)) (St.a))))
                (th 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
