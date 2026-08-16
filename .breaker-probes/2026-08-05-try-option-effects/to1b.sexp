(case "to1b control: try over a CONSTANT Option inside the same handle shape"
  (input  (do
            (effect St (op find (-> Int64 (Option Int64))))
            (def (grab (: k Int64))
              (do
                (def v (try (Option.Some k)))
                (Option.Some (* v 10))))
            (def (main (: n Int64))
              (handle St n
                ((find (k) s (resume (Option.Some k) s)))
                (match (grab 10) ((Option.Some v) v) ((Option.None) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64)))
