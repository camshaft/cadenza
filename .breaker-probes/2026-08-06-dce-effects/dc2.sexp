(case "dc2 a PURE dead binding beside a perform is harmless (the eliminable control)"
  (input  (do
            (effect St (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((peek (u) s (resume s s)))
                (let ((_dead (* n 999)))
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
