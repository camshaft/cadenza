(case "ma1 a THREE-arg effect op: all args evaluated left-to-right, arm reads all three"
  (input  (do
            (effect St (op tri (-> Int64 Int64 Int64 Int64)) (op c (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((tri (a b c) s (resume (+ (* 100 a) (+ (* 10 b) c)) s))
                 (c (u) s (resume s (+ s 1))))
                (St.tri (St.c) (St.c) (St.c))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64)))
