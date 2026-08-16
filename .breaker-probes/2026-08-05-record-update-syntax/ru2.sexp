(case "ru2 record fields as perform args in DECLARATION order (field-value evaluation sequencing)"
  (input  (do
            (effect St (op v (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((v (u) s (resume s (+ s 1))))
                (do
                  (def r (record (a (St.v)) (b (St.v)) (c (St.v))))
                  (+ (* 100 (. r a)) (+ (* 10 (. r b)) (. r c))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64)))
