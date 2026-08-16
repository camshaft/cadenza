(case "sk9b control: BigInt seed, abort arm answers CONSTANT"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St 5N
                ((halt (u) s (* 100 a)))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 200 Int64)))
