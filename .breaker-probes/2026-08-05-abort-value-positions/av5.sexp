(case "av5 an abort value as a RECURSIVE call's argument ((fib (handle ... abort)) — abort feeds recursion)"
  (input  (do
            (effect St (op bail (-> Unit Int64)))
            (def (fib (: n Int64))
              (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
            (def (main (: n Int64))
              (fib (handle St n ((bail (u) s (+ s 4))) (+ 999 (St.bail)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 55 Int64)))
