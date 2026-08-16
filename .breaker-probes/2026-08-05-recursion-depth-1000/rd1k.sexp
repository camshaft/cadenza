(case "rd1k a 1000-iteration effectful loop (deep recursion x performs — stack/fuel behavior at 10x prior scale)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (St.a)))))
            (def (main (: k Int64))
              (handle St 0
                ((a (u) s (resume s (+ s 1))))
                (loop k 0)))
            (export main)))
  (call   main (: 1000 Int64)) (output (: 499500 Int64)))
