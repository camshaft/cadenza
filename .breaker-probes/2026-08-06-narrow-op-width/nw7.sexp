(case "nw7 the UInt16 width: 99999 crosses a (-> UInt16 Int64) op and the arm observes it"
  (input  (do
            (effect Send (op put (-> UInt16 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put 99999)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 99999 Int64)))
