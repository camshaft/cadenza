(case "dr1 200 recursive dispatches each crossing a heap LIST argument — the marshal at depth"
  (input  (do
            (effect St (op scan (-> (List Int64) Int64)))
            (def (loop (: i Int64) (: acc Int64))
              (if (> i 200) acc
                (loop (+ i 1) (+ acc (St.scan (list i 1))))))
            (def (main (: n Int64))
              (handle St 0
                ((scan (xs) s
                  (resume (+ (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                             (match (List.at xs 1) ((Some b) b) ((None _u) 0)))
                          s)))
                (loop 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20300 Int64)))
