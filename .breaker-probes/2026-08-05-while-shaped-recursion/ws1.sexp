(case "ws1 a CONDITION-driven loop (recursion terminating on a perform result, not a counter)"
  (input  (do
            (effect St (op draw (-> Unit Int64)))
            (def (drain (: acc Int64))
              (do
                (def v (St.draw))
                (if (= v 0) acc (drain (+ acc v)))))
            (def (main (: n Int64))
              (handle St n
                ((draw (u) s (resume s (- s 1))))
                (drain 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 10 Int64)))
