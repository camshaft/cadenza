(case "an in-range literal to a narrow effect-op parameter crosses and the arm observes it"
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put 42)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
