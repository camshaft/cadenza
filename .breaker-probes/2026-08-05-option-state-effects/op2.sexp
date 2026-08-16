(case "op2 an Option-of-HEAP state (Option (List Int64)) transitioning None->Some(list)->grown"
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (Option.None)
                ((feed (v) s
                  (match s
                    ((Option.None) (resume 0 (Option.Some (list v))))
                    ((Option.Some xs) (resume (List.len xs) (Option.Some (List.push xs v)))))))
                (+ (* 100 (St.feed a)) (+ (* 10 (St.feed (+ a 1))) (St.feed (+ a 2))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 12 Int64)))
