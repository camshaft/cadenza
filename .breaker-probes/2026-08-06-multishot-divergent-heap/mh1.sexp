(case "mh1 divergent multi-shot states carry HEAP lineages (each branch grows its own list)"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb (list)
                ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20)))))
                (+ (* 10 (Amb.flip)) (Amb.flip))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 66 Int64)))
