(case "pa1 a SAME-effect perform as another op's ARGUMENT — the arg dispatch advances the state the outer dispatch reads"
  (input  (do
            (effect St (op next (-> Int64)) (op scale (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1)))
                 (scale (v) s (resume (* v s) s)))
                (+ (St.scale (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 36 Int64))
  (call   main (: 3 Int64)) (output (: 16 Int64)))
