(case "hr1 a handle whose RESULT is a Map built entirely in the body (handle-value = heap compound)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def m (handle St n
                         ((a (u) s (resume s (+ s 1))))
                         (Map.insert (Map.insert Map.empty (St.a) 100) (St.a) 200)))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (+ n 1)) ((Some v) (/ v 100)) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
