(case "a recursive filter over Map.to-list rebuilds a map keeping only even-valued entries"
  (doc    "The FILTER-rebuild idiom (the :15324 rebuild pin re-inserts EVERY entry unconditionally;
           this CONDITIONALLY re-inserts): fold Map.to-list, keep an entry only if its value is even,
           into a fresh accumulator map — the runtime n=10 makes key-1's value even (kept, len 2) or
           n=15 odd (dropped, len 1). Reads len + membership of the survivors (210 / 110). A filter
           that mis-threaded the accumulator (dropped a kept entry, or carried the source map's
           structure into acc) breaks a digit; the conditional Map.insert on the acc must path-copy
           each survivor onto the growing result, not the enumerated source. The query-projection
           idiom (SELECT WHERE) over a CHAMP.")
  (input  (do
            (def (keep-even (: es (List (Tuple Int64 Int64))) (: acc (Map Int64 Int64)))
              (match es
                ((list) acc)
                ((list e .. t)
                  (keep-even t (if (= (% (. e 1) 2) 0) (Map.insert acc (. e 0) (. e 1)) acc)))))
            (def (main (: n Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty 1 n) 2 20) 3 25)))
                (let ((filtered (keep-even (Map.to-list m) Map.empty)))
                  (+ (* 100 (Map.len filtered))
                     (+ (* 10 (match (Map.lookup filtered 2) ((Some v) 1) ((None u) 0)))
                        (match (Map.lookup filtered 3) ((Some v2) 1) ((None u2) 0)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 210 Int64))
  (call   main (: 15 Int64)) (output (: 110 Int64)))
