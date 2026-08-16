(case "sq1 a SET-valued handler state accumulates uniques across resumes and re-tests at the end"
  (input  (do
            (effect Seen (op mark (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Seen (Set.of (list))
                ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v)))
                 (count (u) s (resume (Set.len s) s)))
                (do
                  (feed 0 n)
                  (Seen.count))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 7 Int64)))
