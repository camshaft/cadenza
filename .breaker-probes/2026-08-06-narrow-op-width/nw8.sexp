(case "nw8 the arm RESUMES 999 into a UInt8-typed op RESULT — does the body observe it"
  (input  (do
            (effect Give (op get (-> Unit UInt8)))
            (def (main (: n Int64))
              (handle Give 0
                ((get (u) s (resume 999 s)))
                (Int64.of (Give.get))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 999 Int64)))
