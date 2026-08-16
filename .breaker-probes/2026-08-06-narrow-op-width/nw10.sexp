(case "nw10 the SIGNED face: -999 crosses an (-> Int8 Int64) op and the arm observes it"
  (input  (do
            (effect Send (op put (-> Int8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put -999)))
            (export main)))
  (call   main (: 0 Int64)) (output (: -999 Int64)))
