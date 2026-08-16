(case "li1 a recursive loop whose CONDITION performs — each iteration RE-dispatches (never hoisted)"
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (go (: i Int64) (: acc Int64))
              (if (< i (St.quota)) (go (+ i 1) (+ acc i)) acc))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (- s 1))))
                (go 0 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))
