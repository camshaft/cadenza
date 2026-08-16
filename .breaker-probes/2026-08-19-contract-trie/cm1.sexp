(case "cm1 @requires over a 40-entry trie argument walks the deep CHAMP at body entry"
  (input  (do
        (def (fill (: i Int64) (: m (Map Int64 Int64)))
          (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 4)))))
        (@ (requires (> (Map.len m) 30)) (def (deep-val (: m (Map Int64 Int64)))
          (match (Map.lookup m 35) ((Some v) v) ((None _u) -1))))
        (def (main (: n Int64))
          (deep-val (fill n Map.empty)))
        (export main)))
  (call   main (: 40 Int64)) (output (: 140 Int64))
  (call   main (: 10 Int64)) (trap "unreachable"))
