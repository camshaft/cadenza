(case "nw4 what does the arm SEE when an overflowing 999 crosses into a UInt8 op parameter"
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put 999)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 999 Int64)))
