(case "qq2 TWO splices from distinct trie retrievals compute in one eval template"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 5)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def a (match (Map.lookup m 10) ((Some x) x) ((None _u) -1)))
                (def b (match (Map.lookup m 20) ((Some x) x) ((None _u) -1)))
                (eval (quasiquote (+ (unquote a) (* (unquote b) 2))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 250 Int64)))
