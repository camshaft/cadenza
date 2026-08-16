(case "be2 multi-limb BigInt as abort VALUE: the abort return carries a >i64 magnitude exactly"
  (input  (do
            (effect St (op halt (-> Unit BigInt)))
            (def (main)
              (if (= (handle St 9223372036854775807N
                       ((halt (u) s (+ s s)))
                       (St.halt))
                     18446744073709551614N)
                1 0))
            (export main)))
  (output (: 1 Int64)))
