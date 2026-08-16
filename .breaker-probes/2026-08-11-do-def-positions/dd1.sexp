(case "dd1 a do-def block computes the handler SEED — the def-bound intermediate feeds the seed expression"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St (do (def base (* n 10)) (+ base 7))
                ((get () s (resume s (+ s 1))))
                (+ (St.get) (* 100 (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3837 Int64))
  (call   main (: 0 Int64)) (output (: 807 Int64)))
