(case "sk2 a rope-built String key probes the trie stored under its FLAT twin at depth"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "y") (- n 1))))
            (def (fill (: i Int64) (: m (Map String Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (rep "p" i) i))))
            (def (main (: n Int64))
              (do
                (def m (Map.insert (fill n Map.empty) "flat-yyy" 777))
                (match (Map.lookup m (String.concat "flat-" (rep "" 3))) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 777 Int64)))
