(case "op1 an OPTION handler state toggling Some/None across performs (sum-typed state transitions)"
  (input  (do
            (effect St (op tog (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Option.Some n)
                ((tog (u) s
                  (match s
                    ((Option.Some v) (resume v (Option.None)))
                    ((Option.None)   (resume -1 (Option.Some 99))))))
                (+ (* 100 (St.tog)) (+ (* 10 (St.tog)) (St.tog)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 789 Int64)))
