(case "a MAP inside a sum payload survives the wrap, match extraction, and edit round-trip"
  (doc    "The EDIT round-trip upgrade of the Map-in-sum-payload pin (:11860 reads len once and its
           doc notes the rust decline — now stale; this runs ALL targets): `add-entry` matches the
           CHAMP out of `Loaded`, inserts, and REWRAPS — twice, from the `Empty` seed through two
           state generations — then reads len + a value through the final wrap (200 + 30) while the
           INTERMEDIATE generation's map still has len 1 (1s digit: persistence across the
           wrap/extract/edit/rewrap cycle) → 231. The store-accumulator idiom (a cache/session sum
           whose Loaded payload grows per event); a rewrap that aliased the extracted map into the
           old generation breaks the persistence digit.")
  (input  (do
            (type Store (Loaded (Map Int64 Int64)) (Empty))
            (def (add-entry (: s Store) (: k Int64) (: v Int64))
              (match s
                ((Loaded m) (Loaded (Map.insert m k v)))
                ((Empty) (Loaded (Map.insert Map.empty k v)))))
            (def (main (: n Int64))
              (let ((s0 (Empty)))
                (let ((s1 (add-entry s0 1 n)))
                  (let ((s2 (add-entry s1 2 20)))
                    (+ (* 100 (match s2 ((Loaded m2) (Map.len m2)) ((Empty) -1)))
                       (+ (* 10 (match s2 ((Loaded m3) (match (Map.lookup m3 1) ((Some v) (- v (- n 3))) ((None u) -1))) ((Empty) -1)))
                          (match s1 ((Loaded m4) (Map.len m4)) ((Empty) -1))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 231 Int64)))
