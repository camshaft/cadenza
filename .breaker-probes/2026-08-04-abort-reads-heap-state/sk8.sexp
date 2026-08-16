(case "sk8 TUPLE-of-heap state read by an abort arm ((list) in a tuple seed)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (tuple 5 (list 9))
                ((halt (u) s (* 100 (+ (. s 0) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 700 Int64)))
