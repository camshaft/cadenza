(case "ao1 a pure HELPER's arguments evaluate left-to-right when each performs"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (place (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (place (St.next) (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64)))
