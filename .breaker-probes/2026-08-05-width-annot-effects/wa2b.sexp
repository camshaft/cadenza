(case "wa2b control: UInt8 state read WITHOUT the Int64.of widen (state used at its own width)"
  (input  (do
            (effect St (op a (-> Unit UInt8)))
            (def (main)
              (handle St (: 250 UInt8)
                ((a (u) s (resume s s)))
                (Int64.of (St.a))))
            (export main)))
  (output (: 250 Int64)))
