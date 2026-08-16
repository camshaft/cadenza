(case "tu2 the tuple state's HEAP component escapes via a resuming op and is mutated OUTSIDE; original state unaffected"
  (input  (do
            (effect St (op push (-> Int64 Int64)) (op grab (-> Unit (List Int64))) (op count (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (tuple 0 (list))
                ((push (v) s (resume 0 (tuple (+ (. s 0) 1) (List.push (. s 1) v))))
                 (grab (u) s (resume (. s 1) s))
                 (count (u) s (resume (List.len (. s 1)) s)))
                (do
                  (def _a (St.push a))
                  (def escaped (St.grab))
                  (def mutated (List.push escaped 99))
                  (+ (* 100 (List.len mutated)) (St.count)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 201 Int64)))
