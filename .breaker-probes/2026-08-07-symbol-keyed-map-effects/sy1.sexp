(case "sy1 a SYMBOL-keyed Map built from performs — interned keys look up across separate interns"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((m (Map.insert (Map.insert Map.empty (Symbol.of "a") (St.next)) (Symbol.of "b") (St.next))))
                  (+ (* 10 (match (Map.lookup m (Symbol.of "a")) ((Some v) v) ((None _u) -1)))
                     (match (Map.lookup m (Symbol.of "b")) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
