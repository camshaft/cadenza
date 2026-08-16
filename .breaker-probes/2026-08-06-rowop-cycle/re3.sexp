(case "re3 pop's two halves recombine: extend(rest, popped) restores the original as a key"
  (input  (do
            (def (main (: n Int64))
              (do
                (def r (record (a n) (b 2) (c 3)))
                (def split (Record.pop r a))
                (def restored (Record.extend (. split 1) #"a" (. split 0)))
                (match (Map.lookup (Map.insert Map.empty r 42) restored)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
