(case "sq2 the mark op's RESULT (dup-flag) counts repeats seen while the set dedupes"
  (input  (do
            (effect Seen (op mark (-> Int64 Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Seen (Set.of (list))
                ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v))))
                (feed 0 n)))
            (export main)))
  (call   main (: 20 Int64)) (output (: 13 Int64)))
