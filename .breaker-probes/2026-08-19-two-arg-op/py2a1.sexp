(case "py2a1 probe: a TWO-ARGUMENT op combine(a,b) whose arm uses BOTH args and the captured state in the resume answer (a*s+b); two dispatches pass different arg pairs while the state threads"
  (input (do
  (effect E (op combine (-> Int64 Int64 Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((combine (a b) s (resume (+ (* a s) b) (+ s 1))))
      (+ (* 100 (E.combine 3 5)) (E.combine 2 7))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 811 Int64))
  (call   main (: 0 Int64)) (output (: 509 Int64)))
