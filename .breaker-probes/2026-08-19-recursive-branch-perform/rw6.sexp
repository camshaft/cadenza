(case "rw6 control: perform as the WHOLE tail of a branch with the self-call in a SIBLING branch"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64)) (if (= n 0) (St.get) (walk (- n 1))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 1 Int64)))
