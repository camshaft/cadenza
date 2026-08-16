(case "a reversed-field-order record key OVERWRITES and REMOVES its canonical twin's entry"
  (doc    "The EDIT faces of record-key field-order independence (the :100 Set pin covers dedup +
           membership READS): inserting `(record (y 2) (x 1))` must OVERWRITE the entry keyed by
           `(record (x 1) (y 2))` — same canonical key, len stays 1 (100s) and the lookup reads the
           NEW value k=20 (10s digit reads v−18 = 2... encoded 100·1 + 10·2 + 0 = 120 with the ones
           digit the post-REMOVE len: removing by the reversed spelling empties the map). A key-hash
           computed over WRITTEN order would give len 2, keep the old value, and miss the remove —
           three faces, one wrong bit each.")
  (input  (do
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert Map.empty (record (x 1) (y 2)) 10)
                                   (record (y 2) (x 1)) k)))
                (+ (* 100 (Map.len m))
                   (+ (* 10 (match (Map.lookup m (record (x 1) (y 2))) ((Some v) (- v 18)) ((None u) -1)))
                      (Map.len (Map.remove m (record (y 2) (x 1))))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 120 Int64)))
