(case "a handle whose VALUE is a CHAMP map built by the body's performs is intact after discharge"
  (doc    "The handle's RESULT is a heap collection whose every VALUE came through a resume: the
           recursive body inserts n↦(Kv.put n) per level (each put reads state+v and advances), and
           the finished map ESCAPES the discharged handle to be read outside — len 3 and the key-1
           entry (last put: 102+1=103 → 3 after the −100 read) → 303. The map is built ACROSS three
           handler frames' worth of resume values and must survive the handle's teardown intact (a
           frame-scoped allocation of the resume values, or a handle exit that dropped the escaping
           CHAMP's interior, corrupts a read outside). The collection-result companion of the
           abort-value heap pins (:2570 — those yield an arm-built list; this yields a body-built
           map assembled FROM resume values through a recursion).")
  (input  (do
            (effect Kv (op put (-> Int64 Int64)))
            (def (go (: n Int64) (: m (Map Int64 Int64)))
              (if (= n 0) m (go (- n 1) (Map.insert m n (Kv.put n)))))
            (def (main (: n Int64))
              (let ((m (handle Kv 100
                         ((put (v) s (resume (+ s v) (+ s 1))))
                         (go n Map.empty))))
                (+ (* 100 (Map.len m))
                   (match (Map.lookup m 1) ((Some v) (- v 100)) ((None u) -1)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 303 Int64)))
