(case "az1 a fold builds a STRING by concat from trie-enumerated values (render walk)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (% i 10)))))
            (def (digit (: v Int64))
              (Option.expect (String.at "0123456789" v) "digit"))
            (def (render (: ps (List (Tuple Int64 Int64))) (: acc String))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (render t (String.concat acc (digit v))))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def s (render (Map.to-list m) ""))
                (+ (* 10 (String.byte-len s))
                   (match (String.at s 0) ((Some c) (if (= c "1") 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 251 Int64)))
