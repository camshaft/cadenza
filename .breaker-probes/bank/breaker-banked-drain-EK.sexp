(case "a fold over Map.to-list accumulates keys and values in canonical order"
  (doc    "The positional-fold face of map enumeration: a list-rest fold over Map.to-list destructures
           each (key, value) TUPLE entry and digit-packs BOTH components in sequence — the full
           six-digit string IS the enumeration order (k=1 → 123456; k=7 re-sorts the runtime key to
           the tail → 345672). The rebuild pin (:15307) reads only map EQUALITY after a fold; the
           order pins read entries by INDEX — this composes to-list order + tuple entry destructuring
           + a positional fold, where a swapped entry, a k/v transposition inside one tuple, or an
           order drift each corrupt a different digit pair. The report-generation idiom (walk a
           table, emit rows in key order).")
  (input  (do
            (def (fold-kv (: es (List (Tuple Int64 Int64))) (: acc Int64))
              (match es
                ((list) acc)
                ((list e .. t) (fold-kv t (+ (* acc 100) (+ (* 10 (. e 0)) (. e 1)))))))
            (def (main (: k Int64))
              (fold-kv (Map.to-list (Map.insert (Map.insert (Map.insert Map.empty 3 4) k 2) 5 6)) 0))
            (export main)))
  (call   main (: 1 Int64)) (output (: 123456 Int64))
  (call   main (: 7 Int64)) (output (: 345672 Int64)))
