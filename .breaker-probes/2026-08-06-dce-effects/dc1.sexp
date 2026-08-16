(case "dc1 a perform bound to an UNUSED binding still dispatches (DCE must not eliminate it)"
  (input  (do
            (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1)))
                 (peek (u) s (resume s s)))
                (let ((_unused (St.bump)))
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
