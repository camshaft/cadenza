(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((m (Map.insert (Map.insert Map.empty (Symbol.of "a") (St.next)) (Symbol.of "b") (St.next))))
        (+ (* 10 (match (Map.lookup m (Symbol.of "a")) ((Some v) v) ((None _u) -1)))
           (match (Map.lookup m (Symbol.of "b")) ((Some v) v) ((None _u) -1))))))
  (export main))
