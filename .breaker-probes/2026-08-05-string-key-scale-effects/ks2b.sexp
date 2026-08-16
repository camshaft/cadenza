(case "ks2b escaped string-keyed state probed OUTSIDE the handle (hit + miss)"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "k") (- n 1))))
            (effect St (op stash (-> Int64 Int64)) (op grab (-> Unit (Map String Int64))))
            (def (fill (: i Int64))
              (if (= i 0) 0 (+ (* 0 (St.stash i)) (fill (- i 1)))))
            (def (main (: n Int64))
              (do
                (def m (handle St Map.empty
                         ((stash (v) s (resume 0 (Map.insert s (rep "" v) v)))
                          (grab (u) s (resume s s)))
                         (do (def _f (fill n)) (St.grab))))
                (+ (* 100 (match (Map.lookup m (rep "" 2)) ((Some x) x) ((None _u) -1)))
                   (match (Map.lookup m (rep "" 9)) ((Some _x) -1) ((None _u) 5)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 205 Int64)))
