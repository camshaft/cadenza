(case "a Map's keys collected into a Set support membership tests distinct from Map.lookup"
  (doc    "The cross-collection projection (a Map's key SET): fold Map.to-list projecting each entry's
           key (`. e 0`) into a fresh Set, then membership-test the Set — a runtime k that collides an
           existing key (k=5) collapses the keyset to len 2, else len 3; `Set.contains ks 5` hits, `99`
           misses → 310/210. The keys flow Map→tuple-project→Set-insert (a value crossing from CHAMP
           entry to CHAMP element), and the resulting Set's dedup catches the k=5 collision
           independently of the Map's own key dedup. The Map-domain-as-Set idiom (key-set algebra:
           which keys does this table have?).")
  (input  (do
            (def (keyset (: es (List (Tuple Int64 Int64))) (: acc (Set Int64)))
              (match es
                ((list) acc)
                ((list e .. t) (keyset t (Set.insert acc (. e 0))))))
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty 1 10) k 20) 5 30)))
                (let ((ks (keyset (Map.to-list m) (Set.of (list)))))
                  (+ (* 100 (Set.len ks))
                     (+ (* 10 (if (Set.contains ks 5) 1 0))
                        (if (Set.contains ks 99) 1 0))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 310 Int64))
  (call   main (: 5 Int64)) (output (: 210 Int64)))
