(case "hb1 a list-to-list TRANSFORMER op — heap payloads cross BOTH slots of one dispatch"
  (input  (do
            (effect St (op grow (-> (List Int64) (List Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grow (xs) s (resume (List.push (List.push xs (* (List.len xs) 10)) n) s)))
                (let ((out (St.grow (list 7 8))))
                  (+ (* 1000 (List.len out))
                     (+ (* 100 (match (List.at out 2) ((Some a) a) ((None _u) -1)))
                        (match (List.at out 3) ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6005 Int64)))
