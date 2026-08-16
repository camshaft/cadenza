(case "nw1 an OVERFLOWING literal to a narrow op parameter compiles-and-runs (was expected CDZ0302)"
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume 7 s)))
                (Send.put 999)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
