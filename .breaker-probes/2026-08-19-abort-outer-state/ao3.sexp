(case "ao3 escalation: HEAP outer state — the inner abort loses the committed push"
  (input  (do
            (effect A (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main)
              (handle A (list)
                ((add (v) s (resume (List.len s) (List.push s v)))
                 (count (u) s (resume (List.len s) s)))
                (let ((b (handle B 0 ((bail (u) s 99)) (do (A.add 5) (B.bail)))))
                  (+ b (A.count)))))
            (export main)))
  (output (: 100 Int64)))
