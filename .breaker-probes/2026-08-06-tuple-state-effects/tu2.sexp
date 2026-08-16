(case "tu2 a tuple-of-heap state ((List, Map) pair) rebuilt per dispatch"
  (input  (do
            (effect St (op rec (-> Int64 Int64)) (op stats (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (tuple (list) (Map.insert Map.empty 0 0))
                ((rec (v) s
                  (match s
                    ((tuple xs m)
                      (resume (List.len xs) (tuple (List.push xs v) (Map.insert m v (* v 2)))))))
                 (stats (u) s
                  (match s
                    ((tuple xs m)
                      (resume (+ (List.len xs) (match (Map.lookup m 7) ((Some x) x) ((None _u) 0))) s)))))
                (+ (St.rec 7) (+ (St.rec n) (* 100 (St.stats))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1601 Int64)))
