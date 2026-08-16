(case "rd1 resume values COLLECTED into a List by the body across performs, folded after"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (suml (: xs (List Int64)))
              (match xs ((list) 0) ((list h .. t) (+ h (suml t)))))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (suml (list (St.next) (St.next) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 18 Int64)))
