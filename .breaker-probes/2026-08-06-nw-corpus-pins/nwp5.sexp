(case "a RUNTIME Int64 argument to a narrow effect-op parameter is rejected as a type mismatch"
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume 7 s)))
                (Send.put n)))
            (export main)))
  (error  CDZ0301))
