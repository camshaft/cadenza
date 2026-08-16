(case "sk1 a trie of 40 STRING keys (rope-built, shared prefixes) resolves content descent"
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (fill (: i Int64) (: m (Map String Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (String.concat "key" (rep "-" i)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (String.concat "key" (rep "-" 25))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 425 Int64)))
