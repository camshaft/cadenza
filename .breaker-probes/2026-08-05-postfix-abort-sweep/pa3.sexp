(case "pa3 the abort arm reads heap state through a MATCH on the state itself (destructure in the abort arm)"
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (list a (+ a 1))
                ((halt (u) s (match s
                               ((list) -1)
                               ((list h .. _t) (* 100 h)))))
                (St.halt)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 700 Int64)))
