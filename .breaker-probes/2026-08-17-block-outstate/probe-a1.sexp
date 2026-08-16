(case "a1 adv-69 escalation: HEAP handler state (list) also reverts at the block boundary"
  (input  (do
            (effect Log (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: x Int64))
              (handle Log (list)
                ((add (v) s (resume v (List.push s v)))
                 (count (u) s (resume (List.len s) s)))
                (let ((v (let ((b true)) (if b (Log.add 5) 99))))
                  (+ (* 10 v) (Log.count)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 51 Int64)))
