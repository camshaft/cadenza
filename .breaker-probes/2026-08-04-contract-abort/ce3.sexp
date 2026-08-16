(case "ce3 a RESUMING handler around a contracted body still enforces @ensures on the resumed value"
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (@ (ensures (< ret 10)) (def (f (: x Int64))
              (+ x (Ask.get))))
            (def (main (: x Int64))
              (handle Ask 0 ((get (_u) s (resume 50 s))) (f x)))
            (export main)))
  (call   main (: 1 Int64)) (trap "unreachable")
  (call   main (: -45 Int64)) (output (: 5 Int64)))
