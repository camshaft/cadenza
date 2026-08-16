(case "mh2 each multi-shot branch observes its own heap-lineage length"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op size (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb (list)
                ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20))))
                 (size (u) s (resume (List.len s) s)))
                (+ (* 10 (Amb.flip)) (Amb.size))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64)))
