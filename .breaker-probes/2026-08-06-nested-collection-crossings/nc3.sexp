(case "nc3 a MAP OF LISTS op result — the body looks up a key and folds the inner list"
  (input  (do
            (effect St (op index (-> Unit (Map String (List Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((index (u) s (resume (Map.insert (Map.insert Map.empty "a" (list 1 2 n)) "b" (list 40)) s)))
                (let ((m (St.index)))
                  (+ (match (Map.lookup m "a")
                       ((Some xs) (+ (List.len xs) (match (List.at xs 2) ((Some v) v) ((None _u) -1))))
                       ((None _u) -100))
                     (match (Map.lookup m "b")
                       ((Some ys) (match (List.at ys 0) ((Some w) w) ((None _u) -1)))
                       ((None _u) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 48 Int64)))
