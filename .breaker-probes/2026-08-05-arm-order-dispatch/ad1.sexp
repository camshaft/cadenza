(case "ad1 arm-order independence: op dispatch routes by NAME not position (arms declared in swapped order)"
  (input  (do
            (effect St (op zz (-> Unit Int64)) (op aa (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((aa (u) s (resume 100 s))
                 (zz (u) s (resume 200 s)))
                (+ (* 10 (St.zz)) (St.aa))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2100 Int64)))
