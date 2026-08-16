(case "sk1 a handler state SHRINKS per dispatch — Map.remove down to empty across resume cycles"
  (input  (do
            (effect Db (op evict (-> String Int64)))
            (def (main (: n Int64))
              (handle Db (Map.insert (Map.insert (Map.insert Map.empty "a" n) "b" 7) "c" 9)
                ((evict (k) m (resume (Map.len (Map.remove m k)) (Map.remove m k))))
                (+ (* 100 (Db.evict "a"))
                   (+ (* 10 (Db.evict "b"))
                      (Db.evict "c")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 210 Int64)))
