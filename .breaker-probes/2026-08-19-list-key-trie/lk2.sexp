(case "lk2 a concat-ASSEMBLED list key probes the trie entry stored under its push-built twin"
  (input  (do
            (def (mk (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (mk (- i 1) (List.push acc i))))
            (def (dseg (: hi Int64) (: lo Int64) (: acc (List Int64)))
              (if (< hi lo) acc (dseg (- hi 1) lo (List.push acc hi))))
            (def (fill (: i Int64) (: m (Map (List Int64) Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (mk i (list)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def probe (List.concat (dseg 30 16 (list)) (dseg 15 1 (list))))
                (match (Map.lookup m probe) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 30 Int64)))
