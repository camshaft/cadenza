(case "hc4 Map remove inside a collision node keeps the sibling's own value retrievable"
  (input  (do
            (def (main (: z Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty (+ z 0) 10) (+ z 162287980) 20) (+ z 530337572) 30)))
                (let ((m2 (Map.remove m 162287981)))
                  (+ (* 1000 (Map.len m2))
                     (+ (* 100 (match (Map.lookup m2 1) ((Some v) (/ v 10)) ((None _u) 0)))
                        (+ (* 10 (match (Map.lookup m2 530337573) ((Some v) (/ v 10)) ((None _u) 0)))
                           (match (Map.lookup m2 162287981) ((Some _v) 9) ((None _u) 0))))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2130 Int64)))
