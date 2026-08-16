(case "sd2 a Set of 40 DISTINCT interned symbols built from rope pieces has full cardinality"
  (input  (do
            (def (build (: i Int64) (: s (Set Symbol)))
              (if (= i 0) s
                  (build (- i 1) (Set.insert s (Symbol.of (String.concat (if (> i 20) "hi" "lo") (if (= (% i 2) 0) "even" "odd")))))))
            (def (main (: k Int64))
              (do
                (def syms (build 40 (Set.of (list (Symbol.of "seed")))))
                (+ (* 10 (Set.len syms))
                   (if (Set.contains syms (Symbol.of (String.concat "hi" "even"))) 1 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 51 Int64)))
