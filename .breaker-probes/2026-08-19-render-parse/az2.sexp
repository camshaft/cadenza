(case "az2 the rendered string PARSES back: char walk rebuilds the digit list (render-parse loop)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (% i 10)))))
            (def (digit (: v Int64))
              (Option.expect (String.at "0123456789" v) "digit"))
            (def (render (: ps (List (Tuple Int64 Int64))) (: acc String))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (render t (String.concat acc (digit v))))))))
            (def (parse (: s String) (: i Int64) (: len Int64) (: acc Int64))
              (if (>= i len) acc
                (parse s (+ i 1) len
                  (+ acc (match (String.at s i)
                           ((Some c) (if (= c "7") 1 0))
                           ((None _u) 0))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def s (render (Map.to-list m) ""))
                (parse s 0 (String.scalar-len s) 0)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 2 Int64)))
