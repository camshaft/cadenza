(case "lr1 list-REST pattern over a perform-built list ((list h .. t) destructure of effectful construction)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (match (list (St.a) (St.a) (St.a))
                  ((list h .. t) (+ (* 100 h) (+ (* 10 (List.len t))
                    (match (List.at t 1) ((Option.Some v) v) ((Option.None) -1)))))
                  (_ -2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 527 Int64)))
