(case "gd2 nested-pattern match over a perform-built compound ((Some (list h .. _)) through effects)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (match (Option.Some (list (St.a) (St.a)))
                  ((Option.Some (list h .. t)) (+ (* 10 h) (List.len t)))
                  (_ -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))
