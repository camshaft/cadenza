(case "wa2 a NARROW (UInt 8) handler STATE advanced per perform (width-typed state slot)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main)
              (handle St (: 250 UInt8)
                ((a (u) s (resume (Int64.of s) s)))
                (St.a)))
            (export main)))
  (output (: 250 Int64)))
