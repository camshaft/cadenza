(case "lf2 a PERFORMING recursive walk — each element visit draws, pairing element order with state order"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (visit (: xs (List Int64)) (: i Int64))
              (match (List.at xs i)
                ((Some v) (+ (* v (St.next)) (visit xs (+ i 1))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (visit (list 3 5 7) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 94 Int64))
  (call   main (: 0 Int64)) (output (: 19 Int64)))
