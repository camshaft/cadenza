(case "er1 the arm ENUMERATES its Map state to a list of tuples and resumes the enumeration"
  (input  (do
            (effect Db (op dump (-> Unit (List (Tuple String Int64)))))
            (def (sum-snd (: xs (List (Tuple String Int64))) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some p) (match p ((tuple k v) (sum-snd xs (+ i 1) (+ acc v)))))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle Db (Map.insert (Map.insert Map.empty "a" n) "b" 30)
                ((dump (u) m (resume (Map.to-list m) m)))
                (let ((xs (Db.dump)))
                  (+ (* 100 (List.len xs)) (sum-snd xs 0 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 235 Int64)))
