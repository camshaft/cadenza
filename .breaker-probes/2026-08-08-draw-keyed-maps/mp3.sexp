(case "mp3 a draw picks the Map.remove key — lookups of all three keys show exactly one hole where the thread pointed"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None) -1)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k (+ (% (E.next) 3) 1)))
                  (let ((m (Map.remove (Map.insert (Map.insert (Map.insert (map) 1 10) 2 20) 3 30) k)))
                    (+ (* 100 (get m 1)) (+ (* 10 (get m 2)) (get m 3)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 130 Int64))
  (call   main (: 1 Int64)) (output (: 1020 Int64))
  (call   main (: 2 Int64)) (output (: 1199 Int64)))
