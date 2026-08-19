(case "pymx1 probe: a MAP-STATE handler with REMOVE across the seam — rm(k) threads Map.remove as the next-state and answers the shrunk length, so a later get(k) of the removed key returns None while a get of a surviving key still returns its value; delete-then-lookup consistency through the resume seam"
  (input (do
  (effect E (op rm (-> Int64 Int64)) (op get (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (Map.insert (Map.insert Map.empty (: 0 Int64) (+ (% n 3) (: 5 Int64))) (: 9 Int64) (: 100 Int64))
      ((rm (k) m (resume (Map.len (Map.remove m k)) (Map.remove m k)))
       (get (k) m (resume (match (Map.lookup m k) ((Some x) x) ((None) (: -1 Int64))) m)))
      (do (E.rm (: 9 Int64))
          (+ (* 1000 (E.get (: 0 Int64))) (E.get (: 9 Int64))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 5999 Int64))
  (call   main (: 0 Int64)) (output (: 4999 Int64)))
