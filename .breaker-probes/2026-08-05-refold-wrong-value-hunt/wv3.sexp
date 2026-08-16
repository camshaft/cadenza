(case "wv3 two-site arm whose RESUME VALUES are heap (Lists) not scalars"
  (input  (do
            (effect St (op grab (-> Int64 (List Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grab (v) s (if (> v 1) (resume (list v v) (+ s 1)) (resume (list) s))))
                (+ (List.len (St.grab n)) (+ (* 10 (List.len (St.grab 1))) (* 100 (List.len (St.grab 4)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 202 Int64)))
