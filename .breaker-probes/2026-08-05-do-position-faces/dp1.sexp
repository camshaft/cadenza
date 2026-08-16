(case "dp1 a handle whose BODY is one bare perform (minimal body — no do, no operator wrapper)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume (* s 7) s)))
                (St.a)))
            (export main)))
  (call   main (: 6 Int64)) (output (: 42 Int64)))
