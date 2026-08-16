(case "ps2 a 50k-iteration loop with a HEAP handler state grown then shrunk — the rope/CHAMP state survives scale without leaking"
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
            (def (grow (: n Int64))
              (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
            (def (main (: n Int64))
              (handle Acc (list)
                ((push (v) s (resume (List.len s) (List.push s v)))
                 (size () s (resume (List.len s) s)))
                (match (grow n) (_ (Acc.size)))))
            (export main)))
  (call   main (: 50000 Int64)) (output (: 50000 Int64))
  (call   main (: 4 Int64)) (output (: 4 Int64)))
