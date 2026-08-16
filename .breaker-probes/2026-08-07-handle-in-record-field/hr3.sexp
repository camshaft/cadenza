(case "hr3 a HANDLE expression as a map-literal VALUE — the region's result is stored under a key and looked up after"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((m (map (1 (handle St n
                                 ((next () s (resume s (+ s 3))))
                                 (+ (St.next) (St.next))))
                            (2 50))))
                (+ (match (Map.lookup m 1) ((Some v) v) ((None) -1))
                   (* 100 (match (Map.lookup m 2) ((Some v) v) ((None) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5013 Int64))
  (call   main (: 0 Int64)) (output (: 5003 Int64)))
