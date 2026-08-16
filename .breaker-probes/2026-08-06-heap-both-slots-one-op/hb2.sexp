(case "hb2 a map-to-map transformer op CHAINED — the second dispatch receives the first's result"
  (input  (do
            (effect St (op stamp (-> (Map String Int64) (Map String Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((stamp (m) s (resume (Map.insert m (if (= s 0) "first" "second") (+ s n)) (+ s 1))))
                (let ((m2 (St.stamp (St.stamp (Map.insert Map.empty "seed" 1)))))
                  (+ (* 100 (Map.len m2))
                     (+ (* 10 (match (Map.lookup m2 "first") ((Some a) a) ((None _u) -1)))
                        (match (Map.lookup m2 "second") ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 356 Int64)))
