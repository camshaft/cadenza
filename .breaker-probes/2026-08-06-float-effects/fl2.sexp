(case "fl2 a Float64 op RESULT + float state arithmetic observed via a comparison"
  (input  (do
            (effect St (op frac (-> Unit Float64)))
            (def (main (: a Int64))
              (handle St 0.5
                ((frac (u) s (resume s (* s 0.5))))
                (if (> (+ (St.frac) (St.frac)) 0.7) 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
