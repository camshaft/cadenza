(case "vv1 verification contracts on the visited-set idiom: @requires gates the mark domain"
  (input  (do
            (effect Seen (op mark (-> Int64 Int64)))
            (@ (requires (>= v 0)) (def (checked-mod (: v Int64)) (% v 7)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Seen.mark (checked-mod i)) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Seen (Set.of (list))
                ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v))))
                (feed 0 n)))
            (export main)))
  (call   main (: 20 Int64)) (output (: 13 Int64)))
