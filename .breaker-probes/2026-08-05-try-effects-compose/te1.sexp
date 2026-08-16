(case "te1 try (early-return Option) composing WITH a perform: the Option comes from a handler op"
  (input  (do
            (effect St (op find (-> Int64 (Option Int64))))
            (def (main (: n Int64))
              (handle St n
                ((find (v) s (if (> v s) (resume (Option.Some (* v 10)) s) (resume (Option.None) s))))
                (+ (match (St.find 10) ((Option.Some x) x) ((Option.None) -1))
                   (* 1000 (match (St.find 1) ((Option.Some _x) 1) ((Option.None) 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2100 Int64)))
