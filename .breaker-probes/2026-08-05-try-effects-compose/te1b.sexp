(case "te1b control: Option-returning op, ONE perform, matched"
  (input  (do
            (effect St (op find (-> Int64 (Option Int64))))
            (def (main (: n Int64))
              (handle St n
                ((find (v) s (if (> v s) (resume (Option.Some (* v 10)) s) (resume (Option.None) s))))
                (match (St.find 10) ((Option.Some x) x) ((Option.None) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64)))
