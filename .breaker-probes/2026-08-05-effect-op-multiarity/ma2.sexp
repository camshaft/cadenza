(case "ma2 a HEAP-arg multi-arity op: (List, Int64) args, arm indexes the list BY the scalar"
  (input  (do
            (effect St (op pick (-> (List Int64) Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((pick (xs i) s
                  (resume (match (List.at xs i) ((Option.Some v) v) ((Option.None) -1)) s)))
                (+ (* 10 (St.pick (list 10 20 30) n))
                   (St.pick (list 7) 5))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 299 Int64)))
