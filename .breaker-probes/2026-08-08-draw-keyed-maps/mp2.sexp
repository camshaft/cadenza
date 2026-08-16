(case "mp2 map VALUES are draws inserted in key order — weighted lookups replay the draw sequence through the map"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None) -999)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (let ((m (Map.insert (Map.insert (Map.insert (map) 1 (E.next)) 2 (E.next)) 3 (E.next))))
                  (+ (* 100 (get m 1)) (+ (* 10 (get m 2)) (get m 3))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 60 Int64))
  (call   main (: 1 Int64)) (output (: 171 Int64))
  (call   main (: -2 Int64)) (output (: -162 Int64)))
