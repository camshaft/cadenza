(case "sk7b control: same Bytes seed, abort arm answers CONSTANT"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (Bytes.of (list 1 2 3))
                ((halt (u) s (* 100 a)))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 200 Int64)))
