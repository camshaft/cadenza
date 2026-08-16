(case "ks1 a Map with STRING keys built across 40 performs, probed by a rebuilt rope key in-handle"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "k") (- n 1))))
            (effect St (op stash (-> Int64 Int64)) (op probe (-> Int64 Int64)))
            (def (fill (: i Int64))
              (if (= i 0) 0 (+ (* 0 (St.stash i)) (fill (- i 1)))))
            (def (main (: n Int64))
              (handle St Map.empty
                ((stash (v) s (resume 0 (Map.insert s (rep "" v) v)))
                 (probe (v) s (resume (match (Map.lookup s (rep "" v)) ((Some x) x) ((None _u) -1)) s)))
                (+ (fill n) (St.probe 25))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 25 Int64)))
