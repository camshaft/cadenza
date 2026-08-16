(case "sk6b rope face: String seed built by String.concat (not a literal)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (String.concat "se" "ed")
                ((halt (u) s (* 100 (+ (String.scalar-len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 600 Int64)))
