(case "mp1 draws pick the Map INSERT key and the LOOKUP key — hit-old, hit-updated, and miss all reachable by input"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k1 (+ (% (E.next) 3) 1)))
                  (let ((k2 (+ (% (E.next) 4) 1)))
                    (let ((m (Map.insert (Map.insert (Map.insert (Map.insert (map) 1 10) 2 20) 3 30) k1 77)))
                      (+ (* 100 (match (Map.lookup m k2)
                                  ((Some v) v)
                                  ((None) -5)))
                         (+ (* 10 k1) k2)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2012 Int64))
  (call   main (: 3 Int64)) (output (: 7711 Int64))
  (call   main (: 2 Int64)) (output (: -466 Int64)))
