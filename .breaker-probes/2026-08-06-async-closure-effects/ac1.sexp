(case "ac1 a pure closure-driver call beside a perform in one handle body"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (apply-twice (fn ((: x Int64)) (* x 2)) n) (St.bump))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 122 Int64)))
