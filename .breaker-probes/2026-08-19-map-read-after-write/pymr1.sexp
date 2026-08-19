(case "pymr1 probe: a MAP-STATE handler with insert-then-read-across-the-seam — put(k,v) threads Map.insert as the next-state, and a later fetch(k) must see the value the earlier put wrote (heap read-after-write through the resume seam), while a fetch of the seeded key still returns the seed"
  (input (do
  (effect E (op put (-> Int64 Int64 Int64)) (op fetch (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (Map.insert Map.empty (: 0 Int64) (+ (% n 3) (: 5 Int64)))
      ((put (k v) m (resume (Map.len m) (Map.insert m k v)))
       (fetch (k) m (resume (match (Map.lookup m k) ((Some x) x) ((None) (: -1 Int64))) m)))
      (do (E.put (: 7 Int64) (* (+ (% n 3) (: 5 Int64)) (: 10 Int64)))
          (+ (* 1000 (E.fetch (: 0 Int64))) (E.fetch (: 7 Int64))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 6060 Int64))
  (call   main (: 0 Int64)) (output (: 5050 Int64)))
