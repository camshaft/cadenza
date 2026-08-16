(case "sc1 a STACKED @requires + @ensures contract on a performing def (full Hoare triple x effects)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0))
            (@ (ensures (>= ret 100))
               (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
