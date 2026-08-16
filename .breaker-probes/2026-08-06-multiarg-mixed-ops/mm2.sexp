(case "mm2 a multi-argument op with TWO heap arguments — a String key and a Map to search"
  (input  (do
            (effect St (op find (-> String (Map String Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((find (k m) s
                  (resume (+ (* 10 (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
                             (Map.len m))
                          s)))
                (St.find (String.concat "k" "1")
                         (Map.insert (Map.insert Map.empty "k1" n) "k2" 30))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 52 Int64)))
