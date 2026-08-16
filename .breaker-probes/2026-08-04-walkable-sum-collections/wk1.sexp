(case "wk1 a list-FREE recursive sum keys a CHAMP trie at depth and set-eq converges order-independently"
  (input  (do
            (type T (TI Int64) (TP T T))
            (def (mk (: i Int64)) (if (= (% i 2) 0) (T.TI i) (T.TP (T.TI i) (T.TI (+ i 1)))))
            (def (up (: i Int64) (: n Int64) (: s (Set T)))
              (if (> i n) s (up (+ i 1) n (Set.insert s (mk i)))))
            (def (down (: i Int64) (: s (Set T)))
              (if (= i 0) s (down (- i 1) (Set.insert s (mk i)))))
            (def (main (: n Int64))
              (+ (* 10 (Set.len (up 1 n (Set.of (list)))))
                 (if (= (up 1 n (Set.of (list))) (down n (Set.of (list)))) 1 0)))
            (export main)))
  (call   main (: 60 Int64)) (output (: 601 Int64)))
