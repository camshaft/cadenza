(case "cx1 CHARS as trie VALUES: multibyte scalars stored and retrieved at 30-entry depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Char)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i
                  (match (String.scalar-at "abcdefghijklmnopqrstuvwxyzéàü∀" (- i 1))
                    ((Some c) c) ((None _u) #\z))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m 27)
                     ((Some c) (if (= c #\é) 1 0))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 301 Int64)))
