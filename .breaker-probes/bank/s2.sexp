(case "s2 abort DIRECT in body, pending add (control - should abandon)"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (Mx.bail 5) 999999)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
