(case "ss4 a STRING slice view inside a nested tuple key at trie depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map (Tuple String Int64) Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (tuple "tag" i) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def w (match (String.slice "xtagy" 1 4) ((Some s) s) ((None _u) "")))
                (match (Map.lookup m (tuple w 25)) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 25 Int64)))
