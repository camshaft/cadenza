(case "gd1 match arms selected by perform-built SUM values (variant routing of effectful scrutinee)"
  (input  (do
            (effect St (op pick (-> Unit Int64)))
            (def (mk (: v Int64)) (if (> v 5) (Option.Some v) (Option.None)))
            (def (main (: n Int64))
              (handle St n
                ((pick (u) s (resume s (+ s 1))))
                (+ (* 100 (match (mk (St.pick)) ((Option.Some v) v) ((Option.None) -1)))
                   (match (mk (St.pick)) ((Option.Some v) v) ((Option.None) -2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -94 Int64)))
