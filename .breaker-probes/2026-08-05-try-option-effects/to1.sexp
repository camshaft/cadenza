(case "to1 try over an Option-returning PERFORM: early-return propagates the None from the arm"
  (input  (do
            (effect St (op find (-> Int64 (Option Int64))))
            (def (grab (: k Int64))
              (do
                (def v (try (St.find k)))
                (Option.Some (* v 10))))
            (def (main (: n Int64))
              (handle St n
                ((find (k) s (resume (if (> k s) (Option.Some k) (Option.None)) s)))
                (+ (* 100 (match (grab 10) ((Option.Some v) v) ((Option.None) -1)))
                   (match (grab 1) ((Option.Some v) v) ((Option.None) -2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9998 Int64)))
