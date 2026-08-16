(case "sk7 BYTES state read by an abort arm"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (Bytes.of (list 1 2 3))
                ((halt (u) s (* 100 (+ (Bytes.len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 500 Int64)))
