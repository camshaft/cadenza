(case "sd1 a 60-entry Map with SYMBOL values built recursively reads back by key"
  (input  (do
            (def (build (: i Int64) (: m (Map Int64 Symbol)))
              (if (= i 0) m
                  (build (- i 1) (Map.insert m i (if (= (% i 2) 0) (Symbol.of "even") (Symbol.of "odd"))))))
            (def (main (: k Int64))
              (do
                (def tbl (build 60 Map.empty))
                (+ (* 10 (match (Map.lookup tbl k)
                           ((Some s) (if (= s (Symbol.of "odd")) 1 2)) ((None _u) -1)))
                   (Map.len tbl))))
            (export main)))
  (call   main (: 33 Int64)) (output (: 70 Int64))
  (call   main (: 40 Int64)) (output (: 80 Int64)))
