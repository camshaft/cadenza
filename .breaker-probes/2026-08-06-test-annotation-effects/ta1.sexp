(case "ta1 a @test-tier @ensures on a performing def runs and checks (test-tier x effects)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ test (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
