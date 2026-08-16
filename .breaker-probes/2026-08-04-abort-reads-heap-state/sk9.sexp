(case "sk9 BIGINT state read by an abort arm (boxed-scalar seed face)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St 5N
                ((halt (u) s (* 100 (+ (Int64.of s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 700 Int64)))
