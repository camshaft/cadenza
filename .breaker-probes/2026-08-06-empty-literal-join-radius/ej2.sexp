(case "ej2 empty SET literal in a match-Option fallback — the Set sibling of ms13"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (Set.of (list))))))
                  (Set.len (Set.insert xs n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
