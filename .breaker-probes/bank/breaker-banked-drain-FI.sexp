(case "LIST keys distinguish by full content — a prefix is a distinct key from its extension"
  (doc    "The prefix-distinction face of list Map keys: `[1,n]`, `[1,n,3]`, and `[1]` are THREE
           distinct keys (a list key's identity is its full content INCLUDING length, so a prefix
           never collides its extension) — all three coexist (len 3), and lookups land on their own
           entries (`[1,n]`→10, `[1]`→30) → 10303 at n=2. A key hash that folded only a prefix, or
           ignored length, would collapse `[1]`/`[1,n]`/`[1,n,3]`; the runtime n threads the shared
           middle element so the const path can't pre-resolve. List keys exist (13-strings:3470
           single-element); the multi-length prefix/extension coexistence was unpinned.")
  (input  (do
            (def (main (: n Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty
                          (list 1 n) 10)
                          (list 1 n 3) 20)
                          (list 1) 30)))
                (+ (* 1000 (match (Map.lookup m (list 1 n)) ((Some v) v) ((None u) -1)))
                   (+ (* 10 (match (Map.lookup m (list 1)) ((Some v2) v2) ((None u2) -1)))
                      (Map.len m)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10303 Int64)))
