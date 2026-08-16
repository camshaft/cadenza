(case "ce1 two IDENTICAL performs are distinct dispatches — never CSE'd into one"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
