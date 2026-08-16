(case "a Map used as a priority queue drains in key order via to-list head extraction"
  (doc    "The min-extraction DRAIN loop over a CHAMP map — the scheduler idiom (a DES ready-queue
           keyed by time): each iteration reads the canonical head (`List.at (Map.to-list m) 0` = the
           SMALLEST key's entry), removes that key, and folds the value positionally (acc·10+v), so
           the digit string IS the drain order — k=2 → keys 2,5,9 → values 1,3,4 → 134; k=7 → 5,7,9 →
           314. Composes three pinned laws into the loop that matters: canonical to-list order (the
           head is the min), remove-path canonicalization (the shrunk map's NEXT head is right), and
           the fold's order-sensitivity. A remove that left non-canonical structure surfaces here as
           a wrong SECOND digit — the loop catches what single-step pins cannot.")
  (input  (do
            (def (drain (: m (Map Int64 Int64)) (: acc Int64) (: fuel Int64))
              (if (= fuel 0)
                acc
                (if (= (Map.len m) 0)
                  acc
                  (match (List.at (Map.to-list m) 0)
                    ((Some e) (drain (Map.remove m (. e 0)) (+ (* acc 10) (. e 1)) (- fuel 1)))
                    ((None u) acc)))))
            (def (main (: k Int64))
              (drain (Map.insert (Map.insert (Map.insert Map.empty 5 3) k 1) 9 4) 0 10))
            (export main)))
  (call   main (: 2 Int64)) (output (: 134 Int64))
  (call   main (: 7 Int64)) (output (: 314 Int64)))
