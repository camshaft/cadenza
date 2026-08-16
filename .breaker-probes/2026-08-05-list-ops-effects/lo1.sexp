(case "lo1 List.update at an index COMPUTED by a perform (effectful index into persistent update)"
  (input  (do
            (effect St (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((pick (u) s (resume s (+ s 1))))
                (do
                  (def xs (list 10 20 30 40))
                  (def ys (List.update xs (St.pick) 99))
                  (+ (match (List.at ys (St.pick)) ((Option.Some v) v) ((Option.None) -1))
                     (match (List.at ys 1) ((Option.Some v) v) ((Option.None) -1))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 129 Int64)))
