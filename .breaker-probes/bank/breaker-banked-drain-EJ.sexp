(case "a record VALUE inside a map is functionally updated and reinserted without disturbing siblings"
  (doc    "The RECORD-valued face of the keyed read-modify-write (:7924 pins the List-valued bucket
           push): lookup key 1, `Record.with` the hp field down, re-insert — the updated entry reads
           15 (1000s), the SIBLING entry 2 is untouched through the new map (10s: 99), and the
           ORIGINAL map's entry 1 still reads the pre-update value (1s: persistence) → 15992 at n=20.
           The modify step routes through the ROW-OP path-copy (with) rather than a collection push,
           and the sibling read pins that the re-insert's CHAMP path-copy left the other entry's
           subtree shared intact — the entity-table update idiom (game state, session stores).")
  (input  (do
            (def (main (: n Int64))
              (let ((m (Map.insert (Map.insert Map.empty
                          1 (record (hp n) (mp 30))
                          ) 2 (record (hp 99) (mp 0)))))
                (let ((m2 (match (Map.lookup m 1)
                            ((Some r) (Map.insert m 1 (Record.with r #"hp" (- (. r hp) 5))))
                            ((None u) m))))
                  (+ (* 1000 (match (Map.lookup m2 1) ((Some r2) (. r2 hp)) ((None u2) -1)))
                     (+ (* 10 (match (Map.lookup m2 2) ((Some r3) (. r3 hp)) ((None u3) -1)))
                        (match (Map.lookup m 1) ((Some r4) (- (. r4 hp) (- n 2))) ((None u4) -1)))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 15992 Int64)))
